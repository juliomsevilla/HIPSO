//! Herramientas de ráster: una imagen multibanda hecha a partir de rásteres (cada banda de entrada
//! pasa a ser una banda de salida) y un mosaico de rásteres que dice qué hacer donde se solapan los
//! píxeles (primero, último, media, mínimo, máximo, mediana, suma o número de capas). Ambas leen
//! las entradas por ventana y escriben la salida como un GeoTIFF optimizado para la nube (teselas y
//! vistas reducidas, véase [`crate::tiled`]), de modo que la memoria sigue a un bloque de filas,
//! nunca a las imágenes.
//!
//! Las entradas pueden estar en distintos sistemas de coordenadas (las de las demás se llevan al de
//! la primera) y tener celdas distintas: se remuestrean a la de la salida.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::pyramid::Pyramid;
use crate::raster::Image;
use crate::{Error, Result};

/// Qué hacer donde se solapan píxeles de varias entradas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlap {
    /// La primera entrada que tiene datos (el orden de la lista).
    First,
    /// La última entrada que tiene datos.
    Last,
    Mean,
    Min,
    Max,
    Median,
    Sum,
    /// Cuántas entradas tienen datos en el píxel.
    Count,
}

impl Overlap {
    pub const ALL: [Overlap; 8] = [
        Overlap::First,
        Overlap::Last,
        Overlap::Mean,
        Overlap::Min,
        Overlap::Max,
        Overlap::Median,
        Overlap::Sum,
        Overlap::Count,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Overlap::First => "Primero",
            Overlap::Last => "Último",
            Overlap::Mean => "Media",
            Overlap::Min => "Mínimo",
            Overlap::Max => "Máximo",
            Overlap::Median => "Mediana",
            Overlap::Sum => "Suma",
            Overlap::Count => "Número de capas",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Overlap::First => "primero",
            Overlap::Last => "ultimo",
            Overlap::Mean => "media",
            Overlap::Min => "min",
            Overlap::Max => "max",
            Overlap::Median => "mediana",
            Overlap::Sum => "suma",
            Overlap::Count => "numcapas",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resample {
    Nearest,
    Bilinear,
    /// La media de los píxeles de origen bajo el píxel de salida (para reducir una imagen fina).
    Average,
    /// Convolución cúbica sobre los 4x4 píxeles más cercanos (Keys, a = -0,5); bilineal donde uno
    /// de ellos está vacío. Suave, puede sobrepasar ligeramente: no para clases.
    Cubic,
    /// El valor más frecuente bajo el píxel de salida (en los empates, el menor): para clases al
    /// reducir una imagen fina.
    Mode,
}

/// El área que cubre la salida.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    Union,
    Intersection,
    /// La de la primera entrada.
    First,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dtype {
    U8,
    U16,
    F32,
}

/// Un ráster de entrada y las bandas que se toman de él (vacío: todas, numeradas desde 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub path: PathBuf,
    pub bands: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct RasterJob {
    pub inputs: Vec<Source>,
    pub output: PathBuf,
    /// Celda de salida; la de la entrada más fina cuando `None`.
    pub cell: Option<f64>,
    pub extent: Extent,
    pub overlap: Overlap,
    pub resample: Resample,
    /// Tipo de muestra de la salida; el de la primera entrada cuando `None`.
    pub dtype: Option<Dtype>,
    /// Un valor de las entradas que significa «sin datos» (además de lo que declaren sus archivos).
    pub src_nodata: Option<f32>,
    /// Lo que contienen los píxeles de salida vacíos.
    pub nodata: f32,
    /// Escribe BigTIFF aunque la imagen sea pequeña (se usa por encima de 4 GB de todos modos).
    pub bigtiff: bool,
    /// Cómo se comprimen las teselas; `None`: JPEG para fotografías de 8 bits y tres bandas,
    /// deflate en los demás casos.
    pub compression: Option<crate::tiled::Compression>,
}

impl RasterJob {
    pub fn new(inputs: Vec<Source>, output: PathBuf) -> Self {
        Self {
            inputs,
            output,
            cell: None,
            extent: Extent::Union,
            overlap: Overlap::First,
            resample: Resample::Nearest,
            dtype: None,
            src_nodata: None,
            nodata: 0.0,
            bigtiff: false,
            compression: None,
        }
    }
}

/// Máximo de bandas de una salida.
pub const MAX_BANDS: usize = 64;

/// Rejilla de salida.
struct Frame {
    x0: f64,
    top: f64,
    cell: f64,
    cols: usize,
    rows: usize,
}

struct Opened {
    pyr: Pyramid,
    /// 0-based bands taken.
    bands: Vec<usize>,
    /// La copia llevada al sistema de la primera entrada, borrada cuando termina el trabajo.
    temp: Option<PathBuf>,
}

impl Drop for Opened {
    fn drop(&mut self) {
        if let Some(t) = &self.temp {
            let _ = std::fs::remove_file(t);
            if let Some(dir) = t.parent() {
                let _ = std::fs::remove_dir(dir);
            }
        }
    }
}

fn open_all(job: &RasterJob) -> Result<Vec<Opened>> {
    if job.inputs.is_empty() {
        return Err(Error::Param("no hay ráster de entrada".into()));
    }
    let mut out: Vec<Opened> = Vec::new();
    for (n, s) in job.inputs.iter().enumerate() {
        let mut pyr = Pyramid::open(&s.path)?;
        let mut temp = None;
        // una entrada en otro sistema se lleva al de la primera
        if let (Some(first), Some(own)) = (out.first().and_then(|o| o.pyr.epsg), pyr.epsg)
            && first != own
        {
            static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "hipso_warp_{}_{}",
                std::process::id(),
                RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir)?;
            let tmp = dir.join(format!("{n}.tif"));
            let mut w = WarpJob::new(s.path.clone(), tmp.clone(), Target::Epsg { code: first, cell: None });
            w.resample = Some(job.resample);
            w.dtype = Some(Dtype::F32);
            w.nodata = f32::NAN;
            warp(&w)?;
            pyr = Pyramid::open(&tmp)?;
            temp = Some(tmp);
        }
        let bands: Vec<usize> = if s.bands.is_empty() {
            (0..pyr.bands).collect()
        } else {
            s.bands.iter().map(|b| b.saturating_sub(1)).collect()
        };
        if let Some(b) = bands.iter().find(|b| **b >= pyr.bands) {
            return Err(Error::Param(format!(
                "{} tiene {} banda(s): no existe la banda {}",
                s.path.display(),
                pyr.bands,
                b + 1
            )));
        }
        out.push(Opened { pyr, bands, temp });
    }
    Ok(out)
}

fn frame_of(job: &RasterJob, ins: &[Opened]) -> Result<Frame> {
    let cell = job
        .cell
        .unwrap_or_else(|| ins.iter().map(|o| o.pyr.cell).fold(f64::INFINITY, f64::min));
    if !(cell > 0.0 && cell.is_finite()) {
        return Err(Error::Param("la celda debe ser positiva".into()));
    }
    let ext: Vec<[f64; 4]> = ins.iter().map(|o| o.pyr.extent()).collect();
    let b = match job.extent {
        Extent::First => ext[0],
        Extent::Union => ext.iter().fold(ext[0], |a, e| {
            [
                a[0].min(e[0]),
                a[1].min(e[1]),
                a[2].max(e[2]),
                a[3].max(e[3]),
            ]
        }),
        Extent::Intersection => ext.iter().fold(ext[0], |a, e| {
            [
                a[0].max(e[0]),
                a[1].max(e[1]),
                a[2].min(e[2]),
                a[3].min(e[3]),
            ]
        }),
    };
    if b[2] <= b[0] || b[3] <= b[1] {
        return Err(Error::Param("los rásteres no se solapan".into()));
    }
    let (cols, rows) = (
        ((b[2] - b[0]) / cell).round().max(1.0) as usize,
        ((b[3] - b[1]) / cell).round().max(1.0) as usize,
    );
    if (cols as u64) * (rows as u64) > (1 << 36) {
        return Err(Error::Param("el resultado sería demasiado grande".into()));
    }
    Ok(Frame {
        x0: b[0],
        top: b[3],
        cell,
        cols,
        rows,
    })
}

/// El valor de la banda `b` de `img` para el píxel de salida centrado en `(x, y)` con lado `cell`.
fn sample(img: &Image, b: usize, x: f64, y: f64, cell: f64, how: Resample) -> f32 {
    let top = img.y0 + img.rows as f64 * img.cell;
    let (fx, fy) = ((x - img.x0) / img.cell, (top - y) / img.cell);
    // misma celda y el centro del píxel sobre el centro de uno de origen (el caso habitual cuando
    // se unen o reescriben hojas o franjas de una misma rejilla): el propio valor de origen, sin
    // interpolación
    if (img.cell - cell).abs() <= cell * 1e-9
        && (fx - fx.floor() - 0.5).abs() < 1e-6
        && (fy - fy.floor() - 0.5).abs() < 1e-6
    {
        let (c, r) = (fx.floor(), fy.floor());
        return if c < 0.0 || r < 0.0 || c >= img.cols as f64 || r >= img.rows as f64 {
            f32::NAN
        } else {
            img.bands[b][r as usize * img.cols + c as usize]
        };
    }
    let at = |c: i64, r: i64| -> f32 {
        if c < 0 || r < 0 || c >= img.cols as i64 || r >= img.rows as i64 {
            f32::NAN
        } else {
            img.bands[b][r as usize * img.cols + c as usize]
        }
    };
    match how {
        Resample::Nearest => at(fx.floor() as i64, fy.floor() as i64),
        Resample::Bilinear => {
            let (gx, gy) = (fx - 0.5, fy - 0.5);
            let (c0, r0) = (gx.floor() as i64, gy.floor() as i64);
            let (tx, ty) = ((gx - gx.floor()) as f32, (gy - gy.floor()) as f32);
            let mut acc = 0.0;
            let mut wsum = 0.0;
            for (dc, dr, w) in [
                (0, 0, (1.0 - tx) * (1.0 - ty)),
                (1, 0, tx * (1.0 - ty)),
                (0, 1, (1.0 - tx) * ty),
                (1, 1, tx * ty),
            ] {
                let v = at(c0 + dc, r0 + dr);
                if v.is_finite() && w > 0.0 {
                    acc += v * w;
                    wsum += w;
                }
            }
            if wsum > 0.0 { acc / wsum } else { f32::NAN }
        }
        Resample::Cubic => {
            let (gx, gy) = (fx - 0.5, fy - 0.5);
            let (c0, r0) = (gx.floor() as i64, gy.floor() as i64);
            let (tx, ty) = (gx - gx.floor(), gy - gy.floor());
            let kernel = |t: f64| -> [f64; 4] {
                // Los pesos de la convolución cúbica de Keys para los píxeles en -1, 0, 1, 2
                let w = |x: f64| {
                    let x = x.abs();
                    if x <= 1.0 {
                        1.5 * x.powi(3) - 2.5 * x * x + 1.0
                    } else if x < 2.0 {
                        -0.5 * x.powi(3) + 2.5 * x * x - 4.0 * x + 2.0
                    } else {
                        0.0
                    }
                };
                [w(1.0 + t), w(t), w(1.0 - t), w(2.0 - t)]
            };
            let (wx, wy) = (kernel(tx), kernel(ty));
            let mut acc = 0.0f64;
            for (j, wj) in wy.iter().enumerate() {
                for (i, wi) in wx.iter().enumerate() {
                    let v = at(c0 - 1 + i as i64, r0 - 1 + j as i64);
                    if !v.is_finite() {
                        return sample(img, b, x, y, cell, Resample::Bilinear);
                    }
                    acc += f64::from(v) * wi * wj;
                }
            }
            acc as f32
        }
        Resample::Mode => {
            let half = cell / img.cell / 2.0;
            if half <= 0.5 {
                return at(fx.floor() as i64, fy.floor() as i64);
            }
            let (c0, c1) = ((fx - half).round() as i64, (fx + half).round() as i64);
            let (r0, r1) = ((fy - half).round() as i64, (fy + half).round() as i64);
            let mut vals: Vec<f32> = Vec::new();
            for r in r0..r1.max(r0 + 1) {
                for c in c0..c1.max(c0 + 1) {
                    let v = at(c, r);
                    if v.is_finite() {
                        vals.push(v);
                    }
                }
            }
            vals.sort_by(f32::total_cmp);
            let (mut best, mut best_n, mut i) = (f32::NAN, 0usize, 0usize);
            while i < vals.len() {
                let mut j = i;
                while j < vals.len() && vals[j] == vals[i] {
                    j += 1;
                }
                if j - i > best_n {
                    (best, best_n) = (vals[i], j - i);
                }
                i = j;
            }
            best
        }
        Resample::Average => {
            let half = cell / img.cell / 2.0;
            if half <= 0.5 {
                return at(fx.floor() as i64, fy.floor() as i64);
            }
            let (c0, c1) = ((fx - half).round() as i64, (fx + half).round() as i64);
            let (r0, r1) = ((fy - half).round() as i64, (fy + half).round() as i64);
            let (mut s, mut n) = (0.0f64, 0u32);
            for r in r0..r1.max(r0 + 1) {
                for c in c0..c1.max(c0 + 1) {
                    let v = at(c, r);
                    if v.is_finite() {
                        s += f64::from(v);
                        n += 1;
                    }
                }
            }
            if n > 0 {
                (s / f64::from(n)) as f32
            } else {
                f32::NAN
            }
        }
    }
}

/// Lee la parte de la entrada `o` que cubre las filas `r0..r1` del marco; `None` cuando queda
/// fuera.
fn read_block(
    o: &Opened,
    f: &Frame,
    r0: usize,
    r1: usize,
    job: &RasterJob,
) -> Result<Option<Image>> {
    let win = [
        f.x0,
        f.top - r1 as f64 * f.cell,
        f.x0 + f.cols as f64 * f.cell,
        f.top - r0 as f64 * f.cell,
    ];
    let e = o.pyr.extent();
    // un margen de dos píxeles de origen para los núcleos bilineal y de media
    let m = o.pyr.cell * 2.0;
    if win[2] <= e[0] || win[0] >= e[2] || win[3] <= e[1] || win[1] >= e[3] {
        return Ok(None);
    }
    let win = [win[0] - m, win[1] - m, win[2] + m, win[3] + m];
    let Ok(mut img) = o.pyr.read(0, win) else {
        return Ok(None);
    };
    if let Some(nd) = job.src_nodata {
        for b in &mut img.bands {
            for v in b.iter_mut() {
                if *v == nd {
                    *v = f32::NAN;
                }
            }
        }
    }
    Ok(Some(img))
}

/// Filas por bloque para que un bloque contenga unos 4 millones de muestras.
fn block_rows(f: &Frame, nb: usize, layers: usize) -> usize {
    (4_000_000 / (f.cols * nb * layers.max(1))).clamp(8, 2048)
}

// -------------------------------------------------------------------------------- writing

/// Escribe el marco como GeoTIFF optimizado para la nube (teselas, vistas reducidas), pidiendo a
/// `rows_of(r0, r1)` las muestras entrelazadas de esas filas, `step` filas cada vez. BigTIFF cuando
/// la imagen podría pasar de 4 GB.
#[allow(clippy::too_many_arguments)]
fn write_frame(
    job: &RasterJob,
    f: &Frame,
    epsg: Option<u16>,
    dtype: Dtype,
    nb: usize,
    step: usize,
    rows_of: impl FnMut(usize, usize) -> Result<Vec<f32>>,
) -> Result<()> {
    let compression = job
        .compression
        .unwrap_or_else(|| crate::tiled::auto_compression(dtype, nb));
    let spec = crate::tiled::Spec {
        cols: f.cols,
        rows: f.rows,
        bands: nb,
        dtype,
        cell: f.cell,
        x0: f.x0,
        top: f.top,
        epsg,
        nodata: job.nodata,
        compression,
        // las clases y los recuentos no se promedian en las vistas reducidas
        nearest: job.overlap == Overlap::Count
            || ((job.resample == Resample::Nearest || job.resample == Resample::Mode) && nb == 1 && dtype != Dtype::F32),
        bigtiff: job.bigtiff,
    };
    crate::tiled::write(&job.output, spec, step, rows_of)
}

fn dtype_of(job: &RasterJob, first: &Pyramid) -> Dtype {
    job.dtype.unwrap_or(match first.bits {
        0..=8 => Dtype::U8,
        9..=16 => Dtype::U16,
        _ => Dtype::F32,
    })
}

// -------------------------------------------------------------------------------- stack

/// Una imagen multibanda: las bandas de cada entrada, en orden, una tras otra.
pub fn stack(job: &RasterJob) -> Result<String> {
    let ins = open_all(job)?;
    let nb: usize = ins.iter().map(|o| o.bands.len()).sum();
    if nb == 0 || nb > MAX_BANDS {
        return Err(Error::Param(format!(
            "{nb} bandas: un resultado tiene de 1 a {MAX_BANDS}"
        )));
    }
    let f = frame_of(job, &ins)?;
    let dtype = dtype_of(job, &ins[0].pyr);
    let step = block_rows(&f, nb, 2);
    write_frame(job, &f, ins[0].pyr.epsg, dtype, nb, step, |r0, r1| {
        let n = (r1 - r0) * f.cols;
        let mut out = vec![f32::NAN; n * nb];
        let mut base = 0;
        for o in &ins {
            if let Some(img) = read_block(o, &f, r0, r1, job)? {
                for (k, &b) in o.bands.iter().enumerate() {
                    for i in 0..n {
                        let (r, c) = (r0 + i / f.cols, i % f.cols);
                        let (x, y) = (
                            f.x0 + (c as f64 + 0.5) * f.cell,
                            f.top - (r as f64 + 0.5) * f.cell,
                        );
                        out[i * nb + base + k] = sample(&img, b, x, y, f.cell, job.resample);
                    }
                }
            }
            base += o.bands.len();
        }
        Ok(out)
    })?;
    Ok(format!(
        "{} bandas de {} rásteres, {} x {} píxeles de {}: {}",
        nb,
        ins.len(),
        f.cols,
        f.rows,
        crate::paths::num(f.cell),
        job.output.display()
    ))
}

// -------------------------------------------------------------------------------- warp

/// Dónde pone una deformación un ráster.
#[derive(Clone, Debug)]
pub enum Target {
    /// Las celdas de este ráster: su sistema, origen, celda y tamaño.
    Reference(PathBuf),
    /// Este sistema (EPSG). La celda es la de origen cuando las unidades coinciden (véase
    /// [`crate::reproject::grid`]) salvo que `cell` diga otra; la extensión es la de origen.
    Epsg { code: u16, cell: Option<f64> },
}

/// Un ráster llevado a otro sistema, extensión o celda: la única operación que hay detrás de
/// «reproyectar», «igualar la extensión de una referencia» y «remuestrear».
#[derive(Clone, Debug)]
pub struct WarpJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub target: Target,
    /// El sistema de la entrada cuando el archivo no lo declara.
    pub from: Option<u16>,
    /// `None` elige según los datos y los tamaños de celda (véase [`Resample::describe`]).
    pub resample: Option<Resample>,
    /// Tipo de muestra de la salida; el de la entrada cuando `None`.
    pub dtype: Option<Dtype>,
    /// Lo que contienen las celdas de salida sin valor.
    pub nodata: f32,
    /// Un valor de la entrada que significa «sin datos» (además de lo que declare su archivo).
    pub src_nodata: Option<f32>,
    pub compression: Option<crate::tiled::Compression>,
}

