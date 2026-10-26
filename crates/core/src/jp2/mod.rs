//! JPEG 2000 reader (ITU-T T.800, Part 1) in pure Rust: the JP2 file format and the bare
//! codestream (`.jp2`, `.j2k`, `.j2c`), the 5/3 and 9/7 wavelets, any progression order, tiles,
//! precincts and the coding options of the block coder. It reads windows at full resolution or
//! with the wavelet's reductions (each one halves the image), decoding only the tiles it needs.
//!
//! Georeferencing comes from GMLJP2 (Sentinel-2) or GeoJP2.

mod codestream;
mod decode;
mod dwt;
mod tier1;
mod tier2;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;

use crate::{Error, Result};
use codestream::Codestream;
use tier2::cdiv;

fn bad(what: &str) -> Error {
    Error::Format(format!("JPEG 2000: {what}"))
}

/// Where the image lies on the ground.
#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub x0: f64,
    pub y_top: f64,
    pub cell: f64,
    pub epsg: Option<u16>,
}

/// An opened JPEG 2000 image. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Jp2 {
    path: PathBuf,
    cs: Arc<Codestream>,
    pub width: u32,
    pub height: u32,
    pub bands: usize,
    pub depth: u8,
    /// How many times the image can be halved by the wavelet.
    pub levels: u32,
    pub geo: Option<Geo>,
}

/// Whether the first bytes of a file are those of a JPEG 2000 file or codestream.
pub fn is_jp2(head: &[u8]) -> bool {
    head.starts_with(&[0, 0, 0, 12, b'j', b'P', b' ', b' '])
        || head.starts_with(&[0xFF, 0x4F, 0xFF, 0x51])
}

struct BoxHead {
    kind: [u8; 4],
    body: u64,
    end: u64,
}

fn read_box(f: &mut File, at: u64, limit: u64) -> Result<Option<BoxHead>> {
    if at + 8 > limit {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(at))?;
    let mut h = [0u8; 8];
    if f.read_exact(&mut h).is_err() {
        return Ok(None);
    }
    let l = u64::from(u32::from_be_bytes([h[0], h[1], h[2], h[3]]));
    let kind = [h[4], h[5], h[6], h[7]];
    let (len, hs) = match l {
        0 => (limit - at, 8),
        1 => {
            let mut x = [0u8; 8];
            f.read_exact(&mut x).map_err(|_| bad("caja cortada"))?;
            (u64::from_be_bytes(x), 16)
        }
        n => (n, 8),
    };
    if len < hs {
        return Err(bad("caja no válida"));
    }
    Ok(Some(BoxHead {
        kind,
        body: at + hs,
        end: (at + len).min(limit),
    }))
}

/// Georeferencing from the text of a GMLJP2 document: the centre of the first pixel and the two
/// offset vectors of the grid.
fn parse_gml(text: &str) -> Option<Geo> {
    let tag_text = |open: &str, close: &str, from: usize| -> Option<(String, usize)> {
        let s = text[from..].find(open)? + from;
        let gt = text[s..].find('>')? + s + 1;
        let e = text[gt..].find(close)? + gt;
        Some((text[gt..e].to_string(), e))
    };
    let nums = |s: &str| -> Vec<f64> {
        s.split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect()
    };
    let (pos, _) = tag_text("<gml:pos", "</gml:pos>", 0)?;
    let pos = nums(&pos);
    let (v1, e1) = tag_text("<gml:offsetVector", "</gml:offsetVector>", 0)?;
    let (v2, _) = tag_text("<gml:offsetVector", "</gml:offsetVector>", e1)?;
    let (v1, v2) = (nums(&v1), nums(&v2));
    if pos.len() < 2 || v1.len() < 2 || v2.len() < 2 {
        return None;
    }
    let cx = v1[0].hypot(v1[1]);
    let cy = v2[0].hypot(v2[1]);
    if cx <= 0.0 || cy <= 0.0 {
        return None;
    }
    let epsg = text.find("srsName=\"").and_then(|i| {
        let rest = &text[i + 9..];
        let name = &rest[..rest.find('"')?];
        name.rsplit([':', '/']).next()?.parse().ok()
    });
    Some(Geo {
        x0: pos[0] - cx / 2.0,
        y_top: pos[1] + cy / 2.0,
        cell: cx,
        epsg,
    })
}

