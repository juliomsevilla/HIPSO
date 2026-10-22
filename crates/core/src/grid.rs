//! North-up float grids: alignment, sampling, hole filling and GeoTIFF I/O.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype};
use tiff::tags::Tag;

use crate::{Error, Result};

/// The value written for empty cells.
pub const NODATA: f32 = -9999.0;

/// Most cells one grid may hold (4 bytes each: 2 GiB).
const MAX_CELLS: usize = 1 << 29;

/// A grid of `cols` x `rows` square cells; row 0 is the northernmost. Empty cells hold NaN.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// West edge.
    pub x0: f64,
    /// South edge.
    pub y0: f64,
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    pub data: Vec<f32>,
}

impl Grid {
    /// The grid whose cells, aligned on multiples of `cell`, cover `[min x, min y, max x, max y]`.
    pub fn covering(b: [f64; 4], cell: f64) -> Result<Self> {
        if !(cell > 0.0) || b.iter().any(|v| !v.is_finite()) || b[2] < b[0] || b[3] < b[1] {
            return Err(Error::Param(format!(
                "a grid needs a positive cell size and an extent (cell {cell})"
            )));
        }
        let x0 = (b[0] / cell).floor() * cell;
        let y0 = (b[1] / cell).floor() * cell;
        let cols = ((b[2] - x0) / cell).floor() as usize + 1;
        let rows = ((b[3] - y0) / cell).floor() as usize + 1;
        Self::new(x0, y0, cell, cols, rows)
    }

    pub fn new(x0: f64, y0: f64, cell: f64, cols: usize, rows: usize) -> Result<Self> {
        let n = cols
            .checked_mul(rows)
            .filter(|n| *n <= MAX_CELLS && *n > 0)
            .ok_or_else(|| {
                Error::Param(format!(
                    "a {cols} x {rows} grid is too large: use a bigger cell size or split the area"
                ))
            })?;
        Ok(Self {
            x0,
            y0,
            cell,
            cols,
            rows,
            data: filled(n, f32::NAN)?,
        })
    }

    /// An empty grid with the same cells.
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

    /// The column and row of a point, if inside.
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

    /// The centre of cell `(col, row)`.
    pub fn center(&self, col: usize, row: usize) -> (f64, f64) {
        (
            self.x0 + (col as f64 + 0.5) * self.cell,
            self.y0 + ((self.rows - row) as f64 - 0.5) * self.cell,
        )
    }

