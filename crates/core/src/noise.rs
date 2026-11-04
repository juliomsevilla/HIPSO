//! Isolated-point noise filter, the usual criterion: a point is noise when the
//! 3 x 3 x 3 block of cells centred on its own cell holds at most `isolated` other points.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::las::Point;

#[derive(Clone, Debug, PartialEq)]
pub struct NoiseParams {
    pub step_xy: f64,
    pub step_z: f64,
    pub isolated: u32,
}

impl Default for NoiseParams {
    fn default() -> Self {
        Self {
            step_xy: 4.0,
            step_z: 4.0,
            isolated: 5,
        }
    }
}

/// A hasher for cell keys that are already well spread integers.
#[derive(Default)]
pub(crate) struct CellHasher(u64);

impl Hasher for CellHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    fn write_u64(&mut self, k: u64) {
        // splitmix64's finaliser.
        let mut z = k.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        self.0 = z ^ (z >> 31);
    }
}

pub(crate) type CellMap<V> = HashMap<u64, V, BuildHasherDefault<CellHasher>>;

/// Packs cell coordinates (each within ±2^20) into one key.
#[inline]
fn key(x: i64, y: i64, z: i64) -> u64 {
    let m = |v: i64| (v + (1 << 20)) as u64 & 0x1F_FFFF;
    (m(x) << 42) | (m(y) << 21) | m(z)
}

/// Which points are isolated. Points already marked noise, or withheld, take no part.
pub fn isolated(pts: &[Point], p: &NoiseParams) -> Vec<bool> {
    let Some(first) = pts.first() else {
        return Vec::new();
    };
    // Cells are counted from the first point, so keys stay small; the alignment is the step's.
    let (ox, oy, oz) = (
        (first.x / p.step_xy).floor(),
        (first.y / p.step_xy).floor(),
        (first.z / p.step_z).floor(),
    );
    let cell = |pt: &Point| {
        (
            (pt.x / p.step_xy).floor() as i64 - ox as i64,
            (pt.y / p.step_xy).floor() as i64 - oy as i64,
            (pt.z / p.step_z).floor() as i64 - oz as i64,
        )
    };
    let mut counts: CellMap<u32> = CellMap::default();
    counts.reserve(pts.len() / 4);
    for pt in pts.iter().filter(|pt| !pt.is_noise()) {
        let (x, y, z) = cell(pt);
        *counts.entry(key(x, y, z)).or_insert(0) += 1;
    }
    pts.iter()
        .map(|pt| {
            if pt.is_noise() {
                return false;
            }
            let (x, y, z) = cell(pt);
            let mut n = 0u32;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        n += counts
                            .get(&key(x + dx, y + dy, z + dz))
                            .copied()
                            .unwrap_or(0);
                    }
                }
            }
            // The point itself is in its own cell.
            n.saturating_sub(1) <= p.isolated
        })
        .collect()
}

