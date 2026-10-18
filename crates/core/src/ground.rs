//! Bare-earth extraction with the Kraus & Pfeifer (1998) filter as FUSION's GroundFilter
//! implements it, done in streaming passes: each pass reads the cloud once and keeps two sums
//! per surface node, so memory depends on the area, never on the number of points.
//!
//! The intermediate surface is a grid of nodes `filter_cell` apart: each node holds the
//! weighted mean elevation of the points nearest to it, short gaps are filled from the nearest
//! nodes in 8 directions, and a point's residual is its elevation minus the surface
//! interpolated bilinearly under it.

use crate::grid::{Grid, MeanGrid};
use crate::las::{Cloud, Point};
use crate::{Error, Result};

/// The filter's parameters (FUSION's GroundFilter defaults).
#[derive(Clone, Debug, PartialEq)]
pub struct GroundParams {
    /// Spacing of the intermediate surface's nodes.
    pub filter_cell: f64,
    /// Residual at or below which a point gets full weight.
    pub g: f64,
    /// Width above `g` over which the weight falls to 0.
    pub w: f64,
    pub a: f64,
    pub b: f64,
    pub iterations: u32,
    /// Ground = within this distance of the final surface (default: residual <= g + w).
    pub tolerance: Option<f64>,
}

impl Default for GroundParams {
    fn default() -> Self {
        Self {
            filter_cell: 5.0,
            g: -2.0,
            w: 2.5,
            a: 1.0,
            b: 4.0,
            iterations: 5,
            tolerance: None,
        }
    }
}

impl GroundParams {
    /// The weight of a residual `v` (point minus surface), as GroundFilter computes it.
    #[inline]
    pub fn weight(&self, v: f64) -> f64 {
        if v <= self.g {
            1.0
        } else if v > self.g + self.w {
            0.0
        } else {
            1.0 / (self.a * (v - self.g).powf(self.b) + 1.0)
        }
    }

