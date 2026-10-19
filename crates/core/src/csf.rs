//! Cloth Simulation Filter for ground points (Zhang et al. 2016, "An Easy-to-Use Airborne
//! LiDAR Data Filtering Method Based on Cloth Simulation"; constants as in the authors' CSF
//! library, Apache-2.0).
//!
//! The cloud is turned upside down and a cloth of particles falls onto it under gravity, held
//! together by springs whose stiffness is the rigidness; particles stop where they meet the
//! (inverted) points. Points within `threshold` of the settled cloth are ground. Works in
//! memory: run it on tiles of bounded size.

use crate::las::Point;
use crate::{Error, Result};

/// Most particles the cloth may have (8 bytes of height and 8 more of position each).
const MAX_PARTICLES: f64 = (1u64 << 27) as f64;

#[derive(Clone, Debug, PartialEq)]
pub struct CsfParams {
    /// Distance between cloth particles.
    pub resolution: f64,
    /// 1 (soft, steep terrain) to 3 (stiff, flat terrain).
    pub rigidness: u8,
    /// Height above the cloth up to which points are ground.
    pub threshold: f64,
    pub time_step: f64,
    pub iterations: u32,
}

impl Default for CsfParams {
    fn default() -> Self {
        Self {
            resolution: 0.5,
            rigidness: 1,
            threshold: 0.5,
            time_step: 0.65,
            iterations: 500,
        }
    }
}

/// How far a constraint moves one free particle, and each of two, by rigidness.
const SINGLE_MOVE: [f64; 4] = [0.0, 0.3, 0.51, 0.657];
const DOUBLE_MOVE: [f64; 4] = [0.0, 0.3, 0.42, 0.468];
const GRAVITY: f64 = 0.2;
const DAMPING: f64 = 0.01;
/// The cloth has settled when no particle moves more than this in a step.
const SETTLED: f64 = 0.005;

/// Which points are ground. Noise and withheld points are never ground and take no part. A
/// cloth too fine for the extent of the cloud is an error, not an empty result.
pub fn ground(pts: &[Point], p: &CsfParams) -> Result<Vec<bool>> {
    let mut out = vec![false; pts.len()];
    let valid: Vec<usize> = (0..pts.len()).filter(|&i| !pts[i].is_noise()).collect();
    if valid.is_empty() {
        return Ok(out);
    }
    if !(p.resolution > 0.0) {
        return Err(Error::Param("the cloth resolution must be positive".into()));
    }
    let (mut x0, mut y0, mut x1, mut y1, mut top) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &i in &valid {
        let q = &pts[i];
        (x0, y0, x1, y1) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y));
        top = top.max(-q.z);
    }
    // A cloth two particles wider than the cloud on every side.
    let r = p.resolution;
    let (ox, oy) = (x0 - 2.0 * r, y0 - 2.0 * r);
    // counted in floating point first: a huge extent over a tiny resolution would overflow
    // the integer product and give a small, wrong cloth
    let (cols_f, rows_f) = (((x1 - ox) / r).ceil() + 3.0, ((y1 - oy) / r).ceil() + 3.0);
    if !(cols_f * rows_f <= MAX_PARTICLES) {
        return Err(Error::Param(format!(
            "the cloth resolution of {r} m is too fine for this area ({cols_f} x {rows_f} particles): use a larger resolution or smaller tiles"
        )));
    }
    let (cols, rows) = (cols_f as usize, rows_f as usize);
    let n = cols * rows;
    // Each particle's collision height: the highest inverted point nearest it.
    let mut ihv = vec![f64::NEG_INFINITY; n];
    let node = |x: f64, y: f64| -> usize {
        let c = (((x - ox) / r).round().max(0.0) as usize).min(cols - 1);
        let rr = (((y - oy) / r).round().max(0.0) as usize).min(rows - 1);
        rr * cols + c
    };
    for &i in &valid {
        let q = &pts[i];
        let k = node(q.x, q.y);
        ihv[k] = ihv[k].max(-q.z);
    }
    fill_scanline(&mut ihv, cols, rows);

    // Cloth: positions now and before, free or stuck.
    let start = top + 1.0;
    let mut z = vec![start; n];
    let mut old = vec![start; n];
    let mut free = vec![true; n];
    let rig = usize::from(p.rigidness.clamp(1, 3));
    let accel = -GRAVITY * p.time_step * p.time_step;
    for _ in 0..p.iterations {
        let mut moved = 0.0f64;
        for k in 0..n {
            if !free[k] {
                continue;
            }
            let now = z[k];
            z[k] = now + (now - old[k]) * (1.0 - DAMPING) + accel;
            old[k] = now;
        }
        // Springs between 4-neighbours, applied `rigidness` times.
        for _ in 0..rig {
            for rr in 0..rows {
                for c in 0..cols {
                    let k = rr * cols + c;
                    for j in [
                        (c + 1 < cols).then(|| k + 1),
                        (rr + 1 < rows).then(|| k + cols),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let d = z[j] - z[k];
                        match (free[k], free[j]) {
                            (true, true) => {
                                z[k] += d * DOUBLE_MOVE[rig];
                                z[j] -= d * DOUBLE_MOVE[rig];
                            }
                            (true, false) => z[k] += d * SINGLE_MOVE[rig],
                            (false, true) => z[j] -= d * SINGLE_MOVE[rig],
                            (false, false) => {}
                        }
                    }
                }
            }
        }
        for k in 0..n {
            if free[k] {
                if z[k] < ihv[k] {
                    z[k] = ihv[k];
                    free[k] = false;
                }
                moved = moved.max((z[k] - old[k]).abs());
            }
        }
        if moved < SETTLED {
            break;
        }
    }

    // Ground: within the threshold of the cloth, interpolated under the point.
    for &i in &valid {
        let q = &pts[i];
        let (fx, fy) = ((q.x - ox) / r, (q.y - oy) / r);
        let (c, rr) = (fx.floor() as usize, fy.floor() as usize);
        if c + 1 >= cols || rr + 1 >= rows {
            continue;
        }
        let (tx, ty) = (fx - c as f64, fy - rr as f64);
        let at = |c: usize, r: usize| z[r * cols + c];
        let s = (at(c, rr) * (1.0 - tx) + at(c + 1, rr) * tx) * (1.0 - ty)
            + (at(c, rr + 1) * (1.0 - tx) + at(c + 1, rr + 1) * tx) * ty;
        out[i] = (s - -q.z).abs() < p.threshold;
    }
    Ok(out)
}

