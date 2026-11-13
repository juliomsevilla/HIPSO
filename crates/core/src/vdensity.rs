//! Point density by Voronoi cells, for the quality control of a cloud.
//!
//! The density of a raster cell is not "points divided by area": it is `n / Σ area` of the Voronoi
//! cells of its `n` points, which does not depend on where the edge of the raster cell falls
//! among them. Cells of a point that are larger than a limit are gaps in the data (water, a roof
//! with no return) and are left out of the density, and their area is reported apart.
//!
//! The cloud is cut into tiles with a margin, so that the cells of the points at the edge of a tile
//! are right; memory follows a tile and the output rasters, not the cloud.

use std::path::PathBuf;

use crate::filter::{ClassSel, Filter, ReturnSel};
use crate::grid::Grid;
use crate::las::Reader;
use crate::pipeline::Tiling;
use crate::tile::{Layout, Tile, Tiled};
use crate::voronoi::{self, Clip};
use crate::{Error, Result};

pub struct VDensityJob {
    pub inputs: Vec<PathBuf>,
    /// Cell of the output rasters (m).
    pub cell: f64,
    pub filter: Filter,
    /// Points per m² the density must reach to pass.
    pub threshold: f64,
    /// A Voronoi cell larger than this (m²) is a gap in the data.
    pub max_cell_area: f64,
    pub tiling: Tiling,
    /// Base path of the results: `<output>.tif`, `<output>_cumple.tif`, `<output>.csv`.
    pub output: PathBuf,
    /// A raster of 1 where the density reaches the threshold and 0 where it does not.
    pub pass_fail: bool,
    pub csv: bool,
}

impl VDensityJob {
    pub fn new(inputs: Vec<PathBuf>, output: PathBuf) -> Self {
        Self {
            inputs,
            cell: 5.0,
            // the pulses: first returns only
            filter: Filter {
                returns: ReturnSel::parse("F").unwrap_or_else(|_| ReturnSel::all()),
                classes: ClassSel::all(),
                skip_noise: true,
                ..Filter::default()
            },
            threshold: 8.0,
            max_cell_area: 100.0,
            tiling: Tiling::default(),
            output,
            pass_fail: true,
            csv: true,
        }
    }
}

