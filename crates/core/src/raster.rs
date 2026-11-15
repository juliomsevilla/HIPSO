//! Teledetección pasiva junto al LiDAR: imágenes multibanda (fotos aéreas, escenas de satélite),
//! composiciones de color e índices espectrales (NDVI, NDWI, EVI, SAVI…) a partir de expresiones de
//! bandas. Las imágenes sirven de mapas base para las nubes y los índices como rásteres con los que
//! comparar el CHM.

use std::io::BufReader;
use std::path::Path;

use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

use crate::grid::Grid;
use crate::{Error, Result};

/// Una imagen georreferenciada con cualquier número de bandas, como decimales; la fila 0 es la más
/// al norte.
#[derive(Clone, Debug)]
pub struct Image {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    pub bands: Vec<Vec<f32>>,
}

/// Extensiones de los rásteres que se pueden leer (para diálogos de archivos), en minúsculas.
pub const EXTENSIONS: &[&str] = &[
    "tif", "tiff", "asc", "jp2", "j2k", "j2c", "png", "jpg", "jpeg", "bil", "bsq", "bip", "img",
    "hgt", "flt",
];

/// Si una extensión en minúsculas es una de [`EXTENSIONS`].
pub fn is_raster_ext(ext: &str) -> bool {
    EXTENSIONS.contains(&ext)
}

fn tiff_err(e: tiff::TiffError) -> Error {
    Error::Format(format!("TIFF: {e}{}", crate::cloud_hint(&e.to_string())))
}

/// Lee un GeoTIFF de cualquier tipo de muestra (8 o 16 bits, entero o decimal) con bandas
/// entrelazadas.
pub fn read_image(path: &Path) -> Result<Image> {
    crate::check_raster_format(path)?;
    if crate::pyramid::is_jp2_file(path) {
        let pyr = crate::pyramid::Pyramid::open(path)?;
        return pyr.read(0, pyr.extent());
    }
    if let Some(l) = crate::rasterfmt::load(path)? {
        return Ok(l.image);
    }
    let mut d = Decoder::new(BufReader::new(crate::open_file(path)?)).map(|d| d.with_limits(tiff::decoder::Limits::unlimited())).map_err(tiff_err)?;
    let (w, h) = d.dimensions().map_err(tiff_err)?;
    let scale = d
        .get_tag_f64_vec(Tag::ModelPixelScaleTag)
        .map_err(tiff_err)?;
    let tie = d.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(tiff_err)?;
    let (Some(&sx), Some(&sy), Some(&tx), Some(&ty)) =
        (scale.first(), scale.get(1), tie.get(3), tie.get(4))
    else {
        return Err(Error::Format(format!(
            "{}: no es un GeoTIFF (sin escala de píxel ni punto de anclaje)",
            path.display()
        )));
    };
    let nodata = d
        .get_tag_ascii_string(Tag::GdalNodata)
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok());
    let (cols, rows) = (w as usize, h as usize);
    // los archivos secuenciales por banda (configuración planar 2) vienen como un plano tras otro
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
                "{}: este tipo de dato no se lee",
                path.display()
            )));
        }
    };
    let n = cols * rows;
    if n == 0 || !raw.len().is_multiple_of(n) {
        return Err(Error::Format(format!(
            "{}: los datos de píxel no encajan en {cols} x {rows}",
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

/// Los valores de banda en los percentiles 2 y 98 (un estirado de contraste que ignora los valores
/// atípicos).
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

/// Qué aspecto tiene una banda: extremos, los percentiles 2 y 98, media y desviación.
#[derive(Clone, Copy, Debug, Default)]
pub struct BandStats {
    pub min: f32,
    pub max: f32,
    pub p2: f32,
    pub p98: f32,
    pub mean: f32,
    pub std: f32,
}

/// Estadísticos de una banda a partir de una muestra de como mucho unos 200 000 de sus píxeles.
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

/// Cómo tres bandas se convierten en una imagen: la banda para rojo, verde y azul, los valores
/// asignados al negro y al blanco para cada una y brillo (−1…1), contraste (1 = ninguno) y gamma (1
/// = ninguna).
#[derive(Clone, Debug)]
pub struct RgbStyle {
    pub bands: [usize; 3],
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub brightness: f32,
    pub contrast: f32,
    pub gamma: f32,
}

/// Una imagen RGBA de la imagen: los píxeles NaN son transparentes.
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

/// Los índices que se ofrecen por defecto, como expresiones sobre los papeles `RED GREEN BLUE NIR
/// SWIR1 SWIR2`.
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

/// Una expresión de bandas compilada una vez y evaluada en cada píxel (`+ - * /`, paréntesis,
/// números, `B1`…`Bn` y nombres de papel como `NIR` a los que `roles` da una banda).
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
                        .map_err(|_| Error::Param(format!("número no válido: '{t}'")))?,
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
                            "'{name}' no es una banda de esta imagen (tiene {nbands})"
                        )));
                    }
                }
                continue;
            }
            toks.push(match c {
                '+' | '-' | '*' | '/' => Tok::Op(c),
                '(' => Tok::Open,
                ')' => Tok::Close,
                _ => return Err(Error::Param(format!("'{c}' inesperado en la expresión"))),
            });
            i += 1;
        }
        // patio de maniobras; un '-' sin nada delante (o tras un operador o '(') es un 0 - x
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
                            return Err(Error::Param("la expresión está mal formada".into()));
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
                            None => return Err(Error::Param("paréntesis desequilibrados".into())),
                        }
                    }
                    prev_value = true;
                }
            }
        }
        while let Some(o) = ops.pop() {
            if o == Tok::Open {
                return Err(Error::Param("paréntesis desequilibrados".into()));
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

/// La expresión aplicada a cada píxel de la imagen, como un ráster de una banda.
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