/// Georeferencing from the little GeoTIFF that a GeoJP2 box holds (its tags only).
fn parse_geotiff(b: &[u8]) -> Option<Geo> {
    let le = match b.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16at = |o: usize| -> Option<u16> {
        let s: [u8; 2] = b.get(o..o + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    };
    let u32at = |o: usize| -> Option<u32> {
        let s: [u8; 4] = b.get(o..o + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    };
    let f64at = |o: usize| -> Option<f64> {
        let s: [u8; 8] = b.get(o..o + 8)?.try_into().ok()?;
        Some(if le {
            f64::from_le_bytes(s)
        } else {
            f64::from_be_bytes(s)
        })
    };
    let ifd = u32at(4)? as usize;
    let n = usize::from(u16at(ifd)?);
    let (mut scale, mut tie, mut keys) = (Vec::new(), Vec::new(), Vec::new());
    for e in 0..n {
        let at = ifd + 2 + e * 12;
        let (tag, count, val) = (u16at(at)?, u32at(at + 4)? as usize, u32at(at + 8)? as usize);
        match tag {
            33550 => scale = (0..count).filter_map(|i| f64at(val + 8 * i)).collect(),
            33922 => tie = (0..count).filter_map(|i| f64at(val + 8 * i)).collect(),
            34735 => keys = (0..count).filter_map(|i| u16at(val + 2 * i)).collect(),
            _ => {}
        }
    }
    let key = |id: u16| {
        keys.get(4..)?
            .as_chunks::<4>()
            .0
            .iter()
            .find(|k| k[0] == id && k[1] == 0)
            .map(|k| k[3])
    };
    let (sx, tx, ty) = (*scale.first()?, *tie.get(3)?, *tie.get(4)?);
    // a "point" raster type puts the tie point at the pixel centre
    let half = if key(1025) == Some(2) { sx / 2.0 } else { 0.0 };
    Some(Geo {
        x0: tx - half,
        y_top: ty + half,
        cell: sx,
        epsg: key(3072).or_else(|| key(2048)),
    })
}

const GEOJP2_UUID: [u8; 16] = [
    0xb1, 0x4b, 0xf8, 0xbd, 0x08, 0x3d, 0x4b, 0x43, 0xa5, 0xae, 0x8c, 0xd7, 0xd5, 0xa6, 0xce, 0x03,
];

fn find_geo(f: &mut File, from: u64, to: u64, depth: u32, found: &mut Option<Geo>) -> Result<()> {
    let mut at = from;
    while let Some(bx) = read_box(f, at, to)? {
        at = bx.end;
        let size = bx.end - bx.body;
        match &bx.kind {
            b"asoc" | b"jp2h" if depth < 4 => find_geo(f, bx.body, bx.end, depth + 1, found)?,
            b"xml " if found.is_none() && size < (8 << 20) => {
                f.seek(SeekFrom::Start(bx.body))?;
                let mut buf = vec![0u8; size as usize];
                f.read_exact(&mut buf)?;
                *found = parse_gml(&String::from_utf8_lossy(&buf));
            }
            b"uuid" if found.is_none() && (16..(8 << 20)).contains(&size) => {
                f.seek(SeekFrom::Start(bx.body))?;
                let mut buf = vec![0u8; size as usize];
                f.read_exact(&mut buf)?;
                if buf[..16] == GEOJP2_UUID {
                    *found = parse_geotiff(&buf[16..]);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

impl Jp2 {
    pub fn open(path: &Path) -> Result<Self> {
        let mut f = File::open(path)?;
        let len = f.metadata()?.len();
        let mut head = [0u8; 12];
        f.read_exact(&mut head)
            .map_err(|_| bad("archivo demasiado corto"))?;
        let (start, end, geo) = if head.starts_with(&[0xFF, 0x4F]) {
            (0, len, None)
        } else if is_jp2(&head) {
            let mut stream = None;
            let mut at = 0;
            while let Some(bx) = read_box(&mut f, at, len)? {
                if &bx.kind == b"jp2c" && stream.is_none() {
                    stream = Some((bx.body, bx.end));
                }
                at = bx.end;
            }
            let (s, e) = stream.ok_or_else(|| bad("no contiene un codestream"))?;
            let mut geo = None;
            find_geo(&mut f, 0, len, 0, &mut geo)?;
            (s, e, geo)
        } else {
            return Err(bad("no es un archivo JPEG 2000"));
        };
        let cs = codestream::scan(&f, start, end)?;
        let siz = &cs.siz;
        if siz.comps.iter().any(|c| c.dx != 1 || c.dy != 1) {
            return Err(Error::Unsupported(
                "JPEG 2000: componentes con submuestreo no están soportados".into(),
            ));
        }
        if u64::from(siz.x - siz.xo) * u64::from(siz.y - siz.yo) > 1 << 36 {
            return Err(bad("imagen demasiado grande"));
        }
        let levels = cs
            .main
            .cod
            .as_ref()
            .map_or(0, |c| u32::from(c.1.levels))
            .min(
                cs.tiles
                    .iter()
                    .filter_map(|t| t.params.cod.as_ref().map(|c| u32::from(c.1.levels)))
                    .min()
                    .unwrap_or(32),
            );
        Ok(Jp2 {
            path: path.to_path_buf(),
            width: siz.x - siz.xo,
            height: siz.y - siz.yo,
            bands: siz.comps.len(),
            depth: siz.comps[0].depth,
            levels,
            geo,
            cs: Arc::new(cs),
        })
    }

    /// Size of the image after `reduce` halvings.
    pub fn size_at(&self, reduce: u32) -> (usize, usize) {
        let s = &self.cs.siz;
        (
            (cdiv(i64::from(s.x), reduce) - cdiv(i64::from(s.xo), reduce)) as usize,
            (cdiv(i64::from(s.y), reduce) - cdiv(i64::from(s.yo), reduce)) as usize,
        )
    }

    /// Decodes the pixels `[x0, x1) x [y0, y1)` of the image reduced `reduce` times, keeping one
    /// pixel in every `step`, as one vector per band (rows north to south).
    pub fn read(
        &self,
        reduce: u32,
        (x0, y0, x1, y1): (usize, usize, usize, usize),
        step: usize,
    ) -> Result<Vec<Vec<f32>>> {
        let step = step.max(1);
        let (w, h) = self.size_at(reduce);
        let (x1, y1) = (x1.min(w), y1.min(h));
        if x1 <= x0 || y1 <= y0 {
            return Err(Error::Param("la ventana queda fuera de la imagen".into()));
        }
        let (cols, rows) = ((x1 - x0).div_ceil(step), (y1 - y0).div_ceil(step));
        let s = &self.cs.siz;
        let (ox, oy) = (
            cdiv(i64::from(s.xo), reduce) as usize,
            cdiv(i64::from(s.yo), reduce) as usize,
        );
        let across = s.tiles_across();
        let mut wanted = Vec::new();
        for t in 0..(across * s.tiles_down()) {
            let (p, q) = (t % across, t / across);
            let tx0 = cdiv(i64::from((s.xto + p * s.xt).max(s.xo)), reduce) as usize - ox;
            let tx1 = cdiv(i64::from((s.xto + (p + 1) * s.xt).min(s.x)), reduce) as usize - ox;
            let ty0 = cdiv(i64::from((s.yto + q * s.yt).max(s.yo)), reduce) as usize - oy;
            let ty1 = cdiv(i64::from((s.yto + (q + 1) * s.yt).min(s.y)), reduce) as usize - oy;
            if tx1 > x0 && tx0 < x1 && ty1 > y0 && ty0 < y1 {
                wanted.push(t as usize);
            }
        }
        let all: Vec<usize> = (0..self.bands).collect();
        let pieces: Vec<Result<Piece>> = wanted
            .par_iter()
            .map(|&t| {
                let tile = decode::decode_tile(&self.path, &self.cs, t, reduce, &all)?;
                let (tx0, ty0) = (tile.rect.x0 as usize - ox, tile.rect.y0 as usize - oy);
                let (tw, th) = (tile.rect.w(), tile.rect.h());
                let first = |from: usize, origin: usize| from.saturating_sub(origin).div_ceil(step);
                let (px0, py0) = (first(tx0, x0), first(ty0, y0));
                let px1 = (tx0 + tw)
                    .min(x1)
                    .saturating_sub(x0)
                    .div_ceil(step)
                    .min(cols);
                let py1 = (ty0 + th)
                    .min(y1)
                    .saturating_sub(y0)
                    .div_ceil(step)
                    .min(rows);
                let (pw, ph) = (px1.saturating_sub(px0), py1.saturating_sub(py0));
                let mut planes = Vec::with_capacity(tile.planes.len());
                for src in &tile.planes {
                    let mut v = Vec::with_capacity(pw * ph);
                    for oy2 in py0..py0 + ph {
                        let sy = y0 + oy2 * step - ty0;
                        for ox2 in px0..px0 + pw {
                            v.push(src[sy * tw + x0 + ox2 * step - tx0]);
                        }
                    }
                    planes.push(v);
                }
                Ok(Piece {
                    x: px0,
                    y: py0,
                    w: pw,
                    h: ph,
                    planes,
                })
            })
            .collect();
        let mut bands = vec![vec![f32::NAN; cols * rows]; self.bands];
        for p in pieces {
            let p = p?;
            for (b, plane) in p.planes.iter().enumerate() {
                for y in 0..p.h {
                    let dst = (p.y + y) * cols + p.x;
                    bands[b][dst..dst + p.w].copy_from_slice(&plane[y * p.w..(y + 1) * p.w]);
                }
            }
        }
        Ok(bands)
    }
}

struct Piece {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    planes: Vec<Vec<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100 x 70 RGB image, lossless, in 64 x 64 tiles, three resolutions, RPCL order; the
    /// values were checked against OpenJPEG.
    fn sample() -> Result<Jp2> {
        Jp2::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/small_lossless.jp2"))
    }

    #[test]
    fn a_lossless_tiled_image_decodes_exactly() -> Result<()> {
        let j = sample()?;
        assert_eq!(
            (j.width, j.height, j.bands, j.depth, j.levels),
            (100, 70, 3, 8, 2)
        );
        let b = j.read(0, (0, 0, 100, 70), 1)?;
        let sum: f64 = b.iter().flatten().map(|v| f64::from(*v)).sum();
        assert_eq!(sum, 2_362_541.0);
        assert_eq!([b[0][0], b[1][0], b[2][0]], [0.0, 0.0, 37.0]);
        assert_eq!(
            [
                b[0][69 * 100 + 99],
                b[1][69 * 100 + 99],
                b[2][69 * 100 + 99]
            ],
            [11.0, 207.0, 177.0]
        );
        let at = 35 * 100 + 50;
        assert_eq!([b[0][at], b[1][at], b[2][at]], [135.0, 250.0, 101.0]);
        assert_eq!(b[1].iter().map(|v| f64::from(*v)).sum::<f64>(), 713_456.0);
        Ok(())
    }

    #[test]
    fn a_window_across_tiles_equals_the_same_part_of_the_whole() -> Result<()> {
        let j = sample()?;
        let full = j.read(0, (0, 0, 100, 70), 1)?;
        let win = j.read(0, (50, 30, 90, 66), 1)?;
        for b in 0..3 {
            for y in 0..36 {
                for x in 0..40 {
                    assert_eq!(win[b][y * 40 + x], full[b][(30 + y) * 100 + 50 + x]);
                }
            }
        }
        // thinned: one pixel in three
        let thin = j.read(0, (0, 0, 100, 70), 3)?;
        assert_eq!(thin[0].len(), 34 * 24);
        assert_eq!(thin[0][5 * 34 + 7], full[0][15 * 100 + 21]);
        Ok(())
    }

    #[test]
    fn reductions_halve_the_image() -> Result<()> {
        let j = sample()?;
        assert_eq!(j.size_at(1), (50, 35));
        assert_eq!(j.size_at(2), (25, 18));
        let r = j.read(2, (0, 0, 25, 18), 1)?;
        assert_eq!(r[0].len(), 25 * 18);
        assert!(j.read(3, (0, 0, 1, 1), 1).is_err());
        Ok(())
    }

    #[test]
    fn gml_origin_is_the_centre_of_the_first_pixel() {
        let g = parse_gml(
            "<gml:origin><gml:Point srsName=\"urn:ogc:def:crs:EPSG::32629\"><gml:pos>600005 4400035</gml:pos></gml:Point></gml:origin><gml:offsetVector srsName=\"x\">10 0</gml:offsetVector><gml:offsetVector srsName=\"x\">0 -10</gml:offsetVector>",
        );
        let g = g.map(|g| (g.x0, g.y_top, g.cell, g.epsg));
        assert_eq!(g, Some((600000.0, 4400040.0, 10.0, Some(32629))));
    }

    #[test]
    fn geojp2_reads_the_tags_of_its_little_geotiff() {
        // little-endian TIFF with one IFD of three entries: scale, tie point, geo keys
        let mut t: Vec<u8> = b"II*\0\x08\0\0\0".to_vec();
        t.extend(3u16.to_le_bytes());
        let data_at = 8 + 2 + 3 * 12 + 4;
        let mut entry = |tag: u16, ty: u16, count: u32, val: u32| {
            t.extend(tag.to_le_bytes());
            t.extend(ty.to_le_bytes());
            t.extend(count.to_le_bytes());
            t.extend(val.to_le_bytes());
        };
        entry(33550, 12, 3, data_at as u32);
        entry(33922, 12, 6, data_at as u32 + 24);
        entry(34735, 3, 8, data_at as u32 + 72);
        t.extend(0u32.to_le_bytes());
        for v in [10.0f64, 10.0, 0.0, 0.0, 0.0, 0.0, 500000.0, 4100000.0, 0.0] {
            t.extend(v.to_le_bytes());
        }
        for v in [1u16, 1, 0, 1, 3072, 0, 1, 25830] {
            t.extend(v.to_le_bytes());
        }
        let g = parse_geotiff(&t).map(|g| (g.x0, g.y_top, g.cell, g.epsg));
        assert_eq!(g, Some((500000.0, 4100000.0, 10.0, Some(25830))));
    }

    #[test]
    fn other_files_are_refused_cleanly() {
        let p = std::env::temp_dir().join(format!("easylidar-notjp2-{}", std::process::id()));
        let _ = std::fs::write(&p, b"this is not a jpeg 2000 file at all");
        assert!(Jp2::open(&p).is_err());
        let _ = std::fs::remove_file(p);
    }

    /// Damaged files must give an error or garbage, never a panic or a hang.
    #[test]
    fn damaged_files_do_not_panic() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/small_lossless.jp2");
        let Ok(orig) = std::fs::read(&src) else {
            return;
        };
        let p = std::env::temp_dir().join(format!("easylidar-fuzz-{}.jp2", std::process::id()));
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..400 {
            let mut b = orig.clone();
            if round % 4 == 0 {
                b.truncate((next() as usize) % b.len());
            } else {
                for _ in 0..1 + round % 5 {
                    let i = (next() as usize) % b.len();
                    b[i] = next() as u8;
                }
            }
            let _ = std::fs::write(&p, &b);
            let r = std::panic::catch_unwind(|| {
                if let Ok(j) = Jp2::open(&p) {
                    let _ = j.read(0, (0, 0, j.width as usize, j.height as usize), 1);
                }
            });
            assert!(r.is_ok(), "panic with round {round}");
        }
        let _ = std::fs::remove_file(p);
    }
}
