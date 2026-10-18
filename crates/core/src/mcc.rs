//! Multiscale Curvature Classification of ground points (Evans & Hudak 2007), following the
//! behaviour of MCC-LIDAR 2.1 (Apache-2.0, https://sourceforge.net/projects/mcclidar/).
//!
//! Three scale domains, with cell sizes 0.5 s, s and 1.5 s and curvature thresholds t, t + 0.1
//! and t + 0.2. In each, passes repeat until fewer than 1%, 0.1% and 0.01% of the remaining
//! points are dropped: a thin-plate spline through the remaining points is rasterised, averaged
//! with a 3 x 3 kernel, and every point higher than the surface of its cell plus the threshold
//! is classified as non-ground. Works in memory: run it on tiles of bounded size.

use crate::las::Point;

#[derive(Clone, Debug, PartialEq)]
pub struct MccParams {
    /// Cell size of scale domain 2 (about the point spacing).
    pub spacing: f64,
    /// Curvature threshold of scale domain 1.
    pub threshold: f64,
}

impl Default for MccParams {
    fn default() -> Self {
        Self {
            spacing: 1.5,
            threshold: 0.3,
        }
    }
}

/// Points per spline, as MCC-LIDAR (ArcGIS Spline's default).
const PTS_PER_REGION: usize = 12;
/// A spline that stays singular with this many points is given up.
const MAX_SPLINE_PTS: usize = 300;
const MAX_PASSES: u32 = 100;

