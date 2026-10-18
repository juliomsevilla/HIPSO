//! Tools between clouds and on repeated points: duplicate points, change detection between two
//! clouds, alignment of one cloud to another (iterative closest point) and the height
//! difference between flight lines. All of them read the clouds as streams; what they hold in
//! memory is a thinned copy of the reference, the changed points, or one number per cell.

// Small fixed-size matrix code reads best with indices.
#![allow(clippy::needless_range_loop)]

use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::{Path, PathBuf};

use crate::filter::ReturnKind;
use crate::grid::Grid;
use crate::las::{Reader, Writer, set_xyz};
use crate::{Error, Result};

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

// ----------------------------------------------------------------------------- duplicates

/// Which attributes must also agree for two points at the same place to be duplicates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DupFields {
    pub class: bool,
    pub gps_time: bool,
    pub intensity: bool,
    pub return_number: bool,
    pub return_count: bool,
    pub source_id: bool,
}

fn dup_key(r: &Reader, rec: &[u8], f: DupFields) -> u64 {
    let mut h = Fx::default();
    h.write(rec.get(..12).unwrap_or(rec));
    let (p, e) = (r.decode(rec), r.decode_extra(rec));
    if let Some(p) = p {
        if f.class {
            h.write_u64(u64::from(p.class) | 1 << 40);
        }
        if f.intensity {
            h.write_u64(u64::from(p.intensity) | 2 << 40);
        }
        if f.return_number {
            h.write_u64(u64::from(p.return_number) | 3 << 40);
        }
        if f.return_count {
            h.write_u64(u64::from(p.number_of_returns) | 4 << 40);
        }
    }
    if f.gps_time {
        h.write_u64(e.gps_time.to_bits());
    }
    if f.source_id {
        h.write_u64(u64::from(e.source_id) | 5 << 40);
    }
    h.finish()
}

/// Writes `input` without its duplicate points: one point is kept from each set of duplicates,
/// or none with `delete_all`. Returns (points read, points removed). Memory: 16 bytes per
/// distinct point.
pub fn remove_duplicates(
    input: &Path,
    output: &Path,
    fields: DupFields,
    delete_all: bool,
) -> Result<(u64, u64)> {
    if input == output {
        return Err(Error::Param(
            "the new file must not overwrite the original".into(),
        ));
    }
    let r = Reader::open(input)?.for_passes()?;
    let rec_len = usize::from(r.header.record_len);
    let mut count: FxMap<u64, u8> = FxMap::default();
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let c = count.entry(dup_key(&r, rec, fields)).or_insert(0);
            *c = c.saturating_add(1);
        }
        Ok(())
    })?;
    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create(output, &r)?;
    let (mut read, mut removed) = (0u64, 0u64);
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            read += 1;
            let k = dup_key(&r, rec, fields);
            let keep = match count.get_mut(&k) {
                Some(c) if delete_all => *c <= 1,
                Some(c) if *c > 0 => {
                    *c = 0;
                    true
                }
                _ => false,
            };
            if keep {
                w.write(rec)?;
            } else {
                removed += 1;
            }
        }
        Ok(())
    })?;
    w.finish()?;
    Ok((read, removed))
}

// ----------------------------------------------------------------------------- comparing

/// Reference points on a grid of cell `cell`, thinned to a third of it, for nearest-point tests.
struct RefGrid {
    origin: [f64; 3],
    cell: f64,
    cells: FxMap<u64, Vec<[f32; 3]>>,
    points: usize,
}

impl RefGrid {
    fn build(paths: &[PathBuf], cell: f64, voxel: f64) -> Result<Self> {
        let mut origin = [f64::MAX; 3];
        for p in paths {
            let b = Reader::open(p)?.header.bounds;
            for k in 0..3 {
                origin[k] = origin[k].min(b[k]);
            }
        }
        let v = voxel;
        let mut seen: FxMap<u64, ()> = FxMap::default();
        let mut cells: FxMap<u64, Vec<[f32; 3]>> = FxMap::default();
        let mut points = 0;
        for p in paths {
            let r = Reader::open(p)?;
            r.scan(|q| {
                if q.is_noise() {
                    return;
                }
                let (x, y, z) = (q.x - origin[0], q.y - origin[1], q.z - origin[2]);
                let key = pack(
                    (x / v).floor() as i64,
                    (y / v).floor() as i64,
                    (z / v).floor() as i64,
                );
                if seen.insert(key, ()).is_none() {
                    cells
                        .entry(pack(
                            (x / cell).floor() as i64,
                            (y / cell).floor() as i64,
                            (z / cell).floor() as i64,
                        ))
                        .or_default()
                        .push([x as f32, y as f32, z as f32]);
                    points += 1;
                }
            })?;
        }
        Ok(Self {
            origin,
            cell,
            cells,
            points,
        })
    }