/// Extended local minimum for low noise. The points are binned in square cells of `cell` metres
/// and the lowest point of each cell is its local minimum. That point is low noise when it lies
/// at least `drop` metres under the lower quartile of the heights of its own cell (it stands
/// apart from what is above it; a burst of up to a quarter of the cell's points is caught too)
/// and also at least `drop` metres under the median of the minima of the cells around
/// it (it is not simply where the ground is low, as in a gully, or a cell whose ground returns
/// are missing). The flagged points are left out and the lowest point of each cell is looked
/// for again, until nothing more is flagged (at most 20 rounds), so a burst of low points in
/// one cell is taken off layer by layer. A cell needs at least four points and four neighbours
/// with points to be judged; those at the edge of the data are left alone.
pub fn elm(pts: &[Point], cell: f64, drop: f64) -> Vec<bool> {
    let mut out = vec![false; pts.len()];
    if !(cell > 0.0) || !(drop > 0.0) {
        return out;
    }
    let mut cells: CellMap<((i64, i64), Vec<u32>)> = CellMap::default();
    for (i, p) in pts.iter().enumerate() {
        if p.is_noise() {
            continue;
        }
        let (cx, cy) = ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
        cells.entry(key(cx, cy, 0)).or_insert(((cx, cy), Vec::new())).1.push(i as u32);
    }
    // each cell's points with the lowest last, so that the next minimum is the next point
    let mut at: Vec<(i64, i64)> = Vec::new();
    let mut members: Vec<Vec<u32>> = Vec::new();
    for (xy, mut m) in cells.into_values() {
        m.sort_unstable_by(|&a, &b| pts[b as usize].z.total_cmp(&pts[a as usize].z));
        at.push(xy);
        members.push(m);
    }
    let index: CellMap<usize> = at.iter().enumerate().map(|(i, &(x, y))| (key(x, y, 0), i)).collect();
    let z_of = |i: u32| pts[i as usize].z;
    for _ in 0..20 {
        let mut flagged = Vec::new();
        for (c, &(x, y)) in at.iter().enumerate() {
            let m = &members[c];
            let n = m.len();
            if n < 4 {
                continue;
            }
            let z1 = z_of(m[n - 1]);
            if z_of(m[n - 1 - n / 4]) - z1 < drop {
                continue;
            }
            let mut around: Vec<f64> = Vec::with_capacity(8);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if (dx, dy) != (0, 0)
                        && let Some(&j) = index.get(&key(x + dx, y + dy, 0))
                        && let Some(&i) = members[j].last()
                    {
                        around.push(z_of(i));
                    }
                }
            }
            if around.len() < 4 {
                continue;
            }
            around.sort_unstable_by(f64::total_cmp);
            let k = around.len();
            let median = if k % 2 == 1 { around[k / 2] } else { (around[k / 2 - 1] + around[k / 2]) / 2.0 };
            if median - z1 >= drop {
                flagged.push(c);
            }
        }
        if flagged.is_empty() {
            break;
        }
        for c in flagged {
            if let Some(i) = members[c].pop() {
                out[i as usize] = true;
            }
        }
    }
    out
}

