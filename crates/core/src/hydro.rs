//! Drenaje sobre un MDT: depresiones rellenadas con la Priority-Flood de Barnes, Lehman y Mulla
//! (2014), direcciones de flujo D8 (O'Callaghan y Mark, 1984), la superficie que drena cada celda,
//! el índice topográfico de humedad de Beven y Kirkby (1979) y las cuencas de los tramos de la red
//! de cauces. Una pasada inunda desde el borde, así que la memoria son unos pocos bytes por celda y
//! el tiempo es lineal; no se ordena nada. Las celdas vacías y el borde de la rejilla son salidas.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use rayon::prelude::*;

use crate::grid::Grid;
use crate::{Error, Result};

/// Los ocho vecinos como pasos (columna, fila): E, SE, S, SO, O, NO, N, NE.
pub(crate) const STEP: [(isize, isize); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];
pub(crate) const NONE: u8 = 255;

/// El índice del vecino de `i` en la dirección `k`, si está dentro de la rejilla.
#[inline]
pub(crate) fn neighbour(g: &Grid, i: usize, k: usize) -> Option<usize> {
    let (c, r) = ((i % g.cols) as isize, (i / g.cols) as isize);
    let (c, r) = (c + STEP[k].0, r + STEP[k].1);
    if c < 0 || r < 0 || c >= g.cols as isize || r >= g.rows as isize {
        None
    } else {
        Some(r as usize * g.cols + c as usize)
    }
}

/// Distancia al vecino en la dirección `k`, en celdas.
#[inline]
pub(crate) fn reach(k: usize) -> f64 {
    if k.is_multiple_of(2) {
        1.0
    } else {
        std::f64::consts::SQRT_2
    }
}

/// Un decimal como entero que ordena igual.
#[inline]
pub(crate) fn key(v: f32) -> u32 {
    let b = v.to_bits();
    if b >> 31 == 1 { !b } else { b | 0x8000_0000 }
}

/// La superficie con sus depresiones rellenadas y, para cada celda, la dirección (0-7) en la que
/// drena, o `NONE` donde sale de la rejilla.
pub struct Drainage {
    pub filled: Vec<f32>,
    pub dir: Vec<u8>,
}

pub fn drainage(g: &Grid, z: f64) -> Result<Drainage> {
    let n = g.data.len();
    let z = z as f32;
    let mut filled: Vec<f32> = g.data.iter().map(|v| v * z).collect();
    // el camino de vuelta a la celda desde la que se llegó a cada celda: donde drenan los llanos y
    // los hoyos rellenados
    let mut back = vec![NONE; n];
    let mut seen = vec![false; n];
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
    let mut flat: VecDeque<u32> = VecDeque::new();
    let index = |i: usize| -> Result<u32> {
        u32::try_from(i).map_err(|_| Error::Param("el ráster tiene demasiadas celdas".into()))
    };
    for i in 0..n {
        if filled[i].is_nan() {
            seen[i] = true;
            continue;
        }
        let outlet = (0..8).any(|k| neighbour(g, i, k).is_none_or(|j| filled[j].is_nan()));
        if outlet {
            seen[i] = true;
            heap.push(Reverse((key(filled[i]), index(i)?)));
        }
    }
    let mut popped = 0usize;
    while let Some(i) = flat
        .pop_front()
        .map(|i| i as usize)
        .or_else(|| heap.pop().map(|Reverse((_, i))| i as usize))
    {
        popped += 1;
        if popped.is_multiple_of(1 << 20) {
            crate::check_cancel()?;
        }
        let zi = filled[i];
        for k in 0..8 {
            let Some(j) = neighbour(g, i, k) else {
                continue;
            };
            if seen[j] {
                continue;
            }
            seen[j] = true;
            back[j] = ((k + 4) % 8) as u8;
            if filled[j] <= zi {
                filled[j] = zi;
                flat.push_back(index(j)?);
            } else {
                heap.push(Reverse((key(filled[j]), index(j)?)));
            }
        }
    }
    // descenso más pronunciado sobre la superficie rellena; las celdas sin nada más bajo siguen el
    // camino de vuelta
    let mut dir = vec![NONE; n];
    dir.par_iter_mut().enumerate().for_each(|(i, d)| {
        if filled[i].is_nan() {
            return;
        }
        let mut best = (0.0f64, NONE);
        for k in 0..8 {
            let Some(j) = neighbour(g, i, k) else {
                continue;
            };
            if filled[j].is_nan() || filled[j] >= filled[i] {
                continue;
            }
            let s = f64::from(filled[i] - filled[j]) / reach(k);
            if s > best.0 {
                best = (s, k as u8);
            }
        }
        *d = if best.1 == NONE { back[i] } else { best.1 };
    });
    Ok(Drainage { filled, dir })
}

