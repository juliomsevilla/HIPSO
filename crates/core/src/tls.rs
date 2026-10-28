//! Stem measurement and cubing from terrestrial (and mobile or photogrammetric) point clouds of
//! a forest plot. The steps:
//!
//! 1. The cloud is read as a stream and reduced to one point per voxel, so memory follows the
//!    occupied voxels and not the number of points.
//! 2. Heights above ground: the points already classified as ground, a cloth simulation on the
//!    lowest points, or heights the cloud already carries.
//! 3. Stems are found in a horizontal stripe (by default 0.7–3.5 m above ground): points whose
//!    neighbourhood is stretched vertically are kept, grouped, and the groups are peeled of
//!    branches by repeating the two steps. A principal axis is fitted to each group.
//! 4. Each stem is followed up and down from the stripe, fitting a circle to a thin horizontal
//!    slice every few centimetres and predicting the next centre from the last ones, so leaning
//!    and curved stems are followed. A section counts only if it has enough points, around
//!    enough of the circumference, and none inside; sections that disagree with their
//!    neighbours are discarded and short gaps are interpolated.
//! 5. Logs: from the base height up, a diameter every `log_len` metres, and Smalian's formula
//!    for the volume of each log, ((A1 + A2) / 2) · L, until the stem is as thin as the top
//!    diameter (the last log is cut where the diameter reaches it).
//! 6. Optionally, every point of the cloud is given to its nearest stem axis and written as one
//!    cloud per tree.

// Small fixed-size matrix code reads best with indices.
#![allow(clippy::needless_range_loop)]

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::csf::{self, CsfParams};
use crate::grid::Grid;
use crate::las::{Point, Reader, Writer, WriterOptions};
use crate::vector::{ColKind, Table, Val};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct TlsParams {
    /// Voxel (m) the cloud is reduced to.
    pub voxel: f64,
    /// The cloud's z are already heights above ground.
    pub normalized: bool,
    /// Distance between cloth particles (m) when the ground is simulated.
    pub cloth: f64,
    /// Stripe where stems are looked for, heights above ground.
    pub stripe: (f64, f64),
    /// Rounds of vertical filtering and grouping ("peeling off branches").
    pub peel: usize,
    /// Radius (m) of the neighbourhood used for verticality.
    pub vert_radius: f64,
    /// Smallest verticality (0–1) of a stem point.
    pub vert_min: f64,
    /// Fewest voxels of a stem group in the stripe.
    pub min_stem_points: usize,
    /// Vertical extent (m) a group must have to be a stem.
    pub height_range: f64,
    /// Largest and smallest diameter (m) of a stem section.
    pub max_diam: f64,
    pub min_diam: f64,
    /// Half thickness (m) of a section slice.
    pub slice: f64,
    /// Fewest voxels in a section, sectors (of 16) it must occupy, and voxels allowed inside
    /// the inner circle.
    pub min_section_points: usize,
    pub min_sectors: usize,
    pub max_inner: usize,
    /// Height (m) above ground where the first log starts.
    pub base_height: f64,
    /// Diameter (m) at which the merchantable stem ends.
    pub top_diam: f64,
    /// Distance (m) between the diameters measured along the stem: the length of a log.
    pub log_len: f64,
    /// Highest section (m).
    pub max_height: f64,
    /// Distance (m) up to which a point is given to a stem for the clouds by tree.
    pub assign_dist: f64,
    /// Distance (m) between the bases of stems that grow on the same stool (sprouts of a
    /// coppice, or the branches of a fork).
    pub stool_dist: f64,
    /// Height (m) of the diameter. Stems that are apart at this height are sprouts of one
    /// stool, each a tree; stems that are one below it and fork above it are one forked tree.
    pub fork_height: f64,
    /// Longest stretch (m) of a stem that may go unmeasured (an occlusion, foliage, a sparse
    /// part) before it is taken as lost. The search widens with the stretch.
    pub max_gap: f64,
    /// Share (0–1) of the height of the tree that its merchantable height is expected to be, used
    /// to estimate it for a stem that was lost while still thick and far below the top of the
    /// tree. 0: no estimate (the stem stays truncated).
    pub lost_top_share: f64,
}

impl Default for TlsParams {
    fn default() -> Self {
        Self {
            voxel: 0.03,
            normalized: false,
            cloth: 0.5,
            stripe: (0.7, 3.5),
            peel: 2,
            vert_radius: 0.1,
            vert_min: 0.7,
            min_stem_points: 300,
            height_range: 0.7,
            max_diam: 1.0,
            min_diam: 0.04,
            slice: 0.05,
            min_section_points: 25,
            min_sectors: 9,
            max_inner: 5,
            base_height: 0.10,
            top_diam: 0.07,
            log_len: 0.5,
            max_height: 40.0,
            assign_dist: 8.0,
            stool_dist: 0.6,
            fork_height: 1.3,
            max_gap: 2.0,
            lost_top_share: 0.0,
        }
    }
}

impl TlsParams {
    fn check(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::Param(m.to_string()));
        if !(self.voxel >= 0.005) {
            return bad("el vóxel debe medir al menos 5 mm");
        }
        if !(self.log_len >= 0.1) {
            return bad("the log length must be at least 0.1 m");
        }
        if !(self.top_diam > 0.0 && self.top_diam < self.max_diam) {
            return bad("the top diameter must be positive and below the largest diameter");
        }
        if !(self.base_height >= 0.0) {
            return bad("the base height cannot be negative");
        }
        if !(self.stripe.0 >= 0.0 && self.stripe.1 > self.stripe.0) {
            return bad("the stripe needs a lower limit below its upper limit");
        }
        if !(self.min_diam > 0.0 && self.min_diam < self.max_diam) {
            return bad("the smallest diameter must be below the largest");
        }
        if !(self.stool_dist >= 0.0) {
            return bad("the distance between the stems of a stool cannot be negative");
        }
        if !(self.fork_height > 0.0) {
            return bad("the height of the diameter must be positive");
        }
        if !(self.max_gap >= 0.0) {
            return bad("the longest gap cannot be negative");
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------------- small helpers

#[derive(Default)]
struct Fx(u64);

impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(*b)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
        self.0 ^= self.0 >> 29;
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

fn pack(ix: i64, iy: i64, iz: i64) -> u64 {
    const M: u64 = 0x1F_FFFF;
    (((ix + (1 << 20)) as u64 & M) << 42)
        | (((iy + (1 << 20)) as u64 & M) << 21)
        | ((iz + (1 << 20)) as u64 & M)
}

/// Eigenvector of the largest eigenvalue of a symmetric 3×3 matrix.
fn principal(m: [[f64; 3]; 3]) -> [f64; 3] {
    eigen(m).1
}

/// Eigenvalues (largest first) and the eigenvector of the largest, by Jacobi rotations.
pub(crate) fn eigen(m: [[f64; 3]; 3]) -> ([f64; 3], [f64; 3]) {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..12 {
        let (mut p, mut q, mut big) = (0, 1, a[0][1].abs());
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > big {
                (p, q, big) = (i, j, a[i][j].abs());
            }
        }
        if big < 1e-14 {
            break;
        }
        let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
        let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
        let (c, s) = (1.0 / (t * t + 1.0).sqrt(), t / (t * t + 1.0).sqrt());
        for k in 0..3 {
            let (akp, akq) = (a[k][p], a[k][q]);
            a[k][p] = c * akp - s * akq;
            a[k][q] = s * akp + c * akq;
        }
        for k in 0..3 {
            let (apk, aqk) = (a[p][k], a[q][k]);
            a[p][k] = c * apk - s * aqk;
            a[q][k] = s * apk + c * aqk;
        }
        for row in &mut v {
            let (vp, vq) = (row[p], row[q]);
            row[p] = c * vp - s * vq;
            row[q] = s * vp + c * vq;
        }
    }
    let mut vals = [a[0][0], a[1][1], a[2][2]];
    let k = (0..3)
        .max_by(|&i, &j| a[i][i].total_cmp(&a[j][j]))
        .unwrap_or(0);
    vals.sort_by(|x, y| y.total_cmp(x));
    (vals, [v[0][k], v[1][k], v[2][k]])
}

pub(crate) fn covariance(
    pts: impl Iterator<Item = [f64; 3]> + Clone,
) -> ([f64; 3], [[f64; 3]; 3], usize) {
    let (mut n, mut m) = (0usize, [0.0; 3]);
    for p in pts.clone() {
        n += 1;
        for k in 0..3 {
            m[k] += p[k];
        }
    }
    if n == 0 {
        return (m, [[0.0; 3]; 3], 0);
    }
    for v in &mut m {
        *v /= n as f64;
    }
    let mut c = [[0.0; 3]; 3];
    for p in pts {
        let d = [p[0] - m[0], p[1] - m[1], p[2] - m[2]];
        for i in 0..3 {
            for j in 0..3 {
                c[i][j] += d[i] * d[j];
            }
        }
    }
    for row in &mut c {
        for v in row {
            *v /= n as f64;
        }
    }
    (m, c, n)
}

// ----------------------------------------------------------------------------- the cloud

struct Cloud {
    /// Lower corner of the cloud: coordinates below are relative to it.
    origin: [f64; 3],
    xyz: Vec<[f32; 3]>,
    class: Vec<u8>,
    /// Height above ground.
    z0: Vec<f32>,
    date: Option<String>,
    epsg: Option<u16>,
    /// The cloud, before voxels: points read.
    read: u64,
}

fn load(input: &Path, p: &TlsParams) -> Result<Cloud> {
    let r = Reader::open(input)?;
    let b = r.header.bounds;
    let origin = [b[0], b[1], b[2]];
    let v = p.voxel;
    let mut seen: FxMap<u64, ()> = FxMap::default();
    let (mut xyz, mut class) = (Vec::new(), Vec::new());
    let mut read = 0u64;
    r.scan(|pt| {
        read += 1;
        if pt.is_noise() {
            return;
        }
        let (x, y, z) = (pt.x - origin[0], pt.y - origin[1], pt.z - origin[2]);
        let key = pack(
            (x / v).floor() as i64,
            (y / v).floor() as i64,
            (z / v).floor() as i64,
        );
        if seen.insert(key, ()).is_none() {
            xyz.push([x as f32, y as f32, z as f32]);
            class.push(pt.class);
        }
    })?;
    if xyz.is_empty() {
        return Err(Error::Param(format!(
            "{}: no hay puntos que analizar",
            input.display()
        )));
    }
    Ok(Cloud {
        origin,
        z0: Vec::new(),
        xyz,
        class,
        date: r.capture_date(),
        epsg: r.header.epsg,
        read,
    })
}

/// Heights above ground for every point; the ground grid used (relative coordinates).
fn normalize(c: &mut Cloud, p: &TlsParams) -> Result<Option<Grid>> {
    if p.normalized {
        c.z0 = c.xyz.iter().map(|q| q[2] + c.origin[2] as f32).collect();
        return Ok(None);
    }
    let (mut x1, mut y1) = (0f32, 0f32);
    for q in &c.xyz {
        x1 = x1.max(q[0]);
        y1 = y1.max(q[1]);
    }
    let cell = p.cloth.max(0.1);
    let mut g = Grid::covering([0.0, 0.0, f64::from(x1) + cell, f64::from(y1) + cell], cell)?;
    let n_ground = c.class.iter().filter(|k| **k == 2).count();
    let ground: Vec<usize> = if n_ground * 200 >= c.class.len() {
        // the delivery's own ground class
        (0..c.class.len()).filter(|&i| c.class[i] == 2).collect()
    } else {
        // cloth simulation on the lowest point of every 0.25 m cell
        let mut low: FxMap<u64, usize> = FxMap::default();
        for (i, q) in c.xyz.iter().enumerate() {
            let k = pack(
                (q[0] / 0.25).floor() as i64,
                (q[1] / 0.25).floor() as i64,
                0,
            );
            low.entry(k)
                .and_modify(|j| {
                    if q[2] < c.xyz[*j][2] {
                        *j = i;
                    }
                })
                .or_insert(i);
        }
        let idx: Vec<usize> = low.into_values().collect();
        let pts: Vec<Point> = idx
            .iter()
            .map(|&i| Point {
                x: f64::from(c.xyz[i][0]),
                y: f64::from(c.xyz[i][1]),
                z: f64::from(c.xyz[i][2]),
                number_of_returns: 1,
                return_number: 1,
                ..Point::default()
            })
            .collect();
        let flags = csf::ground(
            &pts,
            &CsfParams {
                resolution: cell,
                rigidness: 2,
                threshold: 0.25,
                ..CsfParams::default()
            },
        )?;
        idx.into_iter()
            .zip(flags)
            .filter_map(|(i, f)| f.then_some(i))
            .collect()
    };
    if ground.is_empty() {
        return Err(Error::Param(
            "no se encuentra suelo: indica que las alturas ya son sobre el suelo (nube normalizada) o revisa la nube"
                .into(),
        ));
    }
    let mut sum = vec![0f64; g.len()];
    let mut cnt = vec![0u32; g.len()];
    for &i in &ground {
        let q = c.xyz[i];
        if let Some(k) = g.index(f64::from(q[0]), f64::from(q[1])) {
            sum[k] += f64::from(q[2]);
            cnt[k] += 1;
        }
    }
    for k in 0..g.len() {
        g.data[k] = if cnt[k] > 0 {
            (sum[k] / f64::from(cnt[k])) as f32
        } else {
            f32::NAN
        };
    }
    g.fill_holes();
    c.z0 = c
        .xyz
        .iter()
        .map(|q| {
            let gz = g.sample(f64::from(q[0]), f64::from(q[1])).unwrap_or(q[2]);
            q[2] - gz
        })
        .collect();
    Ok(Some(g))
}

// ----------------------------------------------------------------------------- stems in the stripe

/// A stem found in the stripe: a principal axis through `c`, pointing up.
#[derive(Clone, Debug)]
struct Stem {
    /// Point of the axis at height-above-ground `h0` (relative coordinates, x y).
    c: [f64; 2],
    h0: f64,
    /// Horizontal drift of the axis per metre of height.
    slope: [f64; 2],
    n: usize,
    /// The heights the axis was measured between. Inside them it is where the stem is, and
    /// is trusted over a prediction from the sections already measured: that is what keeps the
    /// branches of a fork, which share the stem below, each on its own.
    from: f64,
    to: f64,
}

/// Verticality of the points of `set` (the cosine between the main direction of their
/// neighbourhood and the vertical), using neighbours from `set` only.
fn verticality(xyz: &[[f32; 3]], set: &[u32], radius: f64) -> Vec<f32> {
    let r = radius as f32;
    let key = |q: &[f32; 3]| {
        (
            (q[0] / r).floor() as i64,
            (q[1] / r).floor() as i64,
            (q[2] / r).floor() as i64,
        )
    };
    let mut order: Vec<u32> = (0..set.len() as u32).collect();
    let keys: Vec<u64> = set
        .iter()
        .map(|&i| {
            let k = key(&xyz[i as usize]);
            pack(k.0, k.1, k.2)
        })
        .collect();
    order.par_sort_unstable_by_key(|&j| keys[j as usize]);
    let mut cells: FxMap<u64, (u32, u32)> = FxMap::default();
    let mut s = 0usize;
    while s < order.len() {
        let k = keys[order[s] as usize];
        let mut e = s + 1;
        while e < order.len() && keys[order[e] as usize] == k {
            e += 1;
        }
        cells.insert(k, (s as u32, (e - s) as u32));
        s = e;
    }
    let r2 = r * r;
    (0..set.len())
        .into_par_iter()
        .map(|j| {
            let q = xyz[set[j] as usize];
            let (cx, cy, cz) = key(&q);
            let mut nb: Vec<[f64; 3]> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(&(st, n)) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) {
                            for &o in &order[st as usize..(st + n) as usize] {
                                let w = xyz[set[o as usize] as usize];
                                let d = (w[0] - q[0]).powi(2)
                                    + (w[1] - q[1]).powi(2)
                                    + (w[2] - q[2]).powi(2);
                                if d <= r2 {
                                    nb.push([f64::from(w[0]), f64::from(w[1]), f64::from(w[2])]);
                                }
                            }
                        }
                    }
                }
            }
            if nb.len() < 6 {
                return 0.0;
            }
            let (_, cov, _) = covariance(nb.iter().copied());
            principal(cov)[2].abs() as f32
        })
        .collect()
}

