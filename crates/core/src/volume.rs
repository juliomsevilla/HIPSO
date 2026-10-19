//! Volumes between a surface raster and a reference, for the whole raster or for regions.
//!
//! The surface is a DSM or a CHM; the reference is a DTM, a level, or nothing (a CHM already
//! holds heights above ground). Each surface cell is `h = surface - reference` high: cells at
//! least `min_height` above the reference add `h * cell²` to the volume above it, cells at least
//! that far below add to the volume under it. Regions that overlap are measured one by one.

use std::path::{Path, PathBuf};

use crate::grid::Grid;
use crate::vector::{self, Region};
use crate::{Error, Result};

/// What the surface is measured against.
#[derive(Clone, Copy, Debug)]
pub enum Reference<'a> {
    /// The surface already holds heights (a canopy height model).
    Zero,
    /// A horizontal plane at this elevation.
    Level(f64),
    /// A terrain model, sampled at the centre of every surface cell.
    Dtm(&'a Grid),
}

/// The measure of one region.
#[derive(Clone, Debug, PartialEq)]
pub struct VolumeRow {
    pub name: String,
    /// Area (m²) of the cells measured: those with a surface value and a reference.
    pub area: f64,
    /// Volume (m³) above the reference, and under it.
    pub above: f64,
    pub below: f64,
    /// Mean and largest height of the cells above the reference (NaN when there are none).
    pub mean_h: f64,
    pub max_h: f64,
    /// Cells with a surface value that the DTM does not cover: not measured.
    pub no_reference: u64,
}

impl VolumeRow {
    pub fn net(&self) -> f64 {
        self.above - self.below
    }
}

/// Columns and rows of the cells whose centres can lie inside `bbox` (min x, min y, max x, max y).
fn window(g: &Grid, bbox: [f64; 4]) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let top = g.y0 + g.rows as f64 * g.cell;
    let lim = |v: f64, n: usize| v.clamp(0.0, n as f64) as usize;
    (
        lim(((bbox[0] - g.x0) / g.cell).floor(), g.cols)
            ..lim(((bbox[2] - g.x0) / g.cell).ceil(), g.cols),
        lim(((top - bbox[3]) / g.cell).floor(), g.rows)
            ..lim(((top - bbox[1]) / g.cell).ceil(), g.rows),
    )
}

/// Volumes of the surface over `reference`, one row per region (one row for the whole raster
/// when there are none). Heights closer than `min_height` to the reference count as nothing.
pub fn volumes(
    surface: &Grid,
    reference: Reference,
    regions: &[Region],
    min_height: f64,
) -> Result<Vec<VolumeRow>> {
    if !(min_height >= 0.0) {
        return Err(Error::Param("the minimum height cannot be negative".into()));
    }
    let cell_area = surface.cell * surface.cell;
    let measure = |name: String, bbox: [f64; 4], inside: &dyn Fn(f64, f64) -> bool| {
        let (cols, rows) = window(surface, bbox);
        let mut row = VolumeRow {
            name,
            area: 0.0,
            above: 0.0,
            below: 0.0,
            mean_h: f64::NAN,
            max_h: f64::NAN,
            no_reference: 0,
        };
        let (mut n_above, mut sum_h) = (0u64, 0.0);
        for r in rows {
            for c in cols.clone() {
                let z = surface.data[r * surface.cols + c];
                if z.is_nan() {
                    continue;
                }
                let (x, y) = surface.center(c, r);
                if !inside(x, y) {
                    continue;
                }
                let base = match reference {
                    Reference::Zero => Some(0.0),
                    Reference::Level(l) => Some(l),
                    // `sample` clamps to the edge: outside the DTM there is no reference
                    Reference::Dtm(d) => d.index(x, y).and_then(|_| d.sample(x, y)).map(f64::from),
                };
                let Some(base) = base else {
                    row.no_reference += 1;
                    continue;
                };
                let h = f64::from(z) - base;
                row.area += cell_area;
                row.max_h = row.max_h.max(h);
                if h > 0.0 && h >= min_height {
                    row.above += h * cell_area;
                    n_above += 1;
                    sum_h += h;
                } else if h < 0.0 && -h >= min_height {
                    row.below -= h * cell_area;
                }
            }
        }
        if n_above > 0 {
            row.mean_h = sum_h / n_above as f64;
        }
        row
    };
    if regions.is_empty() {
        let b = [
            surface.x0,
            surface.y0,
            surface.x0 + surface.cols as f64 * surface.cell,
            surface.y0 + surface.rows as f64 * surface.cell,
        ];
        return Ok(vec![measure("Total".into(), b, &|_, _| true)]);
    }
    let mut out = Vec::with_capacity(regions.len());
    for r in regions {
        crate::check_cancel()?;
        out.push(measure(r.name.clone(), r.bbox(), &|x, y| r.contains(x, y)));
    }
    Ok(out)
}

/// Where the reference of a run comes from.
#[derive(Clone, Debug)]
pub enum ReferenceSource {
    Zero,
    Level(f64),
    Dtm(PathBuf),
}

