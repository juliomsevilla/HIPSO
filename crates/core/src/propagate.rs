//! Propaga la clase de una nube de referencia (por ejemplo, una versión adelgazada y ya clasificada)
//! a todos los puntos de otra nube más densa: cada punto recibe la clase del punto de referencia más
//! cercano a menos de un radio. Permite clasificar con algoritmos costosos solo una fracción de los
//! puntos y obtener la clasificación de todos. Los puntos sin referencia cercana conservan su clase.
//!
//! La referencia se carga en memoria (posición y clase, unos 13 bytes por punto) en una rejilla de
//! celdas del tamaño del radio; la nube destino se recorre en flujo y nunca se modifica.

use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;

use crate::classes::ClassSet;
use crate::las::{self, Reader, Writer};
use crate::{Error, Result};

/// Qué se propaga y hasta dónde.
#[derive(Clone, Copy, Debug)]
pub struct PropagateParams {
    /// Distancia máxima, en metros, a la que un punto de referencia da su clase.
    pub radius: f64,
    /// Clases de la referencia que se propagan; los demás puntos de referencia no cuentan.
    pub classes: ClassSet,
    /// Mide solo en planta (x, y) en lugar de en 3D.
    pub flat: bool,
}

/// Cuántos puntos se leyeron y a cuántos se les cambió la clase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropagateReport {
    pub read: u64,
    pub changed: u64,
    pub reference: u64,
}

/// Los puntos de referencia agrupados por celda de planta.
struct Grid {
    cell: f64,
    origin: [f64; 2],
    /// Por celda: el rango de `pts` y `class` que le corresponde.
    cells: HashMap<(i64, i64), (u32, u32)>,
    pts: Vec<[f64; 3]>,
    class: Vec<u8>,
}

impl Grid {
    fn key(&self, x: f64, y: f64) -> (i64, i64) {
        (
            ((x - self.origin[0]) / self.cell).floor() as i64,
            ((y - self.origin[1]) / self.cell).floor() as i64,
        )
    }

    fn build(mut points: Vec<([f64; 3], u8)>, cell: f64) -> Self {
        let origin = points.first().map_or([0.0; 2], |p| [p.0[0], p.0[1]]);
        let mut g = Self {
            cell,
            origin,
            cells: HashMap::new(),
            pts: Vec::new(),
            class: Vec::new(),
        };
        points.sort_by_cached_key(|(p, _)| g.key(p[0], p[1]));
        for (i, (p, c)) in points.iter().enumerate() {
            let e = g.cells.entry(g.key(p[0], p[1])).or_insert((i as u32, 0));
            e.1 += 1;
            g.pts.push(*p);
            g.class.push(*c);
        }
        g
    }

    /// La clase del punto de referencia más cercano a `p` a menos de `radius`, si lo hay.
    fn nearest(&self, p: [f64; 3], radius: f64, flat: bool) -> Option<u8> {
        let (cx, cy) = self.key(p[0], p[1]);
        let mut best: Option<(f64, u8)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(&(first, n)) = self.cells.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                for i in first as usize..(first + n) as usize {
                    let q = self.pts[i];
                    let dz = if flat { 0.0 } else { q[2] - p[2] };
                    let d = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + dz * dz).sqrt();
                    if d <= radius && best.is_none_or(|b| d < b.0) {
                        best = Some((d, self.class[i]));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }
}

/// Copia a `output` la nube `target` con la clase de cada punto tomada del punto más cercano de
/// `reference` a menos de `params.radius`.
pub fn propagate_class(
    reference: &Path,
    target: &Path,
    output: &Path,
    params: &PropagateParams,
) -> Result<PropagateReport> {
    if !(params.radius > 0.0) {
        return Err(Error::Param("el radio debe ser positivo".into()));
    }
    if crate::paths::same_file(target, output) || crate::paths::same_file(reference, output) {
        return Err(Error::Param(
            "la nube de salida debe ser un archivo nuevo".into(),
        ));
    }
    let rr = Reader::open(reference)?;
    let rlen = usize::from(rr.header.record_len);
    let mut pts = Vec::new();
    rr.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rlen) {
            if let Some(p) = rr.decode(rec)
                && params.classes.contains(p.class)
            {
                pts.push(([p.x, p.y, p.z], p.class));
            }
        }
        Ok(())
    })?;
    if pts.is_empty() {
        return Err(Error::Param(
            "la referencia no tiene puntos de las clases pedidas".into(),
        ));
    }
    let big = pts.iter().any(|p| p.1 > 31);
    let reference_points = pts.len() as u64;
    let grid = Grid::build(pts, params.radius);

    crate::paths::ensure_parent(output)?;
    let r = Reader::open(target)?;
    let (old_format, rec_len) = (r.header.format, usize::from(r.header.record_len));
    let upgrade = match (big, old_format) {
        (true, f) if f < 6 => Some(las::upgraded_format(f).ok_or_else(|| {
            Error::Unsupported(format!(
                "el formato de punto {f} no admite clases por encima de 31: convierte antes la nube"
            ))
        })?),
        _ => None,
    };
    let template = match upgrade {
        Some(n) => Reader::open(target)?.upgraded(n),
        None => Reader::open(target)?,
    };
    let format = template.header.format;
    let mut w = Writer::create(output, &template)?;
    let mut rep = PropagateReport {
        reference: reference_points,
        ..Default::default()
    };
    let mut conv = Vec::new();
    r.scan_raw(|bytes| {
        let recs: Vec<&[u8]> = bytes.chunks_exact(rec_len).collect();
        let new: Vec<Option<u8>> = recs
            .par_iter()
            .map(|rec| {
                let p = r.decode(rec)?;
                grid.nearest([p.x, p.y, p.z], params.radius, params.flat)
                    .filter(|&c| c != p.class)
            })
            .collect();
        let mut buf = vec![0u8; rec_len];
        for (rec, class) in recs.iter().zip(new) {
            rep.read += 1;
            let out: &[u8] = if let Some(n) = upgrade {
                las::upgrade_record(old_format, n, rec, &mut conv);
                if let Some(c) = class {
                    las::set_class(format, &mut conv, c);
                }
                &conv
            } else if let Some(c) = class {
                buf.copy_from_slice(rec);
                las::set_class(format, &mut buf, c);
                &buf
            } else {
                rec
            };
            rep.changed += u64::from(class.is_some());
            w.write(out)?;
        }
        Ok(())
    })?;
    w.finish()?;
    Ok(rep)
}

