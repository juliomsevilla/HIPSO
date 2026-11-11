//! Canopy height model, the highest return above the ground in each
//! cell, in one streaming pass.

use crate::Result;
use crate::footprint::Footprint;
use crate::grid::Grid;
use crate::las::Reader;

/// The CHM on cells of `cell`: per cell, the highest point height above `dtm` (negative heights
/// count as 0); empty cells are filled from their neighbours unless `fill` is false.
pub fn chm(reader: &Reader, dtm: &Grid, cell: f64, fill: bool) -> Result<Grid> {
    chm_in(reader, &[], 0.0, dtm, cell, fill)
}

/// The extent of the sheet grown by `margin` on every side.
fn grown(h: &crate::las::Header, margin: f64) -> [f64; 4] {
    [
        h.bounds[0] - margin,
        h.bounds[1] - margin,
        h.bounds[3] + margin,
        h.bounds[4] + margin,
    ]
}

/// [`chm`] of a sheet of a block: the grid reaches `margin` beyond the sheet and the points of
/// its neighbours in `halo` count, so the filling of empty cells at the edge looks across it.
pub fn chm_in(
    reader: &Reader,
    halo: &[crate::las::Point],
    margin: f64,
    dtm: &Grid,
    cell: f64,
    fill: bool,
) -> Result<Grid> {
    let b = grown(&reader.header, margin);
    let mut out = Grid::covering(b, cell)?;
    let fp = Footprint::new(b, cell);
    let mut add = |pt: &crate::las::Point| {
        if pt.is_noise() {
            return;
        }
        fp.add(pt.x, pt.y);
        let (Some(i), Some(gz)) = (out.index(pt.x, pt.y), dtm.sample(pt.x, pt.y)) else {
            return;
        };
        let ht = (pt.z as f32 - gz).max(0.0);
        if let Some(v) = out.data.get_mut(i)
            && (v.is_nan() || ht > *v)
        {
            *v = ht;
        }
    };
    reader.scan(&mut add)?;
    halo.iter().for_each(&mut add);
    if fill {
        out.fill_holes();
        fp.apply(&mut out);
    }
    Ok(out)
}

/// The digital surface model on cells of `cell`: per cell, the highest point, in one streaming
/// pass; empty cells are filled from their neighbours unless `fill` is false.
pub fn dsm(reader: &Reader, cell: f64, fill: bool) -> Result<Grid> {
    dsm_in(reader, &[], 0.0, cell, fill)
}

/// [`dsm`] of a sheet of a block, as [`chm_in`] is of [`chm`].
pub fn dsm_in(
    reader: &Reader,
    halo: &[crate::las::Point],
    margin: f64,
    cell: f64,
    fill: bool,
) -> Result<Grid> {
    let b = grown(&reader.header, margin);
    let mut out = Grid::covering(b, cell)?;
    let fp = Footprint::new(b, cell);
    let mut add = |pt: &crate::las::Point| {
        if pt.is_noise() {
            return;
        }
        fp.add(pt.x, pt.y);
        if let Some(i) = out.index(pt.x, pt.y)
            && let Some(v) = out.data.get_mut(i)
            && (v.is_nan() || pt.z as f32 > *v)
        {
            *v = pt.z as f32;
        }
    };
    reader.scan(&mut add)?;
    halo.iter().for_each(&mut add);
    if fill {
        out.fill_holes();
        fp.apply(&mut out);
    }
    Ok(out)
}
