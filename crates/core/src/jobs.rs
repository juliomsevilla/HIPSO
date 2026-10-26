//! Every tool as a job: its parameters in one value, run by one function that returns a
//! summary. The command line and the desktop app both build jobs and run them here, so they
//! always behave alike.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use crate::grid::Grid;
use crate::ground::GroundParams;
use crate::las::Reader;
use crate::metrics::{self, MetricParams};
use crate::pipeline::{self, Ground, PitFree, Steps, Surface, Tiling};
use crate::trees::{self, CrownParams, TopParams};
use crate::{Result, chm, ground, paths};

pub use crate::obia::job::{ClassifyBy, ObiaJob, StrataOpts, obia};

/// A full report on the files: header, points, quality warnings.
pub fn info(paths: &[PathBuf]) -> Result<String> {
    let sum = crate::inspect::inspect_many(paths)?;
    let mut out: Vec<String> = sum.files.iter().map(crate::inspect::render).collect();
    if sum.files.len() > 1 {
        out.push(format!(
            "Total: {} archivos · {} puntos · densidad {:.2} puntos/m² · X {:.1} a {:.1} · Y {:.1} a {:.1}",
            sum.files.len(),
            sum.points,
            sum.density,
            sum.bounds[0],
            sum.bounds[3],
            sum.bounds[1],
            sum.bounds[4]
        ));
    }
    Ok(out.join(
        "
",
    ))
}

