//! Análisis de terreno sobre un MDT (o cualquier rejilla de alturas): pendiente, orientación,
//! sombreado, curvaturas, rugosidad, escabrosidad e índices de posición (ventanas 3x3), los
//! productos de drenaje de [`crate::hydro`] y la cuenca visual de [`crate::viewshed`]. Cada
//! producto es un GeoTIFF del tamaño de la entrada; la rejilla se lee una vez y cada ventana se
//! ejecuta en paralelo por filas.
//!
//! Los métodos son los publicados: Horn (1981) para pendiente, orientación y sombreado, Zevenbergen
//! y Thorne (1987) para las curvaturas, Wilson et al. (2007) para el índice de escabrosidad y el
//! índice de posición de Weiss (2001). Convenios: por defecto, una celda cuya ventana 3x3 llega más
//! allá del borde o a una celda vacía no tiene valor (véase [`Edges`] para las alternativas), y una
//! celda vacía sigue vacía. Escrito a partir de los métodos publicados.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::{Error, Result};

/// Qué calcular.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Product {
    Slope,
    Aspect,
    Hillshade,
    /// Curvatura transversal a la pendiente (positiva: convexa, como en un espolón).
    PlanCurvature,
    /// Curvatura a lo largo de la pendiente (positiva: convexa, como en un hombro).
    ProfileCurvature,
    /// Total curvature (positive: convex, ridges; negative: concave, valleys).
    Curvature,
    /// Altura más alta menos más baja en la ventana 3x3.
    Roughness,
    /// Escabrosidad del terreno: diferencia absoluta media con los ocho vecinos.
    Ruggedness,
    /// Altura menos la altura media dentro de un radio.
    Position,
    /// Cuán profundas son las depresiones que elimina rellenarlas.
    FillDepth,
    /// Superficie drenada, en m².
    FlowAccumulation,
    /// Topographic wetness index `ln(a / tan β)`.
    Wetness,
    /// Celdas cuya superficie drenada alcanza el umbral.
    Streams,
    /// Cuencas de los tramos de cauce, numeradas.
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

/// Qué hace una ventana 3x3 donde llega más allá del borde del ráster o a una celda vacía.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Edges {
    /// La celda no tiene valor: el resultado nunca se calcula con alturas que no están.
    #[default]
    NoData,
    /// Los vecinos que faltan (borde o vacíos) toman la altura del centro: cada celda recibe un
    /// valor, cerca de los bordes y de los huecos la superficie se aplana.
    Center,
    /// Más allá del borde se repiten las celdas del borde (una ventana del borde como si la
    /// superficie siguiera a nivel); una ventana sobre una celda vacía dentro del ráster sigue sin
    /// valor.
    RepeatEdge,
}

#[derive(Clone, Debug)]
pub struct TerrainJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub product: Product,
    /// Pendiente en porcentaje en lugar de grados.
    pub percent: bool,
    /// Multiplica las alturas cuando no están en las unidades de la celda (1 para metros).
    pub z_factor: f64,
    /// Sombreado: dirección de la luz, grados en el sentido de las agujas del reloj desde el norte.
    pub azimuth: f64,
    /// Sombreado: altura de la luz sobre el horizonte, grados.
    pub altitude: f64,
    /// Índice de posición: radio del entorno, unidades del mapa.
    pub radius: f64,
    /// Cauces y subcuencas: superficie drenada que forma un cauce, m².
    pub threshold: f64,
    /// Cuenca visual: el observador (x, y).
    pub observer: Option<[f64; 2]>,
    /// Cuenca visual: altura del ojo sobre el suelo.
    pub observer_height: f64,
    /// Cuenca visual: altura sobre el suelo que debe verse.
    pub target_height: f64,
    /// Cuenca visual: hasta dónde mira el observador, unidades del mapa.
    pub max_distance: f64,
    /// Productos 3x3: qué hacen las ventanas sobre bordes y celdas vacías.
    pub edges: Edges,
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
            edges: Edges::NoData,
        }
    }
}