impl WarpJob {
    pub fn new(input: PathBuf, output: PathBuf, target: Target) -> Self {
        Self {
            input,
            output,
            target,
            from: None,
            resample: None,
            dtype: None,
            nodata: -9999.0,
            src_nodata: None,
            compression: None,
        }
    }
}

impl Resample {
    pub const ALL: [Resample; 5] = [
        Resample::Nearest,
        Resample::Bilinear,
        Resample::Cubic,
        Resample::Average,
        Resample::Mode,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Resample::Nearest => "Vecino más cercano",
            Resample::Bilinear => "Bilineal",
            Resample::Cubic => "Cúbico",
            Resample::Average => "Media",
            Resample::Mode => "Moda",
        }
    }

    /// Cuándo usarlo, para la ayuda de las herramientas.
    pub fn describe(self) -> &'static str {
        match self {
            Resample::Nearest => "Copia el valor de la celda más cercana, sin inventar valores: es el que hay que usar con clases, identificadores o máscaras. Es el más rápido; en imágenes continuas deja bordes escalonados.",
            Resample::Bilinear => "Pondera las 4 celdas vecinas: suave y sin sobrepasar los valores originales. Es lo habitual para modelos de elevación, temperatura o índices cuando la celda de salida es igual o mayor que la de entrada.",
            Resample::Cubic => "Ajusta una curva por las 16 celdas vecinas (convolución cúbica): conserva mejor el detalle que el bilineal al ampliar o girar una imagen, pero puede pasarse un poco de los valores máximos y mínimos. Para fotografías e imágenes; no para clases.",
            Resample::Average => "Promedia todas las celdas de entrada bajo la de salida: es el que hay que usar al reducir la resolución (celdas más grandes) de datos continuos, porque no pierde información ni crea aliasing. Con celdas más pequeñas equivale al vecino más cercano.",
            Resample::Mode => "El valor más frecuente bajo la celda de salida: es el «media» de las clases. Úsalo al reducir la resolución de un ráster de clases o de usos del suelo.",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Resample::Nearest => "nearest",
            Resample::Bilinear => "bilinear",
            Resample::Cubic => "cubic",
            Resample::Average => "average",
            Resample::Mode => "mode",
        }
    }
}

