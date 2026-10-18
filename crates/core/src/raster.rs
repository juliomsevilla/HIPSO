//! Passive remote sensing next to the LiDAR: multiband images (aerial photos, satellite scenes),
//! colour composites and spectral indices (NDVI, NDWI, EVI, SAVI…) from band expressions. The
//! images serve as base maps for the clouds and the indices as rasters to compare with the CHM.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

use crate::grid::Grid;
use crate::{Error, Result};

/// A georeferenced image with any number of bands, as floats; row 0 is the northernmost.
#[derive(Clone, Debug)]
pub struct Image {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    pub bands: Vec<Vec<f32>>,
}

fn tiff_err(e: tiff::TiffError) -> Error {
    Error::Format(format!("TIFF: {e}"))
}

/// Reads a GeoTIFF of any sample type (8 or 16 bits, integer or float) with interleaved bands.
pub fn read_image(path: &Path) -> Result<Image> {
    let mut d = Decoder::new(BufReader::new(File::open(path)?)).map_err(tiff_err)?;
    let (w, h) = d.dimensions().map_err(tiff_err)?;
    let scale = d
        .get_tag_f64_vec(Tag::ModelPixelScaleTag)
        .map_err(tiff_err)?;
    let tie = d.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(tiff_err)?;
    let (Some(&sx), Some(&sy), Some(&tx), Some(&ty)) =
        (scale.first(), scale.get(1), tie.get(3), tie.get(4))
    else {
        return Err(Error::Format(format!(
            "{}: not a GeoTIFF (no pixel scale or tie point)",
            path.display()
        )));
    };
    let nodata = d
        .get_tag_ascii_string(Tag::GdalNodata)
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok());
    let (cols, rows) = (w as usize, h as usize);
    // band-sequential files (planar configuration 2) come as one plane after another
    let planar = d.get_tag_u32(Tag::PlanarConfiguration).unwrap_or(1) == 2;
    let mut buf = DecodingResult::U8(Vec::new());
    d.read_image_to_buffer(&mut buf).map_err(tiff_err)?;
    let raw: Vec<f32> = match buf {
        DecodingResult::U8(v) => v.into_iter().map(f32::from).collect(),
        DecodingResult::U16(v) => v.into_iter().map(f32::from).collect(),
        DecodingResult::U32(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I16(v) => v.into_iter().map(f32::from).collect(),
        DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::F32(v) => v,
        DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        _ => {
            return Err(Error::Unsupported(format!(
                "{}: this sample type is not read",
                path.display()
            )));
        }
    };
    let n = cols * rows;
    if n == 0 || !raw.len().is_multiple_of(n) {
        return Err(Error::Format(format!(
            "{}: the pixel data does not fit {cols} x {rows}",
            path.display()
        )));
    }
    let nb = raw.len() / n;
    let mut bands = vec![vec![0f32; n]; nb];
    let clean = |v: f32| {
        if Some(v) == nodata || !v.is_finite() {
            f32::NAN
        } else {
            v
        }
    };
    if planar {
        for (b, plane) in raw.chunks_exact(n).enumerate() {
            bands[b] = plane.iter().map(|v| clean(*v)).collect();
        }
    } else {
        for (i, px) in raw.chunks_exact(nb).enumerate() {
            for (b, v) in px.iter().enumerate() {
                bands[b][i] = clean(*v);
            }
        }
    }
    Ok(Image {
        x0: tx,
        y0: ty - rows as f64 * sy,
        cell: sx,
        cols,
        rows,
        bands,
    })
}

/// The band values at the 2nd and 98th percentile (a contrast stretch that ignores outliers).
pub fn stretch(band: &[f32]) -> (f32, f32) {
    let mut v: Vec<f32> = band
        .iter()
        .step_by((band.len() / 100_000).max(1))
        .copied()
        .filter(|x| x.is_finite())
        .collect();
    if v.is_empty() {
        return (0.0, 1.0);
    }
    v.sort_by(f32::total_cmp);
    (
        v[v.len() * 2 / 100],
        v[(v.len() * 98 / 100).min(v.len() - 1)],
    )
}

/// What one band looks like: extremes, the 2nd and 98th percentile, mean and deviation.
#[derive(Clone, Copy, Debug, Default)]
pub struct BandStats {
    pub min: f32,
    pub max: f32,
    pub p2: f32,
    pub p98: f32,
    pub mean: f32,
    pub std: f32,
}

