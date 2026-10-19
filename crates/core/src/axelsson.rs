//! Ground by progressive TIN densification (Axelsson 2000, "Adaptive surface models for the
//! filtering of airborne laser scanner data", and the many filters built on it).
//!
//! A sparse TIN is made of seed points: the lowest of each large cell, so that no building fits
//! in a cell and every cell has a point on the ground. Then, again and again, every point inside a
//! triangle of the TIN is tested against the plane of the triangle: it is ground when it is near
//! the plane (the iteration distance) and the lines from it to the three vertices make small
//! angles with the plane (the iteration angle). The points accepted are added to the TIN, which
//! is made again, until none is accepted. Steep ground gets points in a few steps, where the
//! angle that is allowed is small at each one.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::las::Point;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct AxelssonParams {
    /// Side (m) of the cells whose lowest points seed the TIN: larger than the largest building
    /// or structure in the area that is not ground.
    pub seed_cell: f64,
    /// Largest angle (degrees) between the plane of a triangle and the line from a vertex to a
    /// point that is added to the ground.
    pub iter_angle: f64,
    /// Largest distance (m) from the plane of a triangle to a point that is added.
    pub iter_dist: f64,
    /// Points this close to the plane (m, noise of the sensor) are ground whatever the angle,
    /// which near a vertex is large even for a point that lies on the surface.
    pub flat_tolerance: f64,
    /// Triangles whose longest edge is shorter than this (m) have the angle reduced in
    /// proportion (down to half), so that a dense TIN does not take in low vegetation.
    pub reduce_edge: f64,
    /// Triangles whose longest edge is shorter than this (m) take no more points: the surface
    /// there is dense enough (0: they go on until nothing is added).
    pub min_edge: f64,
    pub max_iterations: u32,
}

impl Default for AxelssonParams {
    fn default() -> Self {
        Self {
            seed_cell: 20.0,
            iter_angle: 8.0,
            iter_dist: 1.4,
            flat_tolerance: 0.10,
            reduce_edge: 5.0,
            min_edge: 0.0,
            max_iterations: 30,
        }
    }
}