/// Un origen cuyos valores son todos números enteros (clases, identificadores) en una muestra de la
/// imagen.
fn looks_categorical(pyr: &Pyramid) -> bool {
    // varias bandas son una imagen (fotografía, composición), no clases
    if pyr.bands != 1 {
        return false;
    }
    let Ok((sample, _)) = pyr.overview(2_000_000) else {
        return false;
    };
    let nd = pyr.nodata();
    let mut seen = 0usize;
    for b in sample.bands.iter().take(1) {
        for v in b.iter().step_by((b.len() / 50_000).max(1)) {
            if !v.is_finite() || nd.is_some_and(|n| *v == n) {
                continue;
            }
            seen += 1;
            if v.fract() != 0.0 {
                return false;
            }
        }
    }
    seen > 0
}

/// El remuestreo que se usa cuando no se pide ninguno: las clases conservan sus valores (la celda
/// más cercana o la más frecuente al reducir), el resto se promedia al reducir y es bilineal en
/// caso contrario.
fn auto_resample(pyr: &Pyramid, reducing: bool) -> Resample {
    if looks_categorical(pyr) {
        if reducing { Resample::Mode } else { Resample::Nearest }
    } else if reducing {
        Resample::Average
    } else {
        Resample::Bilinear
    }
}

/// Traslada los píxeles de un bloque del marco de salida al sistema de origen sin transformar cada
/// uno: la transformación exacta en nodos cada `k` píxeles, interpolada entre ellos, con `k`
/// reducido a la mitad hasta que el error en el centro de cada celda de nodos es inferior a una
/// décima de píxel de origen (hasta `k = 1`, que es exacto).
struct NodeMap {
    cols_at: Vec<usize>,
    rows_at: Vec<usize>,
    /// `[x, y]` del nodo (fila j, columna i) en el sistema de origen; NaN donde no se puede mover.
    xy: Vec<[f64; 2]>,
    k: usize,
}