/// Which points are ground. Noise and withheld points are never ground and take no part.
pub fn ground(pts: &[Point], p: &MccParams) -> Vec<bool> {
    let mut ground = vec![false; pts.len()];
    if !(p.spacing > 0.0) || !p.threshold.is_finite() {
        return ground;
    }
    let cand: Vec<usize> = (0..pts.len())
        .filter(|&i| pts.get(i).is_some_and(|q| !q.is_noise()))
        .collect();
    let Some(&first) = cand.first() else {
        return ground;
    };
    let (ox, oy) = pts.get(first).map_or((0.0, 0.0), |q| (q.x, q.y));

    // Stacked points: of those sharing x and y, only the lowest can be ground; duplicates of
    // the lowest are set aside and take the class of the one kept.
    let mut order = cand.clone();
    let at = |i: usize| pts.get(i).copied().unwrap_or_default();
    order.sort_unstable_by(|&a, &b| {
        let (a, b) = (at(a), at(b));
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    let mut keep: Vec<usize> = Vec::with_capacity(order.len());
    let mut twins: Vec<(usize, usize)> = Vec::new();
    let mut k = 0;
    while k < order.len() {
        let lead = at(order[k]);
        let mut e = k + 1;
        while e < order.len() && at(order[e]).x == lead.x && at(order[e]).y == lead.y {
            e += 1;
        }
        keep.push(order[k]);
        for &j in order.get(k + 1..e).unwrap_or(&[]) {
            if at(j).z == lead.z {
                twins.push((j, order[k]));
            }
        }
        k = e;
    }

    // Coordinates relative to the first point keep the splines well conditioned.
    // In file order, as MCC-LIDAR walks them.
    keep.sort_unstable();
    let mut u: Vec<(usize, [f64; 3])> = keep
        .iter()
        .map(|&i| (i, [at(i).x - ox, at(i).y - oy, at(i).z]))
        .collect();
    let extent = u.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, (_, q)| {
            [
                b[0].min(q[0]),
                b[1].min(q[1]),
                b[2].max(q[0]),
                b[3].max(q[1]),
            ]
        },
    );
    let domains = [
        (0.5 * p.spacing, p.threshold, 0.01),
        (p.spacing, p.threshold + 0.1, 0.001),
        (1.5 * p.spacing, p.threshold + 0.2, 0.0001),
    ];
    for (cell, t, convergence) in domains {
        for _ in 0..MAX_PASSES {
            if u.is_empty() {
                break;
            }
            let mut raster = Raster::new(extent, cell);
            raster.interpolate(&u);
            raster.average3();
            let n = u.len();
            u.retain(|(_, q)| raster.at(q[0], q[1]).is_none_or(|s| q[2] <= s + t));
            if ((n - u.len()) as f64 / n as f64) < convergence {
                break;
            }
        }
    }
    for (i, _) in &u {
        if let Some(g) = ground.get_mut(*i) {
            *g = true;
        }
    }
    for (twin, kept) in twins {
        let g = ground.get(kept).copied().unwrap_or(false);
        if let Some(t) = ground.get_mut(twin) {
            *t = g;
        }
    }
    ground
}

/// A raster surface over the points' extent with half a cell of margin; NaN where no spline
/// could be computed.
struct Raster {
    x0: f64,
    y0: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    z: Vec<f64>,
}

impl Raster {
    fn new(e: [f64; 4], cell: f64) -> Self {
        let (w, h) = (e[2] - e[0] + cell, e[3] - e[1] + cell);
        let cols = ((w / cell).ceil() as usize).max(1);
        let rows = ((h / cell).ceil() as usize).max(1);
        let x0 = e[0] - cell / 2.0 - (cols as f64 * cell - w) / 2.0;
        let y0 = e[1] - cell / 2.0 - (rows as f64 * cell - h) / 2.0;
        Self {
            x0,
            y0,
            cell,
            cols,
            rows,
            z: vec![f64::NAN; cols * rows],
        }
    }

    #[inline]
    fn cell_of(&self, x: f64, y: f64) -> Option<usize> {
        let c = ((x - self.x0) / self.cell).floor();
        let r = ((y - self.y0) / self.cell).floor();
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows)
            .then(|| r as usize * self.cols + c as usize)
    }

    #[inline]
    fn at(&self, x: f64, y: f64) -> Option<f64> {
        self.cell_of(x, y)
            .and_then(|i| self.z.get(i).copied())
            .filter(|v| !v.is_nan())
    }

    /// Fills the raster with thin-plate splines, each fitted to the points of one region of
    /// about [`PTS_PER_REGION`] points (topped up from the nearest rings of regions).
    fn interpolate(&mut self, u: &[(usize, [f64; 3])]) {
        let (w, h) = (self.cols as f64 * self.cell, self.rows as f64 * self.cell);
        let wanted = (u.len() / PTS_PER_REGION).max(1);
        let size = (w * h / wanted as f64).sqrt();
        let nr = ((h / size).floor() as usize).clamp(1, self.rows);
        let nc = ((w / size).floor() as usize).clamp(1, self.cols);
        let (rh, rw) = (h / nr as f64, w / nc as f64);
        let region = |x: f64, y: f64| {
            let c = (((x - self.x0) / rw).floor().max(0.0) as usize).min(nc - 1);
            let r = (((y - self.y0) / rh).floor().max(0.0) as usize).min(nr - 1);
            r * nc + c
        };
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); nr * nc];
        for (k, (_, q)) in u.iter().enumerate() {
            if let Some(m) = members.get_mut(region(q[0], q[1])) {
                m.push(k as u32);
            }
        }
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); nr * nc];
        for r in 0..self.rows {
            for c in 0..self.cols {
                let (x, y) = (
                    self.x0 + (c as f64 + 0.5) * self.cell,
                    self.y0 + (r as f64 + 0.5) * self.cell,
                );
                if let Some(v) = cells.get_mut(region(x, y)) {
                    v.push((r * self.cols + c) as u32);
                }
            }
        }
        let mut chosen: Vec<[f64; 3]> = Vec::with_capacity(64);
        for rr in 0..nr {
            for rc in 0..nc {
                let id = rr * nc + rc;
                let (bx0, by0) = (self.x0 + rc as f64 * rw, self.y0 + rr as f64 * rh);
                let (bx1, by1) = (bx0 + rw, by0 + rh);
                chosen.clear();
                chosen.extend(
                    members
                        .get(id)
                        .into_iter()
                        .flatten()
                        .filter_map(|&k| u.get(k as usize).map(|p| p.1)),
                );
                // Neighbours ring by ring, nearest to the region first.
                let mut ring = 0usize;
                let mut pool: Vec<(f64, [f64; 3])> = Vec::new();
                let mut want = PTS_PER_REGION.saturating_sub(chosen.len());
                let spline = loop {
                    while want > 0 {
                        if pool.is_empty() {
                            ring += 1;
                            if ring > nr.max(nc) {
                                break;
                            }
                            for (r, c) in ring_cells(rr, rc, ring, nr, nc) {
                                for &k in members.get(r * nc + c).into_iter().flatten() {
                                    if let Some((_, q)) = u.get(k as usize) {
                                        let d = (bx0 - q[0])
                                            .max(q[0] - bx1)
                                            .max(0.0)
                                            .max((by0 - q[1]).max(q[1] - by1).max(0.0));
                                        pool.push((d, *q));
                                    }
                                }
                            }
                            // MCC-LIDAR means to take the nearest first, but its sort is empty
                            // (`std::sort(begin, begin)`): points come in ring order. Kept so,
                            // to classify alike; taken from the end.
                            pool.reverse();
                            continue;
                        }
                        if let Some((_, q)) = pool.pop() {
                            chosen.push(q);
                            want -= 1;
                        }
                    }
                    if chosen.is_empty() {
                        break None;
                    }
                    match Spline::fit(&chosen) {
                        Some(s) => break Some(s),
                        None if chosen.len() < MAX_SPLINE_PTS
                            && (!pool.is_empty() || ring <= nr.max(nc)) =>
                        {
                            want = 1
                        }
                        None => break None,
                    }
                };
                let Some(spline) = spline else { continue };
                for &ci in cells.get(id).into_iter().flatten() {
                    let ci = ci as usize;
                    let (x, y) = (
                        self.x0 + ((ci % self.cols) as f64 + 0.5) * self.cell,
                        self.y0 + ((ci / self.cols) as f64 + 0.5) * self.cell,
                    );
                    if let Some(z) = self.z.get_mut(ci) {
                        *z = spline.at(x, y);
                    }
                }
            }
        }
    }

    /// Replaces every valued cell by the mean of the valued cells in its 3 x 3 window.
    fn average3(&mut self) {
        let src = self.z.clone();
        for r in 0..self.rows {
            for c in 0..self.cols {
                let i = r * self.cols + c;
                if src.get(i).is_none_or(|v| v.is_nan()) {
                    continue;
                }
                let (mut s, mut n) = (0.0, 0u32);
                for rr in r.saturating_sub(1)..=(r + 1).min(self.rows - 1) {
                    for cc in c.saturating_sub(1)..=(c + 1).min(self.cols - 1) {
                        if let Some(v) = src.get(rr * self.cols + cc).filter(|v| !v.is_nan()) {
                            s += v;
                            n += 1;
                        }
                    }
                }
                if let Some(z) = self.z.get_mut(i) {
                    *z = s / f64::from(n);
                }
            }
        }
    }
}