#[derive(Clone, Debug)]
pub struct VolumeJob {
    pub surface: PathBuf,
    pub reference: ReferenceSource,
    /// Empty: the whole raster.
    pub regions: Vec<Region>,
    pub min_height: f64,
    pub output: PathBuf,
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// The rows as a CSV table.
pub fn csv_text(rows: &[VolumeRow]) -> String {
    let num = |v: f64| {
        if v.is_finite() {
            format!("{v:.3}")
        } else {
            String::new()
        }
    };
    let mut s = String::from(
        "Region,Area_m2,Vol_sobre_m3,Vol_bajo_m3,Vol_neto_m3,H_media_m,H_max_m,Celdas_sin_MDT\n",
    );
    for r in rows {
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_field(&r.name),
            num(r.area),
            num(r.above),
            num(r.below),
            num(r.net()),
            num(r.mean_h),
            num(r.max_h),
            r.no_reference
        ));
    }
    s
}

/// Measures the volumes of a job and writes the table; one line about the result.
pub fn run(job: &VolumeJob) -> Result<String> {
    let surface = Grid::read(&job.surface)?;
    let dtm;
    let reference = match &job.reference {
        ReferenceSource::Zero => Reference::Zero,
        ReferenceSource::Level(l) => Reference::Level(*l),
        ReferenceSource::Dtm(p) => {
            dtm = Grid::read(p)?;
            Reference::Dtm(&dtm)
        }
    };
    let rows = volumes(&surface, reference, &job.regions, job.min_height)?;
    crate::paths::ensure_parent(&job.output)?;
    std::fs::write(&job.output, csv_text(&rows))?;
    Ok(summary(&rows, &job.output))
}

/// One line about a set of rows.
pub fn summary(rows: &[VolumeRow], out: &Path) -> String {
    let (above, below, area): (f64, f64, f64) = rows.iter().fold((0.0, 0.0, 0.0), |a, r| {
        (a.0 + r.above, a.1 + r.below, a.2 + r.area)
    });
    let skipped: u64 = rows.iter().map(|r| r.no_reference).sum();
    let mut text = format!(
        "{} región(es), {area:.1} m²: {above:.3} m³ sobre la referencia, {below:.3} m³ bajo ella",
        rows.len()
    );
    if skipped > 0 {
        text.push_str(&format!(" ({skipped} celdas sin MDT no se midieron)"));
    }
    format!("{text}: {}", out.display())
}

/// Reads regions from a vector file for a volume run.
pub fn read_regions(path: &Path) -> Result<Vec<Region>> {
    vector::read_regions(path, &vector::ReadOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::Shape2;

    fn grid(z: f32) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 2.0, 4, 4)?;
        g.data.fill(z);
        Ok(g)
    }

    #[test]
    fn a_plateau_over_a_flat_dtm() -> Result<()> {
        // 16 cells of 4 m² at 12 m over a terrain at 10 m: 64 m² x 2 m
        let rows = volumes(&grid(12.0)?, Reference::Dtm(&grid(10.0)?), &[], 0.0)?;
        assert_eq!(rows.len(), 1);
        assert!((rows[0].area - 64.0).abs() < 1e-9);
        assert!((rows[0].above - 128.0).abs() < 1e-9);
        assert_eq!(rows[0].below, 0.0);
        assert!((rows[0].mean_h - 2.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn heights_below_the_reference_and_the_minimum_height() -> Result<()> {
        let rows = volumes(&grid(9.0)?, Reference::Level(10.0), &[], 0.0)?;
        assert!((rows[0].below - 64.0).abs() < 1e-9 && rows[0].above == 0.0);
        // a 2 m surface under a 3 m minimum height counts as nothing
        let rows = volumes(&grid(2.0)?, Reference::Zero, &[], 3.0)?;
        assert_eq!(rows[0].above, 0.0);
        assert!(rows[0].mean_h.is_nan());
        Ok(())
    }

    #[test]
    fn a_region_takes_only_its_cells() -> Result<()> {
        // the left half of the raster: x from 0 to 4
        let left = Region {
            name: "izquierda".into(),
            shape: Shape2::Rect([0.0, 0.0, 4.0, 8.0]),
            attrs: vec![],
        };
        let rows = volumes(&grid(5.0)?, Reference::Zero, &[left], 0.0)?;
        assert!((rows[0].area - 32.0).abs() < 1e-9);
        assert!((rows[0].above - 160.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn cells_outside_the_dtm_are_counted_apart() -> Result<()> {
        let mut small = Grid::new(0.0, 0.0, 2.0, 2, 2)?;
        small.data.fill(0.0);
        // the DTM covers the south-west quarter only
        let rows = volumes(&grid(1.0)?, Reference::Dtm(&small), &[], 0.0)?;
        assert!(rows[0].no_reference > 0);
        assert!(rows[0].area < 64.0);
        Ok(())
    }

    #[test]
    fn csv_quotes_names() {
        let row = VolumeRow {
            name: "a,b".into(),
            area: 1.0,
            above: 2.0,
            below: 0.5,
            mean_h: f64::NAN,
            max_h: 1.0,
            no_reference: 0,
        };
        let t = csv_text(&[row]);
        assert!(t.contains("\"a,b\",1.000,2.000,0.500,1.500,,1.000,0"));
    }
}