impl NodeMap {
    fn new(
        inv: &crate::reproject::Transformer,
        f: &Frame,
        r0: usize,
        r1: usize,
        src_cell: f64,
    ) -> Self {
        let exact = |c: f64, r: f64| -> [f64; 2] {
            let (x, y) = (f.x0 + (c + 0.5) * f.cell, f.top - (r + 0.5) * f.cell);
            inv.transform(x, y).map_or([f64::NAN; 2], |(a, b)| [a, b])
        };
        let nodes = |n: usize, k: usize| -> Vec<usize> {
            let mut v: Vec<usize> = (0..n.saturating_sub(1)).step_by(k).collect();
            v.push(n - 1);
            v.dedup();
            v
        };
        let rows = r1 - r0;
        let mut k = 16usize;
        loop {
            let cols_at = nodes(f.cols, k);
            let rows_at = nodes(rows, k);
            let xy: Vec<[f64; 2]> = rows_at
                .par_iter()
                .flat_map_iter(|&r| {
                    cols_at
                        .iter()
                        .map(move |&c| (c, r))
                        .map(|(c, r)| exact(c as f64, (r0 + r) as f64))
                })
                .collect();
            let m = Self { cols_at, rows_at, xy, k };
            if k == 1 || m.error_ok(&exact, r0, src_cell * 0.1) {
                return m;
            }
            k /= 2;
        }
    }

    fn error_ok(&self, exact: &impl Fn(f64, f64) -> [f64; 2], r0: usize, tol: f64) -> bool {
        let w = self.cols_at.len();
        let stride = (((self.cols_at.len() - 1) * (self.rows_at.len() - 1)) / 4096).max(1);
        (0..self.rows_at.len().saturating_sub(1))
            .flat_map(|j| (0..w.saturating_sub(1)).map(move |i| (j, i)))
            .step_by(stride)
            .all(|(j, i)| {
                let (c, r) = (
                    (self.cols_at[i] + self.cols_at[i + 1]) as f64 / 2.0,
                    (self.rows_at[j] + self.rows_at[j + 1]) as f64 / 2.0,
                );
                let want = exact(c, r0 as f64 + r);
                let n = |a: usize, b: usize| self.xy[b * w + a];
                let have = [
                    (n(i, j)[0] + n(i + 1, j)[0] + n(i, j + 1)[0] + n(i + 1, j + 1)[0]) / 4.0,
                    (n(i, j)[1] + n(i + 1, j)[1] + n(i, j + 1)[1] + n(i + 1, j + 1)[1]) / 4.0,
                ];
                // una celda que la transformación no puede cruzar se deja a la pasada exacta
                if want[0].is_nan() != have[0].is_nan() {
                    return self.k <= 1;
                }
                want[0].is_nan() || (want[0] - have[0]).hypot(want[1] - have[1]) <= tol
            })
    }