/// Gives particles with no point the nearest collision height along their row or column,
/// as CSF does (scanline search), else the lowest one.
fn fill_scanline(v: &mut [f64], cols: usize, rows: usize) {
    let src = v.to_vec();
    let low = src
        .iter()
        .copied()
        .filter(|h| h.is_finite())
        .fold(f64::INFINITY, f64::min);
    for r in 0..rows {
        for c in 0..cols {
            let k = r * cols + c;
            if src[k].is_finite() {
                continue;
            }
            let mut best: Option<(usize, f64)> = None;
            let mut look = |d: usize, h: f64| {
                if h.is_finite() && best.is_none_or(|b| d < b.0) {
                    best = Some((d, h));
                }
            };
            if let Some(d) = (1..=c).find(|d| src[k - d].is_finite()) {
                look(d, src[k - d]);
            }
            if let Some(d) = (1..cols - c).find(|d| src[k + d].is_finite()) {
                look(d, src[k + d]);
            }
            if let Some(d) = (1..=r).find(|d| src[k - d * cols].is_finite()) {
                look(d, src[k - d * cols]);
            }
            if let Some(d) = (1..rows - r).find(|d| src[k + d * cols].is_finite()) {
                look(d, src[k + d * cols]);
            }
            v[k] = best.map_or(low, |b| b.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64) -> Point {
        Point {
            x,
            y,
            z,
            ..Default::default()
        }
    }

    #[test]
    fn a_cloth_too_fine_for_the_area_is_an_error_not_an_empty_result() {
        // 20 km across at 1 cm: 4·10¹² particles
        let pts = [pt(0.0, 0.0, 1.0), pt(20_000.0, 20_000.0, 1.0)];
        let p = CsfParams {
            resolution: 0.01,
            ..CsfParams::default()
        };
        assert!(matches!(ground(&pts, &p), Err(Error::Param(_))));
        // an extent so large that the particle count cannot even be an integer
        let far = [pt(0.0, 0.0, 1.0), pt(1e300, 1e300, 1.0)];
        let p = CsfParams {
            resolution: 1e-6,
            ..CsfParams::default()
        };
        assert!(matches!(ground(&far, &p), Err(Error::Param(_))));
    }

    #[test]
    fn a_resolution_that_is_not_positive_is_an_error() {
        let pts = [pt(0.0, 0.0, 1.0)];
        for r in [0.0, -1.0, f64::NAN] {
            let p = CsfParams {
                resolution: r,
                ..CsfParams::default()
            };
            assert!(matches!(ground(&pts, &p), Err(Error::Param(_))), "{r}");
        }
        // no points take no part: nothing to complain about
        assert!(
            ground(
                &[],
                &CsfParams {
                    resolution: 0.0,
                    ..CsfParams::default()
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn a_cloth_settles_on_the_ground_under_a_tree() -> Result<()> {
        let mut pts = Vec::new();
        for i in 0..60 {
            for j in 0..60 {
                pts.push(Point {
                    x: f64::from(i) * 0.5,
                    y: f64::from(j) * 0.5,
                    z: 50.0 + 0.02 * f64::from(i),
                    ..Default::default()
                });
            }
        }
        for i in 0..10 {
            for j in 0..10 {
                pts.push(Point {
                    x: 12.0 + f64::from(i) * 0.5,
                    y: 12.0 + f64::from(j) * 0.5,
                    z: 62.0,
                    ..Default::default()
                });
            }
        }
        let g = ground(&pts, &CsfParams::default())?;
        assert!(g[..3600].iter().filter(|g| **g).count() > 3500);
        assert!(g[3600..].iter().all(|g| !g));
        Ok(())
    }
}
