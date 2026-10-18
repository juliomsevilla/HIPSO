//! Canopy height model, as FUSION's CanopyModel: the highest return above the ground in each
//! cell, in one streaming pass.

use crate::Result;
use crate::grid::Grid;
use crate::las::Reader;

/// The CHM on cells of `cell`: per cell, the highest point height above `dtm` (negative heights
/// count as 0); empty cells are filled from their neighbours unless `fill` is false.
pub fn chm(reader: &Reader, dtm: &Grid, cell: f64, fill: bool) -> Result<Grid> {
    let h = &reader.header;
    let mut out = Grid::covering([h.bounds[0], h.bounds[1], h.bounds[3], h.bounds[4]], cell)?;
    reader.scan(|pt| {
        if pt.is_noise() {
            return;
        }
        let (Some(i), Some(gz)) = (out.index(pt.x, pt.y), dtm.sample(pt.x, pt.y)) else {
            return;
        };
        let ht = (pt.z as f32 - gz).max(0.0);
        if let Some(v) = out.data.get_mut(i)
            && (v.is_nan() || ht > *v)
        {
            *v = ht;
        }
    })?;
    if fill {
        out.fill_holes();
    }
    Ok(out)
}
