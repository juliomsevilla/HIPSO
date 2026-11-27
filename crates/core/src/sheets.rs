//! Trabajo sobre un conjunto de hojas cargadas (teselas de un vuelo, bloques de una entrega): cada
//! hoja se procesa por sí sola y recibe su propio resultado, una tras otra, de modo que un conjunto
//! grande nunca tiene que caber en memoria a la vez y una hoja que falla destaca. Un mosaico de los
//! resultados se hace solo cuando se pide.

use std::path::{Path, PathBuf};

use crate::jobs::{self, ChmJob, ClassifyJob, DsmJob, DtmJob, MetricsJob};
use crate::pipeline::{Steps, Tiling};
use crate::{Error, Result};

/// Cómo se combinan las celdas solapadas de dos hojas en un mosaico.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlap {
    /// El valor mayor (modelos de copas).
    Max,
    /// El valor de la primera hoja (modelos del terreno).
    First,
}

/// Une rásteres del mismo tamaño de celda en uno. Se hace en franjas de filas: solo están en
/// memoria la franja que se escribe y las hojas que la tocan, de modo que un mosaico de cientos de
/// hojas (cientos de millones de celdas) no necesita más memoria que unas pocas hojas. Las hojas en
/// otro sistema de coordenadas se llevan al de la primera antes de unirlas.
pub fn mosaic(paths: &[PathBuf], output: &Path, how: Overlap, _epsg: Option<u16>) -> Result<()> {
    use crate::rastertools::{self, RasterJob, Source};
    if paths.is_empty() {
        return Err(Error::Param("no hay nada que unir".into()));
    }
    let unified = crate::crs::unify(paths, false)?;
    let paths = unified.all(paths);
    let mut job = RasterJob::new(
        paths
            .into_iter()
            .map(|path| Source {
                path,
                bands: Vec::new(),
            })
            .collect(),
        output.to_path_buf(),
    );
    job.overlap = match how {
        Overlap::Max => rastertools::Overlap::Max,
        Overlap::First => rastertools::Overlap::First,
    };
    job.nodata = crate::grid::NODATA;
    rastertools::mosaic(&job)?;
    Ok(())
}

/// Une imágenes de 8 bits cuyos píxeles vacíos son 0 en una sola, tomando la media donde se
/// solapan.
pub fn mosaic_image(paths: &[PathBuf], output: &Path) -> Result<()> {
    use crate::rastertools::{self, Dtype, RasterJob, Source};
    if paths.is_empty() {
        return Err(Error::Param("no hay nada que unir".into()));
    }
    let unified = crate::crs::unify(paths, false)?;
    let paths = unified.all(paths);
    let mut job = RasterJob::new(
        paths
            .into_iter()
            .map(|path| Source {
                path,
                bands: Vec::new(),
            })
            .collect(),
        output.to_path_buf(),
    );
    job.overlap = rastertools::Overlap::Mean;
    job.dtype = Some(Dtype::U8);
    job.src_nodata = Some(0.0);
    job.nodata = 0.0;
    rastertools::mosaic(&job)?;
    Ok(())
}

fn check_neighbors(neighbors: f64) -> Result<()> {
    if neighbors >= 0.0 && neighbors.is_finite() {
        Ok(())
    } else {
        Err(Error::Param(
            "la distancia a las hojas vecinas no puede ser negativa".into(),
        ))
    }
}

/// Halla el modelo del suelo de una hoja.
type GroundFor<'a> = &'a dyn Fn(&Path) -> Result<PathBuf>;

/// Una carpeta de archivos temporales, borrada con todo lo que contiene al soltarla.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Las cabeceras de las hojas de una ejecución, para saber qué hojas son vecinas, y las franjas de
/// cada hoja que necesitan sus vecinas.
struct Block {
    cat: crate::catalog::Catalog,
    entries: std::sync::Arc<Vec<crate::catalog::Entry>>,
    /// Dónde está cada hoja en el catálogo.
    places: std::collections::HashMap<PathBuf, usize>,
    /// Para cada hoja, archivos pequeños con los puntos de sus vecinas a menos de `margin` de ella.
    strips: Vec<Vec<PathBuf>>,
    _scratch: Option<Scratch>,
}

