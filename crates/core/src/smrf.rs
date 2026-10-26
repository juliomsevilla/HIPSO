//! Simple Morphological Filter for ground points (Pingel, Clarke & McBride 2013, "An improved
//! simple morphological filter for the terrain classification of airborne LIDAR data").
//!
//! On a grid of the lowest point per cell, openings with windows that grow one cell at a time
//! flag the cells that drop by more than the terrain slope allows for that window: they are
//! objects (trees, buildings, cars). The rest, filled where empty, is a provisional ground
//! surface, and a point is ground when it lies within a threshold of it, a threshold that grows
//! with the slope of the surface. Every opening costs O(cells), whatever the window. Works in
//! memory: run it on tiles.

use crate::grid::Grid;
use crate::las::Point;

#[derive(Clone, Debug, PartialEq)]
pub struct SmrfParams {
    /// Cell of the surfaces, in map units (about the point spacing of the lowest returns).
    pub cell: f64,
    /// Largest window radius in map units: bigger than the biggest building.
    pub max_window: f64,
    /// The steepest slope (rise over run) of the terrain: how much a cell may drop between the
    /// surface and its opening before it counts as an object.
    pub slope: f64,
    /// Elevation threshold between a point and the provisional surface, in map units.
    pub threshold: f64,
    /// Added to the threshold per unit of slope of the surface (the paper's elevation scalar).
    pub scalar: f64,
}

impl Default for SmrfParams {
    fn default() -> Self {
        Self {
            cell: 1.0,
            max_window: 18.0,
            slope: 0.15,
            threshold: 0.5,
            scalar: 1.25,
        }
    }
}

/// The minimum or maximum of the window of `2 r + 1` cells around each cell, along rows, by the
/// van Herk / Gil-Werman algorithm: O(cells) whatever `r`.
fn running(data: &[f32], cols: usize, rows: usize, r: usize, max: bool, out: &mut [f32]) {
    let w = 2 * r + 1;
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let mut g = vec![0f32; cols + w];
    let mut h = vec![0f32; cols + w];
    for row in 0..rows {
        let line = &data[row * cols..(row + 1) * cols];
        let n = cols + 2 * r;
        // the line padded by r cells of its edge value at each end
        let at = |i: usize| line[i.saturating_sub(r).min(cols - 1)];
        g.resize(n.div_ceil(w) * w, 0.0);
        h.resize(g.len(), 0.0);
        for i in 0..g.len() {
            let v = if i < n { at(i) } else { at(n - 1) };
            g[i] = if i % w == 0 { v } else { pick(g[i - 1], v) };
        }
        for i in (0..g.len()).rev() {
            let v = if i < n { at(i) } else { at(n - 1) };
            h[i] = if i % w == w - 1 || i + 1 == g.len() {
                v
            } else {
                pick(h[i + 1], v)
            };
        }
        for x in 0..cols {
            // window of padded cells x ..= x + 2 r
            out[row * cols + x] = pick(h[x], g[x + 2 * r]);
        }
    }
}

fn transpose(data: &[f32], cols: usize, rows: usize) -> Vec<f32> {
    let mut t = vec![0f32; data.len()];
    for r in 0..rows {
        for c in 0..cols {
            t[c * rows + r] = data[r * cols + c];
        }
    }
    t
}

/// A square window of radius `r` cells, the minimum (erosion) or the maximum (dilation).
fn window(data: &[f32], cols: usize, rows: usize, r: usize, max: bool) -> Vec<f32> {
    let mut a = vec![0f32; data.len()];
    running(data, cols, rows, r, max, &mut a);
    let t = transpose(&a, cols, rows);
    let mut b = vec![0f32; data.len()];
    running(&t, rows, cols, r, max, &mut b);
    transpose(&b, rows, cols)
}

