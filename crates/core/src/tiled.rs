//! Tiled GeoTIFF of any number of bands, written as a stream and laid out as a Cloud Optimized
//! GeoTIFF: 512 x 512 tiles, halved copies (overviews) down to one tile, and every directory at
//! the start of the file followed by the tiles of the smallest overview up to those of the full
//! image.
//!
//! Rows arrive in order and are kept only until a band of 512 rows is complete: its tiles are
//! compressed in parallel and appended to a temporary file per level, and the band, halved, feeds
//! the next level. At the end the directories are written and the levels copied after them, so
//! memory follows one band of rows per level, never the image, and the disk holds the result
//! twice for a moment.
//!
//! Tiles are compressed with deflate (lossless), or with JPEG for 8-bit images of one or three
//! bands (photographs): three bands go as YCbCr with the chroma halved, as aerial orthophotos are
//! usually distributed, about ten times smaller than deflate.

use std::fs::File;
use std::io::{BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::rastertools::Dtype;
use crate::{Error, Result};

/// Side of the tiles, in pixels.
pub const TILE: usize = 512;

/// How the tiles are compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Lossless.
    Deflate,
    /// Lossy, with this quality (1 to 100); only for 8-bit images of one or three bands.
    Jpeg(u8),
}

/// What is written.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub cols: usize,
    pub rows: usize,
    pub bands: usize,
    pub dtype: Dtype,
    /// Cell size and the corner of the top-left pixel.
    pub cell: f64,
    pub x0: f64,
    pub top: f64,
    pub epsg: Option<u16>,
    /// What empty pixels hold (NaN samples are written as this).
    pub nodata: f32,
    pub compression: Compression,
    /// Overviews take the first valid pixel of each 2 x 2 instead of their mean (classes, ids).
    pub nearest: bool,
    /// Write BigTIFF even if the file is small (it is used above 4 GB anyway).
    pub bigtiff: bool,
}

impl Spec {
    fn bytes_per_sample(&self) -> usize {
        match self.dtype {
            Dtype::U8 => 1,
            Dtype::U16 => 2,
            Dtype::F32 => 4,
        }
    }
}

/// The compression for an image when the user has not chosen: JPEG for 8-bit photographs
/// (three bands), deflate for everything else, which may hold measurements or classes.
pub fn auto_compression(dtype: Dtype, bands: usize) -> Compression {
    if dtype == Dtype::U8 && bands == 3 {
        Compression::Jpeg(90)
    } else {
        Compression::Deflate
    }
}

/// One level of the pyramid while it is written.
struct Level {
    cols: usize,
    rows: usize,
    /// Rows received and not yet cut into tiles, as samples of the output type.
    pending: Vec<u8>,
    /// Rows already cut into tiles.
    done: usize,
    tmp_path: PathBuf,
    /// `None` once the level is closed, so that its file can be removed (Windows does not
    /// remove a file that is still open).
    tmp: Option<BufWriter<File>>,
    /// Compressed size of each tile, in the order they were written (row by row).
    sizes: Vec<u64>,
}

/// A tiled, cloud-optimized GeoTIFF being written. Rows go in with [`TiledWriter::push`];
/// [`TiledWriter::finish`] writes the file.
pub struct TiledWriter {
    spec: Spec,
    path: PathBuf,
    levels: Vec<Level>,
}

/// Removes the temporary files of the levels when the writer is dropped (finished or not).
impl Drop for TiledWriter {
    fn drop(&mut self) {
        for l in &mut self.levels {
            l.tmp = None;
            let _ = std::fs::remove_file(&l.tmp_path);
        }
    }
}

