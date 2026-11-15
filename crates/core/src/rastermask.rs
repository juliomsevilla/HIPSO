//! Recorta un ráster con una máscara: las celdas fuera de los polígonos de una capa (o las celdas
//! que un ráster máscara deja vacías o a cero) quedan vacías, y el ráster se puede cortar a la caja
//! de la máscara. Las celdas son las de la entrada: no se remuestrea nada. Las filas se leen, se
//! enmascaran y se escriben por bloques, de modo que la memoria sigue a un bloque sea cual sea el
//! tamaño del ráster.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::pyramid::Pyramid;
use crate::rastertools::Dtype;
use crate::vector::{Region, RegionIndex};
use crate::{Error, Result};

/// Qué conserva las celdas.
#[derive(Clone, Debug)]
pub enum Mask {
    /// Las celdas cuyo centro está dentro de alguna de estas regiones (en el sistema del ráster:
    /// véase [`regions_of`]).
    Regions(Vec<Region>),
    /// Las celdas donde este ráster tiene datos distintos de cero (su celda más cercana; llevado al
    /// sistema de la entrada si está en otro).
    Raster(PathBuf),
}

#[derive(Clone, Debug)]
pub struct MaskJob {
    pub input: PathBuf,
    pub output: PathBuf,
    pub mask: Mask,
    /// Corta el resultado a la caja de la máscara (un bloque de celdas enteras de la entrada).
    pub crop: bool,
    /// Conserva lo que queda fuera de la máscara en lugar de lo que queda dentro.
    pub invert: bool,
    /// Lo que contienen las celdas dejadas fuera; el valor propio del archivo o, si no, 0 para 8 y
    /// 16 bits y -9999 para decimales, cuando `None`.
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

/// Los polígonos de una capa, llevados al sistema `raster_epsg` si la capa declara otro.
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

/// La máscara como algo que responde por el centro de una celda.
enum Keeper<'a> {
    Regions(RegionIndex<'a>),
    Raster {
        /// Distingue las franjas de este ráster de las de otro trabajo en la caché de un hilo.
        id: usize,
        pyr: Pyramid,
        /// Transformador del sistema de la entrada al de la máscara, cuando difieren.
        tf: Option<crate::reproject::Transformer>,
        nodata: Option<f32>,
    },
}

/// Recorta el ráster (todas sus bandas). Devuelve una línea para el informe.
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
    // la caja de la máscara en el sistema de la entrada, para cortar el resultado a ella
    let mbox = match &keeper {
        Keeper::Regions(ix) => ix.bbox(),
        Keeper::Raster { pyr: m, tf, .. } => {
            let b = m.extent();
            match tf {
                // la caja del ráster máscara llevada de vuelta no es lo que necesita la entrada:
                // las celdas se comprueban una a una, así que solo se corta a la caja propia de la
                // entrada
                Some(_) => e,
                None => b,
            }
        }
    };
    // la ventana de celdas enteras de la entrada que cubre el resultado
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
        // la máscara sobre el bloque: un indicador por celda de la ventana
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
    /// El último bloque del ráster máscara leído por este hilo: (identificador del ráster, [franja,
    /// primera fila], imagen).
    static CACHE: std::cell::RefCell<Option<(usize, [usize; 2], crate::raster::Image)>> =
        const { std::cell::RefCell::new(None) };
}

/// Si el ráster máscara tiene datos (ni vacío ni cero) bajo `(x, y)`: su celda más cercana.
fn mask_cell(id: usize, m: &Pyramid, nodata: Option<f32>, x: f64, y: f64) -> bool {
    let l = m.levels[0];
    let (fc, fr) = ((x - m.x0) / m.cell, (m.y_top - y) / m.cell);
    if fc < 0.0 || fr < 0.0 || fc >= f64::from(l.cols) || fr >= f64::from(l.rows) {
        return false;
    }
    let (c, r) = (fc as usize, fr as usize);
    // una franja de 256 filas cada vez por hilo
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
        // corte: celdas cuyo centro está dentro, sobre toda la caja de celdas enteras
        let job = MaskJob::new(d.join("a.tif"), d.join("c.tif"), m.clone());
        let line = clip(&job)?;
        let c = Grid::read(&d.join("c.tif"))?;
        // x 110..120 (10 celdas), y 205..215 (10 celdas); el centro de la primera columna, 110,5,
        // está dentro
        assert_eq!((c.cols, c.rows, c.x0, c.y0), (10, 10, 110.0, 205.0), "{line}");
        assert!(c.data.iter().all(|v| *v == 7.0 || v.is_nan()));
        assert!(c.data.iter().filter(|v| **v == 7.0).count() > 80);
        // sin corte: el mismo tamaño, el exterior vacío
        let mut full = MaskJob::new(d.join("a.tif"), d.join("f.tif"), m.clone());
        full.crop = false;
        clip(&full)?;
        let f = Grid::read(&d.join("f.tif"))?;
        assert_eq!((f.cols, f.rows), (40, 30));
        assert!(f.data[0].is_nan());
        let inside = f.data.iter().filter(|v| **v == 7.0).count();
        // invertido: lo que estaba dentro queda vacío y el resto se conserva
        let mut inv = MaskJob::new(d.join("a.tif"), d.join("i.tif"), m);
        inv.invert = true;
        clip(&inv)?;
        let i = Grid::read(&d.join("i.tif"))?;
        assert_eq!(i.data.iter().filter(|v| **v == 7.0).count(), 1200 - inside);
        // una máscara ráster: las celdas donde no es cero
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
        // una máscara que no toca el ráster es un error
        let far = Mask::Regions(vec![square("z", [1000.0, 1000.0, 1010.0, 1010.0])]);
        assert!(clip(&MaskJob::new(d.join("a.tif"), d.join("z.tif"), far)).is_err());
        std::fs::remove_dir_all(&d)?;
        Ok(())
    }
}
