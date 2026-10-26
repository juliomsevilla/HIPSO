//! Power-line corridors: the conductors rebuilt from the points classified as wires, the poles
//! from those classified as poles, and the vegetation that comes too close or could reach the
//! line if it fell.
//!
//! 1. The wire points that touch are joined into one conductor each; the conductor's path is the
//!    median height and offset of its points along its main direction, every `step` metres, and
//!    gaps (a few metres where the laser did not hit the wire) are filled with a parabola fitted
//!    to the points next to them, which follows the sag instead of cutting across it.
//! 2. Poles are the clusters of pole points; the spans of a conductor are the stretches between the
//!    poles that stand beside it.
//! 3. The vegetation is read once more; every point within `half_width` of a conductor is measured
//!    against it, and the points are grouped in cells of `cell` metres, each remembering its highest
//!    point and its closest approach. A cell is a hazard when it comes closer than `min_distance`
//!    ("invasión"), or when its height plus a margin reaches the conductor's horizontal position
//!    ("caída").
//!
//! What it cannot do: the wire points of airborne surveys are few and sometimes missing, so a
//! conductor with long gaps is reported in pieces and its clearance there is not measured; and the
//! safety distance depends on the voltage, which is the user's to set.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::filter::ClassSel;
use crate::grid::Grid;
use crate::las::Reader;
use crate::paths::{self, Formats};
use crate::vector::{ColKind, Table, Val, prj_for_epsg, write_points, write_polylines};
use crate::{Error, Result, check_cancel};

pub const WIRE: u8 = 14;
pub const POLE: u8 = 15;

#[derive(Clone, Debug, PartialEq)]
pub struct CorridorParams {
    /// Class of the wire points (ASPRS 14, wire conductor); another only for clouds that use their own codes.
    pub wire_class: u8,
    /// Class of the pole points (ASPRS 15, transmission tower).
    pub pole_class: u8,
    /// Wire points closer than this are the same conductor.
    pub link: f64,
    /// Spacing of the vertices of a conductor's path.
    pub step: f64,
    /// A conductor made of fewer points is dropped.
    pub min_points: usize,
    /// A conductor shorter than this is dropped: a span is tens of metres, while the short
    /// "lines" that a classifier finds in tree crowns are a few.
    pub min_length: f64,
    /// A pole lower than this, or wider than `pole_max_width`, is not one.
    pub min_pole_height: f64,
    pub pole_max_width: f64,
    /// Gaps longer than this are not filled: the conductor is cut there.
    pub gap_max: f64,
    /// Vegetation farther than this from every conductor is not looked at.
    pub half_width: f64,
    /// Closer than this to a conductor (in space) is an invasion.
    pub min_distance: f64,
    /// A tree whose height plus this margin reaches the conductor's horizontal position could hit
    /// it when falling.
    pub fall_margin: f64,
    /// Side of the cells the vegetation is grouped in.
    pub cell: f64,
    /// Vegetation below this height above ground is ignored.
    pub min_height: f64,
    /// Pole points closer than this are the same pole.
    pub pole_link: f64,
    /// A pole farther than this from a conductor is not one of its supports.
    pub pole_reach: f64,
    pub vegetation: ClassSel,
}

impl Default for CorridorParams {
    fn default() -> Self {
        Self {
            wire_class: WIRE,
            pole_class: POLE,
            link: 2.5,
            step: 2.0,
            min_points: 30,
            min_length: 25.0,
            min_pole_height: 6.0,
            pole_max_width: 8.0,
            gap_max: 30.0,
            half_width: 30.0,
            min_distance: 3.0,
            fall_margin: 1.0,
            cell: 3.0,
            min_height: 2.0,
            pole_link: 1.5,
            pole_reach: 8.0,
            vegetation: ClassSel::parse("3,4,5").unwrap_or_else(|_| ClassSel::all()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CorridorJob {
    pub inputs: Vec<PathBuf>,
    /// Ground model, to know how tall the vegetation is.
    pub dtm: PathBuf,
    /// Base path of the results.
    pub output: PathBuf,
    pub params: CorridorParams,
    pub formats: Formats,
}

pub struct Outputs {
    pub conductors: Option<PathBuf>,
    pub poles: Option<PathBuf>,
    pub hazards: Option<PathBuf>,
    pub spans: PathBuf,
    pub hazards_table: PathBuf,
}

impl CorridorJob {
    pub fn outputs(&self) -> Outputs {
        let f = self.formats.fixed();
        let named = |tag: &str, ext: &str| {
            let b = paths::swap_ext(&self.output, ext);
            let stem = b
                .file_stem()
                .map_or(String::new(), |s| s.to_string_lossy().into_owned());
            b.with_file_name(format!("{stem}_{tag}.{ext}"))
        };
        Outputs {
            conductors: f.vector.then(|| named("conductores", "shp")),
            poles: f.vector.then(|| named("postes", "shp")),
            hazards: f.vector.then(|| named("riesgo", "shp")),
            spans: named("vanos", "csv"),
            hazards_table: named("riesgo", "csv"),
        }
    }
}

// ----------------------------------------------------------------------------- clustering

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        let p = parent[x as usize];
        parent[x as usize] = parent[p as usize];
        x = parent[x as usize];
    }
    x
}

/// Groups of points closer than `link` (chained), in 3D when `three_d`, else in plan.
fn cluster(pts: &[[f64; 3]], link: f64, three_d: bool) -> Vec<Vec<usize>> {
    let key = |p: &[f64; 3]| {
        (
            (p[0] / link).floor() as i64,
            (p[1] / link).floor() as i64,
            if three_d {
                (p[2] / link).floor() as i64
            } else {
                0
            },
        )
    };
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        grid.entry(key(p)).or_default().push(i as u32);
    }
    let mut parent: Vec<u32> = (0..pts.len() as u32).collect();
    let dz = i64::from(three_d);
    for (i, p) in pts.iter().enumerate() {
        let (kx, ky, kz) = key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dzz in -dz..=dz {
                    for &j in grid
                        .get(&(kx + dx, ky + dy, kz + dzz))
                        .into_iter()
                        .flatten()
                    {
                        let q = &pts[j as usize];
                        let d2 = (p[0] - q[0]).powi(2)
                            + (p[1] - q[1]).powi(2)
                            + if three_d { (p[2] - q[2]).powi(2) } else { 0.0 };
                        if d2 <= link * link {
                            let (a, b) = (find(&mut parent, i as u32), find(&mut parent, j));
                            if a != b {
                                parent[a.max(b) as usize] = a.min(b);
                            }
                        }
                    }
                }
            }
        }
    }
    let mut groups: HashMap<u32, Vec<usize>> = HashMap::new();
    for i in 0..pts.len() {
        let r = find(&mut parent, i as u32);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    out.sort_by_key(|g| g[0]);
    out
}

