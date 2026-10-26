//! Raster tools:
//! a multiband image made from rasters (each input band becomes one output band) and a mosaic
//! of rasters that says what to do where pixels overlap (first, last, mean, minimum, maximum,
//! median, sum or the number of layers). Both read the inputs by window and write the output as
//! a cloud-optimized GeoTIFF (tiles and overviews, see [`crate::tiled`]), so memory follows a
//! block of rows, never the images.
//!
//! The inputs must share one coordinate system (nothing is reprojected); their cells may
//! differ: they are resampled to the output's.

use std::path::PathBuf;

use crate::pyramid::Pyramid;
use crate::raster::Image;
use crate::{Error, Result};

/// What to do where pixels of several inputs overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlap {
    /// The first input that has data (the order of the list).
    First,
    /// The last input that has data.
    Last,
    Mean,
    Min,
    Max,
    Median,
    Sum,
    /// How many inputs have data at the pixel.
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
    /// The mean of the source pixels under the output pixel (for reducing a fine image).
    Average,
}

/// The area the output covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    Union,
    Intersection,
    /// That of the first input.
    First,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dtype {
    U8,
    U16,
    F32,
}

/// An input raster and the bands taken from it (empty: all, numbered from 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub path: PathBuf,
    pub bands: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct RasterJob {
    pub inputs: Vec<Source>,
    pub output: PathBuf,
    /// Output cell; the finest input's when `None`.
    pub cell: Option<f64>,
    pub extent: Extent,
    pub overlap: Overlap,
    pub resample: Resample,
    /// Sample type of the output; that of the first input when `None`.
    pub dtype: Option<Dtype>,
    /// A value of the inputs that means "no data" (besides what their files declare).
    pub src_nodata: Option<f32>,
    /// What empty output pixels hold.
    pub nodata: f32,
    /// Write BigTIFF even if the image is small (it is used above 4 GB anyway).
    pub bigtiff: bool,
    /// How the tiles are compressed; `None`: JPEG for 8-bit photographs of three bands,
    /// deflate otherwise.
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

/// Most bands of an output.
pub const MAX_BANDS: usize = 64;

/// Output grid.
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
}

