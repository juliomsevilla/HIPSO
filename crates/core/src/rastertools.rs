//! Raster tools in the way of GDAL's `gdal_merge -separate` and `gdalbuildvrt`/`gdal_merge`:
//! a multiband image made from rasters (each input band becomes one output band) and a mosaic
//! of rasters that says what to do where pixels overlap (first, last, mean, minimum, maximum,
//! median, sum or the number of layers). Both read the inputs by window and write the output
//! strip by strip (deflate), so memory follows a block of rows, never the images.
//!
//! The inputs must share one coordinate system (nothing is reprojected); their cells may
//! differ: they are resampled to the output's.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use rayon::prelude::*;

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
        return Err(Error::Param("no input raster".into()));
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
                "{} has {} band(s): there is no band {}",
                s.path.display(),
                pyr.bands,
                b + 1
            )));
        }
        out.push(Opened { pyr, bands });
    }
    if let Some(a) = out.iter().find(|o| o.pyr.epsg != out[0].pyr.epsg) {
        return Err(Error::Param(format!(
            "the rasters are in different coordinate systems (EPSG {:?} and {:?}): reproject them first",
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
        return Err(Error::Param("the cell must be positive".into()));
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
        return Err(Error::Param("the rasters do not overlap".into()));
    }
    let (cols, rows) = (
        ((b[2] - b[0]) / cell).round().max(1.0) as usize,
        ((b[3] - b[1]) / cell).round().max(1.0) as usize,
    );
    if (cols as u64) * (rows as u64) > (1 << 36) {
        return Err(Error::Param("the output would be too large".into()));
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

fn put_samples(out: &mut Vec<u8>, block: &[f32], dtype: Dtype, nodata: f32) {
    for &v in block {
        let v = if v.is_finite() { v } else { nodata };
        match dtype {
            Dtype::U8 => out.push(v.round().clamp(0.0, 255.0) as u8),
            Dtype::U16 => out.extend((v.round().clamp(0.0, 65535.0) as u16).to_le_bytes()),
            Dtype::F32 => out.extend(v.to_le_bytes()),
        }
    }
}

fn deflate(strip: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(strip)?;
    Ok(z.finish()?)
}

/// One entry of an image file directory: tag, type (3 short, 4 long, 12 double, 2 ascii, 16
/// long8), count and the value's bytes.
struct Entry {
    tag: u16,
    kind: u16,
    count: u64,
    data: Vec<u8>,
}

fn shorts(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn doubles(v: &[f64]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Writes the frame as a GeoTIFF in strips compressed with deflate, asking `rows_of(r0, r1)`
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
    mut rows_of: impl FnMut(usize, usize) -> Result<Vec<f32>>,
) -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let path = &job.output;
    crate::paths::ensure_parent(path)?;
    let bytes_per = match dtype {
        Dtype::U8 => 1usize,
        Dtype::U16 => 2,
        Dtype::F32 => 4,
    };
    let row_bytes = f.cols * nb * bytes_per;
    let big = job.bigtiff || (row_bytes as u64) * (f.rows as u64) > 3_900_000_000;
    let rps = ((1usize << 20) / row_bytes.max(1)).clamp(1, f.rows);
    let strip_bytes = rps * row_bytes;
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let header_len: u64 = if big { 16 } else { 8 };
    w.write_all(&vec![0u8; header_len as usize])?;
    let (mut offsets, mut counts): (Vec<u64>, Vec<u64>) = (Vec::new(), Vec::new());
    let mut pos = header_len;
    let mut pending: Vec<u8> = Vec::new();
    let mut flush = |pending: &mut Vec<u8>, last: bool, w: &mut BufWriter<File>| -> Result<()> {
        let mut strips: Vec<Vec<u8>> = Vec::new();
        while pending.len() >= strip_bytes {
            strips.push(pending.drain(..strip_bytes).collect());
        }
        if last && !pending.is_empty() {
            strips.push(std::mem::take(pending));
        }
        let packed: Vec<Result<Vec<u8>>> = strips.par_iter().map(|s| deflate(s)).collect();
        for p in packed {
            let p = p?;
            w.write_all(&p)?;
            offsets.push(pos);
            counts.push(p.len() as u64);
            pos += p.len() as u64;
        }
        Ok(())
    };
    let mut r = 0;
    while r < f.rows {
        crate::check_cancel()?;
        let r1 = (r + step).min(f.rows);
        put_samples(&mut pending, &rows_of(r, r1)?, dtype, job.nodata);
        flush(&mut pending, false, &mut w)?;
        r = r1;
    }
    flush(&mut pending, true, &mut w)?;
    // the directory, after the strips, on an even offset
    if pos % 2 == 1 {
        w.write_all(&[0])?;
        pos += 1;
    }
    let (bits, fmt): (u16, u16) = match dtype {
        Dtype::U8 => (8, 1),
        Dtype::U16 => (16, 1),
        Dtype::F32 => (32, 3),
    };
    let mut keys: Vec<u16> = vec![1, 1, 0, 2, 1024, 0, 1, 1, 1025, 0, 1, 1];
    if let Some(code) = epsg {
        keys[3] = 3;
        keys.extend([3072, 0, 1, code]);
    }
    let offs_data: Vec<u8> = if big {
        offsets.iter().flat_map(|x| x.to_le_bytes()).collect()
    } else {
        offsets
            .iter()
            .flat_map(|x| (*x as u32).to_le_bytes())
            .collect()
    };
    let cnt_data: Vec<u8> = if big {
        counts.iter().flat_map(|x| x.to_le_bytes()).collect()
    } else {
        counts
            .iter()
            .flat_map(|x| (*x as u32).to_le_bytes())
            .collect()
    };
    let long = |v: u64| -> Vec<u8> {
        if big {
            v.to_le_bytes().to_vec()
        } else {
            (v as u32).to_le_bytes().to_vec()
        }
    };
    let nodata_text = format!("{}\0", job.nodata);
    let mut entries = vec![
        Entry {
            tag: 256,
            kind: if big { 16 } else { 4 },
            count: 1,
            data: long(f.cols as u64),
        },
        Entry {
            tag: 257,
            kind: if big { 16 } else { 4 },
            count: 1,
            data: long(f.rows as u64),
        },
        Entry {
            tag: 258,
            kind: 3,
            count: nb as u64,
            data: shorts(&vec![bits; nb]),
        },
        Entry {
            tag: 259,
            kind: 3,
            count: 1,
            data: shorts(&[8]),
        },
        Entry {
            tag: 262,
            kind: 3,
            count: 1,
            data: shorts(&[if nb == 3 { 2 } else { 1 }]),
        },
        Entry {
            tag: 273,
            kind: if big { 16 } else { 4 },
            count: offsets.len() as u64,
            data: offs_data,
        },
        Entry {
            tag: 277,
            kind: 3,
            count: 1,
            data: shorts(&[nb as u16]),
        },
        Entry {
            tag: 278,
            kind: if big { 16 } else { 4 },
            count: 1,
            data: long(rps as u64),
        },
        Entry {
            tag: 279,
            kind: if big { 16 } else { 4 },
            count: counts.len() as u64,
            data: cnt_data,
        },
        Entry {
            tag: 284,
            kind: 3,
            count: 1,
            data: shorts(&[1]),
        },
        Entry {
            tag: 339,
            kind: 3,
            count: nb as u64,
            data: shorts(&vec![fmt; nb]),
        },
        Entry {
            tag: 33550,
            kind: 12,
            count: 3,
            data: doubles(&[f.cell, f.cell, 0.0]),
        },
        Entry {
            tag: 33922,
            kind: 12,
            count: 6,
            data: doubles(&[0.0, 0.0, 0.0, f.x0, f.top, 0.0]),
        },
        Entry {
            tag: 34735,
            kind: 3,
            count: keys.len() as u64,
            data: shorts(&keys),
        },
        Entry {
            tag: 42113,
            kind: 2,
            count: nodata_text.len() as u64,
            data: nodata_text.into_bytes(),
        },
    ];
    entries.sort_by_key(|e| e.tag);
    let inline = if big { 8 } else { 4 };
    // values that do not fit in the entry go before the directory
    let mut outside: Vec<u8> = Vec::new();
    let mut slots: Vec<Vec<u8>> = Vec::new();
    for e in &entries {
        if e.data.len() <= inline {
            let mut v = e.data.clone();
            v.resize(inline, 0);
            slots.push(v);
        } else {
            let at = pos + outside.len() as u64;
            outside.extend(&e.data);
            if outside.len() % 2 == 1 {
                outside.push(0);
            }
            slots.push(if big {
                at.to_le_bytes().to_vec()
            } else {
                (at as u32).to_le_bytes().to_vec()
            });
        }
    }
    w.write_all(&outside)?;
    let ifd_at = pos + outside.len() as u64;
    if big {
        w.write_all(&(entries.len() as u64).to_le_bytes())?;
    } else {
        w.write_all(&(entries.len() as u16).to_le_bytes())?;
    }
    for (e, slot) in entries.iter().zip(&slots) {
        w.write_all(&e.tag.to_le_bytes())?;
        w.write_all(&e.kind.to_le_bytes())?;
        if big {
            w.write_all(&e.count.to_le_bytes())?;
        } else {
            w.write_all(&(e.count as u32).to_le_bytes())?;
        }
        w.write_all(slot)?;
    }
    w.write_all(&if big { vec![0u8; 8] } else { vec![0u8; 4] })?;
    w.seek(SeekFrom::Start(0))?;
    if big {
        w.write_all(b"II")?;
        w.write_all(&43u16.to_le_bytes())?;
        w.write_all(&8u16.to_le_bytes())?;
        w.write_all(&0u16.to_le_bytes())?;
        w.write_all(&ifd_at.to_le_bytes())?;
    } else {
        w.write_all(b"II")?;
        w.write_all(&42u16.to_le_bytes())?;
        w.write_all(&(ifd_at as u32).to_le_bytes())?;
    }
    w.flush()?;
    Ok(())
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
            "{nb} bands: an output has 1 to {MAX_BANDS}"
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

// -------------------------------------------------------------------------------- mosaic

/// A mosaic of rasters with the same bands, resolving overlaps as `job.overlap` says.
pub fn mosaic(job: &RasterJob) -> Result<String> {
    let ins = open_all(job)?;
    let nb = ins[0].bands.len();
    if ins.iter().any(|o| o.bands.len() != nb) {
        return Err(Error::Param(
            "all the rasters of a mosaic need the same number of bands".into(),
        ));
    }
    if nb == 0 || nb > MAX_BANDS {
        return Err(Error::Param(format!(
            "{nb} bands: 1 to {MAX_BANDS} are allowed"
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
