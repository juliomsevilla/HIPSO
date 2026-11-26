//! Rejillas decimales orientadas al norte: alineación, muestreo, relleno de huecos y lectura y
//! escritura de GeoTIFF.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype};
use tiff::tags::Tag;

use crate::{Error, Result};

/// El valor que se escribe para las celdas vacías.
pub const NODATA: f32 = -9999.0;

/// Máximo de celdas que puede contener una rejilla (4 bytes cada una). La memoria del equipo fija
/// el límite real (véase [`Grid::new`]): un mosaico de cientos de hojas tiene cientos de millones
/// de celdas.
pub(crate) const MAX_CELLS: usize = 1 << 32;

/// La parte de la memoria del equipo que puede ocupar una rejilla: las herramientas que leen una
/// rejilla hacen otras igual de grandes (el resultado, el entorno), así que una rejilla por encima
/// de esto haría que el equipo recurriera al intercambio con el disco.
const MEMORY_SHARE: f64 = 0.3;

/// Una rejilla de `cols` x `rows` celdas cuadradas; la fila 0 es la más al norte. Las celdas vacías
/// contienen NaN.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// Borde oeste.
    pub x0: f64,
    /// Borde sur.
    pub y0: f64,
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    pub data: Vec<f32>,
}

impl Grid {
    /// La rejilla cuyas celdas, alineadas en múltiplos de `cell`, cubren `[x mínima, y mínima, x
    /// máxima, y máxima]`.
    pub fn covering(b: [f64; 4], cell: f64) -> Result<Self> {
        if !(cell > 0.0) || b.iter().any(|v| !v.is_finite()) || b[2] < b[0] || b[3] < b[1] {
            return Err(Error::Param(format!(
                "una rejilla necesita una celda positiva y una extensión (celda {cell})"
            )));
        }
        let (x0, y0, cols, rows) = Self::covering_dims(b, cell);
        Self::new(x0, y0, cell, cols, rows)
    }

    /// La esquina y el tamaño de [`Grid::covering`], sin hacer la rejilla.
    pub fn covering_dims(b: [f64; 4], cell: f64) -> (f64, f64, usize, usize) {
        let x0 = (b[0] / cell).floor() * cell;
        let y0 = (b[1] / cell).floor() * cell;
        let cols = ((b[2] - x0) / cell).floor() as usize + 1;
        let rows = ((b[3] - y0) / cell).floor() as usize + 1;
        (x0, y0, cols, rows)
    }

    pub fn new(x0: f64, y0: f64, cell: f64, cols: usize, rows: usize) -> Result<Self> {
        let n = cols
            .checked_mul(rows)
            .filter(|n| *n <= MAX_CELLS && *n > 0)
            .ok_or_else(|| {
                Error::Param(format!(
                    "una rejilla de {cols} x {rows} es demasiado grande: usa una celda mayor o divide la zona"
                ))
            })?;
        static MEMORY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        if let Some(mem) = *MEMORY.get_or_init(crate::copc_cache::total_memory)
            && n as f64 * 4.0 > mem as f64 * MEMORY_SHARE
        {
            return Err(Error::Param(format!(
                "una rejilla de {cols} x {rows} ({} millones de celdas, {:.1} GB) no cabe en la memoria de este equipo ({:.0} GB): usa una celda mayor o divide la zona",
                n / 1_000_000,
                n as f64 * 4.0 / 1e9,
                mem as f64 / 1e9
            )));
        }
        Ok(Self {
            x0,
            y0,
            cell,
            cols,
            rows,
            data: filled(n, f32::NAN)?,
        })
    }

    /// La parte de esta rejilla dentro de `[x mínima, y mínima, x máxima, y máxima]`, sobre las
    /// mismas celdas (las de una rejilla hecha para cubrir esa extensión); las celdas que le faltan
    /// a esta rejilla quedan vacías.
    pub fn crop(&self, b: [f64; 4]) -> Result<Self> {
        let mut out = Self::covering(b, self.cell)?;
        for r in 0..out.rows {
            for c in 0..out.cols {
                let (x, y) = out.center(c, r);
                if let Some(i) = self.index(x, y) {
                    out.data[r * out.cols + c] = self.data[i];
                }
            }
        }
        Ok(out)
    }

