//! Adelgazamiento («diezmado») de una nube: menos puntos, la misma extensión. Sirve para poner
//! nubes de distinta densidad en pie de igualdad, por ejemplo la primera y la tercera cobertura
//! LiDAR de España (PNOA) antes de compararlas. Los métodos que no miran vecinos (uno de cada n,
//! aleatorio, densidad objetivo) funcionan en flujo punto a punto; los métodos por celda guardan un
//! registro por celda, de modo que la memoria sigue a la salida y no a la entrada. Solo el método
//! de percentil guarda una elevación por punto de entrada de las celdas que ordena (4 bytes cada
//! una).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::filter::Filter;
use crate::las::{Reader, Writer};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThinMethod {
    /// El punto más bajo de cada celda cuadrada.
    Lowest,
    /// El punto más alto de cada celda cuadrada.
    Highest,
    /// El punto más cercano al centro de cada celda: un espaciado uniforme.
    Nearest,
    /// El punto en este percentil (0–100) de las elevaciones de cada celda; 50 es la mediana.
    Percentile(f64),
    /// El punto más cercano al centro de cada celda 3D, cuya altura es `vertical`: conserva el
    /// espaciado en tres dimensiones, de modo que las paredes y las pendientes fuertes conservan
    /// sus puntos.
    Voxel { vertical: f64 },
    /// Conserva uno de cada n puntos del archivo.
    EveryNth(u64),
    /// Quita uno de cada n puntos del archivo.
    RemoveNth(u64),
    /// Conserva cada punto con esta probabilidad (0–1).
    Random { fraction: f64, seed: u64 },
    /// Adelgazamiento aleatorio que deja unos tantos puntos por metro cuadrado; una nube menos
    /// densa se deja como está.
    Density { per_m2: f64, seed: u64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThinParams {
    pub method: ThinMethod,
    /// Tamaño de celda en metros (métodos por celda).
    pub cell: f64,
}

impl ThinParams {
    /// Una etiqueta corta para nombres de archivo: `delgada_<etiqueta>`.
    pub fn tag(&self) -> String {
        let n = crate::paths::num;
        match self.method {
            ThinMethod::Lowest => format!("min_{}m", n(self.cell)),
            ThinMethod::Highest => format!("max_{}m", n(self.cell)),
            ThinMethod::Nearest => format!("centro_{}m", n(self.cell)),
            ThinMethod::Percentile(p) => format!("p{}_{}m", n(p), n(self.cell)),
            ThinMethod::Voxel { .. } => format!("3d_{}m", n(self.cell)),
            ThinMethod::EveryNth(k) => format!("cada{k}"),
            ThinMethod::RemoveNth(k) => format!("sin{k}"),
            ThinMethod::Random { fraction, .. } => format!("azar{}", n(fraction * 100.0)),
            ThinMethod::Density { per_m2, .. } => format!("{}ptsm2", n(per_m2)),
        }
    }
}

/// Splitmix64: un hash con semilla del índice del punto, de modo que un adelgazamiento aleatorio da
/// los mismos puntos sean cuales sean los tamaños de lote.
#[inline]
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

type Key = (i64, i64, i64);

/// Adelgaza `inputs` (unidas) en `output`. Devuelve (puntos leídos, puntos conservados).
pub fn thin_cloud(
    inputs: &[PathBuf],
    output: &Path,
    params: &ThinParams,
    filter: &Filter,
) -> Result<(u64, u64)> {
    crate::paths::guard_output(inputs, output)?;
    let first = inputs
        .first()
        .ok_or_else(|| Error::Param("no hay nube de entrada".into()))?;
    let unified = crate::crs::unify(inputs, false)?;
    let moved = unified.all(inputs);
    let (inputs, first) = (moved.as_slice(), &unified.path(first));
    let template = Reader::open(first)?;
    let cell_method = !matches!(
        params.method,
        ThinMethod::EveryNth(_)
            | ThinMethod::RemoveNth(_)
            | ThinMethod::Random { .. }
            | ThinMethod::Density { .. }
    );
    if cell_method && !(params.cell > 0.0) {
        return Err(Error::Param(
            "la celda de adelgazado debe ser positiva".into(),
        ));
    }
    match params.method {
        ThinMethod::EveryNth(k) | ThinMethod::RemoveNth(k) if k < 2 => {
            return Err(Error::Param("n debe ser 2 o más".into()));
        }
        ThinMethod::Percentile(p) if !(0.0..=100.0).contains(&p) => {
            return Err(Error::Param("el percentil está entre 0 y 100".into()));
        }
        ThinMethod::Voxel { vertical } if !(vertical > 0.0) => {
            return Err(Error::Param("la celda vertical debe ser positiva".into()));
        }
        ThinMethod::Random { fraction, .. } if !(fraction > 0.0 && fraction <= 1.0) => {
            return Err(Error::Param(
                "la fracción debe ser mayor que 0 y hasta 1".into(),
            ));
        }
        ThinMethod::Density { per_m2, .. } if !(per_m2 > 0.0) => {
            return Err(Error::Param("la densidad debe ser positiva".into()));
        }
        _ => {}
    }
    let mut readers = Vec::new();
    for path in inputs {
        let r = Reader::open(path)?;
        if r.header.record_len != template.header.record_len
            || r.header.format != template.header.format
        {
            return Err(Error::Param(format!(
                "{} tiene otro formato de punto que {}",
                path.display(),
                first.display()
            )));
        }
        readers.push(r.for_passes()?);
    }
    // el adelgazamiento aleatorio a una densidad necesita la densidad que tiene la nube
    let fraction = match params.method {
        ThinMethod::Random { fraction, .. } => fraction,
        ThinMethod::Density { per_m2, .. } => {
            let (mut n, mut area) = (0u64, 0.0);
            for r in &readers {
                let b = r.header.bounds;
                n += r.header.count;
                area += (b[3] - b[0]).max(0.0) * (b[4] - b[1]).max(0.0);
            }
            let have = if area > 0.0 { n as f64 / area } else { 0.0 };
            if have > 0.0 {
                (per_m2 / have).min(1.0)
            } else {
                1.0
            }
        }
        _ => 1.0,
    };
    let seed = match params.method {
        ThinMethod::Random { seed, .. } | ThinMethod::Density { seed, .. } => seed,
        _ => 0,
    };
    let (cell, vcell) = (
        params.cell,
        match params.method {
            ThinMethod::Voxel { vertical } => vertical,
            _ => 1.0,
        },
    );
    // percentil: la elevación que se pide a cada celda
    let targets: HashMap<Key, f32> = if let ThinMethod::Percentile(pct) = params.method {
        let mut zs: HashMap<Key, Vec<f32>> = HashMap::new();
        for r in &readers {
            let rec_len = usize::from(r.header.record_len);
            r.scan_raw(|bytes| {
                for rec in bytes.chunks_exact(rec_len) {
                    let Some(p) = r.decode(rec) else { continue };
                    if filter.keeps(&p) && filter.z_ok(p.z) {
                        zs.entry(cell_of(p.x, p.y, 0.0, cell, 1.0))
                            .or_default()
                            .push(p.z as f32);
                    }
                }
                Ok(())
            })?;
        }
        zs.into_iter()
            .map(|(k, mut v)| {
                v.sort_unstable_by(f32::total_cmp);
                let at = ((v.len() - 1) as f64 * pct / 100.0).round() as usize;
                (k, v.get(at).copied().unwrap_or(0.0))
            })
            .collect()
    } else {
        HashMap::new()
    };

    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create(output, &template)?;
    let mut best: HashMap<Key, (f64, Vec<u8>)> = HashMap::new();
    let (mut read, mut seen, mut kept) = (0u64, 0u64, 0u64);
    for r in &readers {
        let rec_len = usize::from(r.header.record_len);
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                read += 1;
                let Some(p) = r.decode(rec) else { continue };
                if !filter.keeps(&p) || !filter.z_ok(p.z) {
                    continue;
                }
                seen += 1;
                if filter.decimate > 1 && (seen - 1) % filter.decimate != 0 {
                    continue;
                }
                let score = match params.method {
                    ThinMethod::EveryNth(k) => {
                        if seen % k == 0 {
                            w.write(rec)?;
                            kept += 1;
                        }
                        continue;
                    }
                    ThinMethod::RemoveNth(k) => {
                        if seen % k != 0 {
                            w.write(rec)?;
                            kept += 1;
                        }
                        continue;
                    }
                    ThinMethod::Random { .. } | ThinMethod::Density { .. } => {
                        if (mix(seen ^ seed.wrapping_mul(0xA24B_AED4_963E_E407)) >> 11) as f64
                            / (1u64 << 53) as f64
                            <= fraction
                        {
                            w.write(rec)?;
                            kept += 1;
                        }
                        continue;
                    }
                    ThinMethod::Lowest => p.z,
                    ThinMethod::Highest => -p.z,
                    ThinMethod::Nearest => centre_dist(p.x, p.y, None, cell, 1.0),
                    ThinMethod::Voxel { .. } => centre_dist(p.x, p.y, Some(p.z), cell, vcell),
                    ThinMethod::Percentile(_) => {
                        let key = cell_of(p.x, p.y, 0.0, cell, 1.0);
                        (p.z - f64::from(targets.get(&key).copied().unwrap_or(0.0))).abs()
                    }
                };
                let key = if matches!(params.method, ThinMethod::Voxel { .. }) {
                    cell_of(p.x, p.y, p.z, cell, vcell)
                } else {
                    cell_of(p.x, p.y, 0.0, cell, 1.0)
                };
                match best.get_mut(&key) {
                    Some(e) if score < e.0 => *e = (score, rec.to_vec()),
                    Some(_) => {}
                    None => {
                        best.insert(key, (score, rec.to_vec()));
                    }
                }
            }
            Ok(())
        })?;
    }
    if !best.is_empty() {
        let mut keys: Vec<_> = best.keys().copied().collect();
        keys.sort_unstable();
        for k in &keys {
            if let Some(e) = best.get(k) {
                w.write(&e.1)?;
                kept += 1;
            }
        }
    }
    w.finish()?;
    Ok((read, kept))
}