impl TiledWriter {
    pub fn create(path: &Path, spec: Spec) -> Result<Self> {
        if spec.cols == 0 || spec.rows == 0 || spec.bands == 0 {
            return Err(Error::Param("una imagen vacía no se puede escribir".into()));
        }
        if let Compression::Jpeg(q) = spec.compression
            && (spec.dtype != Dtype::U8 || !(spec.bands == 1 || spec.bands == 3) || q == 0)
        {
            return Err(Error::Param(
                "la compresión JPEG es solo para imágenes de 8 bits de una o tres bandas: usa deflate"
                    .into(),
            ));
        }
        crate::paths::ensure_parent(path)?;
        let (mut cols, mut rows) = (spec.cols, spec.rows);
        let mut levels = Vec::new();
        let name = path
            .file_name()
            .map_or("imagen".into(), |n| n.to_string_lossy().into_owned());
        loop {
            let tmp_path = path.with_file_name(format!(".{name}.nivel{}.tmp", levels.len()));
            let tmp = BufWriter::with_capacity(1 << 20, File::create(&tmp_path)?);
            levels.push(Level {
                cols,
                rows,
                pending: Vec::new(),
                done: 0,
                tmp_path,
                tmp: Some(tmp),
                sizes: Vec::new(),
            });
            if cols <= TILE && rows <= TILE {
                break;
            }
            (cols, rows) = (cols.div_ceil(2), rows.div_ceil(2));
        }
        Ok(Self {
            spec,
            path: path.to_path_buf(),
            levels,
        })
    }

    /// Adds full rows of interleaved samples (band by band within each pixel); NaN is no data.
    pub fn push(&mut self, rows: &[f32]) -> Result<()> {
        let width = self.spec.cols * self.spec.bands;
        if !rows.len().is_multiple_of(width) {
            return Err(Error::Param("filas incompletas".into()));
        }
        let mut bytes = Vec::with_capacity(rows.len() * self.spec.bytes_per_sample());
        for &v in rows {
            put(&mut bytes, v, self.spec.dtype, self.spec.nodata);
        }
        self.take(0, bytes, false)
    }

    /// Hands `bytes` (whole rows of level `k`) to that level, cutting every complete band of
    /// rows into tiles; with `last`, the rows left over too.
    fn take(&mut self, k: usize, bytes: Vec<u8>, last: bool) -> Result<()> {
        let bps = self.spec.bytes_per_sample();
        let nb = self.spec.bands;
        let row_bytes = self.levels[k].cols * nb * bps;
        self.levels[k].pending.extend_from_slice(&bytes);
        loop {
            let have = self.levels[k].pending.len() / row_bytes;
            let left = self.levels[k].rows - self.levels[k].done;
            let take = if have >= TILE {
                TILE
            } else if last && have > 0 && have >= left.min(TILE) {
                have
            } else {
                break;
            };
            crate::check_cancel()?;
            let band: Vec<u8> = self.levels[k].pending.drain(..take * row_bytes).collect();
            self.write_band(k, &band, take)?;
            self.levels[k].done += take;
            if k + 1 < self.levels.len() {
                let half = halve(
                    &band,
                    self.levels[k].cols,
                    take,
                    nb,
                    self.spec.dtype,
                    self.spec.nodata,
                    self.spec.nearest,
                );
                let next_last = last && self.levels[k].done >= self.levels[k].rows;
                self.take(k + 1, half, next_last)?;
            }
        }
        Ok(())
    }

    /// Cuts `rows` rows of level `k` into tiles, compresses them and appends them.
    fn write_band(&mut self, k: usize, band: &[u8], rows: usize) -> Result<()> {
        let spec = &self.spec;
        let (cols, nb, bps) = (self.levels[k].cols, spec.bands, spec.bytes_per_sample());
        let across = cols.div_ceil(TILE);
        let fill = {
            let mut v = Vec::new();
            put(&mut v, f32::NAN, spec.dtype, spec.nodata);
            v
        };
        let packed: Vec<Result<Vec<u8>>> = (0..across)
            .into_par_iter()
            .map(|t| {
                let px = TILE * nb * bps;
                let mut tile = Vec::with_capacity(TILE * px);
                for r in 0..TILE {
                    let c0 = t * TILE;
                    let c1 = (c0 + TILE).min(cols);
                    if r < rows {
                        let at = (r * cols + c0) * nb * bps;
                        tile.extend_from_slice(&band[at..at + (c1 - c0) * nb * bps]);
                    }
                    let used = if r < rows { c1 - c0 } else { 0 };
                    for _ in used..TILE {
                        tile.extend_from_slice(&fill.repeat(nb));
                    }
                }
                compress(&tile, spec)
            })
            .collect();
        let level = &mut self.levels[k];
        for p in packed {
            let p = p?;
            if let Some(t) = level.tmp.as_mut() {
                t.write_all(&p)?;
            }
            level.sizes.push(p.len() as u64);
        }
        Ok(())
    }