    /// La caja de origen de los nodos, `None` cuando ninguno se pudo mover.
    fn bounds(&self) -> Option<[f64; 4]> {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in self.xy.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
        (b[0] <= b[2] && b[1] <= b[3]).then_some(b)
    }

    /// Rellena `line` con las coordenadas de origen de los píxeles de la fila `r` (empezando en 0
    /// en el bloque).
    fn row(&self, r: usize, line: &mut [[f64; 2]]) {
        let w = self.cols_at.len();
        let j = (r / self.k).min(self.rows_at.len().saturating_sub(2));
        let (j1, tj) = if self.rows_at.len() == 1 {
            (j, 0.0)
        } else {
            let (a, b) = (self.rows_at[j] as f64, self.rows_at[j + 1] as f64);
            (j + 1, (r as f64 - a) / (b - a))
        };
        let mut i = 0usize;
        for (c, out) in line.iter_mut().enumerate() {
            while i + 2 < w && self.cols_at[i + 1] < c {
                i += 1;
            }
            let (i1, ti) = if w == 1 {
                (i, 0.0)
            } else {
                let (a, b) = (self.cols_at[i] as f64, self.cols_at[i + 1] as f64);
                (i + 1, (c as f64 - a) / (b - a))
            };
            let (p00, p10) = (self.xy[j * w + i], self.xy[j * w + i1]);
            let (p01, p11) = (self.xy[j1 * w + i], self.xy[j1 * w + i1]);
            let l = |k: usize| {
                let top = p00[k] + (p10[k] - p00[k]) * ti;
                let bot = p01[k] + (p11[k] - p01[k]) * ti;
                top + (bot - top) * tj
            };
            *out = [l(0), l(1)];
        }
    }
}

/// Lleva un ráster (todas sus bandas) a otro sistema, extensión o celda. La memoria sigue a un
/// bloque de filas; el origen se lee por ventana y el trabajo se reparte entre todos los núcleos.
/// Devuelve una línea para el informe.
pub fn warp(job: &WarpJob) -> Result<String> {
    let pyr = Pyramid::open(&job.input)?;
    let nb = pyr.bands;
    if nb == 0 || nb > MAX_BANDS {
        return Err(Error::Param(format!(
            "{nb} bandas: una salida admite de 1 a {MAX_BANDS}"
        )));
    }
    let from = job.from.or(pyr.epsg);
    let e = pyr.extent();
    let (src_cols, src_rows) = (pyr.levels[0].cols as usize, pyr.levels[0].rows as usize);
    // el marco de salida y su sistema
    let (f, out_epsg, note) = match &job.target {
        Target::Reference(path) => {
            let r = Pyramid::open(path)?;
            let note = if r.epsg.is_none() || from.is_none() {
                " (no se pudo comprobar el sistema de coordenadas: falta en uno de los archivos)"
            } else {
                ""
            };
            let out = r.epsg.or(from);
            let f = Frame {
                x0: r.x0,
                top: r.y_top,
                cell: r.cell,
                cols: r.levels[0].cols as usize,
                rows: r.levels[0].rows as usize,
            };
            (f, out, note)
        }
        Target::Epsg { code, cell } => {
            let from = from.ok_or_else(|| {
                Error::Param(format!(
                    "«{}» no declara su sistema de coordenadas: indica el de origen",
                    crate::paths::stem(&job.input)
                ))
            })?;
            let (b, c) = if from == *code {
                (e, pyr.cell)
            } else {
                crate::reproject::target_frame_of(
                    [e[0], e[1], pyr.cell],
                    src_cols,
                    src_rows,
                    from,
                    *code,
                )?
            };
            let c = cell.unwrap_or(c);
            if !(c > 0.0 && c.is_finite()) {
                return Err(Error::Param("la celda debe ser positiva".into()));
            }
            let f = if from == *code {
                Frame {
                    x0: b[0],
                    top: b[3],
                    cell: c,
                    cols: (((b[2] - b[0]) / c).ceil() as usize).max(1),
                    rows: (((b[3] - b[1]) / c).ceil() as usize).max(1),
                }
            } else {
                let (x0, y0, cols, rows) = crate::grid::Grid::covering_dims(b, c);
                Frame {
                    x0,
                    top: y0 + rows as f64 * c,
                    cell: c,
                    cols,
                    rows,
                }
            };
            (f, Some(*code), "")
        }
    };
    if (f.cols as u64) * (f.rows as u64) > (1 << 36) {
        return Err(Error::Param("el resultado sería demasiado grande".into()));
    }
    let inv = match (from, out_epsg) {
        (Some(a), Some(b)) if a != b => Some(crate::reproject::transformer(b, a)?),
        _ => None,
    };
    // celda de origen en las unidades de la salida, para saber si la imagen se reduce
    let src_in_out = match (&inv, from, out_epsg) {
        (Some(_), Some(a), Some(b)) => {
            let fwd = crate::reproject::transformer(a, b)?;
            let (cx, cy) = ((e[0] + e[2]) / 2.0, (e[1] + e[3]) / 2.0);
            match (fwd.transform(cx, cy), fwd.transform(cx + pyr.cell, cy)) {
                (Ok(p), Ok(q)) => (q.0 - p.0).hypot(q.1 - p.1),
                _ => pyr.cell,
            }
        }
        _ => pyr.cell,
    };
    let reducing = f.cell > src_in_out * 1.01;
    let how = job.resample.unwrap_or_else(|| auto_resample(&pyr, reducing));
    let mut sub = RasterJob::new(Vec::new(), job.output.clone());
    sub.src_nodata = job.src_nodata;
    sub.nodata = job.nodata;
    sub.dtype = job.dtype;
    sub.compression = job.compression;
    sub.resample = how;
    let dtype = job.dtype.unwrap_or(match pyr.bits {
        0..=8 => Dtype::U8,
        9..=16 => Dtype::U16,
        _ => Dtype::F32,
    });
    // píxeles de origen por píxel de salida, para el tamaño de los bloques
    let scale = (f.cell / src_in_out).max(1.0);
    // unos 24 millones de muestras por bloque, y filas de tesela enteras del origen donde se pueda
    // (una tesela se decodifica una vez por bloque que la toca)
    let step = ((24_000_000.0 / (f.cols * nb) as f64 / (scale * scale)) as usize).clamp(4, 4096);
    let tile_rows = crate::tiled::TILE as f64 * src_in_out / f.cell;
    let step = if tile_rows >= 1.0 && step as f64 >= 2.0 * tile_rows {
        ((step as f64 / tile_rows).floor() * tile_rows) as usize
    } else {
        step
    };
    let declared = pyr.nodata();
    let src_nd = job.src_nodata;
    let bad = move |v: f32| !v.is_finite() || declared == Some(v) || src_nd == Some(v);
    let margin = |fp: f64| pyr.cell * 3.0 + fp;
    let mut filled = 0u64;
    crate::progress::begin(f.rows as u64);
    write_frame(&sub, &f, out_epsg, dtype, nb, step, |r0, r1| {
        crate::progress::advance((r1 - r0) as u64);
        let n = r1 - r0;
        let mut out = vec![f32::NAN; n * f.cols * nb];
        // la parte del origen bajo el bloque y dónde cae cada píxel de ella
        let (win, map, fp) = match &inv {
            None => {
                let m = pyr.cell * 3.0 + f.cell;
                (
                    [
                        f.x0 - m,
                        f.top - r1 as f64 * f.cell - m,
                        f.x0 + f.cols as f64 * f.cell + m,
                        f.top - r0 as f64 * f.cell + m,
                    ],
                    None,
                    f.cell,
                )
            }
            Some(inv) => {
                let map = NodeMap::new(inv, &f, r0, r1, pyr.cell);
                let Some(b) = map.bounds() else {
                    return Ok(out);
                };
                // la celda de salida en unidades de origen, en el centro del bloque
                let (cx, cy) = (
                    f.x0 + f.cols as f64 / 2.0 * f.cell,
                    f.top - (r0 + r1) as f64 / 2.0 * f.cell,
                );
                let fp = match (
                    inv.transform(cx, cy),
                    inv.transform(cx + f.cell, cy),
                    inv.transform(cx, cy + f.cell),
                ) {
                    (Ok(a), Ok(b), Ok(c)) => {
                        let (sx, sy) = ((b.0 - a.0).hypot(b.1 - a.1), (c.0 - a.0).hypot(c.1 - a.1));
                        (sx * sy).sqrt()
                    }
                    _ => pyr.cell,
                };
                let m = margin(fp);
                ([b[0] - m, b[1] - m, b[2] + m, b[3] + m], Some(map), fp)
            }
        };
        if !(win[0] < e[2] && win[2] > e[0] && win[1] < e[3] && win[3] > e[1]) {
            return Ok(out);
        }
        let Ok(mut img) = pyr.read(0, win) else {
            return Ok(out);
        };
        for b in &mut img.bands {
            for v in b.iter_mut() {
                if bad(*v) {
                    *v = f32::NAN;
                }
            }
        }
        let cols = f.cols;
        let got: u64 = out
            .par_chunks_mut(cols * nb)
            .enumerate()
            .map(|(r, line)| {
                let mut xy = vec![[0.0f64; 2]; if map.is_some() { cols } else { 0 }];
                if let Some(m) = &map {
                    m.row(r, &mut xy);
                }
                let y_plain = f.top - ((r0 + r) as f64 + 0.5) * f.cell;
                let mut cnt = 0u64;
                for c in 0..cols {
                    let (x, y) = match &map {
                        Some(_) => (xy[c][0], xy[c][1]),
                        None => (f.x0 + (c as f64 + 0.5) * f.cell, y_plain),
                    };
                    if !(x.is_finite() && y.is_finite()) {
                        continue;
                    }
                    for b in 0..nb {
                        let v = sample(&img, b, x, y, fp, how);
                        line[c * nb + b] = v;
                        cnt += u64::from(v.is_finite());
                    }
                }
                cnt
            })
            .sum();
        filled += got;
        Ok(out)
    })?;
    let total = (f.cols as u64) * (f.rows as u64) * nb as u64;
    let moved = match (from, out_epsg, inv.is_some()) {
        (Some(a), Some(b), true) => format!(", de EPSG:{a} a EPSG:{b}"),
        _ => String::new(),
    };
    Ok(format!(
        "{}: celdas de {} ({}), {} x {}{moved}, {:.1} % con datos{note} → {}",
        crate::paths::stem(&job.input),
        crate::paths::num(f.cell),
        match how {
            Resample::Nearest => "vecino más cercano",
            Resample::Bilinear => "bilineal",
            Resample::Average => "media",
            Resample::Cubic => "cúbico",
            Resample::Mode => "moda",
        },
        f.cols,
        f.rows,
        100.0 * filled as f64 / total.max(1) as f64,
        job.output.display()
    ))
}

