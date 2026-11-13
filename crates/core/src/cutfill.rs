//! Cut and fill: how much earth has to be removed and added to take an existing terrain to a
//! design.
//!
//! The difference is `dz = design - existing`. Where `dz` is positive the design is higher: earth
//! has to be added (**fill**, terraplén). Where it is negative the design is lower: earth has to
//! be removed (**cut**, desmonte). Differences no larger than the tolerance count as unchanged.
//! The volume of a cell is `|dz| * cell²`.

use std::path::PathBuf;

use crate::geomops::{self, Feature, PolyLayer};
use crate::grid::Grid;
use crate::vector::{self, Region};
use crate::{Error, Result};

/// The surface the ground should have.
#[derive(Clone, Debug, PartialEq)]
pub enum Design {
    /// Another terrain model or a design surface; it is put on the cells of the existing one
    /// when they differ.
    Surface(PathBuf),
    /// A horizontal plane at this elevation.
    Level(f64),
    /// A sloping plane: elevation `z0` at `origin`, going down `slope_pct` per cent towards the
    /// compass direction `aspect_deg` (from north, clockwise), the way water would run.
    Plane {
        z0: f64,
        slope_pct: f64,
        aspect_deg: f64,
        origin: [f64; 2],
    },
}

pub struct CutFillJob {
    pub existing: PathBuf,
    pub design: Design,
    /// Polygons to measure one by one (the whole raster is always measured too).
    pub regions: Option<PathBuf>,
    /// Differences up to this size count as unchanged (m).
    pub tolerance: f64,
    /// Base path of the results; the difference raster is `<output>.tif`.
    pub output: PathBuf,
    pub csv: bool,
    /// Also a raster of whole numbers: 1 cut, 2 fill, 0 unchanged.
    pub classified: bool,
}

impl CutFillJob {
    pub fn new(existing: PathBuf, design: Design, output: PathBuf) -> Self {
        Self {
            existing,
            design,
            regions: None,
            tolerance: 0.0,
            output,
            csv: true,
            classified: false,
        }
    }
}

/// Cut and fill of the whole raster or of one polygon.
#[derive(Clone, Debug, PartialEq)]
pub struct Totals {
    pub name: String,
    /// Volumes (m³) and the areas (m²) they come from.
    pub cut: f64,
    pub fill: f64,
    pub cut_area: f64,
    pub fill_area: f64,
    pub unchanged_area: f64,
}

impl Totals {
    /// Fill minus cut: positive when earth has to be brought in.
    pub fn net(&self) -> f64 {
        self.fill - self.cut
    }

    fn new(name: String) -> Self {
        Self {
            name,
            cut: 0.0,
            fill: 0.0,
            cut_area: 0.0,
            fill_area: 0.0,
            unchanged_area: 0.0,
        }
    }

    fn add(&mut self, dz: f32, tol: f64, cell_area: f64) {
        let d = f64::from(dz);
        if d > tol {
            self.fill += d * cell_area;
            self.fill_area += cell_area;
        } else if d < -tol {
            self.cut -= d * cell_area;
            self.cut_area += cell_area;
        } else {
            self.unchanged_area += cell_area;
        }
    }
}

pub struct CutFillReport {
    pub total: Totals,
    pub rows: Vec<Totals>,
}

/// `design - existing`, cell by cell, on the same cells; empty where either is empty.
pub fn diff(existing: &Grid, design: &Grid) -> Result<Grid> {
    if existing.cols != design.cols
        || existing.rows != design.rows
        || (existing.cell - design.cell).abs() > existing.cell * 1e-6
        || (existing.x0 - design.x0).abs() > existing.cell * 1e-6
        || (existing.y0 - design.y0).abs() > existing.cell * 1e-6
    {
        return Err(Error::Param(
            "el diseño y el terreno existente no tienen las mismas celdas".into(),
        ));
    }
    let mut out = existing.like();
    for (o, (d, e)) in out
        .data
        .iter_mut()
        .zip(design.data.iter().zip(&existing.data))
    {
        *o = d - e;
    }
    Ok(out)
}