    /// La parte de esta rejilla que contiene datos: el menor bloque de celdas enteras que contiene
    /// cada celda no vacía, sobre las mismas celdas. `None` cuando ninguna celda tiene datos.
    pub fn trimmed(&self) -> Option<Self> {
        let (mut c0, mut c1, mut r0, mut r1) = (usize::MAX, 0, usize::MAX, 0);
        for r in 0..self.rows {
            let row = &self.data[r * self.cols..(r + 1) * self.cols];
            let Some(first) = row.iter().position(|v| !v.is_nan()) else {
                continue;
            };
            let last = row.iter().rposition(|v| !v.is_nan()).unwrap_or(first);
            c0 = c0.min(first);
            c1 = c1.max(last);
            r0 = r0.min(r);
            r1 = r;
        }
        if r0 == usize::MAX {
            return None;
        }
        let (cols, rows) = (c1 - c0 + 1, r1 - r0 + 1);
        let mut out = Self::new(
            self.x0 + c0 as f64 * self.cell,
            self.y0 + (self.rows - 1 - r1) as f64 * self.cell,
            self.cell,
            cols,
            rows,
        )
        .ok()?;
        for r in 0..rows {
            let from = (r0 + r) * self.cols + c0;
            out.data[r * cols..(r + 1) * cols].copy_from_slice(&self.data[from..from + cols]);
        }
        Some(out)
    }

    /// Esta superficie sobre las celdas de `like`: cada celda toma el valor bilineal en su centro
    /// ([`Grid::sample`]); las celdas fuera de esta rejilla, o donde está vacía, quedan vacías.
    pub fn resampled_onto(&self, like: &Grid) -> Grid {
        let mut out = like.like();
        for r in 0..out.rows {
            for c in 0..out.cols {
                let (x, y) = out.center(c, r);
                if self.index(x, y).is_some()
                    && let Some(v) = self.sample(x, y)
                {
                    out.data[r * out.cols + c] = v;
                }
            }
        }
        out
    }

