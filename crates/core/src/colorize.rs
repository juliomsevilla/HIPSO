//! Colour of every point from an image of the same place (an orthophoto, usually): each point
//! takes the red, green and blue of the pixel it falls on, and the near infrared too when the
//! cloud's format keeps it (8 and 10) and the image has that band.
//!
//! Point formats without colour are written in the nearest one that has it: 0 and 1 become 7
//! (LAS 1.4: every attribute is kept, the class gets a whole byte), 6 becomes 7. Formats 4 and 9
//! (waveforms) are not converted. The image is read once per cloud, only over the cloud's
//! extent; a window too large for memory is read from a coarser overview, and the report says so.

use std::path::{Path, PathBuf};

use crate::las::{Reader, Writer, WriterOptions};
use crate::pyramid::Pyramid;
use crate::{Error, Result};

/// Most pixels of the window read from the image (about 1.2 GB with three bands).
const MAX_PIXELS: u64 = 100_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct ColorizeJob {
    /// The image (GeoTIFF, JPEG 2000…) in the cloud's coordinate system.
    pub image: PathBuf,
    /// Bands of the image (from 1) for red, green and blue.
    pub rgb: [usize; 3],
    /// Band of the near infrared, for formats 8 and 10.
    pub nir: Option<usize>,
    /// Each cloud and where its coloured copy goes.
    pub inputs: Vec<(PathBuf, PathBuf)>,
    /// Points the image does not cover keep the colour they had (else they get black).
    pub keep_outside: bool,
}

/// Where each format keeps its colour.
fn rgb_at(format: u8) -> Option<usize> {
    match format {
        2 => Some(20),
        3 | 5 => Some(28),
        7 | 8 | 10 => Some(30),
        _ => None,
    }
}

/// The format a cloud is written in: its own if it has colour, else the nearest one that has.
fn coloured_format(format: u8) -> Result<u8> {
    match format {
        2 | 3 | 5 | 7 | 8 | 10 => Ok(format),
        0 | 1 | 6 => Ok(7),
        f => Err(Error::Unsupported(format!(
            "formato de punto {f} (con forma de onda): no se puede añadir color; conviértelo antes a otro formato"
        ))),
    }
}

/// The record of format `old` in format `new` (only the conversions of [`coloured_format`]).
fn convert(old: u8, new: u8, rec: &[u8], out: &mut Vec<u8>) {
    if old == new {
        out.clear();
        out.extend_from_slice(rec);
    } else if old == 6 {
        // 7 is 6 with the colour after its 30 bytes; extra attributes follow
        out.clear();
        out.extend_from_slice(&rec[..30.min(rec.len())]);
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(rec.get(30..).unwrap_or(&[]));
    } else {
        crate::las::upgrade_record(old, new, rec, out);
    }
}

/// How a sample of the image is brought to the 16 bits of LAS colour.
fn to_u16(v: f32, bits: u8) -> u16 {
    let k = if bits <= 8 { 257.0 } else { 1.0 };
    (v * k).round().clamp(0.0, 65535.0) as u16
}

/// Colours one cloud; returns the points coloured, those left uncoloured and a note on the
/// resolution used.
fn colour_one(
    pyr: &Pyramid,
    job: &ColorizeJob,
    input: &Path,
    output: &Path,
) -> Result<(u64, u64, Option<f64>)> {
    let r = Reader::open(input)?;
    if let (Some(a), Some(b)) = (r.header.epsg, pyr.epsg)
        && a != b
    {
        return Err(Error::Param(format!(
            "{} está en EPSG {a} y la imagen en EPSG {b}: pasa una de las dos al sistema de la otra",
            input.display()
        )));
    }
    let old = r.header.format;
    let new = coloured_format(old)?;
    let b = r.header.bounds;
    let win = [
        b[0] - pyr.cell,
        b[1] - pyr.cell,
        b[3] + pyr.cell,
        b[4] + pyr.cell,
    ];
    // the finest level whose window fits the memory budget
    let px = |level: usize| {
        let c = pyr.cell_at(level);
        (((win[2] - win[0]) / c).ceil() * ((win[3] - win[1]) / c).ceil()).max(0.0) as u64
    };
    let level = (0..pyr.levels.len())
        .find(|&l| px(l) <= MAX_PIXELS)
        .unwrap_or(pyr.levels.len() - 1);
    let img = pyr.read(level, win)?;
    let coarse = (level > 0).then(|| pyr.cell_at(level));
    for &band in job.rgb.iter().chain(job.nir.iter()) {
        if band == 0 || band > img.bands.len() {
            return Err(Error::Param(format!(
                "la imagen tiene {} banda(s): no existe la banda {band}",
                img.bands.len()
            )));
        }
    }
    let template = if new == old {
        Reader::open(input)?
    } else {
        Reader::open(input)?.upgraded(new)
    };
    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create_with(output, &template, &WriterOptions::default())?;
    let at = rgb_at(new).unwrap_or(30);
    let nir = job.nir.filter(|_| matches!(new, 8 | 10));
    let top = img.y0 + img.rows as f64 * img.cell;
    let (mut done, mut missed) = (0u64, 0u64);
    let rec_len = usize::from(r.header.record_len);
    let mut out = Vec::with_capacity(rec_len + 8);
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            convert(old, new, rec, &mut out);
            let pixel = r.decode(rec).and_then(|p| {
                let (c, row) = (
                    ((p.x - img.x0) / img.cell).floor(),
                    ((top - p.y) / img.cell).floor(),
                );
                (c >= 0.0 && row >= 0.0 && (c as usize) < img.cols && (row as usize) < img.rows)
                    .then(|| row as usize * img.cols + c as usize)
            });
            let values = pixel.and_then(|i| {
                let v: Vec<f32> = job.rgb.iter().map(|&b| img.bands[b - 1][i]).collect();
                v.iter().all(|x| x.is_finite()).then_some((i, v))
            });
            match values {
                Some((i, v)) => {
                    for (k, x) in v.iter().enumerate() {
                        out[at + 2 * k..at + 2 * k + 2]
                            .copy_from_slice(&to_u16(*x, pyr.bits).to_le_bytes());
                    }
                    if let Some(n) = nir {
                        let x = img.bands[n - 1][i];
                        if x.is_finite() {
                            out[at + 6..at + 8].copy_from_slice(&to_u16(x, pyr.bits).to_le_bytes());
                        }
                    }
                    done += 1;
                }
                None => {
                    if !job.keep_outside {
                        out[at..at + 6].fill(0);
                    }
                    missed += 1;
                }
            }
            w.write(&out)?;
        }
        Ok(())
    })?;
    w.finish()?;
    Ok((done, missed, coarse))
}