    fn validate(&self) -> Result<()> {
        if !(self.filter_cell > 0.0)
            || !(self.w > 0.0)
            || self.iterations == 0
            || !self.a.is_finite()
            || !self.b.is_finite()
        {
            return Err(Error::Param(
                "the ground filter needs a positive filter cell and w, and at least 1 iteration"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Gaps up to this many nodes from a valued node are filled before each pass.
const FILL_STEPS: usize = 2;

/// The intermediate surface: node values, NaN where no point weighed in.
struct Surface {
    x0: f64,
    y0: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    z: Vec<f32>,
}

impl Surface {
    /// Nodes from the cloud's south-west corner, covering it.
    fn new(b: [f64; 6], cell: f64) -> Result<Self> {
        let cols = ((b[3] - b[0] + cell / 2.0) / cell) as usize + 1;
        let rows = ((b[4] - b[1] + cell / 2.0) / cell) as usize + 1;
        let n = cols
            .checked_mul(rows)
            .filter(|n| *n <= 1 << 29)
            .ok_or_else(|| Error::Param("the filter cell is too small for this area".into()))?;
        Ok(Self {
            x0: b[0],
            y0: b[1],
            cell,
            cols,
            rows,
            z: vec![f32::NAN; n],
        })
    }

    /// The node nearest a point (row 0 is the south).
    #[inline]
    fn node(&self, x: f64, y: f64) -> usize {
        let c = (((x - self.x0) / self.cell + 0.5).max(0.0) as usize).min(self.cols - 1);
        let r = (((y - self.y0) / self.cell + 0.5).max(0.0) as usize).min(self.rows - 1);
        r * self.cols + c
    }

    /// The surface under a point, bilinear between the 4 nodes around it; None over a gap.
    #[inline]
    fn at(&self, x: f64, y: f64) -> Option<f64> {
        let (fx, fy) = ((x - self.x0) / self.cell, (y - self.y0) / self.cell);
        let (c, r) = (fx.floor(), fy.floor());
        if c < 0.0 || r < 0.0 || c > (self.cols - 1) as f64 || r > (self.rows - 1) as f64 {
            return None;
        }
        let (c, r) = (c as usize, r as usize);
        let (c1, r1) = ((c + 1).min(self.cols - 1), (r + 1).min(self.rows - 1));
        let v = |c: usize, r: usize| self.z.get(r * self.cols + c).copied().map(f64::from);
        let (z00, z10, z01, z11) = (v(c, r)?, v(c1, r)?, v(c, r1)?, v(c1, r1)?);
        if [z00, z10, z01, z11].iter().any(|z| z.is_nan()) {
            return None;
        }
        let (tx, ty) = (fx - c as f64, fy - r as f64);
        let south = z00 + (z10 - z00) * tx;
        let north = z01 + (z11 - z01) * tx;
        Some(south + (north - south) * ty)
    }

    /// Fills gaps from the first valued node within [`FILL_STEPS`] in each of 8 directions,
    /// weighted by inverse distance; wider gaps stay empty.
    fn fill_gaps(&mut self) {
        let snap = self.z.clone();
        let (cols, rows) = (self.cols as isize, self.rows as isize);
        for r in 0..rows {
            for c in 0..cols {
                let i = (r * cols + c) as usize;
                if !snap.get(i).is_some_and(|v| v.is_nan()) {
                    continue;
                }
                let (mut sw, mut swz) = (0.0f32, 0.0f32);
                for (dc, dr) in [
                    (0, -1),
                    (0, 1),
                    (-1, 0),
                    (1, 0),
                    (-1, -1),
                    (1, -1),
                    (-1, 1),
                    (1, 1),
                ] {
                    let unit = if dc != 0 && dr != 0 {
                        std::f32::consts::SQRT_2
                    } else {
                        1.0
                    };
                    for step in 1..=FILL_STEPS as isize {
                        let (nc, nr) = (c + dc * step, r + dr * step);
                        if nc < 0 || nr < 0 || nc >= cols || nr >= rows {
                            break;
                        }
                        if let Some(&v) =
                            snap.get((nr * cols + nc) as usize).filter(|v| !v.is_nan())
                        {
                            let w = 1.0 / (step as f32 * unit);
                            sw += w;
                            swz += w * v;
                            break;
                        }
                    }
                }
                if sw > 0.0
                    && let Some(z) = self.z.get_mut(i)
                {
                    *z = swz / sw;
                }
            }
        }
    }
}

/// What the filter did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroundReport {
    pub points: u64,
    pub ground: u64,
    pub passes: u32,
}

/// Filters the ground and grids it onto a DTM with cells of `cell`: each cell is the mean
/// elevation of its ground points, and empty cells are filled from their neighbours.
pub fn dtm<C: Cloud>(cloud: &C, cell: f64, p: &GroundParams) -> Result<(Grid, GroundReport)> {
    let (filter, passes) = GroundFilter::run(cloud, p)?;
    let b = cloud.bounds();
    let mut mean = MeanGrid::new(Grid::covering([b[0], b[1], b[3], b[4]], cell)?, b[2]);
    let mut report = GroundReport {
        passes: passes + 1,
        ..Default::default()
    };
    report.points = cloud.scan(|pt| {
        if !pt.is_noise() && filter.is_ground(pt) {
            mean.add(pt.x, pt.y, pt.z);
            report.ground += 1;
        }
    })?;
    if report.ground == 0 {
        return Err(Error::Param(
            "no point was classified as ground: check g, w and the filter cell".into(),
        ));
    }
    let mut dtm = mean.finish();
    dtm.fill_holes();
    Ok((dtm, report))
}

/// The filter's final surface, which tells ground points apart.
pub struct GroundFilter {
    s: Surface,
    p: GroundParams,
}

impl GroundFilter {
    /// Runs the filter over a cloud; the filter and how many passes it read.
    pub fn run<C: Cloud>(cloud: &C, p: &GroundParams) -> Result<(Self, u32)> {
        p.validate()?;
        let mut s = Surface::new(cloud.bounds(), p.filter_cell)?;
        let mut sum_wz = vec![0.0f64; s.z.len()];
        let mut sum_w = vec![0.0f32; s.z.len()];
        let mut passes = 0;
        // Pass 0 weighs every point alike; then each pass reweighs against the last surface.
        for pass in 0..=p.iterations {
            if pass > 0 {
                s.fill_gaps();
            }
            sum_wz.fill(0.0);
            sum_w.fill(0.0);
            cloud.scan(|pt| {
                if pt.is_noise() {
                    return;
                }
                let w = if pass == 0 {
                    1.0
                } else {
                    match s.at(pt.x, pt.y) {
                        Some(sz) => p.weight(pt.z - sz),
                        None => 0.0,
                    }
                };
                if w > 0.0 {
                    let i = s.node(pt.x, pt.y);
                    if let (Some(a), Some(b)) = (sum_wz.get_mut(i), sum_w.get_mut(i)) {
                        *a += w * pt.z;
                        *b += w as f32;
                    }
                }
            })?;
            passes += 1;
            for ((z, a), b) in s.z.iter_mut().zip(&sum_wz).zip(&sum_w) {
                *z = if *b > 0.0 {
                    (*a / f64::from(*b)) as f32
                } else {
                    f32::NAN
                };
            }
        }
        s.fill_gaps();
        Ok((Self { s, p: p.clone() }, passes))
    }

    /// Whether a point lies on the ground: at most g + w above the surface (or within the
    /// tolerance of it).
    #[inline]
    pub fn is_ground(&self, pt: &Point) -> bool {
        match self.s.at(pt.x, pt.y) {
            Some(sz) => match self.p.tolerance {
                Some(t) => (pt.z - sz).abs() <= t,
                None => pt.z - sz <= self.p.g + self.p.w,
            },
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_follows_groundfilter() {
        let p = GroundParams::default();
        assert_eq!(p.weight(-3.0), 1.0);
        assert_eq!(p.weight(-2.0), 1.0);
        assert_eq!(p.weight(0.6), 0.0);
        // Halfway: 1 / (1 * 1^4 + 1).
        assert!((p.weight(-1.0) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn surface_interpolates_and_fills_short_gaps() -> Result<()> {
        let mut s = Surface::new([0.0, 0.0, 0.0, 20.0, 0.0, 0.0], 5.0)?;
        assert_eq!((s.cols, s.rows), (5, 1));
        s.z = vec![10.0, f32::NAN, f32::NAN, f32::NAN, 20.0];
        s.fill_gaps();
        // Node 1 sees node 0 at 1 step and node 4 at 3 (beyond reach); node 2 sees both at 2.
        assert_eq!(s.z[1], 10.0);
        assert_eq!(s.z[2], 15.0);
        assert_eq!(s.z[3], 20.0);
        assert_eq!(s.at(2.5, 0.0), Some(10.0));
        assert_eq!(s.node(2.6, 0.0), 1);
        Ok(())
    }
}
