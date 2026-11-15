//! GeoTIFF por teselas de cualquier número de bandas, escrito como flujo y dispuesto como Cloud
//! Optimized GeoTIFF: teselas de 512 x 512, copias reducidas a la mitad (vistas reducidas) hasta
//! una tesela y todos los directorios al principio del archivo seguidos de las teselas de la vista
//! reducida más pequeña hasta las de la imagen completa.
//!
//! Las filas llegan en orden y se guardan solo hasta que una banda de 512 filas está completa: sus
//! teselas se comprimen en paralelo y se añaden a un archivo temporal por nivel, y la banda,
//! reducida a la mitad, alimenta el nivel siguiente. Al final se escriben los directorios y los
//! niveles se copian tras ellos, de modo que la memoria sigue a una banda de filas por nivel, nunca
//! a la imagen, y el disco guarda el resultado dos veces por un momento.
//!
//! Las teselas se comprimen con deflate (sin pérdidas) o con JPEG para imágenes de 8 bits de una o
//! tres bandas (fotografías): tres bandas van como YCbCr con el croma reducido a la mitad, como
//! suelen distribuirse las ortofotos aéreas, unas diez veces más pequeñas que con deflate.

use std::fs::File;
use std::io::{BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::rastertools::Dtype;
use crate::{Error, Result};

/// Lado de las teselas, en píxeles.
pub const TILE: usize = 512;

/// Cómo se comprimen las teselas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Lossless.
    Deflate,
    /// Con pérdidas, con esta calidad (1 a 100); solo para imágenes de 8 bits de una o tres bandas.
    Jpeg(u8),
}

/// Lo que se escribe.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub cols: usize,
    pub rows: usize,
    pub bands: usize,
    pub dtype: Dtype,
    /// Tamaño de celda y esquina del píxel superior izquierdo.
    pub cell: f64,
    pub x0: f64,
    pub top: f64,
    pub epsg: Option<u16>,
    /// Lo que contienen los píxeles vacíos (las muestras NaN se escriben con esto).
    pub nodata: f32,
    pub compression: Compression,
    /// Las vistas reducidas toman el primer píxel válido de cada 2 x 2 en lugar de su media
    /// (clases, identificadores).
    pub nearest: bool,
    /// Escribe BigTIFF aunque el archivo sea pequeño (se usa por encima de 4 GB de todos modos).
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

/// La compresión de una imagen cuando el usuario no ha elegido: JPEG para fotografías de 8 bits
/// (tres bandas), deflate para todo lo demás, que puede contener medidas o clases.
pub fn auto_compression(dtype: Dtype, bands: usize) -> Compression {
    if dtype == Dtype::U8 && bands == 3 {
        Compression::Jpeg(90)
    } else {
        Compression::Deflate
    }
}

/// Un nivel de la pirámide mientras se escribe.
struct Level {
    cols: usize,
    rows: usize,
    /// Filas recibidas y todavía no cortadas en teselas, como muestras del tipo de salida.
    pending: Vec<u8>,
    /// Filas ya cortadas en teselas.
    done: usize,
    tmp_path: PathBuf,
    /// `None` una vez cerrado el nivel, para poder borrar su archivo (Windows no borra un archivo
    /// que sigue abierto).
    tmp: Option<BufWriter<File>>,
    /// Tamaño comprimido de cada tesela, en el orden en que se escribieron (fila a fila).
    sizes: Vec<u64>,
}

/// Un GeoTIFF por teselas optimizado para la nube que se está escribiendo. Las filas entran con
/// [`TiledWriter::push`]; [`TiledWriter::finish`] escribe el archivo.
pub struct TiledWriter {
    spec: Spec,
    path: PathBuf,
    levels: Vec<Level>,
}

/// Borra los archivos temporales de los niveles cuando se suelta el escritor (terminado o no).
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

    /// Añade filas completas de muestras entrelazadas (banda a banda dentro de cada píxel); NaN es
    /// sin datos.
    pub fn push(&mut self, rows: &[f32]) -> Result<()> {
        let width = self.spec.cols * self.spec.bands;
        if !rows.len().is_multiple_of(width) {
            return Err(Error::Param("filas incompletas".into()));
        }
        let bps = self.spec.bytes_per_sample();
        let (dtype, nodata) = (self.spec.dtype, self.spec.nodata);
        // convertido en todos los núcleos: una franja grande son cientos de millones de muestras
        const CHUNK: usize = 1 << 16;
        let mut bytes = vec![0u8; rows.len() * bps];
        bytes
            .par_chunks_mut(CHUNK * bps)
            .zip(rows.par_chunks(CHUNK))
            .for_each(|(out, vals)| {
                let mut v = Vec::with_capacity(out.len());
                for &x in vals {
                    put(&mut v, x, dtype, nodata);
                }
                out.copy_from_slice(&v);
            });
        self.take(0, bytes, false)
    }

    /// Entrega `bytes` (filas enteras del nivel `k`) a ese nivel, cortando en teselas cada banda de
    /// filas completa; con `last`, también las filas que sobran.
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

    /// Corta `rows` filas del nivel `k` en teselas, las comprime y las añade.
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

    /// Escribe el archivo: cabecera, los directorios de cada nivel y luego las teselas desde la
    /// vista reducida más pequeña hasta la imagen completa.
    pub fn finish(mut self) -> Result<()> {
        // lo que queda de cada nivel, desde la imagen completa hacia abajo (un nivel cuyas filas
        // terminaron en una banda entera no tiene nada que pasar, así que se pregunta a cada uno
        // por turno)
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
        // los directorios con desplazamientos del ancho correcto, primero para medirlos
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
        // la cadena de directorios: cada uno apunta al siguiente
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

/// Escribe filas de `rows_of(r0, r1)` (entrelazadas, NaN para sin datos) como GeoTIFF optimizado
/// para la nube, `step` filas cada vez.
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

/// Una muestra en el tipo de salida; NaN e infinitos pasan a `nodata`.
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

/// Copia a la mitad de tamaño de `rows` filas: cada píxel es la media de los píxeles válidos de los
/// 2 x 2 de debajo (o el primer válido, con `nearest`), banda a banda; todo vacío sigue vacío.
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
                // un píxel está vacío cuando todas sus bandas contienen el valor nodata
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

/// Una entrada de un directorio: etiqueta, tipo (2 ascii, 3 corto, 4 largo, 12 doble, 16 largo8),
/// recuento y los bytes del valor.
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

/// Cuántas entradas tiene un directorio escrito por [`directory`].
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

/// El directorio del nivel `k` escrito en `at`: entradas, un desplazamiento «siguiente» a cero
/// (fijado después) y luego los valores que no caben en sus entradas.
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
        // 1300 x 1100, dos bandas: banda 1 = columna, banda 2 = fila (como u16)
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
            .map(|d| d.with_limits(tiff::decoder::Limits::unlimited()))
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
        // vistas reducidas: 650 x 550 y luego 325 x 275
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
