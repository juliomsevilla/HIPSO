//! Tier-2 of JPEG 2000 (T.800 Annex B): the geometry of a tile (resolutions, subbands, precincts,
//! code-blocks), the packet sequence of each progression order and the packet headers, which say
//! which code-blocks contribute to a layer and how many bytes of each.

use super::codestream::{CodParams, CompCod, Quant, Siz};
use super::dwt::Rect;
use super::tier1::{BYPASS, Segment, TERMALL};
use crate::{Error, Result};

fn bad(what: &str) -> Error {
    Error::Format(format!("JPEG 2000: {what}"))
}

/// Ceiling of `a / 2^s`, also for negative `a`.
#[inline]
pub fn cdiv(a: i64, s: u32) -> i64 {
    (a + (1i64 << s) - 1) >> s
}

// ------------------------------------------------------------------------------------ bit reader

/// Reads the bits of a packet header: after a `0xFF` byte the next one carries only 7 bits.
pub struct Bits<'a> {
    d: &'a [u8],
    pub pos: usize,
    cur: u8,
    left: u8,
}

impl<'a> Bits<'a> {
    pub fn new(d: &'a [u8], pos: usize) -> Self {
        Bits {
            d,
            pos,
            cur: 0,
            left: 0,
        }
    }

    pub fn bit(&mut self) -> Result<u32> {
        if self.left == 0 {
            let prev = self.cur;
            self.cur = *self.d.get(self.pos).ok_or_else(|| bad("paquete cortado"))?;
            self.pos += 1;
            self.left = if prev == 0xFF { 7 } else { 8 };
        }
        self.left -= 1;
        Ok(u32::from((self.cur >> self.left) & 1))
    }

    pub fn bits(&mut self, n: u32) -> Result<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }

    /// Ends the header at a byte boundary (a final `0xFF` is followed by a stuffed byte).
    pub fn align(&mut self) {
        self.left = 0;
        if self.cur == 0xFF {
            self.pos += 1;
        }
        self.cur = 0;
    }
}

// -------------------------------------------------------------------------------------- tag tree

const NONE: usize = usize::MAX;

#[derive(Clone, Copy)]
struct Node {
    value: u32,
    low: u32,
}

/// A tag tree (B.10.2): codes a grid of non-negative numbers incrementally.
pub struct TagTree {
    nodes: Vec<Node>,
    parent: Vec<usize>,
}

impl TagTree {
    pub fn new(w: usize, h: usize) -> Self {
        let mut dims = vec![(w, h)];
        while let Some(&(lw, lh)) = dims.last() {
            if lw <= 1 && lh <= 1 {
                break;
            }
            dims.push((lw.div_ceil(2), lh.div_ceil(2)));
        }
        let total: usize = dims.iter().map(|d| d.0 * d.1).sum();
        let mut parent = vec![NONE; total];
        let mut offset = 0;
        for l in 0..dims.len().saturating_sub(1) {
            let (lw, lh) = dims[l];
            let next = offset + lw * lh;
            for y in 0..lh {
                for x in 0..lw {
                    parent[offset + y * lw + x] = next + (y / 2) * dims[l + 1].0 + x / 2;
                }
            }
            offset = next;
        }
        TagTree {
            nodes: vec![
                Node {
                    value: u32::MAX,
                    low: 0
                };
                total
            ],
            parent,
        }
    }

    /// Whether the value of `leaf` is below `threshold`, reading what the stream says about it.
    pub fn decode(&mut self, bits: &mut Bits, leaf: usize, threshold: u32) -> Result<bool> {
        let mut path = [0usize; 40];
        let mut n = 0;
        let mut i = leaf;
        loop {
            if n == path.len() {
                return Err(bad("árbol de etiquetas demasiado profundo"));
            }
            path[n] = i;
            n += 1;
            if self.parent[i] == NONE {
                break;
            }
            i = self.parent[i];
        }
        let mut low = 0;
        for &k in path[..n].iter().rev() {
            let node = &mut self.nodes[k];
            if low > node.low {
                node.low = low;
            } else {
                low = node.low;
            }
            while low < threshold && low < node.value {
                if bits.bit()? == 1 {
                    node.value = low;
                } else {
                    low += 1;
                }
            }
            node.low = low;
        }
        Ok(self.nodes[leaf].value < threshold)
    }
}

