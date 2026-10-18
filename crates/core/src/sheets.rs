//! Work on a set of loaded sheets (tiles of a flight, delivery blocks): each sheet is processed
//! on its own and gets its own result, one after another, so a large set never has to fit in
//! memory at once and one failing sheet stands out. A mosaic of the results is made only when
//! asked for.

use std::path::{Path, PathBuf};

use crate::grid::Grid;
use crate::jobs::{self, ChmJob, ClassifyJob, DtmJob, MetricsJob};
use crate::las::Reader;
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

/// Joins rasters on the same cell lattice into one.
pub fn mosaic(paths: &[PathBuf], output: &Path, how: Overlap, epsg: Option<u16>) -> Result<()> {
    let grids: Vec<Grid> = paths.iter().map(|p| Grid::read(p)).collect::<Result<_>>()?;
    let first = grids
        .first()
        .ok_or_else(|| Error::Param("nothing to join".into()))?;
    let cell = first.cell;
    if grids.iter().any(|g| (g.cell - cell).abs() > cell * 1e-6) {
        return Err(Error::Param(
            "the sheets have different cell sizes: they cannot be joined".into(),
        ));
    }
    let x0 = grids.iter().map(|g| g.x0).fold(f64::INFINITY, f64::min);
    let y0 = grids.iter().map(|g| g.y0).fold(f64::INFINITY, f64::min);
    let x1 = grids
        .iter()
        .map(|g| g.x0 + g.cols as f64 * cell)
        .fold(f64::NEG_INFINITY, f64::max);
    let y1 = grids
        .iter()
        .map(|g| g.y0 + g.rows as f64 * cell)
        .fold(f64::NEG_INFINITY, f64::max);
    let (cols, rows) = (
        ((x1 - x0) / cell).round() as usize,
        ((y1 - y0) / cell).round() as usize,
    );
    let mut out = Grid::new(x0, y0, cell, cols, rows)?;
    for g in &grids {
        let c0 = ((g.x0 - x0) / cell).round() as usize;
        // rows count from the north
        let r0 = ((y1 - (g.y0 + g.rows as f64 * cell)) / cell).round() as usize;
        for r in 0..g.rows {
            for c in 0..g.cols {
                let v = g.data[r * g.cols + c];
                if v.is_nan() {
                    continue;
                }
                let Some(d) = out.data.get_mut((r0 + r) * cols + c0 + c) else {
                    continue;
                };
                if d.is_nan() || (how == Overlap::Max && v > *d) {
                    *d = v;
                }
            }
        }
    }
    crate::paths::ensure_parent(output)?;
    out.write_tiff(output, epsg)
}

fn epsg_of(p: &Path) -> Option<u16> {
    Reader::open(p).ok().and_then(|r| r.header.epsg)
}

/// Classifies every sheet separately; each gets the path `out_for` gives it.
pub fn classify_each(
    inputs: &[PathBuf],
    out_for: &dyn Fn(&Path) -> PathBuf,
    steps: &Steps,
    tiling: &Tiling,
) -> Result<String> {
    let mut lines = Vec::new();
    for (k, input) in inputs.iter().enumerate() {
        crate::check_cancel()?;
        let job = ClassifyJob {
            inputs: vec![input.clone()],
            output: out_for(input),
            steps: steps.clone(),
            tiling: tiling.clone(),
        };
        lines.push(format!(
            "[{}/{}] {}: {}",
            k + 1,
            inputs.len(),
            input.display(),
            jobs::classify(&job)?
        ));
    }
    Ok(lines.join("\n"))
}

/// One DTM per sheet; with `mosaic_to`, also one joined DTM. Returns the summary and the files.
pub fn dtm_each(
    inputs: &[PathBuf],
    base: &DtmJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    mosaic_to: Option<&Path>,
) -> Result<(String, Vec<PathBuf>)> {
    let (mut lines, mut files) = (Vec::new(), Vec::new());
    for (k, input) in inputs.iter().enumerate() {
        crate::check_cancel()?;
        let job = DtmJob {
            inputs: vec![input.clone()],
            output: out_for(input),
            ..base.clone()
        };
        lines.push(format!("[{}/{}] {}", k + 1, inputs.len(), jobs::dtm(&job)?));
        files.push(job.output);
    }
    if let Some(m) = mosaic_to.filter(|_| files.len() > 1) {
        crate::check_cancel()?;
        mosaic(
            &files,
            m,
            Overlap::First,
            inputs.first().and_then(|p| epsg_of(p)),
        )?;
        lines.push(format!("mosaico de {} MDT: {}", files.len(), m.display()));
        files.push(m.to_path_buf());
    }
    Ok((lines.join("\n"), files))
}

/// One CHM per sheet, each over the DTM `dtm_for` finds for it; optionally a joined CHM.
pub fn chm_each(
    inputs: &[PathBuf],
    base: &ChmJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    dtm_for: &dyn Fn(&Path) -> Result<PathBuf>,
    mosaic_to: Option<&Path>,
) -> Result<(String, Vec<PathBuf>)> {
    let (mut lines, mut files) = (Vec::new(), Vec::new());
    for (k, input) in inputs.iter().enumerate() {
        crate::check_cancel()?;
        let job = ChmJob {
            input: input.clone(),
            dtm: dtm_for(input)?,
            output: out_for(input),
            ..base.clone()
        };
        lines.push(format!("[{}/{}] {}", k + 1, inputs.len(), jobs::chm(&job)?));
        files.push(job.output);
    }
    if let Some(m) = mosaic_to.filter(|_| files.len() > 1) {
        crate::check_cancel()?;
        mosaic(
            &files,
            m,
            Overlap::Max,
            inputs.first().and_then(|p| epsg_of(p)),
        )?;
        lines.push(format!("mosaico de {} CHM: {}", files.len(), m.display()));
        files.push(m.to_path_buf());
    }
    Ok((lines.join("\n"), files))
}

/// Metrics by cell, one result per sheet.
pub fn metrics_each(
    inputs: &[PathBuf],
    base: &MetricsJob,
    out_for: &dyn Fn(&Path) -> PathBuf,
    dtm_for: &dyn Fn(&Path) -> Result<PathBuf>,
) -> Result<String> {
    let mut lines = Vec::new();
    for (k, input) in inputs.iter().enumerate() {
        crate::check_cancel()?;
        let job = MetricsJob {
            input: input.clone(),
            dtm: dtm_for(input)?,
            output: out_for(input),
            ..base.clone()
        };
        lines.push(format!(
            "[{}/{}] {}",
            k + 1,
            inputs.len(),
            jobs::metrics(&job)?
        ));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