/// El resumen de una propagación, en una línea.
pub fn report_text(r: &PropagateReport, output: &Path) -> String {
    format!(
        "{} puntos leídos, {} con clase nueva (referencia de {} puntos) -> {}",
        r.read,
        r.changed,
        r.reference,
        output.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_nearest_respects_radius_and_flat() {
        let g = Grid::build(
            vec![([0.0, 0.0, 0.0], 2), ([10.0, 0.0, 0.0], 5), ([0.0, 0.0, 3.0], 6)],
            2.0,
        );
        assert_eq!(g.nearest([0.5, 0.0, 0.0], 2.0, false), Some(2));
        assert_eq!(g.nearest([5.0, 0.0, 0.0], 2.0, false), None);
        assert_eq!(g.nearest([10.2, 0.0, 0.0], 2.0, false), Some(5));
        // en 3D gana el punto de la misma cota; en planta empatan y se queda el primero
        assert_eq!(g.nearest([0.0, 0.0, 2.9], 3.5, false), Some(6));
        assert_eq!(g.nearest([0.0, 0.0, 2.9], 3.5, true), Some(2));
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    /// Un LAS 1.2 de formato 1 con puntos (x, y, z, clase) en una rejilla de 0,01.
    fn write_las(path: &Path, pts: &[(f64, f64, f64, u8)]) {
        let mut h = vec![0u8; 227];
        h[..4].copy_from_slice(b"LASF");
        h[24] = 1;
        h[25] = 2;
        h[94..96].copy_from_slice(&227u16.to_le_bytes());
        h[96..100].copy_from_slice(&227u32.to_le_bytes());
        h[104] = 1;
        h[105..107].copy_from_slice(&28u16.to_le_bytes());
        h[107..111].copy_from_slice(&(pts.len() as u32).to_le_bytes());
        for (at, v) in [(131, 0.01), (139, 0.01), (147, 0.01)] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for (at, v) in [(179, 100.0), (187, 0.0), (195, 100.0), (203, 0.0), (211, 100.0), (219, 0.0)] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for p in pts {
            let mut rec = [0u8; 28];
            for (k, v) in [p.0, p.1, p.2].into_iter().enumerate() {
                rec[4 * k..4 * k + 4].copy_from_slice(&((v / 0.01).round() as i32).to_le_bytes());
            }
            rec[14] = 1 | (1 << 3);
            rec[15] = p.3;
            h.extend_from_slice(&rec);
        }
        std::fs::write(path, h).unwrap();
    }

    fn classes_of(path: &Path) -> Vec<u8> {
        let r = Reader::open(path).unwrap();
        let len = usize::from(r.header.record_len);
        let mut v = Vec::new();
        r.scan_raw(|b| {
            v.extend(b.chunks_exact(len).filter_map(|c| r.decode(c)).map(|p| p.class));
            Ok(())
        })
        .unwrap();
        v
    }

    #[test]
    fn propagates_to_close_points_and_keeps_the_rest() {
        let dir = std::env::temp_dir();
        let id = std::process::id();
        let p = |n: &str| dir.join(format!("hipso_prop_{n}_{id}.las"));
        let (rf, tg, out) = (p("ref"), p("tgt"), p("out"));
        write_las(&rf, &[(10.0, 10.0, 5.0, 2), (50.0, 50.0, 5.0, 5), (80.0, 80.0, 5.0, 7)]);
        write_las(
            &tg,
            &[(10.5, 10.0, 5.0, 1), (49.0, 50.0, 5.0, 1), (30.0, 30.0, 5.0, 1), (80.0, 80.5, 5.0, 1)],
        );
        let mut classes = ClassSet::all();
        classes.set(7, false);
        let params = PropagateParams { radius: 2.0, classes, flat: false };
        let rep = propagate_class(&rf, &tg, &out, &params).unwrap();
        assert_eq!((rep.read, rep.changed, rep.reference), (4, 2, 2));
        // el último punto solo tenía cerca la clase 7, excluida
        assert_eq!(classes_of(&out), [2, 5, 1, 1]);
        assert!(propagate_class(&rf, &tg, &tg, &params).is_err());
        for f in [rf, tg, out] {
            let _ = std::fs::remove_file(f);
        }
    }
}