/// Groups of points closer than `eps` (chained): the group number of each, and their sizes.
fn group(xyz: &[[f32; 3]], set: &[u32], eps: f64) -> (Vec<u32>, Vec<usize>) {
    let e = eps as f32;
    let key = |q: &[f32; 3]| {
        (
            (q[0] / e).floor() as i64,
            (q[1] / e).floor() as i64,
            (q[2] / e).floor() as i64,
        )
    };
    let mut cells: FxMap<u64, Vec<u32>> = FxMap::default();
    for (j, &i) in set.iter().enumerate() {
        let k = key(&xyz[i as usize]);
        cells.entry(pack(k.0, k.1, k.2)).or_default().push(j as u32);
    }
    let mut label = vec![u32::MAX; set.len()];
    let mut sizes = Vec::new();
    let e2 = e * e;
    let mut stack = Vec::new();
    for s in 0..set.len() {
        if label[s] != u32::MAX {
            continue;
        }
        let id = sizes.len() as u32;
        label[s] = id;
        stack.push(s);
        let mut n = 0;
        while let Some(j) = stack.pop() {
            n += 1;
            let q = xyz[set[j] as usize];
            let (cx, cy, cz) = key(&q);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) {
                            for &o in list {
                                if label[o as usize] != u32::MAX {
                                    continue;
                                }
                                let w = xyz[set[o as usize] as usize];
                                let d = (w[0] - q[0]).powi(2)
                                    + (w[1] - q[1]).powi(2)
                                    + (w[2] - q[2]).powi(2);
                                if d <= e2 {
                                    label[o as usize] = id;
                                    stack.push(o as usize);
                                }
                            }
                        }
                    }
                }
            }
        }
        sizes.push(n);
    }
    (label, sizes)
}

/// One stem followed through the levels of the stripe: (height, x, y, diameter, voxels) of its
/// circle at each level it was found.
/// A circle at a height: (height, x, y, diameter, voxels).
type Circle = (f64, f64, f64, f64, usize);

struct Chain {
    at: Vec<Circle>,
    last_level: usize,
}

impl Chain {
    /// Where the chain's circle is expected at height `h`: its last centre carried on with the
    /// drift between its first and last circles.
    fn predict(&self, h: f64) -> (f64, f64) {
        let (a, b) = (self.at[0], self.at[self.at.len() - 1]);
        let span = b.0 - a.0;
        if span < 1e-6 {
            return (b.1, b.2);
        }
        let drift = |from: f64, to: f64| ((to - from) / span).clamp(-0.47, 0.47);
        (
            b.1 + drift(a.1, b.1) * (h - b.0),
            b.2 + drift(a.2, b.2) * (h - b.0),
        )
    }

    /// The axis the circles lie along, as a stem: a least-squares line of each coordinate on the
    /// height.
    fn stem(&self) -> Stem {
        let n = self.at.len() as f64;
        let mean = |f: &dyn Fn(&Circle) -> f64| self.at.iter().map(f).sum::<f64>() / n;
        let (h0, x0, y0) = (mean(&|a| a.0), mean(&|a| a.1), mean(&|a| a.2));
        let var: f64 = self.at.iter().map(|a| (a.0 - h0).powi(2)).sum();
        let drift = |at: &dyn Fn(&Circle) -> f64, m: f64| {
            if var < 1e-9 {
                0.0
            } else {
                (self
                    .at
                    .iter()
                    .map(|a| (a.0 - h0) * (at(a) - m))
                    .sum::<f64>()
                    / var)
                    .clamp(-0.47, 0.47)
            }
        };
        Stem {
            c: [x0, y0],
            h0,
            slope: [drift(&|a| a.1, x0), drift(&|a| a.2, y0)],
            n: self.at.iter().map(|a| a.4).sum(),
            from: self.at[0].0,
            to: self.at[self.at.len() - 1].0,
        }
    }
}

/// The circles of one group of the stripe followed from level to level: at several levels all the
/// circles of the group are searched (several stems that touch are several circles, not one wide
/// one), and the circles of consecutive levels that continue each other make a chain, a stem
/// candidate.
fn chain_candidates(c: &Cloud, ids: &[u32], p: &TlsParams) -> Vec<Chain> {
    let (lo, hi) = (p.stripe.0 + p.slice, p.stripe.1 - p.slice);
    let mut levels = Vec::new();
    let mut h = lo;
    while h <= hi + 1e-9 {
        levels.push(h);
        h += 0.25;
    }
    let mut chains: Vec<Chain> = Vec::new();
    for (li, &h) in levels.iter().enumerate() {
        let pts: Vec<[f32; 2]> = ids
            .iter()
            .filter(|&&i| (f64::from(c.z0[i as usize]) - h).abs() <= p.slice)
            .map(|&i| [c.xyz[i as usize][0], c.xyz[i as usize][1]])
            .collect();
        if pts.len() < 8 {
            continue;
        }
        let n = pts.len() as f64;
        let centre = (
            pts.iter().map(|q| f64::from(q[0])).sum::<f64>() / n,
            pts.iter().map(|q| f64::from(q[1])).sum::<f64>() / n,
        );
        let mut taken = vec![false; chains.len()];
        let fits = circles_in(&pts, centre, f64::INFINITY, p, None, 6);
        for f in fits {
            // the chain whose circle, carried on, is nearest, and of a like size
            let mut best: Option<(usize, f64)> = None;
            for (k, ch) in chains.iter().enumerate() {
                if taken[k] || li - ch.last_level > 2 {
                    continue;
                }
                let (px, py) = ch.predict(h);
                let (away, d_last) = ((f.0 - px).hypot(f.1 - py), ch.at[ch.at.len() - 1].3);
                if away <= 0.06 + 0.3 * f.2.max(d_last)
                    && (0.6..=1.6).contains(&(f.2 / d_last))
                    && best.is_none_or(|b| away < b.1)
                {
                    best = Some((k, away));
                }
            }
            match best {
                Some((k, _)) => {
                    chains[k].at.push((h, f.0, f.1, f.2, f.3));
                    chains[k].last_level = li;
                    taken[k] = true;
                }
                None => {
                    chains.push(Chain {
                        at: vec![(h, f.0, f.1, f.2, f.3)],
                        last_level: li,
                    });
                    taken.push(true);
                }
            }
        }
    }
    chains
}

/// Whether a chain is a stem: it needs a few levels; a branch that leaves a fork inside the
/// stripe may have only two, if it starts right above a circle of a stem that has them (`solid`,
/// the chains of three levels or more, of any group).
fn is_stem(ch: &Chain, solid: &[&Chain]) -> bool {
    ch.at.len() >= 3
        || (ch.at.len() == 2
            && solid.iter().any(|s| {
                s.at.iter().any(|a| {
                    (0.0..=1.0).contains(&(ch.at[0].0 - a.0))
                        && (a.1 - ch.at[0].1).hypot(a.2 - ch.at[0].2) < 0.5
                })
            }))
}