/// What a classification did, naming only the classes it touched.
fn report(r: &pipeline::Report) -> String {
    let mut parts = vec![format!("{} de suelo", r.ground)];
    for (n, what) in [
        (r.noise, "de ruido"),
        (r.buildings, "de edificios"),
        (r.vegetation, "de vegetación"),
        (r.wires, "de cables"),
        (r.poles, "de postes"),
        (r.rules, "cambiados por reglas"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    format!(
        "{} puntos en {} teselas: {}",
        r.points,
        r.tiles,
        parts.join(", ")
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassifyJob {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub steps: Steps,
    pub tiling: Tiling,
}

pub fn classify(j: &ClassifyJob) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let r = pipeline::classify(&j.inputs, &j.output, &j.steps, &j.tiling)?;
    Ok(format!("{}\n{}", report(&r), j.output.display()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct DtmJob {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub cell: f64,
    pub surface: Surface,
    pub steps: Steps,
    pub tiling: Tiling,
    /// Smoothing window in map units (0: none) and its kind.
    pub smooth: f64,
    pub smoothing: trees::Smoothing,
    /// Breaklines (shapefile of lines or polygons, with or without elevations) that the ground
    /// model must follow; TIN and IDW surfaces only.
    pub breaklines: Option<PathBuf>,
}

/// The Kraus & Pfeifer filter on one file, with a mean surface and no other step, runs streamed over the
/// whole file (same result as the tiled path); anything else runs tiled.
pub fn dtm(j: &DtmJob) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let lines = match &j.breaklines {
        Some(p) => {
            if j.surface == Surface::Mean {
                return Err(crate::Error::Param(
                    "las líneas de rotura necesitan una superficie triangulada (TIN) o ponderada por distancia (IDW), no la media por celda".into(),
                ));
            }
            Some(crate::breaklines::Breaklines::read(p)?)
        }
        None => None,
    };
    let streamed = j.inputs.len() == 1
        && j.surface == Surface::Mean
        && j.steps.noise.is_none()
        && j.steps.buildings.is_none();
    if let (true, Ground::Fusion(p), Some(input)) = (streamed, &j.steps.ground, j.inputs.first()) {
        let r = Reader::open(input)?.for_passes()?;
        let (dtm, rep) = ground::dtm(&r, j.cell, p)?;
        let dtm = smoothed(dtm, j.smooth, j.smoothing);
        dtm.write_tiff(&j.output, r.header.epsg)?;
        return Ok(format!(
            "{} puntos de suelo de {} ({} pasadas); MDT de {} x {}: {}",
            rep.ground,
            rep.points,
            rep.passes,
            dtm.cols,
            dtm.rows,
            j.output.display()
        ));
    }
    let (dtm, rep, epsg) = pipeline::dtm(
        &j.inputs,
        j.cell,
        &j.steps,
        j.surface,
        &j.tiling,
        lines.as_ref(),
    )?;
    let dtm = smoothed(dtm, j.smooth, j.smoothing);
    dtm.write_tiff(&j.output, epsg)?;
    let mut text = format!(
        "{}\nMDT de {} x {}: {}",
        report(&rep),
        dtm.cols,
        dtm.rows,
        j.output.display()
    );
    if let Some(l) = &lines {
        text.push_str(&format!(
            "\n  {} línea(s) de rotura {}",
            l.lines.len(),
            if l.has_z() {
                "con sus cotas"
            } else {
                "sin cotas: siguen el suelo más bajo junto a ellas"
            }
        ));
    }
    Ok(text)
}

/// How a surface model (highest returns) is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DsmMethod {
    /// Highest point of each cell, in one streaming pass.
    Max,
    /// TIN through the highest point of each cell, tile by tile.
    Tin,
    /// Inverse-distance weighting of the highest points of the cells, tile by tile.
    Idw,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DsmJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub cell: f64,
    pub method: DsmMethod,
    /// Fill the empty cells of the streamed surface.
    pub fill: bool,
    pub tiling: Tiling,
    pub smooth: f64,
    pub smoothing: trees::Smoothing,
}

pub fn dsm(j: &DsmJob) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let (g, epsg) = match j.method {
        DsmMethod::Max => {
            let r = Reader::open(&j.input)?;
            (chm::dsm(&r, j.cell, j.fill)?, r.header.epsg)
        }
        DsmMethod::Tin | DsmMethod::Idw => pipeline::dsm(
            std::slice::from_ref(&j.input),
            j.cell,
            if j.method == DsmMethod::Idw {
                Surface::Idw
            } else {
                Surface::Tin
            },
            &j.tiling,
        )?,
    };
    let g = smoothed(g, j.smooth, j.smoothing);
    g.write_tiff(&j.output, epsg)?;
    Ok(format!(
        "MDS de {} x {}: {}",
        g.cols,
        g.rows,
        j.output.display()
    ))
}

/// A surface smoothed with a window of `size` map units (unchanged when `size` is 0).
fn smoothed(g: Grid, size: f64, kind: trees::Smoothing) -> Grid {
    if size > 0.0 {
        trees::smooth(&g, size, kind, trees::Shape::Circle)
    } else {
        g
    }
}

/// A surface model turned into heights above `dtm` (negative heights count as 0); cells the DTM
/// does not cover are empty.
fn above_ground(mut g: Grid, dtm: &Grid) -> Grid {
    for row in 0..g.rows {
        for col in 0..g.cols {
            let (x, y) = g.center(col, row);
            if let Some(v) = g.data.get_mut(row * g.cols + col) {
                *v = match dtm.sample(x, y) {
                    Some(z) if !v.is_nan() => (*v - z).max(0.0),
                    _ => f32::NAN,
                };
            }
        }
    }
    g
}

/// How a CHM is made.
#[derive(Clone, Debug, PartialEq)]
pub enum ChmMethod {
    /// Highest point per cell ; empty cells filled unless `fill` is false.
    Max { fill: bool },
    /// Pit-free stack of TINs.
    PitFree(PitFree),
    /// TIN through the highest point of each cell, minus the DTM.
    Tin,
    /// Inverse-distance weighting of the highest points of the cells, minus the DTM.
    Idw,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChmJob {
    pub input: PathBuf,
    pub dtm: PathBuf,
    pub output: PathBuf,
    pub cell: f64,
    pub method: ChmMethod,
    pub tiling: Tiling,
    /// Smoothing window in map units (0: none) and its kind.
    pub smooth: f64,
    pub smoothing: trees::Smoothing,
}

pub fn chm(j: &ChmJob) -> Result<String> {
    let u = crate::crs::unify(&[j.input.as_path(), j.dtm.as_path()], false)?;
    let j = ChmJob {
        input: u.path(&j.input),
        dtm: u.path(&j.dtm),
        ..j.clone()
    };
    chm_run(&j).map(|s| s + &u.notes())
}

fn chm_run(j: &ChmJob) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let ground = Grid::read(&j.dtm)?;
    let (chm, epsg) = match &j.method {
        ChmMethod::Max { fill } => {
            let r = Reader::open(&j.input)?;
            (chm::chm(&r, &ground, j.cell, *fill)?, r.header.epsg)
        }
        ChmMethod::Tin | ChmMethod::Idw => {
            let surface = if j.method == ChmMethod::Idw {
                Surface::Idw
            } else {
                Surface::Tin
            };
            let (dsm, epsg) =
                pipeline::dsm(std::slice::from_ref(&j.input), j.cell, surface, &j.tiling)?;
            (above_ground(dsm, &ground), epsg)
        }
        ChmMethod::PitFree(p) => pipeline::chm_pitfree(
            std::slice::from_ref(&j.input),
            &ground,
            j.cell,
            p,
            &j.tiling,
        )?,
    };
    let chm = smoothed(chm, j.smooth, j.smoothing);
    chm.write_tiff(&j.output, epsg)?;
    Ok(format!(
        "CHM de {} x {}: {}",
        chm.cols,
        chm.rows,
        j.output.display()
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricsJob {
    pub input: PathBuf,
    pub dtm: PathBuf,
    pub output: PathBuf,
    pub params: MetricParams,
    pub intensity: bool,
    pub strata: bool,
    pub formats: paths::Formats,
}

impl MetricsJob {
    /// `<output stem>_<suffix>.csv` next to the output.
    pub fn sibling(&self, suffix: &str) -> PathBuf {
        let stem = self
            .output
            .file_stem()
            .map_or("metrics".into(), |s| s.to_string_lossy().into_owned());
        self.output.with_file_name(format!("{stem}_{suffix}.csv"))
    }
}

/// Metrics by cell. A `.shp` output (the default) gives a polygon per cell with the metrics as
/// attributes (DBF field names are short: `<name>_campos.txt` lists their meaning); a `.csv`
/// output keeps the FUSION-compatible text table.
pub fn metrics(j: &MetricsJob) -> Result<String> {
    let u = crate::crs::unify(&[j.input.as_path(), j.dtm.as_path()], false)?;
    let j = MetricsJob {
        input: u.path(&j.input),
        dtm: u.path(&j.dtm),
        ..j.clone()
    };
    metrics_run(&j).map(|s| s + &u.notes())
}

fn metrics_run(j: &MetricsJob) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let f = j.formats.fixed();
    let csv_main = paths::swap_ext(&j.output, "csv");
    if !f.vector {
        let job = MetricsJob {
            output: csv_main.clone(),
            ..j.clone()
        };
        return metrics_csv(&job, &csv_main);
    }
    // the vector layers are made from the text tables, which are kept when asked for
    let csv_target = if f.csv {
        csv_main
    } else {
        paths::swap_ext(&j.output, "tmp.csv")
    };
    let tmp_job = MetricsJob {
        output: csv_target.clone(),
        ..j.clone()
    };
    let summary = metrics_csv(&tmp_job, &csv_target)?;
    let prj = Reader::open(&j.input)
        .ok()
        .and_then(|r| r.header.epsg)
        .and_then(crate::vector::prj_for_epsg);
    let mut centres = std::collections::HashMap::new();
    let shp_main = paths::swap_ext(&j.output, "shp");
    let mut files = vec![shp_main.display().to_string()];
    crate::vector::metrics_csv_to_shp(
        &csv_target,
        &shp_main,
        j.params.cell,
        &mut centres,
        prj.as_deref(),
    )?;
    let mut texts = vec![csv_target.clone()];
    for (flag, suffix) in [
        (j.intensity, "intensity"),
        (j.strata, "strata"),
        (j.params.structure.is_some(), "estructura"),
    ] {
        if !flag {
            continue;
        }
        let csv = tmp_job.sibling(suffix);
        let shp = shp_main.with_file_name(format!("{}_{suffix}.shp", paths::stem(&shp_main)));
        crate::vector::metrics_csv_to_shp(&csv, &shp, j.params.cell, &mut centres, prj.as_deref())?;
        files.push(shp.display().to_string());
        texts.push(csv);
    }
    let mut summary = summary.replace(
        &csv_target.display().to_string(),
        &shp_main.display().to_string(),
    );
    if f.csv {
        summary.push_str(&format!(
            "\n  tablas CSV: {}",
            texts
                .iter()
                .map(|t| t.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else {
        for t in &texts {
            let _ = std::fs::remove_file(t);
        }
    }
    Ok(format!("{summary}\n  shapefiles: {}", files.join(", ")))
}

fn metrics_csv(j: &MetricsJob, output: &std::path::Path) -> Result<String> {
    let mut out = BufWriter::new(File::create(output)?);
    let r = Reader::open(&j.input)?.for_passes()?;
    let ground = Grid::read(&j.dtm)?;
    let mut int_out = if j.intensity {
        Some(BufWriter::new(File::create(j.sibling("intensity"))?))
    } else {
        None
    };
    let mut strata_out = if j.strata {
        Some(BufWriter::new(File::create(j.sibling("strata"))?))
    } else {
        None
    };
    let mut structure_out = if j.params.structure.is_some() {
        Some(BufWriter::new(File::create(j.sibling("estructura"))?))
    } else {
        None
    };
    let outs = metrics::Outputs {
        elevation: &mut out,
        intensity: int_out.as_mut().map(|w| w as &mut dyn Write),
        strata: strata_out.as_mut().map(|w| w as &mut dyn Write),
        structure: structure_out.as_mut().map(|w| w as &mut dyn Write),
    };
    let rep = metrics::grid_metrics(&r, &ground, &j.params, outs)?;
    out.flush()?;
    for w in [
        int_out.as_mut(),
        strata_out.as_mut(),
        structure_out.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        w.flush()?;
    }
    Ok(format!(
        "{} de {} celdas con métricas, {} puntos, {} banda(s): {}",
        rep.cells_with_metrics,
        rep.cells,
        rep.points,
        rep.bands,
        j.output.display()
    ))
}

/// Field inventory to check the detections against.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldJob {
    pub file: PathBuf,
    pub columns: crate::forest::FieldColumns,
    /// Plot centre columns, and the plot radius; detections count only inside the plots.
    pub plot_x: String,
    pub plot_y: String,
    pub plot_radius: f64,
    pub max_dist: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreesJob {
    pub chm: PathBuf,
    /// Base path of the trees table (its extension is replaced by the formats chosen).
    pub output: PathBuf,
    pub formats: paths::Formats,
    /// Crown raster of tree ids.
    pub crowns: Option<PathBuf>,
    pub crowns_shp: Option<PathBuf>,
    pub tops_shp: Option<PathBuf>,
    /// A cloud and the copy of it that gets a `treeID` attribute.
    pub cloud: Option<(PathBuf, PathBuf)>,
    /// Tree tops given by hand (CSV: x,y per line) instead of searched for.
    pub tops_file: Option<PathBuf>,
    pub tops: TopParams,
    pub crown: CrownParams,
    /// Smoothing window of the CHM in map units (0: none).
    pub smooth: f64,
    pub smoothing: trees::Smoothing,
    pub field: Option<FieldJob>,
}

/// Tops from a CSV of `x,y` lines (a header line is skipped), their height read from the CHM.
fn manual_tops(path: &std::path::Path, g: &Grid) -> Result<Vec<trees::Tree>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<f64> = line
            .split([',', ';', '\t'])
            .map_while(|t| t.trim().replace(',', ".").parse().ok())
            .collect();
        if let (Some(&x), Some(&y)) = (f.first(), f.get(1))
            && let Some(h) = g.sample(x, y)
        {
            out.push(trees::Tree {
                id: out.len() as u32 + 1,
                x,
                y,
                height: h,
                area: 0.0,
            });
        }
    }
    Ok(out)
}

pub fn trees(j: &TreesJob) -> Result<String> {
    let mut files = vec![j.chm.clone()];
    files.extend(j.cloud.iter().map(|c| c.0.clone()));
    let u = crate::crs::unify(&files, false)?;
    let j = TreesJob {
        chm: u.path(&j.chm),
        cloud: j.cloud.as_ref().map(|(i, o)| (u.path(i), o.clone())),
        ..j.clone()
    };
    trees_run(&j).map(|s| s + &u.notes())
}

fn trees_run(j: &TreesJob) -> Result<String> {
    use crate::forest;
    let mut g = Grid::read(&j.chm)?;
    if j.smooth > 0.0 {
        g = trees::smooth(&g, j.smooth, j.smoothing, j.tops.shape);
    }
    let mut list = match &j.tops_file {
        Some(f) => manual_tops(f, &g)?,
        None => trees::tops(&g, &j.tops),
    };
    let ids = trees::crowns(&g, &mut list, &j.crown);
    paths::ensure_parent(&j.output)?;
    let hulls = forest::crown_hulls(&g, &ids);
    let epsg = Reader::open(&j.cloud.as_ref().map_or(j.chm.clone(), |c| c.0.clone()))
        .ok()
        .and_then(|r| r.header.epsg);
    let f = j.formats.fixed();
    let mut written = Vec::new();
    if f.csv {
        let path = paths::swap_ext(&j.output, "csv");
        let mut out = BufWriter::new(File::create(&path)?);
        writeln!(
            out,
            "id,x,y,height,crown_area,crown_hull_area,crown_diameter"
        )?;
        for t in &list {
            let hull = hulls.get(&t.id).map_or(0.0, |h| forest::polygon_area(h));
            writeln!(
                out,
                "{},{:.3},{:.3},{:.2},{:.2},{:.2},{:.2}",
                t.id,
                t.x,
                t.y,
                t.height,
                t.area,
                hull,
                2.0 * (t.area / std::f64::consts::PI).sqrt()
            )?;
        }
        out.flush()?;
        written.push(path.display().to_string());
    }
    if f.vector {
        // the tree tops with their attributes
        let path = paths::swap_ext(&j.output, "shp");
        forest::write_tops_shp(&path, &list, &hulls, epsg)?;
        written.push(path.display().to_string());
    }
    let mut lines = vec![format!("{} árboles: {}", list.len(), written.join(", "))];
    if let Some(path) = &j.crowns {
        paths::ensure_parent(path)?;
        let mut r = g.like();
        for (v, id) in r.data.iter_mut().zip(&ids) {
            if *id > 0 {
                *v = *id as f32;
            }
        }
        r.write_tiff(path, None)?;
        lines.push(format!("  copas (ráster): {}", path.display()));
    }
    if let Some(path) = &j.crowns_shp {
        paths::ensure_parent(path)?;
        forest::write_crowns_shp(path, &list, &hulls)?;
        lines.push(format!("  copas (polígonos): {}", path.display()));
    }
    // the tops shapefile is already the main result when vector output is on
    if let Some(path) = j.tops_shp.as_ref().filter(|_| !f.vector) {
        paths::ensure_parent(path)?;
        forest::write_tops_shp(path, &list, &hulls, epsg)?;
        lines.push(format!("  ápices (puntos): {}", path.display()));
    }
    if let Some((input, output)) = &j.cloud {
        paths::ensure_parent(output)?;
        let n = forest::write_tree_ids(input, output, &g, &ids)?;
        lines.push(format!(
            "  nube con treeID: {} ({n} puntos dentro de copas)",
            output.display()
        ));
    }
    if let Some(f) = &j.field {
        let field = forest::read_field(&f.file, &f.columns)?;
        let plots = forest::plot_circles(
            &f.file,
            &f.columns.plot,
            &f.plot_x,
            &f.plot_y,
            f.plot_radius,
        )?;
        lines.push(forest::evaluation_text(&forest::evaluate(
            &list, &field, &plots, f.max_dist,
        )));
    }
    Ok(lines.join("\n"))
}

/// Clips clouds with regions and says what each output holds.
pub fn clip(job: &crate::clip::ClipJob) -> Result<String> {
    let (moved, unified) = crate::clip::unified(job)?;
    let r = crate::clip::run(&moved)?;
    let warnings = unified.warnings.clone();
    let mut lines: Vec<String> = r
        .files
        .iter()
        .map(|f| {
            format!(
                "  {:>10} puntos  {}  ({})",
                f.points,
                f.path.display(),
                f.region
            )
        })
        .collect();
    if r.no_ground > 0 {
        lines.push(format!(
            "  {} puntos sin MDT debajo, descartados",
            r.no_ground
        ));
    }
    lines.insert(
        0,
        format!(
            "{} puntos leídos, {} escritos en {} archivo(s), {} pasada(s) sobre la nube",
            r.points_read,
            r.points_written,
            r.files.len(),
            r.passes
        ),
    );
    if let Some(m) = &job.metrics {
        lines.push(format!("  métricas por parcela: {}", m.csv.display()));
    }
    Ok(lines.join("\n") + &crate::crs::notes(&warnings))
}

/// Default Kraus & Pfeifer parameters, for building steps.
pub fn fusion_ground() -> Ground {
    Ground::Fusion(GroundParams::default())
}
