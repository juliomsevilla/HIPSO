//! Cada herramienta como trabajo: sus parámetros en un valor, ejecutado por una función que
//! devuelve un resumen. La línea de comandos y la aplicación de escritorio construyen trabajos y
//! los ejecutan aquí, de modo que siempre se comportan igual.

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

/// Un informe completo de los archivos: cabecera, puntos, avisos de calidad.
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
    if !sum.failed.is_empty() {
        out.push(format!(
            "AVISO: {} archivo(s) no se pudieron leer y no cuentan en el total:",
            sum.failed.len()
        ));
        for (p, e) in &sum.failed {
            out.push(format!("  {}: {e}", p.display()));
        }
    }
    Ok(out.join(
        "
",
    ))
}

/// Lo que hizo una clasificación, nombrando solo las clases que tocó.
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
    classify_with(j, &[])
}

/// [`classify`] sobre una hoja de un bloque: `context` son las hojas vecinas, cuyos puntos ven los
/// filtros cerca de los bordes de la hoja; el archivo escrito tiene solo los puntos de la hoja.
pub fn classify_with(j: &ClassifyJob, context: &[PathBuf]) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let r = pipeline::classify_with_context(&j.inputs, context, &j.output, &j.steps, &j.tiling)?;
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
    /// Ventana de suavizado en unidades del mapa (0: ninguna) y su tipo.
    pub smooth: f64,
    pub smoothing: trees::Smoothing,
    /// Líneas de ruptura (shapefile de líneas o polígonos, con o sin elevaciones) que debe seguir
    /// el modelo del suelo; solo superficies TIN e IDW.
    pub breaklines: Option<PathBuf>,
}

/// El filtro de Kraus y Pfeifer sobre un archivo, con una superficie media y ningún otro paso, se
/// ejecuta en flujo sobre todo el archivo (mismo resultado que por teselas); todo lo demás se
/// ejecuta por teselas.
pub fn dtm(j: &DtmJob) -> Result<String> {
    dtm_with(j, &[])
}

/// [`dtm`] sobre una hoja de un bloque: `context` son las hojas vecinas, cuyos puntos ve la
/// triangulación cerca de los bordes de la hoja (hasta donde llega el margen de las teselas).
pub fn dtm_with(j: &DtmJob, context: &[PathBuf]) -> Result<String> {
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
        && context.is_empty()
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
    let (dtm, rep, epsg) = pipeline::dtm_in(
        &j.inputs,
        context,
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

/// Cómo se hace un modelo de superficie (retornos más altos).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DsmMethod {
    /// Punto más alto de cada celda, en una pasada en flujo.
    Max,
    /// TIN por el punto más alto de cada celda, tesela a tesela.
    Tin,
    /// Ponderación por el inverso de la distancia de los puntos más altos de las celdas, tesela a
    /// tesela.
    Idw,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DsmJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub cell: f64,
    pub method: DsmMethod,
    /// Rellena las celdas vacías de la superficie en flujo.
    pub fill: bool,
    pub tiling: Tiling,
    pub smooth: f64,
    pub smoothing: trees::Smoothing,
}

/// Lo que necesita una hoja de un bloque además de sus propios puntos para tener en sus bordes lo
/// que habría dado el bloque: las hojas vecinas (leídas como contexto por los métodos que
/// trianguan), la franja de sus puntos cerca de esta hoja (para los métodos que toman el punto más
/// alto de una celda), sus modelos del suelo y la extensión a la que se recorta el resultado.
#[derive(Default)]
pub struct SheetContext {
    pub extra: Vec<PathBuf>,
    pub halo: Vec<crate::las::Point>,
    /// Hasta dónde (m) llega la franja más allá de la hoja.
    pub margin: f64,
    pub ground_around: Vec<PathBuf>,
    /// `[x mínima, y mínima, x máxima, y máxima]` de la hoja; el resultado se recorta a ella.
    pub crop: Option<[f64; 4]>,
}

impl SheetContext {
    fn cut(&self, g: Grid) -> Result<Grid> {
        match self.crop {
            Some(b) => g.crop(b),
            None => Ok(g),
        }
    }
}

pub fn dsm(j: &DsmJob) -> Result<String> {
    dsm_with(j, &SheetContext::default())
}

/// [`dsm`] sobre una hoja de un bloque, con su contexto.
pub fn dsm_with(j: &DsmJob, ctx: &SheetContext) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let (g, epsg) = match j.method {
        DsmMethod::Max => {
            let r = Reader::open(&j.input)?;
            (
                chm::dsm_in(&r, &ctx.halo, ctx.margin, j.cell, j.fill)?,
                r.header.epsg,
            )
        }
        DsmMethod::Tin | DsmMethod::Idw => pipeline::dsm_in(
            std::slice::from_ref(&j.input),
            &ctx.extra,
            j.cell,
            if j.method == DsmMethod::Idw {
                Surface::Idw
            } else {
                Surface::Tin
            },
            &j.tiling,
        )?,
    };
    // suavizado sobre toda la superficie y recortado después, de modo que el suavizado tampoco
    // tiene costura
    let g = ctx.cut(smoothed(g, j.smooth, j.smoothing))?;
    g.write_tiff(&j.output, epsg)?;
    Ok(format!(
        "MDS de {} x {}: {}",
        g.cols,
        g.rows,
        j.output.display()
    ))
}