/// The regions on the square ring `d` regions away from `(r, c)` (rows grow northward), in
/// MCC-LIDAR's order: from the north-west corner, east along the north edge, south along the
/// east edge, west along the south edge and north along the west edge, ending at the corner.
fn ring_cells(r: usize, c: usize, d: usize, nr: usize, nc: usize) -> Vec<(usize, usize)> {
    let (r, c, d) = (r as isize, c as isize, d as isize);
    let (mut rr, mut cc) = (r + d, c - d);
    let mut out = Vec::new();
    for (dr, dc) in [(0, 1), (-1, 0), (0, -1), (1, 0)] {
        for _ in 0..2 * d {
            rr += dr;
            cc += dc;
            if rr >= 0 && cc >= 0 && (rr as usize) < nr && (cc as usize) < nc {
                out.push((rr as usize, cc as usize));
            }
        }
    }
    out
}

/// An interpolating thin-plate spline z(x, y) = a0 + a1 x + a2 y + Σ wi U(|p - pi|), U(r) = r² ln r.
struct Spline {
    pts: Vec<[f64; 2]>,
    w: Vec<f64>,
    a: [f64; 3],
    cx: f64,
    cy: f64,
}

#[inline]
fn u_fn(r2: f64) -> f64 {
    if r2 > 0.0 { 0.5 * r2 * r2.ln() } else { 0.0 }
}

