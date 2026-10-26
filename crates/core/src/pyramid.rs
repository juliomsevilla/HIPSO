//! Windowed reading of large GeoTIFFs: the file's own overviews (reduced-resolution images stored
//! after the main one) are the levels of a pyramid, and only the tiles of the window asked for are
//! decoded. A 90 000 x 60 000 aerial photo shows at once at the zoom that fits the screen and
//! sharpens as one zooms in, with memory that depends on the window, not on the file.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tiff::ColorType;
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;

use crate::jp2::{self, Jp2};
use crate::raster::Image;
use crate::{Error, Result};

fn tiff_err(e: tiff::TiffError) -> Error {
    Error::Format(format!("TIFF: {e}{}", crate::cloud_hint(&e.to_string())))
}

/// One resolution of the file: the main image (`ifd` 0) or an overview.
#[derive(Clone, Copy, Debug)]
pub struct Level {
    pub ifd: usize,
    pub cols: u32,
    pub rows: u32,
}

/// A GeoTIFF opened for windowed reads. Cheap to clone and share between threads.
#[derive(Clone, Debug)]
pub struct Pyramid {
    path: PathBuf,
    /// Finest first.
    pub levels: Vec<Level>,
    /// West edge, north edge and cell size of the main image.
    pub x0: f64,
    pub y_top: f64,
    pub cell: f64,
    pub bands: usize,
    /// Bits of each sample (8 for ordinary photos).
    pub bits: u8,
    /// The EPSG code of the projected or geographic system, when the file says it.
    pub epsg: Option<u16>,
    planar: bool,
    ycbcr: bool,
    nodata: Option<f32>,
    /// Set when the file is JPEG 2000: the levels are then its wavelet reductions (`Level::ifd`
    /// holds the number of halvings).
    jp2: Option<Jp2>,
    /// Set for the formats that are read whole (PNG, JPEG, ENVI, HGT).
    mem: Option<Arc<Image>>,
}

/// Whether the file starts like a JPEG 2000 file (whatever its extension).
pub fn is_jp2_file(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 12];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|()| jp2::is_jp2(&head))
}

fn open_decoder(path: &Path) -> Result<Decoder<BufReader<File>>> {
    Ok(Decoder::new(BufReader::new(crate::open_file(path)?))
        .map_err(tiff_err)?
        .with_limits(Limits::unlimited()))
}

fn bit_depth(c: ColorType) -> u8 {
    match c {
        ColorType::Gray(b)
        | ColorType::RGB(b)
        | ColorType::Palette(b)
        | ColorType::GrayA(b)
        | ColorType::RGBA(b)
        | ColorType::CMYK(b)
        | ColorType::CMYKA(b)
        | ColorType::YCbCr(b)
        | ColorType::Lab(b) => b,
        ColorType::Multiband { bit_depth, .. } => bit_depth,
        _ => 8,
    }
}

