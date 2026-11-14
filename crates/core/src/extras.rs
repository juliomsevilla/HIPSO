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
pub const WATER: u8 = 9;
pub const ROAD: u8 = 11;

/// Spacing of the points (m) at which a tower has 12 points per metre of height or fewer.
const POLE_REFERENCE_SPACING: f64 = 0.33;

/// Radius (m) in which a piece of conductor has no other points but its own.
const ALONE: f64 = 2.5;

/// Vertical distances count this much in linking the points of a tower: its pieces are rows of
/// points a few metres apart (a lattice scanned from above), and a vertical gap of 8 m links.
const POLE_Z_SCALE: f64 = 0.19;

/// How many times its own points a pole may have of crown around it (above a third of its height).
const CROWD_TOLERANCE: f64 = 1.6;

/// How many times its own points a thin pole may have around its top (a trunk has its crown).
const CAP_TOLERANCE: f64 = 3.0;

/// Points per metre of height that a tower may have at 9 points/m² (a high-voltage tower with
/// several crossarms has more than a pole).
const TOWER_POINTS_PER_METRE: f64 = 40.0;

/// A structure at least this tall is a tower, whose points grow with the density of the survey; a
/// narrow tree of that height at 45 points/m² has as many, so lower ones keep the count of 9.
const TOWER_HEIGHT: f64 = 15.0;

/// Density factor (see `POLE_REFERENCE_SPACING`) above which tree trunks are resolved as lines.
const TRUNK_DENSITY: f64 = 20.0;

/// Lowest pieces of conductor linked across gaps are above the ground (m): a survey sees them
/// in pieces only when the line is high above everything.
const LINKED_MIN_HEIGHT: f64 = 10.0;

/// Spacing (m) from which wires are looked at in more than one radius.
const WIRE_SPARSE_SPACING: f64 = 0.25;

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
    /// Radius of the neighbourhood (m; 0: six times the spacing of the points, between 0.6 and 3 m,
    /// and with a spacing of 0.25 m or more also 1.5 and 2 times that, up to 4.5 m: the same wire is
    /// a line at 1 m with 45 points/m² and only at 2 to 4 m with 9, where a conductor has a point
    /// every metre or two and crowns beside it make it a line at some radii and not at others).
    pub radius: f64,
    /// Smallest linearity (0–1).
    pub linearity: f64,
    /// Steepest slope of the line (|dz| of its direction, 0 horizontal – 1 vertical).
    pub max_slope: f64,
    /// A wire hangs in the air: at most this many points above `min_height` within
    /// `2 × radius` of it. A branch in a crown has dozens.
    pub max_neighbours: usize,
    /// Widest the neighbourhood may be across its main direction (standard deviation, m): a
    /// conductor is a line, a branch in a crown has width. Unlike a count of neighbours it does
    /// not depend on the density of the points.
    pub max_width: f64,
    /// A wire is continuous: the chain of wire points (each within `2 × radius` of the next)
    /// must reach this far. The scan lines of a survey make pieces of crown look straight for
    /// a metre or two, never for several.
    pub min_length: f64,
    /// A chain of wire points is a line: its length over its width (the square roots of the
    /// two largest eigenvalues of its points). A hedge or a piece of crown that the linearity
    /// of its parts lets through is a blob at that scale.
    pub min_straightness: f64,
    /// Side of the voxels the points are collapsed to before looking at their shape (m).
    pub voxel: f64,
    /// A survey with a few passes over a line sees a conductor only where its scan lines cross
    /// it: short straight pieces (a metre or two) with metres of nothing between them. Pieces
    /// that point at each other, that far apart at most (m, 0 to turn off), make a wire when
    /// three or more line up over at least twice `min_length`.
    pub link_gap: f64,
}