/// Fills the empty (NaN) cells with the value of the nearest valued cell, by two sweeps over the
/// grid: O(cells).
fn fill_nearest(v: &mut [f32], cols: usize, rows: usize) {
    if v.iter().all(|x| x.is_nan()) {
        return;
    }
    let mut dist = vec![u32::MAX; v.len()];
    for (d, x) in dist.iter_mut().zip(v.iter()) {
        if !x.is_nan() {
            *d = 0;
        }
    }
    let relax = |from: usize, to: usize, v: &mut [f32], dist: &mut [u32]| {
        if dist[from] != u32::MAX && dist[from] + 1 < dist[to] {
            dist[to] = dist[from] + 1;
            v[to] = v[from];
        }
    };
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            if c > 0 {
                relax(i - 1, i, v, &mut dist);
            }
            if r > 0 {
                relax(i - cols, i, v, &mut dist);
            }
        }
    }
    for r in (0..rows).rev() {
        for c in (0..cols).rev() {
            let i = r * cols + c;
            if c + 1 < cols {
                relax(i + 1, i, v, &mut dist);
            }
            if r + 1 < rows {
                relax(i + cols, i, v, &mut dist);
            }
        }
    }
}

/// Cells where progressively larger openings of `z` drop by more than the slope allows.
fn objects(
    z: &[f32],
    cols: usize,
    rows: usize,
    cell: f64,
    slope: f64,
    max_window: f64,
) -> Vec<bool> {
    let max_radius = ((max_window / cell).ceil() as usize).max(1);
    let mut obj = vec![false; z.len()];
    let mut prev = z.to_vec();
    for radius in 1..=max_radius {
        if 2 * radius + 1 > cols.max(rows) * 2 {
            break;
        }
        let opened = window(
            &window(z, cols, rows, radius, false),
            cols,
            rows,
            radius,
            true,
        );
        let limit = (slope * cell * radius as f64) as f32;
        for k in 0..z.len() {
            if prev[k] - opened[k] > limit {
                obj[k] = true;
            }
        }
        prev = opened;
    }
    obj
}

/// The slope (rise over run) of a surface at each cell, by central differences.
fn slope_of(z: &[f32], cols: usize, rows: usize, cell: f64) -> Vec<f32> {
    let mut out = vec![0f32; z.len()];
    let at = |r: usize, c: usize| z[r * cols + c];
    for r in 0..rows {
        for c in 0..cols {
            let (c0, c1) = (c.saturating_sub(1), (c + 1).min(cols - 1));
            let (r0, r1) = (r.saturating_sub(1), (r + 1).min(rows - 1));
            let dx = (at(r, c1) - at(r, c0)) / ((c1 - c0).max(1) as f64 * cell) as f32;
            let dy = (at(r1, c) - at(r0, c)) / ((r1 - r0).max(1) as f64 * cell) as f32;
            out[r * cols + c] = dx.hypot(dy);
        }
    }
    out
}

