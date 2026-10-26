//! E57 (ASTM E2807), the exchange format of terrestrial and mobile scanners, converted to LAS or
//! LAZ. Every scan of the file is placed with its own pose (rotation and translation), spherical
//! coordinates become Cartesian, and colour and intensity are scaled to 16 bits. The point source
//! id of each point is the number of its scan (1, 2…), so that the scans can be told apart later.
//!
//! Two passes over the file: the first finds the extent (for the scale and offset of the integer
//! coordinates), the second writes; memory does not grow with the number of points.

use std::path::Path;

use crate::las::{Writer, WriterOptions};
use crate::{Error, Result};

/// What a conversion did.
#[derive(Clone, Debug, PartialEq)]
pub struct E57Report {
    pub scans: usize,
    pub points: u64,
    pub colour: bool,
    pub intensity: bool,
    /// Points left out for having no valid coordinates (directions only, or invalid).
    pub skipped: u64,
}

fn bad(e: impl std::fmt::Display) -> Error {
    Error::Format(format!("E57: {e}"))
}

/// Each valid point of every scan, with the scan number (from 0): coordinates, colour and
/// intensity (both 0 to 1).
fn each_point(
    input: &Path,
    mut f: impl FnMut(usize, [f64; 3], Option<[f32; 3]>, Option<f32>) -> Result<()>,
) -> Result<u64> {
    let mut reader = e57::E57Reader::from_file(input).map_err(bad)?;
    let clouds = reader.pointclouds();
    let mut skipped = 0;
    for (k, pc) in clouds.iter().enumerate() {
        let mut it = reader.pointcloud_simple(pc).map_err(bad)?;
        // colour is colour: an intensity is not passed off as a grey
        it.intensity_to_color(false);
        for p in it {
            crate::check_cancel()?;
            let p = p.map_err(bad)?;
            match p.cartesian {
                e57::CartesianCoordinate::Valid { x, y, z }
                    if x.is_finite() && y.is_finite() && z.is_finite() =>
                {
                    let rgb = p.color.map(|c| [c.red, c.green, c.blue]);
                    f(k, [x, y, z], rgb, p.intensity)?;
                }
                _ => skipped += 1,
            }
        }
    }
    Ok(skipped)
}

/// Converts an E57 file to LAS or LAZ (by the output's extension). `epsg` is the coordinate
/// system of the points, which E57 files rarely say in a form that can be read.
pub fn convert(input: &Path, output: &Path, epsg: Option<u16>) -> Result<E57Report> {
    let reader = e57::E57Reader::from_file(input).map_err(bad)?;
    let clouds = reader.pointclouds();
    if clouds.is_empty() {
        return Err(bad("el archivo no tiene nubes de puntos"));
    }
    let has = |name: &str| {
        clouds.iter().any(|pc| {
            pc.prototype
                .iter()
                .any(|r| format!("{:?}", r.name).contains(name))
        })
    };
    let colour = has("Color");
    let intensity = has("Intensity");
    drop(reader);
    // pass 1: the extent
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let mut finite = 0u64;
    each_point(input, |_, p, _, _| {
        finite += 1;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        Ok(())
    })?;
    if finite == 0 {
        return Err(bad("no hay ningún punto con coordenadas"));
    }
    let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    // a tenth of a millimetre for scanners, unless the cloud is too big for the integers
    let scale = if span < 2.0e5 {
        0.0001
    } else if span < 2.0e6 {
        0.001
    } else {
        0.01
    };
    if span / scale > 2.0e9 {
        return Err(bad("la nube es demasiado grande para coordenadas enteras"));
    }
    let offset = [lo[0].floor(), lo[1].floor(), lo[2].floor()];
    let dir = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(std::env::temp_dir, Path::to_path_buf);
    crate::paths::ensure_parent(output)?;
    let tpl = crate::ply::template(&dir, if colour { 2 } else { 0 }, scale, offset, epsg)?;
    let mut w = Writer::create_with(output, &tpl, &WriterOptions::default())?;
    let mut rec = vec![0u8; if colour { 26 } else { 20 }];
    let to_int = |v: f64, o: f64| ((v - o) / scale).round().clamp(-2.147e9, 2.147e9) as i32;
    let mut written = 0u64;
    let skipped = each_point(input, |scan, p, rgb, int| {
        rec[0..4].copy_from_slice(&to_int(p[0], offset[0]).to_le_bytes());
        rec[4..8].copy_from_slice(&to_int(p[1], offset[1]).to_le_bytes());
        rec[8..12].copy_from_slice(&to_int(p[2], offset[2]).to_le_bytes());
        let i = int.map_or(0.0, |i| (f64::from(i) * 65535.0).clamp(0.0, 65535.0));
        rec[12..14].copy_from_slice(&(i.round() as u16).to_le_bytes());
        // first of one return, unclassified
        rec[14] = 0x09;
        rec[15] = 1;
        rec[18..20].copy_from_slice(&u16::try_from(scan + 1).unwrap_or(u16::MAX).to_le_bytes());
        if colour {
            let c = rgb.unwrap_or([0.0; 3]);
            for (n, comp) in c.iter().enumerate() {
                let q = (f64::from(*comp) * 65535.0).clamp(0.0, 65535.0).round() as u16;
                rec[20 + 2 * n..22 + 2 * n].copy_from_slice(&q.to_le_bytes());
            }
        }
        w.write(&rec)?;
        written += 1;
        Ok(())
    })?;
    w.finish()?;
    Ok(E57Report {
        scans: clouds.len(),
        points: written,
        colour,
        intensity,
        skipped,
    })
}