    /// Distance to the nearest reference point if it is below `reach` (at most the cell).
    fn nearest(&self, x: f64, y: f64, z: f64, reach: f64) -> Option<f64> {
        let (x, y, z) = (x - self.origin[0], y - self.origin[1], z - self.origin[2]);
        let (cx, cy, cz) = (
            (x / self.cell).floor() as i64,
            (y / self.cell).floor() as i64,
            (z / self.cell).floor() as i64,
        );
        let mut best = reach * reach;
        let mut found = false;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = self.cells.get(&pack(cx + dx, cy + dy, cz + dz)) {
                        for q in list {
                            let d = (f64::from(q[0]) - x).powi(2)
                                + (f64::from(q[1]) - y).powi(2)
                                + (f64::from(q[2]) - z).powi(2);
                            if d < best {
                                best = d;
                                found = true;
                            }
                        }
                    }
                }
            }
        }
        found.then(|| best.sqrt())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompareParams {
    /// 3D distance from which a point has changed.
    pub min_dist: f64,
    /// A changed point needs this many other changed points within twice the distance.
    pub min_neighbors: usize,
}

impl Default for CompareParams {
    fn default() -> Self {
        Self {
            min_dist: 0.5,
            min_neighbors: 3,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareReport {
    pub points: u64,
    pub changed: u64,
    pub kept: u64,
    /// Mean and RMS distance of the points that did not change.
    pub mean: f64,
    pub rms: f64,
    /// Share of the points with a counterpart within the distance.
    pub completeness: f64,
}

/// Writes to `output` the points of `source` that lie farther than `min_dist` from every point
/// of `reference`: what changed between two surveys (or what an edit removed).
pub fn compare_clouds(
    reference: &[PathBuf],
    source: &Path,
    output: &Path,
    p: &CompareParams,
) -> Result<CompareReport> {
    if !(p.min_dist > 0.0) {
        return Err(Error::Param("the distance must be positive".into()));
    }
    if reference.iter().any(|r| r == source) || source == output {
        return Err(Error::Param(
            "the reference and the cloud to check must differ".into(),
        ));
    }
    let grid = RefGrid::build(reference, p.min_dist, p.min_dist / 3.0)?;
    if grid.points == 0 {
        return Err(Error::Param("the reference cloud has no points".into()));
    }
    let r = Reader::open(source)?;
    let rec_len = usize::from(r.header.record_len);
    let mut rep = CompareReport::default();
    let (mut sum, mut sum2, mut near) = (0.0, 0.0, 0u64);
    let mut changed: Vec<([f64; 3], Vec<u8>)> = Vec::new();
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let Some(q) = r.decode(rec) else { continue };
            if q.is_noise() {
                continue;
            }
            rep.points += 1;
            match grid.nearest(q.x, q.y, q.z, p.min_dist) {
                Some(d) => {
                    near += 1;
                    sum += d;
                    sum2 += d * d;
                }
                None => changed.push(([q.x, q.y, q.z], rec.to_vec())),
            }
        }
        Ok(())
    })?;
    rep.changed = changed.len() as u64;
    if near > 0 {
        rep.mean = sum / near as f64;
        rep.rms = (sum2 / near as f64).sqrt();
    }
    rep.completeness = near as f64 / rep.points.max(1) as f64 * 100.0;
    // a lone changed point is noise: keep those that have company
    let reach = 2.0 * p.min_dist;
    let mut cells: FxMap<u64, Vec<u32>> = FxMap::default();
    for (i, (c, _)) in changed.iter().enumerate() {
        cells
            .entry(pack(
                (c[0] / reach).floor() as i64,
                (c[1] / reach).floor() as i64,
                (c[2] / reach).floor() as i64,
            ))
            .or_default()
            .push(i as u32);
    }
    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create(output, &r)?;
    for (i, (c, rec)) in changed.iter().enumerate() {
        let (cx, cy, cz) = (
            (c[0] / reach).floor() as i64,
            (c[1] / reach).floor() as i64,
            (c[2] / reach).floor() as i64,
        );
        let mut n = 0usize;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) {
                        for &o in list {
                            let q = &changed[o as usize].0;
                            if o as usize != i
                                && (q[0] - c[0]).powi(2)
                                    + (q[1] - c[1]).powi(2)
                                    + (q[2] - c[2]).powi(2)
                                    <= reach * reach
                            {
                                n += 1;
                                if n >= p.min_neighbors {
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
        }
        if n >= p.min_neighbors {
            w.write(rec)?;
            rep.kept += 1;
        }
    }
    w.finish()?;
    Ok(rep)
}

pub fn compare_text(r: &CompareReport, out: &Path) -> String {
    format!(
        "{} puntos comparados: {} cambiados, {} conservados tras quitar los aislados; distancia media de los que coinciden {:.3} m (RMS {:.3} m); compleción {:.1} %: {}",
        r.points,
        r.changed,
        r.kept,
        r.mean,
        r.rms,
        r.completeness,
        out.display()
    )
}

// ----------------------------------------------------------------------------- aligning

#[derive(Clone, Debug, PartialEq)]
pub struct FitParams {
    /// 3D thinning (m) of the points used to find the transform.
    pub thin: f64,
    /// Farthest a reference point may be, horizontally and vertically, to pair with a point.
    pub max_xy: f64,
    pub max_z: f64,
    pub max_iter: usize,
    /// Stop when the mean distance is below this.
    pub stop_dist: f64,
    /// Only translate (no rotation).
    pub shift_only: bool,
}

impl Default for FitParams {
    fn default() -> Self {
        Self {
            thin: 1.0,
            max_xy: 5.0,
            max_z: 2.0,
            max_iter: 30,
            stop_dist: 0.01,
            shift_only: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FitReport {
    pub iterations: usize,
    pub pairs: usize,
    pub before: f64,
    pub after: f64,
    pub shift: [f64; 3],
    /// Rotation about x, y and z, degrees.
    pub rotation: [f64; 3],
}

/// Solves the symmetric system `a x = b` (Gauss with pivoting); `None` when singular.
fn solve<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for c in 0..N {
        let p = (c..N).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..N {
            let f = a[r][c] / a[c][c];
            for k in c..N {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = [0.0; N];
    for c in (0..N).rev() {
        let s: f64 = (c + 1..N).map(|k| a[c][k] * x[k]).sum();
        x[c] = (b[c] - s) / a[c][c];
    }
    Some(x)
}

/// A rigid motion: p' = r p + t.
#[derive(Clone, Copy, Debug)]
struct Rigid {
    r: [[f64; 3]; 3],
    t: [f64; 3],
}

impl Rigid {
    const ID: Rigid = Rigid {
        r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };

    fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let mut o = self.t;
        for i in 0..3 {
            for j in 0..3 {
                o[i] += self.r[i][j] * p[j];
            }
        }
        o
    }

    /// `self` after `first`.
    fn after(&self, first: &Rigid) -> Rigid {
        let mut r = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                for k in 0..3 {
                    r[i][j] += self.r[i][k] * first.r[k][j];
                }
            }
        }
        let t = self.apply(first.t);
        Rigid { r, t }
    }

    /// The rotation for a small turn `w` (radians about x, y, z), by Rodrigues' formula.
    fn small(w: [f64; 3], t: [f64; 3]) -> Rigid {
        let a = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        let mut r = Self::ID.r;
        if a > 1e-15 {
            let k = [w[0] / a, w[1] / a, w[2] / a];
            let (s, c) = a.sin_cos();
            let kx = [[0.0, -k[2], k[1]], [k[2], 0.0, -k[0]], [-k[1], k[0], 0.0]];
            for i in 0..3 {
                for j in 0..3 {
                    let id = if i == j { 1.0 } else { 0.0 };
                    let kk: f64 = (0..3).map(|m| kx[i][m] * kx[m][j]).sum();
                    r[i][j] = id + s * kx[i][j] + (1.0 - c) * kk;
                }
            }
        }
        Rigid { r, t }
    }
}

/// Moves `source` to lie on `reference` (iterative closest point) and writes it to `output`.
pub fn fit_cloud(
    reference: &[PathBuf],
    source: &Path,
    output: &Path,
    p: &FitParams,
) -> Result<FitReport> {
    if reference.iter().any(|r| r == source) || source == output {
        return Err(Error::Param(
            "the reference and the cloud to move must differ".into(),
        ));
    }
    if !(p.thin > 0.0 && p.max_xy > 0.0 && p.max_z > 0.0) {
        return Err(Error::Param(
            "the thinning and the limits must be positive".into(),
        ));
    }
    let limit = p.max_xy.max(p.max_z);
    let grid = RefGrid::build(reference, limit, p.thin / 2.0)?;
    if grid.points == 0 {
        return Err(Error::Param("the reference cloud has no points".into()));
    }
    let flat: Vec<[f64; 3]> = grid
        .cells
        .values()
        .flatten()
        .map(|s| {
            [
                f64::from(s[0]) + grid.origin[0],
                f64::from(s[1]) + grid.origin[1],
                f64::from(s[2]) + grid.origin[2],
            ]
        })
        .collect();
    drop(grid);
    let r = Reader::open(source)?;
    // the points that look for their pair: last and single returns, one per 3D cell
    let mut seen: FxMap<u64, ()> = FxMap::default();
    let mut sample: Vec<[f64; 3]> = Vec::new();
    r.scan(|q| {
        if q.is_noise() || !matches!(ReturnKind::of(q), ReturnKind::Last | ReturnKind::Single) {
            return;
        }
        let k = pack(
            (q.x / p.thin).floor() as i64,
            (q.y / p.thin).floor() as i64,
            (q.z / p.thin).floor() as i64,
        );
        if seen.insert(k, ()).is_none() {
            sample.push([q.x, q.y, q.z]);
        }
    })?;
    drop(seen);
    if sample.len() < 10 {
        return Err(Error::Param("too few points to align".into()));
    }
    let c0 = sample
        .iter()
        .fold([0.0; 3], |a, q| [a[0] + q[0], a[1] + q[1], a[2] + q[2]])
        .map(|v| v / sample.len() as f64);
    let mut m = Rigid::ID;
    let mut rep = FitReport::default();
    let mut reach = limit;
    for it in 0..p.max_iter.max(1) {
        crate::check_cancel()?;
        // the search narrows as the clouds come together: a grid of cells as wide as the reach
        let mut cells: FxMap<u64, Vec<u32>> = FxMap::default();
        for (i, q) in flat.iter().enumerate() {
            cells
                .entry(pack(
                    (q[0] / reach).floor() as i64,
                    (q[1] / reach).floor() as i64,
                    (q[2] / reach).floor() as i64,
                ))
                .or_default()
                .push(i as u32);
        }
        let (lim_xy, lim_z) = (p.max_xy.min(reach), p.max_z.min(reach));
        let moved: Vec<[f64; 3]> = sample.iter().map(|q| m.apply(*q)).collect();
        let pairs: Vec<([f64; 3], [f64; 3])> = moved
            .par_iter()
            .filter_map(|q| {
                let (cx, cy, cz) = (
                    (q[0] / reach).floor() as i64,
                    (q[1] / reach).floor() as i64,
                    (q[2] / reach).floor() as i64,
                );
                let mut best: Option<(f64, [f64; 3])> = None;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let Some(list) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) else {
                                continue;
                            };
                            for &o in list {
                                let s = flat[o as usize];
                                let (ex, ey, ez) = (s[0] - q[0], s[1] - q[1], s[2] - q[2]);
                                if ex.hypot(ey) > lim_xy || ez.abs() > lim_z {
                                    continue;
                                }
                                let d = ex * ex + ey * ey + ez * ez;
                                if best.is_none_or(|b| d < b.0) {
                                    best = Some((d, s));
                                }
                            }
                        }
                    }
                }
                best.map(|b| (*q, b.1))
            })
            .collect();
        if pairs.len() < 10 {
            return Err(Error::Param(
                "the clouds do not overlap within the limits: raise the horizontal and vertical limits".into(),
            ));
        }
        let mean = pairs
            .iter()
            .map(|(a, b)| {
                ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
            })
            .sum::<f64>()
            / pairs.len() as f64;
        if it == 0 {
            rep.before = mean;
        }
        rep.after = mean;
        rep.pairs = pairs.len();
        rep.iterations = it;
        if mean <= p.stop_dist {
            break;
        }
        reach = (3.0 * mean).clamp(p.thin * 1.5, limit);
        // least squares for the small motion that takes each point to its pair
        let step = if p.shift_only {
            let n = pairs.len() as f64;
            let t = (0..3)
                .map(|k| pairs.iter().map(|(a, b)| b[k] - a[k]).sum::<f64>() / n)
                .collect::<Vec<_>>();
            Rigid::small([0.0; 3], [t[0], t[1], t[2]])
        } else {
            let mut ata = [[0.0; 6]; 6];
            let mut atb = [0.0; 6];
            for (a, b) in &pairs {
                let (x, y, z) = (a[0] - c0[0], a[1] - c0[1], a[2] - c0[2]);
                let rows = [
                    ([0.0, z, -y, 1.0, 0.0, 0.0], b[0] - a[0]),
                    ([-z, 0.0, x, 0.0, 1.0, 0.0], b[1] - a[1]),
                    ([y, -x, 0.0, 0.0, 0.0, 1.0], b[2] - a[2]),
                ];
                for (row, rhs) in rows {
                    for i in 0..6 {
                        atb[i] += row[i] * rhs;
                        for j in 0..6 {
                            ata[i][j] += row[i] * row[j];
                        }
                    }
                }
            }
            let Some(x) = solve(ata, atb) else { break };
            // turn about the centre of the sample, then translate
            let rot = Rigid::small([x[0], x[1], x[2]], [0.0; 3]);
            let back = Rigid {
                r: Rigid::ID.r,
                t: c0,
            };
            let centre_out = Rigid {
                r: Rigid::ID.r,
                t: [-c0[0], -c0[1], -c0[2]],
            };
            let turn = back.after(&rot).after(&centre_out);
            Rigid {
                r: turn.r,
                t: [turn.t[0] + x[3], turn.t[1] + x[4], turn.t[2] + x[5]],
            }
        };
        let shift = step.apply(c0);
        let small = (0..3).all(|k| (shift[k] - c0[k]).abs() < 1e-4);
        m = step.after(&m);
        if small {
            break;
        }
    }
    // the whole cloud follows the motion
    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create(output, &r)?;
    let rec_len = usize::from(r.header.record_len);
    let header = r.header.clone();
    let mut buf = vec![0u8; rec_len];
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            buf.copy_from_slice(rec);
            if let Some(q) = r.decode(rec) {
                let o = m.apply([q.x, q.y, q.z]);
                if !set_xyz(&header, &mut buf, o[0], o[1], o[2]) {
                    return Err(Error::Param(
                        "a moved point does not fit the coordinate scale of the file".into(),
                    ));
                }
            }
            w.write(&buf)?;
        }
        Ok(())
    })?;
    w.finish()?;
    // the displacement of the middle of the cloud
    let mid = m.apply(c0);
    rep.shift = [mid[0] - c0[0], mid[1] - c0[1], mid[2] - c0[2]];
    // angles of the final rotation (x-y-z order)
    let rr = m.r;
    rep.rotation = [
        rr[2][1].atan2(rr[2][2]).to_degrees(),
        (-rr[2][0]).asin().to_degrees(),
        rr[1][0].atan2(rr[0][0]).to_degrees(),
    ];
    Ok(rep)
}

