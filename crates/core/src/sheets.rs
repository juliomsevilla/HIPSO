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

/// Classifies every sheet separately; each gets the path `out_for` gives it.
pub fn classify_each(
    inputs: &[PathBuf],
    out_for: &dyn Fn(&Path) -> PathBuf,
    steps: &Steps,
    tiling: &Tiling,
) -> Result<String> {
    let (lines, _, failed) = each_sheet(inputs, |input| {
        let job = ClassifyJob {
            inputs: vec![input.to_path_buf()],
            output: out_for(input),
            steps: steps.clone(),
            tiling: tiling.clone(),
        };
        Ok((format!("{}: {}", input.display(), or_clean(jobs::classify(&job), &job.output)?), ()))
    })?;
    close_run(lines, failed, inputs.len())
}

/// One DTM per sheet; with `mosaic_to`, also one joined DTM. Returns the summary and the files.
pub fn dtm_each(
    inputs: &[PathBuf],
    base: &DtmJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    mosaic_to: Option<&Path>,
) -> Result<(String, Vec<PathBuf>)> {
    let (mut lines, mut files, failed) = each_sheet(inputs, |input| {
        let job = DtmJob {
            inputs: vec![input.to_path_buf()],
            output: out_for(input),
            ..base.clone()
        };
        Ok((or_clean(jobs::dtm(&job), &job.output)?, job.output))
    })?;
    join_done(&mut files, &mut lines, mosaic_to, Overlap::First, "MDT")?;
    Ok((close_run(lines, failed, inputs.len())?, files))
}

/// One DSM per sheet; optionally a joined DSM.
pub fn dsm_each(
    inputs: &[PathBuf],
    base: &DsmJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    mosaic_to: Option<&Path>,
) -> Result<(String, Vec<PathBuf>)> {
    let (mut lines, mut files, failed) = each_sheet(inputs, |input| {
        let job = DsmJob {
            input: input.to_path_buf(),
            output: out_for(input),
            ..base.clone()
        };
        Ok((or_clean(jobs::dsm(&job), &job.output)?, job.output))
    })?;
    join_done(&mut files, &mut lines, mosaic_to, Overlap::Max, "MDS")?;
    Ok((close_run(lines, failed, inputs.len())?, files))
}

/// One CHM per sheet, each over the DTM `dtm_for` finds for it; optionally a joined CHM.
pub fn chm_each(
    inputs: &[PathBuf],
    base: &ChmJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    dtm_for: &dyn Fn(&Path) -> Result<PathBuf>,
    mosaic_to: Option<&Path>,
) -> Result<(String, Vec<PathBuf>)> {
    let (mut lines, mut files, failed) = each_sheet(inputs, |input| {
        let job = ChmJob {
            input: input.to_path_buf(),
            dtm: dtm_for(input)?,
            output: out_for(input),
            ..base.clone()
        };
        Ok((or_clean(jobs::chm(&job), &job.output)?, job.output))
    })?;
    join_done(&mut files, &mut lines, mosaic_to, Overlap::Max, "CHM")?;
    Ok((close_run(lines, failed, inputs.len())?, files))
}

/// Metrics by cell, one result per sheet.
pub fn metrics_each(
    inputs: &[PathBuf],
    base: &MetricsJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    dtm_for: &dyn Fn(&Path) -> Result<PathBuf>,
) -> Result<String> {
    let (lines, _, failed) = each_sheet(inputs, |input| {
        let job = MetricsJob {
            input: input.to_path_buf(),
            dtm: dtm_for(input)?,
            output: out_for(input),
            ..base.clone()
        };
        Ok((or_clean(jobs::metrics(&job), &job.output)?, ()))
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