fn median(v: &mut [f64]) -> f64 {
    let m = v.len() / 2;
    v.select_nth_unstable_by(m, |a, b| a.total_cmp(b));
    v[m]
}

/// Least-squares parabola `z = a + b t + c t²` through `(t, z)`; `None` when it is degenerate.
fn parabola(samples: &[(f64, f64)]) -> Option<[f64; 3]> {
    let n = samples.len() as f64;
    let (mut s1, mut s2, mut s3, mut s4) = (0.0, 0.0, 0.0, 0.0);
    let (mut z0, mut z1, mut z2) = (0.0, 0.0, 0.0);
    for &(t, z) in samples {
        s1 += t;
        s2 += t * t;
        s3 += t * t * t;
        s4 += t * t * t * t;
        z0 += z;
        z1 += t * z;
        z2 += t * t * z;
    }
    // normal equations by Cramer's rule
    let m = [[n, s1, s2], [s1, s2, s3], [s2, s3, s4]];
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    if d.abs() < 1e-12 {
        return None;
    }
    let rhs = [z0, z1, z2];
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut mk = m;
        for r in 0..3 {
            mk[r][k] = rhs[r];
        }
        *o = det(&mk) / d;
    }
    Some(out)
}

/// Joins groups of wire points that are pieces of one conductor split by a gap in the laser
/// returns: they point the same way, the gap is shorter than `gap_max` and the end of one lies on
/// the line of the other.
fn join_pieces(pts: &[[f64; 3]], groups: Vec<Vec<usize>>, p: &CorridorParams) -> Vec<Vec<usize>> {
    if groups.len() < 2 || groups.len() > 5000 {
        return groups;
    }
    struct Piece {
        end: [[f64; 3]; 2],
        axis: [f64; 2],
    }
    let pieces: Vec<Piece> = groups
        .iter()
        .map(|g| {
            let n = g.len() as f64;
            let (mx, my) = (
                g.iter().map(|&i| pts[i][0]).sum::<f64>() / n,
                g.iter().map(|&i| pts[i][1]).sum::<f64>() / n,
            );
            let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
            for &i in g {
                let (dx, dy) = (pts[i][0] - mx, pts[i][1] - my);
                sxx += dx * dx;
                syy += dy * dy;
                sxy += dx * dy;
            }
            let a = 0.5 * (2.0 * sxy).atan2(sxx - syy);
            let (ux, uy) = (a.cos(), a.sin());
            let s = |i: usize| (pts[i][0] - mx) * ux + (pts[i][1] - my) * uy;
            let lo = g
                .iter()
                .copied()
                .min_by(|&a, &b| s(a).total_cmp(&s(b)))
                .unwrap_or(g[0]);
            let hi = g
                .iter()
                .copied()
                .max_by(|&a, &b| s(a).total_cmp(&s(b)))
                .unwrap_or(g[0]);
            Piece {
                end: [pts[lo], pts[hi]],
                axis: [ux, uy],
            }
        })
        .collect();
    let mut parent: Vec<u32> = (0..groups.len() as u32).collect();
    for a in 0..groups.len() {
        for b in a + 1..groups.len() {
            let (pa, pb) = (&pieces[a], &pieces[b]);
            // the pair of ends that face each other
            let mut best: Option<(f64, [f64; 3], [f64; 3])> = None;
            for ea in &pa.end {
                for eb in &pb.end {
                    let d = (ea[0] - eb[0]).hypot(ea[1] - eb[1]);
                    if best.is_none_or(|x| d < x.0) {
                        best = Some((d, *ea, *eb));
                    }
                }
            }
            let Some((d, ea, eb)) = best else { continue };
            if d > p.gap_max || d <= p.link {
                continue;
            }
            // same direction (either way), and the other end lies on the line of this one
            let cos = (pa.axis[0] * pb.axis[0] + pa.axis[1] * pb.axis[1]).abs();
            let v = [(eb[0] - ea[0]) / d, (eb[1] - ea[1]) / d];
            let along_a = (v[0] * pa.axis[0] + v[1] * pa.axis[1]).abs();
            let along_b = (v[0] * pb.axis[0] + v[1] * pb.axis[1]).abs();
            let slope_ok = (eb[2] - ea[2]).abs() <= 0.3 * d + 1.0;
            if cos > 0.985 && along_a > 0.97 && along_b > 0.97 && slope_ok {
                let (ra, rb) = (find(&mut parent, a as u32), find(&mut parent, b as u32));
                if ra != rb {
                    parent[ra.max(rb) as usize] = ra.min(rb);
                }
            }
        }
    }
    let mut merged: HashMap<u32, Vec<usize>> = HashMap::new();
    for (k, g) in groups.into_iter().enumerate() {
        let r = find(&mut parent, k as u32);
        merged.entry(r).or_default().extend(g);
    }
    let mut out: Vec<Vec<usize>> = merged.into_values().collect();
    out.sort_by_key(|g| g[0]);
    out
}