    /// Una rejilla vacía con las mismas celdas.
    pub fn like(&self) -> Self {
        Self {
            data: vec![f32::NAN; self.data.len()],
            ..*self
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// La columna y la fila de un punto, si está dentro.
    #[inline]
    pub fn col_row(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let c = ((x - self.x0) / self.cell).floor();
        let r = ((y - self.y0) / self.cell).floor();
        if c < 0.0 || r < 0.0 || c >= self.cols as f64 || r >= self.rows as f64 {
            return None;
        }
        Some((c as usize, self.rows - 1 - r as usize))
    }

    #[inline]
    pub fn index(&self, x: f64, y: f64) -> Option<usize> {
        self.col_row(x, y).map(|(c, r)| r * self.cols + c)
    }

    /// El centro de la celda `(col, row)`.
    pub fn center(&self, col: usize, row: usize) -> (f64, f64) {
        (
            self.x0 + (col as f64 + 0.5) * self.cell,
            self.y0 + ((self.rows - row) as f64 - 0.5) * self.cell,
        )
    }

    /// La superficie en `(x, y)`, interpolada entre los cuatro centros de celda más cercanos; los
    /// vecinos vacíos recurren a la celda que contiene el punto. Fuera de la rejilla (su borde
    /// incluido) no hay valor.
    pub fn sample(&self, x: f64, y: f64) -> Option<f32> {
        let (right, top) = (
            self.x0 + self.cols as f64 * self.cell,
            self.y0 + self.rows as f64 * self.cell,
        );
        if !(x >= self.x0 && x <= right && y >= self.y0 && y <= top) {
            return None;
        }
        let fx = (x - self.x0) / self.cell - 0.5;
        let fy = (self.y0 + self.rows as f64 * self.cell - y) / self.cell - 0.5;
        let (c0, r0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - c0) as f32, (fy - r0) as f32);
        let at = |c: f64, r: f64| -> Option<f32> {
            let (c, r) = (
                c.clamp(0.0, (self.cols - 1) as f64) as usize,
                r.clamp(0.0, (self.rows - 1) as f64) as usize,
            );
            self.data
                .get(r * self.cols + c)
                .copied()
                .filter(|v| !v.is_nan())
        };
        match (
            at(c0, r0),
            at(c0 + 1.0, r0),
            at(c0, r0 + 1.0),
            at(c0 + 1.0, r0 + 1.0),
        ) {
            (Some(a), Some(b), Some(c), Some(d)) => {
                Some((a * (1.0 - tx) + b * tx) * (1.0 - ty) + (c * (1.0 - tx) + d * tx) * ty)
            }
            _ => self
                .index(x, y)
                .and_then(|i| self.data.get(i).copied())
                .filter(|v| !v.is_nan()),
        }
    }

    /// Rellena las celdas vacías de fuera adentro: cada una toma la media de sus vecinas ya con
    /// valor (conectividad 8), capa a capa, en una pasada sobre la rejilla.
    pub fn fill_holes(&mut self) {
        self.fill_holes_within(usize::MAX);
    }

    /// Como [`Grid::fill_holes`], pero solo hasta `layers` celdas desde el borde de un hueco: un
    /// hueco mayor conserva su centro vacío (agua, un hueco en el vuelo).
    pub fn fill_holes_within(&mut self, layers: usize) {
        let (cols, rows) = (self.cols as isize, self.rows as isize);
        let neighbours = |i: usize| {
            let (c, r) = ((i % self.cols) as isize, (i / self.cols) as isize);
            (-1..=1)
                .flat_map(move |dr| (-1..=1).map(move |dc| (c + dc, r + dr)))
                .filter(move |&(nc, nr)| {
                    (nc, nr) != (c, r) && nc >= 0 && nr >= 0 && nc < cols && nr < rows
                })
                .map(move |(nc, nr)| (nr * cols + nc) as usize)
        };
        let mut queued = vec![false; self.data.len()];
        let mut layer: Vec<usize> = Vec::new();
        for (i, q) in queued.iter_mut().enumerate() {
            if self.data[i].is_nan() && neighbours(i).any(|n| !self.data[n].is_nan()) {
                *q = true;
                layer.push(i);
            }
        }
        let mut next = VecDeque::new();
        let mut done = 0usize;
        while !layer.is_empty() && done < layers {
            done += 1;
            // Los valores de esta capa vienen solo de capas anteriores: se calculan primero y luego
            // se fijan.
            let values: Vec<f32> = layer
                .iter()
                .map(|&i| {
                    let (mut s, mut n) = (0.0f32, 0u32);
                    for v in neighbours(i).map(|j| self.data[j]).filter(|v| !v.is_nan()) {
                        s += v;
                        n += 1;
                    }
                    if n > 0 { s / n as f32 } else { f32::NAN }
                })
                .collect();
            for (&i, v) in layer.iter().zip(values) {
                self.data[i] = v;
            }
            for &i in &layer {
                for j in neighbours(i) {
                    if !queued[j] && self.data[j].is_nan() {
                        queued[j] = true;
                        next.push_back(j);
                    }
                }
            }
            layer = next.drain(..).collect();
        }
    }

    /// Escribe un GeoTIFF de decimales de 32 bits comprimido con Deflate; `epsg` es el sistema
    /// proyectado.
    pub fn write_tiff(&self, path: &Path, epsg: Option<u16>) -> Result<()> {
        self.write_tiff_vertical(path, epsg, None)
    }

    /// Como [`Grid::write_tiff`], declarando además la referencia vertical de los valores (código
    /// EPSG de un sistema vertical, GeoKey 4096).
    pub fn write_tiff_vertical(
        &self,
        path: &Path,
        epsg: Option<u16>,
        vertical: Option<u16>,
    ) -> Result<()> {
        let out: Vec<f32> = self
            .data
            .iter()
            .map(|v| if v.is_nan() { NODATA } else { *v })
            .collect();
        self.write_as::<colortype::Gray32Float>(path, epsg, vertical, &NODATA.to_string(), &out)
    }

    /// Escribe un GeoTIFF gris de 8 bits (una imagen): `lo` pasa a ser 1 y `hi` 255, los valores
    /// fuera se recortan y las celdas vacías son 0, declarado como sin datos. Los mismos `lo` y
    /// `hi` para varias hojas les dan una misma escala de tonos, de modo que se unen sin costuras.
    pub fn write_tiff_u8(&self, path: &Path, epsg: Option<u16>, lo: f32, hi: f32) -> Result<()> {
        let k = if hi > lo { 254.0 / (hi - lo) } else { 0.0 };
        let out: Vec<u8> = self
            .data
            .iter()
            .map(|v| {
                if v.is_nan() {
                    0
                } else {
                    (1.0 + ((*v - lo) * k).clamp(0.0, 254.0)).round() as u8
                }
            })
            .collect();
        self.write_as::<colortype::Gray8>(path, epsg, None, "0", &out)
    }

    fn write_as<C: colortype::ColorType>(
        &self,
        path: &Path,
        epsg: Option<u16>,
        vertical: Option<u16>,
        nodata: &str,
        data: &[C::Inner],
    ) -> Result<()>
    where
        [C::Inner]: tiff::encoder::TiffValue,
    {
        let w = BufWriter::new(File::create(path)?);
        let mut enc = TiffEncoder::new(w)
            .map_err(tiff_err)?
            .with_compression(Compression::Deflate(DeflateLevel::Balanced));
        let (width, height) = (
            u32::try_from(self.cols).map_err(|_| too_big())?,
            u32::try_from(self.rows).map_err(|_| too_big())?,
        );
        let mut img = enc.new_image::<C>(width, height).map_err(tiff_err)?;
        let e = img.encoder();
        e.write_tag(Tag::ModelPixelScaleTag, &[self.cell, self.cell, 0.0][..])
            .map_err(tiff_err)?;
        let top = self.y0 + self.rows as f64 * self.cell;
        e.write_tag(
            Tag::ModelTiepointTag,
            &[0.0, 0.0, 0.0, self.x0, top, 0.0][..],
        )
        .map_err(tiff_err)?;
        // Proyectado, píxel como área, y el código EPSG cuando la nube lo dio.
        let mut keys: Vec<u16> = vec![1, 1, 0, 2, 1024, 0, 1, 1, 1025, 0, 1, 1];
        if let Some(code) = epsg {
            keys[3] = 3;
            keys.extend([3072, 0, 1, code]);
            if let Some(v) = vertical {
                keys[3] = 4;
                keys.extend([4096, 0, 1, v]);
            }
        }
        e.write_tag(Tag::GeoKeyDirectoryTag, &keys[..])
            .map_err(tiff_err)?;
        e.write_tag(Tag::GdalNodata, nodata).map_err(tiff_err)?;
        img.write_data(data).map_err(tiff_err)?;
        Ok(())
    }

    /// Lee un GeoTIFF o una rejilla ASCII de ESRI (`.asc`, como la escriben las herramientas
    /// compatibles con FUSION).
    pub fn read(path: &Path) -> Result<Self> {
        crate::check_raster_format(path)?;
        let asc = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("asc"));
        if asc {
            Self::read_asc(path)
        } else if crate::pyramid::is_jp2_file(path) || crate::rasterfmt::kind_of(path).is_some() {
            let img = crate::raster::read_image(path)?;
            let mut g = Self::new(img.x0, img.y0, img.cell, img.cols, img.rows)?;
            if let Some(b) = img.bands.into_iter().next() {
                g.data = b;
            }
            Ok(g)
        } else {
            Self::read_tiff(path)
        }
    }