/// The design as a surface on the cells of `existing`.
pub fn design_grid(existing: &Grid, design: &Design) -> Result<Grid> {
    match design {
        Design::Surface(p) => {
            let d = Grid::read(p)?;
            let same = d.cols == existing.cols
                && d.rows == existing.rows
                && (d.cell - existing.cell).abs() <= existing.cell * 1e-6
                && (d.x0 - existing.x0).abs() <= existing.cell * 1e-6
                && (d.y0 - existing.y0).abs() <= existing.cell * 1e-6;
            Ok(if same { d } else { d.resampled_onto(existing) })
        }
        Design::Level(z) => {
            let mut g = existing.like();
            g.data.fill(*z as f32);
            Ok(g)
        }
        Design::Plane {
            z0,
            slope_pct,
            aspect_deg,
            origin,
        } => {
            if !slope_pct.is_finite() || !aspect_deg.is_finite() {
                return Err(Error::Param(
                    "el plano de diseño necesita pendiente y orientación".into(),
                ));
            }
            let (s, c) = aspect_deg.to_radians().sin_cos();
            let k = slope_pct / 100.0;
            let mut g = existing.like();
            for r in 0..g.rows {
                for col in 0..g.cols {
                    let (x, y) = g.center(col, r);
                    // the distance along the downhill direction
                    let along = (x - origin[0]) * s + (y - origin[1]) * c;
                    g.data[r * g.cols + col] = (z0 - k * along) as f32;
                }
            }
            Ok(g)
        }
    }
}

/// Cut and fill of the raster `dz` (from [`diff`]) as a whole and, for each region, inside it.
pub fn totals(dz: &Grid, regions: &[Region], tolerance: f64) -> Result<CutFillReport> {
    if !(tolerance >= 0.0) {
        return Err(Error::Param("la tolerancia no puede ser negativa".into()));
    }
    let cell_area = dz.cell * dz.cell;
    let mut total = Totals::new("Total".into());
    for v in dz.data.iter().filter(|v| !v.is_nan()) {
        total.add(*v, tolerance, cell_area);
    }
    let top = dz.y0 + dz.rows as f64 * dz.cell;
    let lim = |v: f64, n: usize| v.clamp(0.0, n as f64) as usize;
    let mut rows = Vec::with_capacity(regions.len());
    for region in regions {
        crate::check_cancel()?;
        let b = region.bbox();
        let (c0, c1) = (
            lim(((b[0] - dz.x0) / dz.cell).floor(), dz.cols),
            lim(((b[2] - dz.x0) / dz.cell).ceil(), dz.cols),
        );
        let (r0, r1) = (
            lim(((top - b[3]) / dz.cell).floor(), dz.rows),
            lim(((top - b[1]) / dz.cell).ceil(), dz.rows),
        );
        let mut t = Totals::new(region.name.clone());
        for r in r0..r1 {
            for c in c0..c1 {
                let v = dz.data[r * dz.cols + c];
                if v.is_nan() {
                    continue;
                }
                let (x, y) = dz.center(c, r);
                if region.contains(x, y) {
                    t.add(v, tolerance, cell_area);
                }
            }
        }
        rows.push(t);
    }
    Ok(CutFillReport { total, rows })
}

fn num(v: f64) -> String {
    format!("{v:.3}")
}

/// The totals as a CSV table: the whole raster first, then each polygon.
pub fn csv_text(rep: &CutFillReport) -> String {
    let mut s = String::from(
        "Nombre,Desmonte_m3,Terraplen_m3,Neto_m3,Area_desmonte_m2,Area_terraplen_m2,Area_sin_cambio_m2\n",
    );
    for t in std::iter::once(&rep.total).chain(&rep.rows) {
        let name = if t.name.contains([',', '"', '\n']) {
            format!("\"{}\"", t.name.replace('"', "\"\""))
        } else {
            t.name.clone()
        };
        s.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            name,
            num(t.cut),
            num(t.fill),
            num(t.net()),
            num(t.cut_area),
            num(t.fill_area),
            num(t.unchanged_area)
        ));
    }
    s
}

/// The fields the layer of polygons gets.
pub const LAYER_FIELDS: [&str; 6] = [
    "desmonte",
    "terraplen",
    "neto",
    "a_desm",
    "a_terr",
    "a_igual",
];

/// The classes of `dz` as a raster of whole numbers: 1 cut, 2 fill, 0 unchanged, empty where
/// `dz` is.
fn write_classes(
    dz: &Grid,
    tolerance: f64,
    epsg: Option<u16>,
    path: &std::path::Path,
) -> Result<()> {
    use crate::tiled::{Compression, Spec};
    let spec = Spec {
        cols: dz.cols,
        rows: dz.rows,
        bands: 1,
        dtype: crate::rastertools::Dtype::U8,
        cell: dz.cell,
        x0: dz.x0,
        top: dz.y0 + dz.rows as f64 * dz.cell,
        epsg,
        // 255 marks the cells without a value, since 0 is a class
        nodata: 255.0,
        compression: Compression::Deflate,
        nearest: true,
        bigtiff: false,
    };
    crate::tiled::write(path, spec, 256, |r0, r1| {
        Ok(dz.data[r0 * dz.cols..r1 * dz.cols]
            .iter()
            .map(|v| {
                if v.is_nan() {
                    f32::NAN
                } else if f64::from(*v) > tolerance {
                    2.0
                } else if f64::from(*v) < -tolerance {
                    1.0
                } else {
                    0.0
                }
            })
            .collect())
    })
}

