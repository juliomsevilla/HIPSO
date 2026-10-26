//! More classes for the classification pipeline, from the geometry of the points around each
//! one: vegetation in three height strata (low 3, medium 4, high 5), wires (14) and poles or
//! towers (15). They run after ground and buildings on the same tiles, so they know which
//! points are ground.
//!
//! Wires and poles look at the points above the ground only. In the neighbourhood of a point
//! the three eigenvalues l1 ≥ l2 ≥ l3 of the covariance say how the points lie: a line has
//! linearity (l1 − l2)/l1 near 1. A wire is a nearly horizontal line high above the ground, in
//! the air and long: the scan lines of a survey make pieces of crown straight for a metre or
//! two, so short chains and crowded neighbourhoods are not wires. A pole or tower is a narrow
//! structure, isolated from the crowns, that rises from the ground.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::las::Point;

pub const LOW_VEGETATION: u8 = 3;
pub const MEDIUM_VEGETATION: u8 = 4;
pub const HIGH_VEGETATION: u8 = 5;
pub const WIRE: u8 = 14;
pub const POLE: u8 = 15;

/// Lowest a tower head is above the ground (m).
const HEAD_MIN_HEIGHT: f64 = 8.0;

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
    /// A wire hangs in the air: at most this many points above `min_height` within
    /// `2 × radius` of it. A branch in a crown has dozens.
    pub max_neighbours: usize,
    /// A wire is continuous: the chain of wire points (each within `2 × radius` of the next)
    /// must reach this far. The scan lines of a survey make pieces of crown look straight for
    /// a metre or two, never for several.
    pub min_length: f64,
}

impl Default for WireParams {
    fn default() -> Self {
        Self {
            min_height: 5.0,
            radius: 1.5,
            linearity: 0.9,
            max_slope: 0.5,
            max_neighbours: 20,
            min_length: 4.0,
        }
    }
}

/// Poles and towers are found as isolated, narrow structures that rise from the ground: at the
/// density of a survey a lattice tower is a loose column of points, not a clean line, so what
/// marks it is its shape and that nothing touches it.
#[derive(Clone, Debug, PartialEq)]
pub struct PoleParams {
    /// Points lower than this above the ground are not part of a structure.
    pub min_height: f64,
    /// Points closer than this belong to the same structure.
    pub radius: f64,
    /// Widest the structure may be (m, either side of the footprint).
    pub max_width: f64,
    /// Least it must rise (m).
    pub min_extent: f64,
    /// At the density of a survey only the head of a tower may come back: a crossarm a few
    /// metres long, high up, with nothing under it. A short isolated chain of points that
    /// high and that long (m, 0 to turn off) is taken for the head of a tower. A conductor
    /// is longer, and a crown has crown under it.
    pub head_length: f64,
}

impl Default for PoleParams {
    fn default() -> Self {
        Self {
            min_height: 2.0,
            radius: 1.5,
            max_width: 8.0,
            min_extent: 6.0,
            head_length: 12.0,
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

/// For each point of `set`, how many other points of `set` lie within `radius`.
fn neighbour_counts(pts: &[Point], set: &[usize], radius: f64) -> Vec<usize> {
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
            let mut n = 0;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for &o in cells
                            .get(&(cx + dx, cy + dy, cz + dz))
                            .into_iter()
                            .flatten()
                        {
                            let w = &pts[set[o as usize]];
                            if (w.x - q.x).powi(2) + (w.y - q.y).powi(2) + (w.z - q.z).powi(2) <= r2
                            {
                                n += 1;
                            }
                        }
                    }
                }
            }
            n - 1
        })
        .collect()
}

/// Each chain's bounding box `[x0, y0, z0, x1, y1, z1]` and size, by chain.
type ChainBoxes = HashMap<usize, ([f64; 6], usize)>;

