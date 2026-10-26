//! Decoding of one tile: packets, code-blocks, dequantisation, inverse wavelet transform,
//! component transform and level shift.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use rayon::prelude::*;

use super::codestream::{Codestream, Effective};
use super::dwt::{Rect, inverse_53, inverse_97};
use super::tier1::decode_block;
use super::tier2::{TileComp, build_comp, packet_order, read_packet};
use crate::{Error, Result};

fn bad(what: &str) -> Error {
    Error::Format(format!("JPEG 2000: {what}"))
}

/// A decoded tile at some reduction: the samples of the components asked for.
pub struct TileOut {
    /// On the grid of the reduced image (not yet shifted by the image origin).
    pub rect: Rect,
    pub planes: Vec<Vec<f32>>,
}

enum Plane {
    Int(Vec<i32>),
    Float(Vec<f32>),
}

/// Copies a decoded code-block into the buffer of its subband.
fn place<T: Copy>(buf: &mut [T], band: Rect, blk: Rect, vals: &[i32], conv: impl Fn(i32) -> T) {
    let bw = band.w();
    let w = blk.w();
    let (ox, oy) = ((blk.x0 - band.x0) as usize, (blk.y0 - band.y0) as usize);
    for y in 0..blk.h() {
        let dst = (oy + y) * bw + ox;
        for x in 0..w {
            buf[dst + x] = conv(vals[y * w + x]);
        }
    }
}

fn gather_tile(path: &Path, cs: &Codestream, t: usize) -> Result<Vec<u8>> {
    let parts = &cs.tiles[t].parts;
    let total: u64 = parts.iter().map(|p| p.len).sum();
    let mut data = Vec::with_capacity(total as usize);
    let mut f = File::open(path)?;
    for p in parts {
        f.seek(SeekFrom::Start(p.offset))?;
        let start = data.len();
        data.resize(start + p.len as usize, 0);
        f.read_exact(&mut data[start..])
            .map_err(|_| bad("datos de la tesela cortados"))?;
    }
    Ok(data)
}

