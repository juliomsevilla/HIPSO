//! Work on a set of loaded sheets (tiles of a flight, delivery blocks): each sheet is processed
//! on its own and gets its own result, one after another, so a large set never has to fit in
//! memory at once and one failing sheet stands out. A mosaic of the results is made only when
//! asked for.

use std::path::{Path, PathBuf};

use crate::jobs::{self, ChmJob, ClassifyJob, DsmJob, DtmJob, MetricsJob};
use crate::pipeline::{Steps, Tiling};
use crate::{Error, Result};

/// How overlapping cells of two sheets are combined in a mosaic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlap {
    /// The higher value (canopy models).
    Max,
    /// The first sheet's value (terrain models).
    First,
}

/// Joins rasters on the same cell size into one. It is made in strips of rows: only the strip
/// being written and the sheets that touch it are in memory, so a mosaic of hundreds of sheets
/// (hundreds of millions of cells) needs no more memory than a few sheets do. Sheets in another
/// coordinate system are moved to the one of the first before they are joined.
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

fn check_neighbors(neighbors: f64) -> Result<()> {
    if neighbors >= 0.0 && neighbors.is_finite() {
        Ok(())
    } else {
        Err(Error::Param(
            "la distancia a las hojas vecinas no puede ser negativa".into(),
        ))
    }
}

/// Finds the ground model of a sheet.
type GroundFor<'a> = &'a dyn Fn(&Path) -> Result<PathBuf>;

/// A folder of temporary files, deleted with everything in it when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The headers of the sheets of a run, to tell which sheets are neighbours, and the strips of
/// each sheet that its neighbours need.
struct Block {
    cat: crate::catalog::Catalog,
    entries: std::sync::Arc<Vec<crate::catalog::Entry>>,
    /// For each sheet, small files with the points of its neighbours within `margin` of it.
    strips: Vec<Vec<PathBuf>>,
    _scratch: Option<Scratch>,
}

impl Block {
    /// Catalogues the sheets and cuts, from every sheet, the strips its neighbours within
    /// `margin` metres need, in one pass over each sheet. A sheet then works with a few small
    /// files of context and not with whole neighbouring sheets, which would have to be read and
    /// decompressed again for each of its (up to eight) neighbours.
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
            // a sheet that cannot be read is reported when it is processed, not here
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
            // a truncated sheet gives the strips it gave before it failed
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
        Ok(Self {
            cat,
            entries,
            strips,
            _scratch: Some(scratch),
        })
    }

    /// The files with the points around sheet `k` that its neighbours hold.
    fn context(&self, k: usize) -> Vec<PathBuf> {
        self.strips[k].clone()
    }

    fn index(&self, input: &Path) -> Result<usize> {
        self.cat
            .entries
            .iter()
            .position(|e| e.path == input)
            .ok_or_else(|| Error::Param("la cabecera de la hoja no se pudo leer".into()))
    }

    /// The context of a surface (DSM or CHM) of sheet `input`: the neighbours within `neighbors`
    /// metres and, when `strip` (the highest-point methods), their points near the sheet instead of
    /// the whole neighbouring files. `dtm_for` gives the neighbours' ground models for a CHM.
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
        // the ground models of the neighbours that have one
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

/// Runs `work` on every sheet. A sheet that fails (a truncated file, say) is noted and the rest
/// go on, so that hours of work are not lost for one bad file; only a cancel stops the run. Gives
/// the lines of the sheets that went well and the sheets that failed, with the reason.
fn each_sheet<T>(
    inputs: &[PathBuf],
    mut work: impl FnMut(&Path) -> Result<(String, T)>,
) -> Result<(Vec<String>, Vec<T>, Vec<String>)> {
    let (mut lines, mut done, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for (k, input) in inputs.iter().enumerate() {
        crate::check_cancel()?;
        match work(input) {
            Ok((line, t)) => {
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

/// The result of a sheet's job; a failure takes away the output it left half written, which would
/// pass for a good one (and be read by the next step).
fn or_clean<T>(r: Result<T>, output: &Path) -> Result<T> {
    if r.is_err() {
        let _ = std::fs::remove_file(output);
        // a shapefile is several files: the rest of this one go too
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

/// Ends a run over sheets: the failures, if any, make it an error once everything that could be
/// done is done.
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

/// The joined raster of the sheets that went well, when asked for and there is more than one.
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

/// Classifies every sheet separately; each gets the path `out_for` gives it. With `neighbors`
/// (metres) the sheets that close to one are read as context: the ground, noise and building
/// filters see what lies across the edge (a building cut by the edge is not taken for ground),
/// and only the points of the sheet are classified and written. The tiles' buffer (30 m by
/// default) is what the filters look across, so `neighbors` need not be more than that.
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

/// One DTM per sheet; with `mosaic_to`, also one joined DTM. Returns the summary and the files.
/// With `neighbors` (metres), the neighbours within that distance are read as context: the
/// triangulation sees their points as far as the tiles' buffer reaches, and only the sheet's own
/// extent is made, so the edges are the ones the whole block would have given and the sheets'
/// DTMs join without a seam.
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

/// One DSM per sheet; optionally a joined DSM. With `neighbors` (metres) each sheet is made with
/// the sheets that close to it and cut to its own extent, so the triangulation, the filling of
/// empty cells and the smoothing have no seam at its edges. The highest point of a cell has no
/// edge to speak of: with that method and neither filling nor smoothing `neighbors` changes
/// nothing and is not used.
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

/// One CHM per sheet, each over the DTM `dtm_for` finds for it; optionally a joined CHM. With
/// `neighbors`, as [`dsm_each`], and the neighbours' DTMs give the ground under their points.
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

/// Metrics by cell, one result per sheet. The cells are aligned to multiples of the cell size, so
/// the edge of a sheet usually falls in the middle of a cell, and without `neighbors` that cell is
/// in the tables of both sheets with half its points each. With `neighbors` (a distance in metres,
/// at least one cell is used) each sheet takes the points of its neighbours into the cells at its
/// edge and writes only the cells it owns (see [`crate::catalog::owner_of`]): every cell of the
/// block comes out once, as if the block were one file.
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
        // a cancel does stop the run
        let r = each_sheet(&inputs, |_| -> Result<(String, ())> { Err(Error::Cancelled) });
        assert!(matches!(r, Err(Error::Cancelled)));
    }
}
