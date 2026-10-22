//! Terrain analysis on a DTM (or any height grid): slope, aspect, hillshade, curvatures,
//! roughness, ruggedness and position indices (3x3 windows), the drainage products of
//! [`crate::hydro`] and the viewshed of [`crate::viewshed`]. Each product is one GeoTIFF the
//! size of the input; the grid is read once and every window runs in parallel by rows.
//!
//! The methods are the published ones: Horn (1981) for slope, aspect and hillshade, Zevenbergen
//! and Thorne (1987) for curvatures, Wilson et al. (2007) for the ruggedness index, and the
//! position index of Weiss (2001). Conventions follow `gdaldem`: windows over empty or missing
//! neighbours take the centre's height, and a cell that is empty stays empty. Written from the
//! published methods; no source code of other programs is included (see the NOTICE).

use std::path::PathBuf;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::{Error, Result};

/// What to compute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Product {
    Slope,
    Aspect,
    Hillshade,
    /// Curvature across the slope (positive: convex, as on a spur).
    PlanCurvature,
    /// Curvature along the slope (positive: convex, as on a shoulder).
    ProfileCurvature,
    /// Total curvature (positive: convex, ridges; negative: concave, valleys).
    Curvature,
    /// Highest minus lowest height in the 3x3 window.
    Roughness,
    /// Terrain ruggedness: mean absolute difference to the eight neighbours.
    Ruggedness,
    /// Height minus the mean height within a radius.
    Position,
    /// How deep the depressions are that filling them removes.
    FillDepth,
    /// Drained area, in m².
    FlowAccumulation,
    /// Topographic wetness index `ln(a / tan β)`.
    Wetness,
    /// Cells whose drained area reaches the threshold.
    Streams,
    /// Catchments of the stream reaches, numbered.
    Subbasins,
    Viewshed,
}

