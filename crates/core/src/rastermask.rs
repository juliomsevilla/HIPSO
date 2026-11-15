//! Clip a raster with a mask: the cells outside the polygons of a layer (or the cells a mask
//! raster leaves empty or at zero) become empty, and the raster can be cut to the box of the
//! mask. The cells are those of the input: nothing is resampled. Rows are read, masked and
//! written in blocks, so memory follows a block whatever the size of the raster.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::pyramid::Pyramid;
use crate::rastertools::Dtype;
use crate::vector::{Region, RegionIndex};
use crate::{Error, Result};

/// What keeps the cells.
#[derive(Clone, Debug)]
pub enum Mask {
    /// The cells whose centre is inside any of these regions (in the raster's system: see
    /// [`regions_of`]).
    Regions(Vec<Region>),
    /// The cells where this raster has data other than zero (its nearest cell; moved to the
    /// system of the input if it is in another).
    Raster(PathBuf),
}

#[derive(Clone, Debug)]
pub struct MaskJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub mask: Mask,
    /// Cut the result to the box of the mask (a block of whole cells of the input).
    pub crop: bool,
    /// Keep what is outside the mask instead of what is inside.
    pub invert: bool,
    /// What the cells left out hold; the file's own value, else 0 for 8 and 16 bits and -9999 for
    /// decimals, when `None`.
    pub nodata: Option<f32>,
    pub dtype: Option<Dtype>,
    pub compression: Option<crate::tiled::Compression>,
}

impl MaskJob {
    pub fn new(input: PathBuf, output: PathBuf, mask: Mask) -> Self {
        Self {
            input,
            output,
            mask,
            crop: true,
            invert: false,
            nodata: None,
            dtype: None,
            compression: None,
        }
    }
}

/// The polygons of a layer, moved to the system `raster_epsg` if the layer declares another.
pub fn regions_of(layer: &std::path::Path, raster_epsg: Option<u16>) -> Result<Vec<Region>> {
    let mut list = crate::vector::read_regions(layer, &crate::vector::ReadOptions::default())?;
    if list.is_empty() {
        return Err(Error::Param(format!("«{}» no tiene polígonos", layer.display())));
    }
    if let (Some(a), Some(b)) = (crate::crs::epsg_of(layer), raster_epsg)
        && a != b
    {
        list = crate::reproject::regions(&list, a, b)?;
    }
    Ok(list)
}

/// The mask as something that answers for a cell centre.
enum Keeper<'a> {
    Regions(RegionIndex<'a>),
    Raster {
        /// Tells this raster's strips apart from those of another job in a thread's cache.
        id: usize,
        pyr: Pyramid,
        /// Transformer from the input's system to the mask's, when they differ.
        tf: Option<crate::reproject::Transformer>,
        nodata: Option<f32>,
    },
}