// ---------------------------------------------------------------------------------- tile geometry

pub struct CodeBlock {
    pub rect: Rect,
    pub included: bool,
    pub zero_planes: u32,
    pub lblock: u32,
    pub segs: Vec<Segment>,
}

pub struct PrecBand {
    pub blocks: Vec<CodeBlock>,
    incl: TagTree,
    zbp: TagTree,
}

pub struct Band {
    /// 0 LL, 1 HL, 2 LH, 3 HH.
    pub orient: u8,
    pub rect: Rect,
    /// Bit-planes the band can have (`Mb`).
    pub numbps: u32,
    /// Quantisation step (1 when reversible).
    pub step: f32,
    pub precs: Vec<PrecBand>,
}

pub struct Resolution {
    pub rect: Rect,
    pub ppx: u32,
    pub ppy: u32,
    pub npw: usize,
    pub nph: usize,
    pub bands: Vec<Band>,
}

impl Resolution {
    pub fn precincts(&self) -> usize {
        self.npw * self.nph
    }
}

pub struct TileComp {
    pub cod: CompCod,
    pub res: Vec<Resolution>,
    pub roishift: u32,
}

/// Builds the geometry of one component of a tile; `tile` is the tile on the reference grid.
pub fn build_comp(
    siz: &Siz,
    c: usize,
    tile: Rect,
    cod: &CompCod,
    quant: &Quant,
    roishift: u32,
) -> Result<TileComp> {
    let comp = &siz.comps[c];
    let (dx, dy) = (i64::from(comp.dx), i64::from(comp.dy));
    let rect = Rect {
        x0: (tile.x0 + dx - 1) / dx,
        y0: (tile.y0 + dy - 1) / dy,
        x1: (tile.x1 + dx - 1) / dx,
        y1: (tile.y1 + dy - 1) / dy,
    };
    let nl = u32::from(cod.levels);
    let mut res = Vec::new();
    for r in 0..=nl {
        let sh = nl - r;
        let rr = Rect {
            x0: cdiv(rect.x0, sh),
            y0: cdiv(rect.y0, sh),
            x1: cdiv(rect.x1, sh),
            y1: cdiv(rect.y1, sh),
        };
        let (ppx, ppy) = {
            let p = cod.precincts[r as usize];
            (u32::from(p.0), u32::from(p.1))
        };
        if r > 0 && (ppx == 0 || ppy == 0) {
            return Err(bad("precinto de tamaño 1 en una resolución alta"));
        }
        let npw = if rr.x1 > rr.x0 {
            (cdiv(rr.x1, ppx) - (rr.x0 >> ppx)) as usize
        } else {
            0
        };
        let nph = if rr.y1 > rr.y0 {
            (cdiv(rr.y1, ppy) - (rr.y0 >> ppy)) as usize
        } else {
            0
        };
        let np = npw * nph;
        let orients: &[u8] = if r == 0 { &[0] } else { &[1, 2, 3] };
        let nb = if r == 0 { nl } else { nl - r + 1 };
        let mut bands = Vec::new();
        for &o in orients {
            let half = |bit: bool| -> i64 { if bit { 1i64 << (nb - 1) } else { 0 } };
            let (ox, oy) = (half(o == 1 || o == 3), half(o == 2 || o == 3));
            let br = if r == 0 {
                rr
            } else {
                Rect {
                    x0: cdiv(rect.x0 - ox, nb),
                    y0: cdiv(rect.y0 - oy, nb),
                    x1: cdiv(rect.x1 - ox, nb),
                    y1: cdiv(rect.y1 - oy, nb),
                }
            };
            // quantisation of this band
            let idx = if r == 0 {
                0
            } else {
                3 * (r as usize - 1) + o as usize
            };
            let (exp, mant) = if quant.style == 1 {
                let (e, m) = quant.steps[0];
                (
                    (i32::from(e) - if r == 0 { 0 } else { r as i32 - 1 }).max(0),
                    m,
                )
            } else {
                let (e, m) = *quant
                    .steps
                    .get(idx)
                    .ok_or_else(|| bad("faltan pasos de cuantización"))?;
                (i32::from(e), m)
            };
            let numbps = (i32::from(quant.guard) + exp - 1).max(0) as u32;
            let step = if cod.reversible {
                1.0
            } else {
                let gain = match o {
                    0 => 0,
                    3 => 2,
                    _ => 1,
                };
                let rb = i32::from(comp.depth) + gain;
                2f32.powi(rb - exp) * (1.0 + f32::from(mant) / 2048.0)
            };
            // precincts of the band and their code-blocks
            let (cx, cy) = if r == 0 {
                (u32::from(cod.xcb).min(ppx), u32::from(cod.ycb).min(ppy))
            } else {
                (
                    u32::from(cod.xcb).min(ppx - 1),
                    u32::from(cod.ycb).min(ppy - 1),
                )
            };
            let pshift_x = if r == 0 { ppx } else { ppx - 1 };
            let pshift_y = if r == 0 { ppy } else { ppy - 1 };
            let mut precs = Vec::with_capacity(np);
            for py in 0..nph {
                for px in 0..npw {
                    let px0 = ((rr.x0 >> ppx) + px as i64) << pshift_x;
                    let py0 = ((rr.y0 >> ppy) + py as i64) << pshift_y;
                    let pr = Rect {
                        x0: px0.max(br.x0),
                        y0: py0.max(br.y0),
                        x1: (px0 + (1i64 << pshift_x)).min(br.x1),
                        y1: (py0 + (1i64 << pshift_y)).min(br.y1),
                    };
                    let mut blocks = Vec::new();
                    let (mut cbw, mut cbh) = (0usize, 0usize);
                    if pr.x1 > pr.x0 && pr.y1 > pr.y0 {
                        let (bx0, bx1) = (pr.x0 >> cx, cdiv(pr.x1, cx));
                        let (by0, by1) = (pr.y0 >> cy, cdiv(pr.y1, cy));
                        cbw = (bx1 - bx0) as usize;
                        cbh = (by1 - by0) as usize;
                        for by in by0..by1 {
                            for bx in bx0..bx1 {
                                blocks.push(CodeBlock {
                                    rect: Rect {
                                        x0: (bx << cx).max(pr.x0),
                                        y0: (by << cy).max(pr.y0),
                                        x1: ((bx + 1) << cx).min(pr.x1),
                                        y1: ((by + 1) << cy).min(pr.y1),
                                    },
                                    included: false,
                                    zero_planes: 0,
                                    lblock: 3,
                                    segs: Vec::new(),
                                });
                            }
                        }
                    }
                    precs.push(PrecBand {
                        incl: TagTree::new(cbw, cbh),
                        zbp: TagTree::new(cbw, cbh),
                        blocks,
                    });
                }
            }
            bands.push(Band {
                orient: o,
                rect: br,
                numbps,
                step,
                precs,
            });
        }
        res.push(Resolution {
            rect: rr,
            ppx,
            ppy,
            npw,
            nph,
            bands,
        });
    }
    Ok(TileComp {
        cod: cod.clone(),
        res,
        roishift,
    })
}