/// A short text for the report of a conversion.
pub fn describe(r: &E57Report, output: &Path) -> String {
    let mut s = format!(
        "{} puntos de {} escaneo{}",
        r.points,
        r.scans,
        if r.scans == 1 { "" } else { "s" }
    );
    if r.colour {
        s.push_str(" con color");
    }
    if r.intensity {
        s.push_str(if r.colour {
            " e intensidad"
        } else {
            " con intensidad"
        });
    }
    if r.skipped > 0 {
        s.push_str(&format!(
            " ({} sin coordenadas válidas, descartados)",
            r.skipped
        ));
    }
    if r.scans > 1 {
        s.push_str("; cada escaneo lleva su número como ID de fuente");
    }
    format!("{s}: {}", output.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_scans_with_colour_become_one_laz() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("el_e57_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("t.e57");
        {
            let mut w = e57::E57Writer::from_file(&path, "guid-file").map_err(bad)?;
            let proto = vec![
                e57::Record::CARTESIAN_X_F64,
                e57::Record::CARTESIAN_Y_F64,
                e57::Record::CARTESIAN_Z_F64,
                e57::Record::COLOR_RED_U8,
                e57::Record::COLOR_GREEN_U8,
                e57::Record::COLOR_BLUE_U8,
                e57::Record::INTENSITY_U16,
            ];
            for (s, shift) in [(0u8, 0.0f64), (1, 100.0)] {
                let mut pc = w
                    .add_pointcloud(&format!("guid-scan-{s}"), proto.clone())
                    .map_err(bad)?;
                for k in 0..10 {
                    let v = f64::from(k);
                    pc.add_point(vec![
                        e57::RecordValue::Double(500_000.0 + shift + v),
                        e57::RecordValue::Double(4_400_000.0 + v),
                        e57::RecordValue::Double(700.0 + v / 10.0),
                        e57::RecordValue::Integer(255),
                        e57::RecordValue::Integer(0),
                        e57::RecordValue::Integer(i64::from(s) * 255),
                        e57::RecordValue::Integer(65535),
                    ])
                    .map_err(bad)?;
                }
                pc.finalize().map_err(bad)?;
            }
            w.finalize().map_err(bad)?;
        }
        let out = dir.join("t.laz");
        let r = convert(&path, &out, Some(25830))?;
        assert_eq!(
            (r.scans, r.points, r.colour, r.intensity),
            (2, 20, true, true)
        );
        let laz = crate::las::Reader::open(&out)?;
        assert_eq!(laz.header.count, 20);
        assert_eq!(laz.header.epsg, Some(25830));
        let mut sources = Vec::new();
        let rec_len = usize::from(laz.header.record_len);
        laz.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                if let Some(p) = laz.decode(rec) {
                    let e = laz.decode_extra(rec);
                    sources.push((p.x.round(), e.source_id));
                    assert_eq!(e.rgb.map(|c| c[0]), Some(65535));
                }
            }
            Ok(())
        })?;
        assert!(sources.contains(&(500_000.0, 1)));
        assert!(sources.contains(&(500_109.0, 2)));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