fn to_f32(buf: DecodingResult) -> Option<Vec<f32>> {
    Some(match buf {
        DecodingResult::U8(v) => v.into_iter().map(f32::from).collect(),
        DecodingResult::U16(v) => v.into_iter().map(f32::from).collect(),
        DecodingResult::U32(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I16(v) => v.into_iter().map(f32::from).collect(),
        DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::F32(v) => v,
        DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        _ => return None,
    })
}

/// The JPEG-in-TIFF decoder hands YCbCr samples over as they are; this makes them RGB (in place,
/// for interleaved triplets).
pub fn ycbcr_to_rgb(px: &mut [f32]) {
    for t in px.as_chunks_mut::<3>().0.iter_mut() {
        let (y, cb, cr) = (t[0], t[1] - 128.0, t[2] - 128.0);
        t[0] = (y + 1.402 * cr).clamp(0.0, 255.0);
        t[1] = (y - 0.344_136 * cb - 0.714_136 * cr).clamp(0.0, 255.0);
        t[2] = (y + 1.772 * cb).clamp(0.0, 255.0);
    }
}

/// The part of the output that one decoded tile fills: its corner and size in output pixels,
/// samples per pixel and values.
struct Piece {
    plane: usize,
    ox: usize,
    oy: usize,
    ow: usize,
    oh: usize,
    nb: usize,
    data: Vec<f32>,
}

impl Pyramid {
    /// The value the file declares as "no data", if any.
    pub fn nodata(&self) -> Option<f32> {
        self.nodata
    }

    pub fn open(path: &Path) -> Result<Self> {
        crate::check_raster_format(path)?;
        if is_jp2_file(path) {
            return Self::open_jp2(path);
        }
        if let Some(l) = crate::rasterfmt::load(path)? {
            return Ok(Self::from_memory(path, l));
        }
        let mut d = open_decoder(path)?;
        let (w, h) = d.dimensions().map_err(tiff_err)?;
        let scale = d
            .get_tag_f64_vec(Tag::ModelPixelScaleTag)
            .map_err(tiff_err)?;
        let tie = d.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(tiff_err)?;
        let (Some(&sx), Some(&tx), Some(&ty)) = (scale.first(), tie.get(3), tie.get(4)) else {
            return Err(Error::Format(format!(
                "{}: no es un GeoTIFF (sin escala de píxel ni punto de anclaje)",
                path.display()
            )));
        };
        let spp = d.get_tag_u32(Tag::SamplesPerPixel).unwrap_or(1).max(1) as usize;
        let planar = d.get_tag_u32(Tag::PlanarConfiguration).unwrap_or(1) == 2;
        let colortype = d.colortype().map_err(tiff_err)?;
        let nodata = d
            .get_tag_ascii_string(Tag::GdalNodata)
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok());
        // GeoKeyDirectory: a header of four numbers, then four per key (id, location, count,
        // value); 3072 is the projected system and 2048 the geographic one
        let epsg = d
            .get_tag_u32_vec(Tag::GeoKeyDirectoryTag)
            .ok()
            .and_then(|k| {
                let key = |id: u32| {
                    k.get(4..)?
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .find(|e| e[0] == id && e[1] == 0)
                        .and_then(|e| u16::try_from(e[3]).ok())
                };
                key(3072).or_else(|| key(2048))
            });
        let mut levels = vec![Level {
            ifd: 0,
            cols: w,
            rows: h,
        }];
        let mut ifd = 0;
        while d.more_images() && d.next_image().is_ok() {
            ifd += 1;
            let (cw, ch) = d.dimensions().map_err(tiff_err)?;
            // bit 2 of the subfile type marks a transparency mask, not a picture
            let masked = d.get_tag_u32(Tag::NewSubfileType).unwrap_or(0) & 4 != 0;
            let same = d.get_tag_u32(Tag::SamplesPerPixel).unwrap_or(1) as usize == spp;
            if !masked && same && cw < w && ch < h {
                levels.push(Level {
                    ifd,
                    cols: cw,
                    rows: ch,
                });
            }
        }
        levels.sort_by_key(|l| std::cmp::Reverse(l.cols));
        Ok(Self {
            path: path.to_path_buf(),
            levels,
            x0: tx,
            y_top: ty,
            cell: sx,
            bands: spp,
            bits: bit_depth(colortype),
            epsg,
            planar,
            ycbcr: matches!(colortype, ColorType::YCbCr(_)) && spp == 3,
            nodata,
            jp2: None,
            mem: None,
        })
    }

    fn from_memory(path: &Path, l: crate::rasterfmt::Loaded) -> Self {
        let img = l.image;
        Self {
            path: path.to_path_buf(),
            levels: vec![Level {
                ifd: 0,
                cols: img.cols as u32,
                rows: img.rows as u32,
            }],
            x0: img.x0,
            y_top: img.y0 + img.rows as f64 * img.cell,
            cell: img.cell,
            bands: img.bands.len(),
            bits: l.bits,
            epsg: l.epsg,
            planar: false,
            ycbcr: false,
            nodata: None,
            jp2: None,
            mem: Some(Arc::new(img)),
        }
    }

    fn open_jp2(path: &Path) -> Result<Self> {
        let j = Jp2::open(path)?;
        let geo = j.geo.ok_or_else(|| {
            Error::Format(format!(
                "{}: JPEG 2000 sin georreferenciar (ni GMLJP2 ni GeoJP2)",
                path.display()
            ))
        })?;
        let levels = (0..=j.levels)
            .map(|r| {
                let (w, h) = j.size_at(r);
                Level {
                    ifd: r as usize,
                    cols: w as u32,
                    rows: h as u32,
                }
            })
            .filter(|l| l.cols > 0 && l.rows > 0)
            .collect();
        Ok(Self {
            path: path.to_path_buf(),
            levels,
            x0: geo.x0,
            y_top: geo.y_top,
            cell: geo.cell,
            bands: j.bands,
            bits: j.depth,
            epsg: geo.epsg,
            planar: false,
            ycbcr: false,
            nodata: None,
            jp2: Some(j),
            mem: None,
        })
    }

    /// West, south, east, north.
    pub fn extent(&self) -> [f64; 4] {
        let l = self.levels[0];
        [
            self.x0,
            self.y_top - f64::from(l.rows) * self.cell,
            self.x0 + f64::from(l.cols) * self.cell,
            self.y_top,
        ]
    }

    /// Ground size of one pixel of a level.
    pub fn cell_at(&self, level: usize) -> f64 {
        self.cell * f64::from(self.levels[0].cols)
            / f64::from(self.levels[level.min(self.levels.len() - 1)].cols)
    }

    /// The coarsest level that still has a pixel at least as small as `map_per_px`.
    pub fn level_for(&self, map_per_px: f64) -> usize {
        (0..self.levels.len())
            .rev()
            .find(|&k| self.cell_at(k) <= map_per_px * 1.0001)
            .unwrap_or(0)
    }

    /// Decodes the part of `level` that lies in `win` (west, south, east, north), whole tiles at
    /// a time, on all cores. The image returned starts at a pixel boundary of the level.
    pub fn read(&self, level: usize, win: [f64; 4]) -> Result<Image> {
        self.read_step(level, win, 1)
    }

    /// As [`Pyramid::read`], keeping one pixel in every `step` along each axis (the cell of the
    /// image returned is `step` times the level's). Each tile is thinned as soon as it is decoded,
    /// so memory follows the output, not the file.
    pub fn read_step(&self, level: usize, win: [f64; 4], step: usize) -> Result<Image> {
        let step = step.max(1);
        let level = level.min(self.levels.len() - 1);
        let lv = self.levels[level];
        let cell = self.cell_at(level);
        let clamp = |v: f64, hi: u32| v.max(0.0).min(f64::from(hi)) as usize;
        let c0 = clamp(((win[0] - self.x0) / cell).floor(), lv.cols);
        let c1 = clamp(((win[2] - self.x0) / cell).ceil(), lv.cols);
        let r0 = clamp(((self.y_top - win[3]) / cell).floor(), lv.rows);
        let r1 = clamp(((self.y_top - win[1]) / cell).ceil(), lv.rows);
        if c1 <= c0 || r1 <= r0 {
            return Err(Error::Param("la ventana queda fuera de la imagen".into()));
        }
        let (cols, rows) = ((c1 - c0).div_ceil(step), (r1 - r0).div_ceil(step));
        if let Some(m) = &self.mem {
            let bands = m
                .bands
                .iter()
                .map(|b| {
                    let mut v = Vec::with_capacity(cols * rows);
                    for oy in 0..rows {
                        let row = (r0 + oy * step) * m.cols;
                        v.extend((0..cols).map(|ox| b[row + c0 + ox * step]));
                    }
                    v
                })
                .collect();
            return Ok(Image {
                x0: self.x0 + c0 as f64 * cell,
                y0: self.y_top - (r0 + rows * step) as f64 * cell,
                cell: cell * step as f64,
                cols,
                rows,
                bands,
            });
        }
        if let Some(j) = &self.jp2 {
            let bands = j.read(lv.ifd as u32, (c0, r0, c1, r1), step)?;
            return Ok(Image {
                x0: self.x0 + c0 as f64 * cell,
                y0: self.y_top - (r0 + rows * step) as f64 * cell,
                cell: cell * step as f64,
                cols,
                rows,
                bands,
            });
        }
        let (cw, ch) = {
            let mut d = open_decoder(&self.path)?;
            d.seek_to_image(lv.ifd).map_err(tiff_err)?;
            d.chunk_dimensions()
        };
        let (cw, ch) = (cw.max(1) as usize, ch.max(1) as usize);
        let across = (lv.cols as usize).div_ceil(cw);
        let down = (lv.rows as usize).div_ceil(ch);
        let planes = if self.planar { self.bands } else { 1 };
        let mut ids: Vec<u32> = Vec::new();
        for p in 0..planes {
            for ty in r0 / ch..=(r1 - 1) / ch {
                for tx in c0 / cw..=(c1 - 1) / cw {
                    ids.push((p * across * down + ty * across + tx) as u32);
                }
            }
        }
        let parts: Vec<Result<Vec<Piece>>> = ids
            .par_chunks(8)
            .map(|group| {
                let mut d = open_decoder(&self.path)?;
                d.seek_to_image(lv.ifd).map_err(tiff_err)?;
                let mut out = Vec::with_capacity(group.len());
                for &idx in group {
                    let k = idx as usize % (across * down);
                    let (tx, ty) = ((k % across) * cw, (k / across) * ch);
                    let (dw, dh) = d.chunk_data_dimensions(idx);
                    let (dw, dh) = (dw as usize, dh as usize);
                    let mut data = to_f32(d.read_chunk(idx).map_err(tiff_err)?)
                        .ok_or_else(|| Error::Unsupported("este tipo de dato no se lee".into()))?;
                    let nb = if self.planar { 1 } else { self.bands };
                    if data.len() < dw * dh * nb {
                        return Err(Error::Format("una tesela de la imagen es más pequeña de lo que declara (archivo dañado)".into()));
                    }
                    if self.ycbcr {
                        ycbcr_to_rgb(&mut data);
                    }
                    // the output pixels whose source falls in this tile
                    let first =
                        |from: usize, origin: usize| from.saturating_sub(origin).div_ceil(step);
                    let (ox0, oy0) = (first(tx, c0), first(ty, r0));
                    let ox1 = (tx + dw)
                        .min(c1)
                        .saturating_sub(c0)
                        .div_ceil(step)
                        .min(cols);
                    let oy1 = (ty + dh)
                        .min(r1)
                        .saturating_sub(r0)
                        .div_ceil(step)
                        .min(rows);
                    if ox1 <= ox0 || oy1 <= oy0 {
                        continue;
                    }
                    let (ow, oh) = (ox1 - ox0, oy1 - oy0);
                    let mut piece = Vec::with_capacity(ow * oh * nb);
                    for oy in oy0..oy1 {
                        let sy = r0 + oy * step - ty;
                        for ox in ox0..ox1 {
                            let sx = c0 + ox * step - tx;
                            let at = (sy * dw + sx) * nb;
                            piece.extend_from_slice(&data[at..at + nb]);
                        }
                    }
                    out.push(Piece {
                        plane: idx as usize / (across * down),
                        ox: ox0,
                        oy: oy0,
                        ow,
                        oh,
                        nb,
                        data: piece,
                    });
                }
                Ok(out)
            })
            .collect();
        let mut bands = vec![vec![f32::NAN; cols * rows]; self.bands];
        for part in parts {
            for p in part? {
                for y in 0..p.oh {
                    for x in 0..p.ow {
                        let dst = (p.oy + y) * cols + p.ox + x;
                        for b in 0..p.nb {
                            let v = p.data[(y * p.ow + x) * p.nb + b];
                            bands[p.plane + b][dst] = if Some(v) == self.nodata || !v.is_finite() {
                                f32::NAN
                            } else {
                                v
                            };
                        }
                    }
                }
            }
        }
        Ok(Image {
            x0: self.x0 + c0 as f64 * cell,
            y0: self.y_top - (r0 + rows * step) as f64 * cell,
            cell: cell * step as f64,
            cols,
            rows,
            bands,
        })
    }

    /// The whole image with at most about `max_px` pixels: the finest level that fits, or the
    /// coarsest one thinned. Returns the image and its reduction against the file's resolution.
    pub fn overview(&self, max_px: u64) -> Result<(Image, usize)> {
        let fits = self
            .levels
            .iter()
            .position(|l| u64::from(l.cols) * u64::from(l.rows) <= max_px);
        let (k, step) = match fits {
            Some(k) => (k, 1),
            None => {
                let k = self.levels.len() - 1;
                let l = self.levels[k];
                let px = u64::from(l.cols) * u64::from(l.rows);
                (
                    k,
                    ((px as f64 / max_px as f64).sqrt().ceil() as usize).max(1),
                )
            }
        };
        let img = self.read_step(k, self.extent(), step)?;
        let reduction = (img.cell / self.cell).round().max(1.0) as usize;
        Ok((img, reduction))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ycbcr_grey_and_red() {
        let mut p = [128.0, 128.0, 128.0, 76.0, 85.0, 255.0];
        ycbcr_to_rgb(&mut p);
        assert!((p[0] - 128.0).abs() < 1.0 && (p[1] - 128.0).abs() < 1.0);
        // pure red in YCbCr is (76, 85, 255)
        assert!(p[3] > 250.0 && p[4] < 5.0 && p[5] < 5.0);
    }

    /// Writes a window of the file named by `RUSTLIDAR_IMAGE` as `RUSTLIDAR_OUT` (a PNG) and
    /// prints the levels (`-- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn sample_window() -> Result<()> {
        let (Some(p), Some(out)) = (
            std::env::var_os("RUSTLIDAR_IMAGE"),
            std::env::var_os("RUSTLIDAR_OUT"),
        ) else {
            return Ok(());
        };
        let t = std::time::Instant::now();
        let pyr = Pyramid::open(Path::new(&p))?;
        println!(
            "levels {:?} cell {} bands {} bits {} epsg {:?} ycbcr {} open {:?}",
            pyr.levels
                .iter()
                .map(|l| (l.cols, l.rows))
                .collect::<Vec<_>>(),
            pyr.cell,
            pyr.bands,
            pyr.bits,
            pyr.epsg,
            pyr.ycbcr,
            t.elapsed()
        );
        let e = pyr.extent();
        let (cx, cy) = ((e[0] + e[2]) / 2.0, (e[1] + e[3]) / 2.0);
        let half = 150.0;
        let t = std::time::Instant::now();
        let img = pyr.read(
            pyr.level_for(0.3),
            [cx - half, cy - half, cx + half, cy + half],
        )?;
        println!(
            "window {}x{} cell {} in {:?}",
            img.cols,
            img.rows,
            img.cell,
            t.elapsed()
        );
        let mut rgb = Vec::with_capacity(img.cols * img.rows * 3);
        for i in 0..img.cols * img.rows {
            for b in 0..3 {
                rgb.push(img.bands[b][i].clamp(0.0, 255.0) as u8);
            }
        }
        let f = File::create(out)?;
        let mut enc =
            png::Encoder::new(std::io::BufWriter::new(f), img.cols as u32, img.rows as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc
            .write_header()
            .map_err(|e| Error::Format(e.to_string()))?;
        w.write_image_data(&rgb)
            .map_err(|e| Error::Format(e.to_string()))?;
        Ok(())
    }
}