fn open_all(job: &RasterJob) -> Result<Vec<Opened>> {
    if job.inputs.is_empty() {
        return Err(Error::Param("no hay ráster de entrada".into()));
    }
    let mut out = Vec::new();
    for s in &job.inputs {
        let pyr = Pyramid::open(&s.path)?;
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
        out.push(Opened { pyr, bands });
    }
    if let Some(a) = out.iter().find(|o| o.pyr.epsg != out[0].pyr.epsg) {
        return Err(Error::Param(format!(
            "los rásteres están en sistemas de coordenadas distintos (EPSG {:?} y {:?}): reproyéctalos antes",
            out[0].pyr.epsg, a.pyr.epsg
        )));
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

/// The value of band `b` of `img` for the output pixel centred on `(x, y)` with side `cell`.
fn sample(img: &Image, b: usize, x: f64, y: f64, cell: f64, how: Resample) -> f32 {
    let top = img.y0 + img.rows as f64 * img.cell;
    let (fx, fy) = ((x - img.x0) / img.cell, (top - y) / img.cell);
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

/// Reads the part of input `o` that covers rows `r0..r1` of the frame; `None` when it lies
/// outside.
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
    // a margin of two source pixels for the bilinear and average kernels
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

/// Rows per block so that a block holds about 4 million samples.
fn block_rows(f: &Frame, nb: usize, layers: usize) -> usize {
    (4_000_000 / (f.cols * nb * layers.max(1))).clamp(8, 2048)
}

// -------------------------------------------------------------------------------- writing

/// Writes the frame as a cloud-optimized GeoTIFF (tiles, overviews), asking `rows_of(r0, r1)`
/// for the interleaved samples of those rows, `step` rows at a time. BigTIFF when the image
/// could pass 4 GB.
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
        // classes and counts are not averaged in the overviews
        nearest: job.overlap == Overlap::Count
            || (job.resample == Resample::Nearest && nb == 1 && dtype != Dtype::F32),
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

/// A multiband image: the bands of every input, in order, one after another.
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

// -------------------------------------------------------------------------------- align

/// Puts rasters on the cells of another: same origin, same cell, same size, so that layer by layer
/// they can be compared and stacked.
#[derive(Clone, Debug)]
pub struct AlignJob {
    /// The raster whose cells every other one takes.
    pub reference: PathBuf,
    /// Each raster to align and where its aligned copy goes.
    pub inputs: Vec<(PathBuf, PathBuf)>,
    /// How values are carried over; `None` chooses by cell size: the mean of the source pixels
    /// when they are finer than the reference's, else bilinear. Use `Nearest` for classes.
    pub resample: Option<Resample>,
    /// Sample type of the copies; that of each input when `None`.
    pub dtype: Option<Dtype>,
    /// What cells outside the input hold.
    pub nodata: f32,
    /// A value of the inputs that means "no data" (besides what their files declare).
    pub src_nodata: Option<f32>,
    /// How the copies are compressed; `None`: as [`RasterJob::compression`].
    pub compression: Option<crate::tiled::Compression>,
}

/// The resampling used when none is asked for.
fn auto_resample(source_cell: f64, reference_cell: f64) -> Resample {
    if source_cell < reference_cell * 0.99 {
        Resample::Average
    } else {
        Resample::Bilinear
    }
}

/// Writes each raster of the job on the cells of the reference. The coordinate systems must be the
/// same (nothing is reprojected). Memory follows a block of rows.
pub fn align(job: &AlignJob) -> Result<String> {
    let reference = Pyramid::open(&job.reference)?;
    let frame_of_ref = || Frame {
        x0: reference.x0,
        top: reference.y_top,
        cell: reference.cell,
        cols: reference.levels[0].cols as usize,
        rows: reference.levels[0].rows as usize,
    };
    let mut lines = Vec::new();
    for (input, output) in &job.inputs {
        crate::check_cancel()?;
        let mut sub = RasterJob::new(
            vec![Source {
                path: input.clone(),
                bands: Vec::new(),
            }],
            output.clone(),
        );
        sub.src_nodata = job.src_nodata;
        sub.nodata = job.nodata;
        sub.dtype = job.dtype;
        sub.compression = job.compression;
        let ins = open_all(&sub)?;
        let o = &ins[0];
        if let (Some(a), Some(b)) = (reference.epsg, o.pyr.epsg)
            && a != b
        {
            return Err(Error::Param(format!(
                "{} está en otro sistema de coordenadas (EPSG {b}) que la referencia (EPSG {a}): reproyéctalo antes",
                input.display()
            )));
        }
        let nb = o.bands.len();
        if nb == 0 || nb > MAX_BANDS {
            return Err(Error::Param(format!(
                "{nb} bandas: una salida admite de 1 a {MAX_BANDS}"
            )));
        }
        sub.resample = job
            .resample
            .unwrap_or_else(|| auto_resample(o.pyr.cell, reference.cell));
        let f = frame_of_ref();
        let dtype = dtype_of(&sub, &o.pyr);
        let step = block_rows(&f, nb, 2);
        let mut filled = 0u64;
        write_frame(
            &sub,
            &f,
            reference.epsg.or(o.pyr.epsg),
            dtype,
            nb,
            step,
            |r0, r1| {
                let n = (r1 - r0) * f.cols;
                let mut out = vec![f32::NAN; n * nb];
                if let Some(img) = read_block(o, &f, r0, r1, &sub)? {
                    for i in 0..n {
                        let (r, c) = (r0 + i / f.cols, i % f.cols);
                        let (x, y) = (
                            f.x0 + (c as f64 + 0.5) * f.cell,
                            f.top - (r as f64 + 0.5) * f.cell,
                        );
                        for (k, &b) in o.bands.iter().enumerate() {
                            let v = sample(&img, b, x, y, f.cell, sub.resample);
                            out[i * nb + k] = v;
                            filled += u64::from(v.is_finite());
                        }
                    }
                }
                Ok(out)
            },
        )?;
        let total = (f.cols as u64) * (f.rows as u64) * nb as u64;
        let how = match sub.resample {
            Resample::Nearest => "vecino más cercano",
            Resample::Bilinear => "bilineal",
            Resample::Average => "media",
        };
        let epsg_note = if reference.epsg.is_none() || o.pyr.epsg.is_none() {
            " (no se pudo comprobar el sistema de coordenadas: falta en uno de los archivos)"
        } else {
            ""
        };
        lines.push(format!(
            "{}: celdas de {} ({how}), {} x {}, {:.1} % con datos{epsg_note} → {}",
            crate::paths::stem(input),
            crate::paths::num(f.cell),
            f.cols,
            f.rows,
            100.0 * filled as f64 / total.max(1) as f64,
            output.display()
        ));
    }
    Ok(lines.join("\n"))
}

// -------------------------------------------------------------------------------- mosaic

/// A mosaic of rasters with the same bands, resolving overlaps as `job.overlap` says.
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
        // running value and number of layers with data, per sample
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
        // a: x 0..4 = 2, b: x 2..6 = 6 and c: x 2..6 = 10 (both overlap a in x 2..4)
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
        // reference: 1 m cells, x 100..106, y 200..204
        let mut r = Grid::new(100.0, 200.0, 1.0, 6, 4)?;
        r.data.fill(1.0);
        r.write_tiff(&dir.join("ref.tif"), Some(25830))?;
        // input: 0.5 m cells, shifted by a quarter of a metre, bigger than the reference
        let mut g = Grid::new(99.0, 199.0, 0.5, 20, 14)?;
        for (i, v) in g.data.iter_mut().enumerate() {
            *v = if (i % 20) < 10 { 4.0 } else { 8.0 };
        }
        g.write_tiff(&dir.join("fine.tif"), Some(25830))?;
        // and one that only covers part of the reference
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
        // the fine raster changes from 4 to 8 at x = 104 (cell 10 of 0.5 m from 99): the first
        // four columns of the reference are 4 and the last two 8
        let a = Grid::read(&dir.join("fine_a.tif"))?;
        assert_eq!(&a.data[..6], &[4.0, 4.0, 4.0, 4.0, 8.0, 8.0]);
        // the partial raster leaves the cells it does not cover empty
        let p = Grid::read(&dir.join("part_a.tif"))?;
        assert_eq!(p.data[0], 5.0);
        assert!(p.data[5].is_nan());
        // another coordinate system is refused
        let mut other = Grid::new(100.0, 200.0, 1.0, 6, 4)?;
        other.data.fill(1.0);
        other.write_tiff(&dir.join("other.tif"), Some(25829))?;
        let bad = AlignJob {
            inputs: vec![(dir.join("other.tif"), dir.join("other_a.tif"))],
            ..job
        };
        assert!(align(&bad).is_err());
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