    /// Lee una rejilla ASCII de ESRI.
    pub fn read_asc(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut words = text.split_ascii_whitespace();
        let (mut cols, mut rows, mut x, mut y, mut cell, mut nodata, mut center) =
            (0usize, 0usize, 0.0, 0.0, 0.0, None, false);
        let bad = |w: &str| {
            Error::Format(format!(
                "{}: cabecera de rejilla ASCII no válida cerca de '{w}'",
                path.display()
            ))
        };
        let mut first_value = None;
        while let Some(key) = words.next() {
            let k = key.to_ascii_lowercase();
            if !k.starts_with(|c: char| c.is_ascii_alphabetic()) {
                first_value = Some(key);
                break;
            }
            let v = words.next().ok_or_else(|| bad(key))?;
            match k.as_str() {
                "ncols" => cols = v.parse().map_err(|_| bad(v))?,
                "nrows" => rows = v.parse().map_err(|_| bad(v))?,
                "xllcorner" | "xllcenter" => {
                    (x, center) = (v.parse().map_err(|_| bad(v))?, k.ends_with("center"))
                }
                "yllcorner" | "yllcenter" => y = v.parse().map_err(|_| bad(v))?,
                "cellsize" => cell = v.parse().map_err(|_| bad(v))?,
                "nodata_value" => nodata = Some(v.parse::<f32>().map_err(|_| bad(v))?),
                _ => return Err(bad(key)),
            }
        }
        let half = if center { cell / 2.0 } else { 0.0 };
        let mut g = Self::new(x - half, y - half, cell, cols, rows)?;
        let mut n = 0;
        for (slot, w) in g.data.iter_mut().zip(first_value.into_iter().chain(words)) {
            let v: f32 = w.parse().map_err(|_| bad(w))?;
            *slot = if Some(v) == nodata { f32::NAN } else { v };
            n += 1;
        }
        if n != g.data.len() {
            return Err(Error::Format(format!(
                "{}: {n} valores para una rejilla de {cols} x {rows}",
                path.display()
            )));
        }
        Ok(g)
    }