pub fn decode_tile(
    path: &Path,
    cs: &Codestream,
    t: usize,
    reduce: u32,
    want: &[usize],
) -> Result<TileOut> {
    let siz = &cs.siz;
    let th = &cs.tiles[t];
    if th.parts.is_empty() {
        return Err(bad("falta una tesela en el archivo"));
    }
    if cs.main.poc || th.params.poc {
        return Err(Error::Unsupported(
            "JPEG 2000: los cambios de progresión (POC) no están soportados".into(),
        ));
    }
    if cs.main.packed || th.params.packed {
        return Err(Error::Unsupported(
            "JPEG 2000: las cabeceras de paquetes empaquetadas (PPM/PPT) no están soportadas"
                .into(),
        ));
    }
    let eff = Effective {
        main: &cs.main,
        tile: &th.params,
    };
    let cod = eff.cod();
    let across = siz.tiles_across();
    let (p, q) = (t as u32 % across, t as u32 / across);
    let tile = Rect {
        x0: i64::from((siz.xto + p * siz.xt).max(siz.xo)),
        y0: i64::from((siz.yto + q * siz.yt).max(siz.yo)),
        x1: i64::from((siz.xto + (p + 1) * siz.xt).min(siz.x)),
        y1: i64::from((siz.yto + (q + 1) * siz.yt).min(siz.y)),
    };
    let ncomp = siz.comps.len();
    if (tile.x1 - tile.x0) as u64 * (tile.y1 - tile.y0) as u64 > 1 << 31 {
        return Err(bad("tesela demasiado grande"));
    }
    let mut tcs: Vec<TileComp> = Vec::with_capacity(ncomp);
    for c in 0..ncomp {
        let cc = eff
            .comp_cod(c)
            .ok_or_else(|| bad("falta el estilo de codificación"))?;
        let qq = eff.quant(c).ok_or_else(|| bad("falta la cuantización"))?;
        tcs.push(build_comp(siz, c, tile, cc, qq, eff.roi_shift(c))?);
    }
    // the packets
    let data = gather_tile(path, cs, t)?;
    let order = packet_order(tile, &tcs, cod.prog, usize::from(cod.layers))?;
    let mut pos = 0usize;
    for pk in order {
        if pos >= data.len() {
            break;
        }
        read_packet(&data, &mut pos, &mut tcs[pk.comp], pk, &cod)?;
    }
    drop(data);

    // components to decode: the ones asked for, and the first three when they are transformed
    let mct = cod.mct == 1 && ncomp >= 3;
    let mut need: Vec<usize> = want.to_vec();
    if mct {
        for c in 0..3 {
            if !need.contains(&c) {
                need.push(c);
            }
        }
    }
    need.sort_unstable();
    let mut planes: Vec<Option<Plane>> = (0..ncomp).map(|_| None).collect();
    let mut out_rect = None;
    for &c in &need {
        let tc = &tcs[c];
        let nl = tc.res.len() - 1;
        if reduce as usize > nl {
            return Err(Error::Param(format!(
                "JPEG 2000: la imagen solo tiene {nl} niveles de reducción"
            )));
        }
        let target = nl - reduce as usize;
        let rev = tc.cod.reversible;
        // tier-1 on every code-block up to the target resolution
        let mut jobs = Vec::new();
        for r in 0..=target {
            for (b, band) in tc.res[r].bands.iter().enumerate() {
                for (pi, pb) in band.precs.iter().enumerate() {
                    for (k, cb) in pb.blocks.iter().enumerate() {
                        if !cb.segs.is_empty() {
                            jobs.push((r, b, pi, k));
                        }
                    }
                }
            }
        }
        let decoded: Vec<Vec<i32>> = jobs
            .par_iter()
            .map(|&(r, b, pi, k)| {
                let band = &tc.res[r].bands[b];
                let cb = &band.precs[pi].blocks[k];
                decode_block(
                    cb.rect.w(),
                    cb.rect.h(),
                    band.orient,
                    tc.cod.style,
                    band.numbps.saturating_sub(cb.zero_planes),
                    tc.roishift,
                    &cb.segs,
                )
            })
            .collect();
        // subband buffers, then the inverse transform level by level
        let mut cur_i: Vec<i32> = Vec::new();
        let mut cur_f: Vec<f32> = Vec::new();
        let mut at = 0;
        for r in 0..=target {
            let res = &tc.res[r];
            let mut bi: Vec<Vec<i32>> = Vec::new();
            let mut bf: Vec<Vec<f32>> = Vec::new();
            for band in &res.bands {
                let n = band.rect.w() * band.rect.h();
                if rev {
                    bi.push(vec![0; n]);
                } else {
                    bf.push(vec![0.0; n]);
                }
            }
            while at < jobs.len() && jobs[at].0 == r {
                let (_, b, pi, k) = jobs[at];
                let band = &res.bands[b];
                let cb = &band.precs[pi].blocks[k];
                if rev {
                    place(&mut bi[b], band.rect, cb.rect, &decoded[at], |v| v / 2);
                } else {
                    let s = 0.5 * band.step;
                    place(&mut bf[b], band.rect, cb.rect, &decoded[at], |v| {
                        v as f32 * s
                    });
                }
                at += 1;
            }
            if r == 0 {
                if rev {
                    cur_i = bi.swap_remove(0);
                } else {
                    cur_f = bf.swap_remove(0);
                }
            } else if rev {
                cur_i = inverse_53(res.rect, &cur_i, &bi[0], &bi[1], &bi[2]);
            } else {
                cur_f = inverse_97(res.rect, &cur_f, &bf[0], &bf[1], &bf[2]);
            }
        }
        out_rect = Some(tc.res[target].rect);
        planes[c] = Some(if rev {
            Plane::Int(cur_i)
        } else {
            Plane::Float(cur_f)
        });
    }
    let rect = out_rect.ok_or_else(|| bad("ningún componente que decodificar"))?;

    // component transform
    if mct {
        let [Some(a), Some(b), Some(cc)] = [planes[0].take(), planes[1].take(), planes[2].take()]
        else {
            return Err(bad("faltan componentes para la transformación"));
        };
        match (a, b, cc) {
            (Plane::Int(mut y0), Plane::Int(mut y1), Plane::Int(mut y2)) => {
                for i in 0..y0.len() {
                    let g = y0[i] - ((y2[i] + y1[i]) >> 2);
                    let r = y2[i] + g;
                    let bl = y1[i] + g;
                    (y0[i], y1[i], y2[i]) = (r, g, bl);
                }
                planes[0] = Some(Plane::Int(y0));
                planes[1] = Some(Plane::Int(y1));
                planes[2] = Some(Plane::Int(y2));
            }
            (Plane::Float(mut y0), Plane::Float(mut y1), Plane::Float(mut y2)) => {
                for i in 0..y0.len() {
                    let (y, cb, cr) = (y0[i], y1[i], y2[i]);
                    y0[i] = y + 1.402 * cr;
                    y1[i] = y - 0.344_136 * cb - 0.714_136 * cr;
                    y2[i] = y + 1.772 * cb;
                }
                planes[0] = Some(Plane::Float(y0));
                planes[1] = Some(Plane::Float(y1));
                planes[2] = Some(Plane::Float(y2));
            }
            _ => {
                return Err(Error::Unsupported(
                    "JPEG 2000: componentes con filtros distintos".into(),
                ));
            }
        }
    }

    // level shift and clamping, into f32 samples
    let mut result = Vec::with_capacity(want.len());
    for &c in want {
        let comp = &siz.comps[c];
        let d = i32::from(comp.depth);
        let (lo, hi, shift) = if comp.signed {
            (-(1i64 << (d - 1)), (1i64 << (d - 1)) - 1, 0i64)
        } else {
            (0, (1i64 << d) - 1, 1i64 << (d - 1))
        };
        let (lo, hi) = (lo as f32, hi as f32);
        let shift = shift as f32;
        let v: Vec<f32> = match planes[c].take() {
            Some(Plane::Int(v)) => v
                .into_iter()
                .map(|x| (x as f32 + shift).clamp(lo, hi))
                .collect(),
            Some(Plane::Float(v)) => v
                .into_iter()
                .map(|x| (x + shift).round().clamp(lo, hi))
                .collect(),
            None => return Err(bad("componente sin decodificar")),
        };
        result.push(v);
    }
    Ok(TileOut {
        rect,
        planes: result,
    })
}