    /// The surface at `(x, y)`, interpolated between the four nearest cell centres; empty
    /// neighbours fall back to the cell holding the point.
    pub fn sample(&self, x: f64, y: f64) -> Option<f32> {
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

    /// Fills empty cells from the outside in: each takes the mean of its already-valued
    /// neighbours (8-connected), layer by layer, in one pass over the grid.
    pub fn fill_holes(&mut self) {
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
        while !layer.is_empty() {
            // Values of this layer come only from earlier layers: computed first, then set.
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

    /// Writes a Deflate-compressed 32-bit float GeoTIFF; `epsg` is the projected system.
    pub fn write_tiff(&self, path: &Path, epsg: Option<u16>) -> Result<()> {
        let w = BufWriter::new(File::create(path)?);
        let mut enc = TiffEncoder::new(w)
            .map_err(tiff_err)?
            .with_compression(Compression::Deflate(DeflateLevel::Balanced));
        let (width, height) = (
            u32::try_from(self.cols).map_err(|_| too_big())?,
            u32::try_from(self.rows).map_err(|_| too_big())?,
        );
        let mut img = enc
            .new_image::<colortype::Gray32Float>(width, height)
            .map_err(tiff_err)?;
        let e = img.encoder();
        e.write_tag(Tag::ModelPixelScaleTag, &[self.cell, self.cell, 0.0][..])
            .map_err(tiff_err)?;
        let top = self.y0 + self.rows as f64 * self.cell;
        e.write_tag(
            Tag::ModelTiepointTag,
            &[0.0, 0.0, 0.0, self.x0, top, 0.0][..],
        )
        .map_err(tiff_err)?;
        // Projected, pixel-is-area, and the EPSG code when the cloud gave one.
        let mut keys: Vec<u16> = vec![1, 1, 0, 2, 1024, 0, 1, 1, 1025, 0, 1, 1];
        if let Some(code) = epsg {
            keys[3] = 3;
            keys.extend([3072, 0, 1, code]);
        }
        e.write_tag(Tag::GeoKeyDirectoryTag, &keys[..])
            .map_err(tiff_err)?;
        e.write_tag(Tag::GdalNodata, NODATA.to_string().as_str())
            .map_err(tiff_err)?;
        let out: Vec<f32> = self
            .data
            .iter()
            .map(|v| if v.is_nan() { NODATA } else { *v })
            .collect();
        img.write_data(&out).map_err(tiff_err)?;
        Ok(())
    }

    /// Reads a GeoTIFF, or an ESRI ASCII grid (`.asc`, as FUSION's DTM2ASCII writes).
    pub fn read(path: &Path) -> Result<Self> {
        let asc = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("asc"));
        if asc {
            Self::read_asc(path)
        } else {
            Self::read_tiff(path)
        }
    }

    /// Reads an ESRI ASCII grid.
    pub fn read_asc(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut words = text.split_ascii_whitespace();
        let (mut cols, mut rows, mut x, mut y, mut cell, mut nodata, mut center) =
            (0usize, 0usize, 0.0, 0.0, 0.0, None, false);
        let bad = |w: &str| {
            Error::Format(format!(
                "{}: bad ASCII grid header near '{w}'",
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
                "{}: {n} values for a {cols} x {rows} grid",
                path.display()
            )));
        }
        Ok(g)
    }

    /// The EPSG code a GeoTIFF declares (projected, else geographic), if any.
    pub fn read_epsg(path: &Path) -> Option<u16> {
        use tiff::decoder::Decoder;
        let mut d = Decoder::new(BufReader::new(File::open(path).ok()?)).ok()?;
        let keys = d.get_tag_u32_vec(Tag::GeoKeyDirectoryTag).ok()?;
        // four numbers of header, then four per key: id, location, count, value
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

    /// Reads a north-up 32-bit float GeoTIFF such as [`Grid::write_tiff`] writes.
    pub fn read_tiff(path: &Path) -> Result<Self> {
        use tiff::decoder::{Decoder, DecodingResult};
        let mut d = Decoder::new(BufReader::new(File::open(path)?)).map_err(tiff_err)?;
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
                "{}: not a GeoTIFF (no pixel scale or tie point)",
                path.display()
            )));
        };
        if (sx - sy).abs() > sx * 1e-9
            || tie.first().copied().unwrap_or(0.0) != 0.0
            || tie.get(1).copied().unwrap_or(0.0) != 0.0
        {
            return Err(Error::Unsupported(format!(
                "{}: only square cells tied at the corner pixel are read",
                path.display()
            )));
        }
        let (cols, rows) = (w as usize, h as usize);
        let mut g = Self::new(tx, ty - rows as f64 * sy, sx, cols, rows)?;
        let data = match d.read_image().map_err(tiff_err)? {
            DecodingResult::F32(v) => v,
            DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
            _ => {
                return Err(Error::Unsupported(format!(
                    "{}: only float rasters are read",
                    path.display()
                )));
            }
        };
        if data.len() != g.data.len() {
            return Err(Error::Format(format!(
                "{}: one band of {cols} x {rows} floats expected",
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

/// Accumulates the mean elevation of the points falling in each cell of a grid; sums are kept
/// relative to a reference height for precision.
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

    /// The grid of means; cells without points stay empty.
    pub fn finish(mut self) -> Grid {
        for ((v, s), n) in self.grid.data.iter_mut().zip(&self.sum).zip(&self.count) {
            if *n > 0 {
                *v = (f64::from(*s / *n as f32) + self.zref) as f32;
            }
        }
        self.grid
    }
}

/// `n` copies of `v`, or an error (not an abort) when the memory is not there.
fn filled<T: Copy>(n: usize, v: T) -> Result<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| {
        Error::Param(
            "not enough memory for this grid: use a bigger cell size or split the area".into(),
        )
    })?;
    out.resize(n, v);
    Ok(out)
}

fn tiff_err(e: tiff::TiffError) -> Error {
    Error::Format(format!("GeoTIFF: {e}"))
}

fn too_big() -> Error {
    Error::Param("grid too large for a TIFF".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covering_aligns_and_indexes() -> Result<()> {
        let g = Grid::covering([10.2, 20.7, 13.9, 22.0], 1.0)?;
        assert_eq!((g.x0, g.y0, g.cols, g.rows), (10.0, 20.0, 4, 3));
        // The southwest point is in the last row.
        assert_eq!(g.col_row(10.2, 20.7), Some((0, 2)));
        assert_eq!(g.col_row(13.9, 22.0), Some((3, 0)));
        assert_eq!(g.col_row(9.9, 21.0), None);
        assert_eq!(g.center(0, 2), (10.5, 20.5));
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
        Ok(())
    }

    #[test]
    fn tiff_round_trip() -> Result<()> {
        let mut g = Grid::new(277750.0, 6122250.0, 2.0, 3, 2)?;
        g.data = vec![1.0, f32::NAN, 3.0, 4.0, 5.0, 6.5];
        let p = std::env::temp_dir().join("rustlidar_grid_test.tif");
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
