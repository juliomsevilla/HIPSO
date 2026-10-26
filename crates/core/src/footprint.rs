//! The ground a point cloud actually covers. Surfaces are gridded on the cloud's bounding box,
//! and filling their empty cells would invent values wherever the cloud has no points (the
//! corners of a rotated block, a gap between flight lines). The footprint records where points
//! fall, on a coarse grid and closed over small gaps, so a surface can be cut to it.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::grid::Grid;

/// Cells of the footprint grid, in grid cells of the surface; at least `MIN_SIZE` map units.
const SIZE_IN_CELLS: f64 = 6.0;
const MIN_SIZE: f64 = 3.0;
/// Coarse cells a gap may span and still count as covered (closing by dilation and erosion).
const CLOSE: usize = 2;
/// Most coarse cells a footprint keeps.
const MAX_CELLS: usize = 1 << 24;

/// Which coarse cells of an area hold points. Shared across threads: `add` takes `&self`.
pub struct Footprint {
    x0: f64,
    y0: f64,
    size: f64,
    cols: usize,
    rows: usize,
    seen: Vec<AtomicBool>,
}

impl Footprint {
    /// A footprint over `[min x, min y, max x, max y]` for surfaces of `cell`.
    pub fn new(b: [f64; 4], cell: f64) -> Self {
        let mut size = (cell * SIZE_IN_CELLS).max(MIN_SIZE);
        let dims = |s: f64| {
            (
                ((b[2] - b[0]) / s).floor() as usize + 1,
                ((b[3] - b[1]) / s).floor() as usize + 1,
            )
        };
        let (mut cols, mut rows) = dims(size);
        while cols.saturating_mul(rows) > MAX_CELLS {
            size *= 2.0;
            (cols, rows) = dims(size);
        }
        Self {
            x0: b[0],
            y0: b[1],
            size,
            cols,
            rows,
            seen: (0..cols * rows).map(|_| AtomicBool::new(false)).collect(),
        }
    }

    /// Marks the coarse cell holding `(x, y)`.
    #[inline]
    pub fn add(&self, x: f64, y: f64) {
        let c = ((x - self.x0) / self.size).floor();
        let r = ((y - self.y0) / self.size).floor();
        if c >= 0.0
            && r >= 0.0
            && c < self.cols as f64
            && r < self.rows as f64
            && let Some(s) = self.seen.get(r as usize * self.cols + c as usize)
        {
            s.store(true, Ordering::Relaxed);
        }
    }

    /// The covered cells after bridging gaps narrower than `2 * CLOSE` coarse cells; cells
    /// beyond the edge of the area count as covered while eroding, so the edge is not eaten.
    fn closed(&self) -> Vec<bool> {
        let mut m: Vec<bool> = self
            .seen
            .iter()
            .map(|s| s.load(Ordering::Relaxed))
            .collect();
        for _ in 0..CLOSE {
            m = self.morph(&m, true);
        }
        for _ in 0..CLOSE {
            m = self.morph(&m, false);
        }
        m
    }

    /// One step of dilation (`grow`) or erosion over the 8 neighbours.
    fn morph(&self, m: &[bool], grow: bool) -> Vec<bool> {
        let (cols, rows) = (self.cols as isize, self.rows as isize);
        (0..m.len())
            .map(|i| {
                let (c, r) = ((i % self.cols) as isize, (i / self.cols) as isize);
                let at = |dc: isize, dr: isize| {
                    let (nc, nr) = (c + dc, r + dr);
                    if nc < 0 || nr < 0 || nc >= cols || nr >= rows {
                        !grow
                    } else {
                        m.get((nr * cols + nc) as usize).copied().unwrap_or(false)
                    }
                };
                let around = (-1..=1).flat_map(|dr| (-1..=1).map(move |dc| (dc, dr)));
                if grow {
                    around.into_iter().any(|(dc, dr)| at(dc, dr))
                } else {
                    around.into_iter().all(|(dc, dr)| at(dc, dr))
                }
            })
            .collect()
    }

    /// Empties the cells of `g` that lie outside the footprint.
    pub fn apply(&self, g: &mut Grid) {
        let covered = self.closed();
        for row in 0..g.rows {
            for col in 0..g.cols {
                let (x, y) = g.center(col, row);
                let c = ((x - self.x0) / self.size).floor();
                let r = ((y - self.y0) / self.size).floor();
                let inside = c >= 0.0
                    && r >= 0.0
                    && c < self.cols as f64
                    && r < self.rows as f64
                    && covered
                        .get(r as usize * self.cols + c as usize)
                        .copied()
                        .unwrap_or(false);
                if !inside && let Some(v) = g.data.get_mut(row * g.cols + col) {
                    *v = f32::NAN;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;

    #[test]
    fn a_surface_is_cut_to_the_points_not_to_their_box() -> Result<()> {
        // an L: points along the bottom and the left edge, the top right corner is empty
        let fp = Footprint::new([0.0, 0.0, 100.0, 100.0], 1.0);
        for k in 0..100 {
            let t = f64::from(k) + 0.5;
            for w in 0..20 {
                fp.add(t, f64::from(w));
                fp.add(f64::from(w), t);
            }
        }
        let mut g = Grid::covering([0.0, 0.0, 99.0, 99.0], 1.0)?;
        g.data.fill(1.0);
        fp.apply(&mut g);
        assert!(g.sample(5.0, 90.0).is_some());
        assert!(g.sample(90.0, 5.0).is_some());
        assert!(g.sample(90.0, 90.0).is_none());
        Ok(())
    }

    #[test]
    fn a_small_gap_inside_the_cloud_stays_covered() -> Result<()> {
        let fp = Footprint::new([0.0, 0.0, 60.0, 60.0], 1.0);
        for x in 0..60 {
            for y in 0..60 {
                if !(28..32).contains(&x) || !(28..32).contains(&y) {
                    fp.add(f64::from(x) + 0.5, f64::from(y) + 0.5);
                }
            }
        }
        let mut g = Grid::covering([0.0, 0.0, 59.0, 59.0], 1.0)?;
        g.data.fill(1.0);
        fp.apply(&mut g);
        assert!(g.sample(30.0, 30.0).is_some());
        Ok(())
    }
}