fn find_stems(c: &Cloud, p: &TlsParams) -> Vec<Stem> {
    let mut set: Vec<u32> = (0..c.xyz.len() as u32)
        .filter(|&i| {
            let h = f64::from(c.z0[i as usize]);
            h >= p.stripe.0 && h <= p.stripe.1
        })
        .collect();
    let eps = p.voxel * 2.2;
    let mut labels: Vec<u32> = Vec::new();
    for _ in 0..p.peel.max(1) {
        if set.is_empty() {
            return Vec::new();
        }
        let v = verticality(&c.xyz, &set, p.vert_radius);
        set = set
            .iter()
            .zip(&v)
            .filter(|(_, v)| f64::from(**v) >= p.vert_min)
            .map(|(i, _)| *i)
            .collect();
        let sizes;
        (labels, sizes) = group(&c.xyz, &set, eps);
        let keep: Vec<bool> = sizes.iter().map(|n| *n >= p.min_stem_points).collect();
        let kept: Vec<(u32, u32)> = set
            .iter()
            .zip(&labels)
            .filter(|(_, l)| keep[**l as usize])
            .map(|(i, l)| (*i, *l))
            .collect();
        set = kept.iter().map(|k| k.0).collect();
        labels = kept.iter().map(|k| k.1).collect();
    }
    if set.is_empty() {
        return Vec::new();
    }
    let mut by: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, l) in set.iter().zip(&labels) {
        by.entry(*l).or_default().push(*i);
    }
    let groups: Vec<Vec<u32>> = by
        .into_values()
        .filter(|ids| {
            let (lo, hi) = ids.iter().fold((f64::MAX, f64::MIN), |a, &i| {
                let z = f64::from(c.z0[i as usize]);
                (a.0.min(z), a.1.max(z))
            });
            hi - lo >= p.height_range
        })
        .collect();
    // the circles of every group first: a branch of a fork can be in a group of its own, apart
    // from the stem under it, and what shows it is a branch is the stem that ends just below
    let all: Vec<Vec<Chain>> = groups
        .iter()
        .map(|ids| chain_candidates(c, ids, p))
        .collect();
    let solid: Vec<&Chain> = all.iter().flatten().filter(|ch| ch.at.len() >= 3).collect();
    let mut stems = Vec::new();
    for (ids, chains) in groups.iter().zip(&all) {
        let found: Vec<Stem> = chains
            .iter()
            .filter(|ch| is_stem(ch, &solid))
            .map(Chain::stem)
            .collect();
        if !found.is_empty() {
            stems.extend(found);
            continue;
        }
        // no circle could be followed (stems seen only in arcs): the axis of the whole group
        let pts = ids.iter().map(|&i| {
            let q = c.xyz[i as usize];
            [
                f64::from(q[0]),
                f64::from(q[1]),
                f64::from(c.z0[i as usize]),
            ]
        });
        let (m, cov, n) = covariance(pts.clone());
        let (zmin, zmax) = pts
            .clone()
            .fold((f64::MAX, f64::MIN), |a, q| (a.0.min(q[2]), a.1.max(q[2])));
        let mut d = principal(cov);
        if d[2] < 0.0 {
            d = [-d[0], -d[1], -d[2]];
        }
        if d[2] < 0.7 {
            continue;
        }
        let slope = [d[0] / d[2], d[1] / d[2]];
        // a stem is a thin column around its axis
        let far = pts
            .clone()
            .map(|q| {
                let t = q[2] - m[2];
                ((q[0] - m[0] - slope[0] * t).powi(2) + (q[1] - m[1] - slope[1] * t).powi(2)).sqrt()
            })
            .fold(0.0, f64::max);
        if far > p.max_diam * 0.75 {
            continue;
        }
        stems.push(Stem {
            c: [m[0], m[1]],
            h0: m[2],
            slope,
            n,
            from: zmin,
            to: zmax,
        });
    }
    stems.sort_by(|a, b| a.c[0].total_cmp(&b.c[0]).then(a.c[1].total_cmp(&b.c[1])));
    // two groups of one stem (a gap in the stripe): keep the larger
    let mut out: Vec<Stem> = Vec::new();
    for s in stems {
        let near = out.iter_mut().find(|o| {
            let t = s.h0 - o.h0;
            let (px, py) = (o.c[0] + o.slope[0] * t, o.c[1] + o.slope[1] * t);
            ((px - s.c[0]).powi(2) + (py - s.c[1]).powi(2)).sqrt() < 0.12
        });
        match near {
            Some(o) if s.n > o.n => *o = s,
            Some(_) => {}
            None => out.push(s),
        }
    }
    out
}

// ----------------------------------------------------------------------------- sections

/// Voxels by horizontal cell, sorted by height, to cut slices quickly.
struct SliceIndex {
    cell: f32,
    order: Vec<u32>,
    cells: FxMap<u64, (u32, u32)>,
}

impl SliceIndex {
    fn new(c: &Cloud) -> Self {
        let cell = 0.5f32;
        let keys: Vec<u64> = c
            .xyz
            .iter()
            .map(|q| {
                pack(
                    (q[0] / cell).floor() as i64,
                    (q[1] / cell).floor() as i64,
                    0,
                )
            })
            .collect();
        let mut order: Vec<u32> = (0..c.xyz.len() as u32).collect();
        order.par_sort_unstable_by(|&a, &b| {
            keys[a as usize]
                .cmp(&keys[b as usize])
                .then(c.z0[a as usize].total_cmp(&c.z0[b as usize]))
        });
        let mut cells: FxMap<u64, (u32, u32)> = FxMap::default();
        let mut s = 0usize;
        while s < order.len() {
            let k = keys[order[s] as usize];
            let mut e = s + 1;
            while e < order.len() && keys[order[e] as usize] == k {
                e += 1;
            }
            cells.insert(k, (s as u32, (e - s) as u32));
            s = e;
        }
        Self { cell, order, cells }
    }

    /// The points within `rad` of `(x, y)` and between heights `lo` and `hi`.
    fn slice(&self, c: &Cloud, x: f64, y: f64, rad: f64, lo: f64, hi: f64) -> Vec<[f32; 2]> {
        let cell = f64::from(self.cell);
        let (cx0, cx1) = (
            ((x - rad) / cell).floor() as i64,
            ((x + rad) / cell).floor() as i64,
        );
        let (cy0, cy1) = (
            ((y - rad) / cell).floor() as i64,
            ((y + rad) / cell).floor() as i64,
        );
        let mut out = Vec::new();
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                let Some(&(s, n)) = self.cells.get(&pack(cx, cy, 0)) else {
                    continue;
                };
                let list = &self.order[s as usize..(s + n) as usize];
                let a = list.partition_point(|&i| f64::from(c.z0[i as usize]) < lo);
                for &i in &list[a..] {
                    if f64::from(c.z0[i as usize]) > hi {
                        break;
                    }
                    let q = c.xyz[i as usize];
                    if (f64::from(q[0]) - x).hypot(f64::from(q[1]) - y) <= rad {
                        out.push([q[0], q[1]]);
                    }
                }
            }
        }
        out
    }
}