/// Which points are ground. Noise and withheld points are never ground and take no part.
pub fn ground(pts: &[Point], p: &SmrfParams) -> Vec<bool> {
    let none = vec![false; pts.len()];
    if !(p.cell > 0.0) || !(p.max_window > 0.0) || pts.is_empty() {
        return none;
    }
    let b = pts.iter().filter(|q| !q.is_noise()).fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, q| [b[0].min(q.x), b[1].min(q.y), b[2].max(q.x), b[3].max(q.y)],
    );
    let Ok(base) = Grid::covering(b, p.cell) else {
        return none;
    };
    let (cols, rows) = (base.cols, base.rows);
    // the lowest point of each cell (rows run north to south in a Grid, as here)
    let mut zmin = vec![f32::NAN; cols * rows];
    let cell_of: Vec<Option<usize>> = pts
        .iter()
        .map(|q| {
            if q.is_noise() {
                None
            } else {
                base.index(q.x, q.y)
            }
        })
        .collect();
    for (q, c) in pts.iter().zip(&cell_of) {
        if let Some(c) = c {
            let z = q.z as f32;
            if zmin[*c].is_nan() || z < zmin[*c] {
                zmin[*c] = z;
            }
        }
    }
    let mut z = zmin.clone();
    fill_nearest(&mut z, cols, rows);
    // low outliers: a single cell far below its neighbours, found on the inverted surface
    let neg: Vec<f32> = z.iter().map(|v| -v).collect();
    let low = objects(&neg, cols, rows, p.cell, 5.0, p.cell);
    let obj = objects(&z, cols, rows, p.cell, p.slope, p.max_window);
    // the provisional surface: the minimum surface without objects and low outliers, filled
    let mut prov = zmin;
    for k in 0..prov.len() {
        if obj[k] || low[k] {
            prov[k] = f32::NAN;
        }
    }
    fill_nearest(&mut prov, cols, rows);
    let slope = slope_of(&prov, cols, rows, p.cell);
    pts.iter()
        .zip(&cell_of)
        .map(|(q, c)| {
            c.is_some_and(|c| {
                let s = prov[c];
                !s.is_nan()
                    && (q.z - f64::from(s)).abs() <= p.threshold + p.scalar * f64::from(slope[c])
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64) -> Point {
        Point {
            x,
            y,
            z,
            return_number: 1,
            number_of_returns: 1,
            ..Point::default()
        }
    }

    /// A 120 m square on a 10 % slope, a 20 x 20 m building 8 m high, a 3 m tree, a low spike,
    /// and no ground points under the building.
    fn scene() -> (Vec<Point>, Vec<&'static str>) {
        let mut pts = Vec::new();
        let mut what = Vec::new();
        let ground_z = |x: f64| 100.0 + 0.1 * x;
        for i in 0..240 {
            for j in 0..240 {
                let (x, y) = (f64::from(i) * 0.5 + 0.25, f64::from(j) * 0.5 + 0.25);
                let building = (40.0..60.0).contains(&x) && (40.0..60.0).contains(&y);
                if building {
                    pts.push(pt(x, y, ground_z(x) + 8.0));
                    what.push("building");
                } else if (90.0..93.0).contains(&x) && (90.0..93.0).contains(&y) {
                    pts.push(pt(x, y, ground_z(x) + 3.0));
                    what.push("tree");
                } else {
                    pts.push(pt(x, y, ground_z(x)));
                    what.push("ground");
                }
            }
        }
        pts.push(pt(20.2, 20.2, ground_z(20.2) - 15.0));
        what.push("spike");
        (pts, what)
    }

    #[test]
    fn separates_ground_from_objects_on_a_slope() {
        let (pts, what) = scene();
        let g = ground(&pts, &SmrfParams::default());
        let count = |kind: &str, want: bool| {
            what.iter()
                .zip(&g)
                .filter(|(w, b)| **w == kind && **b == want)
                .count()
        };
        let total = |kind: &str| what.iter().filter(|w| **w == kind).count();
        // nearly all the ground is found (the edge of the building may cost a few cells)
        assert!(
            count("ground", true) as f64 > 0.99 * total("ground") as f64,
            "{}",
            count("ground", true)
        );
        // the building and the tree are not ground
        assert_eq!(count("building", true), 0);
        assert_eq!(count("tree", true), 0);
        // nor is the low spike
        assert_eq!(count("spike", true), 0);
    }

    #[test]
    fn noise_and_empty_input_are_never_ground() {
        assert!(ground(&[], &SmrfParams::default()).is_empty());
        let mut noisy = pt(0.0, 0.0, 0.0);
        noisy.class = 7;
        let g = ground(&[noisy, pt(1.0, 1.0, 0.0)], &SmrfParams::default());
        assert_eq!(g, vec![false, true]);
    }

    #[test]
    fn running_extremes_match_the_obvious_loop() {
        let data: Vec<f32> = (0..7 * 5).map(|i| ((i * 37 + 11) % 23) as f32).collect();
        for r in 1..4 {
            let got = window(&data, 7, 5, r, false);
            for row in 0..5usize {
                for col in 0..7usize {
                    let mut m = f32::INFINITY;
                    for rr in row.saturating_sub(r)..=(row + r).min(4) {
                        for cc in col.saturating_sub(r)..=(col + r).min(6) {
                            m = m.min(data[rr * 7 + cc]);
                        }
                    }
                    assert_eq!(got[row * 7 + col], m, "r={r} ({row},{col})");
                }
            }
        }
    }
}