/// Computes cut and fill and writes what was asked; returns the summary.
pub fn run(job: &CutFillJob) -> Result<String> {
    if let Ok(pyr) = crate::pyramid::Pyramid::open(&job.existing)
        && let Some(mem) = crate::copc_cache::total_memory()
    {
        let e = pyr.extent();
        let cells = ((e[2] - e[0]) / pyr.cell) * ((e[3] - e[1]) / pyr.cell);
        // the existing surface, the design and the difference
        if cells * 12.0 > mem as f64 * 0.7 {
            return Err(Error::Param(format!(
                "{:.0} millones de celdas no caben en la memoria: recorta el terreno a la zona de la obra con un polígono",
                cells / 1e6
            )));
        }
    }
    let existing = Grid::read(&job.existing)?;
    let epsg = Grid::read_epsg(&job.existing);
    let design = design_grid(&existing, &job.design)?;
    let dz = diff(&existing, &design)?;
    let regions = match &job.regions {
        Some(p) => volume_regions(p)?,
        None => Vec::new(),
    };
    let rep = totals(&dz, &regions, job.tolerance)?;
    crate::paths::ensure_parent(&job.output)?;
    let mut lines = vec![format!(
        "desmonte {:.2} m³ ({:.1} m²), terraplén {:.2} m³ ({:.1} m²), neto {:+.2} m³ (positivo: falta tierra); sin cambio {:.1} m²",
        rep.total.cut,
        rep.total.cut_area,
        rep.total.fill,
        rep.total.fill_area,
        rep.total.net(),
        rep.total.unchanged_area
    )];
    let tif = crate::paths::swap_ext(&job.output, "tif");
    dz.write_tiff(&tif, epsg)?;
    lines.push(format!(
        "  diferencia (diseño − existente): {}",
        tif.display()
    ));
    if job.classified {
        let p = tif.with_file_name(format!("{}_clases.tif", crate::paths::stem(&tif)));
        write_classes(&dz, job.tolerance, epsg, &p)?;
        lines.push(format!(
            "  clases (1 desmonte, 2 terraplén, 0 sin cambio): {}",
            p.display()
        ));
    }
    if job.csv {
        let p = crate::paths::swap_ext(&job.output, "csv");
        std::fs::write(&p, csv_text(&rep))?;
        lines.push(format!("  tabla: {}", p.display()));
    }
    if let (Some(src), false) = (&job.regions, regions.is_empty()) {
        let features: Vec<Feature> = regions
            .iter()
            .zip(&rep.rows)
            .map(|(region, t)| {
                let mut attrs: Vec<(String, String)> = region
                    .attrs
                    .iter()
                    .filter(|(k, _)| !LAYER_FIELDS.iter().any(|f| f.eq_ignore_ascii_case(k)))
                    .cloned()
                    .collect();
                let values = [
                    num(t.cut),
                    num(t.fill),
                    num(t.net()),
                    num(t.cut_area),
                    num(t.fill_area),
                    num(t.unchanged_area),
                ];
                attrs.extend(LAYER_FIELDS.iter().map(|f| (*f).to_string()).zip(values));
                Feature {
                    geom: geomops::multipolygon_of(&geomops::rings_of(region)),
                    attrs,
                }
            })
            .collect();
        let prj = std::fs::read_to_string(src.with_extension("prj"))
            .ok()
            .or_else(|| epsg.and_then(vector::prj_for_epsg));
        let shp = crate::paths::swap_ext(&job.output, "shp");
        geomops::write_polygons(&PolyLayer { features, prj }, &shp)?;
        lines.push(format!("  polígonos: {}", shp.display()));
    }
    Ok(lines.join("\n"))
}