/// Una superficie suavizada con una ventana de `size` unidades del mapa (sin cambios cuando `size`
/// es 0).
fn smoothed(g: Grid, size: f64, kind: trees::Smoothing) -> Grid {
    if size > 0.0 {
        trees::smooth(&g, size, kind, trees::Shape::Circle)
    } else {
        g
    }
}

/// Un modelo de superficie convertido en alturas sobre `dtm` (las alturas negativas cuentan como
/// 0); las celdas que el MDT no cubre quedan vacías.
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

/// Cómo se hace un CHM.
#[derive(Clone, Debug, PartialEq)]
pub enum ChmMethod {
    /// Punto más alto por celda; las celdas vacías se rellenan salvo que `fill` sea falso.
    Max { fill: bool },
    /// Pila de TIN sin picos.
    PitFree(PitFree),
    /// TIN por el punto más alto de cada celda, menos el MDT.
    Tin,
    /// Ponderación por el inverso de la distancia de los puntos más altos de las celdas, menos el
    /// MDT.
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
    /// Ventana de suavizado en unidades del mapa (0: ninguna) y su tipo.
    pub smooth: f64,
    pub smoothing: trees::Smoothing,
}

pub fn chm(j: &ChmJob) -> Result<String> {
    chm_with(j, &SheetContext::default())
}

/// [`chm`] sobre una hoja de un bloque, con su contexto.
pub fn chm_with(j: &ChmJob, ctx: &SheetContext) -> Result<String> {
    let u = crate::crs::unify(&[j.input.as_path(), j.dtm.as_path()], false)?;
    let j = ChmJob {
        input: u.path(&j.input),
        dtm: u.path(&j.dtm),
        ..j.clone()
    };
    chm_run(&j, ctx).map(|s| s + &u.notes())
}

fn chm_run(j: &ChmJob, ctx: &SheetContext) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let mut ground = Grid::read(&j.dtm)?;
    // los puntos de las vecinas necesitan el suelo bajo ellos
    if !ctx.ground_around.is_empty() && ctx.margin > 0.0 {
        ground = grown_with_neighbours(ground, &ctx.ground_around, ctx.margin)?.0;
    }
    let (chm, epsg) = match &j.method {
        ChmMethod::Max { fill } => {
            let r = Reader::open(&j.input)?;
            (
                chm::chm_in(&r, &ctx.halo, ctx.margin, &ground, j.cell, *fill)?,
                r.header.epsg,
            )
        }
        ChmMethod::Tin | ChmMethod::Idw => {
            let surface = if j.method == ChmMethod::Idw {
                Surface::Idw
            } else {
                Surface::Tin
            };
            let (dsm, epsg) = pipeline::dsm_in(
                std::slice::from_ref(&j.input),
                &ctx.extra,
                j.cell,
                surface,
                &j.tiling,
            )?;
            (above_ground(dsm, &ground), epsg)
        }
        ChmMethod::PitFree(p) => pipeline::chm_pitfree_in(
            std::slice::from_ref(&j.input),
            &ctx.extra,
            &ground,
            j.cell,
            p,
            &j.tiling,
        )?,
    };
    let chm = ctx.cut(smoothed(chm, j.smooth, j.smoothing))?;
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
    /// `<nombre de salida>_<sufijo>.csv` junto a la salida.
    pub fn sibling(&self, suffix: &str) -> PathBuf {
        let stem = self
            .output
            .file_stem()
            .map_or("metrics".into(), |s| s.to_string_lossy().into_owned());
        self.output.with_file_name(format!("{stem}_{suffix}.csv"))
    }
}