/// The density of the cloud.
#[derive(Clone, Debug, PartialEq)]
pub struct VDensityReport {
    pub mean: f64,
    pub median: f64,
    pub p05: f64,
    pub p95: f64,
    /// Share of the cells with a density that reach the threshold.
    pub pass_pct: f64,
    /// Area (m²) of the Voronoi cells taken for gaps.
    pub void_area: f64,
    /// Cells with a density.
    pub cells: u64,
    pub points: u64,
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

/// The density rasters and the report for `job`; the density is empty where no point had a
/// Voronoi cell small enough.
pub fn density(job: &VDensityJob) -> Result<(Grid, Option<Grid>, VDensityReport, Option<u16>)> {
    if !(job.cell > 0.0) || !(job.max_cell_area > 0.0) || !(job.threshold >= 0.0) {
        return Err(Error::Param(
            "la celda y el área máxima deben ser positivas, y el umbral no negativo".into(),
        ));
    }
    if job.inputs.is_empty() {
        return Err(Error::Param("no hay nubes de entrada".into()));
    }
    let readers: Vec<Reader> = job
        .inputs
        .iter()
        .map(|p| Reader::open(p))
        .collect::<Result<_>>()?;
    let epsg = readers.first().and_then(|r| r.header.epsg);
    let b = readers.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, r| {
            let h = r.header.bounds;
            [
                b[0].min(h[0]),
                b[1].min(h[1]),
                b[2].max(h[3]),
                b[3].max(h[4]),
            ]
        },
    );
    let layout = Layout::new(b, job.tiling.size, job.tiling.buffer, job.cell)?;
    let mut count = Grid::covering(b, job.cell)?;
    count.data.fill(0.0);
    let mut area = count.clone();
    let mut tiled = Tiled::spool(&readers, layout)?;
    tiled.keep_records = false;
    let (mut void_area, mut points) = (0.0f64, 0u64);
    let buffer = job.tiling.buffer;
    let cell = job.cell;
    tiled.process(
        job.tiling.threads,
        |tile: Tile| {
            let e = tile.core;
            let (cols, rows) = (
                ((e[2] - e[0]) / cell).round() as usize,
                ((e[3] - e[1]) / cell).round() as usize,
            );
            let mut n = Grid::new(e[0], e[1], cell, cols, rows)?;
            n.data.fill(0.0);
            let mut a = n.clone();
            let (mut sites, mut own) = (Vec::new(), Vec::new());
            for (q, is_own) in tile.points.pts.iter().zip(&tile.own) {
                if job.filter.keeps(q) && job.filter.z_ok(q.z) {
                    sites.push([q.x, q.y]);
                    own.push(*is_own);
                }
            }
            let mut void = 0.0;
            let mut pts = 0u64;
            if sites.len() >= 3 {
                let clip = Clip::Bbox([e[0] - buffer, e[1] - buffer, e[2] + buffer, e[3] + buffer]);
                let areas = match voronoi::cell_areas(&sites, &clip) {
                    Ok(a) => a,
                    // too few distinct points or all on a line: nothing to measure here
                    Err(Error::Param(_)) => vec![f64::NAN; sites.len()],
                    Err(e) => return Err(e),
                };
                for ((s, ar), is_own) in sites.iter().zip(&areas).zip(&own) {
                    if !*is_own {
                        continue;
                    }
                    pts += 1;
                    let Some(i) = n.index(s[0], s[1]) else {
                        continue;
                    };
                    if ar.is_nan() {
                        continue;
                    }
                    if *ar > job.max_cell_area {
                        void += ar;
                        continue;
                    }
                    n.data[i] += 1.0;
                    a.data[i] += *ar as f32;
                }
            }
            Ok((n, a, void, pts))
        },
        |(n, a, void, pts)| {
            void_area += void;
            points += pts;
            for row in 0..n.rows {
                for col in 0..n.cols {
                    let (x, y) = n.center(col, row);
                    if let Some(i) = count.index(x, y) {
                        count.data[i] += n.data[row * n.cols + col];
                        area.data[i] += a.data[row * n.cols + col];
                    }
                }
            }
            Ok(())
        },
    )?;
    let mut dens = count.like();
    let mut vals: Vec<f64> = Vec::new();
    for (d, (n, a)) in dens.data.iter_mut().zip(count.data.iter().zip(&area.data)) {
        if *a > 0.0 && *n > 0.0 {
            *d = (f64::from(*n) / f64::from(*a)) as f32;
            vals.push(f64::from(*d));
        }
    }
    vals.sort_by(f64::total_cmp);
    let pass = vals.iter().filter(|v| **v >= job.threshold).count();
    let passfail = job.pass_fail.then(|| {
        let mut g = dens.like();
        for (o, d) in g.data.iter_mut().zip(&dens.data) {
            if !d.is_nan() {
                *o = if f64::from(*d) >= job.threshold {
                    1.0
                } else {
                    0.0
                };
            }
        }
        g
    });
    let report = VDensityReport {
        mean: if vals.is_empty() {
            f64::NAN
        } else {
            vals.iter().sum::<f64>() / vals.len() as f64
        },
        median: percentile(&vals, 0.5),
        p05: percentile(&vals, 0.05),
        p95: percentile(&vals, 0.95),
        pass_pct: if vals.is_empty() {
            f64::NAN
        } else {
            100.0 * pass as f64 / vals.len() as f64
        },
        void_area,
        cells: vals.len() as u64,
        points,
    };
    Ok((dens, passfail, report, epsg))
}

/// Computes the density and writes what was asked; returns the summary.
pub fn run(job: &VDensityJob) -> Result<String> {
    let (dens, passfail, rep, epsg) = density(job)?;
    crate::paths::ensure_parent(&job.output)?;
    let tif = crate::paths::swap_ext(&job.output, "tif");
    dens.write_tiff(&tif, epsg)?;
    let mut lines = vec![format!(
        "densidad de Voronoi en {} celdas de {} m ({} puntos): media {:.2}, mediana {:.2}, P5 {:.2}, P95 {:.2} puntos/m²; {:.1} % de las celdas llega a {} puntos/m²; {:.0} m² sin datos (celdas de más de {} m²) -> {}",
        rep.cells,
        job.cell,
        rep.points,
        rep.mean,
        rep.median,
        rep.p05,
        rep.p95,
        rep.pass_pct,
        job.threshold,
        rep.void_area,
        job.max_cell_area,
        tif.display()
    )];
    if let Some(g) = passfail {
        let p = tif.with_file_name(format!("{}_cumple.tif", crate::paths::stem(&tif)));
        g.write_tiff(&p, epsg)?;
        lines.push(format!("  cumple o no cumple el umbral: {}", p.display()));
    }
    if job.csv {
        let p = crate::paths::swap_ext(&job.output, "csv");
        std::fs::write(
            &p,
            format!(
                "Celdas,Puntos,Media,Mediana,P5,P95,Pct_cumple,Umbral,Area_sin_datos_m2\n{},{},{:.4},{:.4},{:.4},{:.4},{:.2},{},{:.1}\n",
                rep.cells,
                rep.points,
                rep.mean,
                rep.median,
                rep.p05,
                rep.p95,
                rep.pass_pct,
                job.threshold,
                rep.void_area
            ),
        )?;
        lines.push(format!("  tabla: {}", p.display()));
    }
    Ok(lines.join("\n"))
}