/// Clips the raster (all its bands). Returns a line for the report.
pub fn clip(job: &MaskJob) -> Result<String> {
    let pyr = Pyramid::open(&job.input)?;
    let nb = pyr.bands;
    if nb == 0 || nb > crate::rastertools::MAX_BANDS {
        return Err(Error::Param(format!(
            "{nb} bandas: una salida admite de 1 a {}",
            crate::rastertools::MAX_BANDS
        )));
    }
    let (cols, rows) = (pyr.levels[0].cols as usize, pyr.levels[0].rows as usize);
    let cell = pyr.cell;
    let e = pyr.extent();
    let regions: Vec<Region> = match &job.mask {
        Mask::Regions(r) => r.clone(),
        Mask::Raster(_) => Vec::new(),
    };
    let keeper = match &job.mask {
        Mask::Regions(r) => {
            if r.is_empty() {
                return Err(Error::Param("la máscara no tiene polígonos".into()));
            }
            Keeper::Regions(RegionIndex::new(&regions))
        }
        Mask::Raster(p) => {
            let m = Pyramid::open(p)?;
            let tf = match (pyr.epsg, m.epsg) {
                (Some(a), Some(b)) if a != b => Some(crate::reproject::transformer(a, b)?),
                _ => None,
            };
            let nodata = m.nodata();
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Keeper::Raster { id, pyr: m, tf, nodata }
        }
    };
    // the box of the mask in the input's system, to cut the result to it
    let mbox = match &keeper {
        Keeper::Regions(ix) => ix.bbox(),
        Keeper::Raster { pyr: m, tf, .. } => {
            let b = m.extent();
            match tf {
                // the box of the mask raster moved back is not what the input needs: the
                // cells are tested one by one, so only the input's own box is cut to
                Some(_) => e,
                None => b,
            }
        }
    };
    // the window of whole input cells that the result covers
    let (c0, c1, r0, r1) = if job.crop && !job.invert {
        let c0 = (((mbox[0] - e[0]) / cell).floor().max(0.0) as usize).min(cols);
        let c1 = (((mbox[2] - e[0]) / cell).ceil().max(0.0) as usize).min(cols);
        let r0 = (((e[3] - mbox[3]) / cell).floor().max(0.0) as usize).min(rows);
        let r1 = (((e[3] - mbox[1]) / cell).ceil().max(0.0) as usize).min(rows);
        (c0, c1, r0, r1)
    } else {
        (0, cols, 0, rows)
    };
    if c1 <= c0 || r1 <= r0 {
        return Err(Error::Param(
            "la máscara no se solapa con el ráster (¿están en el mismo sistema de coordenadas?)".into(),
        ));
    }
    let (ocols, orows) = (c1 - c0, r1 - r0);
    let dtype = job.dtype.unwrap_or(match pyr.bits {
        0..=8 => Dtype::U8,
        9..=16 => Dtype::U16,
        _ => Dtype::F32,
    });
    let nodata = job.nodata.or(pyr.nodata()).unwrap_or(match dtype {
        Dtype::F32 => -9999.0,
        _ => 0.0,
    });
    let declared = pyr.nodata();
    let step = (16_000_000 / (ocols * nb).max(1)).clamp(4, 4096);
    let spec = crate::tiled::Spec {
        cols: ocols,
        rows: orows,
        bands: nb,
        dtype,
        cell,
        x0: pyr.x0 + c0 as f64 * cell,
        top: pyr.y_top - r0 as f64 * cell,
        epsg: pyr.epsg,
        nodata,
        compression: job
            .compression
            .unwrap_or_else(|| crate::tiled::auto_compression(dtype, nb)),
        nearest: nb == 1 && dtype != Dtype::F32,
        bigtiff: false,
    };
    crate::paths::ensure_parent(&job.output)?;
    crate::progress::begin(orows as u64);
    let kept = std::sync::atomic::AtomicU64::new(0);
    let invert = job.invert;
    crate::tiled::write(&job.output, spec, step, |a, b| {
        crate::progress::advance((b - a) as u64);
        let img = pyr.read_rows(r0 + a, r0 + b)?;
        if img.cols != cols || img.rows != b - a {
            return Err(Error::Format("el bloque leído no tiene el tamaño esperado".into()));
        }
        let top = pyr.y_top - (r0 + a) as f64 * cell;
        // the mask over the block: one flag per cell of the window
        let keep: Vec<bool> = (0..b - a)
            .into_par_iter()
            .flat_map_iter(|r| {
                let y = top - (r as f64 + 0.5) * cell;
                let keeper = &keeper;
                (c0..c1).map(move |c| {
                    let x = pyr.x0 + (c as f64 + 0.5) * cell;
                    let hit = match keeper {
                        Keeper::Regions(ix) => ix.any(x, y),
                        Keeper::Raster { id, pyr: m, tf, nodata } => {
                            let (mx, my) = match tf {
                                Some(t) => match t.transform(x, y) {
                                    Ok(p) => p,
                                    Err(_) => return false,
                                },
                                None => (x, y),
                            };
                            mask_cell(*id, m, *nodata, mx, my)
                        }
                    };
                    hit != invert
                })
            })
            .collect();
        let mut out = vec![f32::NAN; (b - a) * ocols * nb];
        let n: u64 = out
            .par_chunks_mut(ocols * nb)
            .enumerate()
            .map(|(r, line)| {
                let mut n = 0u64;
                for c in 0..ocols {
                    if !keep[r * ocols + c] {
                        continue;
                    }
                    n += 1;
                    for (k, band) in img.bands.iter().enumerate() {
                        let v = band[r * cols + c0 + c];
                        line[c * nb + k] = if declared == Some(v) { f32::NAN } else { v };
                    }
                }
                n
            })
            .sum();
        kept.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        Ok(out)
    })?;
    let kept = kept.into_inner();
    Ok(format!(
        "{}: {} x {} celdas, {:.1} % dentro de la máscara → {}",
        crate::paths::stem(&job.input),
        ocols,
        orows,
        100.0 * kept as f64 / (ocols as f64 * orows as f64).max(1.0),
        job.output.display()
    ))
}

thread_local! {
    /// The last block of the mask raster read by this thread: (raster id, [strip, first row], image).
    static CACHE: std::cell::RefCell<Option<(usize, [usize; 2], crate::raster::Image)>> =
        const { std::cell::RefCell::new(None) };
}