    /// El código EPSG que declara un GeoTIFF (proyectado o, si no, geográfico), si lo hay.
    pub fn read_epsg(path: &Path) -> Option<u16> {
        use tiff::decoder::Decoder;
        if crate::pyramid::is_jp2_file(path) {
            return crate::jp2::Jp2::open(path).ok()?.geo?.epsg;
        }
        if crate::rasterfmt::kind_of(path).is_some() {
            return crate::pyramid::Pyramid::open(path).ok()?.epsg;
        }
        let mut d = Decoder::new(BufReader::new(File::open(path).ok()?)).ok()?;
        let keys = d.get_tag_u32_vec(Tag::GeoKeyDirectoryTag).ok()?;
        // cuatro números de cabecera y luego cuatro por clave: identificador, ubicación, recuento,
        // valor
        let key = |id: u32| {
            keys.get(4..)?
                .as_chunks::<4>()
                .0
                .iter()
                .find(|e| e[0] == id && e[1] == 0)
                .and_then(|e| u16::try_from(e[3]).ok())
        };
        key(3072).or_else(|| key(2048))
    }

    /// El código EPSG del sistema vertical que declara un GeoTIFF (GeoKey 4096), si lo hay.
    pub fn read_vertical_epsg(path: &Path) -> Option<u16> {
        use tiff::decoder::Decoder;
        let mut d = Decoder::new(BufReader::new(File::open(path).ok()?)).ok()?;
        let keys = d.get_tag_u32_vec(Tag::GeoKeyDirectoryTag).ok()?;
        keys.get(4..)?
            .as_chunks::<4>()
            .0
            .iter()
            .find(|e| e[0] == 4096 && e[1] == 0 && e[3] != 32767 && e[3] != 0)
            .and_then(|e| u16::try_from(e[3]).ok())
    }

