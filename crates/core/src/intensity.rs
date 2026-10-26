//! Intensity normalised for range: the energy that comes back falls with the square of the
//! distance to the scanner (extended targets: ground, canopy), so the same surface looks darker
//! at the edge of a swath or from a higher flight line. Each intensity is multiplied by
//! `(R / R_ref)^a` (Höfle and Pfeifer 2007, "Correction of laser scanning intensity data: data
//! and model-driven approaches", ISPRS J. Photogramm. 62: 415–433), with `a = 2` by default and
//! `R_ref` the mean range of the cloud unless one is given, so values keep their usual size.
//!
//! The range `R` of each point comes from the trajectory of the scanner (time, x, y, z) at the
//! point's GPS time, or, without one, from the height of the sensor above the point and the scan
//! angle: `R = (Z_sensor − z) / cos(angle)`.

use std::path::{Path, PathBuf};

use crate::las::{Reader, Writer, WriterOptions};
use crate::{Error, Result};

/// Where the scanner was.
#[derive(Clone, Debug, PartialEq)]
pub enum Sensor {
    /// A text file of the trajectory: GPS time, x, y, z per line (comma, semicolon, tab or
    /// spaces; a header line is skipped), in the cloud's coordinates and time.
    Trajectory(PathBuf),
    /// The altitude of the flight (same vertical datum as the points), constant.
    Altitude(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntensityJob {
    pub sensor: Sensor,
    /// Exponent of the range: 2 for extended targets, about 3 for wires and edges.
    pub exponent: f64,
    /// Reference range; the mean range of each cloud when `None`.
    pub reference: Option<f64>,
    pub inputs: Vec<(PathBuf, PathBuf)>,
}

/// A trajectory sorted by time.
struct Track {
    t: Vec<f64>,
    xyz: Vec<[f64; 3]>,
}

impl Track {
    fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {e}", path.display()),
            ))
        })?;
        let mut rows: Vec<(f64, [f64; 3])> = Vec::new();
        for line in text.lines() {
            let v: Vec<f64> = line
                .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                .filter(|w| !w.is_empty())
                .map_while(|w| w.parse::<f64>().ok())
                .collect();
            if v.len() >= 4 && v[..4].iter().all(|x| x.is_finite()) {
                rows.push((v[0], [v[1], v[2], v[3]]));
            }
        }
        if rows.len() < 2 {
            return Err(Error::Format(format!(
                "{}: la trayectoria necesita al menos dos líneas de «tiempo x y z»",
                path.display()
            )));
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(Self {
            t: rows.iter().map(|r| r.0).collect(),
            xyz: rows.iter().map(|r| r.1).collect(),
        })
    }

    /// The position at time `t`, interpolated; `None` outside the trajectory (by more than a
    /// second).
    fn at(&self, t: f64) -> Option<[f64; 3]> {
        let n = self.t.len();
        if !(t >= self.t[0] - 1.0 && t <= self.t[n - 1] + 1.0) {
            return None;
        }
        let k = self.t.partition_point(|&x| x < t).clamp(1, n - 1);
        let (t0, t1) = (self.t[k - 1], self.t[k]);
        let w = if t1 > t0 {
            ((t - t0) / (t1 - t0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (a, b) = (self.xyz[k - 1], self.xyz[k]);
        Some([
            a[0] + (b[0] - a[0]) * w,
            a[1] + (b[1] - a[1]) * w,
            a[2] + (b[2] - a[2]) * w,
        ])
    }
}

/// The range of a point to the sensor, if it can be known.
fn range(
    track: Option<&Track>,
    altitude: Option<f64>,
    p: [f64; 3],
    t: f64,
    angle: f32,
) -> Option<f64> {
    let r = if let Some(tr) = track {
        let s = tr.at(t)?;
        ((s[0] - p[0]).powi(2) + (s[1] - p[1]).powi(2) + (s[2] - p[2]).powi(2)).sqrt()
    } else {
        let h = altitude? - p[2];
        h / f64::from(angle).to_radians().cos().max(0.2)
    };
    (r.is_finite() && r > 0.5).then_some(r)
}

/// Normalises the intensity of every cloud of the job.
pub fn run(job: &IntensityJob) -> Result<String> {
    if !(job.exponent > 0.0 && job.exponent <= 6.0) {
        return Err(Error::Param(
            "el exponente va de 0 a 6 (2 para superficies)".into(),
        ));
    }
    if job.reference.is_some_and(|r| !(r > 0.0)) {
        return Err(Error::Param(
            "la distancia de referencia debe ser positiva".into(),
        ));
    }
    let track = match &job.sensor {
        Sensor::Trajectory(p) => Some(Track::read(p)?),
        Sensor::Altitude(_) => None,
    };
    let altitude = match job.sensor {
        Sensor::Altitude(a) => Some(a),
        Sensor::Trajectory(_) => None,
    };
    let mut lines = Vec::new();
    for (input, output) in &job.inputs {
        crate::check_cancel()?;
        let r = Reader::open(input)?;
        let has_time = matches!(r.header.format, 1 | 3 | 4 | 5..=10);
        if track.is_some() && !has_time {
            return Err(Error::Param(format!(
                "{}: su formato de punto ({}) no guarda el tiempo GPS, que hace falta para usar la trayectoria; usa la altura de vuelo",
                input.display(),
                r.header.format
            )));
        }
        if let Some(a) = altitude
            && a <= r.header.bounds[5]
        {
            return Err(Error::Param(format!(
                "la altura de vuelo ({a} m) está por debajo del punto más alto de {} ({:.1} m)",
                input.display(),
                r.header.bounds[5]
            )));
        }
        let rec_len = usize::from(r.header.record_len);
        let point_range = |rec: &[u8]| -> Option<f64> {
            let p = r.decode(rec)?;
            let t = if track.is_some() {
                r.decode_extra(rec).gps_time
            } else {
                0.0
            };
            range(track.as_ref(), altitude, [p.x, p.y, p.z], t, p.scan_angle)
        };
        // pass 1: the mean range, unless a reference is given
        let reference = match job.reference {
            Some(v) => v,
            None => {
                let (mut sum, mut n) = (0.0, 0u64);
                r.scan_raw(|bytes| {
                    for rec in bytes.chunks_exact(rec_len) {
                        if let Some(d) = point_range(rec) {
                            sum += d;
                            n += 1;
                        }
                    }
                    Ok(())
                })?;
                if n == 0 {
                    return Err(Error::Param(format!(
                        "{}: no se ha podido calcular la distancia de ningún punto (¿la trayectoria es de otro vuelo o de otro tiempo GPS?)",
                        input.display()
                    )));
                }
                sum / n as f64
            }
        };
        crate::paths::ensure_parent(output)?;
        let mut w = Writer::create_with(output, &Reader::open(input)?, &WriterOptions::default())?;
        let (mut done, mut kept, mut clipped) = (0u64, 0u64, 0u64);
        let (mut lo, mut hi) = (f64::INFINITY, 0.0f64);
        let mut out = Vec::with_capacity(rec_len);
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                out.clear();
                out.extend_from_slice(rec);
                if let Some(d) = point_range(rec) {
                    let i = f64::from(u16::from_le_bytes([rec[12], rec[13]]));
                    let v = i * (d / reference).powf(job.exponent);
                    if v > 65535.0 {
                        clipped += 1;
                    }
                    out[12..14]
                        .copy_from_slice(&(v.round().clamp(0.0, 65535.0) as u16).to_le_bytes());
                    lo = lo.min(d);
                    hi = hi.max(d);
                    done += 1;
                } else {
                    kept += 1;
                }
                w.write(&out)?;
            }
            Ok(())
        })?;
        w.finish()?;
        let mut line = format!(
            "{done} intensidades normalizadas a {} m (distancias de {:.0} a {:.0} m, exponente {})",
            crate::paths::num((reference * 10.0).round() / 10.0),
            lo,
            hi,
            crate::paths::num(job.exponent)
        );
        if kept > 0 {
            line.push_str(&format!(
                ", {kept} sin distancia conocida (se dejan como estaban)"
            ));
        }
        if clipped > 0 {
            line.push_str(&format!(", {clipped} recortadas a 65535"));
        }
        lines.push(format!("{line}: {}", output.display()));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trajectory_is_interpolated_in_time() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("el_int_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let p = dir.join("tray.txt");
        std::fs::write(&p, "time x y z\n10 0 0 1000\n20 100 0 1000\n")?;
        let t = Track::read(&p)?;
        assert_eq!(t.at(15.0), Some([50.0, 0.0, 1000.0]));
        assert_eq!(t.at(30.0), None);
        // straight below at 1000 m, and 600 m to the side
        let d0 = range(Some(&t), None, [50.0, 0.0, 0.0], 15.0, 0.0);
        assert_eq!(d0, Some(1000.0));
        let d1 = range(Some(&t), None, [50.0, 600.0, 200.0], 15.0, 0.0).unwrap_or(0.0);
        assert!((d1 - 1000.0).abs() < 1e-9, "{d1}");
        // without a trajectory: the altitude and the angle
        let d2 = range(None, Some(1100.0), [0.0, 0.0, 100.0], 0.0, 60.0).unwrap_or(0.0);
        assert!((d2 - 2000.0).abs() < 1e-6, "{d2}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_far_point_gets_brighter_and_a_near_one_darker() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("el_int2_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let tpl = crate::ply::template(&dir, 0, 0.01, [0.0, 0.0, 0.0], Some(25830))?;
        let cloud = dir.join("n.las");
        let mut w = Writer::create(&cloud, &tpl)?;
        // same intensity 1000 at 0 m and at 500 m of height: sensor at 1000 m
        for z in [0.0f64, 500.0] {
            let mut rec = vec![0u8; 20];
            rec[8..12].copy_from_slice(&((z / 0.01).round() as i32).to_le_bytes());
            rec[12..14].copy_from_slice(&1000u16.to_le_bytes());
            rec[14] = 0x09;
            w.write(&rec)?;
        }
        w.finish()?;
        let out = dir.join("n_int.las");
        let job = IntensityJob {
            sensor: Sensor::Altitude(1000.0),
            exponent: 2.0,
            reference: Some(1000.0),
            inputs: vec![(cloud, out.clone())],
        };
        run(&job)?;
        let mut got = Vec::new();
        Reader::open(&out)?.scan(|p| got.push(p.intensity))?;
        // 1000 m away keeps 1000; 500 m away: 1000 · 0.5² = 250
        assert_eq!(got, vec![1000, 250]);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