pub fn fill_depth(g: &Grid, z: f64) -> Result<Grid> {
    let mut out = g.like();
    let d = drainage(g, z)?;
    for (o, (f, v)) in out.data.iter_mut().zip(d.filled.iter().zip(&g.data)) {
        if !f.is_nan() {
            *o = (f64::from(*f) - f64::from(*v) * z) as f32;
        }
    }
    Ok(out)
}

/// Celdas que drenan por cada celda (ella misma incluida) y el orden, aguas arriba primero.
pub(crate) fn accumulate(g: &Grid, dir: &[u8]) -> (Vec<u32>, Vec<u32>) {
    let n = dir.len();
    let mut indeg = vec![0u8; n];
    for (i, d) in dir.iter().enumerate() {
        if *d != NONE
            && let Some(j) = neighbour(g, i, usize::from(*d))
        {
            indeg[j] += 1;
        }
    }
    let mut acc: Vec<u32> = g.data.iter().map(|v| u32::from(!v.is_nan())).collect();
    let mut order: Vec<u32> = Vec::with_capacity(n);
    let mut stack: Vec<u32> = (0..n)
        .filter(|i| !g.data[*i].is_nan() && indeg[*i] == 0)
        .map(|i| i as u32)
        .collect();
    while let Some(i) = stack.pop() {
        order.push(i);
        let i = i as usize;
        let d = dir[i];
        if d == NONE {
            continue;
        }
        if let Some(j) = neighbour(g, i, usize::from(d)) {
            acc[j] = acc[j].saturating_add(acc[i]);
            indeg[j] -= 1;
            if indeg[j] == 0 {
                stack.push(j as u32);
            }
        }
    }
    (acc, order)
}

/// Superficie drenada por cada celda, m².
pub fn flow_accumulation(g: &Grid, z: f64) -> Result<Grid> {
    let mut out = g.like();
    let d = drainage(g, z)?;
    let (acc, _) = accumulate(g, &d.dir);
    let a = g.cell * g.cell;
    for (o, (n, v)) in out.data.iter_mut().zip(acc.iter().zip(&g.data)) {
        if !v.is_nan() {
            *o = (f64::from(*n) * a) as f32;
        }
    }
    Ok(out)
}

/// `ln(a / tan β)`, con `a` la superficie drenada por metro de curva de nivel y `β` la pendiente de
/// la dirección de drenaje (nunca más llana que 0,1 %, para que los llanos no se disparen).
pub fn wetness(g: &Grid, z: f64) -> Result<Grid> {
    let mut out = g.like();
    let d = drainage(g, z)?;
    let (acc, _) = accumulate(g, &d.dir);
    let cell = g.cell;
    out.data.par_iter_mut().enumerate().for_each(|(i, o)| {
        if g.data[i].is_nan() {
            return;
        }
        let tan = match d.dir[i] {
            NONE => 0.0,
            k => neighbour(g, i, usize::from(k)).map_or(0.0, |j| {
                f64::from(d.filled[i] - d.filled[j]) / (reach(usize::from(k)) * cell)
            }),
        }
        .max(0.001);
        let a = f64::from(acc[i]) * cell;
        *o = (a / tan).ln() as f32;
    });
    Ok(out)
}

pub(crate) fn threshold_cells(g: &Grid, area: f64) -> Result<u32> {
    if !(area > 0.0) {
        return Err(Error::Param(
            "el área mínima de la red debe ser positiva".into(),
        ));
    }
    Ok((area / (g.cell * g.cell))
        .ceil()
        .clamp(1.0, f64::from(u32::MAX)) as u32)
}

/// 1 donde la superficie drenada alcanza `area` m², vacío en el resto.
pub fn streams(g: &Grid, z: f64, area: f64) -> Result<Grid> {
    let t = threshold_cells(g, area)?;
    let d = drainage(g, z)?;
    let (acc, _) = accumulate(g, &d.dir);
    let mut out = g.like();
    for (o, (n, v)) in out.data.iter_mut().zip(acc.iter().zip(&g.data)) {
        if !v.is_nan() && *n >= t {
            *o = 1.0;
        }
    }
    Ok(out)
}

