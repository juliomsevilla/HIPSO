//! The layers an object analysis works on besides the rasters it is given: statistics of the
//! points in each cell (read once, whatever the number of layers asked for) and the local
//! variation of a layer.

use std::path::PathBuf;

use rayon::prelude::*;

use super::{Lattice, Layer};
use crate::filter::Filter;
use crate::las::Reader;
use crate::{Result, check_cancel};

/// A per-cell statistic of the points, on the lattice of the analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudLayer {
    /// Mean intensity.
    Intensity,
    /// Mean number of returns of the pulses the points belong to: high where the light
    /// crosses the canopy, 1 on roofs and bare ground.
    Returns,
    /// Share of the points classified as ground (class 2).
    Ground,
    /// Points per square map unit.
    Density,
}

impl CloudLayer {
    pub fn name(self) -> &'static str {
        match self {
            CloudLayer::Intensity => "intensidad",
            CloudLayer::Returns => "retornos",
            CloudLayer::Ground => "suelo",
            CloudLayer::Density => "densidad",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "intensidad" | "intensity" => Some(Self::Intensity),
            "retornos" | "returns" => Some(Self::Returns),
            "suelo" | "ground" => Some(Self::Ground),
            "densidad" | "density" => Some(Self::Density),
            _ => None,
        }
    }
}

/// The asked-for statistics of the points of `inputs` in every cell of `lat`, in one pass over
/// the clouds. Cells without points are NaN (density: 0).
pub fn cloud_layers(
    inputs: &[PathBuf],
    lat: &Lattice,
    wanted: &[CloudLayer],
    filter: &Filter,
) -> Result<Vec<Layer>> {
    let n = lat.len();
    let has = |l: CloudLayer| wanted.contains(&l);
    let mut count = vec![0u32; n];
    let mut sum_i = has(CloudLayer::Intensity).then(|| vec![0f32; n]);
    let mut sum_r = has(CloudLayer::Returns).then(|| vec![0f32; n]);
    let mut ground = has(CloudLayer::Ground).then(|| vec![0u32; n]);
    for input in inputs {
        let reader = Reader::open(input)?;
        let mut seen = 0u64;
        reader.scan(|p| {
            if !filter.keeps(p) || !filter.z_ok(p.z) {
                return;
            }
            seen += 1;
            if filter.decimate > 1 && !(seen - 1).is_multiple_of(filter.decimate) {
                return;
            }
            let Some(i) = lat.index(p.x, p.y) else { return };
            count[i] += 1;
            if let Some(s) = sum_i.as_mut() {
                s[i] += f32::from(p.intensity);
            }
            if let Some(s) = sum_r.as_mut() {
                s[i] += f32::from(p.number_of_returns.max(1));
            }
            if let Some(g) = ground.as_mut()
                && p.class == 2
            {
                g[i] += 1;
            }
        })?;
        check_cancel()?;
    }
    let area = lat.cell * lat.cell;
    let mut out = Vec::new();
    for &w in wanted {
        let data: Vec<f32> = match w {
            CloudLayer::Density => count
                .iter()
                .map(|&c| (f64::from(c) / area) as f32)
                .collect(),
            CloudLayer::Intensity => mean_of(&count, sum_i.as_deref()),
            CloudLayer::Returns => mean_of(&count, sum_r.as_deref()),
            CloudLayer::Ground => count
                .iter()
                .zip(ground.iter().flatten())
                .map(|(&c, &g)| {
                    if c == 0 {
                        f32::NAN
                    } else {
                        g as f32 / c as f32
                    }
                })
                .collect(),
        };
        out.push(Layer {
            name: w.name().to_string(),
            data,
        });
    }
    Ok(out)
}

fn mean_of(count: &[u32], sum: Option<&[f32]>) -> Vec<f32> {
    match sum {
        Some(s) => count
            .iter()
            .zip(s)
            .map(|(&c, &v)| if c == 0 { f32::NAN } else { v / c as f32 })
            .collect(),
        None => vec![f32::NAN; count.len()],
    }
}