/// One conductor: a path of vertices (x, y, z) every `step` metres, in order.
#[derive(Clone, Debug)]
pub struct Conductor {
    pub verts: Vec<[f64; 3]>,
    /// Distance along the path of each vertex.
    pub cum: Vec<f64>,
    pub points: usize,
    /// Stretches that were filled.
    pub gaps: usize,
    /// Parts of the cluster dropped for being cut by a gap too long to fill.
    pub breaks: usize,
}

impl Conductor {
    fn from_verts(verts: Vec<[f64; 3]>, points: usize, gaps: usize, breaks: usize) -> Self {
        let mut cum = vec![0.0];
        for w in verts.windows(2) {
            let d = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            cum.push(cum[cum.len() - 1] + d);
        }
        Self {
            verts,
            cum,
            points,
            gaps,
            breaks,
        }
    }

    pub fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// The point of the path closest to `(x, y)` in plan: the distance along the path and the plan
    /// distance to it.
    pub fn locate(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        for k in 1..self.verts.len() {
            let (d, t) = segment(&self.verts[k - 1], &self.verts[k], x, y);
            if best.is_none_or(|b| d < b.1) {
                best = Some((self.cum[k - 1] + t * (self.cum[k] - self.cum[k - 1]), d));
            }
        }
        best
    }
}