/// Statistics of a band from a sample of at most ~200 000 of its pixels.
pub fn band_stats(band: &[f32]) -> BandStats {
    let mut v: Vec<f32> = band
        .iter()
        .step_by((band.len() / 200_000).max(1))
        .copied()
        .filter(|x| x.is_finite())
        .collect();
    if v.is_empty() {
        return BandStats {
            max: 1.0,
            p98: 1.0,
            ..BandStats::default()
        };
    }
    v.sort_by(f32::total_cmp);
    let n = v.len() as f64;
    let mean = v.iter().map(|x| f64::from(*x)).sum::<f64>() / n;
    let var = v
        .iter()
        .map(|x| (f64::from(*x) - mean).powi(2))
        .sum::<f64>()
        / n;
    BandStats {
        min: v[0],
        max: v[v.len() - 1],
        p2: v[v.len() * 2 / 100],
        p98: v[(v.len() * 98 / 100).min(v.len() - 1)],
        mean: mean as f32,
        std: var.sqrt() as f32,
    }
}

/// How three bands become a picture: the band for red, green and blue, the values mapped to black
/// and white for each of them, and brightness (−1…1), contrast (1 = none) and gamma (1 = none).
#[derive(Clone, Debug)]
pub struct RgbStyle {
    pub bands: [usize; 3],
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub brightness: f32,
    pub contrast: f32,
    pub gamma: f32,
}

/// An RGBA picture of the image: NaN pixels are transparent.
pub fn composite_style(img: &Image, st: &RgbStyle) -> Vec<u8> {
    use rayon::prelude::*;
    let pick = |k: usize| img.bands.get(k).or_else(|| img.bands.first());
    let (Some(b0), Some(b1), Some(b2)) = (pick(st.bands[0]), pick(st.bands[1]), pick(st.bands[2]))
    else {
        return Vec::new();
    };
    let src = [b0, b1, b2];
    let inv_gamma = 1.0 / st.gamma.max(0.05);
    let map = |v: f32, k: usize| {
        let t = ((v - st.lo[k]) / (st.hi[k] - st.lo[k]).max(1e-9)).clamp(0.0, 1.0);
        let t = if (inv_gamma - 1.0).abs() > 1e-3 {
            t.powf(inv_gamma)
        } else {
            t
        };
        (((t - 0.5) * st.contrast + 0.5 + st.brightness).clamp(0.0, 1.0) * 255.0) as u8
    };
    let mut out = vec![0u8; img.cols * img.rows * 4];
    out.par_chunks_mut(4 * img.cols.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let i = y * img.cols + x;
                if src[0][i].is_nan() {
                    continue;
                }
                for k in 0..3 {
                    px[k] = map(src[k][i], k);
                }
                px[3] = 255;
            }
        });
    out
}

/// The indices offered by default, as expressions over the roles `RED GREEN BLUE NIR SWIR1 SWIR2`.
pub const INDICES: [(&str, &str); 9] = [
    ("NDVI (vegetación)", "(NIR-RED)/(NIR+RED)"),
    ("GNDVI (clorofila)", "(NIR-GREEN)/(NIR+GREEN)"),
    ("NDWI (agua)", "(GREEN-NIR)/(GREEN+NIR)"),
    ("EVI", "2.5*(NIR-RED)/(NIR+6*RED-7.5*BLUE+1)"),
    ("SAVI (suelo)", "1.5*(NIR-RED)/(NIR+RED+0.5)"),
    ("NDMI (humedad)", "(NIR-SWIR1)/(NIR+SWIR1)"),
    ("NBR (quemado)", "(NIR-SWIR2)/(NIR+SWIR2)"),
    ("VARI (solo RGB)", "(GREEN-RED)/(GREEN+RED-BLUE)"),
    ("ExG (solo RGB)", "2*GREEN-RED-BLUE"),
];

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f32),
    Band(usize),
    Op(char),
    Open,
    Close,
}

/// A band expression compiled once and evaluated on every pixel (`+ - * /`, brackets, numbers,
/// `B1`…`Bn`, and role names such as `NIR` given a band by `roles`).
#[derive(Clone, Debug)]
pub struct Expr {
    /// Reverse Polish form.
    rpn: Vec<Tok>,
}