// -------------------------------------------------------------------------------- packet sequence

#[derive(Clone, Copy)]
pub struct Packet {
    pub comp: usize,
    pub res: usize,
    pub prec: usize,
    pub layer: usize,
}

/// The packets of a tile in the order of its progression (B.12).
pub fn packet_order(tile: Rect, tcs: &[TileComp], prog: u8, layers: usize) -> Result<Vec<Packet>> {
    let max_res = tcs.iter().map(|t| t.res.len()).max().unwrap_or(0);
    let mut out = Vec::new();
    let np = |c: usize, r: usize| tcs[c].res.get(r).map_or(0, Resolution::precincts);
    match prog {
        0 => {
            for l in 0..layers {
                for r in 0..max_res {
                    for c in 0..tcs.len() {
                        for p in 0..np(c, r) {
                            out.push(Packet {
                                comp: c,
                                res: r,
                                prec: p,
                                layer: l,
                            });
                        }
                    }
                }
            }
        }
        1 => {
            for r in 0..max_res {
                for l in 0..layers {
                    for c in 0..tcs.len() {
                        for p in 0..np(c, r) {
                            out.push(Packet {
                                comp: c,
                                res: r,
                                prec: p,
                                layer: l,
                            });
                        }
                    }
                }
            }
        }
        2..=4 => {
            // steps of the position loops: the finest precinct spacing on the reference grid
            let (mut sx, mut sy) = (i64::MAX, i64::MAX);
            for t in tcs {
                let nl = t.res.len() as u32 - 1;
                for (r, rs) in t.res.iter().enumerate() {
                    sx = sx.min(1i64 << (rs.ppx + nl - r as u32).min(40));
                    sy = sy.min(1i64 << (rs.ppy + nl - r as u32).min(40));
                }
            }
            let precinct_at = |c: usize, r: usize, x: i64, y: i64| -> Option<usize> {
                let t = &tcs[c];
                let rs = t.res.get(r)?;
                if rs.precincts() == 0 {
                    return None;
                }
                let sh = t.res.len() as u32 - 1 - r as u32;
                let (stx, sty) = (1i64 << (rs.ppx + sh), 1i64 << (rs.ppy + sh));
                let okx = x % stx == 0 || (x == tile.x0 && (rs.rect.x0 << sh) % stx != 0);
                let oky = y % sty == 0 || (y == tile.y0 && (rs.rect.y0 << sh) % sty != 0);
                if !(okx && oky) {
                    return None;
                }
                let kx = (cdiv(x, sh) >> rs.ppx) - (rs.rect.x0 >> rs.ppx);
                let ky = (cdiv(y, sh) >> rs.ppy) - (rs.rect.y0 >> rs.ppy);
                if kx < 0 || ky < 0 || kx as usize >= rs.npw || ky as usize >= rs.nph {
                    return None;
                }
                Some(ky as usize * rs.npw + kx as usize)
            };
            let positions = |f: &mut dyn FnMut(i64, i64)| {
                let mut y = tile.y0;
                while y < tile.y1 {
                    let mut x = tile.x0;
                    while x < tile.x1 {
                        f(x, y);
                        x = (x / sx + 1) * sx;
                    }
                    y = (y / sy + 1) * sy;
                }
            };
            let push = |out: &mut Vec<Packet>, c: usize, r: usize, p: usize| {
                for l in 0..layers {
                    out.push(Packet {
                        comp: c,
                        res: r,
                        prec: p,
                        layer: l,
                    });
                }
            };
            match prog {
                2 => {
                    for r in 0..max_res {
                        positions(&mut |x, y| {
                            for c in 0..tcs.len() {
                                if let Some(p) = precinct_at(c, r, x, y) {
                                    push(&mut out, c, r, p);
                                }
                            }
                        });
                    }
                }
                3 => positions(&mut |x, y| {
                    for (c, t) in tcs.iter().enumerate() {
                        for r in 0..t.res.len() {
                            if let Some(p) = precinct_at(c, r, x, y) {
                                push(&mut out, c, r, p);
                            }
                        }
                    }
                }),
                _ => {
                    for (c, t) in tcs.iter().enumerate() {
                        positions(&mut |x, y| {
                            for r in 0..t.res.len() {
                                if let Some(p) = precinct_at(c, r, x, y) {
                                    push(&mut out, c, r, p);
                                }
                            }
                        });
                    }
                }
            }
        }
        _ => return Err(bad("orden de progresión desconocido")),
    }
    Ok(out)
}

