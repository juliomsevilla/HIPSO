//! The JPEG 2000 codestream's headers (T.800 Annex A): image and tile geometry, coding and
//! quantisation parameters, and where the data of each tile lies in the file.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

use crate::{Error, Result};

fn bad(what: &str) -> Error {
    Error::Format(format!("JPEG 2000: {what}"))
}

#[derive(Clone, Debug)]
pub struct CompSiz {
    pub depth: u8,
    pub signed: bool,
    pub dx: u32,
    pub dy: u32,
}

#[derive(Clone, Debug)]
pub struct Siz {
    pub x: u32,
    pub y: u32,
    pub xo: u32,
    pub yo: u32,
    pub xt: u32,
    pub yt: u32,
    pub xto: u32,
    pub yto: u32,
    pub comps: Vec<CompSiz>,
}

impl Siz {
    pub fn tiles_across(&self) -> u32 {
        self.x.saturating_sub(self.xto).div_ceil(self.xt.max(1))
    }
    pub fn tiles_down(&self) -> u32 {
        self.y.saturating_sub(self.yto).div_ceil(self.yt.max(1))
    }
}

/// What the `COD` marker says for the whole tile.
#[derive(Clone, Copy, Debug)]
pub struct CodParams {
    pub prog: u8,
    pub layers: u16,
    pub mct: u8,
    pub sop: bool,
    pub eph: bool,
}

/// Coding style of a component (`COD` or `COC`).
#[derive(Clone, Debug)]
pub struct CompCod {
    pub levels: u8,
    pub xcb: u8,
    pub ycb: u8,
    pub style: u8,
    pub reversible: bool,
    /// Precinct size exponents per resolution, lowest first; 15 when not set.
    pub precincts: Vec<(u8, u8)>,
}

/// Quantisation (`QCD` or `QCC`).
#[derive(Clone, Debug)]
pub struct Quant {
    /// 0 none, 1 scalar derived, 2 scalar expounded.
    pub style: u8,
    pub guard: u8,
    /// Exponent and mantissa per subband (one only for the derived style).
    pub steps: Vec<(u8, u16)>,
}

/// The parameters of the main header or of a tile-part header.
#[derive(Clone, Debug, Default)]
pub struct Params {
    pub cod: Option<(CodParams, CompCod)>,
    pub coc: HashMap<usize, CompCod>,
    pub qcd: Option<Quant>,
    pub qcc: HashMap<usize, Quant>,
    pub rgn: HashMap<usize, u8>,
    pub poc: bool,
    pub packed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct TilePart {
    /// Where the packet data starts in the file and how long it is.
    pub offset: u64,
    pub len: u64,
}

#[derive(Clone, Debug, Default)]
pub struct TileHeader {
    pub parts: Vec<TilePart>,
    pub params: Params,
}

#[derive(Clone, Debug)]
pub struct Codestream {
    pub siz: Siz,
    pub main: Params,
    pub tiles: Vec<TileHeader>,
}

fn be16(b: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(
        b.get(at..at + 2)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| bad("encabezado cortado"))?,
    ))
}

fn be32(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        b.get(at..at + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| bad("encabezado cortado"))?,
    ))
}

fn parse_siz(b: &[u8]) -> Result<Siz> {
    let n = usize::from(be16(b, 34)?);
    if n == 0 || n > 16384 {
        return Err(bad("número de componentes no válido"));
    }
    let mut comps = Vec::with_capacity(n);
    for k in 0..n {
        let s = *b.get(36 + 3 * k).ok_or_else(|| bad("SIZ cortado"))?;
        comps.push(CompSiz {
            depth: (s & 0x7F) + 1,
            signed: s & 0x80 != 0,
            dx: u32::from(*b.get(37 + 3 * k).ok_or_else(|| bad("SIZ cortado"))?),
            dy: u32::from(*b.get(38 + 3 * k).ok_or_else(|| bad("SIZ cortado"))?),
        });
    }
    let siz = Siz {
        x: be32(b, 2)?,
        y: be32(b, 6)?,
        xo: be32(b, 10)?,
        yo: be32(b, 14)?,
        xt: be32(b, 18)?,
        yt: be32(b, 22)?,
        xto: be32(b, 26)?,
        yto: be32(b, 30)?,
        comps,
    };
    if siz.xt == 0 || siz.yt == 0 || siz.x <= siz.xo || siz.y <= siz.yo {
        return Err(bad("dimensiones no válidas"));
    }
    // the tile grid starts at or before the image and its first tile reaches into it (T.800 A.5.1)
    if siz.xto > siz.xo
        || siz.yto > siz.yo
        || u64::from(siz.xto) + u64::from(siz.xt) <= u64::from(siz.xo)
        || u64::from(siz.yto) + u64::from(siz.yt) <= u64::from(siz.yo)
    {
        return Err(bad("la cuadrícula de teselas no cubre la imagen"));
    }
    if siz
        .comps
        .iter()
        .any(|c| c.dx == 0 || c.dy == 0 || c.depth > 31)
    {
        return Err(bad("componentes no válidos"));
    }
    if u64::from(siz.tiles_across()) * u64::from(siz.tiles_down()) > 65535 {
        return Err(bad("demasiados teselas"));
    }
    Ok(siz)
}