/// Calcula el producto sobre una rejilla en memoria.
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
        Product::Slope => slope(g, z, job.percent, job.edges),
        Product::Aspect => aspect(g, z, job.edges),
        Product::Hillshade => hillshade(g, z, job.azimuth, job.altitude, job.edges),
        Product::PlanCurvature => curvature(g, z, Curv::Plan, job.edges),
        Product::ProfileCurvature => curvature(g, z, Curv::Profile, job.edges),
        Product::Curvature => curvature(g, z, Curv::Total, job.edges),
        Product::Roughness => roughness(g, z, job.edges),
        Product::Ruggedness => ruggedness(g, z, job.edges),
        Product::Position => position(g, z, job.radius)?,
        Product::FillDepth => crate::hydro::fill_depth(g, z)?,
        Product::FlowAccumulation => crate::hydro::flow_accumulation(g, z)?,
        Product::Wetness => crate::hydro::wetness(g, z)?,
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

/// Los rásteres con más celdas que esto se trabajan en franjas (véase [`run_streamed`]).
const STREAM_CELLS: u64 = 32_000_000;

/// Filas de margen que necesita una franja para dar, celda a celda, lo que daría el ráster entero:
/// los productos que miran solo el entorno de una celda. `None` para los que necesitan el ráster
/// entero (flujo, depresiones, cuencas, visibilidad).
fn halo(job: &TerrainJob, cell: f64) -> Option<usize> {
    match job.product {
        Product::Slope
        | Product::Aspect
        | Product::Hillshade
        | Product::PlanCurvature
        | Product::ProfileCurvature
        | Product::Curvature
        | Product::Roughness
        | Product::Ruggedness => Some(1),
        Product::Position => Some((job.radius / cell).round().max(1.0) as usize + 1),
        _ => None,
    }
}

/// El producto de un ráster de cualquier tamaño, en franjas de filas con el margen que necesita el
/// producto: solo hay una franja en memoria (con sus teselas decodificadas en todos los núcleos) y
/// su resultado se escribe en cuanto se hace, de modo que un ráster de miles de millones de celdas
/// ocupa la memoria de unos pocos millones. El resultado es el mismo que trabajando el ráster
/// entero.
fn run_streamed(job: &TerrainJob, pyr: &crate::pyramid::Pyramid, margin: usize) -> Result<String> {
    use crate::tiled::{Compression, Spec};
    let e = pyr.extent();
    let cell = pyr.cell;
    let (cols, rows) = (
        ((e[2] - e[0]) / cell).round() as usize,
        ((e[3] - e[1]) / cell).round() as usize,
    );
    if pyr.bands != 1 {
        return Err(Error::Param("el análisis del terreno necesita un ráster de una banda".into()));
    }
    let nodata = pyr.nodata();
    // una franja (y su margen) ocupa una parte de la memoria del equipo: 33 millones de celdas en
    // uno de 16 GB, 134 millones en uno de 64 GB, de modo que un equipo grande trabaja en menos
    // franjas, más largas
    let budget = crate::copc_cache::total_memory().map_or(24_000_000, |m| (m / 512).clamp(16_000_000, 256_000_000));
    let step = (budget as usize / cols.max(1)).clamp(64, 16_384).max(margin * 8);
    crate::paths::ensure_parent(&job.output)?;
    let spec = Spec {
        cols,
        rows,
        bands: 1,
        dtype: crate::rastertools::Dtype::F32,
        cell,
        x0: e[0],
        top: e[3],
        epsg: pyr.epsg,
        nodata: crate::grid::NODATA,
        compression: Compression::Deflate,
        nearest: false,
        bigtiff: false,
    };
    let (mut lo, mut hi, mut n) = (f32::INFINITY, f32::NEG_INFINITY, 0u64);
    let started = std::time::Instant::now();
    crate::progress::begin(rows as u64);
    crate::tiled::write(&job.output, spec, step, |r0, r1| {
        crate::progress::advance((r1 - r0) as u64);
        let (w0, w1) = (r0.saturating_sub(margin), (r1 + margin).min(rows));
        // un cuarto de celda dentro de la franja, para que los límites de píxel sean los correctos
        let q = cell * 0.25;
        let win = [
            e[0] + q,
            e[3] - w1 as f64 * cell + q,
            e[2] - q,
            e[3] - w0 as f64 * cell - q,
        ];
        let mut img = pyr.read(0, win)?;
        if img.cols != cols || img.rows != w1 - w0 {
            return Err(Error::Format(format!(
                "la franja leída mide {} x {} y se esperaba {} x {}",
                img.cols,
                img.rows,
                cols,
                w1 - w0
            )));
        }
        let mut data = std::mem::take(&mut img.bands[0]);
        for v in &mut data {
            if !v.is_finite() || nodata.is_some_and(|nd| *v == nd) {
                *v = f32::NAN;
            }
        }
        let g = Grid {
            x0: img.x0,
            y0: img.y0,
            cell,
            cols,
            rows: w1 - w0,
            data,
        };
        let out = compute(&g, job)?;
        let core = &out.data[(r0 - w0) * cols..(r1 - w0) * cols];
        for v in core.iter().filter(|v| !v.is_nan()) {
            lo = lo.min(*v);
            hi = hi.max(*v);
            n += 1;
        }
        Ok(core.to_vec())
    })?;
    Ok(if n == 0 {
        format!("{}: sin celdas con valor", job.product.name())
    } else {
        format!(
            "{}: {n} celdas, de {lo:.4} a {hi:.4}, por franjas de {step} filas en {:.0} s -> {}",
            job.product.name(),
            started.elapsed().as_secs_f64(),
            job.output.display()
        )
    })
}

