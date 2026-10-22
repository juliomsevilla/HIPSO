//! Viewshed: which cells an observer sees. Rays leave the observer at an angular step finer
//! than the cell at the far end, and along each one a cell is seen when the line from the eye
//! to its top (ground plus the height to be seen) rises at least as steeply as everything
//! before it. The work grows with the radius and the number of rays, not with the cells in the
//! disc, and the rays run in parallel. The Earth's curvature and refraction are not applied
//! (negligible within a few kilometres).

use std::sync::atomic::{AtomicU8, Ordering};

use rayon::prelude::*;

use crate::grid::Grid;
use crate::{Error, Result};

/// 1 where the observer sees, 0 where not, empty beyond `max_distance` or without data.
pub fn viewshed(
    g: &Grid,
    z: f64,
    observer: [f64; 2],
    eye: f64,
    target: f64,
    max_distance: f64,
) -> Result<Grid> {
    if !(max_distance >= g.cell) || !(eye >= 0.0) || !(target >= 0.0) {
        return Err(Error::Param(
            "la cuenca visual necesita un alcance de al menos una celda y alturas no negativas"
                .into(),
        ));
    }
    let (oc, or) = g
        .col_row(observer[0], observer[1])
        .ok_or_else(|| Error::Param("el observador está fuera del ráster".into()))?;
    let ground = g.data[or * g.cols + oc];
    if ground.is_nan() {
        return Err(Error::Param(
            "el observador está sobre una celda sin datos".into(),
        ));
    }
    let zo = f64::from(ground) * z + eye;
    let reach = max_distance / g.cell;
    // sub-cell steps and rays, so no cell between two rays goes unvisited
    let rays = ((std::f64::consts::TAU * reach * 2.0).ceil() as usize).max(360);
    let steps = (reach * 2.0).ceil() as usize;
    // per cell: bit 0 reached by a ray, bit 1 seen
    let state: Vec<AtomicU8> = (0..g.data.len()).map(|_| AtomicU8::new(0)).collect();
    // the observer's cell, in cell units from the grid's north-west corner
    let (ox, oy) = (
        (observer[0] - g.x0) / g.cell,
        (g.y0 + g.rows as f64 * g.cell - observer[1]) / g.cell,
    );
    (0..rays).into_par_iter().for_each(|k| {
        let a = std::f64::consts::TAU * k as f64 / rays as f64;
        let (dx, dy) = (a.sin() * 0.5, -a.cos() * 0.5);
        let mut top = f64::NEG_INFINITY;
        let mut last = usize::MAX;
        for s in 1..=steps {
            let (x, y) = (ox + dx * s as f64, oy + dy * s as f64);
            if x < 0.0 || y < 0.0 || x >= g.cols as f64 || y >= g.rows as f64 {
                break;
            }
            let i = y as usize * g.cols + x as usize;
            if i == last {
                continue;
            }
            last = i;
            // distance to the cell's centre, so the cell is judged the same from every ray
            let (cx, cy) = (
                (i % g.cols) as f64 + 0.5 - ox,
                (i / g.cols) as f64 + 0.5 - oy,
            );
            let d = cx.hypot(cy) * g.cell;
            if d > max_distance {
                break;
            }
            state[i].fetch_or(1, Ordering::Relaxed);
            let h = g.data[i];
            if h.is_nan() {
                continue;
            }
            let h = f64::from(h) * z;
            if (h + target - zo) / d >= top {
                state[i].fetch_or(2, Ordering::Relaxed);
            }
            top = top.max((h - zo) / d);
        }
    });
    let mut out = g.like();
    for (i, o) in out.data.iter_mut().enumerate() {
        let st = state[i].load(Ordering::Relaxed);
        if st & 1 == 1 && !g.data[i].is_nan() {
            *o = f32::from(st >> 1);
        }
    }
    out.data[or * g.cols + oc] = 1.0;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(cols: usize, rows: usize) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 1.0, cols, rows)?;
        g.data.fill(10.0);
        Ok(g)
    }

    #[test]
    fn open_ground_is_seen_up_to_the_range() -> Result<()> {
        let g = flat(41, 41)?;
        let v = viewshed(&g, 1.0, [20.5, 20.5], 1.7, 0.0, 15.0)?;
        assert_eq!(v.data[20 * 41 + 20], 1.0);
        assert_eq!(v.data[20 * 41 + 30], 1.0);
        assert_eq!(v.data[8 * 41 + 20], 1.0);
        // beyond 15 m: empty
        assert!(v.data[20 * 41 + 38].is_nan());
        assert!(v.data[0].is_nan());
        let seen = v.data.iter().filter(|x| **x == 1.0).count() as f64;
        let disc = std::f64::consts::PI * 15.0 * 15.0;
        assert!((seen / disc - 1.0).abs() < 0.05, "{seen} vs {disc}");
        Ok(())
    }

    #[test]
    fn a_wall_hides_what_is_behind_it() -> Result<()> {
        let mut g = flat(41, 41)?;
        for r in 0..41 {
            g.data[r * 41 + 25] = 30.0;
        }
        let v = viewshed(&g, 1.0, [20.5, 20.5], 1.7, 0.0, 18.0)?;
        assert_eq!(v.data[20 * 41 + 24], 1.0, "in front of the wall");
        assert_eq!(v.data[20 * 41 + 25], 1.0, "the wall itself");
        assert_eq!(v.data[20 * 41 + 30], 0.0, "behind it");
        // a tall enough target is seen over a low wall
        let mut low = flat(41, 41)?;
        for r in 0..41 {
            low.data[r * 41 + 25] = 12.0;
        }
        let hidden = viewshed(&low, 1.0, [20.5, 20.5], 1.7, 0.0, 18.0)?;
        let tall = viewshed(&low, 1.0, [20.5, 20.5], 1.7, 20.0, 18.0)?;
        assert_eq!(hidden.data[20 * 41 + 28], 0.0);
        assert_eq!(tall.data[20 * 41 + 28], 1.0);
        Ok(())
    }

    #[test]
    fn the_observer_must_be_on_data() -> Result<()> {
        let g = flat(5, 5)?;
        assert!(viewshed(&g, 1.0, [50.0, 50.0], 1.7, 0.0, 3.0).is_err());
        Ok(())
    }
}