fn parse_comp_cod(b: &[u8], precincts_defined: bool) -> Result<CompCod> {
    let levels = *b.first().ok_or_else(|| bad("COD cortado"))?;
    if levels > 32 {
        return Err(bad("demasiados niveles de descomposición"));
    }
    let (xcb, ycb) = (
        b.get(1).copied().ok_or_else(|| bad("COD cortado"))? & 0x0F,
        b.get(2).copied().ok_or_else(|| bad("COD cortado"))? & 0x0F,
    );
    if xcb + 2 > 10 || ycb + 2 > 10 || xcb + ycb + 4 > 12 {
        return Err(bad("tamaño de bloque de código no válido"));
    }
    let style = *b.get(3).ok_or_else(|| bad("COD cortado"))?;
    let transform = *b.get(4).ok_or_else(|| bad("COD cortado"))?;
    let mut precincts = Vec::new();
    for r in 0..=usize::from(levels) {
        precincts.push(if precincts_defined {
            let p = *b.get(5 + r).ok_or_else(|| bad("COD cortado"))?;
            (p & 0x0F, p >> 4)
        } else {
            (15, 15)
        });
    }
    Ok(CompCod {
        levels,
        xcb: xcb + 2,
        ycb: ycb + 2,
        style,
        reversible: transform == 1,
        precincts,
    })
}

fn parse_quant(b: &[u8]) -> Result<Quant> {
    let s = *b.first().ok_or_else(|| bad("QCD cortado"))?;
    let (style, guard) = (s & 0x1F, s >> 5);
    let mut steps = Vec::new();
    match style {
        0 => {
            for &v in &b[1..] {
                steps.push((v >> 3, 0));
            }
        }
        1 | 2 => {
            for c in b[1..].as_chunks::<2>().0 {
                let v = u16::from_be_bytes([c[0], c[1]]);
                steps.push(((v >> 11) as u8, v & 0x7FF));
            }
        }
        _ => return Err(bad("estilo de cuantización desconocido")),
    }
    if steps.is_empty() {
        return Err(bad("QCD vacío"));
    }
    Ok(Quant {
        style,
        guard,
        steps,
    })
}

/// Reads one marker segment body (the bytes after its length) into `p`.
fn parse_segment(marker: u16, body: &[u8], ncomps: usize, p: &mut Params) -> Result<()> {
    let comp_index = |b: &[u8]| -> Result<(usize, usize)> {
        if ncomps < 257 {
            Ok((
                usize::from(*b.first().ok_or_else(|| bad("segmento cortado"))?),
                1,
            ))
        } else {
            Ok((usize::from(be16(b, 0)?), 2))
        }
    };
    match marker {
        0xFF52 => {
            let scod = *body.first().ok_or_else(|| bad("COD cortado"))?;
            let cod = CodParams {
                prog: *body.get(1).ok_or_else(|| bad("COD cortado"))?,
                layers: be16(body, 2)?,
                mct: *body.get(4).ok_or_else(|| bad("COD cortado"))?,
                sop: scod & 2 != 0,
                eph: scod & 4 != 0,
            };
            if cod.layers == 0 {
                return Err(bad("cero capas"));
            }
            let cc = parse_comp_cod(&body[5..], scod & 1 != 0)?;
            p.cod = Some((cod, cc));
        }
        0xFF53 => {
            let (c, n) = comp_index(body)?;
            let scoc = *body.get(n).ok_or_else(|| bad("COC cortado"))?;
            p.coc
                .insert(c, parse_comp_cod(&body[n + 1..], scoc & 1 != 0)?);
        }
        0xFF5C => p.qcd = Some(parse_quant(body)?),
        0xFF5D => {
            let (c, n) = comp_index(body)?;
            p.qcc.insert(c, parse_quant(&body[n..])?);
        }
        0xFF5E => {
            let (c, n) = comp_index(body)?;
            let shift = *body.get(n + 1).ok_or_else(|| bad("RGN cortado"))?;
            p.rgn.insert(c, shift);
        }
        0xFF5F => p.poc = true,
        0xFF60 | 0xFF61 => p.packed = true,
        // tile-part lengths, packet lengths, component registration, comments
        0xFF55 | 0xFF57 | 0xFF58 | 0xFF63 | 0xFF64 => {}
        _ => {}
    }
    Ok(())
}

fn read_u16(r: &mut impl Read) -> Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)
        .map_err(|_| bad("fin de archivo inesperado"))?;
    Ok(u16::from_be_bytes(b))
}

