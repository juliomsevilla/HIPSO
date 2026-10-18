//! Building and vegetation classification, for cleaner ground models in built-up areas.
//!
//! A terrain reference is built from the lowest point of each cell by a morphological opening
//! wider than any building, which removes roofs whatever the ground filter decided about them.
//! Cells standing more than `offset` above it form candidate regions; a region is a building
//! when it is large enough, smooth (roofs are planar, crowns are rugged) and bounded by sharp
//! steps (roof edges, unlike the gentle sides of a hill). Points of building cells above the
//! offset become buildings (class 6), even those the ground filter had kept as ground; other
//! non-ground points above the offset become high vegetation (class 5). Works in memory: run
//! it on tiles with a buffer of at least half the window.

use std::collections::VecDeque;

use crate::grid::Grid;
use crate::las::Point;

#[derive(Clone, Debug, PartialEq)]
pub struct BuildingParams {
    /// Cell size of the analysis grids.
    pub cell: f64,
    /// Height above the terrain from which points can be buildings or trees.
    pub offset: f64,
    /// Widest building expected: the window of the terrain opening.
    pub window: f64,
    /// Smallest building footprint, in square units.
    pub min_area: f64,
    /// Roughness (RMS of the lowest surface around a cell from its best plane) up to which a
    /// cell counts as planar.
    pub planar: f64,
}

impl Default for BuildingParams {
    fn default() -> Self {
        Self {
            cell: 1.0,
            offset: 2.0,
            window: 60.0,
            min_area: 40.0,
            planar: 0.2,
        }
    }
}

pub const BUILDING: u8 = 6;
pub const HIGH_VEGETATION: u8 = 5;

/// The new class of each point, if any: [`BUILDING`] or [`HIGH_VEGETATION`]. `ground` says
/// which points the ground filter kept.
pub fn classify(pts: &[Point], ground: &[bool], p: &BuildingParams) -> Vec<Option<u8>> {
    let mut out = vec![None; pts.len()];
    let valid: Vec<&Point> = pts.iter().filter(|q| !q.is_noise()).collect();
    if valid.is_empty() || !(p.cell > 0.0) {
        return out;
    }
    let b = valid.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, q| [b[0].min(q.x), b[1].min(q.y), b[2].max(q.x), b[3].max(q.y)],
    );
    let Ok(mut low) = Grid::covering(b, p.cell) else {
        return out;
    };
    for q in &valid {
        if let Some(v) = low.index(q.x, q.y).and_then(|i| low.data.get_mut(i))
            && (v.is_nan() || (q.z as f32) < *v)
        {
            *v = q.z as f32;
        }
    }
    low.fill_holes();
    let k = ((p.window / p.cell).round() as usize) | 1;
    let terrain = opening(&low, k);

    // Candidate cells: the lowest surface well above the terrain.
    let (cols, rows) = (low.cols, low.rows);
    let n = cols * rows;
    let raised: Vec<bool> = (0..n)
        .map(|i| low.data[i] - terrain[i] > p.offset as f32)
        .collect();
    let rough = roughness(&low);
    // Roof candidates: raised and planar. Trees touching a roof stay out of its region.
    let cand: Vec<bool> = (0..n)
        .map(|i| raised[i] && rough[i] <= p.planar as f32)
        .collect();
    let mut building = vec![false; n];
    let mut seen = vec![false; n];
    let min_cells = (p.min_area / (p.cell * p.cell)).ceil() as usize;
    for start in 0..n {
        if !cand[start] || seen[start] {
            continue;
        }
        // One 8-connected region of candidate cells.
        let mut region = Vec::new();
        let mut queue = VecDeque::from([start]);
        seen[start] = true;
        while let Some(i) = queue.pop_front() {
            region.push(i);
            for j in neighbours8(i, cols, rows) {
                if cand[j] && !seen[j] {
                    seen[j] = true;
                    queue.push_back(j);
                }
            }
        }
        if region.len() < min_cells {
            continue;
        }
        // Steps down to the terrain around it: sharp at roof edges, gentle on hillsides.
        let (mut edges, mut sharp) = (0usize, 0usize);
        for &i in &region {
            for j in neighbours4(i, cols, rows) {
                if !raised[j] {
                    edges += 1;
                    if low.data[i] - low.data[j] > 0.5 * p.offset as f32 {
                        sharp += 1;
                    }
                }
            }
        }
        if edges == 0 || sharp * 2 >= edges {
            for &i in &region {
                building[i] = true;
            }
        }
    }
    // Roof edges are rough (their window crosses the edge): grow buildings into the raised
    // cells around them, two cells deep.
    for _ in 0..2 {
        let grown: Vec<usize> = (0..n)
            .filter(|&i| {
                !building[i] && raised[i] && neighbours8(i, cols, rows).any(|j| building[j])
            })
            .collect();
        for i in grown {
            building[i] = true;
        }
    }

    for (k, q) in pts.iter().enumerate() {
        if q.is_noise() {
            continue;
        }
        let Some(i) = low.index(q.x, q.y) else {
            continue;
        };
        let h = q.z as f32 - terrain[i];
        if h <= p.offset as f32 {
            continue;
        }
        if building[i] {
            out[k] = Some(BUILDING);
        } else if !ground.get(k).copied().unwrap_or(false) {
            out[k] = Some(HIGH_VEGETATION);
        }
    }
    out
}

fn neighbours8(i: usize, cols: usize, rows: usize) -> impl Iterator<Item = usize> {
    let (c, r) = ((i % cols) as isize, (i / cols) as isize);
    (-1isize..=1)
        .flat_map(move |dr| (-1isize..=1).map(move |dc| (c + dc, r + dr)))
        .filter(move |&(nc, nr)| {
            (nc, nr) != (c, r) && nc >= 0 && nr >= 0 && (nc as usize) < cols && (nr as usize) < rows
        })
        .map(move |(nc, nr)| nr as usize * cols + nc as usize)
}