/// Groups the points `cand` (indices into `pts`) into chains: two points within `link` of each
/// other are in the same one. Returns each point's group and each group's bounding box
/// `[x0, y0, z0, x1, y1, z1]` and size.
fn chains(pts: &[Point], cand: &[usize], link: f64) -> (Vec<usize>, ChainBoxes) {
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let key = |q: &Point| -> Cell {
        (
            (q.x / link).floor() as i32,
            (q.y / link).floor() as i32,
            (q.z / link).floor() as i32,
        )
    };
    let mut cells: HashMap<Cell, Vec<usize>> = HashMap::new();
    for (j, &i) in cand.iter().enumerate() {
        cells.entry(key(&pts[i])).or_default().push(j);
    }
    let mut parent: Vec<usize> = (0..cand.len()).collect();
    let l2 = link * link;
    for (j, &i) in cand.iter().enumerate() {
        let q = &pts[i];
        let (cx, cy, cz) = key(q);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &o in cells
                        .get(&(cx + dx, cy + dy, cz + dz))
                        .into_iter()
                        .flatten()
                    {
                        let w = &pts[cand[o]];
                        if o > j
                            && (w.x - q.x).powi(2) + (w.y - q.y).powi(2) + (w.z - q.z).powi(2) <= l2
                        {
                            let (a, b) = (root(&mut parent, j), root(&mut parent, o));
                            parent[a] = b;
                        }
                    }
                }
            }
        }
    }
    let group: Vec<usize> = (0..cand.len()).map(|j| root(&mut parent, j)).collect();
    let mut boxes: HashMap<usize, ([f64; 6], usize)> = HashMap::new();
    for (&g, &i) in group.iter().zip(cand) {
        let q = &pts[i];
        let (b, n) = boxes.entry(g).or_insert((
            [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN],
            0,
        ));
        for (k, v) in [q.x, q.y, q.z].into_iter().enumerate() {
            b[k] = b[k].min(v);
            b[k + 3] = b[k + 3].max(v);
        }
        *n += 1;
    }
    (group, boxes)
}