fn volume_regions(p: &std::path::Path) -> Result<Vec<Region>> {
    crate::volume::read_regions(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::Shape2;
    use crate::volume::{self, Reference};

    fn flat(z: f32) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 10, 10)?;
        g.data.fill(z);
        Ok(g)
    }

    #[test]
    fn a_design_one_metre_higher_is_all_fill() -> Result<()> {
        let existing = flat(10.0)?;
        let design = design_grid(&existing, &Design::Level(11.0))?;
        let rep = totals(&diff(&existing, &design)?, &[], 0.0)?;
        assert_eq!(rep.total.fill, 100.0);
        assert_eq!(rep.total.cut, 0.0);
        assert_eq!(rep.total.fill_area, 100.0);
        assert_eq!(rep.total.net(), 100.0);
        Ok(())
    }

    #[test]
    fn the_sign_is_fixed_a_higher_design_is_fill_and_a_lower_one_is_cut() -> Result<()> {
        let existing = flat(10.0)?;
        let dz = diff(&existing, &flat(12.0)?)?;
        assert_eq!(dz.data[0], 2.0);
        let up = totals(&dz, &[], 0.0)?.total;
        assert!(up.fill > 0.0 && up.cut == 0.0);
        let down = totals(&diff(&existing, &flat(7.0)?)?, &[], 0.0)?.total;
        assert!(down.cut == 300.0 && down.fill == 0.0);
        // the same convention as the volume above and below a reference
        let r = volume::volumes(&flat(12.0)?, Reference::Dtm(&existing), &[], 0.0)?;
        assert_eq!(r[0].above, up.fill);
        let r = volume::volumes(&flat(7.0)?, Reference::Dtm(&existing), &[], 0.0)?;
        assert_eq!(r[0].below, down.cut);
        Ok(())
    }

    #[test]
    fn a_level_at_the_mean_balances() -> Result<()> {
        let mut existing = flat(0.0)?;
        for r in 0..10 {
            for c in 0..10 {
                existing.data[r * 10 + c] = (c as f32 - 4.5) * 0.5 + 20.0;
            }
        }
        let mean = existing.data.iter().map(|v| f64::from(*v)).sum::<f64>() / 100.0;
        let design = design_grid(&existing, &Design::Level(mean))?;
        let t = totals(&diff(&existing, &design)?, &[], 1e-6)?.total;
        assert!(t.net().abs() < 1e-3, "{}", t.net());
        assert!(t.cut > 0.0 && t.fill > 0.0);
        Ok(())
    }

    #[test]
    fn the_tolerance_leaves_small_differences_unchanged() -> Result<()> {
        let existing = flat(10.0)?;
        let t = totals(&diff(&existing, &flat(10.04)?)?, &[], 0.05)?.total;
        assert_eq!((t.fill, t.cut, t.unchanged_area), (0.0, 0.0, 100.0));
        let t = totals(&diff(&existing, &flat(10.06)?)?, &[], 0.05)?.total;
        assert!(t.fill > 0.0);
        assert!(totals(&existing, &[], -1.0).is_err());
        Ok(())
    }

    #[test]
    fn a_sloping_plane_goes_down_the_way_it_faces() -> Result<()> {
        let existing = flat(0.0)?;
        // 10 % down towards the east (90 degrees), from 20 m at x = 0
        let d = design_grid(
            &existing,
            &Design::Plane {
                z0: 20.0,
                slope_pct: 10.0,
                aspect_deg: 90.0,
                origin: [0.0, 0.0],
            },
        )?;
        let (west, east) = (d.sample(0.5, 5.0), d.sample(9.5, 5.0));
        assert!((f64::from(west.unwrap_or(0.0)) - 19.95).abs() < 1e-4);
        assert!((f64::from(east.unwrap_or(0.0)) - 19.05).abs() < 1e-4);
        Ok(())
    }

    #[test]
    fn polygons_are_measured_one_by_one_and_empty_cells_do_not_count() -> Result<()> {
        let mut existing = flat(10.0)?;
        existing.data[0] = f32::NAN;
        let dz = diff(&existing, &flat(11.0)?)?;
        let reg = Region {
            name: "mitad".into(),
            shape: Shape2::Rect([0.0, 0.0, 5.0, 10.0]),
            attrs: Vec::new(),
        };
        let rep = totals(&dz, &[reg], 0.0)?;
        assert_eq!(rep.total.fill, 99.0);
        assert_eq!(rep.rows[0].fill, 49.0);
        Ok(())
    }

    #[test]
    fn another_lattice_is_put_on_the_existing_one() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_cutfill_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let existing = flat(10.0)?;
        existing.write_tiff(&dir.join("e.tif"), Some(25829))?;
        let mut d = Grid::new(0.0, 0.0, 0.5, 20, 20)?;
        d.data.fill(12.0);
        d.write_tiff(&dir.join("d.tif"), Some(25829))?;
        let mut job = CutFillJob::new(
            dir.join("e.tif"),
            Design::Surface(dir.join("d.tif")),
            dir.join("r"),
        );
        job.classified = true;
        let msg = run(&job)?;
        assert!(msg.contains("terraplén 200.00"), "{msg}");
        let dz = Grid::read(&dir.join("r.tif"))?;
        assert!(dz.data.iter().all(|v| (*v - 2.0).abs() < 1e-5));
        assert!(dir.join("r_clases.tif").exists());
        assert!(
            std::fs::read_to_string(dir.join("r.csv"))?.contains("Total,0.000,200.000,200.000")
        );
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn different_cells_cannot_be_subtracted() -> Result<()> {
        let a = flat(1.0)?;
        let b = Grid::new(0.0, 0.0, 2.0, 5, 5)?;
        assert!(diff(&a, &b).is_err());
        Ok(())
    }
}