impl Product {
    pub const ALL: [Product; 15] = [
        Product::Slope,
        Product::Aspect,
        Product::Hillshade,
        Product::PlanCurvature,
        Product::ProfileCurvature,
        Product::Curvature,
        Product::Roughness,
        Product::Ruggedness,
        Product::Position,
        Product::FillDepth,
        Product::FlowAccumulation,
        Product::Wetness,
        Product::Streams,
        Product::Subbasins,
        Product::Viewshed,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Product::Slope => "Pendiente",
            Product::Aspect => "Orientación",
            Product::Hillshade => "Sombreado",
            Product::PlanCurvature => "Curvatura en planta",
            Product::ProfileCurvature => "Curvatura de perfil",
            Product::Curvature => "Curvatura total",
            Product::Roughness => "Rugosidad",
            Product::Ruggedness => "Índice de rugosidad (TRI)",
            Product::Position => "Posición topográfica (TPI)",
            Product::FillDepth => "Profundidad de depresiones",
            Product::FlowAccumulation => "Acumulación de flujo",
            Product::Wetness => "Índice de humedad (TWI)",
            Product::Streams => "Red de drenaje",
            Product::Subbasins => "Subcuencas",
            Product::Viewshed => "Cuenca visual",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Product::Slope => "pendiente",
            Product::Aspect => "orientacion",
            Product::Hillshade => "sombreado",
            Product::PlanCurvature => "curv_planta",
            Product::ProfileCurvature => "curv_perfil",
            Product::Curvature => "curvatura",
            Product::Roughness => "rugosidad",
            Product::Ruggedness => "tri",
            Product::Position => "tpi",
            Product::FillDepth => "depresiones",
            Product::FlowAccumulation => "acumulacion",
            Product::Wetness => "twi",
            Product::Streams => "drenaje",
            Product::Subbasins => "subcuencas",
            Product::Viewshed => "visibilidad",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TerrainJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub product: Product,
    /// Slope in percent instead of degrees.
    pub percent: bool,
    /// Multiplies heights when they are not in the units of the cell (1 for metres).
    pub z_factor: f64,
    /// Hillshade: direction of the light, degrees clockwise from north.
    pub azimuth: f64,
    /// Hillshade: height of the light over the horizon, degrees.
    pub altitude: f64,
    /// Position index: radius of the neighbourhood, map units.
    pub radius: f64,
    /// Streams and subbasins: drained area that makes a stream, m².
    pub threshold: f64,
    /// Viewshed: the observer (x, y).
    pub observer: Option<[f64; 2]>,
    /// Viewshed: eye height over the ground.
    pub observer_height: f64,
    /// Viewshed: height over the ground that must be seen.
    pub target_height: f64,
    /// Viewshed: how far the observer looks, map units.
    pub max_distance: f64,
}

impl TerrainJob {
    pub fn new(input: PathBuf, output: PathBuf, product: Product) -> Self {
        Self {
            input,
            output,
            product,
            percent: false,
            z_factor: 1.0,
            azimuth: 315.0,
            altitude: 45.0,
            radius: 10.0,
            threshold: 10_000.0,
            observer: None,
            observer_height: 1.7,
            target_height: 0.0,
            max_distance: 1000.0,
        }
    }
}

/// Computes the product on a grid in memory.
pub fn compute(g: &Grid, job: &TerrainJob) -> Result<Grid> {
    if !(g.cell > 1e-3) {
        return Err(Error::Param(
            "la celda mide menos de 1 mm: el ráster parece estar en grados; reproyéctalo a un sistema métrico (UTM)"
                .into(),
        ));
    }
    if !(job.z_factor > 0.0) {
        return Err(Error::Param("el factor Z debe ser positivo".into()));
    }
    crate::check_cancel()?;
    let z = job.z_factor;
    Ok(match job.product {
        Product::Slope => slope(g, z, job.percent),
        Product::Aspect => aspect(g, z),
        Product::Hillshade => hillshade(g, z, job.azimuth, job.altitude),
        Product::PlanCurvature => curvature(g, z, Curv::Plan),
        Product::ProfileCurvature => curvature(g, z, Curv::Profile),
        Product::Curvature => curvature(g, z, Curv::Total),
        Product::Roughness => roughness(g, z),
        Product::Ruggedness => ruggedness(g, z),
        Product::Position => position(g, z, job.radius)?,
        Product::FillDepth => crate::hydro::fill_depth(g, z),
        Product::FlowAccumulation => crate::hydro::flow_accumulation(g, z),
        Product::Wetness => crate::hydro::wetness(g, z),
        Product::Streams => crate::hydro::streams(g, z, job.threshold)?,
        Product::Subbasins => crate::hydro::subbasins(g, z, job.threshold)?,
        Product::Viewshed => {
            let obs = job.observer.ok_or_else(|| {
                Error::Param("la cuenca visual necesita la posición del observador".into())
            })?;
            crate::viewshed::viewshed(
                g,
                z,
                obs,
                job.observer_height,
                job.target_height,
                job.max_distance,
            )?
        }
    })
}

/// Reads the input, computes the product and writes it as a GeoTIFF beside the input's system.
pub fn run(job: &TerrainJob) -> Result<String> {
    let g = Grid::read(&job.input)?;
    let out = compute(&g, job)?;
    crate::paths::ensure_parent(&job.output)?;
    out.write_tiff(&job.output, Grid::read_epsg(&job.input))?;
    let (mut lo, mut hi, mut n) = (f32::INFINITY, f32::NEG_INFINITY, 0usize);
    for v in out.data.iter().filter(|v| !v.is_nan()) {
        lo = lo.min(*v);
        hi = hi.max(*v);
        n += 1;
    }
    Ok(if n == 0 {
        format!("{}: sin celdas con valor", job.product.name())
    } else {
        format!(
            "{}: {n} celdas, de {lo:.4} a {hi:.4} -> {}",
            job.product.name(),
            job.output.display()
        )
    })
}

/// The 3x3 window around `(c, r)`, row by row from the north: `[a b c / d e f / g h i]`.
/// Neighbours past the edge or empty take the centre's height. `None` if the centre is empty.
#[inline]
fn window(g: &Grid, c: usize, r: usize) -> Option<[f32; 9]> {
    let e = *g.data.get(r * g.cols + c)?;
    if e.is_nan() {
        return None;
    }
    let mut w = [e; 9];
    for (k, slot) in w.iter_mut().enumerate() {
        let rr = (r + k / 3).saturating_sub(1).min(g.rows - 1);
        let cc = (c + k % 3).saturating_sub(1).min(g.cols - 1);
        if let Some(v) = g.data.get(rr * g.cols + cc).filter(|v| !v.is_nan()) {
            *slot = *v;
        }
    }
    Some(w)
}

/// Applies `f` to the window of every cell, rows in parallel.
fn map3(g: &Grid, f: impl Fn(&[f32; 9]) -> f32 + Sync) -> Grid {
    let mut out = g.like();
    out.data
        .par_chunks_mut(g.cols)
        .enumerate()
        .for_each(|(r, row)| {
            for (c, v) in row.iter_mut().enumerate() {
                if let Some(w) = window(g, c, r) {
                    *v = f(&w);
                }
            }
        });
    out
}

/// Horn's gradient: (east, north) rise per unit of distance.
#[inline]
fn gradient(w: &[f32; 9], cell: f64, z: f64) -> (f64, f64) {
    let v = |k: usize| f64::from(w[k]);
    let k = z / (8.0 * cell);
    (
        ((v(2) + 2.0 * v(5) + v(8)) - (v(0) + 2.0 * v(3) + v(6))) * k,
        ((v(0) + 2.0 * v(1) + v(2)) - (v(6) + 2.0 * v(7) + v(8))) * k,
    )
}

pub fn slope(g: &Grid, z: f64, percent: bool) -> Grid {
    let cell = g.cell;
    map3(g, |w| {
        let (gx, gy) = gradient(w, cell, z);
        let s = gx.hypot(gy);
        (if percent {
            100.0 * s
        } else {
            s.atan().to_degrees()
        }) as f32
    })
}

/// Direction the slope faces, degrees clockwise from north; flat cells are empty.
pub fn aspect(g: &Grid, z: f64) -> Grid {
    let cell = g.cell;
    map3(g, |w| {
        let (gx, gy) = gradient(w, cell, z);
        if gx.hypot(gy) < 1e-9 {
            return f32::NAN;
        }
        // downhill is (-gx, -gy) in (east, north)
        let a = (-gx).atan2(-gy).to_degrees();
        (if a < 0.0 { a + 360.0 } else { a + 0.0 }) as f32
    })
}

/// Illumination 0-255 from a light at `azimuth` and `altitude` (degrees).
pub fn hillshade(g: &Grid, z: f64, azimuth: f64, altitude: f64) -> Grid {
    let cell = g.cell;
    let (az, alt) = (azimuth.to_radians(), altitude.to_radians());
    let (lx, ly, lz) = (az.sin() * alt.cos(), az.cos() * alt.cos(), alt.sin());
    map3(g, |w| {
        let (gx, gy) = gradient(w, cell, z);
        let shade = (lz - gx * lx - gy * ly) / (1.0 + gx * gx + gy * gy).sqrt();
        (255.0 * shade.max(0.0)) as f32
    })
}

#[derive(Clone, Copy)]
enum Curv {
    Plan,
    Profile,
    Total,
}

/// Zevenbergen and Thorne's curvatures, in 1/m; positive is convex.
fn curvature(g: &Grid, z: f64, kind: Curv) -> Grid {
    let l = g.cell;
    map3(g, |w| {
        let v = |k: usize| f64::from(w[k]) * z;
        let d = ((v(3) + v(5)) / 2.0 - v(4)) / (l * l);
        let e = ((v(1) + v(7)) / 2.0 - v(4)) / (l * l);
        let f = (v(2) - v(0) - v(8) + v(6)) / (4.0 * l * l);
        let gg = (v(5) - v(3)) / (2.0 * l);
        let h = (v(1) - v(7)) / (2.0 * l);
        let s = gg * gg + h * h;
        let c = match kind {
            Curv::Total => -2.0 * (d + e),
            _ if s < 1e-12 => 0.0,
            Curv::Profile => -2.0 * (d * gg * gg + e * h * h + f * gg * h) / s,
            Curv::Plan => -2.0 * (d * h * h + e * gg * gg - f * gg * h) / s,
        };
        c as f32
    })
}

pub fn roughness(g: &Grid, z: f64) -> Grid {
    map3(g, |w| {
        let (lo, hi) = w.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |a, v| {
            (a.0.min(*v), a.1.max(*v))
        });
        ((hi - lo) as f64 * z) as f32
    })
}