/// Métricas por celda. Una salida `.shp` (la de por defecto) da un polígono por celda con las
/// métricas como atributos (los nombres de campo DBF son cortos: `<nombre>_campos.txt` lista su
/// significado); una salida `.csv` conserva la tabla de texto compatible con FUSION.
pub fn metrics(j: &MetricsJob) -> Result<String> {
    metrics_with(j, &MetricsContext::default())
}

/// Lo que necesita una hoja de un bloque además de sus propios puntos y su modelo del suelo (véase
/// [`crate::metrics::Context`]): la franja de los puntos de sus vecinas, los modelos del suelo de
/// las vecinas y qué celdas escribe la hoja.
#[derive(Default)]
pub struct MetricsContext {
    pub halo: Vec<crate::las::Point>,
    /// Modelos del suelo de las hojas vecinas, usados `margin` metros más allá del de esta.
    pub ground_around: Vec<PathBuf>,
    pub margin: f64,
    pub owns: Option<Box<dyn Fn(f64, f64) -> bool + Send + Sync>>,
}

/// [`metrics`] sobre una hoja de un bloque, con su contexto.
pub fn metrics_with(j: &MetricsJob, ctx: &MetricsContext) -> Result<String> {
    let u = crate::crs::unify(&[j.input.as_path(), j.dtm.as_path()], false)?;
    let j = MetricsJob {
        input: u.path(&j.input),
        dtm: u.path(&j.dtm),
        ..j.clone()
    };
    metrics_run(&j, ctx).map(|s| s + &u.notes())
}

