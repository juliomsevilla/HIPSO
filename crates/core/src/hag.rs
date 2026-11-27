//! Altura sobre el suelo de cada punto, a partir de los puntos de suelo que ya tiene la nube: un
//! TIN de los puntos de la clase suelo da el suelo bajo cada punto, y la altura es la diferencia.
//! No hace falta un modelo del suelo aparte. Se escribe como elevación de los puntos (la nube
//! normalizada, como hacen las demás herramientas que toman un modelo del suelo) o como atributo
//! adicional `HeightAboveGround`, conservando las elevaciones.

use std::path::Path;

use crate::grid::Grid;
use crate::las::{self, ExtraAttr, ExtraKind, Reader, Writer, WriterOptions};
use crate::{Error, Result, tin};

#[derive(Clone, Debug, PartialEq)]
pub struct HagParams {
    /// Celda de la superficie del suelo sobre la que se muestrea el TIN, en unidades del mapa.
    pub cell: f64,
    /// La clase de los puntos de suelo (2 en el convenio ASPRS).
    pub ground_class: u8,
    /// Sustituye la elevación por la altura (la nube normalizada); si no, conserva las elevaciones
    /// y añade la altura como atributo adicional.
    pub replace_z: bool,
}

impl Default for HagParams {
    fn default() -> Self {
        Self {
            cell: 0.5,
            ground_class: 2,
            replace_z: false,
        }
    }
}

/// La mayor superficie de suelo, en celdas.
const MAX_CELLS: usize = 400_000_000;