    /// La extensión `[x mínima, y mínima, x máxima, y máxima]` y el tamaño de celda de un GeoTIFF
    /// orientado al norte, solo a partir de su cabecera.
    pub fn read_extent(path: &Path) -> Result<([f64; 4], f64)> {
        use tiff::decoder::Decoder;
        let mut d = Decoder::new(BufReader::new(crate::open_file(path)?)).map_err(tiff_err)?;
        let (w, h) = d.dimensions().map_err(tiff_err)?;
        let scale = d
            .get_tag_f64_vec(Tag::ModelPixelScaleTag)
            .map_err(tiff_err)?;
        let tie = d.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(tiff_err)?;
        let (Some(&sx), Some(&sy), Some(&tx), Some(&ty)) =
            (scale.first(), scale.get(1), tie.get(3), tie.get(4))
        else {
            return Err(Error::Format(format!(
                "{}: no es un GeoTIFF (sin escala de píxel ni punto de anclaje)",
                path.display()
            )));
        };
        let (tx, ty) = crate::tiffio::corner(&mut d, tx, ty, sx, sy);
        Ok(([tx, ty - f64::from(h) * sy, tx + f64::from(w) * sx, ty], sx))
    }

    /// Copia en las celdas vacías de esta rejilla los valores de `other` que caen sobre ellas (cada
    /// celda toma la celda de `other` que hay bajo su centro).
    pub fn fill_from(&mut self, other: &Grid) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                let i = r * self.cols + c;
                if self.data[i].is_finite() {
                    continue;
                }
                let (x, y) = self.center(c, r);
                if let Some(j) = other.index(x, y) {
                    self.data[i] = other.data[j];
                }
            }
        }
    }

    /// Lee un GeoTIFF de decimales de 32 bits orientado al norte como los que escribe
    /// [`Grid::write_tiff`].
    pub fn read_tiff(path: &Path) -> Result<Self> {
        use tiff::decoder::DecodingResult;
        let mut d = crate::tiffio::open(path)?;
        let (w, h) = d.dimensions().map_err(tiff_err)?;
        let scale = d
            .get_tag_f64_vec(Tag::ModelPixelScaleTag)
            .map_err(tiff_err)?;
        let tie = d.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(tiff_err)?;
        let nodata = d
            .get_tag_ascii_string(Tag::GdalNodata)
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok());
        let (Some(&sx), Some(&sy), Some(&tx), Some(&ty)) =
            (scale.first(), scale.get(1), tie.get(3), tie.get(4))
        else {
            return Err(Error::Format(format!(
                "{}: no es un GeoTIFF (sin escala de píxel ni punto de anclaje)",
                path.display()
            )));
        };
        if (sx - sy).abs() > sx * 1e-9
            || tie.first().copied().unwrap_or(0.0) != 0.0
            || tie.get(1).copied().unwrap_or(0.0) != 0.0
        {
            return Err(Error::Unsupported(format!(
                "{}: solo se leen celdas cuadradas ancladas en la esquina del píxel",
                path.display()
            )));
        }
        let (cols, rows) = (w as usize, h as usize);
        let (tx, ty) = crate::tiffio::corner(&mut d, tx, ty, sx, sy);
        // los valores se leen antes de reservar la rejilla: los límites del lector rechazan una
        // cabecera que declara un tamaño imposible para el archivo
        let data = match d.read_image().map_err(tiff_err)? {
            DecodingResult::F32(v) => v,
            DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::U8(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::U16(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::U32(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::I8(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::I16(v) => v.into_iter().map(|x| x as f32).collect(),
            DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
            _ => {
                return Err(Error::Unsupported(format!(
                    "{}: tipo de dato de ráster no admitido",
                    path.display()
                )));
            }
        };
        let mut g = Self::new(tx, ty - rows as f64 * sy, sx, cols, rows)?;
        if data.len() != g.data.len() {
            return Err(Error::Format(format!(
                "{}: se esperaba una banda de {cols} x {rows} valores reales",
                path.display()
            )));
        }
        g.data = data
            .into_iter()
            .map(|v| {
                if Some(v) == nodata || !v.is_finite() {
                    f32::NAN
                } else {
                    v
                }
            })
            .collect();
        Ok(g)
    }
}

/// Acumula la elevación media de los puntos que caen en cada celda de una rejilla; las sumas se
/// guardan relativas a una altura de referencia para tener precisión.
pub struct MeanGrid {
    grid: Grid,
    zref: f64,
    sum: Vec<f32>,
    count: Vec<u32>,
}

impl MeanGrid {
    pub fn new(grid: Grid, zref: f64) -> Result<Self> {
        let n = grid.len();
        Ok(Self {
            grid,
            zref,
            sum: filled(n, 0.0)?,
            count: filled(n, 0)?,
        })
    }

    #[inline]
    pub fn add(&mut self, x: f64, y: f64, z: f64) {
        if let Some(i) = self.grid.index(x, y)
            && let (Some(s), Some(n)) = (self.sum.get_mut(i), self.count.get_mut(i))
        {
            *s += (z - self.zref) as f32;
            *n += 1;
        }
    }

    /// La rejilla de medias; las celdas sin puntos quedan vacías.
    pub fn finish(mut self) -> Grid {
        for ((v, s), n) in self.grid.data.iter_mut().zip(&self.sum).zip(&self.count) {
            if *n > 0 {
                *v = (f64::from(*s / *n as f32) + self.zref) as f32;
            }
        }
        self.grid
    }
}

/// `n` copias de `v`, o un error (no un aborto) cuando no hay memoria.
fn filled<T: Copy>(n: usize, v: T) -> Result<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| {
        Error::Param(
            "no hay memoria para esta rejilla: usa una celda mayor o divide la zona".into(),
        )
    })?;
    out.resize(n, v);
    Ok(out)
}

