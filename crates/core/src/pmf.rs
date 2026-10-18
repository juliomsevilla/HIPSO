//! Progressive Morphological Filter for ground points (Zhang et al. 2003, "A progressive
//! morphological filter for removing nonground measurements from airborne LIDAR data").
//!
//! On a grid of the lowest point per cell, openings with windows that grow step by step remove
//! objects smaller than the window; a point more than the step's height threshold above the
//! opened surface is non-ground. Thresholds grow with the window by the terrain slope. Each
//! opening costs O(cells), whatever the window. Works in memory: run it on tiles.

use crate::grid::Grid;
use crate::las::Point;

#[derive(Clone, Debug, PartialEq)]
pub struct PmfParams {
    pub cell: f64,
    /// Largest window, in map units.
    pub max_window: f64,
    /// Terrain slope (rise over run) used to raise the threshold with the window.
    pub slope: f64,
    /// Threshold of the first window, and the largest threshold.
    pub dh0: f64,
    pub dh_max: f64,
}

impl Default for PmfParams {
    fn default() -> Self {
        Self {
            cell: 1.0,
            max_window: 20.0,
            slope: 1.0,
            dh0: 0.5,
            dh_max: 3.0,
        }
    }
}

/// Zhang et al.'s linear window sequence (in cells: 3, 5, 7, … up to the largest) and the
/// threshold of each.
pub fn schedule(p: &PmfParams) -> Vec<(usize, f64)> {
    let max = ((p.max_window / p.cell).round() as usize).max(3);
    let mut out = Vec::new();
    let mut prev = 1usize;
    let mut k = 1usize;
    loop {
        let w = 2 * k + 1;
        if w > max {
            break;
        }
        let dh = if w <= 3 {
            p.dh0
        } else {
            (p.slope * (w - prev) as f64 * p.cell + p.dh0).min(p.dh_max)
        };
        out.push((w, dh));
        prev = w;
        k += 1;
    }
    out
}

/// Which points are ground. Noise and withheld points are never ground and take no part.
pub fn ground(pts: &[Point], p: &PmfParams) -> Vec<bool> {
    let mut ground: Vec<bool> = pts.iter().map(|q| !q.is_noise()).collect();
    if !(p.cell > 0.0) || pts.is_empty() {
        return vec![false; pts.len()];
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
        return vec![false; pts.len()];
    };
    for (w, dh) in schedule(p) {
        // The lowest remaining ground candidate per cell.
        let mut low = base.like();
        for (q, g) in pts.iter().zip(&ground) {
            if *g
                && let Some(v) = low.index(q.x, q.y).and_then(|i| low.data.get_mut(i))
                && (v.is_nan() || (q.z as f32) < *v)
            {
                *v = q.z as f32;
            }
        }
        low.fill_holes();
        let opened = crate::buildings::opening(&low, w);
        for (q, g) in pts.iter().zip(ground.iter_mut()) {
            if *g
                && let Some(i) = low.index(q.x, q.y)
                && q.z as f32 - opened[i] > dh as f32
            {
                *g = false;
            }
        }
    }
    ground
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_grows_windows_and_thresholds() {
        let s = schedule(&PmfParams::default());
        assert_eq!(s.first(), Some(&(3, 0.5)));
        assert_eq!(s.get(1), Some(&(5, 2.5)));
        assert!(s.iter().all(|(w, dh)| *w <= 20 && *dh <= 3.0));
    }

    #[test]
    fn a_house_is_removed() {
        let mut pts = Vec::new();
        for i in 0..50 {
            for j in 0..50 {
                let roof = (20..30).contains(&i) && (20..30).contains(&j);
                pts.push(Point {
                    x: f64::from(i) + 0.5,
                    y: f64::from(j) + 0.5,
                    z: if roof { 106.0 } else { 100.0 },
                    ..Default::default()
                });
            }
        }
        let g = ground(&pts, &PmfParams::default());
        assert_eq!(g.iter().filter(|g| !**g).count(), 100);
    }
}