impl Spline {
    /// None when the points don't determine a spline (too few, or collinear).
    fn fit(pts: &[[f64; 3]]) -> Option<Self> {
        let n = pts.len();
        if n < 3 {
            return None;
        }
        let (cx, cy) = (
            pts.iter().map(|p| p[0]).sum::<f64>() / n as f64,
            pts.iter().map(|p| p[1]).sum::<f64>() / n as f64,
        );
        let xy: Vec<[f64; 2]> = pts.iter().map(|p| [p[0] - cx, p[1] - cy]).collect();
        let m = n + 3;
        let mut a = vec![0.0f64; m * m];
        let mut b = vec![0.0f64; m];
        for i in 0..n {
            for j in 0..n {
                let (dx, dy) = (xy[i][0] - xy[j][0], xy[i][1] - xy[j][1]);
                a[i * m + j] = u_fn(dx * dx + dy * dy);
            }
            for (k, v) in [1.0, xy[i][0], xy[i][1]].into_iter().enumerate() {
                a[i * m + n + k] = v;
                a[(n + k) * m + i] = v;
            }
            b[i] = pts[i][2];
        }
        let sol = solve(&mut a, &mut b, m)?;
        Some(Self {
            w: sol[..n].to_vec(),
            a: [sol[n], sol[n + 1], sol[n + 2]],
            pts: xy,
            cx,
            cy,
        })
    }

    fn at(&self, x: f64, y: f64) -> f64 {
        let (x, y) = (x - self.cx, y - self.cy);
        let mut z = self.a[0] + self.a[1] * x + self.a[2] * y;
        for (p, w) in self.pts.iter().zip(&self.w) {
            let (dx, dy) = (x - p[0], y - p[1]);
            z += w * u_fn(dx * dx + dy * dy);
        }
        z
    }
}

/// Gaussian elimination with partial pivoting; None when singular.
#[allow(clippy::needless_range_loop)]
fn solve(a: &mut [f64], b: &mut [f64], m: usize) -> Option<Vec<f64>> {
    let scale = a.iter().fold(0.0f64, |s, v| s.max(v.abs())).max(1.0);
    for col in 0..m {
        let piv =
            (col..m).max_by(|&i, &j| a[i * m + col].abs().total_cmp(&a[j * m + col].abs()))?;
        if a[piv * m + col].abs() <= 1e-12 * scale {
            return None;
        }
        if piv != col {
            for k in 0..m {
                a.swap(piv * m + k, col * m + k);
            }
            b.swap(piv, col);
        }
        for row in col + 1..m {
            let f = a[row * m + col] / a[col * m + col];
            if f != 0.0 {
                for k in col..m {
                    a[row * m + k] -= f * a[col * m + k];
                }
                b[row] -= f * b[col];
            }
        }
    }
    let mut x = vec![0.0; m];
    for row in (0..m).rev() {
        let s: f64 = (row + 1..m).map(|k| a[row * m + k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row * m + row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_interpolates_its_points_and_planes() {
        let pts: Vec<[f64; 3]> = (0..12)
            .map(|i| {
                let (x, y) = (f64::from(i % 4), f64::from(i / 4));
                [x, y, 2.0 + 0.5 * x - y]
            })
            .collect();
        let s = Spline::fit(&pts).unwrap_or_else(|| Spline {
            pts: vec![],
            w: vec![],
            a: [0.0; 3],
            cx: 0.0,
            cy: 0.0,
        });
        assert!((s.at(1.0, 1.0) - 1.5).abs() < 1e-6);
        // A plane is reproduced between the points too.
        assert!((s.at(1.5, 0.5) - 2.25).abs() < 1e-6);
    }

    #[test]
    fn tree_points_are_not_ground() {
        let mut pts = Vec::new();
        for i in 0..40 {
            for j in 0..40 {
                pts.push(Point {
                    x: f64::from(i),
                    y: f64::from(j),
                    z: 100.0 + 0.05 * f64::from(i),
                    ..Default::default()
                });
            }
        }
        // A crown 10 m up over the middle.
        for i in 15..25 {
            for j in 15..25 {
                pts.push(Point {
                    x: f64::from(i) + 0.5,
                    y: f64::from(j) + 0.5,
                    z: 110.0,
                    ..Default::default()
                });
            }
        }
        let g = ground(
            &pts,
            &MccParams {
                spacing: 1.0,
                threshold: 0.3,
            },
        );
        assert!(g[..1600].iter().filter(|g| **g).count() > 1550);
        assert!(g[1600..].iter().all(|g| !g));
    }
}