/// Escribe `input` con la altura sobre el suelo de cada punto; la línea de informe dice lo que se
/// hizo.
pub fn run(input: &Path, output: &Path, p: &HagParams) -> Result<String> {
    if !(p.cell > 0.0) {
        return Err(Error::Param("la celda debe ser positiva".into()));
    }
    let r = Reader::open(input)?;
    let rec_len = usize::from(r.header.record_len);
    // pasada 1: los puntos de suelo
    let mut ground: Vec<[f64; 3]> = Vec::new();
    r.scan(|q| {
        if q.class == p.ground_class && !q.is_noise() {
            ground.push([q.x, q.y, q.z]);
        }
    })?;
    if ground.is_empty() {
        return Err(Error::Param(format!(
            "la nube no tiene puntos de la clase {} (suelo): clasifícala antes",
            p.ground_class
        )));
    }
    let b = r.header.bounds;
    let cell = {
        // una superficie sobre toda la extensión debe caber en memoria: una celda mayor si no
        // cabría
        let cells = ((b[3] - b[0]) / p.cell + 1.0) * ((b[4] - b[1]) / p.cell + 1.0);
        if cells > MAX_CELLS as f64 {
            p.cell * (cells / MAX_CELLS as f64).sqrt()
        } else {
            p.cell
        }
    };
    let mut surface = Grid::covering([b[0], b[1], b[3], b[4]], cell)?;
    tin::rasterize(&ground, &mut surface);
    drop(ground);
    surface.fill_holes();
    crate::paths::ensure_parent(output)?;
    let opts = WriterOptions {
        extra: if p.replace_z {
            Vec::new()
        } else {
            vec![ExtraAttr {
                name: "HeightAboveGround".into(),
                description: "height above ground".into(),
                kind: ExtraKind::F32,
            }]
        },
        ..WriterOptions::default()
    };
    let mut w = Writer::create_with(output, &r, &opts)?;
    let h = r.header.clone();
    let (mut written, mut dropped, mut above) = (0u64, 0u64, 0u64);
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    crate::progress::begin(h.count);
    r.scan_raw(|bytes| {
        crate::check_cancel()?;
        let mut rec_out = vec![0u8; rec_len];
        for rec in bytes.chunks_exact(rec_len) {
            let Some(q) = r.decode(rec) else { continue };
            let ground_z = surface
                .sample(q.x, q.y)
                .filter(|_| surface.index(q.x, q.y).is_some());
            let height = ground_z.map(|g| (q.z - f64::from(g)) as f32);
            if let Some(v) = height {
                lo = lo.min(v);
                hi = hi.max(v);
                above += u64::from(v > 0.0);
            }
            if p.replace_z {
                let Some(v) = height else {
                    dropped += 1;
                    continue;
                };
                rec_out.copy_from_slice(rec);
                if !las::set_xyz(&h, &mut rec_out, q.x, q.y, f64::from(v)) {
                    dropped += 1;
                    continue;
                }
                w.write(&rec_out)?;
            } else {
                // sin suelo bajo el punto: no es un número, de modo que nada lo confunde con una
                // altura
                w.write_with_extra(rec, &height.unwrap_or(f32::NAN).to_le_bytes())?;
            }
            written += 1;
        }
        crate::progress::advance((bytes.len() / rec_len) as u64);
        Ok(())
    })?;
    w.finish()?;
    let lost = if dropped > 0 {
        format!(", {dropped} puntos sin suelo debajo descartados")
    } else {
        String::new()
    };
    Ok(format!(
        "{written} puntos con su altura sobre el suelo ({} sobre el suelo, de {:.2} a {:.2} m){lost} → {}",
        above,
        if lo.is_finite() { lo } else { 0.0 },
        if hi.is_finite() { hi } else { 0.0 },
        output.display()
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Un archivo LAS 1.2 de formato 1 con puntos (x, y, z, clase) en una rejilla milimétrica.
    pub(crate) fn write_las(path: &Path, pts: &[(f64, f64, f64, u8)]) -> Result<()> {
        let mut h = vec![0u8; 227];
        h[..4].copy_from_slice(b"LASF");
        h[24] = 1;
        h[25] = 2;
        h[94..96].copy_from_slice(&227u16.to_le_bytes());
        h[96..100].copy_from_slice(&227u32.to_le_bytes());
        h[104] = 1;
        h[105..107].copy_from_slice(&28u16.to_le_bytes());
        h[107..111].copy_from_slice(&(pts.len() as u32).to_le_bytes());
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in pts {
            for (k, v) in [p.0, p.1, p.2].into_iter().enumerate() {
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
        }
        for (at, v) in [
            (131, 0.001),
            (139, 0.001),
            (147, 0.001),
            (155, 0.0),
            (163, 0.0),
            (171, 0.0),
        ] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for (at, v) in [
            (179, hi[0]),
            (187, lo[0]),
            (195, hi[1]),
            (203, lo[1]),
            (211, hi[2]),
            (219, lo[2]),
        ] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for p in pts {
            let mut rec = [0u8; 28];
            for (k, v) in [p.0, p.1, p.2].into_iter().enumerate() {
                rec[4 * k..4 * k + 4].copy_from_slice(&((v / 0.001).round() as i32).to_le_bytes());
            }
            rec[14] = 1 | (1 << 3);
            rec[15] = p.3;
            h.extend_from_slice(&rec);
        }
        std::fs::write(path, h)?;
        Ok(())
    }

    fn scene() -> Vec<(f64, f64, f64, u8)> {
        let ground = |x: f64, y: f64| 100.0 + 0.1 * x + 0.05 * y;
        let mut pts = Vec::new();
        for i in 0..80 {
            for j in 0..80 {
                let (x, y) = (f64::from(i) * 0.5, f64::from(j) * 0.5);
                // un objeto de 5 m sobre el medio, con suelo solo alrededor
                if (15.0..25.0).contains(&x) && (15.0..25.0).contains(&y) {
                    pts.push((x, y, ground(x, y) + 5.0, 1));
                } else {
                    pts.push((x, y, ground(x, y), 2));
                }
            }
        }
        pts
    }

    #[test]
    fn heights_above_the_ground_of_the_cloud_itself() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_hag_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        write_las(&dir.join("in.las"), &scene())?;
        // como atributo adicional: las elevaciones se quedan
        let p = HagParams {
            cell: 0.5,
            ..HagParams::default()
        };
        let msg = run(&dir.join("in.las"), &dir.join("extra.las"), &p)?;
        assert!(msg.contains("6400 puntos"), "{msg}");
        let r = Reader::open(&dir.join("extra.las"))?;
        let len = usize::from(r.header.record_len);
        assert_eq!(len, 32);
        let (mut object, mut flat) = (Vec::new(), Vec::new());
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(len) {
                let q = r.decode(rec).ok_or_else(|| Error::Param("record".into()))?;
                let hag = f32::from_le_bytes([rec[28], rec[29], rec[30], rec[31]]);
                if q.class == 1 {
                    object.push(hag);
                } else {
                    flat.push(hag);
                }
            }
            Ok(())
        })?;
        assert_eq!(object.len(), 400);
        // la superficie bajo el objeto es el TIN a través del hueco: un plano, así que exacta
        assert!(
            object.iter().all(|h| (h - 5.0).abs() < 0.05),
            "{:?}",
            &object[..5]
        );
        // en el borde de la nube la superficie se rellena, desviada en aproximadamente la celda por
        // la pendiente
        assert!(flat.iter().all(|h| h.abs() < 0.06));
        // normalizada: la elevación es la altura
        let p = HagParams {
            replace_z: true,
            ..p
        };
        run(&dir.join("in.las"), &dir.join("norm.las"), &p)?;
        let mut max_z = f64::NEG_INFINITY;
        Reader::open(&dir.join("norm.las"))?.scan(|q| max_z = max_z.max(q.z))?;
        assert!((max_z - 5.0).abs() < 0.05, "{max_z}");
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn a_cloud_without_ground_is_refused() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_hag_none_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let pts: Vec<_> = scene()
            .into_iter()
            .map(|(x, y, z, _)| (x, y, z, 1))
            .collect();
        write_las(&dir.join("in.las"), &pts)?;
        let e = run(
            &dir.join("in.las"),
            &dir.join("out.las"),
            &HagParams::default(),
        )
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        assert!(e.contains("clase 2"), "{e}");
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