impl Default for WireParams {
    fn default() -> Self {
        Self {
            min_height: 5.0,
            radius: 0.0,
            linearity: 0.7,
            max_slope: 0.5,
            max_neighbours: 1000,
            max_width: 0.0,
            min_length: 10.0,
            min_straightness: 10.0,
            voxel: 0.05,
            link_gap: 15.0,
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
    /// Thin poles (lamp posts, utility poles): a vertical line of points, narrow, that rises at
    /// least this far (m; 0 to turn off).
    pub thin_extent: f64,
}

impl Default for PoleParams {
    fn default() -> Self {
        Self {
            min_height: 2.0,
            radius: 1.5,
            max_width: 8.0,
            min_extent: 6.0,
            head_length: 12.0,
            thin_extent: 3.0,
        }
    }
}

/// Roads are ground that is smooth, flat and dark to the laser: asphalt gives back less
/// intensity than soil, and a road surface is a plane (small roughness and slope). What is
/// measured is the shape of the ground points within `radius`.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadParams {
    /// Highest intensity of a road point, in the sensor's own scale (asphalt is dark); 0: a share
    /// (`relative_intensity`) of the median intensity of the ground of the tile, which is the
    /// same whatever the sensor, its scale (16 bits, 0 to 255) or the flight.
    pub max_intensity: f64,
    /// With `max_intensity` 0, the share of the median ground intensity that a road may reach.
    pub relative_intensity: f64,
    /// Roughest the surface may be: standard deviation to the local plane (m).
    pub max_roughness: f64,
    /// Steepest the local plane may be (degrees).
    pub max_slope: f64,
    /// Radius of the neighbourhood (m).
    pub radius: f64,
}

impl Default for RoadParams {
    fn default() -> Self {
        Self {
            max_intensity: 0.0,
            relative_intensity: 0.6,
            max_roughness: 0.03,
            max_slope: 5.0,
            radius: 1.5,
        }
    }
}

/// Water is a flat sheet that gives back almost nothing to the laser: ground points of a single
/// return, of low intensity, level, that make an extended region whose heights hardly vary.
/// Cells of `cell` metres that hold such points are joined; a region is water when it is at
/// least `min_area` square metres and its heights deviate less than `max_deviation`.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterParams {
    /// Highest intensity of a water point, in the sensor's own scale; 0: a share
    /// (`relative_intensity`) of the median intensity of the ground of the tile.
    pub max_intensity: f64,
    /// With `max_intensity` 0, the share of the median ground intensity that water may reach.
    pub relative_intensity: f64,
    /// Steepest the local plane may be (degrees).
    pub max_slope: f64,
    /// Side of the cells that are joined (m).
    pub cell: f64,
    /// Smallest region (m²).
    pub min_area: f64,
    /// Largest standard deviation of the heights of a region (m).
    pub max_deviation: f64,
}

impl Default for WaterParams {
    fn default() -> Self {
        Self {
            max_intensity: 0.0,
            relative_intensity: 0.375,
            max_slope: 3.0,
            cell: 2.0,
            min_area: 30.0,
            max_deviation: 0.2,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub roads: Option<RoadParams>,
    pub water: Option<WaterParams>,
    pub vegetation: Option<VegParams>,
    pub wires: Option<WireParams>,
    pub poles: Option<PoleParams>,
}

impl Extras {
    pub fn any(&self) -> bool {
        self.vegetation.is_some()
            || self.wires.is_some()
            || self.poles.is_some()
            || self.roads.is_some()
            || self.water.is_some()
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
        if self.roads.is_some() {
            t.push_str("_carreteras");
        }
        if self.water.is_some() {
            t.push_str("_agua");
        }
        t
    }
}

/// The same, for the examples that look at heights.
#[doc(hidden)]
pub fn ground_surface_pub(pts: &[Point], ground: &[bool], cell: f64) -> Option<Grid> {
    ground_surface(pts, ground, cell)
}

/// Surface through the ground points, for heights above ground.
pub(crate) fn ground_surface(pts: &[Point], ground: &[bool], cell: f64) -> Option<Grid> {
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
    chains_z(pts, cand, link, 1.0)
}

/// [`chains`] where heights count `zscale` times as much as distances on the plan (below 1, two
/// points that far apart vertically are still linked: the pieces of a lattice tower are rows of
/// points with metres between them).
fn chains_z(
    pts: &[Point],
    cand: &[usize],
    link: f64,
    zscale: f64,
) -> (Vec<usize>, ChainBoxes) {
    let group = crate::cluster::link_labels(
        cand.len(),
        |j| {
            let q = &pts[cand[j]];
            [q.x, q.y, q.z]
        },
        link,
        zscale,
    );
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

/// Ground the step may still name: not taken by another class.
fn plain_ground(pts: &[Point], ground: &[bool], class: &[Option<u8>], k: usize) -> bool {
    ground[k] && !pts[k].is_noise() && class[k].is_none_or(|c| c == 2)
}

/// The points with those that are not plain ground set aside (withheld), so that the shape of
/// a neighbourhood is the shape of the ground.
fn ground_only(pts: &[Point], ground: &[bool], class: &[Option<u8>]) -> Vec<Point> {
    pts.iter()
        .enumerate()
        .map(|(k, q)| {
            let mut q = *q;
            if !plain_ground(pts, ground, class, k) {
                q.withheld = true;
            }
            q
        })
        .collect()
}

/// Road points: ground that is smooth, flat and dark.
fn roads(pts: &[Point], ground: &[bool], class: &mut [Option<u8>], p: &RoadParams) {
    let limit = intensity_limit(pts, ground, class, p.max_intensity, p.relative_intensity);
    let wanted: Vec<bool> = (0..pts.len())
        .map(|k| plain_ground(pts, ground, class, k) && f64::from(pts[k].intensity) <= limit)
        .collect();
    let only = ground_only(pts, ground, class);
    let f = crate::features::compute(&only, &wanted, p.radius);
    for (k, f) in f.iter().enumerate() {
        if let Some(f) = f
            && f64::from(f.roughness) <= p.max_roughness
            && f64::from(f.slope) <= p.max_slope
        {
            class[k] = Some(ROAD);
        }
    }
}

/// The highest intensity that counts as dark: `absolute` when it is set, else `relative` times the
/// median intensity of the plain ground of the tile (0 when there is none).
fn intensity_limit(
    pts: &[Point],
    ground: &[bool],
    class: &[Option<u8>],
    absolute: f64,
    relative: f64,
) -> f64 {
    if absolute > 0.0 {
        return absolute;
    }
    let mut v: Vec<f64> = (0..pts.len())
        .filter(|&k| plain_ground(pts, ground, class, k))
        .map(|k| f64::from(pts[k].intensity))
        .collect();
    if v.is_empty() {
        return 0.0;
    }
    relative * crate::cluster::median(&mut v)
}

/// Water points: extended level regions of dark single-return ground.
fn water(pts: &[Point], ground: &[bool], class: &mut [Option<u8>], p: &WaterParams) {
    let limit = intensity_limit(pts, ground, class, p.max_intensity, p.relative_intensity);
    let wanted: Vec<bool> = (0..pts.len())
        .map(|k| {
            plain_ground(pts, ground, class, k)
                && pts[k].number_of_returns <= 1
                && f64::from(pts[k].intensity) <= limit
        })
        .collect();
    if !wanted.iter().any(|w| *w) {
        return;
    }
    let only = ground_only(pts, ground, class);
    let f = crate::features::compute(&only, &wanted, 2.0 * p.cell);
    let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (k, f) in f.iter().enumerate() {
        if let Some(f) = f
            && f64::from(f.slope) <= p.max_slope
        {
            cells
                .entry((
                    (pts[k].x / p.cell).floor() as i32,
                    (pts[k].y / p.cell).floor() as i32,
                ))
                .or_default()
                .push(k);
        }
    }
    // regions of 8-connected cells
    let mut seen: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let keys: Vec<(i32, i32)> = cells.keys().copied().collect();
    for start in keys {
        if !seen.insert(start) {
            continue;
        }
        let mut stack = vec![start];
        let mut members: Vec<(i32, i32)> = vec![start];
        while let Some((cx, cy)) = stack.pop() {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let n = (cx + dx, cy + dy);
                    if cells.contains_key(&n) && seen.insert(n) {
                        stack.push(n);
                        members.push(n);
                    }
                }
            }
        }
        if (members.len() as f64) * p.cell * p.cell < p.min_area {
            continue;
        }
        let zs: Vec<f64> = members
            .iter()
            .flat_map(|c| cells[c].iter().map(|&k| pts[k].z))
            .collect();
        let n = zs.len() as f64;
        let mean = zs.iter().sum::<f64>() / n;
        let sd = (zs.iter().map(|z| (z - mean).powi(2)).sum::<f64>() / n).sqrt();
        if sd <= p.max_deviation {
            for c in &members {
                for &k in &cells[c] {
                    class[k] = Some(WATER);
                }
            }
        }
    }
}

/// Mean distance between the points of a tile (square root of the area per point, the bounding
/// box over the points that are not noise); `None` with fewer than 100 points.
fn spacing(pts: &[Point]) -> Option<f64> {
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN, 0u64);
    for q in pts.iter().filter(|q| !q.is_noise()) {
        (x0, y0, x1, y1, n) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y), n + 1);
    }
    let area = (x1 - x0) * (y1 - y0);
    (n >= 100 && area > 0.0).then(|| (area / n as f64).sqrt())
}

