//! Rasterising a TIN (Delaunay triangulation) of ground points onto a grid, tile by tile,
//! with memory bounded by the tile, not the cloud.

use crate::grid::Grid;

/// Longest triangle edge interpolated across; larger gaps are left to hole filling.
pub const MAX_EDGE: f64 = 50.0;

/// Which point of a cell enters the triangulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Thin {
    /// The nearest to the cell centre (terrain).
    Nearest,
    /// The highest (canopy surfaces).
    Highest,
}

/// Writes into the empty cells of `grid` the TIN surface through `pts` (x, y, z), thinned to
/// the point nearest each cell centre so memory and time follow the grid, not the density.
pub fn rasterize(pts: &[[f64; 3]], grid: &mut Grid) {
    rasterize_with(pts, grid, MAX_EDGE, Thin::Nearest);
}

/// [`rasterize`] with the longest edge interpolated across and the thinning chosen.
pub fn rasterize_with(pts: &[[f64; 3]], grid: &mut Grid, max_edge: f64, thin: Thin) {
    // Thin: one point per cell, on a dense lattice over the points (the tile and its buffer):
    // 8 bytes a cell, the index of the chosen point and its score. Points outside the grid help
    // near its edges, so they are kept too.
    let Some(b) = pts.iter().try_fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, q| {
            q[0].is_finite().then(|| {
                [
                    b[0].min(q[0]),
                    b[1].min(q[1]),
                    b[2].max(q[0]),
                    b[3].max(q[1]),
                ]
            })
        },
    ) else {
        return;
    };
    let (c0, r0) = (
        ((b[0] - grid.x0) / grid.cell).floor(),
        ((b[1] - grid.y0) / grid.cell).floor(),
    );
    let lcols = (((b[2] - grid.x0) / grid.cell).floor() - c0) as usize + 1;
    let lrows = (((b[3] - grid.y0) / grid.cell).floor() - r0) as usize + 1;
    let Some(cells) = lcols.checked_mul(lrows).filter(|n| *n <= 1 << 28) else {
        return;
    };
    let mut best: Vec<(u32, f32)> = vec![(u32::MAX, f32::INFINITY); cells];
    for (k, q) in pts.iter().enumerate() {
        let (fc, fr) = ((q[0] - grid.x0) / grid.cell, (q[1] - grid.y0) / grid.cell);
        let (c, r) = (fc.floor(), fr.floor());
        let d = match thin {
            Thin::Nearest => ((fc - c - 0.5).powi(2) + (fr - r - 0.5).powi(2)) as f32,
            Thin::Highest => -q[2] as f32,
        };
        let i = (r - r0) as usize * lcols + (c - c0) as usize;
        if let Some(e) = best.get_mut(i)
            && d < e.1
        {
            *e = (k as u32, d);
        }
    }
    let kept: Vec<[f64; 3]> = best
        .iter()
        .filter(|e| e.0 != u32::MAX)
        .filter_map(|e| pts.get(e.0 as usize).copied())
        .collect();
    drop(best);
    if kept.len() < 3 {
        return;
    }
    // Coordinates relative to the grid keep the triangulation precise.
    let xy: Vec<delaunator::Point> = kept
        .iter()
        .map(|q| delaunator::Point {
            x: q[0] - grid.x0,
            y: q[1] - grid.y0,
        })
        .collect();
    let t = delaunator::triangulate(&xy);
    let top = grid.rows as f64 * grid.cell;
    for tri in t.triangles.as_chunks::<3>().0 {
        let (Some(a), Some(b), Some(c)) = (xy.get(tri[0]), xy.get(tri[1]), xy.get(tri[2])) else {
            continue;
        };
        let (Some(za), Some(zb), Some(zc)) = (kept.get(tri[0]), kept.get(tri[1]), kept.get(tri[2]))
        else {
            continue;
        };
        let edge = |p: &delaunator::Point, q: &delaunator::Point| {
            ((p.x - q.x).powi(2) + (p.y - q.y).powi(2)).sqrt()
        };
        if edge(a, b).max(edge(b, c)).max(edge(c, a)) > max_edge {
            continue;
        }
        let det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
        if det.abs() < 1e-12 {
            continue;
        }
        // Cells whose centres fall in the triangle's box.
        let cell = grid.cell;
        let c0 = (((a.x.min(b.x).min(c.x)) / cell - 0.5).ceil().max(0.0)) as usize;
        let c1 = (((a.x.max(b.x).max(c.x)) / cell - 0.5)
            .floor()
            .min(grid.cols as f64 - 1.0)) as isize;
        // Rows count from the north; y is from the grid's south edge.
        let r0 = (((top - a.y.max(b.y).max(c.y)) / cell - 0.5).ceil().max(0.0)) as usize;
        let r1 = (((top - a.y.min(b.y).min(c.y)) / cell - 0.5)
            .floor()
            .min(grid.rows as f64 - 1.0)) as isize;
        if c1 < 0 || r1 < 0 {
            continue;
        }
        for r in r0..=r1 as usize {
            let y = top - (r as f64 + 0.5) * cell;
            for cc in c0..=c1 as usize {
                let x = (cc as f64 + 0.5) * cell;
                let l1 = ((b.y - c.y) * (x - c.x) + (c.x - b.x) * (y - c.y)) / det;
                let l2 = ((c.y - a.y) * (x - c.x) + (a.x - c.x) * (y - c.y)) / det;
                let l3 = 1.0 - l1 - l2;
                if l1 < -1e-9 || l2 < -1e-9 || l3 < -1e-9 {
                    continue;
                }
                if let Some(v) = grid.data.get_mut(r * grid.cols + cc)
                    && v.is_nan()
                {
                    *v = (l1 * za[2] + l2 * zb[2] + l3 * zc[2]) as f32;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plane_is_reproduced() -> crate::Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 10, 10)?;
        let pts: Vec<[f64; 3]> = (0..121)
            .map(|i| {
                let (x, y) = (f64::from(i % 11), f64::from(i / 11));
                [x, y, 5.0 + 0.5 * x + 0.25 * y]
            })
            .collect();
        rasterize(&pts, &mut g);
        // Cell (col 3, row 0) is centred at (3.5, 9.5).
        assert!((g.data[3] - (5.0 + 1.75 + 2.375)).abs() < 1e-4);
        assert!(g.data.iter().all(|v| !v.is_nan()));
        Ok(())
    }
}
