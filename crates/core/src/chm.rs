//! Modelo de altura de copas, el retorno más alto sobre el suelo en cada celda, en una pasada en
//! flujo.

use crate::Result;
use crate::footprint::Footprint;
use crate::grid::Grid;
use crate::las::Reader;

/// El CHM en celdas de `cell`: por celda, la altura del punto más alto sobre `dtm` (las alturas
/// negativas cuentan como 0); las celdas vacías se rellenan desde sus vecinas salvo que `fill` sea
/// falso.
pub fn chm(reader: &Reader, dtm: &Grid, cell: f64, fill: bool) -> Result<Grid> {
    chm_in(reader, &[], 0.0, dtm, cell, fill)
}

/// La extensión de la hoja ampliada `margin` por cada lado.
fn grown(h: &crate::las::Header, margin: f64) -> [f64; 4] {
    [
        h.bounds[0] - margin,
        h.bounds[1] - margin,
        h.bounds[3] + margin,
        h.bounds[4] + margin,
    ]
}

/// [`chm`] de una hoja de un bloque: la rejilla llega `margin` más allá de la hoja y cuentan los
/// puntos de sus vecinas en `halo`, de modo que el relleno de celdas vacías en el borde mira a
/// través de él.
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

/// El modelo digital de superficie en celdas de `cell`: por celda, el punto más alto, en una pasada
/// en flujo; las celdas vacías se rellenan desde sus vecinas salvo que `fill` sea falso.
pub fn dsm(reader: &Reader, cell: f64, fill: bool) -> Result<Grid> {
    dsm_in(reader, &[], 0.0, cell, fill)
}

/// [`dsm`] de una hoja de un bloque, como [`chm_in`] lo es de [`chm`].
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