pub fn ruggedness(g: &Grid, z: f64) -> Grid {
    map3(g, |w| {
        let sum: f32 = w.iter().map(|v| (v - w[4]).abs()).sum();
        (f64::from(sum) / 8.0 * z) as f32
    })
}

/// Height minus the mean height of the cells within `radius` (the centre excluded). Each row of
/// the disc is taken from running sums, so the cost grows with the radius, not its square.
pub fn position(g: &Grid, z: f64, radius: f64) -> Result<Grid> {
    if !(radius >= g.cell) {
        return Err(Error::Param(
            "el radio de la posición topográfica debe ser al menos una celda".into(),
        ));
    }
    let rc = (radius / g.cell).round().max(1.0) as isize;
    if rc as usize > g.cols.max(g.rows) {
        return Err(Error::Param("el radio es mayor que el ráster".into()));
    }
    let stride = g.cols + 1;
    // running sums along each row, of the heights and of how many cells have one
    let mut sum = vec![0f64; g.rows * stride];
    let mut cnt = vec![0u32; g.rows * stride];
    sum.par_chunks_mut(stride)
        .zip(cnt.par_chunks_mut(stride))
        .enumerate()
        .for_each(|(r, (s, n))| {
            for c in 0..g.cols {
                let v = g.data[r * g.cols + c];
                let ok = !v.is_nan();
                s[c + 1] = s[c] + if ok { f64::from(v) } else { 0.0 };
                n[c + 1] = n[c] + u32::from(ok);
            }
        });
    let half: Vec<isize> = (-rc..=rc)
        .map(|dy| ((rc * rc - dy * dy) as f64).sqrt().floor() as isize)
        .collect();
    let mut out = g.like();
    out.data
        .par_chunks_mut(g.cols)
        .enumerate()
        .for_each(|(r, row)| {
            for (c, o) in row.iter_mut().enumerate() {
                let e = g.data[r * g.cols + c];
                if e.is_nan() {
                    continue;
                }
                let (mut s, mut n) = (-f64::from(e), -1i64);
                for (k, hw) in half.iter().enumerate() {
                    let rr = r as isize + k as isize - rc;
                    if rr < 0 || rr >= g.rows as isize {
                        continue;
                    }
                    let c0 = (c as isize - hw).max(0) as usize;
                    let c1 = ((c as isize + hw + 1) as usize).min(g.cols);
                    let base = rr as usize * stride;
                    s += sum[base + c1] - sum[base + c0];
                    n += i64::from(cnt[base + c1] - cnt[base + c0]);
                }
                if n > 0 {
                    *o = (f64::from(e) * z - s / n as f64 * z) as f32;
                }
            }
        });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of 1 m cells whose height is `f(x, y)` at the cell centres.
    fn surface(cols: usize, rows: usize, f: impl Fn(f64, f64) -> f64) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 1.0, cols, rows)?;
        for r in 0..rows {
            for c in 0..cols {
                let (x, y) = g.center(c, r);
                g.data[r * cols + c] = f(x, y) as f32;
            }
        }
        Ok(g)
    }

    fn mid(g: &Grid) -> f32 {
        g.data[(g.rows / 2) * g.cols + g.cols / 2]
    }

    #[test]
    fn slope_of_a_plane_is_exact() -> Result<()> {
        // rises 0.5 m per metre to the east
        let g = surface(9, 9, |x, _| 0.5 * x)?;
        assert!((mid(&slope(&g, 1.0, false)) - 0.5f32.atan().to_degrees()).abs() < 1e-4);
        assert!((mid(&slope(&g, 1.0, true)) - 50.0).abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn aspect_is_the_compass_direction_downhill() -> Result<()> {
        // rises to the west: faces east (90); rises to the south: faces north (0)
        let east = surface(9, 9, |x, _| -x)?;
        assert!((mid(&aspect(&east, 1.0)) - 90.0).abs() < 1e-3);
        let north = surface(9, 9, |_, y| -y)?;
        let a = mid(&aspect(&north, 1.0));
        assert!(a < 1e-3 || a > 359.999, "{a}");
        let west = surface(9, 9, |x, _| x)?;
        assert!((mid(&aspect(&west, 1.0)) - 270.0).abs() < 1e-3);
        let flat = surface(9, 9, |_, _| 5.0)?;
        assert!(mid(&aspect(&flat, 1.0)).is_nan());
        Ok(())
    }

    #[test]
    fn hillshade_is_bright_toward_the_light() -> Result<()> {
        // slope facing north-west with the light at north-west (315), 45 deg
        let nw = surface(9, 9, |x, y| 0.5 * (x - y))?;
        let toward = mid(&hillshade(&nw, 1.0, 315.0, 45.0));
        let away = mid(&hillshade(&nw, 1.0, 135.0, 45.0));
        assert!(toward > away + 100.0, "{toward} {away}");
        // flat ground: 255 * sin(altitude)
        let flat = surface(9, 9, |_, _| 3.0)?;
        assert!(
            (mid(&hillshade(&flat, 1.0, 315.0, 45.0)) - 255.0 * 45f32.to_radians().sin()).abs()
                < 1e-3
        );
        Ok(())
    }

    #[test]
    fn curvature_is_positive_on_a_dome_and_negative_in_a_bowl() -> Result<()> {
        let dome = surface(9, 9, |x, y| -0.1 * ((x - 4.5).powi(2) + (y - 4.5).powi(2)))?;
        let bowl = surface(9, 9, |x, y| 0.1 * ((x - 4.5).powi(2) + (y - 4.5).powi(2)))?;
        assert!((mid(&curvature(&dome, 1.0, Curv::Total)) - 0.4).abs() < 1e-4);
        assert!((mid(&curvature(&bowl, 1.0, Curv::Total)) + 0.4).abs() < 1e-4);
        // off the summit, on a dome both curvatures are convex
        let at = |g: &Grid, k: Curv| curvature(g, 1.0, k).data[4 * 9 + 6];
        assert!(at(&dome, Curv::Profile) > 0.0 && at(&dome, Curv::Plan) > 0.0);
        assert!(at(&bowl, Curv::Profile) < 0.0 && at(&bowl, Curv::Plan) < 0.0);
        Ok(())
    }

    #[test]
    fn roughness_ruggedness_and_position() -> Result<()> {
        let mut g = surface(7, 7, |_, _| 10.0)?;
        g.data[3 * 7 + 3] = 18.0;
        assert_eq!(mid(&roughness(&g, 1.0)), 8.0);
        assert_eq!(mid(&ruggedness(&g, 1.0)), 8.0);
        let p = position(&g, 1.0, 2.0)?;
        assert!((mid(&p) - 8.0).abs() < 1e-4);
        // a neighbour of the peak sees it in its mean
        assert!(p.data[3 * 7 + 4] < 0.0);
        Ok(())
    }

    #[test]
    fn position_matches_a_direct_disc() -> Result<()> {
        let g = surface(21, 17, |x, y| {
            (x * 0.7).sin() * 3.0 + (y * 0.4).cos() * 2.0 + 0.1 * x
        })?;
        let p = position(&g, 1.0, 3.0)?;
        for &(c, r) in &[(10usize, 8usize), (1, 1), (19, 15), (0, 8)] {
            let (mut s, mut n) = (0.0f64, 0);
            for dr in -3i32..=3 {
                for dc in -3i32..=3 {
                    if (dr, dc) == (0, 0) || dr * dr + dc * dc > 9 {
                        continue;
                    }
                    let (rr, cc) = (r as i32 + dr, c as i32 + dc);
                    if rr >= 0 && cc >= 0 && (rr as usize) < g.rows && (cc as usize) < g.cols {
                        s += f64::from(g.data[rr as usize * g.cols + cc as usize]);
                        n += 1;
                    }
                }
            }
            let want = f64::from(g.data[r * g.cols + c]) - s / f64::from(n);
            assert!(
                (f64::from(p.data[r * g.cols + c]) - want).abs() < 1e-4,
                "({c},{r})"
            );
        }
        Ok(())
    }

    #[test]
    fn empty_cells_stay_empty_and_neighbours_do_not_poison() -> Result<()> {
        let mut g = surface(5, 5, |x, _| x)?;
        g.data[2 * 5 + 2] = f32::NAN;
        assert!(mid(&slope(&g, 1.0, false)).is_nan());
        assert!(slope(&g, 1.0, false).data[2 * 5 + 1].is_finite());
        Ok(())
    }

    #[test]
    fn degrees_are_refused() -> Result<()> {
        let g = Grid::new(0.0, 0.0, 0.00001, 4, 4)?;
        let job = TerrainJob::new("a".into(), "b".into(), Product::Slope);
        assert!(compute(&g, &job).is_err());
        Ok(())
    }
}