/// A circle by least squares: algebraic start, then Gauss–Newton on the geometric distances.
fn fit_circle(pts: &[[f32; 2]]) -> Option<(f64, f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let (mx, my) = pts.iter().fold((0.0, 0.0), |a, q| {
        (a.0 + f64::from(q[0]), a.1 + f64::from(q[1]))
    });
    let (mx, my) = (mx / n as f64, my / n as f64);
    let (mut suu, mut suv, mut svv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for q in pts {
        let (u, v) = (f64::from(q[0]) - mx, f64::from(q[1]) - my);
        suu += u * u;
        suv += u * v;
        svv += v * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-12 {
        return None;
    }
    let (b1, b2) = (0.5 * (suuu + suvv), 0.5 * (svvv + svuu));
    let (mut uc, mut vc) = ((b1 * svv - b2 * suv) / det, (b2 * suu - b1 * suv) / det);
    let mut r = (uc * uc + vc * vc + (suu + svv) / n as f64).sqrt();
    for _ in 0..6 {
        let (mut a, mut b, mut c2, mut g1, mut g2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for q in pts {
            let (u, v) = (f64::from(q[0]) - mx - uc, f64::from(q[1]) - my - vc);
            let d = u.hypot(v).max(1e-12);
            let res = d - r;
            let (ju, jv) = (-u / d, -v / d);
            a += ju * ju;
            b += ju * jv;
            c2 += jv * jv;
            g1 += ju * res;
            g2 += jv * res;
        }
        let dd = a * c2 - b * b;
        if dd.abs() < 1e-12 {
            break;
        }
        let (du, dv) = ((c2 * g1 - b * g2) / dd, (a * g2 - b * g1) / dd);
        uc -= du;
        vc -= dv;
        r = pts
            .iter()
            .map(|q| (f64::from(q[0]) - mx - uc).hypot(f64::from(q[1]) - my - vc))
            .sum::<f64>()
            / n as f64;
        if du.abs() + dv.abs() < 1e-6 {
            break;
        }
    }
    (r.is_finite() && uc.is_finite() && vc.is_finite()).then_some((mx + uc, my + vc, r))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Height above ground.
    pub h: f64,
    /// Centre and diameter (relative coordinates until the result is made).
    pub x: f64,
    pub y: f64,
    pub d: f64,
    pub ok: bool,
    pub points: usize,
    pub sectors: usize,
}

/// A circle through three points.
fn circumcircle(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> Option<(f64, f64, f64)> {
    let (ax, ay) = (f64::from(a[0]), f64::from(a[1]));
    let (bx, by) = (f64::from(b[0]) - ax, f64::from(b[1]) - ay);
    let (cx, cy) = (f64::from(c[0]) - ax, f64::from(c[1]) - ay);
    let d = 2.0 * (bx * cy - by * cx);
    if d.abs() < 1e-9 {
        return None;
    }
    let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
    let (ux, uy) = ((cy * b2 - by * c2) / d, (bx * c2 - cx * b2) / d);
    Some((ax + ux, ay + uy, ux.hypot(uy)))
}

/// The circle most of whose circumference is covered, among those through random triples:
/// robust to branches, leaves and neighbours that are in the slice. `tol` is how far from it a
/// point may be; `per_m` the points a full ring has for each metre of circumference. Covering
/// is what counts, not how many points: a wide circle drawn through arcs of several stems has
/// as many points as the ring of one stem, but covers a fraction of its own circumference.
fn ransac_circle(
    pts: &[[f32; 2]],
    rmin: f64,
    rmax: f64,
    guess: (f64, f64),
    reach: f64,
    tol: f64,
    per_m: f64,
) -> Option<(f64, f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let mut state = 0x2545_F491_4F6C_DD1Du64 ^ (n as u64);
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    let (mut best, mut best_cover) = (None, 0.0f64);
    for _ in 0..150 {
        let (i, j, k) = (next(), next(), next());
        if i == j || j == k || i == k {
            continue;
        }
        let Some((cx, cy, r)) = circumcircle(pts[i], pts[j], pts[k]) else {
            continue;
        };
        if r < rmin || r > rmax || (cx - guess.0).hypot(cy - guess.1) > reach {
            continue;
        }
        let inliers = pts
            .iter()
            .filter(|q| ((f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) - r).abs() <= tol)
            .count();
        let cover = inliers as f64 / (std::f64::consts::TAU * r * per_m);
        if inliers >= 6 && cover > best_cover {
            (best, best_cover) = (Some((cx, cy, r)), cover);
        }
    }
    best
}

/// A circle found in a slice: centre, diameter, voxels that agree and sectors they occupy.
type Fit = (f64, f64, f64, usize, usize);

/// The circles of a slice, the best supported first, at most `max`. After each one the points
/// around it are dropped, so that the circles of stems that touch (sprouts of one stool, the
/// branches of a fork) are found one by one instead of as a single wide circle around both.
fn circles_in(
    pts: &[[f32; 2]],
    guess: (f64, f64),
    reach: f64,
    p: &TlsParams,
    prev: Option<f64>,
    max: usize,
) -> Vec<Fit> {
    let tol = p.voxel * 0.75 + 0.004;
    let mut rest = pts.to_vec();
    let mut out = Vec::new();
    for _ in 0..max {
        let Some(f) = circle_at(&rest, guess, reach, p, prev) else {
            break;
        };
        let cut = f.2 / 2.0 + tol * 2.0;
        rest.retain(|q| (f64::from(q[0]) - f.0).hypot(f64::from(q[1]) - f.1) > cut);
        out.push(f);
    }
    out
}

/// How much further (m) to look for a stem that has not been measured for `gap` metres: the
/// drift it was carried on with is less sure the longer the stretch.
fn widening(gap: f64) -> f64 {
    0.06 * gap.clamp(0.0, 3.0)
}

/// The circle of a slice nearest to where the stem is expected, if the slice holds a stem
/// section. `prev` is the diameter of the section before, if any: the number of points and
/// sectors asked for follow the size of the stem. Choosing the nearest circle, and not the best
/// supported, is what keeps a stem from jumping to a thicker neighbour a few centimetres away.
fn measure(
    pts: &[[f32; 2]],
    guess: (f64, f64),
    p: &TlsParams,
    prev: Option<f64>,
    gap: f64,
) -> Option<Fit> {
    let d_ref = prev.unwrap_or(0.3);
    // the stem may be further from the carried-on guess the longer it has not been seen
    let reach = (d_ref * 1.2 + 0.15).max(0.3) + widening(gap);
    let away = |f: &Fit| (f.0 - guess.0).hypot(f.1 - guess.1);
    circles_in(pts, guess, reach, p, prev, 3)
        .into_iter()
        .min_by(|a, b| away(a).total_cmp(&away(b)))
}

/// Fits and checks one circle of a slice within `reach` of `guess`; `None` when there is none
/// that is a stem section.
fn circle_at(
    pts: &[[f32; 2]],
    guess: (f64, f64),
    reach: f64,
    p: &TlsParams,
    prev: Option<f64>,
) -> Option<Fit> {
    let d_ref = prev.unwrap_or(0.3);
    // what a stem of this size gives in a slice of this thickness, at this voxel
    let expected =
        0.5 * (std::f64::consts::PI * d_ref / p.voxel) * (2.0 * p.slice / p.voxel).max(1.0);
    let min_pts = (p.min_section_points as f64).min(expected).max(8.0) as usize;
    let min_sectors = if d_ref < 0.15 {
        p.min_sectors.min(7)
    } else {
        p.min_sectors
    };
    if pts.len() < min_pts {
        return None;
    }
    let tol = p.voxel * 0.75 + 0.004;
    // points per metre of a full ring: the layers of voxels in the slice, over the voxel
    let per_m = (2.0 * p.slice / p.voxel).max(1.0) / p.voxel;
    let (cx, cy, r) = ransac_circle(
        pts,
        p.min_diam / 2.0,
        p.max_diam / 2.0,
        guess,
        reach,
        tol,
        per_m,
    )?;
    // refine on the points that agree, a few rounds, with a tolerance that follows the data
    let mut kept: Vec<[f32; 2]> = pts
        .iter()
        .copied()
        .filter(|q| ((f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) - r).abs() <= tol * 1.5)
        .collect();
    let (mut cx, mut cy, mut r) = fit_circle(&kept)?;
    for _ in 0..3 {
        let t = (2.5 * rms_residual(&kept, cx, cy, r)).max(tol * 0.6);
        let next: Vec<[f32; 2]> = pts
            .iter()
            .copied()
            .filter(|q| ((f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) - r).abs() <= t)
            .collect();
        if next.len() < min_pts {
            return None;
        }
        let (a, b, c) = fit_circle(&next)?;
        (cx, cy, r) = (a, b, c);
        kept = next;
    }
    if r * 2.0 < p.min_diam || r * 2.0 > p.max_diam {
        return None;
    }
    // the residual allowed grows with the stem, but only so far: a wide circle around two
    // stems that touch has a large residual that 10 % of its radius would let through
    let floor = p.voxel * 0.6;
    if rms_residual(&kept, cx, cy, r) > (0.1 * r).clamp(floor, floor + 0.02) {
        return None;
    }
    let mut occupied = [false; 16];
    for q in &kept {
        let a = (f64::from(q[1]) - cy).atan2(f64::from(q[0]) - cx);
        occupied[(((a + std::f64::consts::PI) / std::f64::consts::TAU * 16.0) as usize).min(15)] =
            true;
    }
    let sectors = occupied.iter().filter(|o| **o).count();
    // a stem is a surface: little of the slice lies inside it. A circle around two stems that
    // touch, or around a branch with foliage, has voxels in its inner circle.
    let inner = pts
        .iter()
        .filter(|q| (f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) < r * 0.5)
        .count();
    (sectors >= min_sectors && inner <= p.max_inner).then_some((
        cx,
        cy,
        2.0 * r,
        kept.len(),
        sectors,
    ))
}

fn rms_residual(pts: &[[f32; 2]], cx: f64, cy: f64, r: f64) -> f64 {
    if pts.is_empty() {
        return 0.0;
    }
    (pts.iter()
        .map(|q| ((f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) - r).powi(2))
        .sum::<f64>()
        / pts.len() as f64)
        .sqrt()
}

/// Sections of one stem, from the base height to where the stem is lost or thinner than the
/// top diameter, `step` metres apart.
fn track(c: &Cloud, idx: &SliceIndex, stem: &Stem, p: &TlsParams, step: f64) -> Vec<Section> {
    let at_axis = |h: f64| {
        let t = h - stem.h0;
        (stem.c[0] + stem.slope[0] * t, stem.c[1] + stem.slope[1] * t)
    };
    let slice_at =
        |x: f64, y: f64, rad: f64, h: f64| idx.slice(c, x, y, rad, h - p.slice, h + p.slice);
    // start at breast height, or the nearest height of the stripe that gives a section
    let j_of = |h: f64| ((h - p.base_height) / step).round() as i64;
    let h_of = |j: i64| p.base_height + j as f64 * step;
    let mut starts: Vec<f64> = vec![1.3, 1.5, 1.1, 1.7, 0.9, 1.9, 2.2, 2.6, 3.0];
    starts.retain(|h| *h >= p.stripe.0 && *h <= p.stripe.1);
    if starts.is_empty() {
        starts.push((p.stripe.0 + p.stripe.1) / 2.0);
    }
    let rad0 = p.max_diam / 2.0 * 1.2 + 0.05;
    let mut first = None;
    for h in starts {
        let j = j_of(h);
        let hh = h_of(j);
        let (ax, ay) = at_axis(hh);
        let pts = slice_at(ax, ay, rad0, hh);
        if let Some(m) = measure(&pts, (ax, ay), p, None, 0.0) {
            first = Some((j, m));
            break;
        }
    }
    let Some((j0, m0)) = first else {
        return Vec::new();
    };
    let mk = |h: f64, m: Option<(f64, f64, f64, usize, usize)>, fallback: (f64, f64)| match m {
        Some((x, y, d, n, s)) => Section {
            h,
            x,
            y,
            d,
            ok: true,
            points: n,
            sectors: s,
        },
        None => Section {
            h,
            x: fallback.0,
            y: fallback.1,
            d: f64::NAN,
            ok: false,
            points: 0,
            sectors: 0,
        },
    };
    let mut up: Vec<Section> = vec![mk(h_of(j0), Some(m0), (0.0, 0.0))];
    // upwards, then downwards, with the same rules
    for dir in [1i64, -1i64] {
        let mut chain: Vec<Section> = vec![up[0].clone()];
        let mut fails = 0;
        let mut thin = 0;
        let mut j = j0;
        let mut slope = stem.slope;
        loop {
            j += dir;
            let h = h_of(j);
            if (dir == 1 && h > p.max_height) || (dir == -1 && j < 0) {
                break;
            }
            let last_ok = chain
                .iter()
                .rev()
                .find(|s| s.ok)
                .cloned()
                .unwrap_or_else(|| up[0].clone());
            let dh = h - last_ok.h;
            // where the stem was measured, its own axis; elsewhere, the last section carried on
            let (px, py) = if (stem.from - 0.3..=stem.to + 0.3).contains(&h) {
                at_axis(h)
            } else {
                (last_ok.x + slope[0] * dh, last_ok.y + slope[1] * dh)
            };
            // the stretch since the last section measured: the search widens with it
            let gap = dh.abs();
            let rad = (last_ok.d * 0.75 + 0.1 + widening(gap)).min(p.max_diam);
            let pts = slice_at(px, py, rad, h);
            let mut m = measure(&pts, (px, py), p, Some(last_ok.d), gap);
            // a sudden change of size or position is not a stem section
            if let Some((x, y, d, ..)) = m {
                let jump = (x - px).hypot(y - py) > 0.15 + 0.5 * last_ok.d + widening(gap);
                let size = (d - last_ok.d).abs() > 0.3 * last_ok.d + 0.03;
                if jump || size {
                    m = None;
                }
            }
            match m {
                Some((x, y, d, ..)) => {
                    fails = 0;
                    if dh.abs() > 1e-9 {
                        let s = [(x - last_ok.x) / dh, (y - last_ok.y) / dh];
                        // blend: the drift of a stem changes slowly, and stays below 25 degrees
                        for k in 0..2 {
                            slope[k] = (0.5 * slope[k] + 0.5 * s[k]).clamp(-0.47, 0.47);
                        }
                    }
                    thin = if d < p.top_diam * 0.85 { thin + 1 } else { 0 };
                }
                None => fails += 1,
            }
            chain.push(mk(h, m, (px, py)));
            // lost: the stem has gone unmeasured for the longest stretch allowed (downwards the
            // ground is near and a longer one is worth trying), or it is as thin as the top
            if dir == 1 && (gap >= p.max_gap - 1e-9 && fails > 0 || thin >= 2) {
                break;
            }
            if dir == -1 && gap >= p.max_gap.max(2.0) - 1e-9 && fails > 0 {
                break;
            }
        }
        up.extend(chain.into_iter().skip(1));
    }
    up.sort_by(|a, b| a.h.total_cmp(&b.h));
    reject_outliers(&mut up);
    // trailing failures carry no information
    while up.last().is_some_and(|s| !s.ok) {
        up.pop();
    }
    up
}

/// Sections that disagree with both neighbours are not stem sections. They lose their diameter
/// as well as their `ok`: a diameter left on a rejected section would pass for a measurement
/// wherever the gap fill cannot replace it (a gap of more than a metre between the sections
/// around it), and the cubing would take it in.
fn reject_outliers(secs: &mut [Section]) {
    let ok_idx: Vec<usize> = (0..secs.len()).filter(|&i| secs[i].ok).collect();
    let mut rejected = Vec::new();
    // every comparison reads the diameters as measured, so the rejections wait
    for w in 1..ok_idx.len().saturating_sub(1) {
        let (a, b, n) = (ok_idx[w - 1], ok_idx[w], ok_idx[w + 1]);
        let mean = (secs[a].d + secs[n].d) / 2.0;
        if (secs[b].d - mean).abs() > 0.2 * mean + 0.02
            && (secs[b].d - secs[a].d).abs() > 0.1 * secs[a].d
        {
            rejected.push(b);
        }
    }
    for b in rejected {
        secs[b].ok = false;
        secs[b].d = f64::NAN;
    }
}

/// A stem narrows upwards. From 1 m up (below, the stem may flare towards the root) the sections
/// kept make an envelope that never rises: the smallest median of three consecutive sections
/// kept so far. A section wider than it (by more than 8 % and 1 cm) has caught a branch, a
/// neighbouring stem or a fork, and loses its diameter; being a floor of medians, neither one
/// thin section nor a slow creep of wide ones can move the envelope. Returns how many sections
/// were dropped.
fn enforce_taper(secs: &mut [Section]) -> usize {
    let mut last: Vec<f64> = Vec::new();
    let mut envelope = f64::INFINITY;
    let mut dropped = 0;
    for s in secs.iter_mut().filter(|s| s.ok && s.h >= 1.0) {
        if s.d > envelope * 1.08 + 0.01 {
            s.ok = false;
            s.d = f64::NAN;
            dropped += 1;
            continue;
        }
        last.push(s.d);
        if last.len() > 3 {
            last.remove(0);
        }
        if last.len() >= 2 {
            let mut w = last.clone();
            w.sort_by(f64::total_cmp);
            envelope = envelope.min(w[w.len() / 2]);
        }
    }
    dropped
}

// ----------------------------------------------------------------------------- cubing

#[derive(Clone, Debug, PartialEq)]
pub struct Log {
    pub from: f64,
    pub to: f64,
    pub d_from: f64,
    pub d_to: f64,
    /// Smalian's volume, m³.
    pub volume: f64,
    /// 0 for the stem, 1 and up for each branch above a fork.
    pub branch: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cubing {
    pub logs: Vec<Log>,
    pub volume: f64,
    /// Height where the stem reaches the top diameter, or where it is lost.
    pub top_h: f64,
    /// The stem was lost before it reached the top diameter.
    pub truncated: bool,
    /// The stem was lost, and the stretch from there to the top diameter is an estimate.
    pub estimated: bool,
}

fn area(d: f64) -> f64 {
    std::f64::consts::PI * d * d / 4.0
}

/// Diameter at height `h` interpolated linearly in `profile` (ascending heights).
pub(crate) fn diam_at(profile: &[(f64, f64)], h: f64) -> Option<f64> {
    let (first, last) = (profile.first()?, profile.last()?);
    if h < first.0 - 1e-9 || h > last.0 + 1e-9 {
        return None;
    }
    let k = profile.partition_point(|q| q.0 < h);
    if k == 0 {
        return Some(first.1);
    }
    let (a, b) = (profile[k - 1], profile[k.min(profile.len() - 1)]);
    if (b.0 - a.0).abs() < 1e-12 {
        return Some(b.1);
    }
    Some(a.1 + (b.1 - a.1) * (h - a.0) / (b.0 - a.0))
}

/// Smalian's cubing of a stem from its diameter `profile` ((height, diameter), ascending): logs
/// of `log_len` from `base` up, the last one cut where the diameter falls to `top_d`.
pub fn smalian(profile: &[(f64, f64)], base: f64, log_len: f64, top_d: f64) -> Cubing {
    let mut out = Cubing::default();
    let Some(&(h_end, _)) = profile.last() else {
        return out;
    };
    // below the lowest section the diameter is taken as the lowest one's
    let first = profile[0];
    let pf: Vec<(f64, f64)> = if first.0 > base {
        std::iter::once((base, first.1))
            .chain(profile.iter().copied())
            .collect()
    } else {
        profile.to_vec()
    };
    let Some(mut d) = diam_at(&pf, base) else {
        return out;
    };
    if d < top_d {
        return out;
    }
    let mut h = base;
    out.top_h = base;
    loop {
        let hn = (h + log_len).min(h_end);
        let dn = diam_at(&pf, hn).unwrap_or(d);
        if dn >= top_d {
            let l = hn - h;
            if l > 1e-9 {
                let v = (area(d) + area(dn)) / 2.0 * l;
                out.logs.push(Log {
                    from: h,
                    to: hn,
                    d_from: d,
                    d_to: dn,
                    volume: v,
                    branch: 0,
                });
                out.volume += v;
            }
            out.top_h = hn;
            if hn >= h_end - 1e-9 {
                out.truncated = true;
                break;
            }
            (h, d) = (hn, dn);
            continue;
        }
        // the diameter reaches the top diameter inside this log: find where
        let mut cut = hn;
        let mut prev = (h, d);
        for &(ph, pd) in pf.iter().filter(|q| q.0 > h) {
            if pd < top_d {
                cut = prev.0 + (prev.1 - top_d) / (prev.1 - pd).max(1e-12) * (ph - prev.0);
                break;
            }
            prev = (ph, pd);
        }
        let l = cut - h;
        if l > 1e-9 {
            let v = (area(d) + area(top_d)) / 2.0 * l;
            out.logs.push(Log {
                from: h,
                to: cut,
                d_from: d,
                d_to: top_d,
                volume: v,
                branch: 0,
            });
            out.volume += v;
        }
        out.top_h = cut;
        break;
    }
    out
}

// ----------------------------------------------------------------------------- the result

#[derive(Clone, Debug, PartialEq)]
pub struct TreeResult {
    pub id: u32,
    /// Position of the stem at the base, and the ground there (map coordinates).
    pub x: f64,
    pub y: f64,
    pub ground_z: f64,
    /// Diameter at 1.3 m, m.
    pub dbh: Option<f64>,
    pub height: f64,
    /// Inclination of the stem axis from the vertical, degrees.
    pub tilt: f64,
    pub cubing: Cubing,
    pub sections: Vec<Section>,
    pub points: usize,
    /// Share of the sections along the measured length that passed the checks.
    pub quality: f64,
    pub d_base: f64,
    pub flags: String,
    /// Axis through (x, y) at the base with this horizontal drift per metre (for clouds by tree).
    pub slope: [f64; 2],
    /// The stool this stem grows on (the same number for the sprouts of one stool) and how many
    /// sprouts it has: 1 for a stem on its own.
    pub stool: u32,
    pub sprouts: u32,
    /// Height of the fork, for a tree that forks above the height of the diameter (1.3 m): its
    /// stem below is one, and each branch above is cubed apart.
    pub fork_h: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct TlsResult {
    pub trees: Vec<TreeResult>,
    pub date: Option<String>,
    pub epsg: Option<u16>,
    pub points_read: u64,
    pub voxels: usize,
    origin: [f64; 3],
}

/// The tallest point of the stem's own cluster: the voxels of 0.3 m within 1.5 m of the stem's
/// axis, extended upwards with the stem's lean, that chain to the top of the measured stem.
fn tree_height(c: &Cloud, idx: &SliceIndex, secs: &[Section], slope: [f64; 2]) -> f64 {
    let Some(top) = secs.iter().rev().find(|s| s.ok) else {
        return 0.0;
    };
    let mut vox: FxMap<u64, ((i64, i64, i64), f64)> = FxMap::default();
    for (z, x, y) in idx.slice_all(c, top.x, top.y, 8.0) {
        let t = z - top.h;
        let (ax, ay) = (top.x + slope[0] * t, top.y + slope[1] * t);
        if (x - ax).hypot(y - ay) > 1.5 || z < 0.5 {
            continue;
        }
        let k = (
            (x / 0.3).floor() as i64,
            (y / 0.3).floor() as i64,
            (z / 0.3).floor() as i64,
        );
        let e = vox.entry(pack(k.0, k.1, k.2)).or_insert((k, z));
        e.1 = e.1.max(z);
    }
    let start = (
        (top.x / 0.3).floor() as i64,
        (top.y / 0.3).floor() as i64,
        (top.h / 0.3).floor() as i64,
    );
    let mut best = top.h;
    let mut seen: FxMap<u64, ()> = FxMap::default();
    let mut stack = vec![start];
    seen.insert(pack(start.0, start.1, start.2), ());
    while let Some((a, b, d)) = stack.pop() {
        if let Some((_, z)) = vox.get(&pack(a, b, d)) {
            best = best.max(*z);
        }
        for da in -1..=1 {
            for db in -1..=1 {
                for dd in -1..=1 {
                    let k = pack(a + da, b + db, d + dd);
                    if vox.contains_key(&k) && seen.insert(k, ()).is_none() {
                        stack.push((a + da, b + db, d + dd));
                    }
                }
            }
        }
    }
    best
}

impl SliceIndex {
    /// (height, x, y) of every point within `rad` of `(x, y)`, whatever its height.
    fn slice_all(&self, c: &Cloud, x: f64, y: f64, rad: f64) -> Vec<(f64, f64, f64)> {
        let cell = f64::from(self.cell);
        let mut out = Vec::new();
        for cx in ((x - rad) / cell).floor() as i64..=((x + rad) / cell).floor() as i64 {
            for cy in ((y - rad) / cell).floor() as i64..=((y + rad) / cell).floor() as i64 {
                if let Some(&(s, n)) = self.cells.get(&pack(cx, cy, 0)) {
                    for &i in &self.order[s as usize..(s + n) as usize] {
                        let q = c.xyz[i as usize];
                        if (f64::from(q[0]) - x).hypot(f64::from(q[1]) - y) <= rad {
                            out.push((
                                f64::from(c.z0[i as usize]),
                                f64::from(q[0]),
                                f64::from(q[1]),
                            ));
                        }
                    }
                }
            }
        }
        out
    }
}

/// Measures and cubes the trees of one cloud.
pub fn analyse(input: &Path, p: &TlsParams) -> Result<TlsResult> {
    p.check()?;
    let mut c = load(input, p)?;
    crate::check_cancel()?;
    let ground = normalize(&mut c, p)?;
    crate::check_cancel()?;
    let stems = find_stems(&c, p);
    crate::check_cancel()?;
    let idx = SliceIndex::new(&c);
    let n_sub = (p.log_len / 0.25).ceil().max(1.0);
    let step = p.log_len / n_sub;
    let measured: Vec<Option<TreeResult>> = stems
        .par_iter()
        .map(|stem| {
            if crate::is_cancelled() {
                return None;
            }
            let secs = track(&c, &idx, stem, p, step);
            if secs.iter().filter(|s| s.ok).count() < 2 {
                return None;
            }
            Some(build_tree(&c, &idx, ground.as_ref(), stem, secs, p))
        })
        .collect();
    crate::check_cancel()?;
    let mut trees = crate::stools::resolve(measured.into_iter().flatten().collect(), p);
    if p.lost_top_share > 0.0 {
        estimate_lost_tops(&mut trees, p);
    }
    trees.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    // numbers: the trees in order, and the stools in the order their first tree comes
    let mut stools: HashMap<u32, u32> = HashMap::new();
    for (k, t) in trees.iter_mut().enumerate() {
        t.id = k as u32 + 1;
        let next = stools.len() as u32 + 1;
        t.stool = *stools.entry(t.stool).or_insert(next);
    }
    Ok(TlsResult {
        trees,
        date: c.date.clone(),
        epsg: c.epsg,
        points_read: c.read,
        voxels: c.xyz.len(),
        origin: c.origin,
    })
}

/// A stem that was lost while still thicker than the top diameter, well below the top of the
/// tree, was not measured to its end: something (foliage, a branch whorl, an occlusion) hid it,
/// because a stem does not stop being cubable at 8 cm and 20 m of height. Its merchantable
/// height is then estimated as `p.lost_top_share` of the height of the tree (what the user
/// expects of the species, for instance 0.65 for eucalyptus), and the diameter goes down linearly
/// from the last section to the top diameter there. Forked trees are left as they are. The trees
/// get the flag `ESTIMADO` instead of `TRUNCADO`.
fn estimate_lost_tops(trees: &mut [TreeResult], p: &TlsParams) {
    let profile_of = |t: &TreeResult| -> Vec<(f64, f64)> {
        t.sections
            .iter()
            .filter(|s| s.ok || s.d.is_finite())
            .map(|s| (s.h, s.d))
            .collect()
    };
    let ratio = p.lost_top_share.clamp(0.1, 0.95);
    for t in trees.iter_mut() {
        if !t.cubing.truncated || t.fork_h.is_some() {
            continue;
        }
        let mut profile = profile_of(t);
        let Some(&(last_h, last_d)) = profile.last() else {
            continue;
        };
        let h_com = ratio * t.height;
        // lost near the top of the tree it really is the end; lost short of the estimate, keep it
        if t.height - last_h <= 1.0 || h_com <= last_h + p.log_len || last_d < p.top_diam {
            continue;
        }
        profile.push((h_com, p.top_diam));
        profile.push((h_com + 0.05, p.top_diam * 0.9));
        let mut cubing = smalian(&profile, p.base_height, p.log_len, p.top_diam);
        cubing.estimated = true;
        cubing.truncated = false;
        t.cubing = cubing;
        let flags: Vec<&str> = t
            .flags
            .split(' ')
            .filter(|f| !f.is_empty() && *f != "TRUNCADO")
            .chain(std::iter::once("ESTIMADO"))
            .collect();
        t.flags = flags.join(" ");
    }
}

fn build_tree(
    c: &Cloud,
    idx: &SliceIndex,
    ground: Option<&Grid>,
    stem: &Stem,
    mut secs: Vec<Section>,
    p: &TlsParams,
) -> TreeResult {
    let mut flags = Vec::new();
    if enforce_taper(&mut secs) > 0 {
        flags.push("ENSANCHA");
    }
    // trailing sections left without a diameter carry no information
    while secs.last().is_some_and(|s| !s.ok) {
        secs.pop();
    }
    // short gaps are filled from the sections around them
    let ok: Vec<(f64, f64)> = secs.iter().filter(|s| s.ok).map(|s| (s.h, s.d)).collect();
    let fine = secs.windows(2).next().map_or(0.25, |w| w[1].h - w[0].h);
    let mut filled = 0;
    for s in &mut secs {
        if !s.ok {
            let k = ok.partition_point(|q| q.0 < s.h);
            if k > 0 && k < ok.len() && ok[k].0 - ok[k - 1].0 <= 1.0 + 1e-9 {
                s.d = diam_at(&ok, s.h).unwrap_or(f64::NAN);
                filled += 1;
            }
        }
    }
    let profile: Vec<(f64, f64)> = secs
        .iter()
        .filter(|s| s.ok || s.d.is_finite())
        .map(|s| (s.h, s.d))
        .collect();
    let cubing = smalian(&profile, p.base_height, p.log_len, p.top_diam);
    let dbh =
        diam_at(&profile, 1.3).filter(|_| secs.iter().any(|s| s.ok && (s.h - 1.3).abs() <= 0.35));
    if dbh.is_none() {
        flags.push("SIN_DAP");
    }
    // a profile that still widens above breast height (the envelope starts on the sections it
    // finds, which may be the wrong ones) is left to be checked by eye
    if !flags.contains(&"ENSANCHA")
        && dbh.is_some_and(|d| profile.iter().any(|&(h, x)| h > 1.3 && x > d * 1.2 + 0.01))
    {
        flags.push("ENSANCHA");
    }
    if cubing.truncated {
        flags.push("TRUNCADO");
    }
    if cubing.logs.is_empty() {
        flags.push("SIN_VOLUMEN");
    }
    if filled > 0 {
        flags.push("INTERPOLADO");
    }
    let first_ok = secs.iter().find(|s| s.ok);
    let last_ok = secs.iter().rev().find(|s| s.ok);
    let (tilt, slope) = match (first_ok, last_ok) {
        (Some(a), Some(b)) if b.h - a.h > 0.5 => {
            let s = [(b.x - a.x) / (b.h - a.h), (b.y - a.y) / (b.h - a.h)];
            (s[0].hypot(s[1]).atan().to_degrees(), s)
        }
        _ => (
            stem.slope[0].hypot(stem.slope[1]).atan().to_degrees(),
            stem.slope,
        ),
    };
    let height = tree_height(c, idx, &secs, slope);
    if tilt > 25.0 {
        flags.push("INCLINADO");
    }
    let (bx, by) = first_ok.map_or((stem.c[0], stem.c[1]), |s| {
        (
            s.x - slope[0] * (s.h - p.base_height),
            s.y - slope[1] * (s.h - p.base_height),
        )
    });
    let gz = ground.and_then(|g| g.sample(bx, by)).map_or(0.0, f64::from);
    let span = last_ok.map_or(0.0, |l| l.h) - p.base_height;
    let total = (span / fine.max(1e-6)).round().max(1.0);
    let good = secs.iter().filter(|s| s.ok).count() as f64;
    TreeResult {
        id: 0,
        x: bx,
        y: by,
        ground_z: gz,
        dbh,
        height,
        tilt,
        d_base: diam_at(&profile, p.base_height).unwrap_or(f64::NAN),
        points: secs.iter().map(|s| s.points).sum(),
        quality: (good / total).min(1.0) * 100.0,
        cubing,
        sections: secs,
        flags: flags.join(" "),
        slope,
        stool: 0,
        sprouts: 1,
        fork_h: None,
    }
}

// ----------------------------------------------------------------------------- outputs

impl TlsResult {
    fn abs(&self, t: &TreeResult) -> (f64, f64, f64) {
        (
            t.x + self.origin[0],
            t.y + self.origin[1],
            t.ground_z + self.origin[2],
        )
    }
}

fn tree_table(r: &TlsResult, p: &TlsParams) -> (Vec<[f64; 3]>, Table) {
    let cols: Vec<(String, ColKind)> = vec![
        ("TreeID".into(), ColKind::Num(6, 0)),
        ("X".into(), ColKind::Num(12, 3)),
        ("Y".into(), ColKind::Num(13, 3)),
        ("Z_SUELO".into(), ColKind::Num(9, 2)),
        ("DAP_CM".into(), ColKind::Num(7, 1)),
        ("ALT_TOT".into(), ColKind::Num(7, 2)),
        ("ALT_COM".into(), ColKind::Num(7, 2)),
        ("VOL_M3".into(), ColKind::Num(9, 4)),
        ("NTROZAS".into(), ColKind::Num(4, 0)),
        ("D_BASE_CM".into(), ColKind::Num(7, 1)),
        ("D_TOP_CM".into(), ColKind::Num(7, 1)),
        ("AB_M2".into(), ColKind::Num(8, 4)),
        ("INCLIN".into(), ColKind::Num(5, 1)),
        ("PUNTOS".into(), ColKind::Num(8, 0)),
        ("CALIDAD".into(), ColKind::Num(5, 0)),
        ("AVISOS".into(), ColKind::Text(40)),
        (crate::vector::CAPTURE_FIELD.into(), ColKind::Text(10)),
        ("CEPA".into(), ColKind::Num(6, 0)),
        ("BROTES".into(), ColKind::Num(3, 0)),
        ("H_BIF".into(), ColKind::Num(6, 2)),
    ];
    let mut pts = Vec::new();
    let mut rows = Vec::new();
    for t in &r.trees {
        let (x, y, gz) = r.abs(t);
        pts.push([x, y, gz]);
        let dbh = t
            .dbh
            .map_or(Val::Text(String::new()), |d| Val::Num(d * 100.0));
        let ab = t
            .dbh
            .map_or(Val::Text(String::new()), |d| Val::Num(area(d)));
        let d_top = t.cubing.logs.last().map_or(f64::NAN, |l| l.d_to);
        let n = |v: f64| {
            if v.is_finite() {
                Val::Num(v)
            } else {
                Val::Text(String::new())
            }
        };
        rows.push(vec![
            Val::Int(i64::from(t.id)),
            Val::Num(x),
            Val::Num(y),
            Val::Num(gz),
            dbh,
            Val::Num(t.height),
            Val::Num((t.cubing.top_h - p.base_height).max(0.0)),
            Val::Num(t.cubing.volume),
            Val::Int(t.cubing.logs.len() as i64),
            n(t.d_base * 100.0),
            n(d_top * 100.0),
            ab,
            Val::Num(t.tilt),
            Val::Int(t.points as i64),
            Val::Num(t.quality.round()),
            Val::Text(t.flags.clone()),
            Val::Text(r.date.clone().unwrap_or_default()),
            Val::Int(i64::from(t.stool)),
            Val::Int(i64::from(t.sprouts)),
            t.fork_h.map_or(Val::Text(String::new()), Val::Num),
        ]);
    }
    (
        pts,
        Table {
            columns: cols,
            rows,
        },
    )
}

fn csv_text(t: &Table) -> String {
    let mut s = t
        .columns
        .iter()
        .map(|c| c.0.clone())
        .collect::<Vec<_>>()
        .join(",");
    s.push('\n');
    for row in &t.rows {
        let cells: Vec<String> = row
            .iter()
            .map(|v| match v {
                Val::Text(x) => x.clone(),
                Val::Num(x) => format!("{x:.4}"),
                Val::Int(x) => x.to_string(),
            })
            .collect();
        s.push_str(&cells.join(","));
        s.push('\n');
    }
    s
}

/// Writes the trees as a point shapefile (at the base of each stem) and/or a CSV, and the logs
/// of every tree as `<name>_trozas.csv`. Returns the files.
pub fn write_results(
    r: &TlsResult,
    p: &TlsParams,
    out: &Path,
    formats: crate::paths::Formats,
) -> Result<Vec<PathBuf>> {
    crate::paths::ensure_parent(out)?;
    let (pts, table) = tree_table(r, p);
    let mut files = Vec::new();
    if formats.vector {
        let shp = crate::paths::swap_ext(out, "shp");
        let prj = r.epsg.and_then(crate::vector::prj_for_epsg);
        crate::vector::write_points(&shp, &pts, true, &table, prj.as_deref())?;
        files.push(shp);
    }
    if formats.csv {
        let csv = crate::paths::swap_ext(out, "csv");
        std::fs::write(&csv, csv_text(&table))?;
        files.push(csv);
    }
    // the logs
    let mut logs = Table {
        columns: vec![
            ("TreeID".into(), ColKind::Num(6, 0)),
            ("Troza".into(), ColKind::Num(4, 0)),
            ("H_DESDE".into(), ColKind::Num(7, 2)),
            ("H_HASTA".into(), ColKind::Num(7, 2)),
            ("D_DESDE_CM".into(), ColKind::Num(7, 1)),
            ("D_HASTA_CM".into(), ColKind::Num(7, 1)),
            ("LONG_M".into(), ColKind::Num(6, 2)),
            ("VOL_M3".into(), ColKind::Num(9, 5)),
            ("RAMA".into(), ColKind::Num(3, 0)),
        ],
        rows: Vec::new(),
    };
    for t in &r.trees {
        for (k, l) in t.cubing.logs.iter().enumerate() {
            logs.rows.push(vec![
                Val::Int(i64::from(t.id)),
                Val::Int(k as i64 + 1),
                Val::Num(l.from),
                Val::Num(l.to),
                Val::Num(l.d_from * 100.0),
                Val::Num(l.d_to * 100.0),
                Val::Num(l.to - l.from),
                Val::Num(l.volume),
                Val::Int(i64::from(l.branch)),
            ]);
        }
    }
    let lp = out.with_file_name(format!("{}_trozas.csv", crate::paths::stem(out)));
    std::fs::write(&lp, csv_text(&logs))?;
    files.push(lp);
    Ok(files)
}

/// Clouds by tree written in one read of the input.
const OPEN_CLOUDS: usize = 150;

/// Gives every point of the cloud to its nearest stem axis (within `assign_dist`, above the
/// ground) and writes one cloud per tree into `dir`; returns how many clouds and points.
pub fn write_tree_clouds(
    input: &Path,
    r: &TlsResult,
    p: &TlsParams,
    dir: &Path,
) -> Result<(usize, u64)> {
    if r.trees.is_empty() {
        return Ok((0, 0));
    }
    std::fs::create_dir_all(dir)?;
    let template = Reader::open(input)?;
    let ext = input
        .extension()
        .map_or("las".to_string(), |e| e.to_string_lossy().into_owned());
    let o = r.origin;
    let cell = p.assign_dist.max(1.0);
    let mut grid: FxMap<u64, Vec<usize>> = FxMap::default();
    for (k, t) in r.trees.iter().enumerate() {
        grid.entry(pack(
            (t.x / cell).floor() as i64,
            (t.y / cell).floor() as i64,
            0,
        ))
        .or_default()
        .push(k);
    }
    let rec_len = usize::from(template.header.record_len);
    let mut total = 0u64;
    let mut n = 0;
    // a file per tree is open while the cloud is read; the system limits how many files a
    // program has open (256 on macOS), so the trees go in batches, one read of the cloud each
    for lo in (0..r.trees.len()).step_by(OPEN_CLOUDS) {
        let hi = (lo + OPEN_CLOUDS).min(r.trees.len());
        let mut writers: HashMap<usize, Writer> = HashMap::new();
        template.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let Some(pt) = template.decode(rec) else {
                    continue;
                };
                if pt.is_noise() {
                    continue;
                }
                let (x, y, z) = (pt.x - o[0], pt.y - o[1], pt.z);
                let (cx, cy) = ((x / cell).floor() as i64, (y / cell).floor() as i64);
                let mut best: Option<(f64, usize)> = None;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        let Some(list) = grid.get(&pack(cx + dx, cy + dy, 0)) else {
                            continue;
                        };
                        for &k in list {
                            let t = &r.trees[k];
                            let gz = t.ground_z + o[2];
                            let h = z - gz;
                            if h < 0.2 {
                                continue;
                            }
                            // distance to the axis at this height
                            let (ax, ay) = (
                                t.x + t.slope[0] * (h - p.base_height),
                                t.y + t.slope[1] * (h - p.base_height),
                            );
                            let d = (x - ax).hypot(y - ay);
                            if d <= p.assign_dist && best.is_none_or(|b| d < b.0) {
                                best = Some((d, k));
                            }
                        }
                    }
                }
                if let Some((_, k)) = best
                    && (lo..hi).contains(&k)
                {
                    if let std::collections::hash_map::Entry::Vacant(v) = writers.entry(k) {
                        let path = dir.join(format!("arbol_{:04}.{ext}", r.trees[k].id));
                        v.insert(Writer::create_with(
                            &path,
                            &template,
                            &WriterOptions::small(),
                        )?);
                    }
                    if let Some(w) = writers.get_mut(&k) {
                        w.write(rec)?;
                        total += 1;
                    }
                }
            }
            Ok(())
        })?;
        n += writers.len();
        for (_, w) in writers {
            w.finish()?;
        }
    }
    Ok((n, total))
}

/// How many points `classify_stems` gave to the stems, to the branches and left as they were.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StemReport {
    pub stem: u64,
    pub branch: u64,
    pub other: u64,
}