/// Which of the points `cand` belong to a chain whose bounding box diagonal is at least
/// `min_len`.
fn long_chains(pts: &[Point], cand: &[usize], link: f64, min_len: f64) -> Vec<bool> {
    let (group, boxes) = chains(pts, cand, link);
    group
        .iter()
        .map(|g| {
            let (b, _) = boxes[g];
            ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt()
                >= min_len
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
        let crowd = neighbour_counts(pts, &set, 2.0 * w.radius);
        let mut cand: Vec<usize> = Vec::new();
        for ((&k, f), n) in set
            .iter()
            .zip(line_features(pts, &set, w.radius))
            .zip(crowd)
        {
            if let Some((lin, dz, _)) = f
                && lin >= w.linearity
                && dz <= w.max_slope
                && n <= w.max_neighbours
            {
                cand.push(k);
            }
        }
        for (&k, keep) in cand
            .iter()
            .zip(long_chains(pts, &cand, 2.0 * w.radius, w.min_length))
        {
            if keep {
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
        // what stands around a structure: points more than 3 m above the ground, by 3 m cell
        let mut under: HashMap<(i32, i32), Vec<[f64; 3]>> = HashMap::new();
        for &k in &above {
            if height(k) >= 3.0 {
                under
                    .entry((
                        (pts[k].x / 3.0).floor() as i32,
                        (pts[k].y / 3.0).floor() as i32,
                    ))
                    .or_default()
                    .push([pts[k].x, pts[k].y, pts[k].z]);
            }
        }
        let (group, boxes) = chains(pts, &set, p.radius);
        let mut members: HashMap<usize, usize> = HashMap::new();
        for (&k, g) in set.iter().zip(&group) {
            if height(k) >= 3.0 {
                *members.entry(*g).or_default() += 1;
            }
        }
        // points more than 3 m above the ground within 3 m of the chain on the plan, those of
        // the chain apart: a tower in a corridor has none, a tree at the edge of a stand has
        let crowded = |g: &usize| -> bool {
            let (b, _) = boxes[g];
            let (cx0, cx1) = (
                (b[0] / 3.0).floor() as i32 - 1,
                (b[3] / 3.0).floor() as i32 + 1,
            );
            let (cy0, cy1) = (
                (b[1] / 3.0).floor() as i32 - 1,
                (b[4] / 3.0).floor() as i32 + 1,
            );
            let near: usize = (cx0..=cx1)
                .flat_map(|cx| (cy0..=cy1).map(move |cy| (cx, cy)))
                .filter_map(|c| under.get(&c))
                .flatten()
                .filter(|q| {
                    q[0] > b[0] - 3.0 && q[0] < b[3] + 3.0 && q[1] > b[1] - 3.0 && q[1] < b[4] + 3.0
                })
                .count();
            near > members.get(g).copied().unwrap_or(0) + 2
        };
        // a structure stands on the ground, is narrow for its height and is sparse; a crown is
        // wide and dense, a forest is one chain far wider than any tower
        let is_pole = |g: &usize| -> bool {
            let (b, n) = boxes[g];
            let width = (b[3] - b[0]).max(b[4] - b[1]);
            let extent = b[5] - b[2];
            n >= 5
                && width <= p.max_width
                && extent >= p.min_extent
                && width <= 0.35 * extent
                && (n as f64) <= 12.0 * extent
        };
        let low = |g: &usize| -> bool {
            // reaches the ground: some point of the chain is within 3 m of it
            let (b, _) = boxes[g];
            let (x, y) = ((b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0);
            surface
                .sample(x, y)
                .is_some_and(|gz| b[2] - f64::from(gz) <= p.min_height + 3.0)
        };
        let poles: std::collections::HashSet<usize> = boxes
            .keys()
            .filter(|g| is_pole(g) && low(g) && !crowded(g))
            .copied()
            .collect();
        for (&k, g) in set.iter().zip(&group) {
            if poles.contains(g) {
                class[k] = Some(POLE);
            }
        }
        if p.head_length > 0.0 {
            let high: Vec<usize> = above
                .iter()
                .copied()
                .filter(|&k| height(k) >= HEAD_MIN_HEIGHT && class[k] != Some(POLE))
                .collect();
            let (group, boxes) = chains(pts, &high, p.radius);
            // a crossarm is a straight bar: the spread of its points on the plan is long
            // one way and thin the other (a piece of crown is a blob)
            let mut plan: HashMap<usize, Vec<[f64; 2]>> = HashMap::new();
            for (&k, g) in high.iter().zip(&group) {
                plan.entry(*g).or_default().push([pts[k].x, pts[k].y]);
            }
            let straight = |g: &usize| -> bool {
                let q = &plan[g];
                let n = q.len() as f64;
                let (mx, my) = (
                    q.iter().map(|v| v[0]).sum::<f64>() / n,
                    q.iter().map(|v| v[1]).sum::<f64>() / n,
                );
                let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
                for v in q {
                    sxx += (v[0] - mx).powi(2);
                    syy += (v[1] - my).powi(2);
                    sxy += (v[0] - mx) * (v[1] - my);
                }
                let (sxx, syy, sxy) = (sxx / n, syy / n, sxy / n);
                let mid = (sxx + syy) / 2.0;
                let d = ((sxx - syy) / 2.0).hypot(sxy);
                let (l1, l2) = (mid + d, (mid - d).max(0.0));
                l2.sqrt() <= 0.6 && l1.sqrt() >= 3.0 * l2.sqrt()
            };
            let is_head = |g: &usize| -> bool {
                let (b, n) = boxes[g];
                if !straight(g) {
                    return false;
                }
                let diag =
                    ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
                if n < 8 || diag < 3.0 || diag > p.head_length {
                    return false;
                }
                // nothing within 3 m of the chain on the plan and under it, down to the ground
                let (cx0, cx1) = (
                    (b[0] / 3.0).floor() as i32 - 1,
                    (b[3] / 3.0).floor() as i32 + 1,
                );
                let (cy0, cy1) = (
                    (b[1] / 3.0).floor() as i32 - 1,
                    (b[4] / 3.0).floor() as i32 + 1,
                );
                !(cx0..=cx1).any(|cx| {
                    (cy0..=cy1).any(|cy| {
                        under.get(&(cx, cy)).is_some_and(|ps| {
                            ps.iter().any(|q| {
                                q[2] < b[2] - 1.0
                                    && q[0] > b[0] - 3.0
                                    && q[0] < b[3] + 3.0
                                    && q[1] > b[1] - 3.0
                                    && q[1] < b[4] + 3.0
                            })
                        })
                    })
                })
            };
            let heads: std::collections::HashSet<usize> =
                boxes.keys().filter(|g| is_head(g)).copied().collect();
            for (&k, g) in high.iter().zip(&group) {
                if heads.contains(g) {
                    class[k] = Some(POLE);
                }
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
        // a lattice tower far from the rest: a loose column 2 m wide, 18 m tall
        let tower_from = pts.len();
        for i in 0..24 {
            let z = 1.0 + f64::from(i) * 0.75;
            let w = 1.0 - z / 40.0;
            pts.push(pt(10.0 - w, 3.0, z));
            pts.push(pt(10.0 + w, 3.0 + w, z + 0.3));
        }
        let tower_to = pts.len();
        // a lone crown, wide for its height, must not count
        let crown_from = pts.len();
        for i in 0..8 {
            for j in 0..8 {
                pts.push(pt(
                    24.0 + f64::from(i) * 0.4,
                    6.0 + f64::from(j) * 0.4,
                    5.0 + f64::from(i % 3),
                ));
            }
        }
        // the head of a tower far from anything: a 6 m crossarm 12 m up with nothing under it
        let head_from = pts.len();
        for i in 0..24 {
            pts.push(pt(27.0, 12.0 + f64::from(i) * 0.25, 12.0));
        }
        let head_to = pts.len();
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
        let tower = class[tower_from..tower_to]
            .iter()
            .filter(|c| **c == Some(POLE))
            .count();
        assert!(tower >= 40, "{tower}");
        assert!(
            class[crown_from..head_from]
                .iter()
                .all(|c| *c != Some(POLE))
        );
        assert!(
            class[head_from..head_to].iter().all(|c| *c == Some(POLE)),
            "{:?}",
            &class[head_from..head_to]
        );
    }
}