/// The radii at which a point is looked at to see if it is part of a wire: the one of the tile
/// (or the one the parameters give) and, in a sparse survey (0.25 m between points or more), 1.5 and
/// 2 times it. A wire is a line at the radius that reaches its next point and has no crown beside it,
/// and that radius is not the same along the line.
fn wire_radii(pts: &[Point], w: &WireParams) -> Vec<f64> {
    let (base, _) = wire_scale(pts, w);
    let sparse = w.radius <= 0.0 && spacing(pts).is_some_and(|s| s >= WIRE_SPARSE_SPACING);
    if sparse {
        vec![base, (1.5 * base).min(4.5), (2.0 * base).min(4.5)]
    } else {
        vec![base]
    }
}

/// Radius of the neighbourhood and widest it may be across, for the wires of a tile: the
/// parameters, or (when 0) from the spacing of its points.
fn wire_scale(pts: &[Point], w: &WireParams) -> (f64, f64) {
    let radius = if w.radius > 0.0 {
        w.radius
    } else {
        spacing(pts).map_or(1.5, |s| (6.0 * s).clamp(0.6, 3.0))
    };
    let width = if w.max_width > 0.0 {
        w.max_width
    } else {
        0.35 * radius
    };
    (radius, width)
}

/// Which of the points `cand` belong to a chain that is long (bounding box diagonal at least
/// `min_len`) and straight (the square root of its first eigenvalue is at least `straightness`
/// times that of its second).
fn straight_chains(
    pts: &[Point],
    cand: &[usize],
    link: f64,
    min_len: f64,
    straightness: f64,
) -> Vec<bool> {
    let (group, boxes) = chains(pts, cand, link);
    let mut members: HashMap<usize, Vec<[f64; 3]>> = HashMap::new();
    for (&g, &i) in group.iter().zip(cand) {
        let (b, _) = boxes[&g];
        let diag = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
        if diag >= min_len {
            members
                .entry(g)
                .or_default()
                .push([pts[i].x, pts[i].y, pts[i].z]);
        }
    }
    let straight: HashMap<usize, bool> = members
        .iter()
        .map(|(g, m)| {
            let (_, cov, n) = crate::tls::covariance(m.iter().copied());
            let (vals, _) = crate::tls::eigen(cov);
            let ok = n >= 4 && vals[1].max(1e-9).sqrt() * straightness <= vals[0].max(0.0).sqrt();
            (*g, ok)
        })
        .collect();
    group
        .iter()
        .map(|g| straight.get(g).copied().unwrap_or(false))
        .collect()
}