/// Colours every cloud of the job.
pub fn run(job: &ColorizeJob) -> Result<String> {
    if job.inputs.is_empty() {
        return Err(Error::Param("no hay nube de entrada".into()));
    }
    let pyr = Pyramid::open(&job.image)?;
    if pyr.bands < 3 && job.rgb.iter().any(|&b| b > pyr.bands) {
        return Err(Error::Param(format!(
            "la imagen tiene {} banda(s): para colorear hacen falta tres (rojo, verde y azul)",
            pyr.bands
        )));
    }
    let mut lines = Vec::new();
    for (input, output) in &job.inputs {
        crate::check_cancel()?;
        let (done, missed, coarse) = colour_one(&pyr, job, input, output)?;
        let mut line = format!("{done} puntos coloreados");
        if missed > 0 {
            line.push_str(&format!(", {missed} fuera de la imagen (sin color)"));
        }
        if let Some(c) = coarse {
            line.push_str(&format!(
                " (la nube es muy extensa: se usó la imagen reducida a {} m de píxel)",
                crate::paths::num(c)
            ));
        }
        lines.push(format!("{line}: {}", output.display()));
    }
    if pyr.epsg.is_none() {
        lines.push(
            "Aviso: la imagen no declara su sistema de coordenadas: se ha supuesto el de la nube"
                .into(),
        );
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiled::{Compression, Spec};

    #[test]
    fn points_take_the_colour_of_their_pixel() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("el_colorize_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // a 20 x 20 m image of 1 m pixels: red rises to the east, green to the north, blue 7
        let img = dir.join("orto.tif");
        let spec = Spec {
            cols: 20,
            rows: 20,
            bands: 3,
            dtype: crate::rastertools::Dtype::U8,
            cell: 1.0,
            x0: 1000.0,
            top: 2020.0,
            epsg: Some(25830),
            nodata: 0.0,
            compression: Compression::Deflate,
            nearest: false,
            bigtiff: false,
        };
        crate::tiled::write(&img, spec, 20, |r0, r1| {
            let mut v = Vec::new();
            for r in r0..r1 {
                for c in 0..20 {
                    v.extend([10.0 * c as f32 + 5.0, 10.0 * (19 - r) as f32 + 5.0, 7.0]);
                }
            }
            Ok(v)
        })?;
        // a cloud of format 1 (no colour) with points at known pixels and one outside
        let tpl = crate::ply::template(&dir, 0, 0.01, [1000.0, 2000.0, 0.0], Some(25830))?;
        let cloud = dir.join("nube.las");
        let mut w = Writer::create(&cloud, &tpl)?;
        for (x, y) in [(1000.5, 2000.5), (1012.5, 2017.5), (1050.0, 2050.0)] {
            let mut rec = vec![0u8; 20];
            let xi = ((x - 1000.0) / 0.01f64).round() as i32;
            let yi = ((y - 2000.0) / 0.01f64).round() as i32;
            rec[0..4].copy_from_slice(&xi.to_le_bytes());
            rec[4..8].copy_from_slice(&yi.to_le_bytes());
            rec[14] = 0x09;
            rec[15] = 2;
            w.write(&rec)?;
        }
        w.finish()?;
        let out = dir.join("nube_rgb.las");
        let job = ColorizeJob {
            image: img,
            rgb: [1, 2, 3],
            nir: None,
            inputs: vec![(cloud, out.clone())],
            keep_outside: false,
        };
        let text = run(&job)?;
        assert!(text.contains("2 puntos coloreados"), "{text}");
        let r = Reader::open(&out)?;
        // format 0 has no colour: it is written as 7
        assert_eq!(r.header.format, 7);
        let rec_len = usize::from(r.header.record_len);
        let mut got = Vec::new();
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let p = r.decode(rec);
                got.push((p.map(|p| p.class), r.decode_extra(rec).rgb));
            }
            Ok(())
        })?;
        assert_eq!(got[0], (Some(2), Some([5 * 257, 5 * 257, 7 * 257])));
        assert_eq!(got[1], (Some(2), Some([125 * 257, 175 * 257, 7 * 257])));
        assert_eq!(got[2].1, Some([0, 0, 0]));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