// -------------------------------------------------------------------------------- align

/// Pone rásteres sobre las celdas de otro: mismo sistema, origen, celda y tamaño, de modo que capa
/// a capa se puedan comparar y apilar. Un ráster en otro sistema se lleva al de la referencia.
#[derive(Clone, Debug)]
pub struct AlignJob {
    /// El ráster cuyas celdas toman todos los demás.
    pub reference: PathBuf,
    /// Cada ráster que alinear y adónde va su copia alineada.
    pub inputs: Vec<(PathBuf, PathBuf)>,
    /// Cómo se trasladan los valores; `None` elige según los datos y las celdas: las clases
    /// conservan sus valores (celda más cercana o la más frecuente al reducir), el resto se
    /// promedia al reducir y es bilineal en caso contrario.
    pub resample: Option<Resample>,
    /// Tipo de muestra de las copias; el de cada entrada cuando `None`.
    pub dtype: Option<Dtype>,
    /// Lo que contienen las celdas fuera de la entrada.
    pub nodata: f32,
    /// Un valor de las entradas que significa «sin datos» (además de lo que declaren sus archivos).
    pub src_nodata: Option<f32>,
    /// Cómo se comprimen las copias; `None`: como [`RasterJob::compression`].
    pub compression: Option<crate::tiled::Compression>,
}

/// Escribe cada ráster del trabajo sobre las celdas de la referencia.
pub fn align(job: &AlignJob) -> Result<String> {
    let mut lines = Vec::new();
    for (input, output) in &job.inputs {
        crate::check_cancel()?;
        let mut w = WarpJob::new(
            input.clone(),
            output.clone(),
            Target::Reference(job.reference.clone()),
        );
        w.resample = job.resample;
        w.dtype = job.dtype;
        w.nodata = job.nodata;
        w.src_nodata = job.src_nodata;
        w.compression = job.compression;
        lines.push(warp(&w)?);
    }
    Ok(lines.join("\n"))
}

// -------------------------------------------------------------------------------- mosaic