/// Fills each empty (NaN) cell with the value of the nearest cell that has one, when that is at
/// most `max` cells away; farther cells stay empty. The nearest cell is found by two sweeps over
/// the lattice that carry the best seed along, so the cost is linear and does not depend on
/// `max`.
pub fn fill_empty(lat: &Lattice, data: &mut [f32], max: usize) {
    let (cols, rows) = (lat.cols, lat.rows);
    if max == 0 || data.len() != cols * rows || cols == 0 {
        return;
    }
    let mut seed = vec![u32::MAX; data.len()];
    for (s, (i, v)) in seed.iter_mut().zip(data.iter().enumerate()) {
        if !v.is_nan() {
            *s = i as u32;
        }
    }
    let d2 = |i: usize, s: u32| -> u64 {
        if s == u32::MAX {
            return u64::MAX;
        }
        let (x, y, sx, sy) = ((i % cols) as i64, (i / cols) as i64, (s as usize % cols) as i64, (s as usize / cols) as i64);
        ((x - sx) * (x - sx) + (y - sy) * (y - sy)) as u64
    };
    let sweep = |seed: &mut Vec<u32>, order: &mut dyn Iterator<Item = usize>, back: bool| {
        for i in order {
            let (x, y) = (i % cols, i / cols);
            let mut best = seed[i];
            let mut best_d = d2(i, best);
            let try_nb = |j: usize, best: &mut u32, best_d: &mut u64| {
                let s = seed[j];
                let d = d2(i, s);
                if d < *best_d {
                    (*best, *best_d) = (s, d);
                }
            };
            if !back {
                if x > 0 {
                    try_nb(i - 1, &mut best, &mut best_d);
                }
                if y > 0 {
                    try_nb(i - cols, &mut best, &mut best_d);
                    if x > 0 {
                        try_nb(i - cols - 1, &mut best, &mut best_d);
                    }
                    if x + 1 < cols {
                        try_nb(i - cols + 1, &mut best, &mut best_d);
                    }
                }
            } else {
                if x + 1 < cols {
                    try_nb(i + 1, &mut best, &mut best_d);
                }
                if y + 1 < rows {
                    try_nb(i + cols, &mut best, &mut best_d);
                    if x + 1 < cols {
                        try_nb(i + cols + 1, &mut best, &mut best_d);
                    }
                    if x > 0 {
                        try_nb(i + cols - 1, &mut best, &mut best_d);
                    }
                }
            }
            seed[i] = best;
        }
    };
    sweep(&mut seed, &mut (0..data.len()), false);
    sweep(&mut seed, &mut (0..data.len()).rev(), true);
    let limit = (max as u64) * (max as u64);
    let values: Vec<f32> = data.to_vec();
    for (i, v) in data.iter_mut().enumerate() {
        if v.is_nan() && seed[i] != u32::MAX && d2(i, seed[i]) <= limit {
            *v = values[seed[i] as usize];
        }
    }
}

/// The mean of a layer in the square window of `radius` cells around each cell (clipped at the
/// borders, empty cells left out). A cell with no value in its window stays empty. This evens
/// out the cell-to-cell noise of statistics made from few points.
pub fn local_mean(lat: &Lattice, data: &[f32], radius: usize) -> Vec<f32> {
    let (cols, rows) = (lat.cols, lat.rows);
    let r = radius.max(1);
    if data.len() != cols * rows || cols == 0 {
        return data.to_vec();
    }
    // horizontal sums, then vertical ones, of the values and of how many there are
    let mut hs = vec![0f64; data.len()];
    let mut hc = vec![0u32; data.len()];
    hs.par_chunks_mut(cols)
        .zip(hc.par_chunks_mut(cols))
        .zip(data.par_chunks(cols))
        .for_each(|((s, c), row)| {
            for x in 0..cols {
                let (lo, hi) = (x.saturating_sub(r), (x + r).min(cols - 1));
                let (mut t, mut n) = (0f64, 0u32);
                for v in &row[lo..=hi] {
                    if !v.is_nan() {
                        t += f64::from(*v);
                        n += 1;
                    }
                }
                s[x] = t;
                c[x] = n;
            }
        });
    let mut out = vec![f32::NAN; data.len()];
    out.par_chunks_mut(cols).enumerate().for_each(|(y, row)| {
        let (lo, hi) = (y.saturating_sub(r), (y + r).min(rows - 1));
        for (x, o) in row.iter_mut().enumerate() {
            let (mut t, mut n) = (0f64, 0u32);
            for yy in lo..=hi {
                t += hs[yy * cols + x];
                n += hc[yy * cols + x];
            }
            if n > 0 {
                *o = (t / f64::from(n)) as f32;
            }
        }
    });
    out
}