impl Expr {
    pub fn parse(text: &str, roles: &[(&str, usize)], nbands: usize) -> Result<Self> {
        let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
        let mut toks: Vec<Tok> = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_ascii_digit() || c == '.' {
                let s = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let t: String = chars[s..i].iter().collect();
                toks.push(Tok::Num(
                    t.parse()
                        .map_err(|_| Error::Param(format!("bad number '{t}'")))?,
                ));
                continue;
            }
            if c.is_ascii_alphabetic() {
                let s = i;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                let name: String = chars[s..i].iter().collect::<String>().to_ascii_uppercase();
                let band =
                    if let Some(n) = name.strip_prefix('B').and_then(|n| n.parse::<usize>().ok()) {
                        n.checked_sub(1)
                    } else {
                        roles
                            .iter()
                            .find(|r| r.0.eq_ignore_ascii_case(&name))
                            .map(|r| r.1)
                    };
                match band.filter(|b| *b < nbands) {
                    Some(b) => toks.push(Tok::Band(b)),
                    None => {
                        return Err(Error::Param(format!(
                            "'{name}' is not a band of this image ({nbands} bands)"
                        )));
                    }
                }
                continue;
            }
            toks.push(match c {
                '+' | '-' | '*' | '/' => Tok::Op(c),
                '(' => Tok::Open,
                ')' => Tok::Close,
                _ => return Err(Error::Param(format!("unexpected '{c}' in the expression"))),
            });
            i += 1;
        }
        // shunting-yard; a '-' with nothing before it (or after an operator or '(') is a 0 - x
        let prec = |o: char| if o == '+' || o == '-' { 1 } else { 2 };
        let (mut out, mut ops): (Vec<Tok>, Vec<Tok>) = (Vec::new(), Vec::new());
        let mut prev_value = false;
        for t in toks {
            match t {
                Tok::Num(_) | Tok::Band(_) => {
                    out.push(t);
                    prev_value = true;
                }
                Tok::Op(o) => {
                    if !prev_value {
                        if o != '-' {
                            return Err(Error::Param("the expression is malformed".into()));
                        }
                        out.push(Tok::Num(0.0));
                    }
                    while let Some(Tok::Op(p)) = ops.last() {
                        if prec(*p) >= prec(o) {
                            out.push(ops.pop().unwrap_or(Tok::Close));
                        } else {
                            break;
                        }
                    }
                    ops.push(Tok::Op(o));
                    prev_value = false;
                }
                Tok::Open => {
                    ops.push(Tok::Open);
                    prev_value = false;
                }
                Tok::Close => {
                    loop {
                        match ops.pop() {
                            Some(Tok::Open) => break,
                            Some(o) => out.push(o),
                            None => return Err(Error::Param("unbalanced brackets".into())),
                        }
                    }
                    prev_value = true;
                }
            }
        }
        while let Some(o) = ops.pop() {
            if o == Tok::Open {
                return Err(Error::Param("unbalanced brackets".into()));
            }
            out.push(o);
        }
        Ok(Self { rpn: out })
    }

    pub fn eval(&self, px: &[f32]) -> f32 {
        let mut st: Vec<f32> = Vec::with_capacity(8);
        for t in &self.rpn {
            match t {
                Tok::Num(v) => st.push(*v),
                Tok::Band(b) => st.push(px.get(*b).copied().unwrap_or(f32::NAN)),
                Tok::Op(o) => {
                    let (b, a) = (st.pop().unwrap_or(f32::NAN), st.pop().unwrap_or(f32::NAN));
                    st.push(match o {
                        '+' => a + b,
                        '-' => a - b,
                        '*' => a * b,
                        _ => {
                            if b == 0.0 {
                                f32::NAN
                            } else {
                                a / b
                            }
                        }
                    });
                }
                _ => {}
            }
        }
        st.pop().filter(|v| v.is_finite()).unwrap_or(f32::NAN)
    }
}

/// The expression applied to every pixel of the image, as a one-band raster.
pub fn index(img: &Image, expr: &Expr) -> Result<Grid> {
    let mut g = Grid::new(img.x0, img.y0, img.cell, img.cols, img.rows)?;
    let mut px = vec![0f32; img.bands.len()];
    for (i, out) in g.data.iter_mut().enumerate() {
        for (b, v) in img.bands.iter().enumerate() {
            px[b] = v[i];
        }
        *out = expr.eval(&px);
    }
    Ok(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions_follow_precedence_and_roles() -> Result<()> {
        let roles = [("NIR", 3usize), ("RED", 2usize)];
        let e = Expr::parse("(NIR-RED)/(NIR+RED)", &roles, 4)?;
        let v = e.eval(&[0.0, 0.0, 10.0, 30.0]);
        assert!((v - 0.5).abs() < 1e-6);
        assert!((Expr::parse("2+3*4", &[], 1)?.eval(&[0.0]) - 14.0).abs() < 1e-6);
        assert!((Expr::parse("-B1+5", &[], 1)?.eval(&[2.0]) - 3.0).abs() < 1e-6);
        assert!(Expr::parse("B1/B2", &[], 2)?.eval(&[1.0, 0.0]).is_nan());
        assert!(Expr::parse("B9", &[], 2).is_err());
        Ok(())
    }

    #[test]
    fn composite_maps_values_and_hides_nodata() {
        let img = Image {
            x0: 0.0,
            y0: 0.0,
            cell: 1.0,
            cols: 2,
            rows: 1,
            bands: vec![vec![0.0, f32::NAN]; 3],
        };
        let st = RgbStyle {
            bands: [0, 1, 2],
            lo: [0.0; 3],
            hi: [1.0; 3],
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
        };
        let px = composite_style(&img, &st);
        assert_eq!(&px[..4], &[0, 0, 0, 255]);
        assert_eq!(px[7], 0);
    }
}