/// Which of the points `set` are pieces of a conductor linked across gaps: short compact pieces
/// (0.4 to 4 m, points within `link` of each other, collapsed to voxels; thin, and level) that
/// stand in a row: three of them, each at most `link_gap` from the next, with the turn at the
/// middle one under 8° and the steepness of both steps within `max_slope`, make a chain; chains
/// that span at least twice `min_length` are wire. The pieces are what a survey with few passes
/// sees of a conductor: the scan lines cross it here and there, and between them there are
/// metres without a point. Three pieces in a row by chance, that far apart, hardly happens in
/// a crown.
fn linked_wires(pts: &[Point], set: &[usize], w: &WireParams, link: f64) -> Vec<bool> {
    let vs = w.voxel.max(0.05);
    let mut voxel_of: HashMap<(i32, i32, i32), usize> = HashMap::new();
    let mut centres: Vec<Point> = Vec::new();
    let in_voxel: Vec<usize> = set
        .iter()
        .map(|&k| {
            let q = &pts[k];
            let key = (
                (q.x / vs).floor() as i32,
                (q.y / vs).floor() as i32,
                (q.z / vs).floor() as i32,
            );
            *voxel_of.entry(key).or_insert_with(|| {
                centres.push(Point {
                    x: (f64::from(key.0) + 0.5) * vs,
                    y: (f64::from(key.1) + 0.5) * vs,
                    z: (f64::from(key.2) + 0.5) * vs,
                    ..Point::default()
                });
                centres.len() - 1
            })
        })
        .collect();
    let all: Vec<usize> = (0..centres.len()).collect();
    let (group, boxes) = chains(&centres, &all, link);
    let mut members: HashMap<usize, Vec<[f64; 3]>> = HashMap::new();
    for (&g, &i) in group.iter().zip(&all) {
        let (b, n) = boxes[&g];
        let diag = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
        if n >= 8 && (0.4..=4.0).contains(&diag) && b[5] - b[2] <= 1.0 {
            members
                .entry(g)
                .or_default()
                .push([centres[i].x, centres[i].y, centres[i].z]);
        }
    }
    // what stands around a piece: a conductor hangs alone, a piece of crown has crown around it
    let mut around: HashMap<Cell, Vec<usize>> = HashMap::new();
    for (i, c) in centres.iter().enumerate() {
        around
            .entry((
                (c.x / ALONE).floor() as i32,
                (c.y / ALONE).floor() as i32,
                (c.z / ALONE).floor() as i32,
            ))
            .or_default()
            .push(i);
    }
    let alone = |g: usize, c: &[f64; 3]| -> bool {
        let (cx, cy, cz) = (
            (c[0] / ALONE).floor() as i32,
            (c[1] / ALONE).floor() as i32,
            (c[2] / ALONE).floor() as i32,
        );
        let mut others = 0;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in around.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                        let q = &centres[i];
                        if group[i] != g
                            && (q.x - c[0]).powi(2) + (q.y - c[1]).powi(2) + (q.z - c[2]).powi(2)
                                <= ALONE * ALONE
                        {
                            others += 1;
                        }
                    }
                }
            }
        }
        others <= 2
    };
    // (group, centre) of the pieces that are thin and alone
    let mut pieces: Vec<(usize, [f64; 3])> = Vec::new();
    for (g, m) in &members {
        let (c, cov, _) = crate::tls::covariance(m.iter().copied());
        let (vals, _) = crate::tls::eigen(cov);
        if vals[1].max(0.0).sqrt() <= 0.3 && alone(*g, &c) {
            pieces.push((*g, c));
        }
    }
    pieces.sort_by_key(|p| p.0);
    let mut out = vec![false; set.len()];
    if pieces.len() < 3 {
        return out;
    }
    let centres_of: Vec<[f64; 3]> = pieces.iter().map(|p| p.1).collect();
    let mut dsu = crate::cluster::in_a_row(
        &centres_of,
        2.0 * link,
        w.link_gap,
        w.max_slope,
        crate::cluster::STRAIGHT,
    );
    let mut rows: HashMap<usize, (usize, [f64; 6])> = HashMap::new();
    for (j, p) in pieces.iter().enumerate() {
        let r = dsu.find(j);
        let (n, b) = rows.entry(r).or_insert((
            0,
            [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN],
        ));
        *n += 1;
        for k in 0..3 {
            b[k] = b[k].min(p.1[k]);
            b[k + 3] = b[k + 3].max(p.1[k]);
        }
    }
    let wire_group: std::collections::HashSet<usize> = pieces
        .iter()
        .enumerate()
        .filter(|(j, _)| {
            let (n, b) = rows[&dsu.find(*j)];
            let span =
                ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
            n >= 3 && span >= 2.0 * w.min_length
        })
        .map(|(_, p)| p.0)
        .collect();
    for (o, &v) in out.iter_mut().zip(&in_voxel) {
        *o = wire_group.contains(&group[v]);
    }
    out
}