/// Statistical outlier removal (Rusu et al. 2008): a point is noise when its mean distance to
/// its `k` nearest neighbours exceeds the mean of that distance over all points by more than
/// `m` standard deviations. Neighbours are found in a hash grid of cubes, so the cost grows
/// with the points, not their square.
pub fn sor(pts: &[Point], k: usize, m: f64) -> Vec<bool> {
    let idx: Vec<usize> = (0..pts.len()).filter(|&i| !pts[i].is_noise()).collect();
    let mut out = vec![false; pts.len()];
    if idx.len() <= k || k == 0 {
        return out;
    }
    let b = idx.iter().fold(
        [f64::INFINITY; 3]
            .into_iter()
            .chain([f64::NEG_INFINITY; 3])
            .collect::<Vec<_>>(),
        |mut b, &i| {
            let q = &pts[i];
            b[0] = b[0].min(q.x);
            b[1] = b[1].min(q.y);
            b[2] = b[2].min(q.z);
            b[3] = b[3].max(q.x);
            b[4] = b[4].max(q.y);
            b[5] = b[5].max(q.z);
            b
        },
    );
    // Cubes holding about k points on average, in the cloud's (mostly flat) layer.
    let area = ((b[3] - b[0]) * (b[4] - b[1])).max(1e-6);
    let s = (area * k as f64 / idx.len() as f64).sqrt().max(1e-3);
    let cell = |q: &Point| {
        (
            ((q.x - b[0]) / s).floor() as i64,
            ((q.y - b[1]) / s).floor() as i64,
            ((q.z - b[2]) / s).floor() as i64,
        )
    };
    let mut cubes: CellMap<Vec<u32>> = CellMap::default();
    for &i in &idx {
        let (x, y, z) = cell(&pts[i]);
        cubes.entry(key(x, y, z)).or_default().push(i as u32);
    }
    let mut best: Vec<f64> = Vec::with_capacity(k + 1);
    let mean_d: Vec<f64> = idx
        .iter()
        .map(|&i| {
            let q = &pts[i];
            let (cx, cy, cz) = cell(q);
            best.clear();
            // Shells of cubes outward until the k-th distance is inside the searched radius.
            for ring in 0i64.. {
                for dx in -ring..=ring {
                    for dy in -ring..=ring {
                        for dz in -ring..=ring {
                            if dx.abs().max(dy.abs()).max(dz.abs()) != ring {
                                continue;
                            }
                            for &j in cubes
                                .get(&key(cx + dx, cy + dy, cz + dz))
                                .into_iter()
                                .flatten()
                            {
                                if j as usize == i {
                                    continue;
                                }
                                let o = &pts[j as usize];
                                let d = ((o.x - q.x).powi(2)
                                    + (o.y - q.y).powi(2)
                                    + (o.z - q.z).powi(2))
                                .sqrt();
                                if best.len() < k || d < best[k - 1] {
                                    let at = best.partition_point(|&v| v < d);
                                    best.insert(at, d);
                                    best.truncate(k);
                                }
                            }
                        }
                    }
                }
                if best.len() == k && best[k - 1] <= ring as f64 * s {
                    break;
                }
                if ring > 64 {
                    break;
                }
            }
            best.iter().sum::<f64>() / best.len().max(1) as f64
        })
        .collect();
    let n = mean_d.len() as f64;
    let mu = mean_d.iter().sum::<f64>() / n;
    let sd = (mean_d.iter().map(|d| (d - mu).powi(2)).sum::<f64>() / n).sqrt();
    for (&i, d) in idx.iter().zip(&mean_d) {
        out[i] = *d > mu + m * sd;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sor_flags_the_stray_point() {
        let mut pts: Vec<Point> = (0..400)
            .map(|i| pt(f64::from(i % 20) * 0.5, f64::from(i / 20) * 0.5, 0.0))
            .collect();
        pts.push(pt(5.0, 5.0, 30.0));
        let noise = sor(&pts, 10, 3.0);
        assert!(noise[400]);
        assert!(noise[..400].iter().all(|n| !n));
    }

    fn pt(x: f64, y: f64, z: f64) -> Point {
        Point {
            x,
            y,
            z,
            return_number: 1,
            number_of_returns: 1,
            class: 1,
            ..Default::default()
        }
    }

    #[test]
    fn elm_flags_deep_low_points_and_leaves_flat_ground_and_slopes() {
        // gently noisy ground over two 20 m cells, three deep low points in the first
        let mut pts: Vec<Point> = (0..800)
            .map(|i| {
                let (x, y) = (f64::from(i % 40) * 1.0, f64::from(i / 40) * 1.0);
                pt(x, y, 100.0 + 0.1 * f64::from((i * 7919) % 11) / 11.0)
            })
            .collect();
        for k in 0..3 {
            pts.push(pt(5.0 + f64::from(k), 5.0, 80.0));
        }
        let low = elm(&pts, 5.0, 3.0);
        assert!(low[800] && low[801] && low[802]);
        assert!(low[..800].iter().all(|n| !n));
        // a plain slope inside one cell is not noise at a generous threshold
        let slope: Vec<Point> = (0..400).map(|i| pt(f64::from(i % 20) * 0.9, f64::from(i / 20), f64::from(i % 20) * 0.1)).collect();
        assert!(elm(&slope, 20.0, 3.0).iter().all(|n| !n));
    }

    #[test]
    fn a_lone_point_above_a_crowd_is_noise() {
        let mut pts: Vec<Point> = (0..100)
            .map(|i| pt(f64::from(i % 10), f64::from(i / 10), 0.0))
            .collect();
        pts.push(pt(5.0, 5.0, 60.0));
        let noise = isolated(&pts, &NoiseParams::default());
        assert!(noise[100]);
        assert!(noise[..100].iter().all(|n| !n));
    }
}