static STREAM_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Fija si los rásteres grandes se procesan por franjas (`Some(true)`: siempre; `Some(false)`:
/// nunca) o lo decide su tamaño (`None`, salvo que `HIPSO_STREAM` diga otra cosa).
pub fn set_stream_mode(forced: Option<bool>) {
    let v = match forced {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    STREAM_MODE.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// Lee la entrada, calcula el producto y lo escribe como GeoTIFF junto al sistema de la entrada.
pub fn run(job: &TerrainJob) -> Result<String> {
    if let Ok(pyr) = crate::pyramid::Pyramid::open(&job.input)
        && pyr.bands == 1
        && let Some(margin) = halo(job, pyr.cell)
    {
        let e = pyr.extent();
        let cells = (((e[2] - e[0]) / pyr.cell) * ((e[3] - e[1]) / pyr.cell)) as u64;
        // HIPSO_STREAM=1 fuerza las franjas, =0 las prohíbe (para comparar las dos maneras)
        let forced = std::env::var("HIPSO_STREAM").ok();
        let by_mode = match STREAM_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        };
        if by_mode.unwrap_or(match forced.as_deref() {
            Some("0") => false,
            Some(_) => true,
            None => cells > STREAM_CELLS,
        }) {
            return run_streamed(job, &pyr, margin);
        }
    }
    if matches!(
        job.product,
        Product::FillDepth
            | Product::FlowAccumulation
            | Product::Wetness
            | Product::Streams
            | Product::Subbasins
    ) && let Ok(pyr) = crate::pyramid::Pyramid::open(&job.input)
        && let Some(mem) = crate::copc_cache::total_memory()
    {
        // estos siguen el agua por todo el ráster: se guarda en memoria, con las direcciones, el
        // orden y el resultado (medido: 16 bytes por celda). Se dice antes de leer
        let e = pyr.extent();
        let cells = ((e[2] - e[0]) / pyr.cell) * ((e[3] - e[1]) / pyr.cell);
        let need = cells * 20.0;
        if need > mem as f64 * 0.7 {
            return Err(Error::Param(format!(
                "{} sobre {:.0} millones de celdas necesita unos {:.0} GB de memoria (este equipo tiene {:.0} GB): el agua se sigue por todo el ráster a la vez. Usa una celda mayor o divide la zona en cuencas",
                job.product.name().to_lowercase(),
                cells / 1e6,
                need / 1e9,
                mem as f64 / 1e9
            )));
        }
    }
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

/// La ventana 3x3 alrededor de `(c, r)`, fila a fila desde el norte: `[a b c / d e f / g h i]`.
/// `None` si el centro está vacío o falta un vecino y `edges` no lo rellena.
#[inline]
fn window(g: &Grid, c: usize, r: usize, edges: Edges) -> Option<[f32; 9]> {
    let e = *g.data.get(r * g.cols + c)?;
    if e.is_nan() {
        return None;
    }
    let mut w = [e; 9];
    for (k, slot) in w.iter_mut().enumerate() {
        let (dr, dc) = ((k / 3) as isize - 1, (k % 3) as isize - 1);
        let (rr, cc) = (r as isize + dr, c as isize + dc);
        let outside = rr < 0 || cc < 0 || rr >= g.rows as isize || cc >= g.cols as isize;
        let (rr, cc) = if outside && edges == Edges::RepeatEdge {
            (
                rr.clamp(0, g.rows as isize - 1) as usize,
                cc.clamp(0, g.cols as isize - 1) as usize,
            )
        } else if outside {
            if edges == Edges::NoData {
                return None;
            }
            continue;
        } else {
            (rr as usize, cc as usize)
        };
        match g.data.get(rr * g.cols + cc).filter(|v| !v.is_nan()) {
            Some(v) => *slot = *v,
            None if edges == Edges::Center => {}
            None => return None,
        }
    }
    Some(w)
}

/// Aplica `f` a la ventana de cada celda, con las filas en paralelo.
fn map3(g: &Grid, edges: Edges, f: impl Fn(&[f32; 9]) -> f32 + Sync) -> Grid {
    let mut out = g.like();
    out.data
        .par_chunks_mut(g.cols)
        .enumerate()
        .for_each(|(r, row)| {
            for (c, v) in row.iter_mut().enumerate() {
                if let Some(w) = window(g, c, r, edges) {
                    *v = f(&w);
                }
            }
        });
    out
}

/// Gradiente de Horn: (este, norte) de subida por unidad de distancia.
#[inline]
fn gradient(w: &[f32; 9], cell: f64, z: f64) -> (f64, f64) {
    let v = |k: usize| f64::from(w[k]);
    let k = z / (8.0 * cell);
    (
        ((v(2) + 2.0 * v(5) + v(8)) - (v(0) + 2.0 * v(3) + v(6))) * k,
        ((v(0) + 2.0 * v(1) + v(2)) - (v(6) + 2.0 * v(7) + v(8))) * k,
    )
}

pub fn slope(g: &Grid, z: f64, percent: bool, edges: Edges) -> Grid {
    let cell = g.cell;
    map3(g, edges, |w| {
        let (gx, gy) = gradient(w, cell, z);
        let s = gx.hypot(gy);
        (if percent {
            100.0 * s
        } else {
            s.atan().to_degrees()
        }) as f32
    })
}

/// Dirección a la que mira la pendiente, grados en el sentido de las agujas del reloj desde el
/// norte; las celdas llanas quedan vacías.
pub fn aspect(g: &Grid, z: f64, edges: Edges) -> Grid {
    let cell = g.cell;
    map3(g, edges, |w| {
        let (gx, gy) = gradient(w, cell, z);
        if gx.hypot(gy) < 1e-9 {
            return f32::NAN;
        }
        // cuesta abajo es (-gx, -gy) en (este, norte)
        let a = (-gx).atan2(-gy).to_degrees();
        (if a < 0.0 { a + 360.0 } else { a + 0.0 }) as f32
    })
}

/// Iluminación 0-255 de una luz con `azimuth` y `altitude` (grados).
pub fn hillshade(g: &Grid, z: f64, azimuth: f64, altitude: f64, edges: Edges) -> Grid {
    let cell = g.cell;
    let (az, alt) = (azimuth.to_radians(), altitude.to_radians());
    let (lx, ly, lz) = (az.sin() * alt.cos(), az.cos() * alt.cos(), alt.sin());
    map3(g, edges, |w| {
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

/// Curvaturas de Zevenbergen y Thorne, en 1/m; positivo es convexo.
fn curvature(g: &Grid, z: f64, kind: Curv, edges: Edges) -> Grid {
    let l = g.cell;
    map3(g, edges, |w| {
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

pub fn roughness(g: &Grid, z: f64, edges: Edges) -> Grid {
    map3(g, edges, |w| {
        let (lo, hi) = w.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |a, v| {
            (a.0.min(*v), a.1.max(*v))
        });
        ((hi - lo) as f64 * z) as f32
    })
}

pub fn ruggedness(g: &Grid, z: f64, edges: Edges) -> Grid {
    map3(g, edges, |w| {
        let sum: f32 = w.iter().map(|v| (v - w[4]).abs()).sum();
        (f64::from(sum) / 8.0 * z) as f32
    })
}

/// Altura menos la altura media de las celdas dentro de `radius` (excluido el centro). Cada fila
/// del disco se toma de sumas acumuladas, de modo que el coste crece con el radio, no con su
/// cuadrado.
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
    // sumas acumuladas a lo largo de cada fila, de las alturas y de cuántas celdas tienen una
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

    /// Una rejilla de celdas de 1 m cuya altura es `f(x, y)` en los centros de celda.
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
        // sube 0,5 m por metro hacia el este
        let g = surface(9, 9, |x, _| 0.5 * x)?;
        assert!((mid(&slope(&g, 1.0, false, Edges::NoData)) - 0.5f32.atan().to_degrees()).abs() < 1e-4);
        assert!((mid(&slope(&g, 1.0, true, Edges::NoData)) - 50.0).abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn aspect_is_the_compass_direction_downhill() -> Result<()> {
        // sube hacia el oeste: mira al este (90); sube hacia el sur: mira al norte (0)
        let east = surface(9, 9, |x, _| -x)?;
        assert!((mid(&aspect(&east, 1.0, Edges::NoData)) - 90.0).abs() < 1e-3);
        let north = surface(9, 9, |_, y| -y)?;
        let a = mid(&aspect(&north, 1.0, Edges::NoData));
        assert!(a < 1e-3 || a > 359.999, "{a}");
        let west = surface(9, 9, |x, _| x)?;
        assert!((mid(&aspect(&west, 1.0, Edges::NoData)) - 270.0).abs() < 1e-3);
        let flat = surface(9, 9, |_, _| 5.0)?;
        assert!(mid(&aspect(&flat, 1.0, Edges::NoData)).is_nan());
        Ok(())
    }

    #[test]
    fn hillshade_is_bright_toward_the_light() -> Result<()> {
        // pendiente orientada al noroeste con la luz al noroeste (315), 45 grados
        let nw = surface(9, 9, |x, y| 0.5 * (x - y))?;
        let toward = mid(&hillshade(&nw, 1.0, 315.0, 45.0, Edges::NoData));
        let away = mid(&hillshade(&nw, 1.0, 135.0, 45.0, Edges::NoData));
        assert!(toward > away + 100.0, "{toward} {away}");
        // suelo llano: 255 * sin(altitud)
        let flat = surface(9, 9, |_, _| 3.0)?;
        assert!(
            (mid(&hillshade(&flat, 1.0, 315.0, 45.0, Edges::NoData)) - 255.0 * 45f32.to_radians().sin()).abs()
                < 1e-3
        );
        Ok(())
    }

    #[test]
    fn curvature_is_positive_on_a_dome_and_negative_in_a_bowl() -> Result<()> {
        let dome = surface(9, 9, |x, y| -0.1 * ((x - 4.5).powi(2) + (y - 4.5).powi(2)))?;
        let bowl = surface(9, 9, |x, y| 0.1 * ((x - 4.5).powi(2) + (y - 4.5).powi(2)))?;
        assert!((mid(&curvature(&dome, 1.0, Curv::Total, Edges::Center)) - 0.4).abs() < 1e-4);
        assert!((mid(&curvature(&bowl, 1.0, Curv::Total, Edges::Center)) + 0.4).abs() < 1e-4);
        // fuera de la cima, en una cúpula las dos curvaturas son convexas
        let at = |g: &Grid, k: Curv| curvature(g, 1.0, k, Edges::Center).data[4 * 9 + 6];
        assert!(at(&dome, Curv::Profile) > 0.0 && at(&dome, Curv::Plan) > 0.0);
        assert!(at(&bowl, Curv::Profile) < 0.0 && at(&bowl, Curv::Plan) < 0.0);
        Ok(())
    }

    #[test]
    fn roughness_ruggedness_and_position() -> Result<()> {
        let mut g = surface(7, 7, |_, _| 10.0)?;
        g.data[3 * 7 + 3] = 18.0;
        assert_eq!(mid(&roughness(&g, 1.0, Edges::Center)), 8.0);
        assert_eq!(mid(&ruggedness(&g, 1.0, Edges::Center)), 8.0);
        let p = position(&g, 1.0, 2.0)?;
        assert!((mid(&p) - 8.0).abs() < 1e-4);
        // un vecino de la cima la ve en su media
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
        assert!(mid(&slope(&g, 1.0, false, Edges::NoData)).is_nan());
        // por defecto: las ocho celdas alrededor de un hueco tampoco tienen valor, igual que las
        // celdas del borde
        let s0 = slope(&g, 1.0, false, Edges::NoData);
        for (r, c) in [(1, 1), (1, 2), (1, 3), (2, 1), (2, 3), (3, 1), (3, 2), (3, 3)] {
            assert!(s0.data[r * 5 + c].is_nan(), "({c},{r})");
        }
        assert!(s0.data[0].is_nan() && s0.data[4].is_nan() && s0.data[24].is_nan());
        // el relleno del centro les da un valor
        let sc = slope(&g, 1.0, false, Edges::Center);
        assert!(sc.data[2 * 5 + 1].is_finite() && sc.data[0].is_finite());
        // repetir el borde rellena el borde pero no las celdas alrededor de un hueco
        let sr = slope(&g, 1.0, false, Edges::RepeatEdge);
        assert!(sr.data[0].is_finite() && sr.data[2 * 5 + 1].is_nan());
        Ok(())
    }

    #[test]
    fn nodata_default_leaves_the_interior_of_a_full_raster_unchanged() -> Result<()> {
        let g = surface(9, 9, |x, y| 0.5 * x + 0.25 * y)?;
        let a = slope(&g, 1.0, true, Edges::NoData);
        let b = slope(&g, 1.0, true, Edges::Center);
        for r in 1..8 {
            for c in 1..8 {
                assert_eq!(a.data[r * 9 + c], b.data[r * 9 + c]);
            }
        }
        // un plano conserva su pendiente en el borde solo cuando el borde no se repite
        assert!(a.data[0].is_nan());
        Ok(())
    }

    #[test]
    fn degrees_are_refused() -> Result<()> {
        let g = Grid::new(0.0, 0.0, 0.00001, 4, 4)?;
        let job = TerrainJob::new("a".into(), "b".into(), Product::Slope);
        assert!(compute(&g, &job).is_err());
        Ok(())
    }

    /// Un ráster trabajado en franjas da, celda a celda, lo que da el ráster entero.
    #[test]
    fn strips_give_the_same_cells_as_the_whole_raster() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_strips_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // una superficie con colinas y un hueco, de 300 x 200 celdas: varias franjas de 64 filas
        let mut g = Grid::new(1000.0, 5000.0, 2.0, 300, 200)?;
        for r in 0..g.rows {
            for c in 0..g.cols {
                let (x, y) = (c as f32, r as f32);
                g.data[r * g.cols + c] = 100.0 + 8.0 * (x / 17.0).sin() + 6.0 * (y / 11.0).cos() + x * 0.05;
            }
        }
        for r in 90..96 {
            for c in 140..150 {
                g.data[r * g.cols + c] = f32::NAN;
            }
        }
        let input = dir.join("dem.tif");
        g.write_tiff(&input, Some(25830))?;
        let pyr = crate::pyramid::Pyramid::open(&input)?;
        for product in [
            Product::Slope,
            Product::Aspect,
            Product::Hillshade,
            Product::Curvature,
            Product::Roughness,
            Product::Position,
        ] {
            let mut job = TerrainJob::new(input.clone(), dir.join("out.tif"), product);
            job.radius = 6.0;
            let whole = compute(&Grid::read(&input)?, &job)?;
            let margin = halo(&job, 2.0).unwrap();
            run_streamed(&job, &pyr, margin)?;
            let strips = Grid::read(&job.output)?;
            assert_eq!((strips.cols, strips.rows), (whole.cols, whole.rows));
            for (a, b) in whole.data.iter().zip(&strips.data) {
                assert!(
                    (a.is_nan() && b.is_nan()) || a == b,
                    "{}: {a} {b}",
                    product.name()
                );
            }
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