impl AxelssonParams {
    fn validate(&self) -> Result<()> {
        if !(self.seed_cell > 0.0)
            || !(self.iter_angle > 0.0 && self.iter_angle < 90.0)
            || !(self.iter_dist > 0.0)
            || !(self.flat_tolerance >= 0.0)
            || !(self.reduce_edge >= 0.0)
            || !(self.min_edge >= 0.0)
            || self.max_iterations == 0
        {
            return Err(Error::Param(
                "the progressive TIN filter needs a positive seed cell, angle and distance, \
                 and at least 1 iteration"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// A vertex of the TIN.
#[derive(Clone, Copy)]
struct V {
    x: f64,
    y: f64,
    z: f64,
}

/// The candidate points bucketed in a regular grid, so that the ones inside a triangle are found
/// by looking at the cells its box covers.
struct Buckets {
    cell: f64,
    cols: usize,
    rows: usize,
    start: Vec<u32>,
    items: Vec<u32>,
}

impl Buckets {
    fn new(xy: &[(f64, f64)], ids: &[usize], extent: (f64, f64)) -> Self {
        // about 3 points to a cell
        let cell = ((extent.0 * extent.1 * 3.0 / ids.len().max(1) as f64).sqrt()).max(0.25);
        let cols = ((extent.0 / cell).floor() as usize + 1).min(1 << 14);
        let rows = ((extent.1 / cell).floor() as usize + 1).min(1 << 14);
        let at = |i: usize| -> usize {
            let (x, y) = xy[i];
            let c = ((x / cell).max(0.0) as usize).min(cols - 1);
            let r = ((y / cell).max(0.0) as usize).min(rows - 1);
            r * cols + c
        };
        let mut start = vec![0u32; cols * rows + 1];
        for &i in ids {
            start[at(i) + 1] += 1;
        }
        for k in 0..cols * rows {
            start[k + 1] += start[k];
        }
        let mut fill = start.clone();
        let mut items = vec![0u32; ids.len()];
        for &i in ids {
            let slot = &mut fill[at(i)];
            items[*slot as usize] = i as u32;
            *slot += 1;
        }
        Self {
            cell,
            cols,
            rows,
            start,
            items,
        }
    }

    /// The candidates in the cells that the box covers.
    fn in_box(&self, lo: (f64, f64), hi: (f64, f64)) -> impl Iterator<Item = usize> + '_ {
        let c0 = ((lo.0 / self.cell).max(0.0) as usize).min(self.cols - 1);
        let c1 = ((hi.0 / self.cell).max(0.0) as usize).min(self.cols - 1);
        let r0 = ((lo.1 / self.cell).max(0.0) as usize).min(self.rows - 1);
        let r1 = ((hi.1 / self.cell).max(0.0) as usize).min(self.rows - 1);
        (r0..=r1).flat_map(move |r| {
            let (a, b) = (
                self.start[r * self.cols + c0] as usize,
                self.start[r * self.cols + c1 + 1] as usize,
            );
            self.items[a..b].iter().map(|&i| i as usize)
        })
    }
}

/// The candidates of one triangle that are ground: near its plane, with small angles to the
/// vertices.
fn accepted(
    tri: [V; 3],
    xy: &[(f64, f64)],
    z: &[f64],
    buckets: &Buckets,
    p: &AxelssonParams,
) -> Vec<usize> {
    let [a, b, c] = tri;
    let edge = |u: V, v: V| (u.x - v.x).hypot(u.y - v.y);
    let longest = edge(a, b).max(edge(b, c)).max(edge(c, a));
    if longest < p.min_edge {
        return Vec::new();
    }
    // the plane through the vertices: a normal n; a vertical one has no height to compare
    let (ux, uy, uz) = (b.x - a.x, b.y - a.y, b.z - a.z);
    let (vx, vy, vz) = (c.x - a.x, c.y - a.y, c.z - a.z);
    let n = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    if n.2.abs() < 1e-9 {
        return Vec::new();
    }
    let norm = (n.0 * n.0 + n.1 * n.1 + n.2 * n.2).sqrt();
    let reduce = if longest < p.reduce_edge && p.reduce_edge > 0.0 {
        (longest / p.reduce_edge).clamp(0.5, 1.0)
    } else {
        1.0
    };
    let max_angle = (p.iter_angle * reduce).to_radians();
    let lo = (a.x.min(b.x).min(c.x), a.y.min(b.y).min(c.y));
    let hi = (a.x.max(b.x).max(c.x), a.y.max(b.y).max(c.y));
    let area = ux * vy - uy * vx;
    let mut out = Vec::new();
    for q in buckets.in_box(lo, hi) {
        let (qx, qy) = xy[q];
        // inside the triangle: the three barycentric weights are not negative
        let l1 = ((b.y - c.y) * (qx - c.x) + (c.x - b.x) * (qy - c.y)) / area;
        let l2 = ((c.y - a.y) * (qx - c.x) + (a.x - c.x) * (qy - c.y)) / area;
        if l1 < -1e-9 || l2 < -1e-9 || 1.0 - l1 - l2 < -1e-9 {
            continue;
        }
        // distance to the plane, measured perpendicular to it
        let zp = a.z - (n.0 * (qx - a.x) + n.1 * (qy - a.y)) / n.2;
        let d = (z[q] - zp) * n.2.abs() / norm;
        if d.abs() <= p.flat_tolerance {
            out.push(q);
            continue;
        }
        if d.abs() > p.iter_dist {
            continue;
        }
        let steep = [a, b, c].iter().any(|v| {
            let r = ((qx - v.x).powi(2) + (qy - v.y).powi(2) + (z[q] - v.z).powi(2)).sqrt();
            r > 1e-9 && (d.abs() / r).min(1.0).asin() > max_angle
        });
        if !steep {
            out.push(q);
        }
    }
    out
}

/// Which points are ground. Noise and withheld points are never ground and take no part.
pub fn ground(pts: &[Point], p: &AxelssonParams) -> Result<Vec<bool>> {
    p.validate()?;
    let n = pts.len();
    let mut ground = vec![false; n];
    let valid: Vec<usize> = (0..n)
        .filter(|&i| !pts[i].is_noise() && pts[i].x.is_finite() && pts[i].y.is_finite())
        .collect();
    if valid.len() < 3 {
        return Ok(ground);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &i in &valid {
        (x0, y0, x1, y1) = (
            x0.min(pts[i].x),
            y0.min(pts[i].y),
            x1.max(pts[i].x),
            y1.max(pts[i].y),
        );
    }
    // coordinates relative to the corner keep the triangulation precise
    let xy: Vec<(f64, f64)> = pts.iter().map(|q| (q.x - x0, q.y - y0)).collect();
    let z: Vec<f64> = pts.iter().map(|q| q.z).collect();
    let extent = (x1 - x0, y1 - y0);
    // the seeds: the lowest point of each cell
    let mut lowest: HashMap<(i64, i64), usize> = HashMap::new();
    for &i in &valid {
        let key = (
            (xy[i].0 / p.seed_cell).floor() as i64,
            (xy[i].1 / p.seed_cell).floor() as i64,
        );
        lowest
            .entry(key)
            .and_modify(|j| {
                if z[i] < z[*j] {
                    *j = i;
                }
            })
            .or_insert(i);
    }
    for &i in lowest.values() {
        ground[i] = true;
    }
    let mut candidates: Vec<usize> = valid.iter().copied().filter(|&i| !ground[i]).collect();
    for _ in 0..p.max_iterations {
        crate::check_cancel()?;
        if candidates.is_empty() {
            break;
        }
        let ids: Vec<usize> = valid.iter().copied().filter(|&i| ground[i]).collect();
        if ids.len() < 3 {
            break;
        }
        let corners: Vec<delaunator::Point> = ids
            .iter()
            .map(|&i| delaunator::Point {
                x: xy[i].0,
                y: xy[i].1,
            })
            .collect();
        let t = delaunator::triangulate(&corners);
        if t.triangles.is_empty() {
            break;
        }
        let buckets = Buckets::new(&xy, &candidates, extent);
        let vertex = |k: usize| V {
            x: corners[k].x,
            y: corners[k].y,
            z: z[ids[k]],
        };
        let found: Vec<usize> = t
            .triangles
            .par_chunks_exact(3)
            .flat_map_iter(|tri| {
                accepted(
                    [vertex(tri[0]), vertex(tri[1]), vertex(tri[2])],
                    &xy,
                    &z,
                    &buckets,
                    p,
                )
            })
            .collect();
        let mut added = 0usize;
        for i in found {
            if !ground[i] {
                ground[i] = true;
                added += 1;
            }
        }
        if added == 0 {
            break;
        }
        candidates.retain(|&i| !ground[i]);
    }
    Ok(ground)
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

    /// Terrain on a slope with a gentle wave, a building with a flat roof, and trees.
    fn scene() -> (Vec<Point>, Vec<bool>) {
        let terrain = |x: f64, y: f64| 0.2 * x + 0.1 * y + 0.4 * (x / 9.0).sin();
        let mut pts = Vec::new();
        let mut is_ground = Vec::new();
        let step = 0.5;
        let mut x = 0.0;
        while x <= 100.0 {
            let mut y = 0.0;
            while y <= 100.0 {
                let in_building = (40.0..58.0).contains(&x) && (40.0..52.0).contains(&y);
                let in_tree = [(15.0, 20.0), (75.0, 30.0), (30.0, 80.0), (80.0, 80.0)]
                    .iter()
                    .any(|t: &(f64, f64)| (x - t.0).hypot(y - t.1) < 3.0);
                if in_building {
                    // only the roof, 6 m above the ground
                    pts.push(pt(x, y, terrain(x, y) + 6.0));
                    is_ground.push(false);
                } else {
                    pts.push(pt(x, y, terrain(x, y)));
                    is_ground.push(true);
                    if in_tree {
                        // a crown: points 6 to 12 m above the ground
                        for k in 0..6 {
                            pts.push(pt(x, y, terrain(x, y) + 6.0 + f64::from(k)));
                            is_ground.push(false);
                        }
                    }
                }
                y += step;
            }
            x += step;
        }
        (pts, is_ground)
    }

    #[test]
    fn it_takes_the_ground_of_a_slope_and_leaves_roofs_and_crowns() -> Result<()> {
        let (pts, is_ground) = scene();
        let g = ground(&pts, &AxelssonParams::default())?;
        let (mut hit, mut total, mut wrong, mut others) = (0, 0, 0, 0);
        for (k, truth) in is_ground.iter().enumerate() {
            if *truth {
                total += 1;
                hit += usize::from(g[k]);
            } else {
                others += 1;
                wrong += usize::from(g[k]);
            }
        }
        // nearly all the ground, and no roof or crown point
        assert!(hit as f64 / total as f64 > 0.99, "{hit} of {total}");
        assert_eq!(
            wrong, 0,
            "{wrong} of {others} points above the ground taken"
        );
        Ok(())
    }

    #[test]
    fn a_building_larger_than_the_seed_cell_is_taken_for_ground() -> Result<()> {
        // the seed cell is smaller than the 18 x 12 m roof, so a cell can hold only roof points
        let (pts, is_ground) = scene();
        let small = AxelssonParams {
            seed_cell: 5.0,
            ..AxelssonParams::default()
        };
        let g = ground(&pts, &small)?;
        let roof_taken = is_ground
            .iter()
            .zip(&g)
            .zip(&pts)
            .filter(|((truth, got), q)| !**truth && **got && (40.0..58.0).contains(&q.x))
            .count();
        assert!(roof_taken > 0);
        Ok(())
    }

    #[test]
    fn noise_and_a_cloud_too_small_to_triangulate_are_left_out() -> Result<()> {
        assert_eq!(
            ground(&[pt(0.0, 0.0, 0.0)], &AxelssonParams::default())?,
            [false]
        );
        let bad = AxelssonParams {
            iter_angle: 0.0,
            ..AxelssonParams::default()
        };
        assert!(matches!(ground(&[], &bad), Err(Error::Param(_))));
        Ok(())
    }
}