    /// Writes the file: header, the directories of every level, then the tiles from the smallest
    /// overview to the full image.
    pub fn finish(mut self) -> Result<()> {
        // what is left of every level, from the full image down (a level whose rows ended on a
        // whole band has nothing to hand on, so each one is asked in turn)
        for k in 0..self.levels.len() {
            self.take(k, Vec::new(), true)?;
        }
        for k in 0..self.levels.len() {
            if self.levels[k].done < self.levels[k].rows {
                return Err(Error::Param(format!(
                    "faltan filas: el nivel {k} tiene {} de {}",
                    self.levels[k].done, self.levels[k].rows
                )));
            }
            if let Some(t) = self.levels[k].tmp.as_mut() {
                t.flush()?;
            }
        }
        let data: u64 = self.levels.iter().flat_map(|l| &l.sizes).sum();
        let big = self.spec.bigtiff || data > 3_900_000_000;
        let header = if big { 16u64 } else { 8 };
        // the directories with offsets of the right width, first to measure them
        let dirs = |start: u64| -> Vec<Vec<u8>> {
            let mut at = start;
            let mut base: Vec<u64> = vec![0; self.levels.len()];
            for k in (0..self.levels.len()).rev() {
                base[k] = at;
                at += self.levels[k].sizes.iter().sum::<u64>();
            }
            let mut out = Vec::new();
            let mut pos = header;
            for (k, l) in self.levels.iter().enumerate() {
                let mut offsets = Vec::with_capacity(l.sizes.len());
                let mut o = base[k];
                for s in &l.sizes {
                    offsets.push(o);
                    o += s;
                }
                let d = directory(&self.spec, l.cols, l.rows, k, &offsets, &l.sizes, pos, big);
                pos += d.len() as u64;
                out.push(d);
            }
            out
        };
        let size: u64 = dirs(0).iter().map(|d| d.len() as u64).sum();
        let start = header + size;
        let dirs = dirs(start);
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&self.path)?);
        if big {
            w.write_all(b"II")?;
            w.write_all(&43u16.to_le_bytes())?;
            w.write_all(&8u16.to_le_bytes())?;
            w.write_all(&0u16.to_le_bytes())?;
            w.write_all(&16u64.to_le_bytes())?;
        } else {
            w.write_all(b"II")?;
            w.write_all(&42u16.to_le_bytes())?;
            w.write_all(&8u32.to_le_bytes())?;
        }
        for d in &dirs {
            w.write_all(d)?;
        }
        for k in (0..self.levels.len()).rev() {
            crate::check_cancel()?;
            let mut r = BufReader::with_capacity(1 << 20, File::open(&self.levels[k].tmp_path)?);
            std::io::copy(&mut r, &mut w)?;
        }
        w.flush()?;
        drop(w);
        // the directory chain: each one points at the next
        let mut f = std::fs::OpenOptions::new().write(true).open(&self.path)?;
        let mut pos = header;
        for (k, d) in dirs.iter().enumerate() {
            let next = if k + 1 < dirs.len() {
                pos + d.len() as u64
            } else {
                0
            };
            let (count_len, entry_len) = if big { (8u64, 20u64) } else { (2, 12) };
            let n = ifd_entries(d, big);
            f.seek(SeekFrom::Start(pos + count_len + n * entry_len))?;
            if big {
                f.write_all(&next.to_le_bytes())?;
            } else {
                f.write_all(&(next as u32).to_le_bytes())?;
            }
            pos += d.len() as u64;
        }
        f.flush()?;
        Ok(())
    }
}