/// The measured stem of one tree as a function of height: the centre and radius of its sections.
struct StemProfile {
    /// Height above ground, centre (relative coordinates) and radius of each good section.
    h: Vec<f64>,
    c: Vec<[f64; 2]>,
    r: Vec<f64>,
    /// Height of the tree: the stem is taken to go on up to it above the last section.
    top: f64,
    /// Horizontal drift and radius change per metre above the last section.
    drift: [f64; 2],
    taper: f64,
}

impl StemProfile {
    fn of(t: &TreeResult) -> Option<Self> {
        let mut ok: Vec<&Section> = t.sections.iter().filter(|s| s.ok && s.d > 0.0).collect();
        ok.sort_by(|a, b| a.h.total_cmp(&b.h));
        let last = *ok.last()?;
        // taper over the last metre of sections, never growing
        let near: Vec<&&Section> = ok.iter().filter(|s| last.h - s.h <= 1.0).collect();
        let taper = match (near.first(), near.last()) {
            (Some(a), Some(b)) if b.h - a.h > 0.2 => {
                ((b.d - a.d) / 2.0 / (b.h - a.h)).clamp(-0.02, -0.005)
            }
            _ => -0.01,
        };
        Some(Self {
            h: ok.iter().map(|s| s.h).collect(),
            c: ok.iter().map(|s| [s.x, s.y]).collect(),
            r: ok.iter().map(|s| s.d / 2.0).collect(),
            top: t.height.max(last.h),
            drift: t.slope,
            taper,
        })
    }