/// Un mosaico de rásteres con las mismas bandas, que resuelve los solapes como dice `job.overlap`.
pub fn mosaic(job: &RasterJob) -> Result<String> {
    let ins = open_all(job)?;
    let nb = ins[0].bands.len();
    if ins.iter().any(|o| o.bands.len() != nb) {
        return Err(Error::Param(
            "todos los rásteres de un mosaico necesitan el mismo número de bandas".into(),
        ));
    }
    if nb == 0 || nb > MAX_BANDS {
        return Err(Error::Param(format!(
            "{nb} bandas: se admiten de 1 a {MAX_BANDS}"
        )));
    }
    let f = frame_of(job, &ins)?;
    let dtype = if job.overlap == Overlap::Count {
        job.dtype.unwrap_or(Dtype::U16)
    } else {
        dtype_of(job, &ins[0].pyr)
    };
    let keep_layers = job.overlap == Overlap::Median;
    let step = block_rows(&f, nb, if keep_layers { ins.len() + 1 } else { 2 });
    let mut overlapped = 0u64;
    write_frame(job, &f, ins[0].pyr.epsg, dtype, nb, step, |r0, r1| {
        let n = (r1 - r0) * f.cols;
        // valor acumulado y número de capas con datos, por muestra
        let mut acc = vec![0f32; n * nb];
        let mut cnt = vec![0u16; n * nb];
        let mut layers: Vec<Vec<f32>> = Vec::new();
        for o in &ins {
            let Some(img) = read_block(o, &f, r0, r1, job)? else {
                continue;
            };
            let mut layer = if keep_layers {
                vec![f32::NAN; n * nb]
            } else {
                Vec::new()
            };
            for i in 0..n {
                let (r, c) = (r0 + i / f.cols, i % f.cols);
                let (x, y) = (
                    f.x0 + (c as f64 + 0.5) * f.cell,
                    f.top - (r as f64 + 0.5) * f.cell,
                );
                for (k, &b) in o.bands.iter().enumerate() {
                    let v = sample(&img, b, x, y, f.cell, job.resample);
                    if !v.is_finite() {
                        continue;
                    }
                    let s = i * nb + k;
                    if keep_layers {
                        layer[s] = v;
                    }
                    let first = cnt[s] == 0;
                    cnt[s] = cnt[s].saturating_add(1);
                    acc[s] = match job.overlap {
                        Overlap::First if !first => acc[s],
                        Overlap::Min if !first => acc[s].min(v),
                        Overlap::Max if !first => acc[s].max(v),
                        Overlap::Mean | Overlap::Sum if !first => acc[s] + v,
                        _ => v,
                    };
                }
            }
            if keep_layers {
                layers.push(layer);
            }
        }
        for s in 0..n * nb {
            if cnt[s] == 0 {
                acc[s] = f32::NAN;
                continue;
            }
            if cnt[s] > 1 {
                overlapped += 1;
            }
            match job.overlap {
                Overlap::Mean => acc[s] /= f32::from(cnt[s]),
                Overlap::Count => acc[s] = f32::from(cnt[s]),
                Overlap::Median => {
                    let mut v: Vec<f32> = layers
                        .iter()
                        .map(|l| l[s])
                        .filter(|x| x.is_finite())
                        .collect();
                    v.sort_unstable_by(f32::total_cmp);
                    let m = v.len() / 2;
                    acc[s] = if v.len() % 2 == 1 {
                        v[m]
                    } else {
                        (v[m - 1] + v[m]) / 2.0
                    };
                }
                _ => {}
            }
        }
        Ok(acc)
    })?;
    Ok(format!(
        "mosaico de {} rásteres ({}), {} x {} píxeles de {}, {} muestras solapadas: {}",
        ins.len(),
        job.overlap.name().to_lowercase(),
        f.cols,
        f.rows,
        crate::paths::num(f.cell),
        overlapped,
        job.output.display()
    ))
}