// ------------------------------------------------------------------------------ packet headers

fn read_passes(b: &mut Bits) -> Result<u32> {
    if b.bit()? == 0 {
        return Ok(1);
    }
    if b.bit()? == 0 {
        return Ok(2);
    }
    let v = b.bits(2)?;
    if v < 3 {
        return Ok(3 + v);
    }
    let v = b.bits(5)?;
    if v < 31 {
        return Ok(6 + v);
    }
    Ok(37 + b.bits(7)?)
}

/// Passes a codeword segment can hold: the coding style decides where the coder is terminated.
fn segment_capacity(style: u8, index: usize, previous: u32) -> u32 {
    if style & TERMALL != 0 {
        1
    } else if style & BYPASS != 0 {
        if index == 0 {
            10
        } else if previous == 1 || previous == 10 {
            2
        } else {
            1
        }
    } else {
        109
    }
}

/// Reads the packet at `*pos` of `data` into the code-blocks of `tc`.
pub fn read_packet(
    data: &[u8],
    pos: &mut usize,
    tc: &mut TileComp,
    pk: Packet,
    cod: &CodParams,
) -> Result<()> {
    if cod.sop && data.get(*pos..*pos + 2) == Some(&[0xFF, 0x91]) {
        *pos += 6;
    }
    let style = tc.cod.style;
    let mut bits = Bits::new(data, *pos);
    let mut pending: Vec<(usize, usize, usize, usize)> = Vec::new();
    if bits.bit()? == 1 {
        let rs = &mut tc.res[pk.res];
        for (bi, band) in rs.bands.iter_mut().enumerate() {
            let Some(pb) = band.precs.get_mut(pk.prec) else {
                continue;
            };
            for k in 0..pb.blocks.len() {
                let included = if pb.blocks[k].included {
                    bits.bit()? == 1
                } else {
                    pb.incl.decode(&mut bits, k, pk.layer as u32 + 1)?
                };
                if !included {
                    continue;
                }
                if !pb.blocks[k].included {
                    let mut i = 1;
                    while !pb.zbp.decode(&mut bits, k, i)? {
                        i += 1;
                        if i > 64 {
                            return Err(bad("planos de bits no válidos"));
                        }
                    }
                    pb.blocks[k].zero_planes = i - 1;
                    pb.blocks[k].included = true;
                }
                let mut left = read_passes(&mut bits)?;
                while bits.bit()? == 1 {
                    pb.blocks[k].lblock += 1;
                    if pb.blocks[k].lblock > 32 {
                        return Err(bad("longitud de segmento no válida"));
                    }
                }
                let cb = &mut pb.blocks[k];
                while left > 0 {
                    let need_new = match cb.segs.last() {
                        None => true,
                        Some(s) => s.passes >= s.max_passes,
                    };
                    if need_new {
                        let prev = cb.segs.last().map_or(0, |s| s.max_passes);
                        cb.segs.push(Segment {
                            data: Vec::new(),
                            passes: 0,
                            max_passes: segment_capacity(style, cb.segs.len(), prev),
                        });
                    }
                    let si = cb.segs.len() - 1;
                    let seg = &mut cb.segs[si];
                    let n = left.min(seg.max_passes - seg.passes);
                    let nbits = cb.lblock + (31 - n.leading_zeros());
                    if nbits > 32 {
                        return Err(bad("longitud de segmento no válida"));
                    }
                    let len = bits.bits(nbits)? as usize;
                    seg.passes += n;
                    left -= n;
                    pending.push((bi, k, si, len));
                }
            }
        }
    }
    bits.align();
    *pos = bits.pos;
    if cod.eph && data.get(*pos..*pos + 2) == Some(&[0xFF, 0x92]) {
        *pos += 2;
    }
    let rs = &mut tc.res[pk.res];
    for (bi, k, si, len) in pending {
        let end = (*pos + len).min(data.len());
        let chunk = data.get(*pos..end).unwrap_or(&[]);
        if let Some(pb) = rs.bands[bi].precs.get_mut(pk.prec) {
            pb.blocks[k].segs[si].data.extend_from_slice(chunk);
        }
        *pos = end;
    }
    Ok(())
}