/// Reads the headers of the codestream that starts at `start` and ends at `end` (exclusive), and
/// finds the tile-parts.
pub fn scan(file: &File, start: u64, end: u64) -> Result<Codestream> {
    let mut r = BufReader::with_capacity(1 << 16, file.try_clone()?);
    r.seek(SeekFrom::Start(start))?;
    if read_u16(&mut r)? != 0xFF4F {
        return Err(bad("no empieza con la marca SOC"));
    }
    let mut siz: Option<Siz> = None;
    let mut main = Params::default();
    // the main header, up to the first tile-part
    let mut marker;
    loop {
        marker = read_u16(&mut r)?;
        if marker == 0xFF90 || marker == 0xFFD9 {
            break;
        }
        if marker & 0xFF00 != 0xFF00 {
            return Err(bad("marca no válida en el encabezado principal"));
        }
        let len = usize::from(read_u16(&mut r)?);
        if len < 2 {
            return Err(bad("segmento demasiado corto"));
        }
        let mut body = vec![0u8; len - 2];
        r.read_exact(&mut body)
            .map_err(|_| bad("encabezado cortado"))?;
        if marker == 0xFF51 {
            siz = Some(parse_siz(&body)?);
        } else {
            let n = siz.as_ref().map_or(0, |s| s.comps.len());
            parse_segment(marker, &body, n, &mut main)?;
        }
    }
    let siz = siz.ok_or_else(|| bad("falta el marcador SIZ"))?;
    if main.cod.is_none() || main.qcd.is_none() {
        return Err(bad("faltan los marcadores COD o QCD"));
    }
    let ntiles = (siz.tiles_across() * siz.tiles_down()) as usize;
    let mut tiles = vec![TileHeader::default(); ntiles];
    // the tile-parts
    while marker == 0xFF90 {
        let sot_at = r.stream_position()? - 2;
        let (lsot, isot, psot) = (read_u16(&mut r)?, read_u16(&mut r)?, {
            let hi = u32::from(read_u16(&mut r)?);
            (hi << 16) | u32::from(read_u16(&mut r)?)
        });
        let mut rest = [0u8; 2];
        r.read_exact(&mut rest)?;
        if lsot != 10 || usize::from(isot) >= ntiles {
            return Err(bad("tesela fuera de la cuadrícula"));
        }
        let tile = &mut tiles[usize::from(isot)];
        // the tile-part header, up to SOD
        loop {
            let m = read_u16(&mut r)?;
            if m == 0xFF93 {
                break;
            }
            let len = usize::from(read_u16(&mut r)?);
            if len < 2 {
                return Err(bad("segmento demasiado corto"));
            }
            let mut body = vec![0u8; len - 2];
            r.read_exact(&mut body)
                .map_err(|_| bad("encabezado de tesela cortado"))?;
            parse_segment(m, &body, siz.comps.len(), &mut tile.params)?;
        }
        let data_at = r.stream_position()?;
        let part_end = if psot == 0 {
            end.saturating_sub(2).max(data_at)
        } else {
            (sot_at + u64::from(psot)).min(end)
        };
        tile.parts.push(TilePart {
            offset: data_at,
            len: part_end.saturating_sub(data_at),
        });
        r.seek(SeekFrom::Start(part_end))?;
        let mut m = [0u8; 2];
        if r.read_exact(&mut m).is_err() {
            break;
        }
        marker = u16::from_be_bytes(m);
        if marker != 0xFF90 {
            break;
        }
    }
    Ok(Codestream { siz, main, tiles })
}

/// The effective parameters of a tile: the main header's, over which those of the tile-part
/// headers take precedence (a tile's `COC` over its `COD` over the main `COC` over the main `COD`).
pub struct Effective<'a> {
    pub main: &'a Params,
    pub tile: &'a Params,
}

impl Effective<'_> {
    pub fn cod(&self) -> CodParams {
        self.tile
            .cod
            .as_ref()
            .or(self.main.cod.as_ref())
            .map(|c| c.0)
            .unwrap_or(CodParams {
                prog: 0,
                layers: 1,
                mct: 0,
                sop: false,
                eph: false,
            })
    }

    pub fn comp_cod(&self, c: usize) -> Option<&CompCod> {
        self.tile
            .coc
            .get(&c)
            .or_else(|| self.tile.cod.as_ref().map(|d| &d.1))
            .or_else(|| self.main.coc.get(&c))
            .or_else(|| self.main.cod.as_ref().map(|d| &d.1))
    }

    pub fn quant(&self, c: usize) -> Option<&Quant> {
        self.tile
            .qcc
            .get(&c)
            .or(self.tile.qcd.as_ref())
            .or_else(|| self.main.qcc.get(&c))
            .or(self.main.qcd.as_ref())
    }

    pub fn roi_shift(&self, c: usize) -> u32 {
        u32::from(
            self.tile
                .rgn
                .get(&c)
                .or_else(|| self.main.rgn.get(&c))
                .copied()
                .unwrap_or(0),
        )
    }
}