impl Block {
    /// Cataloga las hojas y corta, de cada hoja, las franjas que necesitan sus vecinas a menos de
    /// `margin` metros, en una pasada sobre cada hoja. Una hoja trabaja entonces con unos pocos
    /// archivos pequeños de contexto y no con hojas vecinas enteras, que habría que leer y
    /// descomprimir de nuevo por cada una de sus (hasta ocho) vecinas.
    fn of(inputs: &[PathBuf], margin: f64) -> Result<Self> {
        let cat = crate::catalog::catalog(inputs)?;
        let entries = std::sync::Arc::new(cat.entries.clone());
        let mut strips: Vec<Vec<PathBuf>> = vec![Vec::new(); entries.len()];
        let dir = std::env::temp_dir().join(format!(
            "hipso-strips-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir)?;
        let scratch = Scratch(dir.clone());
        for (i, e) in entries.iter().enumerate() {
            crate::check_cancel()?;
            let near = cat.neighbours(i, margin);
            if near.is_empty() {
                continue;
            }
            // una hoja que no se puede leer se comunica cuando se procesa, no aquí
            let Ok(reader) = crate::las::Reader::open(&e.path) else {
                continue;
            };
            let areas: Vec<[f64; 4]> = near
                .iter()
                .map(|&j| {
                    let b = entries[j].footprint();
                    [b[0] - margin, b[1] - margin, b[2] + margin, b[3] + margin]
                })
                .collect();
            let mut writers: Vec<Option<crate::las::Writer>> =
                (0..near.len()).map(|_| None).collect();
            let rec_len = usize::from(reader.header.record_len);
            let cut = reader.scan_raw(|bytes| {
                for rec in bytes.chunks_exact(rec_len) {
                    let Some(p) = reader.decode(rec) else { continue };
                    for (n, a) in areas.iter().enumerate() {
                        if p.x >= a[0] && p.x < a[2] && p.y >= a[1] && p.y < a[3] {
                            if writers[n].is_none() {
                                let path = dir.join(format!("{i}_{}.las", near[n]));
                                writers[n] = Some(crate::las::Writer::create(&path, &reader)?);
                            }
                            if let Some(w) = writers[n].as_mut() {
                                w.write(rec)?;
                            }
                        }
                    }
                }
                Ok(())
            });
            // una hoja truncada da las franjas que dio antes de fallar
            if let Err(Error::Cancelled) = cut {
                return Err(Error::Cancelled);
            }
            for (n, w) in writers.into_iter().enumerate() {
                if let Some(w) = w {
                    w.finish()?;
                    strips[near[n]].push(dir.join(format!("{i}_{}.las", near[n])));
                }
            }
        }
        let places = entries.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
        Ok(Self {
            places,
            cat,
            entries,
            strips,
            _scratch: Some(scratch),
        })
    }

    /// Los archivos con los puntos alrededor de la hoja `k` que guardan sus vecinas.
    fn context(&self, k: usize) -> Vec<PathBuf> {
        self.strips[k].clone()
    }

    fn index(&self, input: &Path) -> Result<usize> {
        self.places
            .get(input)
            .copied()
            .ok_or_else(|| Error::Param("la cabecera de la hoja no se pudo leer".into()))
    }

    /// El contexto de una superficie (MDS o CHM) de la hoja `input`: las vecinas a menos de
    /// `neighbors` metros y, cuando `strip` (los métodos del punto más alto), sus puntos cerca de
    /// la hoja en lugar de los archivos vecinos enteros. `dtm_for` da los modelos del suelo de las
    /// vecinas para un CHM.
    fn surface_context(
        &self,
        input: &Path,
        neighbors: f64,
        strip: bool,
        dtm_for: Option<GroundFor>,
    ) -> Result<jobs::SheetContext> {
        let k = self.index(input)?;
        let near = self.cat.neighbours(k, neighbors);
        if near.is_empty() {
            return Ok(jobs::SheetContext::default());
        }
        let files = self.context(k);
        let sheets: Vec<PathBuf> = near.iter().map(|&j| self.entries[j].path.clone()).collect();
        let footprint = self.entries[k].footprint();
        Ok(jobs::SheetContext {
            halo: if strip {
                crate::catalog::halo_points(&files, footprint, neighbors)?
            } else {
                Vec::new()
            },
            extra: if strip { Vec::new() } else { files.clone() },
            margin: neighbors,
            ground_around: dtm_for
                .map(|f| {
                    sheets
                        .iter()
                        .filter_map(|p| f(p).ok())
                        .filter(|p| p.is_file())
                        .collect()
                })
                .unwrap_or_default(),
            crop: Some(footprint),
        })
    }

    fn metrics_context(
        &self,
        input: &Path,
        cell: f64,
        neighbors: f64,
        dtm_for: &dyn Fn(&Path) -> Result<PathBuf>,
    ) -> Result<jobs::MetricsContext> {
        let k = self.index(input)?;
        let near = self.cat.neighbours(k, neighbors.max(cell));
        if near.is_empty() {
            return Ok(jobs::MetricsContext::default());
        }
        let halo =
            crate::catalog::halo_points(&self.context(k), self.entries[k].footprint(), cell)?;
        // los modelos del suelo de las vecinas que tienen uno
        let ground_around = near
            .iter()
            .filter_map(|&j| dtm_for(&self.entries[j].path).ok())
            .filter(|p| p.is_file())
            .collect();
        let mut candidates = near;
        candidates.push(k);
        let entries = self.entries.clone();
        Ok(jobs::MetricsContext {
            halo,
            ground_around,
            margin: cell,
            owns: Some(Box::new(move |x, y| {
                crate::catalog::owner_of(&entries, &candidates, x, y) == Some(k)
            })),
        })
    }
}

/// La variable de entorno con la lista (un archivo, una ruta por línea) de las hojas que ya están
/// hechas: sirven de contexto a sus vecinas y no se repiten. La usan los flujos al reanudar.
pub const SKIP_ENV: &str = "HIPSO_HOJAS_HECHAS";
/// Cómo empieza la línea con la que se avisa de que una hoja está terminada, si quien lanzó el
/// proceso pidió los avisos.
pub const DONE: &str = "» hecha: ";

/// Ejecuta `work` sobre cada hoja. Una hoja que falla (un archivo truncado, por ejemplo) se anota y
/// las demás siguen, de modo que no se pierden horas de trabajo por un mal archivo; solo una
/// cancelación detiene la ejecución. Da las líneas de las hojas que fueron bien y las hojas que
/// fallaron, con el motivo.
fn each_sheet<T>(
    inputs: &[PathBuf],
    mut work: impl FnMut(&Path) -> Result<(String, T)>,
) -> Result<(Vec<String>, Vec<T>, Vec<String>)> {
    let (mut lines, mut done, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    let skip: std::collections::HashSet<PathBuf> = std::env::var_os(SKIP_ENV)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|t| t.lines().map(PathBuf::from).collect())
        .unwrap_or_default();
    let tell = std::env::var_os(crate::paths::ANNOUNCE_ENV).is_some();
    for (k, input) in inputs.iter().enumerate() {
        crate::check_cancel()?;
        if skip.contains(input) {
            continue;
        }
        match work(input) {
            Ok((line, t)) => {
                if tell {
                    println!("{DONE}{}", input.display());
                }
                lines.push(format!("[{}/{}] {}", k + 1, inputs.len(), line));
                done.push(t);
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => {
                let m = format!("[{}/{}] {}: ERROR: {e}", k + 1, inputs.len(), input.display());
                eprintln!("{m}");
                failed.push(m);
            }
        }
    }
    Ok((lines, done, failed))
}

/// El resultado del trabajo de una hoja; un fallo se lleva la salida que dejó a medio escribir, que
/// pasaría por buena (y la leería el paso siguiente).
fn or_clean<T>(r: Result<T>, output: &Path) -> Result<T> {
    if r.is_err() {
        let _ = std::fs::remove_file(output);
        // un shapefile son varios archivos: los demás de este también se van
        if output.extension().is_some_and(|e| e.eq_ignore_ascii_case("shp")) {
            let stem = crate::paths::stem(output);
            for ext in ["dbf", "shx", "prj", "cpg"] {
                let _ = std::fs::remove_file(output.with_extension(ext));
            }
            let _ = std::fs::remove_file(output.with_file_name(format!("{stem}_campos.txt")));
        }
    }
    r
}

/// Termina una ejecución sobre hojas: los fallos, si los hay, la convierten en un error una vez
/// hecho todo lo que se podía hacer.
fn close_run(mut lines: Vec<String>, failed: Vec<String>, total: usize) -> Result<String> {
    if failed.is_empty() {
        return Ok(lines.join("\n"));
    }
    lines.extend(failed.iter().cloned());
    Err(Error::Param(format!(
        "{} de {} hojas no se pudieron procesar (el resto está hecho):\n{}",
        failed.len(),
        total,
        failed.join("\n")
    )))
}

/// El ráster unido de las hojas que fueron bien, cuando se pide y hay más de una.
fn join_done(
    files: &mut Vec<PathBuf>,
    lines: &mut Vec<String>,
    mosaic_to: Option<&Path>,
    how: Overlap,
    what: &str,
) -> Result<()> {
    if let Some(m) = mosaic_to.filter(|_| files.len() > 1) {
        crate::check_cancel()?;
        mosaic(files, m, how, None)?;
        lines.push(format!("mosaico de {} {what}: {}", files.len(), m.display()));
        files.push(m.to_path_buf());
    }
    Ok(())
}

/// Clasifica cada hoja por separado; cada una recibe la ruta que le da `out_for`. Con `neighbors`
/// (metros), las hojas que están a esa distancia de una se leen como contexto: los filtros de
/// suelo, ruido y edificios ven lo que hay al otro lado del borde (un edificio cortado por el borde
/// no se toma por suelo), y solo se clasifican y escriben los puntos de la hoja. El margen de las
/// teselas (30 m por defecto) es lo que miran los filtros al otro lado, así que `neighbors` no
/// necesita ser mayor que eso.
pub fn classify_each(
    inputs: &[PathBuf],
    out_for: &dyn Fn(&Path) -> PathBuf,
    steps: &Steps,
    tiling: &Tiling,
    neighbors: f64,
) -> Result<String> {
    check_neighbors(neighbors)?;
    let block = if neighbors > 0.0 {
        Some(Block::of(inputs, neighbors)?)
    } else {
        None
    };
    let (lines, _, failed) = each_sheet(inputs, |input| {
        let job = ClassifyJob {
            inputs: vec![input.to_path_buf()],
            output: out_for(input),
            steps: steps.clone(),
            tiling: tiling.clone(),
        };
        let context: Vec<PathBuf> = match &block {
            Some(b) => b.context(b.index(input)?),
            None => Vec::new(),
        };
        Ok((
            format!(
                "{}: {}",
                input.display(),
                or_clean(jobs::classify_with(&job, &context), &job.output)?
            ),
            (),
        ))
    })?;
    close_run(lines, failed, inputs.len())
}

/// Un MDT por hoja; con `mosaic_to`, también un MDT unido. Devuelve el resumen y los archivos. Con
/// `neighbors` (metros), las vecinas a esa distancia se leen como contexto: la triangulación ve sus
/// puntos hasta donde llega el margen de las teselas, y solo se hace la propia extensión de la
/// hoja, de modo que los bordes son los que habría dado todo el bloque y los MDT de las hojas se
/// unen sin costura.
pub fn dtm_each(
    inputs: &[PathBuf],
    base: &DtmJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    mosaic_to: Option<&Path>,
    neighbors: f64,
) -> Result<(String, Vec<PathBuf>)> {
    check_neighbors(neighbors)?;
    let block = if neighbors > 0.0 {
        Some(Block::of(inputs, neighbors)?)
    } else {
        None
    };
    let (mut lines, mut files, failed) = each_sheet(inputs, |input| {
        let job = DtmJob {
            inputs: vec![input.to_path_buf()],
            output: out_for(input),
            ..base.clone()
        };
        let context: Vec<PathBuf> = match &block {
            Some(b) => b.context(b.index(input)?),
            None => Vec::new(),
        };
        Ok((or_clean(jobs::dtm_with(&job, &context), &job.output)?, job.output))
    })?;
    join_done(&mut files, &mut lines, mosaic_to, Overlap::First, "MDT")?;
    Ok((close_run(lines, failed, inputs.len())?, files))
}

/// Un MDS por hoja; opcionalmente un MDS unido. Con `neighbors` (metros) cada hoja se hace con las
/// hojas que están a esa distancia y se recorta a su propia extensión, de modo que la
/// triangulación, el relleno de celdas vacías y el suavizado no tienen costura en sus bordes. El
/// punto más alto de una celda no tiene borde del que hablar: con ese método y sin relleno ni
/// suavizado `neighbors` no cambia nada y no se usa.
pub fn dsm_each(
    inputs: &[PathBuf],
    base: &DsmJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    mosaic_to: Option<&Path>,
    neighbors: f64,
) -> Result<(String, Vec<PathBuf>)> {
    check_neighbors(neighbors)?;
    let strip = base.method == jobs::DsmMethod::Max;
    let needed = !strip || base.fill || base.smooth > 0.0;
    let block = if neighbors > 0.0 && needed {
        Some(Block::of(inputs, neighbors)?)
    } else {
        None
    };
    let (mut lines, mut files, failed) = each_sheet(inputs, |input| {
        let job = DsmJob {
            input: input.to_path_buf(),
            output: out_for(input),
            ..base.clone()
        };
        let ctx = match &block {
            Some(b) => b.surface_context(input, neighbors, strip, None)?,
            None => jobs::SheetContext::default(),
        };
        Ok((or_clean(jobs::dsm_with(&job, &ctx), &job.output)?, job.output))
    })?;
    join_done(&mut files, &mut lines, mosaic_to, Overlap::Max, "MDS")?;
    Ok((close_run(lines, failed, inputs.len())?, files))
}

/// Un CHM por hoja, cada uno sobre el MDT que le halla `dtm_for`; opcionalmente un CHM unido. Con
/// `neighbors`, como [`dsm_each`], y los MDT de las vecinas dan el suelo bajo sus puntos.
pub fn chm_each(
    inputs: &[PathBuf],
    base: &ChmJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    dtm_for: &dyn Fn(&Path) -> Result<PathBuf>,
    mosaic_to: Option<&Path>,
    neighbors: f64,
) -> Result<(String, Vec<PathBuf>)> {
    check_neighbors(neighbors)?;
    let strip = matches!(base.method, jobs::ChmMethod::Max { .. });
    let needed = match &base.method {
        jobs::ChmMethod::Max { fill } => *fill || base.smooth > 0.0,
        _ => true,
    };
    let block = if neighbors > 0.0 && needed {
        Some(Block::of(inputs, neighbors)?)
    } else {
        None
    };
    let (mut lines, mut files, failed) = each_sheet(inputs, |input| {
        let job = ChmJob {
            input: input.to_path_buf(),
            dtm: dtm_for(input)?,
            output: out_for(input),
            ..base.clone()
        };
        let ctx = match &block {
            Some(b) => b.surface_context(input, neighbors, strip, Some(dtm_for))?,
            None => jobs::SheetContext::default(),
        };
        Ok((or_clean(jobs::chm_with(&job, &ctx), &job.output)?, job.output))
    })?;
    join_done(&mut files, &mut lines, mosaic_to, Overlap::Max, "CHM")?;
    Ok((close_run(lines, failed, inputs.len())?, files))
}

/// Métricas por celda, un resultado por hoja. Las celdas se alinean a múltiplos del tamaño de
/// celda, de modo que el borde de una hoja suele caer en medio de una celda, y sin `neighbors` esa
/// celda está en las tablas de ambas hojas con la mitad de sus puntos cada una. Con `neighbors`
/// (una distancia en metros; se usa al menos una celda) cada hoja toma los puntos de sus vecinas en
/// las celdas de su borde y escribe solo las celdas que le pertenecen (véase
/// [`crate::catalog::owner_of`]): cada celda del bloque sale una vez, como si el bloque fuera un
/// solo archivo.
pub fn metrics_each(
    inputs: &[PathBuf],
    base: &MetricsJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    dtm_for: &dyn Fn(&Path) -> Result<PathBuf>,
    neighbors: f64,
) -> Result<String> {
    check_neighbors(neighbors)?;
    let block = if neighbors > 0.0 {
        Some(Block::of(inputs, neighbors.max(base.params.cell))?)
    } else {
        None
    };
    let (lines, _, failed) = each_sheet(inputs, |input| {
        let job = MetricsJob {
            input: input.to_path_buf(),
            dtm: dtm_for(input)?,
            output: out_for(input),
            ..base.clone()
        };
        let ctx = match &block {
            Some(b) => b.metrics_context(input, base.params.cell, neighbors, dtm_for)?,
            None => jobs::MetricsContext::default(),
        };
        Ok((or_clean(jobs::metrics_with(&job, &ctx), &job.output)?, ()))
    })?;
    close_run(lines, failed, inputs.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;

    #[test]
    fn mosaic_joins_neighbouring_sheets() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_mosaic_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let mut a = Grid::new(0.0, 0.0, 1.0, 4, 2)?;
        a.data.iter_mut().for_each(|v| *v = 1.0);
        let mut b = Grid::new(4.0, 0.0, 1.0, 4, 2)?;
        b.data.iter_mut().for_each(|v| *v = 2.0);
        a.write_tiff(&dir.join("a.tif"), None)?;
        b.write_tiff(&dir.join("b.tif"), None)?;
        mosaic(
            &[dir.join("a.tif"), dir.join("b.tif")],
            &dir.join("m.tif"),
            Overlap::First,
            None,
        )?;
        let m = Grid::read(&dir.join("m.tif"))?;
        assert_eq!((m.cols, m.rows), (8, 2));
        assert_eq!((m.data[0], m.data[7]), (1.0, 2.0));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_bad_sheet_does_not_stop_the_others_and_is_named() {
        let inputs: Vec<PathBuf> = ["a.laz", "bad.laz", "c.laz"].iter().map(PathBuf::from).collect();
        let mut seen = Vec::new();
        let (lines, done, failed) = each_sheet(&inputs, |p| {
            seen.push(p.to_path_buf());
            if p.ends_with("bad.laz") {
                Err(Error::Format("el archivo se acaba antes de tiempo".into()))
            } else {
                Ok((format!("{} bien", p.display()), p.to_path_buf()))
            }
        })
        .unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!((lines.len(), done.len(), failed.len()), (2, 2, 1));
        assert!(failed[0].contains("bad.laz") && failed[0].contains("[2/3]"));
        let e = close_run(lines, failed, 3).unwrap_err().to_string();
        assert!(e.contains("1 de 3") && e.contains("bad.laz"), "{e}");
        // una cancelación sí detiene la ejecución
        let r = each_sheet(&inputs, |_| -> Result<(String, ())> { Err(Error::Cancelled) });
        assert!(matches!(r, Err(Error::Cancelled)));
    }
}