/// La cuenca de cada tramo de la red de cauces, numerada desde 1. Un tramo va de un nacimiento o
/// una confluencia a la siguiente confluencia; cada celda toma el número del tramo en que drena.
/// Las celdas que salen del mapa sin encontrar un cauce quedan vacías.
pub fn subbasins(g: &Grid, z: f64, area: f64) -> Result<Grid> {
    let t = threshold_cells(g, area)?;
    let d = drainage(g, z)?;
    let (acc, order) = accumulate(g, &d.dir);
    let n = g.data.len();
    let stream = |i: usize| acc[i] >= t && !g.data[i].is_nan();
    let down = |i: usize| match d.dir[i] {
        NONE => None,
        k => neighbour(g, i, usize::from(k)),
    };
    // cauces que desembocan en cada celda de cauce
    let mut inflow = vec![0u8; n];
    for i in 0..n {
        if stream(i)
            && let Some(j) = down(i)
            && stream(j)
        {
            inflow[j] += 1;
        }
    }
    let mut id = vec![0u32; n];
    let mut next = 1u32;
    for &i in &order {
        let i = i as usize;
        if !stream(i) {
            continue;
        }
        if inflow[i] != 1 {
            id[i] = next;
            next += 1;
        }
        if let Some(j) = down(i)
            && stream(j)
            && inflow[j] == 1
        {
            id[j] = id[i];
        }
    }
    // aguas abajo primero, para que la celda de abajo ya esté numerada
    for &i in order.iter().rev() {
        let i = i as usize;
        if !stream(i)
            && let Some(j) = down(i)
        {
            id[i] = id[j];
        }
    }
    let mut out = g.like();
    for (o, v) in out.data.iter_mut().zip(&id) {
        if *v != 0 {
            *o = *v as f32;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&[f32]]) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 1.0, rows[0].len(), rows.len())?;
        for (r, row) in rows.iter().enumerate() {
            g.data[r * rows[0].len()..(r + 1) * rows[0].len()].copy_from_slice(row);
        }
        Ok(g)
    }

    #[test]
    fn a_pit_is_filled_to_its_lowest_exit() -> Result<()> {
        let g = grid(&[
            &[9., 9., 9., 9., 9.],
            &[9., 5., 5., 5., 9.],
            &[9., 5., 1., 5., 3.],
            &[9., 5., 5., 5., 9.],
            &[9., 9., 9., 9., 9.],
        ])?;
        let d = drainage(&g, 1.0)?;
        // el cuenco desborda por el 3 del borde este
        assert_eq!(d.filled[2 * 5 + 2], 5.0);
        assert_eq!(d.filled[2 * 5 + 4], 3.0);
        let depth = fill_depth(&g, 1.0)?;
        assert_eq!(depth.data[2 * 5 + 2], 4.0);
        assert_eq!(depth.data[0], 0.0);
        Ok(())
    }

    #[test]
    fn every_cell_drains_to_the_outlet_and_nothing_loops() -> Result<()> {
        // una cuenca llana con una sola salida
        let mut g = Grid::new(0.0, 0.0, 1.0, 12, 9)?;
        g.data.fill(5.0);
        g.data[4 * 12 + 11] = 4.0;
        let d = drainage(&g, 1.0)?;
        let (acc, order) = accumulate(&g, &d.dir);
        assert_eq!(order.len(), g.data.len(), "a loop leaves cells unvisited");
        // toda la superficie sale por el borde de la rejilla
        let out: u32 = (0..g.data.len())
            .filter(|i| d.dir[*i] == NONE)
            .map(|i| acc[i])
            .sum();
        assert_eq!(out as usize, g.data.len());
        Ok(())
    }

    #[test]
    fn a_v_valley_collects_its_slopes() -> Result<()> {
        // altura = |x - 5| + 0,1 y: el agua corre hasta el eje x = 5 y baja hasta y = 0
        let mut g = Grid::new(0.0, 0.0, 2.0, 11, 11)?;
        for r in 0..11 {
            for c in 0..11 {
                g.data[r * 11 + c] = ((c as f32 - 5.0).abs()) + 0.1 * (10 - r) as f32;
            }
        }
        let acc = flow_accumulation(&g, 1.0)?;
        // la celda del eje en la última fila ha drenado la mayor parte del mapa (celda de 4 m²)
        let axis = acc.data[10 * 11 + 5];
        let side = acc.data[10 * 11 + 1];
        assert!(axis > 30.0 * 4.0, "{axis}");
        assert!(axis > 5.0 * side, "{axis} {side}");
        let s = streams(&g, 1.0, 4.0 * 20.0)?;
        assert_eq!(s.data[10 * 11 + 5], 1.0);
        assert!(s.data[10 * 11 + 1].is_nan());
        let w = wetness(&g, 1.0)?;
        assert!(w.data[10 * 11 + 5] > w.data[5 * 11 + 1]);
        Ok(())
    }

    #[test]
    fn subbasins_split_at_confluences() -> Result<()> {
        // dos valles (x = 2 y x = 8) que se unen a un tronco en la fila de abajo
        let mut g = Grid::new(0.0, 0.0, 1.0, 11, 12)?;
        for r in 0..12 {
            for c in 0..11 {
                let to_axis = ((c as f32 - 2.0).abs()).min((c as f32 - 8.0).abs());
                g.data[r * 11 + c] = if r == 11 {
                    0.5 * (c as f32 - 5.0).abs()
                } else {
                    1.0 + to_axis + 0.5 * (10 - r) as f32
                };
            }
        }
        let b = subbasins(&g, 1.0, 12.0)?;
        let left = b.data[6 * 11 + 2];
        let right = b.data[6 * 11 + 8];
        assert!(left.is_finite() && right.is_finite());
        assert_ne!(left, right);
        // una celda de ladera pertenece al tramo que tiene al lado
        assert_eq!(b.data[6 * 11 + 1], left);
        assert_eq!(b.data[6 * 11 + 9], right);
        Ok(())
    }
}