pub fn fit_text(r: &FitReport, out: &Path) -> String {
    format!(
        "{} iteraciones, {} pares; distancia media {:.3} m → {:.3} m; desplazamiento ({:.3}, {:.3}, {:.3}) m, giro ({:.4}°, {:.4}°, {:.4}°): {}",
        r.iterations,
        r.pairs,
        r.before,
        r.after,
        r.shift[0],
        r.shift[1],
        r.shift[2],
        r.rotation[0],
        r.rotation[1],
        r.rotation[2],
        out.display()
    )
}

// ----------------------------------------------------------------------------- swaths

/// Which returns count in the swath separation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwathReturns {
    All,
    Last,
    Single,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwathReport {
    pub cells: usize,
    pub overlap_cells: usize,
    /// Mean absolute difference and RMS of the differences between swaths, m.
    pub mean_abs: f64,
    pub rms: f64,
    pub max: f64,
    /// Share of the overlap cells whose difference passes 8 cm and 16 cm.
    pub over_8: f64,
    pub over_16: f64,
}

/// Height difference between flight lines: per cell, the largest gap between the mean
/// elevations of the lines (source ids) that have points in it. Writes a GeoTIFF of the
/// difference and returns the figures of the overlap.
pub fn swath_separation(
    inputs: &[PathBuf],
    output: &Path,
    cell: f64,
    returns: SwathReturns,
    each_cloud_a_swath: bool,
) -> Result<SwathReport> {
    if !(cell > 0.0) {
        return Err(Error::Param("the cell must be positive".into()));
    }
    let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let readers: Vec<Reader> = inputs
        .iter()
        .map(|p| Reader::open(p))
        .collect::<Result<_>>()?;
    for r in &readers {
        let b = r.header.bounds;
        bounds = [
            bounds[0].min(b[0]),
            bounds[1].min(b[1]),
            bounds[2].max(b[3]),
            bounds[3].max(b[4]),
        ];
    }
    let mut g = Grid::covering(bounds, cell)?;
    let mut acc: FxMap<u32, Vec<(u32, f64, u32)>> = FxMap::default();
    for (k, r) in readers.iter().enumerate() {
        let rec_len = usize::from(r.header.record_len);
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let Some(q) = r.decode(rec) else { continue };
                let kind = ReturnKind::of(&q);
                let ok = match returns {
                    SwathReturns::All => true,
                    SwathReturns::Last => matches!(kind, ReturnKind::Last | ReturnKind::Single),
                    SwathReturns::Single => kind == ReturnKind::Single,
                };
                if q.is_noise() || !ok {
                    continue;
                }
                let Some(i) = g.index(q.x, q.y) else { continue };
                let id = if each_cloud_a_swath {
                    k as u32
                } else {
                    u32::from(r.decode_extra(rec).source_id)
                };
                let list = acc.entry(i as u32).or_default();
                match list.iter_mut().find(|e| e.0 == id) {
                    Some(e) => {
                        e.1 += q.z;
                        e.2 += 1;
                    }
                    None => list.push((id, q.z, 1)),
                }
            }
            Ok(())
        })?;
    }
    let mut rep = SwathReport::default();
    let (mut sum, mut sum2) = (0.0, 0.0);
    let (mut o8, mut o16) = (0usize, 0usize);
    g.data.fill(f32::NAN);
    rep.cells = acc.len();
    for (i, list) in &acc {
        if list.len() < 2 {
            continue;
        }
        let means: Vec<f64> = list.iter().map(|e| e.1 / f64::from(e.2)).collect();
        let (lo, hi) = means
            .iter()
            .fold((f64::MAX, f64::MIN), |a, m| (a.0.min(*m), a.1.max(*m)));
        let d = hi - lo;
        g.data[*i as usize] = d as f32;
        rep.overlap_cells += 1;
        sum += d;
        sum2 += d * d;
        rep.max = rep.max.max(d);
        o8 += usize::from(d > 0.08);
        o16 += usize::from(d > 0.16);
    }
    if rep.overlap_cells > 0 {
        let n = rep.overlap_cells as f64;
        rep.mean_abs = sum / n;
        rep.rms = (sum2 / n).sqrt();
        rep.over_8 = o8 as f64 / n * 100.0;
        rep.over_16 = o16 as f64 / n * 100.0;
    }
    crate::paths::ensure_parent(output)?;
    g.write_tiff(output, readers.first().and_then(|r| r.header.epsg))?;
    Ok(rep)
}

pub fn swath_text(r: &SwathReport, out: &Path) -> String {
    if r.overlap_cells == 0 {
        return format!(
            "sin solapes entre pasadas en {} celdas: {}",
            r.cells,
            out.display()
        );
    }
    format!(
        "{} celdas de solape: diferencia media {:.3} m, RMS {:.3} m, máxima {:.3} m; {:.1} % pasa de 8 cm y {:.1} % de 16 cm: {}",
        r.overlap_cells,
        r.mean_abs,
        r.rms,
        r.max,
        r.over_8,
        r.over_16,
        out.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_small_turn_matches_rotation() {
        let m = Rigid::small([0.0, 0.0, std::f64::consts::FRAC_PI_2], [0.0; 3]);
        let p = m.apply([1.0, 0.0, 0.0]);
        assert!((p[0]).abs() < 1e-12 && (p[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn linear_system_solves() {
        let x = solve([[2.0, 1.0], [1.0, 3.0]], [5.0, 10.0]);
        assert!(x.is_some_and(|x| (x[0] - 1.0).abs() < 1e-12 && (x[1] - 3.0).abs() < 1e-12));
    }
}