#[inline]
fn cell_of(x: f64, y: f64, z: f64, cell: f64, vcell: f64) -> Key {
    (
        (x / cell).floor() as i64,
        (y / cell).floor() as i64,
        (z / vcell).floor() as i64,
    )
}

/// Distancia al cuadrado, en celdas, del punto al centro de su celda.
#[inline]
fn centre_dist(x: f64, y: f64, z: Option<f64>, cell: f64, vcell: f64) -> f64 {
    let f = |v: f64, c: f64| {
        let t = v / c;
        (t - t.floor() - 0.5).powi(2)
    };
    f(x, cell) + f(y, cell) + z.map_or(0.0, |z| f(z, vcell))
}

/// La línea de informe de un adelgazamiento.
pub fn report(read: u64, kept: u64, output: &Path) -> String {
    format!(
        "{read} puntos leídos, {kept} conservados ({:.1} %): {}",
        kept as f64 / read.max(1) as f64 * 100.0,
        output.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_is_uniform_enough() {
        let n = (0..100_000u64)
            .filter(|i| (mix(*i) >> 11) as f64 / (1u64 << 53) as f64 <= 0.25)
            .count();
        assert!((24_000..26_000).contains(&n), "{n}");
    }

    #[test]
    fn centre_distance_is_zero_at_the_centre() {
        assert!(centre_dist(0.5, 0.5, None, 1.0, 1.0) < 1e-12);
        assert!(centre_dist(0.0, 0.0, None, 1.0, 1.0) > 0.4);
    }
}