/// Gives the extra classes to the points of a tile. `class` holds the class each step decided
/// (`None`: the point's own); `ground` says which points are ground.
pub fn apply(pts: &[Point], ground: &[bool], class: &mut [Option<u8>], e: &Extras) {
    if let Some(w) = &e.water {
        water(pts, ground, class, w);
    }
    if let Some(r) = &e.roads {
        roads(pts, ground, class, r);
    }
    if e.vegetation.is_none() && e.wires.is_none() && e.poles.is_none() {
        return;
    }
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
        // the points of a conductor come in clumps with gaps between them (the scan lines of a
        // survey): in a neighbourhood of points a clump is a blob, not a line. The points are
        // collapsed to the centres of the voxels they fill, so that the shape seen is the one of
        // the structure, whatever the density
        let (radius, _) = wire_scale(pts, w);
        let vs = w.voxel.max(0.05);
        let mut voxel_of: HashMap<(i32, i32, i32), usize> = HashMap::new();
        let mut centres: Vec<Point> = Vec::new();
        let in_voxel: Vec<usize> = set
            .iter()
            .map(|&k| {
                let q = &pts[k];
                let key = (
                    (q.x / vs).floor() as i32,
                    (q.y / vs).floor() as i32,
                    (q.z / vs).floor() as i32,
                );
                *voxel_of.entry(key).or_insert_with(|| {
                    centres.push(Point {
                        x: (f64::from(key.0) + 0.5) * vs,
                        y: (f64::from(key.1) + 0.5) * vs,
                        z: (f64::from(key.2) + 0.5) * vs,
                        ..Point::default()
                    });
                    centres.len() - 1
                })
            })
            .collect();
        let all: Vec<usize> = (0..centres.len()).collect();
        // a voxel is a candidate if it looks like a wire at some radius
        let mut is_cand = vec![false; centres.len()];
        let mut reach = radius;
        for r in wire_radii(pts, w) {
            reach = reach.max(r);
            let width_limit = if w.max_width > 0.0 { w.max_width } else { 0.35 * r };
            let crowd = neighbour_counts(&centres, &all, 2.0 * r);
            for ((&v, f), n) in all.iter().zip(line_features(&centres, &all, r)).zip(crowd) {
                if let Some((lin, dz, width)) = f
                    && lin >= w.linearity
                    && dz <= w.max_slope
                    && width <= width_limit
                    && n <= w.max_neighbours
                {
                    is_cand[v] = true;
                }
            }
        }
        let cand: Vec<usize> = all.iter().copied().filter(|&v| is_cand[v]).collect();
        let keep = straight_chains(
            &centres,
            &cand,
            reach,
            w.min_length,
            w.min_straightness,
        );
        let mut wire = vec![false; centres.len()];
        for (&v, k) in cand.iter().zip(keep) {
            wire[v] = k;
        }
        for (&k, &v) in set.iter().zip(&in_voxel) {
            if wire[v] {
                class[k] = Some(WIRE);
            }
        }
        if w.link_gap > 0.0 {
            let rest: Vec<usize> = set
                .iter()
                .copied()
                .filter(|&k| class[k] != Some(WIRE) && height(k) >= LINKED_MIN_HEIGHT)
                .collect();
            for (&k, hit) in rest.iter().zip(linked_wires(pts, &rest, w, radius)) {
                if hit {
                    class[k] = Some(WIRE);
                }
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
        let mut under: HashMap<(i32, i32), Vec<[f64; 4]>> = HashMap::new();
        for &k in &above {
            // conductors reach the tower: they are not crown around it
            if height(k) >= 3.0 && class[k] != Some(WIRE) {
                under
                    .entry((
                        (pts[k].x / 3.0).floor() as i32,
                        (pts[k].y / 3.0).floor() as i32,
                    ))
                    .or_default()
                    .push([pts[k].x, pts[k].y, pts[k].z, height(k)]);
            }
        }
        // how many points a structure may have, and how many strays a tower tolerates, grow with
        // the density of the survey: 12 per metre of height is the count of a tower at 9 points/m²
        let density = spacing(pts).map_or(1.0, |s| (POLE_REFERENCE_SPACING / s).powi(2).max(1.0));
        let (group, boxes) = chains_z(pts, &set, p.radius, POLE_Z_SCALE);
        let mut members: HashMap<usize, Vec<f64>> = HashMap::new();
        for (&k, g) in set.iter().zip(&group) {
            if height(k) >= 3.0 {
                members.entry(*g).or_default().push(height(k));
            }
        }
        // points high above the ground within 3 m of the chain on the plan, those of the chain
        // apart: a tower in a corridor has none, a tree at the edge of a stand has. "High" is 3 m
        // or a third of the height of the structure, whichever is more: a crop of low trees at the
        // foot of a tower is not crown around it
        let crowd = |b: &[f64; 6], own: &[f64], tolerance: f64| -> bool {
            let high = (0.3 * (b[5] - b[2])).max(3.0);
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
                    q[3] >= high
                        && q[0] > b[0] - 3.0
                        && q[0] < b[3] + 3.0
                        && q[1] > b[1] - 3.0
                        && q[1] < b[4] + 3.0
                })
                .count();
            let own = own.iter().filter(|h| **h >= high).count() as f64;
            near as f64 > own * tolerance + 2.0 * density
        };
        // a trunk goes up into its crown: around its top, from 2 m under it, there are many more
        // points than its own. A lamp post, a pole, has a lamp and little else up there.
        let capped = |b: &[f64; 6], own: usize| -> bool {
            let (cx0, cx1) = (
                ((b[0] - 2.0) / 3.0).floor() as i32,
                ((b[3] + 2.0) / 3.0).floor() as i32,
            );
            let (cy0, cy1) = (
                ((b[1] - 2.0) / 3.0).floor() as i32,
                ((b[4] + 2.0) / 3.0).floor() as i32,
            );
            let near: usize = (cx0..=cx1)
                .flat_map(|cx| (cy0..=cy1).map(move |cy| (cx, cy)))
                .filter_map(|c| under.get(&c))
                .flatten()
                .filter(|q| {
                    q[2] >= b[5] - 2.0
                        && q[0] > b[0] - 2.0
                        && q[0] < b[3] + 2.0
                        && q[1] > b[1] - 2.0
                        && q[1] < b[4] + 2.0
                })
                .count();
            near as f64 > own as f64 * CAP_TOLERANCE + 2.0 * density
        };
        let crowded = |g: &usize| -> bool {
            let (b, _) = boxes[g];
            crowd(&b, members.get(g).map_or(&[][..], |v| v.as_slice()), CROWD_TOLERANCE)
        };
        // a structure stands on the ground, is narrow for its height and is sparse; a crown is
        // wide and dense, a forest is one chain far wider than any tower
        // the plan of each chain, to measure its width without what is attached to it: the
        // conductors that reach a tower, or a branch that touches a pole, stretch its box
        let mut plan: HashMap<usize, (Vec<f64>, Vec<f64>)> = HashMap::new();
        for (&k, g) in set.iter().zip(&group) {
            let e = plan.entry(*g).or_default();
            e.0.push(pts[k].x);
            e.1.push(pts[k].y);
        }
        let core_width = |g: &usize| -> f64 {
            plan.get(g).map_or(0.0, |(x, y)| {
                crate::cluster::core_spread(x).max(crate::cluster::core_spread(y))
            })
        };
        // the points of a chain inside its core: those of a tower, without the conductors attached
        let core_points = |g: &usize, width: f64| -> usize {
            let Some((x, y)) = plan.get(g) else { return 0 };
            let mid = |v: &[f64]| {
                let mut v = v.to_vec();
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            };
            let (mx, my) = (mid(x), mid(y));
            let half = width / 2.0 + 0.5;
            x.iter()
                .zip(y)
                .filter(|(a, b)| (**a - mx).abs() <= half && (**b - my).abs() <= half)
                .count()
        };
        let is_pole = |g: &usize| -> bool {
            let (b, _) = boxes[g];
            let width = core_width(g);
            let n = core_points(g, width);
            let extent = b[5] - b[2];
            n >= 5
                && width <= p.max_width
                && extent >= p.min_extent
                && width <= 0.35 * extent
                && (n as f64)
                    <= if extent >= TOWER_HEIGHT {
                        TOWER_POINTS_PER_METRE * extent * density
                    } else {
                        12.0 * extent
                    }
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
        if p.thin_extent > 0.0 {
            // a thin pole is a vertical line: the same look as a wire, turned upright
            let set: Vec<usize> = above
                .iter()
                .copied()
                .filter(|&k| height(k) >= 1.0 && class[k].is_none_or(|c| c != POLE && c != WIRE))
                .collect();
            let w = WireParams::default();
            let (radius, max_width) = wire_scale(pts, &w);
            let vs = w.voxel.max(0.05);
            let mut voxel_of: HashMap<(i32, i32, i32), usize> = HashMap::new();
            let mut centres: Vec<Point> = Vec::new();
            let in_voxel: Vec<usize> = set
                .iter()
                .map(|&k| {
                    let q = &pts[k];
                    let key = (
                        (q.x / vs).floor() as i32,
                        (q.y / vs).floor() as i32,
                        (q.z / vs).floor() as i32,
                    );
                    *voxel_of.entry(key).or_insert_with(|| {
                        centres.push(Point {
                            x: (f64::from(key.0) + 0.5) * vs,
                            y: (f64::from(key.1) + 0.5) * vs,
                            z: (f64::from(key.2) + 0.5) * vs,
                            ..Point::default()
                        });
                        centres.len() - 1
                    })
                })
                .collect();
            let all: Vec<usize> = (0..centres.len()).collect();
            let cand: Vec<usize> = all
                .iter()
                .zip(line_features(&centres, &all, radius))
                .filter_map(|(&v, f)| {
                    f.filter(|&(lin, dz, width)| lin >= 0.7 && dz >= 0.8 && width <= max_width)
                        .map(|_| v)
                })
                .collect();
            // trunks show as vertical lines only when the survey is dense (above ~180 points/m²)
            let trunks_resolved = density >= TRUNK_DENSITY;
            let (group, boxes) = chains(&centres, &cand, radius);
            // the heights of the points of each chain, to tell a pole from a trunk: a trunk
            // has its crown around it
            let group_of: HashMap<usize, usize> =
                cand.iter().copied().zip(group.iter().copied()).collect();
            let mut own: HashMap<usize, Vec<f64>> = HashMap::new();
            for (&k, v) in set.iter().zip(&in_voxel) {
                if let Some(g) = group_of.get(v) {
                    own.entry(*g).or_default().push(height(k));
                }
            }
            let good: std::collections::HashSet<usize> = boxes
                .iter()
                .filter(|(g, (b, n))| {
                    let plan = (b[3] - b[0]).hypot(b[4] - b[1]);
                    *n >= 4
                        && b[5] - b[2] >= p.thin_extent
                        && plan <= 0.5 * (b[5] - b[2]).max(1.0)
                        && !(trunks_resolved && capped(b, own.get(g).map_or(0, Vec::len)))
                })
                .map(|(g, _)| *g)
                .collect();
            let mut keep = vec![false; centres.len()];
            for (&v, g) in cand.iter().zip(&group) {
                keep[v] = good.contains(g);
            }
            for (&k, &v) in set.iter().zip(&in_voxel) {
                if keep[v] {
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
            ..Extras::default()
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

    fn flat_ground(w: i32, h: i32, step: f64) -> Vec<Point> {
        let mut pts = Vec::new();
        for i in 0..(f64::from(w) / step) as i32 {
            for j in 0..(f64::from(h) / step) as i32 {
                pts.push(pt(f64::from(i) * step, f64::from(j) * step, 0.0));
            }
        }
        pts
    }

    /// A survey with few passes sees a conductor in pieces a metre long with metres of nothing
    /// between: three or more in a row, far from anything else, are a wire; two are not, and
    /// pieces with crown around them are not.
    /// A sparse survey looks at a wire from three radii, a dense one from one.
    #[test]
    fn wires_are_looked_at_from_more_radii_in_a_sparse_survey() {
        let w = WireParams::default();
        let dense = flat_ground(60, 60, 0.1);
        let sparse = flat_ground(60, 60, 0.4);
        assert_eq!(wire_radii(&dense, &w).len(), 1);
        let r = wire_radii(&sparse, &w);
        assert_eq!(r.len(), 3);
        assert!(r[0] < r[1] && r[1] < r[2] && r[2] <= 4.5, "{r:?}");
        let fixed = WireParams {
            radius: 2.0,
            ..w
        };
        assert_eq!(wire_radii(&sparse, &fixed), vec![2.0]);
    }

    #[test]
    fn pieces_of_conductor_in_a_row_are_wire() {
        let mut pts = flat_ground(120, 40, 0.5);
        let n_ground = pts.len();
        let piece = |pts: &mut Vec<Point>, x: f64, y: f64, z: f64| {
            for i in 0..20 {
                pts.push(pt(x + f64::from(i) * 0.05, y, z));
            }
        };
        // six pieces 10 m apart at 14 m, and another two apart from them
        let wire_from = pts.len();
        for k in 0..6 {
            piece(&mut pts, 5.0 + 10.0 * f64::from(k), 25.0, 14.0);
        }
        let wire_to = pts.len();
        piece(&mut pts, 5.0, 5.0, 14.0);
        piece(&mut pts, 15.0, 5.0, 14.0);
        let pair_to = pts.len();
        // three pieces in a row, each in the middle of a blob of crown points
        for k in 0..3 {
            let x = 70.0 + 10.0 * f64::from(k);
            piece(&mut pts, x, 25.0, 14.0);
            for a in 0..6 {
                for b in 0..6 {
                    for c in 0..6 {
                        pts.push(pt(
                            x - 0.6 + f64::from(a) * 0.25,
                            24.0 + f64::from(b) * 0.25,
                            13.0 + f64::from(c) * 0.25,
                        ));
                    }
                }
            }
        }
        let mut ground = vec![false; pts.len()];
        ground[..n_ground].fill(true);
        let mut class = vec![None; pts.len()];
        let e = Extras {
            wires: Some(WireParams::default()),
            ..Extras::default()
        };
        apply(&pts, &ground, &mut class, &e);
        let wires = |a: usize, b: usize| class[a..b].iter().filter(|c| **c == Some(WIRE)).count();
        assert_eq!(wires(wire_from, wire_to), wire_to - wire_from);
        assert_eq!(wires(wire_to, pair_to), 0, "two pieces are not a line");
        assert_eq!(wires(pair_to, pts.len()), 0, "pieces inside crown are not wire");
    }

    /// At the density of a survey a lattice tower has thousands of points, a loose column with
    /// metres between its rows; low crops at its foot are not crown around it.
    #[test]
    fn a_dense_survey_tower_among_low_crops_is_a_pole() {
        let mut pts = flat_ground(40, 40, 0.1);
        let n_ground = pts.len();
        let tower_from = pts.len();
        for k in 0..13 {
            let z = 3.0 + 3.0 * f64::from(k);
            for i in 0..50 {
                let d = -1.5 + f64::from(i) * 0.06;
                pts.push(pt(20.0 + d, 20.0, z));
                pts.push(pt(20.0, 20.0 + d, z));
            }
        }
        let tower_to = pts.len();
        // crops 3.5 m high in a ring 4 to 6 m from the tower
        for i in 0..400 {
            let a = f64::from(i) * 0.37;
            let r = 4.0 + f64::from(i % 20) * 0.1;
            pts.push(pt(20.0 + r * a.cos(), 20.0 + r * a.sin(), 3.5));
        }
        let mut ground = vec![false; pts.len()];
        ground[..n_ground].fill(true);
        let mut class = vec![None; pts.len()];
        let e = Extras {
            poles: Some(PoleParams::default()),
            ..Extras::default()
        };
        apply(&pts, &ground, &mut class, &e);
        let poles = class[tower_from..tower_to]
            .iter()
            .filter(|c| **c == Some(POLE))
            .count();
        assert!(poles >= (tower_to - tower_from) * 9 / 10, "{poles}");
        assert!(class[tower_to..].iter().all(|c| *c != Some(POLE)));
    }

    /// A flat dark strip of ground is a road; bright ground beside it and rough ground are not.
    #[test]
    fn dark_smooth_ground_is_road() {
        let mut pts = Vec::new();
        let mut rough_seed = 7u64;
        for i in 0..40 {
            for j in 0..40 {
                let mut q = pt(f64::from(i) * 0.5, f64::from(j) * 0.5, 0.0);
                if i < 20 {
                    // road: dark, flat
                    q.intensity = 5000;
                } else if j < 20 {
                    // bright soil, flat
                    q.intensity = 30000;
                } else {
                    // dark but rough ground
                    q.intensity = 5000;
                    rough_seed = rough_seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    q.z = ((rough_seed >> 40) as f64 / f64::from(1u32 << 24) - 0.5) * 0.4;
                }
                pts.push(q);
            }
        }
        let ground = vec![true; pts.len()];
        let mut class = vec![None; pts.len()];
        let e = Extras {
            roads: Some(RoadParams {
                max_intensity: 10_000.0,
                ..RoadParams::default()
            }),
            ..Extras::default()
        };
        apply(&pts, &ground, &mut class, &e);
        let road = |i: usize, j: usize| class[i * 40 + j] == Some(ROAD);
        assert!(road(5, 5) && road(10, 30), "the dark flat strip is road");
        assert!(!road(30, 5), "bright soil is not");
        assert!(!road(30, 30), "rough ground is not");
    }

    /// Without a set limit, dark is relative to the ground of the tile: the same scene is found
    /// with intensities of 16 bits and with intensities of 0 to 255.
    #[test]
    fn the_road_limit_follows_the_scale_of_the_sensor() {
        for scale in [1.0, 65535.0 / 255.0] {
            let mut pts = Vec::new();
            for i in 0..40 {
                for j in 0..40 {
                    let mut q = pt(f64::from(i) * 0.5, f64::from(j) * 0.5, 0.0);
                    // a dark strip of a fifth of the ground, bright soil elsewhere
                    q.intensity = if i < 8 { 20.0 * scale } else { 120.0 * scale } as u16;
                    pts.push(q);
                }
            }
            let ground = vec![true; pts.len()];
            let mut class = vec![None; pts.len()];
            let e = Extras {
                roads: Some(RoadParams::default()),
                ..Extras::default()
            };
            apply(&pts, &ground, &mut class, &e);
            assert!(class[4 * 40 + 20] == Some(ROAD), "scale {scale}");
            assert!(class[30 * 40 + 20] != Some(ROAD), "scale {scale}");
        }
    }

    /// A big level sheet of dark single returns is water; a small one is not.
    #[test]
    fn a_dark_level_sheet_is_water() {
        let mut pts = Vec::new();
        for i in 0..60 {
            for j in 0..60 {
                let mut q = pt(f64::from(i) * 0.5, f64::from(j) * 0.5, 10.0);
                q.intensity = 800;
                pts.push(q);
            }
        }
        // a small dark patch far away
        let lake = pts.len();
        for i in 0..6 {
            for j in 0..6 {
                let mut q = pt(200.0 + f64::from(i) * 0.5, f64::from(j) * 0.5, 10.0);
                q.intensity = 800;
                pts.push(q);
            }
        }
        let ground = vec![true; pts.len()];
        let mut class = vec![None; pts.len()];
        let e = Extras {
            water: Some(WaterParams {
                max_intensity: 6_000.0,
                ..WaterParams::default()
            }),
            ..Extras::default()
        };
        apply(&pts, &ground, &mut class, &e);
        assert_eq!(class[30 * 60 + 30], Some(WATER));
        assert_eq!(class[lake + 10], None);
    }
}