    /// Centre and radius at height `h`, or `None` above the stem.
    fn at(&self, h: f64) -> Option<([f64; 2], f64)> {
        let n = self.h.len();
        let last = self.h[n - 1];
        if h > self.top {
            return None;
        }
        if h >= last {
            let dh = h - last;
            let r = (self.r[n - 1] + self.taper * dh).max(0.03);
            return Some((
                [
                    self.c[n - 1][0] + self.drift[0] * dh,
                    self.c[n - 1][1] + self.drift[1] * dh,
                ],
                r,
            ));
        }
        let k = self.h.partition_point(|v| *v <= h);
        if k == 0 {
            // below the first section: its circle, moved along the lean
            let dh = h - self.h[0];
            return Some((
                [
                    self.c[0][0] + self.drift[0] * dh,
                    self.c[0][1] + self.drift[1] * dh,
                ],
                self.r[0],
            ));
        }
        let (a, b) = (k - 1, k);
        let u = (h - self.h[a]) / (self.h[b] - self.h[a]).max(1e-9);
        let l = |x: f64, y: f64| x + (y - x) * u;
        Some((
            [l(self.c[a][0], self.c[b][0]), l(self.c[a][1], self.c[b][1])],
            l(self.r[a], self.r[b]),
        ))
    }
}