fn tiff_err(e: tiff::TiffError) -> Error {
    Error::Format(format!("GeoTIFF: {e}{}", crate::cloud_hint(&e.to_string())))
}

fn too_big() -> Error {
    Error::Param("rejilla demasiado grande para un TIFF".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_holes_within_leaves_big_holes_open() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 9, 1)?;
        g.data[0] = 1.0;
        g.data[8] = 1.0;
        g.fill_holes_within(2);
        // dos capas desde cada extremo: se rellenan las cuatro celdas, la celda del medio no
        assert!(g.data[1..=2].iter().all(|v| !v.is_nan()));
        assert!(g.data[6..=7].iter().all(|v| !v.is_nan()));
        assert!(g.data[3..=5].iter().all(|v| v.is_nan()));
        Ok(())
    }

    #[test]
    fn u8_image_stretches_and_keeps_empty_cells_as_zero() -> Result<()> {
        let mut g = Grid::new(100.0, 200.0, 1.0, 3, 1)?;
        g.data[0] = 10.0;
        g.data[1] = 110.0;
        let path = std::env::temp_dir().join(format!("hipso_u8_{}.tif", std::process::id()));
        g.write_tiff_u8(&path, Some(25830), 10.0, 110.0)?;
        let back = Grid::read(&path)?;
        let _ = std::fs::remove_file(&path);
        assert_eq!((back.cols, back.rows), (3, 1));
        assert_eq!(back.data[0], 1.0);
        assert_eq!(back.data[1], 255.0);
        assert!(back.data[2].is_nan() || back.data[2] == 0.0);
        Ok(())
    }

    #[test]
    fn trimmed_keeps_the_cells_with_data() -> Result<()> {
        let mut g = Grid::new(100.0, 200.0, 2.0, 6, 5)?;
        // datos solo en las columnas 2..=3 y las filas 1..=2
        g.data[6 + 2] = 1.0;
        g.data[2 * 6 + 3] = 4.0;
        let t = g.trimmed().ok_or(Error::Param("vacía".into()))?;
        assert_eq!((t.cols, t.rows), (2, 2));
        assert_eq!((t.x0, t.y0), (104.0, 204.0));
        assert_eq!(t.sample(105.0, 207.0), Some(1.0));
        assert_eq!(t.data[3], 4.0);
        assert!(Grid::new(0.0, 0.0, 1.0, 3, 3)?.trimmed().is_none());
        Ok(())
    }

    #[test]
    fn covering_aligns_and_indexes() -> Result<()> {
        let g = Grid::covering([10.2, 20.7, 13.9, 22.0], 1.0)?;
        assert_eq!((g.x0, g.y0, g.cols, g.rows), (10.0, 20.0, 4, 3));
        // El punto suroeste está en la última fila.
        assert_eq!(g.col_row(10.2, 20.7), Some((0, 2)));
        assert_eq!(g.col_row(13.9, 22.0), Some((3, 0)));
        assert_eq!(g.col_row(9.9, 21.0), None);
        assert_eq!(g.center(0, 2), (10.5, 20.5));
        Ok(())
    }

    #[test]
    fn resampling_onto_other_cells() -> Result<()> {
        // una rampa z = x en celdas de 1 m, leída en celdas de 0,5 m que empiezan dentro y
        // sobresalen
        let mut g = Grid::new(0.0, 0.0, 1.0, 4, 2)?;
        for r in 0..2 {
            for c in 0..4 {
                g.data[r * 4 + c] = c as f32 + 0.5;
            }
        }
        let like = Grid::new(1.0, 0.0, 0.5, 8, 4)?;
        let out = g.resampled_onto(&like);
        // el centro de la primera celda es x = 1,25: la rampa da 1,25
        assert!((out.data[0] - 1.25).abs() < 1e-5, "{}", out.data[0]);
        // más allá de x = 4 no hay nada
        assert!(out.data[7].is_nan());
        assert_eq!(out.cols, 8);
        Ok(())
    }

    #[test]
    fn holes_take_their_neighbours_mean() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 5, 1)?;
        g.data = vec![1.0, f32::NAN, f32::NAN, f32::NAN, 5.0];
        g.fill_holes();
        assert_eq!(g.data, vec![1.0, 1.0, 3.0, 5.0, 5.0]);
        Ok(())
    }

    #[test]
    fn sample_interpolates_between_centres() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 2, 1)?;
        g.data = vec![0.0, 2.0];
        assert_eq!(g.sample(1.0, 0.5), Some(1.0));
        assert_eq!(g.sample(0.5, 0.5), Some(0.0));
        // el borde cuenta como dentro; más allá no hay valor
        assert_eq!(g.sample(2.0, 1.0), Some(2.0));
        assert_eq!(g.sample(2.01, 0.5), None);
        assert_eq!(g.sample(-0.01, 0.5), None);
        assert_eq!(g.sample(0.5, 1.01), None);
        Ok(())
    }

    #[test]
    fn tiff_round_trip() -> Result<()> {
        let mut g = Grid::new(277750.0, 6122250.0, 2.0, 3, 2)?;
        g.data = vec![1.0, f32::NAN, 3.0, 4.0, 5.0, 6.5];
        let p = std::env::temp_dir().join("hipso_grid_test.tif");
        g.write_tiff(&p, Some(32754))?;
        let back = Grid::read_tiff(&p)?;
        let _ = std::fs::remove_file(&p);
        assert_eq!(
            (back.x0, back.y0, back.cell, back.cols, back.rows),
            (g.x0, g.y0, g.cell, g.cols, g.rows)
        );
        assert!(back.data[1].is_nan());
        assert_eq!(back.data[5], 6.5);
        Ok(())
    }
}