fn metrics_run(j: &MetricsJob, ctx: &MetricsContext) -> Result<String> {
    paths::ensure_parent(&j.output)?;
    let f = j.formats.fixed();
    let csv_main = paths::swap_ext(&j.output, "csv");
    if !f.vector {
        let job = MetricsJob {
            output: csv_main.clone(),
            ..j.clone()
        };
        return metrics_csv(&job, &csv_main, ctx);
    }
    // las capas vectoriales se hacen a partir de las tablas de texto, que se conservan cuando se
    // piden
    let csv_target = if f.csv {
        csv_main
    } else {
        paths::swap_ext(&j.output, "tmp.csv")
    };
    let tmp_job = MetricsJob {
        output: csv_target.clone(),
        ..j.clone()
    };
    let summary = metrics_csv(&tmp_job, &csv_target, ctx)?;
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

fn metrics_csv(j: &MetricsJob, output: &std::path::Path, ctx: &MetricsContext) -> Result<String> {
    let mut out = BufWriter::new(File::create(output)?);
    let r = Reader::open(&j.input)?.for_passes()?;
    let mut ground = Grid::read(&j.dtm)?;
    if !ctx.ground_around.is_empty() && ctx.margin > 0.0 {
        ground = grown_with_neighbours(ground, &ctx.ground_around, ctx.margin)?.0;
    }
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
    let rep = metrics::grid_metrics_in(
        &r,
        &ground,
        &j.params,
        outs,
        &metrics::Context {
            halo: &ctx.halo,
            owns: ctx.owns.as_deref().map(|f| f as &(dyn Fn(f64, f64) -> bool + Sync)),
        },
    )?;
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

/// Inventario de campo con el que comprobar las detecciones.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldJob {
    pub file: PathBuf,
    pub columns: crate::forest::FieldColumns,
    /// Columnas del centro de la parcela y radio de la parcela; las detecciones cuentan solo dentro
    /// de las parcelas.
    pub plot_x: String,
    pub plot_y: String,
    pub plot_radius: f64,
    pub max_dist: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreesJob {
    pub chm: PathBuf,
    /// Ruta base de la tabla de árboles (su extensión se sustituye por los formatos elegidos).
    pub output: PathBuf,
    pub formats: paths::Formats,
    /// Ráster de copas con los identificadores de árbol.
    pub crowns: Option<PathBuf>,
    pub crowns_shp: Option<PathBuf>,
    pub tops_shp: Option<PathBuf>,
    /// Una nube y la copia suya que recibe un atributo `treeID`.
    pub cloud: Option<(PathBuf, PathBuf)>,
    /// Cimas de árboles dadas a mano (CSV: x,y por línea) en lugar de buscadas.
    pub tops_file: Option<PathBuf>,
    pub tops: TopParams,
    pub crown: CrownParams,
    /// Ventana de suavizado del CHM en unidades del mapa (0: ninguna).
    pub smooth: f64,
    pub smoothing: trees::Smoothing,
    pub field: Option<FieldJob>,
    /// Modelos de copas de las hojas que rodean a esta (mismo tamaño de celda). Con ellos, los
    /// árboles se buscan en este modelo ampliado `margin` metros por cada lado con lo que tienen
    /// allí las vecinas, y solo se conservan los árboles cuya cima está en esta hoja: un árbol
    /// cortado por el borde de una hoja lo encuentra una sola vez, entero, la hoja que contiene su
    /// cima.
    pub around: Vec<PathBuf>,
    pub margin: f64,
    /// Escribe además las cimas y las copas juntas en un GeoPackage (`<salida>.gpkg`, tablas
    /// `apices` y `copas`).
    pub gpkg: bool,
    /// Nubes cuyos puntos dentro de cada copa se miden (las métricas de Extraer muestras); los
    /// polígonos de copa las llevan como atributos. Vacío: sin métricas.
    pub crown_metrics: Vec<PathBuf>,
}

/// Una fila de una capa construida en memoria: los anillos de un elemento y sus valores de
/// atributo.
type LayerRow = (Vec<Vec<[f64; 2]>>, Vec<String>);

/// Las métricas de copa de cada árbol, como columnas y como una fila de valores por árbol.
type CrownTable = (Vec<String>, std::collections::HashMap<u32, Vec<String>>);

/// Una capa vectorial construida en memoria: un elemento por fila, sus anillos y sus valores de
/// atributo.
fn vector_of(
    fields: Vec<String>,
    rows: Vec<LayerRow>,
    polygons: bool,
) -> crate::view::VectorData {
    let mut d = crate::view::VectorData {
        fields,
        polygons,
        points: !polygons,
        bounds: [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
        ..Default::default()
    };
    for (parts, values) in rows {
        for p in parts.iter().flatten() {
            d.bounds = [
                d.bounds[0].min(p[0]),
                d.bounds[1].min(p[1]),
                d.bounds[2].max(p[0]),
                d.bounds[3].max(p[1]),
            ];
        }
        d.num.push(
            values
                .iter()
                .map(|t| t.trim().parse().unwrap_or(f64::NAN))
                .collect(),
        );
        d.text.push(values);
        d.features.push(crate::view::Feature { parts });
    }
    d
}

/// Las métricas de Extraer muestras para los puntos dentro de cada copa: los nombres de las
/// columnas y, por identificador de árbol, sus valores.
fn crown_metrics(
    clouds: &[PathBuf],
    trees: &[trees::Tree],
    hulls: &std::collections::BTreeMap<u32, Vec<[f64; 2]>>,
    base: &std::path::Path,
) -> Result<CrownTable> {
    use crate::clip::{ClipJob, ClipMode, SampleMetrics};
    use crate::vector::{Polygon, Region, Shape2};
    let regions: Vec<Region> = trees
        .iter()
        .filter_map(|t| {
            let h = hulls.get(&t.id).filter(|h| h.len() >= 3)?;
            Some(Region {
                name: t.id.to_string(),
                shape: Shape2::Polygon(Polygon::new(vec![h.clone()])),
                attrs: Vec::new(),
            })
        })
        .collect();
    if regions.is_empty() {
        return Ok((Vec::new(), Default::default()));
    }
    let csv = paths::swap_ext(base, "csv").with_file_name(format!(
        "{}_metricas_copas_tmp.csv",
        paths::stem(base)
    ));
    let mut job = ClipJob::new(clouds.to_vec(), regions, csv.clone());
    job.mode = ClipMode::MetricsOnly;
    job.metrics = Some(SampleMetrics {
        csv: csv.clone(),
        formats: paths::Formats { vector: false, csv: true },
        heightbreak: 2.0,
        min_height: 0.0,
        only: Vec::new(),
    });
    crate::clip::run(&job)?;
    let text = std::fs::read_to_string(&csv);
    let _ = std::fs::remove_file(&csv);
    let text = text?;
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
    // Muestra, Area, las métricas, Date
    let names: Vec<String> = head
        .get(2..head.len().saturating_sub(1))
        .unwrap_or(&[])
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut by_id = std::collections::HashMap::new();
    for l in lines {
        let f: Vec<&str> = l.split(',').collect();
        if let Some(id) = f.first().and_then(|v| v.trim().parse::<u32>().ok()) {
            by_id.insert(
                id,
                f.get(2..f.len().saturating_sub(1))
                    .unwrap_or(&[])
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            );
        }
    }
    Ok((names, by_id))
}

/// Cimas a partir de un CSV de líneas `x,y` (se salta una línea de cabecera), con su altura leída
/// del CHM.
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

/// El modelo de esta hoja ampliado `margin` con lo que tienen allí las hojas de alrededor; la
/// extensión propia y el número de celdas añadidas por cada lado.
fn grown_with_neighbours(
    own: Grid,
    around: &[PathBuf],
    margin: f64,
) -> Result<(Grid, [f64; 4], usize)> {
    let b = [
        own.x0,
        own.y0,
        own.x0 + own.cols as f64 * own.cell,
        own.y0 + own.rows as f64 * own.cell,
    ];
    let k = (margin / own.cell).ceil() as usize;
    let mut g = Grid::new(
        own.x0 - k as f64 * own.cell,
        own.y0 - k as f64 * own.cell,
        own.cell,
        own.cols + 2 * k,
        own.rows + 2 * k,
    )?;
    g.fill_from(&own);
    let reach = k as f64 * own.cell;
    for path in around {
        crate::check_cancel()?;
        let Ok((e, cell)) = Grid::read_extent(path) else {
            continue;
        };
        let touches = e[0] < b[2] + reach
            && e[2] > b[0] - reach
            && e[1] < b[3] + reach
            && e[3] > b[1] - reach;
        if !touches || (cell - own.cell).abs() > own.cell * 1e-6 {
            continue;
        }
        g.fill_from(&Grid::read(path)?);
    }
    Ok((g, b, k))
}

fn trees_run(j: &TreesJob) -> Result<String> {
    use crate::forest;
    let mut g = Grid::read(&j.chm)?;
    // la extensión propia y el margen, cuando se usan las vecinas
    let mut owned: Option<([f64; 4], usize)> = None;
    if !j.around.is_empty() && j.margin > 0.0 {
        let (grown, own, k) = grown_with_neighbours(g, &j.around, j.margin)?;
        g = grown;
        owned = Some((own, k));
    }
    if j.smooth > 0.0 {
        g = trees::smooth(&g, j.smooth, j.smoothing, j.tops.shape);
    }
    let mut list = match &j.tops_file {
        Some(f) => manual_tops(f, &g)?,
        None => trees::tops(&g, &j.tops),
    };
    let mut ids = trees::crowns(&g, &mut list, &j.crown);
    // los árboles cuya cima está en esta hoja (los bordes oeste y sur le pertenecen, los este y
    // norte a la hoja siguiente, de modo que ningún árbol se encuentra dos veces)
    let mut dropped = 0usize;
    if let Some((b, _)) = owned {
        let mine = |t: &trees::Tree| t.x >= b[0] && t.x < b[2] && t.y >= b[1] && t.y < b[3];
        let keep: std::collections::HashSet<u32> =
            list.iter().filter(|t| mine(t)).map(|t| t.id).collect();
        dropped = list.len() - keep.len();
        list.retain(|t| keep.contains(&t.id));
        for id in &mut ids {
            if *id != 0 && !keep.contains(id) {
                *id = 0;
            }
        }
    }
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
        // las cimas de los árboles con sus atributos
        let path = paths::swap_ext(&j.output, "shp");
        forest::write_tops_shp(&path, &list, &hulls, epsg)?;
        written.push(path.display().to_string());
    }
    let mut lines = vec![format!("{} individuos: {}", list.len(), written.join(", "))];
    if owned.is_some() {
        lines.push(format!(
            "  con las hojas vecinas: {dropped} árbol(es) con la cima en otra hoja se dejan a esa hoja"
        ));
    }
    if let Some(path) = &j.crowns {
        paths::ensure_parent(path)?;
        // con vecinas, el ráster de copas se recorta de nuevo a la extensión propia de la hoja
        let cut: Option<(Grid, Vec<u32>)> = owned.map(|(b, k)| {
            let mut own = Grid::new(b[0], b[1], g.cell, g.cols - 2 * k, g.rows - 2 * k)
                .unwrap_or_else(|_| g.like());
            let mut own_ids = vec![0u32; own.data.len()];
            for r in 0..own.rows {
                for c in 0..own.cols {
                    own_ids[r * own.cols + c] = ids[(r + k) * g.cols + c + k];
                    own.data[r * own.cols + c] = g.data[(r + k) * g.cols + c + k];
                }
            }
            (own, own_ids)
        });
        let (g, ids): (&Grid, &Vec<u32>) = match &cut {
            Some((a, b)) => (a, b),
            None => (&g, &ids),
        };
        // un decimal de 32 bits guarda números enteros exactamente hasta 2^24 y todo visor lo lee;
        // más allá, los números van en un ráster entero, que los conserva todos
        if ids.iter().copied().max().unwrap_or(0) > 1 << 24 {
            let labels = crate::obia::Labels {
                lat: crate::obia::Lattice::of(g),
                count: ids.iter().copied().max().unwrap_or(0),
                ids: ids.clone(),
            };
            labels.write_tiff(path, Grid::read_epsg(&j.chm).or(epsg))?;
        } else {
            let mut r = g.like();
            for (v, id) in r.data.iter_mut().zip(ids) {
                if *id > 0 {
                    *v = *id as f32;
                }
            }
            r.write_tiff(path, None)?;
        }
        lines.push(format!("  individuos (ráster): {}", path.display()));
    }
    // los polígonos de copa como capa en memoria, cuando llevan métricas o van en el GeoPackage
    let want_layer = j.crowns_shp.is_some() || j.gpkg;
    let crown_layer = if want_layer && (j.gpkg || !j.crown_metrics.is_empty()) {
        let (names, by_id) = if j.crown_metrics.is_empty() {
            (Vec::new(), Default::default())
        } else {
            crown_metrics(&j.crown_metrics, &list, &hulls, &j.output)?
        };
        let mut fields: Vec<String> = ["TREEID", "Z", "AREA"].map(String::from).to_vec();
        fields.extend(names.iter().cloned());
        let rows = list
            .iter()
            .filter_map(|t| {
                let h = hulls.get(&t.id).filter(|h| h.len() >= 3)?;
                let mut v = vec![
                    t.id.to_string(),
                    format!("{:.2}", t.height),
                    format!("{:.2}", forest::polygon_area(h)),
                ];
                match by_id.get(&t.id) {
                    Some(m) => v.extend(m.iter().cloned()),
                    None => v.extend(names.iter().map(|_| String::new())),
                }
                Some((vec![h.clone()], v))
            })
            .collect();
        Some(vector_of(fields, rows, true))
    } else {
        None
    };
    if let Some(path) = &j.crowns_shp {
        paths::ensure_parent(path)?;
        match &crown_layer {
            Some(d) if !j.crown_metrics.is_empty() => {
                let prj = epsg.and_then(crate::vector::prj_for_epsg);
                crate::vector::write_layer(path, d, crate::view::GeomKind::Polygon, prj.as_deref())?;
            }
            _ => forest::write_crowns_shp(path, &list, &hulls)?,
        }
        lines.push(format!("  individuos (polígonos): {}", path.display()));
    }
    if j.gpkg {
        let path = paths::swap_ext(&j.output, "gpkg");
        paths::ensure_parent(&path)?;
        let tops = vector_of(
            ["TREEID", "Z", "AREA", "HULLAREA", "CROWNDIAM"].map(String::from).to_vec(),
            list.iter()
                .map(|t| {
                    let hull = hulls.get(&t.id).map_or(0.0, |h| forest::polygon_area(h));
                    (
                        vec![vec![[t.x, t.y]]],
                        vec![
                            t.id.to_string(),
                            format!("{:.2}", t.height),
                            format!("{:.2}", t.area),
                            format!("{hull:.2}"),
                            format!("{:.2}", 2.0 * (t.area / std::f64::consts::PI).sqrt()),
                        ],
                    )
                })
                .collect(),
            false,
        );
        let mut tables = vec![crate::gpkg_write::Table {
            name: "apices",
            data: &tops,
            kind: crate::view::GeomKind::Point,
            epsg,
        }];
        if let Some(d) = &crown_layer {
            tables.push(crate::gpkg_write::Table {
                name: "copas",
                data: d,
                kind: crate::view::GeomKind::Polygon,
                epsg,
            });
        }
        crate::gpkg_write::write(&path, &tables)?;
        lines.push(format!("  GeoPackage (ápices y copas): {}", path.display()));
    }
    // el shapefile de cimas ya es el resultado principal cuando la salida vectorial está activada
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

/// Recorta nubes con regiones y dice qué contiene cada salida.
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
        lines.push(format!("  métricas por muestra: {}", m.csv.display()));
    }
    Ok(lines.join("\n") + &crate::crs::notes(&warnings))
}

/// Parámetros por defecto de Kraus y Pfeifer, para construir pasos.
pub fn fusion_ground() -> Ground {
    Ground::Fusion(GroundParams::default())
}