/// Writes the cloud with the points of the stems in class [`crate::classes::STEM`] and the other
/// points near a stem (up to `assign_dist` from its axis, above 0.2 m) in
/// [`crate::classes::BRANCH`]; the rest keep their class. A point belongs to the stem when it
/// lies within the measured circle of its tree at that height, plus a margin of a voxel. Heights
/// where the stem was not measured take the circle between the sections around them, and above
/// the last one the stem goes on to the top of the tree along its lean, thinning 0.5–2 cm per
/// metre of radius.
pub fn classify_stems(
    input: &Path,
    r: &TlsResult,
    p: &TlsParams,
    output: &Path,
) -> Result<StemReport> {
    use rayon::prelude::*;
    if input == output {
        return Err(Error::Param(
            "la nube clasificada debe ir a un archivo nuevo, no sobre el original".into(),
        ));
    }
    crate::paths::ensure_parent(output)?;
    let reader = Reader::open(input)?;
    let (old_format, rec_len) = (reader.header.format, usize::from(reader.header.record_len));
    // classes above 31 need a full class byte: legacy files are written as LAS 1.4
    let upgrade = if old_format < 6 {
        Some(crate::las::upgraded_format(old_format).ok_or_else(|| {
            Error::Unsupported(format!(
                "el formato de punto {old_format} no admite clases por encima de 31: convierte antes la nube"
            ))
        })?)
    } else {
        None
    };
    let template = match upgrade {
        Some(n) => Reader::open(input)?.upgraded(n),
        None => Reader::open(input)?,
    };
    let format = template.header.format;
    let mut w = Writer::create(output, &template)?;
    let o = r.origin;
    let cell = p.assign_dist.max(1.0);
    let margin = p.voxel.max(0.02);
    let profiles: Vec<Option<StemProfile>> = r.trees.iter().map(StemProfile::of).collect();
    let mut grid: FxMap<u64, Vec<usize>> = FxMap::default();
    for (k, t) in r.trees.iter().enumerate() {
        grid.entry(pack(
            (t.x / cell).floor() as i64,
            (t.y / cell).floor() as i64,
            0,
        ))
        .or_default()
        .push(k);
    }
    // 0: not touched, 1: stem, 2: branch
    let kind = |rec: &[u8]| -> u8 {
        let Some(pt) = reader.decode(rec) else {
            return 0;
        };
        if pt.is_noise() {
            return 0;
        }
        let (x, y, z) = (pt.x - o[0], pt.y - o[1], pt.z);
        let (cx, cy) = ((x / cell).floor() as i64, (y / cell).floor() as i64);
        let mut near = false;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(list) = grid.get(&pack(cx + dx, cy + dy, 0)) else {
                    continue;
                };
                for &k in list {
                    let t = &r.trees[k];
                    let h = z - (t.ground_z + o[2]);
                    if h < 0.2 {
                        continue;
                    }
                    let (ax, ay) = (
                        t.x + t.slope[0] * (h - p.base_height),
                        t.y + t.slope[1] * (h - p.base_height),
                    );
                    if (x - ax).hypot(y - ay) > p.assign_dist {
                        continue;
                    }
                    near = true;
                    if let Some(prof) = &profiles[k]
                        && let Some((c, rad)) = prof.at(h)
                        && (x - c[0]).hypot(y - c[1]) <= rad + margin
                    {
                        return 1;
                    }
                }
            }
        }
        if near { 2 } else { 0 }
    };
    let mut rep = StemReport::default();
    let mut conv = Vec::new();
    reader.scan_raw(|bytes| {
        let recs: Vec<&[u8]> = bytes.chunks_exact(rec_len).collect();
        let kinds: Vec<u8> = recs.par_iter().map(|rec| kind(rec)).collect();
        for (rec, k) in recs.iter().zip(kinds) {
            let class = match k {
                1 => {
                    rep.stem += 1;
                    Some(crate::classes::STEM)
                }
                2 => {
                    rep.branch += 1;
                    Some(crate::classes::BRANCH)
                }
                _ => {
                    rep.other += 1;
                    None
                }
            };
            if let Some(new) = upgrade {
                crate::las::upgrade_record(old_format, new, rec, &mut conv);
                if let Some(c) = class {
                    crate::las::set_class(format, &mut conv, c);
                }
                w.write(&conv)?;
            } else if let Some(c) = class {
                conv.clear();
                conv.extend_from_slice(rec);
                crate::las::set_class(format, &mut conv, c);
                w.write(&conv)?;
            } else {
                w.write(rec)?;
            }
        }
        Ok(())
    })?;
    w.finish()?;
    Ok(rep)
}

/// Everything one run does: measure, cube, write the results and, if asked, the clouds by tree.
#[derive(Clone, Debug)]
pub struct TlsJob {
    pub input: PathBuf,
    /// The trees' table (`.shp` or `.csv`).
    pub output: PathBuf,
    pub params: TlsParams,
    pub formats: crate::paths::Formats,
    pub tree_clouds: bool,
    /// Also write the cloud with the points of the stems and of the branches classified.
    pub stem_classes: bool,
}

impl TlsJob {
    /// The folder of the clouds by tree: `<output name>_arboles`.
    pub fn clouds_dir(&self) -> PathBuf {
        self.output
            .with_file_name(format!("{}_arboles", crate::paths::stem(&self.output)))
    }
}

impl TlsJob {
    /// The cloud with stems and branches classified: `<output name>_fustes` with the input's format.
    pub fn stems_path(&self) -> PathBuf {
        let ext = self
            .input
            .extension()
            .map_or("las".to_string(), |e| e.to_string_lossy().into_owned());
        self.output
            .with_file_name(format!("{}_fustes.{ext}", crate::paths::stem(&self.output)))
    }
}

pub fn run(job: &TlsJob) -> Result<String> {
    let r = analyse(&job.input, &job.params)?;
    let files = write_results(&r, &job.params, &job.output, job.formats)?;
    let mut text = summary(&r, &job.output);
    if r.trees.is_empty() {
        text.push_str(
            ". No se encontró ningún tronco: revisa la franja de búsqueda y la altura de los datos",
        );
    }
    if job.tree_clouds {
        let (n, pts) = write_tree_clouds(&job.input, &r, &job.params, &job.clouds_dir())?;
        text.push_str(&format!(
            "
{n} nubes por árbol ({pts} puntos): {}",
            job.clouds_dir().display()
        ));
    }
    if job.stem_classes && !r.trees.is_empty() {
        let rep = classify_stems(&job.input, &r, &job.params, &job.stems_path())?;
        text.push_str(&format!(
            "
Fustes (clase {}): {} puntos; ramas (clase {}): {} puntos: {}",
            crate::classes::STEM,
            rep.stem,
            crate::classes::BRANCH,
            rep.branch,
            job.stems_path().display()
        ));
    }
    let _ = files;
    Ok(text)
}

