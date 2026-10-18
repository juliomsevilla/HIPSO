//! Thinning ("decimating") a cloud: fewer points, same extent. Used to bring clouds of different
//! density to the same footing, for example the first and the third LiDAR coverage of Spain
//! (PNOA) before comparing them. Methods that look at no neighbours (every n-th, random, target
//! density) stream point by point; the cell methods keep one record per cell, so the memory
//! follows the output and not the input. Only the percentile method holds an elevation per
//! input point of the cells it ranks (4 bytes each).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::filter::Filter;
use crate::las::{Reader, Writer};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThinMethod {
    /// The lowest point of each square cell.
    Lowest,
    /// The highest point of each square cell.
    Highest,
    /// The point nearest to the centre of each cell: an even spacing.
    Nearest,
    /// The point at this percentile (0–100) of the elevations of each cell; 50 is the median.
    Percentile(f64),
    /// The point nearest to the centre of each 3D cell, whose height is `vertical`: keeps the
    /// spacing in three dimensions, so walls and steep slopes keep their points.
    Voxel { vertical: f64 },
    /// Keeps every n-th point of the file.
    EveryNth(u64),
    /// Removes every n-th point of the file.
    RemoveNth(u64),
    /// Keeps each point with this probability (0–1).
    Random { fraction: f64, seed: u64 },
    /// Random thinning that leaves about this many points per square metre; a cloud that is
    /// less dense is left as it is.
    Density { per_m2: f64, seed: u64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThinParams {
    pub method: ThinMethod,
    /// Cell size in metres (cell methods).
    pub cell: f64,
}

impl ThinParams {
    /// A short tag for file names: `delgada_<tag>`.
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

/// Splitmix64: a seeded hash of the point index, so a random thinning gives the same points
/// whatever the batch sizes.
#[inline]
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

type Key = (i64, i64, i64);

/// Thins `inputs` (joined) into `output`. Returns (points read, points kept).
pub fn thin_cloud(
    inputs: &[PathBuf],
    output: &Path,
    params: &ThinParams,
    filter: &Filter,
) -> Result<(u64, u64)> {
    let first = inputs
        .first()
        .ok_or_else(|| Error::Param("no input cloud".into()))?;
    let template = Reader::open(first)?;
    let cell_method = !matches!(
        params.method,
        ThinMethod::EveryNth(_)
            | ThinMethod::RemoveNth(_)
            | ThinMethod::Random { .. }
            | ThinMethod::Density { .. }
    );
    if cell_method && !(params.cell > 0.0) {
        return Err(Error::Param("the thinning cell must be positive".into()));
    }
    match params.method {
        ThinMethod::EveryNth(k) | ThinMethod::RemoveNth(k) if k < 2 => {
            return Err(Error::Param("n must be 2 or more".into()));
        }
        ThinMethod::Percentile(p) if !(0.0..=100.0).contains(&p) => {
            return Err(Error::Param("the percentile is between 0 and 100".into()));
        }
        ThinMethod::Voxel { vertical } if !(vertical > 0.0) => {
            return Err(Error::Param("the vertical cell must be positive".into()));
        }
        ThinMethod::Random { fraction, .. } if !(fraction > 0.0 && fraction <= 1.0) => {
            return Err(Error::Param("the fraction is above 0 and up to 1".into()));
        }
        ThinMethod::Density { per_m2, .. } if !(per_m2 > 0.0) => {
            return Err(Error::Param("the density must be positive".into()));
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
                "{} has another point format than {}",
                path.display(),
                first.display()
            )));
        }
        readers.push(r.for_passes()?);
    }
    // random thinning to a density needs the density the cloud has
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
    // percentile: the elevation each cell is asked for
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

/// Squared distance, in cells, from the point to the centre of its cell.
#[inline]
fn centre_dist(x: f64, y: f64, z: Option<f64>, cell: f64, vcell: f64) -> f64 {
    let f = |v: f64, c: f64| {
        let t = v / c;
        (t - t.floor() - 0.5).powi(2)
    };
    f(x, cell) + f(y, cell) + z.map_or(0.0, |z| f(z, vcell))
}

/// The report line of a thinning.
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