/// Writes rows from `rows_of(r0, r1)` (interleaved, NaN for no data) as a cloud-optimized
/// GeoTIFF, `step` rows at a time.
pub fn write(
    path: &Path,
    spec: Spec,
    step: usize,
    mut rows_of: impl FnMut(usize, usize) -> Result<Vec<f32>>,
) -> Result<()> {
    let rows = spec.rows;
    let mut w = TiledWriter::create(path, spec)?;
    let mut r = 0;
    while r < rows {
        crate::check_cancel()?;
        let r1 = (r + step.max(1)).min(rows);
        w.push(&rows_of(r, r1)?)?;
        r = r1;
    }
    w.finish()
}

/// A sample in the output type; NaN and infinities become `nodata`.
fn put(out: &mut Vec<u8>, v: f32, dtype: Dtype, nodata: f32) {
    let v = if v.is_finite() { v } else { nodata };
    match dtype {
        Dtype::U8 => out.push(v.round().clamp(0.0, 255.0) as u8),
        Dtype::U16 => out.extend((v.round().clamp(0.0, 65535.0) as u16).to_le_bytes()),
        Dtype::F32 => out.extend(v.to_le_bytes()),
    }
}

fn get(bytes: &[u8], i: usize, dtype: Dtype) -> f32 {
    match dtype {
        Dtype::U8 => f32::from(bytes[i]),
        Dtype::U16 => f32::from(u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])),
        Dtype::F32 => f32::from_le_bytes([
            bytes[4 * i],
            bytes[4 * i + 1],
            bytes[4 * i + 2],
            bytes[4 * i + 3],
        ]),
    }
}

/// Half-size copy of `rows` rows: each pixel is the mean of the valid pixels of the 2 x 2 below
/// it (or the first valid one, with `nearest`), band by band; all empty stays empty.
fn halve(
    band: &[u8],
    cols: usize,
    rows: usize,
    nb: usize,
    dtype: Dtype,
    nodata: f32,
    nearest: bool,
) -> Vec<u8> {
    let (hc, hr) = (cols.div_ceil(2), rows.div_ceil(2));
    let rows_out: Vec<Vec<u8>> = (0..hr)
        .into_par_iter()
        .map(|r| {
            let mut out = Vec::with_capacity(hc * nb * 4);
            for c in 0..hc {
                // a pixel is empty when all its bands hold the nodata value
                let mut px: Vec<usize> = Vec::with_capacity(4);
                for dr in 0..2 {
                    for dc in 0..2 {
                        let (rr, cc) = (2 * r + dr, 2 * c + dc);
                        if rr < rows && cc < cols {
                            let i = (rr * cols + cc) * nb;
                            if (0..nb).any(|b| get(band, i + b, dtype) != nodata) {
                                px.push(i);
                            }
                        }
                    }
                }
                for b in 0..nb {
                    let v = if px.is_empty() {
                        f32::NAN
                    } else if nearest {
                        get(band, px[0] + b, dtype)
                    } else {
                        let sum: f32 = px.iter().map(|&i| get(band, i + b, dtype)).sum();
                        sum / px.len() as f32
                    };
                    put(&mut out, v, dtype, nodata);
                }
            }
            out
        })
        .collect();
    rows_out.concat()
}

fn compress(tile: &[u8], spec: &Spec) -> Result<Vec<u8>> {
    match spec.compression {
        Compression::Deflate => {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            z.write_all(tile)?;
            Ok(z.finish()?)
        }
        Compression::Jpeg(q) => {
            let mut out = Vec::new();
            let mut enc = jpeg_encoder::Encoder::new(&mut out, q.min(100));
            let color = if spec.bands == 3 {
                enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
                jpeg_encoder::ColorType::Rgb
            } else {
                jpeg_encoder::ColorType::Luma
            };
            enc.encode(tile, TILE as u16, TILE as u16, color)
                .map_err(|e| Error::Format(format!("JPEG: {e}")))?;
            Ok(out)
        }
    }
}