/// The conductor paths of a cluster of wire points (more than one when a long gap cuts it).
fn fit(points: &[[f64; 3]], p: &CorridorParams) -> Vec<Conductor> {
    let n = points.len() as f64;
    let (mx, my) = (
        points.iter().map(|q| q[0]).sum::<f64>() / n,
        points.iter().map(|q| q[1]).sum::<f64>() / n,
    );
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for q in points {
        let (dx, dy) = (q[0] - mx, q[1] - my);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (ux, uy) = (angle.cos(), angle.sin());
    let along = |q: &[f64; 3]| (q[0] - mx) * ux + (q[1] - my) * uy;
    let side = |q: &[f64; 3]| -(q[0] - mx) * uy + (q[1] - my) * ux;
    let smin = points.iter().map(&along).fold(f64::INFINITY, f64::min);
    let smax = points.iter().map(&along).fold(f64::NEG_INFINITY, f64::max);
    let nbins = ((smax - smin) / p.step).floor() as usize + 1;
    let mut bins: Vec<(Vec<f64>, Vec<f64>)> = vec![(Vec::new(), Vec::new()); nbins];
    for q in points {
        let b = (((along(q) - smin) / p.step).floor() as usize).min(nbins - 1);
        bins[b].0.push(side(q));
        bins[b].1.push(q[2]);
    }
    // the vertex of each bin that has points: (s, side, z)
    let mut vs: Vec<Option<(f64, f64, f64)>> = bins
        .iter_mut()
        .enumerate()
        .map(|(b, (d, z))| {
            (!z.is_empty()).then(|| (smin + (b as f64 + 0.5) * p.step, median(d), median(z)))
        })
        .collect();
    // fill the empty bins between full ones with a parabola of the neighbours, unless the gap is
    // too long; pieces are the runs that remain
    let mut pieces: Vec<Vec<(f64, f64, f64)>> = Vec::new();
    let mut cur: Vec<(f64, f64, f64)> = Vec::new();
    let mut gaps = 0usize;
    let mut breaks = 0usize;
    let mut b = 0;
    while b < nbins {
        if let Some(v) = vs[b] {
            cur.push(v);
            b += 1;
            continue;
        }
        let start = b;
        while b < nbins && vs[b].is_none() {
            b += 1;
        }
        let len_m = (b - start) as f64 * p.step;
        if cur.is_empty() || b >= nbins {
            // gaps at the ends are just shorter conductors
            continue;
        }
        if len_m > p.gap_max {
            pieces.push(std::mem::take(&mut cur));
            breaks += 1;
            continue;
        }
        // parabola through up to four vertices on each side
        let before: Vec<(f64, f64, f64)> = cur.iter().rev().take(4).copied().collect();
        let after: Vec<(f64, f64, f64)> = (b..nbins.min(b + 4)).filter_map(|k| vs[k]).collect();
        let mid = smin + (start as f64 + (b - start) as f64 / 2.0) * p.step;
        let samples: Vec<(f64, f64)> = before
            .iter()
            .chain(&after)
            .map(|v| (v.0 - mid, v.2))
            .collect();
        let fitted = (samples.len() >= 4).then(|| parabola(&samples)).flatten();
        let (first, last) = (before[0], after[0]);
        for (k, slot) in vs.iter_mut().enumerate().take(b).skip(start) {
            let s = smin + (k as f64 + 0.5) * p.step;
            let f = (s - first.0) / (last.0 - first.0);
            let side_v = first.1 + f * (last.1 - first.1);
            let z = match fitted {
                Some(c) => {
                    let t = s - mid;
                    c[0] + c[1] * t + c[2] * t * t
                }
                None => first.2 + f * (last.2 - first.2),
            };
            *slot = Some((s, side_v, z));
            cur.push((s, side_v, z));
        }
        gaps += 1;
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces
        .into_iter()
        .filter(|v| v.len() >= 2)
        .map(|v| {
            let verts = v
                .iter()
                .map(|&(s, d, z)| [mx + s * ux - d * uy, my + s * uy + d * ux, z])
                .collect();
            Conductor::from_verts(verts, points.len(), gaps, breaks)
        })
        .collect()
}

// ----------------------------------------------------------------------------- the scene

/// A pole.
#[derive(Clone, Debug)]
pub struct Pole {
    pub x: f64,
    pub y: f64,
    pub base_z: f64,
    pub top_z: f64,
    pub points: usize,
    /// The larger side of its plan extent.
    pub width: f64,
}

/// The stretch of a conductor between two poles.
#[derive(Clone, Debug)]
pub struct Span {
    pub conductor: usize,
    pub index: usize,
    pub pole_a: Option<usize>,
    pub pole_b: Option<usize>,
    /// Distance along the conductor where it begins and ends.
    pub from: f64,
    pub to: f64,
    pub sag: f64,
}

#[derive(Default)]
struct SpanStats {
    invasion: u32,
    fall: u32,
    min_d3: f64,
    tallest: f64,
}

/// A group of vegetation points.
#[derive(Clone, Copy)]
struct Cell {
    max_h: f64,
    top: [f64; 3],
    d_h: f64,
    wire_z: f64,
    d3_min: f64,
    conductor: usize,
    along: f64,
}

/// One hazard found.
#[derive(Clone, Debug)]
pub struct Hazard {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub height: f64,
    pub horizontal: f64,
    pub distance: f64,
    pub clearance: f64,
    pub invasion: bool,
    pub fall: bool,
    pub conductor: usize,
    pub span: usize,
}

/// What a run found.
pub struct Scene {
    pub conductors: Vec<Conductor>,
    pub poles: Vec<Pole>,
    pub spans: Vec<Span>,
    pub hazards: Vec<Hazard>,
    pub span_stats: Vec<(f64, u32, u32, f64)>,
    pub vegetation_points: u64,
    /// Conductor pieces left out for being shorter than `min_length`.
    pub discarded: usize,
}

/// The closest point of segment `a`–`b` to `(x, y)` in plan: `(distance, t)`.
fn segment(a: &[f64; 3], b: &[f64; 3], x: f64, y: f64) -> (f64, f64) {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = vx * vx + vy * vy;
    let t = if l2 > 0.0 {
        (((x - a[0]) * vx + (y - a[1]) * vy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((x - a[0] - t * vx).hypot(y - a[1] - t * vy), t)
}

/// Plan index of the conductors' vertices.
struct Index {
    cell: f64,
    grid: HashMap<(i64, i64), Vec<(u32, u32)>>,
}

impl Index {
    fn new(conductors: &[Conductor], cell: f64) -> Self {
        let mut grid: HashMap<(i64, i64), Vec<(u32, u32)>> = HashMap::new();
        for (c, cd) in conductors.iter().enumerate() {
            for (i, v) in cd.verts.iter().enumerate() {
                grid.entry(((v[0] / cell).floor() as i64, (v[1] / cell).floor() as i64))
                    .or_default()
                    .push((c as u32, i as u32));
            }
        }
        Self { cell, grid }
    }

    /// The nearest conductor to `(x, y, z)` in plan within `reach`: its number, the distance along
    /// it, the plan distance, and the conductor's height there.
    fn nearest(
        &self,
        conductors: &[Conductor],
        x: f64,
        y: f64,
        reach: f64,
    ) -> Option<(usize, f64, f64, f64)> {
        let (kx, ky) = (
            (x / self.cell).floor() as i64,
            (y / self.cell).floor() as i64,
        );
        let mut best: Option<(f64, u32, u32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &(c, i) in self.grid.get(&(kx + dx, ky + dy)).into_iter().flatten() {
                    let v = &conductors[c as usize].verts[i as usize];
                    let d = (x - v[0]).hypot(y - v[1]);
                    if d <= reach && best.is_none_or(|b| d < b.0) {
                        best = Some((d, c, i));
                    }
                }
            }
        }
        let (_, c, i) = best?;
        let cd = &conductors[c as usize];
        let i = i as usize;
        // refine on the two segments next to the vertex
        let mut out: Option<(f64, f64, f64)> = None;
        for (a, b) in [(i.wrapping_sub(1), i), (i, i + 1)] {
            if a < cd.verts.len() && b < cd.verts.len() {
                let (d, t) = segment(&cd.verts[a], &cd.verts[b], x, y);
                if out.is_none_or(|o| d < o.0) {
                    let z = cd.verts[a][2] + t * (cd.verts[b][2] - cd.verts[a][2]);
                    let s = cd.cum[a] + t * (cd.cum[b] - cd.cum[a]);
                    out = Some((d, s, z));
                }
            }
        }
        let (d, s, z) = out?;
        Some((c as usize, s, d, z))
    }
}

/// Poles from the pole points.
fn find_poles(pts: &[[f64; 3]], p: &CorridorParams) -> Vec<Pole> {
    cluster(pts, p.pole_link, false)
        .into_iter()
        .filter(|g| g.len() >= 5)
        .map(|g| {
            let n = g.len() as f64;
            let (w0, w1) = (
                g.iter().map(|&i| pts[i][0]).fold(f64::INFINITY, f64::min),
                g.iter()
                    .map(|&i| pts[i][0])
                    .fold(f64::NEG_INFINITY, f64::max),
            );
            let (v0, v1) = (
                g.iter().map(|&i| pts[i][1]).fold(f64::INFINITY, f64::min),
                g.iter()
                    .map(|&i| pts[i][1])
                    .fold(f64::NEG_INFINITY, f64::max),
            );
            Pole {
                width: (w1 - w0).max(v1 - v0),
                x: g.iter().map(|&i| pts[i][0]).sum::<f64>() / n,
                y: g.iter().map(|&i| pts[i][1]).sum::<f64>() / n,
                base_z: g.iter().map(|&i| pts[i][2]).fold(f64::INFINITY, f64::min),
                top_z: g
                    .iter()
                    .map(|&i| pts[i][2])
                    .fold(f64::NEG_INFINITY, f64::max),
                points: g.len(),
            }
        })
        .collect()
}

/// The spans of every conductor: between consecutive poles standing beside it.
fn find_spans(conductors: &[Conductor], poles: &[Pole], p: &CorridorParams) -> Vec<Span> {
    let mut spans = Vec::new();
    for (c, cd) in conductors.iter().enumerate() {
        // poles beside this conductor (every conductor a pole stands beside has it as a support)
        // and where along it they stand
        let mut mine: Vec<(f64, usize)> = poles
            .iter()
            .enumerate()
            .filter_map(|(k, pole)| {
                let (s, d) = cd.locate(pole.x, pole.y)?;
                (d <= p.pole_reach).then_some((s, k))
            })
            .collect();
        mine.sort_by(|a, b| a.0.total_cmp(&b.0));
        let z_at = |s: f64| -> f64 {
            let k = cd
                .cum
                .partition_point(|&v| v < s)
                .clamp(1, cd.verts.len() - 1);
            let (a, b) = (k - 1, k);
            let f = if cd.cum[b] > cd.cum[a] {
                (s - cd.cum[a]) / (cd.cum[b] - cd.cum[a])
            } else {
                0.0
            };
            cd.verts[a][2] + f.clamp(0.0, 1.0) * (cd.verts[b][2] - cd.verts[a][2])
        };
        let mut bounds: Vec<(f64, Option<usize>)> = vec![(0.0, None)];
        bounds.extend(mine.iter().map(|&(s, k)| (s, Some(k))));
        bounds.push((cd.length(), None));
        for (idx, w) in bounds.windows(2).enumerate() {
            let (from, to) = (w[0].0, w[1].0);
            if to - from < p.step {
                continue;
            }
            // sag: how far the conductor hangs below the chord between the ends of the span
            let (za, zb) = (z_at(from), z_at(to));
            let sag = cd
                .verts
                .iter()
                .zip(&cd.cum)
                .filter(|(_, s)| **s >= from && **s <= to)
                .map(|(v, s)| za + (zb - za) * (s - from) / (to - from).max(1e-9) - v[2])
                .fold(0.0, f64::max);
            spans.push(Span {
                conductor: c,
                index: idx,
                pole_a: w[0].1,
                pole_b: w[1].1,
                from,
                to,
                sag,
            });
        }
    }
    spans
}

fn span_of(spans: &[Span], conductor: usize, s: f64) -> usize {
    spans
        .iter()
        .position(|sp| sp.conductor == conductor && s >= sp.from && s <= sp.to)
        .or_else(|| spans.iter().position(|sp| sp.conductor == conductor))
        .unwrap_or(0)
}

/// Reads the clouds and finds the conductors, poles and hazards.
pub fn analyse(j: &CorridorJob) -> Result<Scene> {
    let p = &j.params;
    if !(p.link > 0.0 && p.step > 0.0 && p.cell > 0.0 && p.half_width > 0.0) {
        return Err(Error::Param("las distancias deben ser positivas".into()));
    }
    let dtm = Grid::read(&j.dtm)?;
    // pass 1: wires and poles
    let (mut wires, mut pole_pts): (Vec<[f64; 3]>, Vec<[f64; 3]>) = (Vec::new(), Vec::new());
    for input in &j.inputs {
        Reader::open(input)?.scan(|pt| {
            if pt.withheld {
                return;
            }
            if pt.class == p.wire_class {
                wires.push([pt.x, pt.y, pt.z]);
            } else if pt.class == p.pole_class {
                pole_pts.push([pt.x, pt.y, pt.z]);
            }
        })?;
        check_cancel()?;
    }
    if wires.is_empty() {
        return Err(Error::Param(format!(
            "la nube no tiene puntos de cables (clase {}): clasifica antes «Cables y postes» en Clasificar",
            p.wire_class
        )));
    }
    let mut conductors: Vec<Conductor> = Vec::new();
    for g in join_pieces(&wires, cluster(&wires, p.link, true), p) {
        if g.len() < p.min_points {
            continue;
        }
        let pts: Vec<[f64; 3]> = g.iter().map(|&i| wires[i]).collect();
        conductors.extend(fit(&pts, p));
    }
    drop(wires);
    if conductors.is_empty() {
        return Err(Error::Param(format!(
            "ningún grupo de puntos de cable llega a {} puntos: baja el mínimo o sube el radio de unión",
            p.min_points
        )));
    }
    // what the classifier took for lines in the crowns is short, and what it took for poles is low,
    // wide or far from every conductor
    let before = conductors.len();
    conductors.retain(|c| c.length() >= p.min_length);
    if conductors.is_empty() {
        return Err(Error::Param(format!(
            "ninguno de los {before} grupos de puntos de cable mide {} m o más: parecen ruido de la clasificación (copas de árboles); baja la longitud mínima si de verdad hay vanos más cortos",
            p.min_length
        )));
    }
    let discarded = before - conductors.len();
    let poles: Vec<Pole> = find_poles(&pole_pts, p)
        .into_iter()
        .filter(|q| q.top_z - q.base_z >= p.min_pole_height && q.width <= p.pole_max_width)
        .filter(|q| {
            conductors
                .iter()
                .any(|c| c.locate(q.x, q.y).is_some_and(|(_, d)| d <= p.pole_reach))
        })
        .collect();
    let spans = find_spans(&conductors, &poles, p);
    check_cancel()?;
    // pass 2: the vegetation near the conductors
    let index = Index::new(&conductors, p.half_width.max(5.0));
    let mut cells: HashMap<(i64, i64), Cell> = HashMap::new();
    let mut seen = 0u64;
    for input in &j.inputs {
        Reader::open(input)?.scan(|pt| {
            if pt.withheld || !p.vegetation.allows(pt.class) {
                return;
            }
            let Some(g) = dtm.sample(pt.x, pt.y) else {
                return;
            };
            let h = pt.z - f64::from(g);
            if h < p.min_height {
                return;
            }
            let Some((c, s, d_h, zc)) = index.nearest(&conductors, pt.x, pt.y, p.half_width) else {
                return;
            };
            seen += 1;
            let d3 = d_h.hypot(zc - pt.z);
            let key = (
                (pt.x / p.cell).floor() as i64,
                (pt.y / p.cell).floor() as i64,
            );
            let cell = cells.entry(key).or_insert(Cell {
                max_h: f64::NEG_INFINITY,
                top: [pt.x, pt.y, pt.z],
                d_h,
                wire_z: zc,
                d3_min: f64::INFINITY,
                conductor: c,
                along: s,
            });
            cell.d3_min = cell.d3_min.min(d3);
            if h > cell.max_h {
                *cell = Cell {
                    max_h: h,
                    top: [pt.x, pt.y, pt.z],
                    d_h,
                    wire_z: zc,
                    d3_min: cell.d3_min,
                    conductor: c,
                    along: s,
                };
            }
        })?;
        check_cancel()?;
    }
    let mut hazards = Vec::new();
    let mut keys: Vec<_> = cells.keys().copied().collect();
    keys.sort_unstable();
    let mut stats: Vec<SpanStats> = spans
        .iter()
        .map(|_| SpanStats {
            min_d3: f64::INFINITY,
            ..SpanStats::default()
        })
        .collect();
    for k in keys {
        let c = cells[&k];
        let invasion = c.d3_min < p.min_distance;
        let fall = c.max_h + p.fall_margin >= c.d_h;
        let sp = span_of(&spans, c.conductor, c.along);
        if let Some(st) = stats.get_mut(sp) {
            st.min_d3 = st.min_d3.min(c.d3_min);
            if invasion || fall {
                st.invasion += u32::from(invasion);
                st.fall += u32::from(fall);
                st.tallest = st.tallest.max(c.max_h);
            }
        }
        if invasion || fall {
            hazards.push(Hazard {
                x: c.top[0],
                y: c.top[1],
                z: c.top[2],
                height: c.max_h,
                horizontal: c.d_h,
                distance: c.d3_min,
                clearance: c.wire_z - c.top[2],
                invasion,
                fall,
                conductor: c.conductor,
                span: sp,
            });
        }
    }
    let span_stats = stats
        .iter()
        .map(|s| (s.min_d3, s.invasion, s.fall, s.tallest))
        .collect();
    Ok(Scene {
        conductors,
        poles,
        spans,
        hazards,
        span_stats,
        vegetation_points: seen,
        discarded,
    })
}

fn kind(h: &Hazard) -> &'static str {
    match (h.invasion, h.fall) {
        (true, true) => "invasion+caida",
        (true, false) => "invasion",
        _ => "caida",
    }
}

fn name(c: usize, s: &Span) -> String {
    format!("C{}-V{}", c + 1, s.index + 1)
}

/// Runs the whole analysis and writes the results.
pub fn run(j: &CorridorJob) -> Result<String> {
    let mut files = j.inputs.clone();
    files.push(j.dtm.clone());
    let unified = crate::crs::unify(&files, false)?;
    let notes = unified.warnings.clone();
    let moved = CorridorJob {
        inputs: unified.all(&j.inputs),
        dtm: unified.path(&j.dtm),
        ..j.clone()
    };
    let sc = analyse(&moved)?;
    let out = j.outputs();
    paths::ensure_parent(&j.output)?;
    let epsg = moved
        .inputs
        .first()
        .and_then(|p| Reader::open(p).ok())
        .and_then(|r| r.header.epsg);
    let prj = epsg.and_then(prj_for_epsg);
    let mut lines = vec![format!(
        "{} conductor(es), {} poste(s), {} vano(s); {} puntos de vegetación a menos de {} m de un conductor",
        sc.conductors.len(),
        sc.poles.len(),
        sc.spans.len(),
        sc.vegetation_points,
        j.params.half_width
    )];
    // spans
    let mut csv = std::io::BufWriter::new(std::fs::File::create(&out.spans)?);
    writeln!(
        csv,
        "vano,conductor,poste_a,poste_b,longitud_m,flecha_m,celdas_invasion,celdas_caida,distancia_min_m,altura_max_riesgo_m"
    )?;
    for (sp, st) in sc.spans.iter().zip(&sc.span_stats) {
        let pole = |k: Option<usize>| k.map_or(String::new(), |k| format!("P{}", k + 1));
        writeln!(
            csv,
            "{},C{},{},{},{:.1},{:.2},{},{},{},{}",
            name(sp.conductor, sp),
            sp.conductor + 1,
            pole(sp.pole_a),
            pole(sp.pole_b),
            sp.to - sp.from,
            sp.sag,
            st.1,
            st.2,
            if st.0.is_finite() {
                format!("{:.2}", st.0)
            } else {
                String::new()
            },
            if st.3 > 0.0 {
                format!("{:.1}", st.3)
            } else {
                String::new()
            },
        )?;
    }
    csv.flush()?;
    lines.push(format!("  vanos: {}", out.spans.display()));
    // hazards
    let mut csv = std::io::BufWriter::new(std::fs::File::create(&out.hazards_table)?);
    writeln!(
        csv,
        "id,riesgo,vano,x,y,z,altura_m,distancia_horizontal_m,distancia_min_m,holgura_vertical_m"
    )?;
    for (k, h) in sc.hazards.iter().enumerate() {
        writeln!(
            csv,
            "{},{},{},{:.2},{:.2},{:.2},{:.1},{:.1},{:.2},{:.2}",
            k + 1,
            kind(h),
            sc.spans
                .get(h.span)
                .map_or(String::new(), |s| name(h.conductor, s)),
            h.x,
            h.y,
            h.z,
            h.height,
            h.horizontal,
            h.distance,
            h.clearance
        )?;
    }
    csv.flush()?;
    let (inv, fall) = (
        sc.hazards.iter().filter(|h| h.invasion).count(),
        sc.hazards.iter().filter(|h| h.fall).count(),
    );
    lines.push(format!(
        "  puntos de riesgo: {} ({inv} por invasión a menos de {} m, {fall} por posible caída): {}",
        sc.hazards.len(),
        j.params.min_distance,
        out.hazards_table.display()
    ));
    // shapefiles
    if let Some(path) = &out.conductors {
        let table = Table {
            columns: vec![
                ("id".into(), ColKind::Num(8, 0)),
                ("puntos".into(), ColKind::Num(10, 0)),
                ("longitud".into(), ColKind::Num(12, 1)),
                ("huecos".into(), ColKind::Num(6, 0)),
                ("cortes".into(), ColKind::Num(6, 0)),
            ],
            rows: sc
                .conductors
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    vec![
                        Val::Int(k as i64 + 1),
                        Val::Int(c.points as i64),
                        Val::Num(c.length()),
                        Val::Int(c.gaps as i64),
                        Val::Int(c.breaks as i64),
                    ]
                })
                .collect(),
        };
        let lines3: Vec<Vec<[f64; 3]>> = sc.conductors.iter().map(|c| c.verts.clone()).collect();
        write_polylines(path, &lines3, true, &table, prj.as_deref())?;
        lines.push(format!("  conductores: {}", path.display()));
    }
    if let Some(path) = &out.poles {
        let table = Table {
            columns: vec![
                ("id".into(), ColKind::Num(8, 0)),
                ("altura".into(), ColKind::Num(8, 1)),
                ("puntos".into(), ColKind::Num(10, 0)),
            ],
            rows: sc
                .poles
                .iter()
                .enumerate()
                .map(|(k, p)| {
                    vec![
                        Val::Int(k as i64 + 1),
                        Val::Num(p.top_z - p.base_z),
                        Val::Int(p.points as i64),
                    ]
                })
                .collect(),
        };
        let pts: Vec<[f64; 3]> = sc.poles.iter().map(|p| [p.x, p.y, p.top_z]).collect();
        write_points(path, &pts, true, &table, prj.as_deref())?;
        lines.push(format!("  postes: {}", path.display()));
    }
    if let Some(path) = &out.hazards {
        let table = Table {
            columns: vec![
                ("id".into(), ColKind::Num(8, 0)),
                ("riesgo".into(), ColKind::Text(16)),
                ("vano".into(), ColKind::Text(12)),
                ("altura".into(), ColKind::Num(8, 1)),
                ("dist_h".into(), ColKind::Num(8, 1)),
                ("dist_min".into(), ColKind::Num(8, 2)),
                ("holgura".into(), ColKind::Num(8, 2)),
            ],
            rows: sc
                .hazards
                .iter()
                .enumerate()
                .map(|(k, h)| {
                    vec![
                        Val::Int(k as i64 + 1),
                        Val::Text(kind(h).to_string()),
                        Val::Text(
                            sc.spans
                                .get(h.span)
                                .map_or(String::new(), |s| name(h.conductor, s)),
                        ),
                        Val::Num(h.height),
                        Val::Num(h.horizontal),
                        Val::Num(h.distance),
                        Val::Num(h.clearance),
                    ]
                })
                .collect(),
        };
        let pts: Vec<[f64; 3]> = sc.hazards.iter().map(|h| [h.x, h.y, h.z]).collect();
        write_points(path, &pts, true, &table, prj.as_deref())?;
        lines.push(format!("  riesgo (puntos): {}", path.display()));
    }
    if sc.discarded > 0 {
        lines.push(format!(
            "  {} grupo(s) de puntos de cable descartados por medir menos de {} m",
            sc.discarded, j.params.min_length
        ));
    }
    let gaps: usize = sc.conductors.iter().map(|c| c.breaks).sum();
    if gaps > 0 {
        lines.push(format!(
            "  Aviso: {gaps} corte(s) de conductor por huecos de más de {} m sin puntos de cable; ahí no se ha medido",
            j.params.gap_max
        ));
    }
    Ok(lines.join("\n") + &crate::crs::notes(&notes))
}

/// The paths of the inputs the job reads.
pub fn inputs(j: &CorridorJob) -> Vec<&Path> {
    let mut v: Vec<&Path> = j.inputs.iter().map(PathBuf::as_path).collect();
    v.push(j.dtm.as_path());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::las::{Writer, WriterOptions};

    /// A record of point format 0: position (scaled by 0.001 from the offset), class.
    fn record(x: f64, y: f64, z: f64, class: u8) -> Vec<u8> {
        let mut r = vec![0u8; 20];
        for (k, v) in [x - 500_000.0, y - 4_000_000.0, z].iter().enumerate() {
            r[4 * k..4 * k + 4].copy_from_slice(&((v / 0.001).round() as i32).to_le_bytes());
        }
        r[14] = 0x09;
        r[15] = class;
        r
    }

    #[test]
    fn conductors_poles_and_hazards_are_found_in_a_synthetic_line() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_corridor_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let (x0, y0) = (500_000.0, 4_000_000.0);
        let mut recs: Vec<Vec<u8>> = Vec::new();
        // two conductors 4 m apart, from x = 0 to 100 m, hanging 4 m at mid-span from 12 m
        let sag = |s: f64| 12.0 - 4.0 * (1.0 - ((s - 50.0) / 50.0).powi(2));
        for (k, dy) in [0.0, 4.0].iter().enumerate() {
            let mut s = 0.0;
            while s <= 100.0 {
                // a missing stretch in the middle of the first conductor
                if !(k == 0 && (40.0..46.0).contains(&s)) {
                    recs.push(record(
                        x0 + s,
                        y0 + dy,
                        sag(s) + 0.05 * ((s * 7.0).sin()),
                        WIRE,
                    ));
                }
                s += 0.5;
            }
        }
        // two poles, between the conductors, 12 m high
        for px in [0.0, 100.0] {
            let mut z = 0.0;
            while z <= 12.0 {
                recs.push(record(x0 + px, y0 + 2.0, z, POLE));
                z += 0.5;
            }
        }
        // trees: A under the line (invasion), B tall and 15 m aside (fall), C short and far
        for (tx, ty, h) in [(50.0, 2.0, 6.5), (20.0, 15.0, 20.0), (80.0, 25.0, 10.0)] {
            let mut z = 2.0;
            while z <= h {
                recs.push(record(x0 + tx, y0 + ty, z, 5));
                z += 0.5;
            }
        }
        // a header-only template, then the file
        let tpl = crate::ply::template(&dir, 0, 0.001, [x0, y0, 0.0], Some(25830))?;
        let cloud = dir.join("linea.las");
        let mut w = Writer::create_with(&cloud, &tpl, &WriterOptions::default())?;
        for r in &recs {
            w.write(r)?;
        }
        w.finish()?;
        let mut dtm = Grid::new(x0 - 50.0, y0 - 50.0, 1.0, 250, 150)?;
        dtm.data.fill(0.0);
        dtm.write_tiff(&dir.join("mdt.tif"), Some(25830))?;
        let job = CorridorJob {
            inputs: vec![cloud],
            dtm: dir.join("mdt.tif"),
            output: dir.join("linea.shp"),
            params: CorridorParams::default(),
            formats: Formats {
                vector: true,
                csv: true,
            },
        };
        let sc = analyse(&job)?;
        // the missing stretch is filled, so both are whole conductors, 100 m long
        assert_eq!(
            sc.conductors.len(),
            2,
            "{:?}",
            sc.conductors.iter().map(|c| c.length()).collect::<Vec<_>>()
        );
        for c in &sc.conductors {
            assert!((c.length() - 100.0).abs() < 4.0, "{}", c.length());
        }
        assert!(sc.conductors.iter().any(|c| c.gaps > 0));
        // the filled gap follows the sag: at s = 43 the wire hangs about 12 - 4(1-(7/50)^2) = 8.08
        let first = sc
            .conductors
            .iter()
            .find(|c| (c.verts[0][1] - y0).abs() < 1.0)
            .ok_or(Error::Cancelled)?;
        let v = first
            .verts
            .iter()
            .min_by(|a, b| {
                (a[0] - (x0 + 43.0))
                    .abs()
                    .total_cmp(&(b[0] - (x0 + 43.0)).abs())
            })
            .ok_or(Error::Cancelled)?;
        assert!((v[2] - 8.08).abs() < 0.4, "{}", v[2]);
        assert_eq!(sc.poles.len(), 2);
        assert!((sc.poles[0].top_z - sc.poles[0].base_z - 12.0).abs() < 0.6);
        assert_eq!(sc.spans.len(), 2);
        assert!(
            sc.spans
                .iter()
                .all(|s| s.pole_a.is_some() && s.pole_b.is_some()),
            "{:?}",
            sc.spans
        );
        assert!((sc.spans[0].sag - 4.0).abs() < 0.8, "{}", sc.spans[0].sag);
        // A is an invasion, B a fall hazard, C nothing
        assert_eq!(sc.hazards.len(), 2, "{:?}", sc.hazards);
        let a = sc
            .hazards
            .iter()
            .find(|h| h.invasion)
            .ok_or(Error::Cancelled)?;
        assert!((a.x - (x0 + 50.0)).abs() < 3.0 && a.distance < 3.0);
        let b = sc
            .hazards
            .iter()
            .find(|h| !h.invasion)
            .ok_or(Error::Cancelled)?;
        assert!(b.fall && (b.x - (x0 + 20.0)).abs() < 3.0);
        // the report and the files
        let text = run(&job)?;
        assert!(
            text.contains("2 conductor(es), 2 poste(s), 2 vano(s)"),
            "{text}"
        );
        let out = job.outputs();
        let spans = std::fs::read_to_string(&out.spans)?;
        assert_eq!(spans.lines().count(), 3, "{spans}");
        assert!(out.conductors.as_ref().is_some_and(|p| p.exists()));
        assert!(out.hazards.as_ref().is_some_and(|p| p.exists()));
        // a cloud without wires says so
        let none = dir.join("sin.las");
        let mut w = Writer::create_with(&none, &tpl, &WriterOptions::default())?;
        w.write(&record(x0, y0, 1.0, 2))?;
        w.finish()?;
        let bad = CorridorJob {
            inputs: vec![none],
            ..job
        };
        assert!(analyse(&bad).is_err());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_parabola_is_recovered_from_samples() {
        let samples: Vec<(f64, f64)> = [-6.0, -4.0, 4.0, 6.0]
            .iter()
            .map(|&t| (t, 3.0 - 0.1 * t + 0.02 * t * t))
            .collect();
        let c = parabola(&samples).unwrap_or_default();
        assert!(
            (c[0] - 3.0).abs() < 1e-9 && (c[1] + 0.1).abs() < 1e-9 && (c[2] - 0.02).abs() < 1e-9
        );
        // three samples on one point cannot say anything
        assert!(parabola(&[(1.0, 1.0), (1.0, 1.0), (1.0, 1.0), (1.0, 1.0)]).is_none());
    }
}