/// One line about the run.
pub fn summary(r: &TlsResult, out: &Path) -> String {
    let vol: f64 = r.trees.iter().map(|t| t.cubing.volume).sum();
    let stools = r
        .trees
        .iter()
        .filter(|t| t.sprouts > 1)
        .map(|t| t.stool)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let forked = r.trees.iter().filter(|t| t.fork_h.is_some()).count();
    let truncated: Vec<&TreeResult> = r.trees.iter().filter(|t| t.cubing.truncated).collect();
    let mut extra = String::new();
    if stools > 0 {
        extra.push_str(&format!(", {stools} cepas con varios brotes"));
    }
    if forked > 0 {
        extra.push_str(&format!(", {forked} bifurcados"));
    }
    let mut text = format!(
        "{} árboles cubicados{extra}, {:.3} m³ en total ({} puntos leídos, {} vóxeles): {}",
        r.trees.len(),
        vol,
        r.points_read,
        r.voxels,
        out.display()
    );
    let estimated: Vec<&TreeResult> = r.trees.iter().filter(|t| t.cubing.estimated).collect();
    if !estimated.is_empty() {
        let part: f64 = estimated.iter().map(|t| t.cubing.volume).sum();
        text.push_str(&format!(
            "\n  Aviso: en {} de {} árboles (ESTIMADO) el fuste se perdió estando aún grueso y lejos de la copa; su altura comercial se estima con la proporción de la altura total que se ha indicado, y su volumen ({part:.3} m³) incluye ese tramo estimado",
            estimated.len(),
            r.trees.len()
        ));
    }
    if !truncated.is_empty() {
        let part: f64 = truncated.iter().map(|t| t.cubing.volume).sum();
        text.push_str(&format!(
            "\n  Aviso: en {} de {} árboles (TRUNCADO) el fuste no se pudo seguir hasta el diámetro en punta: \
             su volumen ({part:.3} m³) es solo el del tramo medido, y el total se queda corto",
            truncated.len(),
            r.trees.len()
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stem_that_widens_upwards_loses_the_wide_sections() {
        // the stem of 8-9 cm catches a branch from 2.6 m up: 12, 14, 16 cm
        let mut secs: Vec<Section> = [
            (0.1, 0.089),
            (0.6, 0.088),
            (1.1, 0.088),
            (1.6, 0.080),
            (2.1, 0.086),
            (2.6, 0.119),
            (3.1, 0.144),
            (3.6, 0.162),
        ]
        .iter()
        .map(|&(h, d)| sec(h, d))
        .collect();
        assert_eq!(enforce_taper(&mut secs), 3);
        assert!(secs[..5].iter().all(|s| s.ok));
        assert!(secs[5..].iter().all(|s| !s.ok && s.d.is_nan()));
        // a butt swell below a metre is the root flare, not a branch
        let mut flare: Vec<Section> = [(0.1, 0.40), (0.6, 0.31), (1.1, 0.30), (1.6, 0.29)]
            .iter()
            .map(|&(h, d)| sec(h, d))
            .collect();
        assert_eq!(enforce_taper(&mut flare), 0);
        // nor is the noise of a few millimetres on a stem of constant size
        let mut even: Vec<Section> = (0..12)
            .map(|k| sec(0.5 * f64::from(k), 0.30 + 0.01 * f64::from(k % 2)))
            .collect();
        assert_eq!(enforce_taper(&mut even), 0);
        // nor can a stem creep wider a little at a time
        let mut creep: Vec<Section> = (0..10)
            .map(|k| sec(1.0 + 0.5 * f64::from(k), 0.080 + 0.008 * f64::from(k)))
            .collect();
        assert!(enforce_taper(&mut creep) >= 6);
        assert!(
            creep
                .iter()
                .filter(|s| s.ok)
                .all(|s| s.d <= 0.08 * 1.08 + 0.01 + 0.008)
        );
    }

    #[test]
    fn smalian_is_exact_for_a_cylinder() {
        let prof: Vec<(f64, f64)> = (0..=40).map(|k| (f64::from(k) * 0.25, 0.3)).collect();
        let c = smalian(&prof, 0.1, 0.5, 0.07);
        let exact = area(0.3) * (10.0 - 0.1);
        assert!((c.volume - exact).abs() < 1e-9, "{} {exact}", c.volume);
        assert!(c.truncated);
        assert_eq!(c.logs.len(), 20);
    }

    #[test]
    fn smalian_cuts_a_cone_at_the_top_diameter() {
        // d falls from 0.4 at 0 to 0 at 20: it is 0.07 at 16.5
        let prof: Vec<(f64, f64)> = (0..=76)
            .map(|k| {
                (
                    f64::from(k) * 0.25,
                    0.4 * (1.0 - f64::from(k) * 0.25 / 20.0),
                )
            })
            .collect();
        let c = smalian(&prof, 0.1, 0.5, 0.07);
        assert!((c.top_h - 16.5).abs() < 0.01, "{}", c.top_h);
        assert!(!c.truncated);
        // volume of the frustum between 0.1 and 16.5 m
        let (d0, d1) = (0.4 * (1.0 - 0.1 / 20.0), 0.07);
        let l = 16.4;
        let exact = std::f64::consts::PI * l / 12.0 * (d0 * d0 + d0 * d1 + d1 * d1);
        assert!(
            (c.volume - exact).abs() / exact < 0.01,
            "{} {exact}",
            c.volume
        );
        let last = c.logs.last();
        assert!(last.is_some_and(|l| (l.d_to - 0.07).abs() < 1e-9));
    }

    #[test]
    fn circle_fit_recovers_a_circle() {
        let pts: Vec<[f32; 2]> = (0..60)
            .map(|k| {
                let a = f64::from(k) / 60.0 * std::f64::consts::TAU;
                [(3.0 + 0.2 * a.cos()) as f32, (5.0 + 0.2 * a.sin()) as f32]
            })
            .collect();
        let (x, y, r) = fit_circle(&pts).unwrap_or_default();
        assert!((x - 3.0).abs() < 1e-4 && (y - 5.0).abs() < 1e-4 && (r - 0.2).abs() < 1e-4);
    }

    fn sec(h: f64, d: f64) -> Section {
        Section {
            h,
            x: 0.0,
            y: 0.0,
            d,
            ok: d.is_finite(),
            points: 30,
            sectors: 12,
        }
    }

    #[test]
    fn a_rejected_section_loses_its_diameter() {
        // 0.50 m between 0.30 and 0.28 disagrees with both, and the sections that could have
        // filled its place fail: the metre and a half around it is a gap the fill leaves alone
        let mut s = vec![
            sec(1.0, 0.30),
            sec(1.5, 0.50),
            sec(1.75, f64::NAN),
            sec(2.5, f64::NAN),
            sec(3.0, 0.28),
        ];
        reject_outliers(&mut s);
        assert!(!s[1].ok && s[1].d.is_nan());
        assert!(s[0].ok && s[0].d == 0.30 && s[4].ok && s[4].d == 0.28);
        // nothing of the section is left to be taken as a measurement
        assert!(s.iter().filter(|x| x.ok || x.d.is_finite()).count() == 2);
    }

    #[test]
    fn rejections_are_decided_on_the_diameters_as_measured() {
        // 0.50 and then 0.31: if rejecting the first erased its diameter before the second was
        // compared, the second would be compared with NaN. Both are judged on the originals.
        let mut s = vec![
            sec(1.0, 0.30),
            sec(1.25, 0.50),
            sec(1.5, 0.31),
            sec(1.75, 0.30),
        ];
        reject_outliers(&mut s);
        assert!(!s[1].ok);
        assert!(s[2].ok && s[2].d == 0.31);
    }

    /// Voxels of a ring of radius `r` centred at `(cx, cy)`, every 2 cm, in two layers.
    fn ring(cx: f64, cy: f64, r: f64) -> Vec<[f32; 2]> {
        let n = (std::f64::consts::TAU * r / 0.02).round() as usize;
        (0..n * 5)
            .map(|k| {
                let a = (k % n) as f64 * std::f64::consts::TAU / n as f64;
                [(cx + r * a.cos()) as f32, (cy + r * a.sin()) as f32]
            })
            .collect()
    }

    #[test]
    fn circles_of_stems_that_touch_are_found_one_by_one() {
        let p = TlsParams {
            voxel: 0.02,
            ..TlsParams::default()
        };
        for r in [0.08, 0.12, 0.15, 0.25] {
            let found = circles_in(&ring(1.0, 2.0, r), (1.0, 2.0), f64::INFINITY, &p, None, 6);
            assert_eq!(found.len(), 1, "r = {r}");
            assert!(
                (found[0].2 - 2.0 * r).abs() < 0.01,
                "r = {r}: {:?}",
                found[0]
            );
        }
        // two sprouts 35 cm apart
        let mut two = ring(0.0, 0.0, 0.08);
        two.extend(ring(0.35, 0.0, 0.08));
        assert_eq!(
            circles_in(&two, (0.2, 0.0), f64::INFINITY, &p, None, 6).len(),
            2
        );
    }

    #[test]
    fn principal_axis_of_a_line() {
        let v = principal([[0.01, 0.0, 0.0], [0.0, 0.01, 0.0], [0.0, 0.0, 5.0]]);
        assert!(v[2].abs() > 0.999);
    }

    #[test]
    fn stem_profile_interpolates_between_sections_and_goes_on_above() {
        let prof = StemProfile {
            h: vec![1.0, 3.0],
            c: vec![[0.0, 0.0], [0.2, 0.0]],
            r: vec![0.10, 0.08],
            top: 6.0,
            drift: [0.1, 0.0],
            taper: -0.01,
        };
        // halfway between the sections
        let (c, r) = prof.at(2.0).unwrap_or_default();
        assert!((c[0] - 0.1).abs() < 1e-9 && (r - 0.09).abs() < 1e-9);
        // two metres above the last one: drifts along the lean and thins
        let (c, r) = prof.at(5.0).unwrap_or_default();
        assert!((c[0] - 0.4).abs() < 1e-9 && (r - 0.06).abs() < 1e-9);
        // above the tree: no stem
        assert!(prof.at(6.5).is_none());
    }

    fn stem_lost_at(last_h: f64, height: f64, p: &TlsParams) -> TreeResult {
        let sections: Vec<Section> = (0..)
            .map(|k| 0.1 + 0.25 * f64::from(k))
            .take_while(|h| *h <= last_h)
            .map(|h| Section {
                h,
                x: 0.0,
                y: 0.0,
                d: 0.20 - 0.004 * h,
                ok: true,
                points: 30,
                sectors: 12,
            })
            .collect();
        let profile: Vec<(f64, f64)> = sections.iter().map(|s| (s.h, s.d)).collect();
        TreeResult {
            id: 1,
            x: 0.0,
            y: 0.0,
            ground_z: 0.0,
            dbh: Some(0.19),
            height,
            tilt: 0.0,
            cubing: smalian(&profile, p.base_height, p.log_len, p.top_diam),
            sections,
            points: 0,
            quality: 100.0,
            d_base: 0.2,
            flags: "TRUNCADO".into(),
            slope: [0.0, 0.0],
            stool: 1,
            sprouts: 1,
            fork_h: None,
        }
    }

    #[test]
    fn a_stem_lost_thick_and_far_below_the_top_gets_the_expected_merchantable_height() {
        let p = TlsParams {
            lost_top_share: 0.65,
            ..TlsParams::default()
        };
        // lost at 4 m with 18 cm in a tree of 20 m: its merchantable height is 13 m
        let mut trees = vec![stem_lost_at(4.0, 20.0, &p), stem_lost_at(19.5, 20.0, &p)];
        assert!(trees[0].cubing.truncated);
        let before = trees[0].cubing.volume;
        estimate_lost_tops(&mut trees, &p);
        assert!(trees[0].cubing.estimated && !trees[0].cubing.truncated);
        assert!((trees[0].cubing.top_h - 13.0).abs() < 0.1);
        assert!(trees[0].cubing.volume > before);
        assert!(trees[0].flags.contains("ESTIMADO") && !trees[0].flags.contains("TRUNCADO"));
        // lost within a metre of the top of the tree: that is where the stem ends
        assert!(trees[1].cubing.truncated && !trees[1].cubing.estimated);
    }
}