/// One entry of a directory: tag, type (2 ascii, 3 short, 4 long, 12 double, 16 long8), count
/// and the value's bytes.
struct Entry {
    tag: u16,
    kind: u16,
    count: u64,
    data: Vec<u8>,
}

fn shorts(tag: u16, v: &[u16]) -> Entry {
    Entry {
        tag,
        kind: 3,
        count: v.len() as u64,
        data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
    }
}

fn longs(tag: u16, v: &[u64], big: bool) -> Entry {
    Entry {
        tag,
        kind: if big { 16 } else { 4 },
        count: v.len() as u64,
        data: if big {
            v.iter().flat_map(|x| x.to_le_bytes()).collect()
        } else {
            v.iter().flat_map(|x| (*x as u32).to_le_bytes()).collect()
        },
    }
}

fn doubles(tag: u16, v: &[f64]) -> Entry {
    Entry {
        tag,
        kind: 12,
        count: v.len() as u64,
        data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
    }
}

/// How many entries a directory written by [`directory`] has.
fn ifd_entries(d: &[u8], big: bool) -> u64 {
    if big {
        d.get(..8)
            .and_then(|b| b.try_into().ok())
            .map_or(0, u64::from_le_bytes)
    } else {
        d.get(..2)
            .and_then(|b| b.try_into().ok())
            .map_or(0, |b| u64::from(u16::from_le_bytes(b)))
    }
}