fn neighbours4(i: usize, cols: usize, rows: usize) -> impl Iterator<Item = usize> {
    let (c, r) = ((i % cols) as isize, (i / cols) as isize);
    [(c - 1, r), (c + 1, r), (c, r - 1), (c, r + 1)]
        .into_iter()
        .filter(move |&(nc, nr)| nc >= 0 && nr >= 0 && (nc as usize) < cols && (nr as usize) < rows)
        .map(move |(nc, nr)| nr as usize * cols + nc as usize)
}

/// Grey opening (erosion then dilation) with a square window of `k` cells.
pub(crate) fn opening(g: &Grid, k: usize) -> Vec<f32> {
    let e = filter2(&g.data, g.cols, g.rows, k, f32::min);
    filter2(&e, g.cols, g.rows, k, f32::max)
}

/// A separable running min or max over a `k` x `k` window (windows clipped at the edges).
fn filter2(v: &[f32], cols: usize, rows: usize, k: usize, f: fn(f32, f32) -> f32) -> Vec<f32> {
    let h = run(v, cols, rows, k, f, true);
    run(&h, cols, rows, k, f, false)
}

/// One direction of the running filter, with a monotonic queue: O(1) per cell.
fn run(
    v: &[f32],
    cols: usize,
    rows: usize,
    k: usize,
    f: fn(f32, f32) -> f32,
    along_rows: bool,
) -> Vec<f32> {
    let mut out = v.to_vec();
    let r = k / 2;
    let (lines, len) = if along_rows {
        (rows, cols)
    } else {
        (cols, rows)
    };
    let idx = |line: usize, pos: usize| {
        if along_rows {
            line * cols + pos
        } else {
            pos * cols + line
        }
    };
    // f(a, b) == a means a is at least as good as b.
    let better = |a: f32, b: f32| f(a, b) == a;
    let mut q: VecDeque<usize> = VecDeque::new();
    for line in 0..lines {
        q.clear();
        let mut next = 0usize;
        for pos in 0..len {
            let hi = (pos + r).min(len - 1);
            while next <= hi {
                let x = v[idx(line, next)];
                while q.back().is_some_and(|&b| better(x, v[idx(line, b)])) {
                    q.pop_back();
                }
                q.push_back(next);
                next += 1;
            }
            while q.front().is_some_and(|&a| a + r < pos) {
                q.pop_front();
            }
            if let Some(&a) = q.front() {
                out[idx(line, pos)] = v[idx(line, a)];
            }
        }
    }
    out
}

/// RMS distance of each 3 x 3 window of the surface from its least-squares plane.
fn roughness(g: &Grid) -> Vec<f32> {
    let (cols, rows) = (g.cols, g.rows);
    let mut out = vec![f32::INFINITY; cols * rows];
    for r in 1..rows.saturating_sub(1) {
        for c in 1..cols.saturating_sub(1) {
            let mut z = [0f64; 9];
            for (k, slot) in z.iter_mut().enumerate() {
                *slot = f64::from(g.data[(r + k / 3 - 1) * cols + c + k % 3 - 1]);
            }
            // Plane z = a + bx + cy over x, y in {-1, 0, 1}: closed form.
            let mean = z.iter().sum::<f64>() / 9.0;
            let bx = (z[2] + z[5] + z[8] - z[0] - z[3] - z[6]) / 6.0;
            let by = (z[6] + z[7] + z[8] - z[0] - z[1] - z[2]) / 6.0;
            let ss: f64 = z
                .iter()
                .enumerate()
                .map(|(k, v)| {
                    let (x, y) = ((k % 3) as f64 - 1.0, (k / 3) as f64 - 1.0);
                    (v - (mean + bx * x + by * y)).powi(2)
                })
                .sum();
            out[r * cols + c] = (ss / 9.0).sqrt() as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_min_and_max() {
        let v = [5.0, 1.0, 4.0, 3.0, 9.0];
        assert_eq!(
            run(&v, 5, 1, 3, f32::min, true),
            vec![1.0, 1.0, 1.0, 3.0, 3.0]
        );
        assert_eq!(
            run(&v, 5, 1, 3, f32::max, true),
            vec![5.0, 5.0, 4.0, 9.0, 9.0]
        );
    }

    #[test]
    fn a_box_is_a_building_and_a_hill_is_not() {
        let mut pts = Vec::new();
        for i in 0..120 {
            for j in 0..120 {
                let (x, y) = (f64::from(i), f64::from(j));
                // A smooth hill rising 6 m to the east, and a 20 x 20 m flat roof 8 m up.
                let ground = 100.0 + 6.0 * (x / 120.0);
                let roof = (20.0..40.0).contains(&x) && (50.0..70.0).contains(&y);
                pts.push(Point {
                    x: x + 0.5,
                    y: y + 0.5,
                    z: if roof { ground + 8.0 } else { ground },
                    ..Default::default()
                });
            }
        }
        let ground: Vec<bool> = pts.iter().map(|_| true).collect();
        let cls = classify(&pts, &ground, &BuildingParams::default());
        let roof = pts
            .iter()
            .zip(&cls)
            .filter(|(q, _)| (20.0..40.0).contains(&q.x) && (50.0..70.0).contains(&q.y));
        assert!(roof.clone().all(|(_, c)| *c == Some(BUILDING)));
        assert_eq!(cls.iter().filter(|c| c.is_some()).count(), 400);
    }
}