#[cfg(test)]
mod tests {
    fn ramp(cols: usize, rows: usize, f: impl Fn(f64, f64) -> f32) -> Image {
        let mut d = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                d.push(f(c as f64 + 0.5, (rows - r) as f64 - 0.5));
            }
        }
        Image { x0: 0.0, y0: 0.0, cell: 1.0, cols, rows, bands: vec![d] }
    }

    #[test]
    fn cubic_reproduces_a_plane_and_mode_picks_the_majority() {
        let img = ramp(10, 10, |x, y| (2.0 * x + 3.0 * y) as f32);
        let v = sample(&img, 0, 4.3, 5.8, 1.0, Resample::Cubic);
        assert!((f64::from(v) - (2.0 * 4.3 + 3.0 * 5.8)).abs() < 1e-4, "{v}");
        // un píxel de 4x4 celdas de origen: 10 celdas de clase 7, 6 de clase 3
        let mut img = ramp(8, 8, |_, _| 3.0);
        for r in 0..2 {
            for c in 0..4 {
                img.bands[0][r * 8 + c] = 7.0;
            }
        }
        for c in 0..2 {
            img.bands[0][2 * 8 + c] = 7.0;
        }
        assert_eq!(sample(&img, 0, 2.0, 6.0, 4.0, Resample::Mode), 7.0);
        // un hueco entre los 16 vecinos cúbicos recurre al bilineal, no a NaN
        let mut img = ramp(10, 10, |x, _| x as f32);
        img.bands[0][4 * 10 + 4] = f32::NAN;
        assert!(sample(&img, 0, 4.6, 5.4, 1.0, Resample::Cubic).is_finite());
    }

    use super::*;
    use crate::grid::Grid;

    fn tile(x0: f64, v: f32, cols: usize) -> Result<Grid> {
        let mut g = Grid::new(x0, 0.0, 1.0, cols, 2)?;
        g.data.fill(v);
        Ok(g)
    }

    fn run(how: Overlap) -> Result<Vec<f32>> {
        let dir =
            std::env::temp_dir().join(format!("rl-mosaic-{}-{}", std::process::id(), how.tag()));
        std::fs::create_dir_all(&dir)?;
        // a: x 0..4 = 2, b: x 2..6 = 6 y c: x 2..6 = 10 (ambos se solapan con a en x 2..4)
        let files = [
            ("a.tif", 0.0, 2.0),
            ("b.tif", 2.0, 6.0),
            ("c.tif", 2.0, 10.0),
        ];
        let mut srcs = Vec::new();
        for (n, x, v) in files {
            let p = dir.join(n);
            tile(x, v, 4)?.write_tiff(&p, Some(25830))?;
            srcs.push(Source {
                path: p,
                bands: vec![],
            });
        }
        let out = dir.join("m.tif");
        let mut job = RasterJob::new(srcs, out.clone());
        job.overlap = how;
        job.dtype = Some(Dtype::F32);
        job.nodata = -1.0;
        mosaic(&job)?;
        let g = Grid::read(&out)?;
        let v = g.data[..g.cols].to_vec();
        std::fs::remove_dir_all(&dir)?;
        Ok(v)
    }

    #[test]
    fn overlap_modes() -> Result<()> {
        assert_eq!(run(Overlap::First)?, [2.0, 2.0, 2.0, 2.0, 6.0, 6.0]);
        assert_eq!(run(Overlap::Last)?, [2.0, 2.0, 10.0, 10.0, 10.0, 10.0]);
        assert_eq!(run(Overlap::Max)?, [2.0, 2.0, 10.0, 10.0, 10.0, 10.0]);
        assert_eq!(run(Overlap::Min)?, [2.0, 2.0, 2.0, 2.0, 6.0, 6.0]);
        assert_eq!(run(Overlap::Mean)?, [2.0, 2.0, 6.0, 6.0, 8.0, 8.0]);
        assert_eq!(run(Overlap::Median)?, [2.0, 2.0, 6.0, 6.0, 8.0, 8.0]);
        assert_eq!(run(Overlap::Sum)?, [2.0, 2.0, 18.0, 18.0, 16.0, 16.0]);
        assert_eq!(run(Overlap::Count)?, [1.0, 1.0, 3.0, 3.0, 2.0, 2.0]);
        Ok(())
    }

    #[test]
    fn bigtiff_reads_back() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let p = dir.join("a.tif");
        tile(0.0, 7.0, 5)?.write_tiff(&p, Some(25830))?;
        let out = dir.join("b.tif");
        let mut job = RasterJob::new(
            vec![Source {
                path: p,
                bands: vec![],
            }],
            out.clone(),
        );
        job.bigtiff = true;
        job.dtype = Some(Dtype::U16);
        mosaic(&job)?;
        let img = crate::raster::read_image(&out)?;
        assert_eq!(img.bands[0][3], 7.0);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn align_puts_a_raster_on_the_reference_cells() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl-align-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // referencia: celdas de 1 m, x 100..106, y 200..204
        let mut r = Grid::new(100.0, 200.0, 1.0, 6, 4)?;
        r.data.fill(1.0);
        r.write_tiff(&dir.join("ref.tif"), Some(25830))?;
        // entrada: celdas de 0,5 m, desplazada un cuarto de metro, mayor que la referencia
        let mut g = Grid::new(99.0, 199.0, 0.5, 20, 14)?;
        for (i, v) in g.data.iter_mut().enumerate() {
            *v = if (i % 20) < 10 { 4.5 } else { 8.5 };
        }
        g.write_tiff(&dir.join("fine.tif"), Some(25830))?;
        // y una que solo cubre parte de la referencia
        let mut part = Grid::new(100.0, 200.0, 1.0, 3, 4)?;
        part.data.fill(5.0);
        part.write_tiff(&dir.join("part.tif"), Some(25830))?;
        let job = AlignJob {
            reference: dir.join("ref.tif"),
            inputs: vec![
                (dir.join("fine.tif"), dir.join("fine_a.tif")),
                (dir.join("part.tif"), dir.join("part_a.tif")),
            ],
            resample: None,
            dtype: Some(Dtype::F32),
            nodata: -9999.0,
            compression: None,
            src_nodata: None,
        };
        let text = align(&job)?;
        assert!(text.contains("media"), "{text}");
        let reference = Grid::read(&dir.join("ref.tif"))?;
        for name in ["fine_a.tif", "part_a.tif"] {
            let a = Grid::read(&dir.join(name))?;
            assert!(
                (a.x0, a.y0, a.cell, a.cols, a.rows)
                    == (
                        reference.x0,
                        reference.y0,
                        reference.cell,
                        reference.cols,
                        reference.rows
                    ),
                "{name} does not sit on the reference cells"
            );
        }
        // el ráster fino cambia de 4 a 8 en x = 104 (celda 10 de 0,5 m desde 99): las cuatro
        // primeras columnas de la referencia son 4 y las dos últimas 8
        let a = Grid::read(&dir.join("fine_a.tif"))?;
        assert_eq!(&a.data[..6], &[4.5, 4.5, 4.5, 4.5, 8.5, 8.5]);
        // el ráster parcial deja vacías las celdas que no cubre
        let p = Grid::read(&dir.join("part_a.tif"))?;
        assert_eq!(p.data[0], 5.0);
        assert!(p.data[5].is_nan());
        // las clases conservan sus valores: los números enteros los lleva la celda más cercana, o
        // la más frecuente al reducir (aquí 2 x 2 celdas finas bajo cada celda de referencia)
        let mut cls = Grid::new(100.0, 200.0, 0.5, 12, 8)?;
        for (i, v) in cls.data.iter_mut().enumerate() {
            *v = if (i % 12) < 6 { 3.0 } else { 7.0 };
        }
        cls.write_tiff(&dir.join("cls.tif"), Some(25830))?;
        let text = align(&AlignJob {
            inputs: vec![(dir.join("cls.tif"), dir.join("cls_a.tif"))],
            ..job.clone()
        })?;
        assert!(text.contains("moda"), "{text}");
        let c = Grid::read(&dir.join("cls_a.tif"))?;
        assert_eq!(&c.data[..6], &[3.0, 3.0, 3.0, 7.0, 7.0, 7.0]);
        // un ráster en otro sistema se lleva al de la referencia: las celdas son las de la
        // referencia y los valores vienen de donde cae cada celda en el origen
        let mut other = Grid::new(100.0, 200.0, 1.0, 6, 4)?;
        other.data.fill(1.5);
        other.write_tiff(&dir.join("other.tif"), Some(25829))?;
        align(&AlignJob {
            inputs: vec![(dir.join("other.tif"), dir.join("other_a.tif"))],
            ..job
        })?;
        let o = Grid::read(&dir.join("other_a.tif"))?;
        assert_eq!((o.cols, o.rows, o.x0, o.y0), (6, 4, 100.0, 200.0));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn stack_joins_bands() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl-stack-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let mut srcs = Vec::new();
        for (n, v) in [("r.tif", 1.0), ("g.tif", 2.0), ("nir.tif", 3.0)] {
            let p = dir.join(n);
            tile(0.0, v, 5)?.write_tiff(&p, Some(25830))?;
            srcs.push(Source {
                path: p,
                bands: vec![],
            });
        }
        let out = dir.join("s.tif");
        let mut job = RasterJob::new(srcs, out.clone());
        job.dtype = Some(Dtype::F32);
        stack(&job)?;
        let img = crate::raster::read_image(&out)?;
        assert_eq!(img.bands.len(), 3);
        assert_eq!(
            (img.bands[0][0], img.bands[1][0], img.bands[2][4]),
            (1.0, 2.0, 3.0)
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