/// The directory of level `k` written at `at`: entries, a zero "next" offset (set later), then
/// the values that do not fit in their entries.
#[allow(clippy::too_many_arguments)]
fn directory(
    spec: &Spec,
    cols: usize,
    rows: usize,
    k: usize,
    offsets: &[u64],
    sizes: &[u64],
    at: u64,
    big: bool,
) -> Vec<u8> {
    let nb = spec.bands;
    let (bits, format): (u16, u16) = match spec.dtype {
        Dtype::U8 => (8, 1),
        Dtype::U16 => (16, 1),
        Dtype::F32 => (32, 3),
    };
    let jpeg = matches!(spec.compression, Compression::Jpeg(_));
    let photometric: u16 = match (nb, jpeg) {
        (3, true) => 6,
        (3, false) => 2,
        _ => 1,
    };
    let mut e = Vec::new();
    if k > 0 {
        e.push(longs(254, &[1], false));
    }
    e.push(longs(256, &[cols as u64], false));
    e.push(longs(257, &[rows as u64], false));
    e.push(shorts(258, &vec![bits; nb]));
    e.push(shorts(259, &[if jpeg { 7 } else { 8 }]));
    e.push(shorts(262, &[photometric]));
    e.push(shorts(277, &[nb as u16]));
    e.push(shorts(284, &[1]));
    e.push(shorts(322, &[TILE as u16]));
    e.push(shorts(323, &[TILE as u16]));
    e.push(longs(324, offsets, big));
    e.push(longs(325, sizes, big));
    let extra = match photometric {
        1 => nb.saturating_sub(1),
        _ => nb.saturating_sub(3),
    };
    if extra > 0 {
        e.push(shorts(338, &vec![0; extra]));
    }
    e.push(shorts(339, &vec![format; nb]));
    if photometric == 6 {
        e.push(shorts(530, &[2, 2]));
    }
    if k == 0 {
        e.push(doubles(33550, &[spec.cell, spec.cell, 0.0]));
        e.push(doubles(33922, &[0.0, 0.0, 0.0, spec.x0, spec.top, 0.0]));
        let mut keys: Vec<u16> = vec![1, 1, 0, 2, 1024, 0, 1, 1, 1025, 0, 1, 1];
        if let Some(code) = spec.epsg {
            keys[3] = 3;
            keys.extend([3072, 0, 1, code]);
        }
        e.push(shorts(34735, &keys));
    }
    let nodata = format!("{}\0", spec.nodata);
    e.push(Entry {
        tag: 42113,
        kind: 2,
        count: nodata.len() as u64,
        data: nodata.into_bytes(),
    });
    e.sort_by_key(|x| x.tag);

    let (count_len, entry_len, inline) = if big { (8, 20, 8) } else { (2, 12, 4) };
    let next_len = if big { 8 } else { 4 };
    let mut extra_at = at + (count_len + e.len() * entry_len + next_len) as u64;
    let mut head = Vec::new();
    let mut tail = Vec::new();
    if big {
        head.extend((e.len() as u64).to_le_bytes());
    } else {
        head.extend((e.len() as u16).to_le_bytes());
    }
    for x in &e {
        head.extend(x.tag.to_le_bytes());
        head.extend(x.kind.to_le_bytes());
        if big {
            head.extend(x.count.to_le_bytes());
        } else {
            head.extend((x.count as u32).to_le_bytes());
        }
        if x.data.len() <= inline {
            let mut v = x.data.clone();
            v.resize(inline, 0);
            head.extend(v);
        } else {
            if big {
                head.extend(extra_at.to_le_bytes());
            } else {
                head.extend((extra_at as u32).to_le_bytes());
            }
            tail.extend(&x.data);
            if tail.len() % 2 == 1 {
                tail.push(0);
            }
            extra_at = at + (count_len + e.len() * entry_len + next_len + tail.len()) as u64;
        }
    }
    head.extend(vec![0u8; next_len]);
    head.extend(tail);
    if head.len() % 2 == 1 {
        head.push(0);
    }
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(cols: usize, rows: usize, bands: usize, dtype: Dtype, c: Compression) -> Spec {
        Spec {
            cols,
            rows,
            bands,
            dtype,
            cell: 1.0,
            x0: 500_000.0,
            top: 4_400_000.0,
            epsg: Some(25830),
            nodata: 0.0,
            compression: c,
            nearest: false,
            bigtiff: false,
        }
    }

    #[test]
    fn a_tiled_image_reads_back_with_its_overviews() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("el_tiled_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("t.tif");
        // 1300 x 1100, two bands: band 1 = column, band 2 = row (as u16)
        let (cols, rows) = (1300, 1100);
        write(
            &path,
            spec(cols, rows, 2, Dtype::U16, Compression::Deflate),
            70,
            |r0, r1| {
                let mut v = Vec::new();
                for r in r0..r1 {
                    for c in 0..cols {
                        v.push(c as f32);
                        v.push(r as f32);
                    }
                }
                Ok(v)
            },
        )?;
        let mut d = tiff::decoder::Decoder::new(BufReader::new(File::open(&path)?))
            .map_err(|e| Error::Format(e.to_string()))?;
        assert_eq!(
            d.dimensions().map_err(|e| Error::Format(e.to_string()))?,
            (1300, 1100)
        );
        let img = d.read_image().map_err(|e| Error::Format(e.to_string()))?;
        let tiff::decoder::DecodingResult::U16(px) = img else {
            return Err(Error::Format("tipo".into()));
        };
        for &(r, c) in &[(0usize, 0usize), (513, 1299), (1099, 700), (600, 511)] {
            assert_eq!(px[(r * cols + c) * 2], c as u16);
            assert_eq!(px[(r * cols + c) * 2 + 1], r as u16);
        }
        // overviews: 650 x 550, then 325 x 275
        let mut sizes = Vec::new();
        while d.more_images() {
            d.next_image().map_err(|e| Error::Format(e.to_string()))?;
            sizes.push(d.dimensions().map_err(|e| Error::Format(e.to_string()))?);
        }
        assert_eq!(sizes, vec![(650, 550), (325, 275)]);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn jpeg_is_only_for_8_bit_photographs() {
        let p = std::env::temp_dir().join("el_tiled_never.tif");
        assert!(
            TiledWriter::create(&p, spec(10, 10, 2, Dtype::U8, Compression::Jpeg(90))).is_err()
        );
        assert!(
            TiledWriter::create(&p, spec(10, 10, 1, Dtype::F32, Compression::Jpeg(90))).is_err()
        );
    }
}
