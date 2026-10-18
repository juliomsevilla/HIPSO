//! More classes for the classification pipeline, from the geometry of the points around each
//! one: vegetation in three height strata (low 3, medium 4, high 5), wires (14) and poles or
//! towers (15). They run after ground and buildings on the same tiles, so they know which
//! points are ground.
//!
//! Wires and poles look at the points above the ground only. In the neighbourhood of a point
//! the three eigenvalues l1 ≥ l2 ≥ l3 of the covariance say how the points lie: a line has
//! linearity (l1 − l2)/l1 near 1. A wire is a nearly horizontal line high above the ground;
//! a pole is a thin, nearly vertical one.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::las::Point;

pub const LOW_VEGETATION: u8 = 3;
pub const MEDIUM_VEGETATION: u8 = 4;
pub const HIGH_VEGETATION: u8 = 5;
pub const WIRE: u8 = 14;
pub const POLE: u8 = 15;

#[derive(Clone, Debug, PartialEq)]
pub struct VegParams {
    /// Height above ground up to which vegetation is low, and up to which it is medium.
    pub low: f64,
    pub medium: f64,
    /// Cell of the ground surface the heights come from.
    pub cell: f64,
}

impl Default for VegParams {
    fn default() -> Self {
        Self {
            low: 0.5,
            medium: 2.0,
            cell: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WireParams {
    /// Lowest height above ground of a wire.
    pub min_height: f64,
    /// Radius of the neighbourhood.
    pub radius: f64,
    /// Smallest linearity (0–1).
    pub linearity: f64,
    /// Steepest slope of the line (|dz| of its direction, 0 horizontal – 1 vertical).
    pub max_slope: f64,
}

impl Default for WireParams {
    fn default() -> Self {
        Self {
            min_height: 5.0,
            radius: 1.5,
            linearity: 0.9,
            max_slope: 0.5,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoleParams {
    pub min_height: f64,
    pub radius: f64,
    pub linearity: f64,
    /// Smallest verticality of the line (|dz| of its direction).
    pub min_vertical: f64,
    /// Widest the neighbourhood may be across the line (standard deviation, m).
    pub max_width: f64,
}

impl Default for PoleParams {
    fn default() -> Self {
        Self {
            min_height: 2.0,
            radius: 1.0,
            linearity: 0.9,
            min_vertical: 0.9,
            max_width: 0.25,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub vegetation: Option<VegParams>,
    pub wires: Option<WireParams>,
    pub poles: Option<PoleParams>,
}

impl Extras {
    pub fn any(&self) -> bool {
        self.vegetation.is_some() || self.wires.is_some() || self.poles.is_some()
    }

    /// A tag for file names.
    pub fn tag(&self) -> String {
        let mut t = String::new();
        if self.vegetation.is_some() {
            t.push_str("_veg");
        }
        if self.wires.is_some() {
            t.push_str("_cables");
        }
        if self.poles.is_some() {
            t.push_str("_postes");
        }
        t
    }
}

/// Surface through the ground points, for heights above ground.
fn ground_surface(pts: &[Point], ground: &[bool], cell: f64) -> Option<Grid> {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for q in pts.iter().filter(|q| !q.is_noise()) {
        (x0, y0, x1, y1) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y));
    }
    if x1 < x0 {
        return None;
    }
    let mut g = Grid::covering([x0, y0, x1, y1], cell).ok()?;
    let mut sum = vec![0f64; g.len()];
    let mut n = vec![0u32; g.len()];
    for (q, _) in pts.iter().zip(ground).filter(|(_, g)| **g) {
        if let Some(i) = g.index(q.x, q.y) {
            sum[i] += q.z;
            n[i] += 1;
        }
    }
    if n.iter().all(|c| *c == 0) {
        return None;
    }
    for i in 0..g.len() {
        g.data[i] = if n[i] > 0 {
            (sum[i] / f64::from(n[i])) as f32
        } else {
            f32::NAN
        };
    }
    g.fill_holes();
    Some(g)
}

type Cell = (i32, i32, i32);

/// For each point of `set`, (linearity, |dz| of the main direction, width across it) of its
/// neighbours of `set` within `radius`; `None` with fewer than four.
fn line_features(pts: &[Point], set: &[usize], radius: f64) -> Vec<Option<(f64, f64, f64)>> {
    let key = |q: &Point| -> Cell {
        (
            (q.x / radius).floor() as i32,
            (q.y / radius).floor() as i32,
            (q.z / radius).floor() as i32,
        )
    };
    let mut cells: HashMap<Cell, Vec<u32>> = HashMap::new();
    for (j, &i) in set.iter().enumerate() {
        cells.entry(key(&pts[i])).or_default().push(j as u32);
    }
    let r2 = radius * radius;
    (0..set.len())
        .into_par_iter()
        .map(|j| {
            let q = &pts[set[j]];
            let (cx, cy, cz) = key(q);
            let mut nb: Vec<[f64; 3]> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(list) = cells.get(&(cx + dx, cy + dy, cz + dz)) else {
                            continue;
                        };
                        for &o in list {
                            let w = &pts[set[o as usize]];
                            if (w.x - q.x).powi(2) + (w.y - q.y).powi(2) + (w.z - q.z).powi(2) <= r2
                            {
                                nb.push([w.x, w.y, w.z]);
                            }
                        }
                    }
                }
            }
            if nb.len() < 4 {
                return None;
            }
            let (_, cov, _) = crate::tls::covariance(nb.iter().copied());
            let (vals, vec) = crate::tls::eigen(cov);
            if vals[0] <= 1e-12 {
                return None;
            }
            Some((
                (vals[0] - vals[1]) / vals[0],
                vec[2].abs(),
                vals[1].max(0.0).sqrt(),
            ))
        })
        .collect()
}

/// Gives the extra classes to the points of a tile. `class` holds the class each step decided
/// (`None`: the point's own); `ground` says which points are ground.
pub fn apply(pts: &[Point], ground: &[bool], class: &mut [Option<u8>], e: &Extras) {
    let now = |class: &[Option<u8>], k: usize| class[k].unwrap_or(pts[k].class);
    let surface_cell = e.vegetation.as_ref().map_or(1.0, |v| v.cell.max(0.25));
    let needs_heights = e.any();
    let surface = if needs_heights {
        ground_surface(pts, ground, surface_cell)
    } else {
        None
    };
    let Some(surface) = surface else { return };
    let height = |k: usize| -> f64 {
        surface
            .sample(pts[k].x, pts[k].y)
            .map_or(0.0, |g| pts[k].z - f64::from(g))
    };
    let above: Vec<usize> = (0..pts.len())
        .filter(|&k| !ground[k] && !pts[k].is_noise() && now(class, k) != 2 && now(class, k) != 6)
        .collect();
    if let Some(v) = &e.vegetation {
        for &k in &above {
            let h = height(k);
            class[k] = Some(if h < v.low {
                LOW_VEGETATION
            } else if h < v.medium {
                MEDIUM_VEGETATION
            } else {
                HIGH_VEGETATION
            });
        }
    }
    if let Some(w) = &e.wires {
        let set: Vec<usize> = above
            .iter()
            .copied()
            .filter(|&k| height(k) >= w.min_height)
            .collect();
        for (&k, f) in set.iter().zip(line_features(pts, &set, w.radius)) {
            if let Some((lin, dz, _)) = f
                && lin >= w.linearity
                && dz <= w.max_slope
            {
                class[k] = Some(WIRE);
            }
        }
    }
    if let Some(p) = &e.poles {
        let set: Vec<usize> = above
            .iter()
            .copied()
            .filter(|&k| height(k) >= p.min_height && now(class, k) != WIRE)
            .collect();
        for (&k, f) in set.iter().zip(line_features(pts, &set, p.radius)) {
            if let Some((lin, dz, width)) = f
                && lin >= p.linearity
                && dz >= p.min_vertical
                && width <= p.max_width
            {
                class[k] = Some(POLE);
            }
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
            class: 1,
            return_number: 1,
            number_of_returns: 1,
            ..Point::default()
        }
    }

    #[test]
    fn strata_wire_and_pole() {
        let mut pts = Vec::new();
        // flat ground
        for i in 0..30 {
            for j in 0..30 {
                pts.push(pt(f64::from(i), f64::from(j), 0.0));
            }
        }
        let ground = vec![true; pts.len()];
        let n_ground = pts.len();
        // vegetation at 0.2, 1.0 and 6.0 m
        for (k, z) in [0.2, 1.0, 6.0].into_iter().enumerate() {
            pts.push(pt(5.0 + k as f64 * 0.3, 5.0, z));
        }
        // a wire at 8 m along x, a pole along z at x = 20
        for i in 0..40 {
            pts.push(pt(2.0 + f64::from(i) * 0.5, 25.0, 8.0));
        }
        for i in 0..30 {
            pts.push(pt(20.0, 10.0, 0.5 + f64::from(i) * 0.3));
        }
        let mut ground_all = ground;
        ground_all.resize(pts.len(), false);
        let mut class = vec![None; pts.len()];
        let e = Extras {
            vegetation: Some(VegParams::default()),
            wires: Some(WireParams::default()),
            poles: Some(PoleParams::default()),
        };
        apply(&pts, &ground_all, &mut class, &e);
        assert_eq!(class[n_ground], Some(LOW_VEGETATION));
        assert_eq!(class[n_ground + 1], Some(MEDIUM_VEGETATION));
        assert_eq!(class[n_ground + 2], Some(HIGH_VEGETATION));
        let wires = class.iter().filter(|c| **c == Some(WIRE)).count();
        let poles = class.iter().filter(|c| **c == Some(POLE)).count();
        assert!(wires >= 35, "{wires}");
        assert!(poles >= 25, "{poles}");
    }
}