/// The standard deviation of a layer in the square window of `radius` cells around each cell
/// (clipped at the borders, empty cells left out): a roughness measure that tells a crown, whose
/// height varies from cell to cell, from a roof or a lawn. Two running-sum passes, so the cost
/// per cell does not depend on the radius.
pub fn local_std(lat: &Lattice, data: &[f32], radius: usize) -> Vec<f32> {
    let (cols, rows) = (lat.cols, lat.rows);
    let r = radius.max(1);
    // centred values keep the sums of squares small
    let (mut total, mut seen) = (0f64, 0u64);
    for v in data.iter().filter(|v| !v.is_nan()) {
        total += f64::from(*v);
        seen += 1;
    }
    let mean = if seen > 0 { total / seen as f64 } else { 0.0 };
    // horizontal window sums of the value, its square and the number of cells
    let mut hs = vec![0f32; data.len()];
    let mut hq = vec![0f32; data.len()];
    let mut hc = vec![0u32; data.len()];
    hs.par_chunks_mut(cols)
        .zip(hq.par_chunks_mut(cols))
        .zip(hc.par_chunks_mut(cols))
        .zip(data.par_chunks(cols))
        .for_each(|(((s, q), c), row)| {
            let (mut ws, mut wq, mut wc) = (0f64, 0f64, 0u32);
            let add = |k: usize, ws: &mut f64, wq: &mut f64, wc: &mut u32, sign: f64| {
                let v = row[k];
                if !v.is_nan() {
                    let z = f64::from(v) - mean;
                    *ws += sign * z;
                    *wq += sign * z * z;
                    *wc = if sign > 0.0 { *wc + 1 } else { *wc - 1 };
                }
            };
            for k in 0..r.min(cols) {
                add(k, &mut ws, &mut wq, &mut wc, 1.0);
            }
            for x in 0..cols {
                if x + r < cols {
                    add(x + r, &mut ws, &mut wq, &mut wc, 1.0);
                }
                if x > r {
                    add(x - r - 1, &mut ws, &mut wq, &mut wc, -1.0);
                }
                s[x] = ws as f32;
                q[x] = wq as f32;
                c[x] = wc;
            }
        });
    // vertical sums, band by band so the bands run in parallel
    let mut out = vec![f32::NAN; data.len()];
    const BAND: usize = 64;
    out.par_chunks_mut(BAND * cols)
        .enumerate()
        .for_each(|(b, chunk)| {
            let first = b * BAND;
            let last = (first + BAND).min(rows);
            let (mut vs, mut vq) = (vec![0f64; cols], vec![0f64; cols]);
            let mut vc = vec![0i64; cols];
            let apply = |y: usize, sign: i64, vs: &mut [f64], vq: &mut [f64], vc: &mut [i64]| {
                let o = y * cols;
                for x in 0..cols {
                    vs[x] += sign as f64 * f64::from(hs[o + x]);
                    vq[x] += sign as f64 * f64::from(hq[o + x]);
                    vc[x] += sign * i64::from(hc[o + x]);
                }
            };
            // the window of the band's first row, then slide it down a row at a time
            for y in first.saturating_sub(r)..=(first + r).min(rows - 1) {
                apply(y, 1, &mut vs, &mut vq, &mut vc);
            }
            for y in first..last {
                if y > first {
                    if y + r < rows {
                        apply(y + r, 1, &mut vs, &mut vq, &mut vc);
                    }
                    if y > r {
                        apply(y - r - 1, -1, &mut vs, &mut vq, &mut vc);
                    }
                }
                let o = (y - first) * cols;
                for x in 0..cols {
                    if data[y * cols + x].is_nan() || vc[x] < 1 {
                        continue;
                    }
                    let c = vc[x] as f64;
                    let var = (vq[x] / c - (vs[x] / c).powi(2)).max(0.0);
                    chunk[o + x] = var.sqrt() as f32;
                }
            }
        });
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_cells_take_the_nearest_value_within_the_distance() {
        let lat = Lattice { x0: 0.0, y0: 0.0, cell: 1.0, cols: 6, rows: 1 };
        let mut d = vec![1.0, f32::NAN, f32::NAN, f32::NAN, f32::NAN, 9.0];
        fill_empty(&lat, &mut d, 2);
        assert_eq!(d, vec![1.0, 1.0, 1.0, 9.0, 9.0, 9.0]);
        // too far from any value: stays empty
        let mut e = vec![1.0, f32::NAN, f32::NAN, f32::NAN, f32::NAN, f32::NAN];
        fill_empty(&lat, &mut e, 2);
        assert!(e[3].is_nan() && e[2] == 1.0);
    }

    #[test]
    fn the_local_mean_leaves_out_empty_cells() {
        let lat = Lattice { x0: 0.0, y0: 0.0, cell: 1.0, cols: 3, rows: 3 };
        let mut d = vec![f32::NAN; 9];
        d[4] = 6.0;
        d[0] = 2.0;
        let m = local_mean(&lat, &d, 1);
        // the centre sees both values; the far corner sees only the centre
        assert!((m[4] - 4.0).abs() < 1e-6 && (m[8] - 6.0).abs() < 1e-6);
    }

    use super::*;

    fn lat(cols: usize, rows: usize) -> Lattice {
        Lattice {
            x0: 0.0,
            y0: 0.0,
            cell: 1.0,
            cols,
            rows,
        }
    }

    /// The same quantity the slow way.
    fn brute(l: &Lattice, d: &[f32], r: usize) -> Vec<f32> {
        let mut out = vec![f32::NAN; d.len()];
        for y in 0..l.rows {
            for x in 0..l.cols {
                if d[y * l.cols + x].is_nan() {
                    continue;
                }
                let mut v = Vec::new();
                for yy in y.saturating_sub(r)..=(y + r).min(l.rows - 1) {
                    for xx in x.saturating_sub(r)..=(x + r).min(l.cols - 1) {
                        let z = d[yy * l.cols + xx];
                        if !z.is_nan() {
                            v.push(f64::from(z));
                        }
                    }
                }
                let m = v.iter().sum::<f64>() / v.len() as f64;
                let var = v.iter().map(|z| (z - m).powi(2)).sum::<f64>() / v.len() as f64;
                out[y * l.cols + x] = var.sqrt() as f32;
            }
        }
        out
    }

    #[test]
    fn local_std_matches_the_direct_computation() {
        // taller than one band, with holes
        let l = lat(37, 150);
        let d: Vec<f32> = (0..l.len())
            .map(|i| {
                if i % 17 == 3 {
                    f32::NAN
                } else {
                    ((i * 7919) % 101) as f32 / 10.0
                }
            })
            .collect();
        for r in [1usize, 2, 5] {
            let fast = local_std(&l, &d, r);
            let slow = brute(&l, &d, r);
            for (i, (a, b)) in fast.iter().zip(&slow).enumerate() {
                assert!(
                    (a.is_nan() && b.is_nan()) || (a - b).abs() < 2e-3,
                    "r={r} cell {i}: {a} vs {b}"
                );
            }
        }
    }
}