/// Whether the mask raster has data (not empty, not zero) under `(x, y)`: its nearest cell.
fn mask_cell(id: usize, m: &Pyramid, nodata: Option<f32>, x: f64, y: f64) -> bool {
    let l = m.levels[0];
    let (fc, fr) = ((x - m.x0) / m.cell, (m.y_top - y) / m.cell);
    if fc < 0.0 || fr < 0.0 || fc >= f64::from(l.cols) || fr >= f64::from(l.rows) {
        return false;
    }
    let (c, r) = (fc as usize, fr as usize);
    // a strip of 256 rows at a time per thread
    let strip = r / 256;
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let have = cache
            .as_ref()
            .is_some_and(|(i, w, _)| *i == id && w[0] == strip);
        if !have {
            let (a, b) = (strip * 256, ((strip + 1) * 256).min(l.rows as usize));
            match m.read_rows(a, b) {
                Ok(img) => *cache = Some((id, [strip, a], img)),
                Err(_) => return false,
            }
        }
        let Some((_, w, img)) = cache.as_ref() else {
            return false;
        };
        let v = img.bands[0][(r - w[1]) * img.cols + c];
        v.is_finite() && v != 0.0 && nodata != Some(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::vector::{Polygon, Shape2};

    fn square(name: &str, b: [f64; 4]) -> Region {
        Region {
            name: name.into(),
            shape: Shape2::Polygon(Polygon::new(vec![vec![
                [b[0], b[1]],
                [b[2], b[1]],
                [b[2], b[3]],
                [b[0], b[3]],
                [b[0], b[1]],
            ]])),
            attrs: vec![],
        }
    }

    #[test]
    fn cells_outside_the_polygon_are_emptied_and_the_result_is_cut_to_it() -> Result<()> {
        let d = std::env::temp_dir().join(format!("rl-mask-{}", std::process::id()));
        std::fs::create_dir_all(&d)?;
        let mut g = Grid::new(100.0, 200.0, 1.0, 40, 30)?;
        g.data.fill(7.0);
        g.write_tiff(&d.join("a.tif"), Some(25830))?;
        let m = Mask::Regions(vec![square("a", [110.2, 205.2, 120.0, 215.0])]);
        // cut: cells whose centre is inside, over the whole box of whole cells
        let job = MaskJob::new(d.join("a.tif"), d.join("c.tif"), m.clone());
        let line = clip(&job)?;
        let c = Grid::read(&d.join("c.tif"))?;
        // x 110..120 (10 cells), y 205..215 (10 cells); the first column's centre 110.5 is in
        assert_eq!((c.cols, c.rows, c.x0, c.y0), (10, 10, 110.0, 205.0), "{line}");
        assert!(c.data.iter().all(|v| *v == 7.0 || v.is_nan()));
        assert!(c.data.iter().filter(|v| **v == 7.0).count() > 80);
        // no cut: the same size, the outside empty
        let mut full = MaskJob::new(d.join("a.tif"), d.join("f.tif"), m.clone());
        full.crop = false;
        clip(&full)?;
        let f = Grid::read(&d.join("f.tif"))?;
        assert_eq!((f.cols, f.rows), (40, 30));
        assert!(f.data[0].is_nan());
        let inside = f.data.iter().filter(|v| **v == 7.0).count();
        // inverted: what was inside is empty and the rest kept
        let mut inv = MaskJob::new(d.join("a.tif"), d.join("i.tif"), m);
        inv.invert = true;
        clip(&inv)?;
        let i = Grid::read(&d.join("i.tif"))?;
        assert_eq!(i.data.iter().filter(|v| **v == 7.0).count(), 1200 - inside);
        // a raster mask: the cells where it is not zero
        let mut mk = Grid::new(100.0, 200.0, 2.0, 20, 15)?;
        for r in 0..15 {
            for c in 0..20 {
                mk.data[r * 20 + c] = if c < 10 { 1.0 } else { 0.0 };
            }
        }
        mk.write_tiff(&d.join("m.tif"), Some(25830))?;
        let mut rj = MaskJob::new(d.join("a.tif"), d.join("r.tif"), Mask::Raster(d.join("m.tif")));
        rj.crop = false;
        clip(&rj)?;
        let r = Grid::read(&d.join("r.tif"))?;
        assert_eq!(r.data[0], 7.0);
        assert!(r.data[30].is_nan());
        // a mask that does not touch the raster is an error
        let far = Mask::Regions(vec![square("z", [1000.0, 1000.0, 1010.0, 1010.0])]);
        assert!(clip(&MaskJob::new(d.join("a.tif"), d.join("z.tif"), far)).is_err());
        std::fs::remove_dir_all(&d)?;
        Ok(())
    }
}
