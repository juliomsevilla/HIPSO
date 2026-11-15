//! El terreno que cubre de verdad una nube de puntos. Las superficies se rejillan sobre la caja
//! envolvente de la nube, y rellenar sus celdas vacías inventaría valores donde la nube no tiene
//! puntos (las esquinas de un bloque girado, un hueco entre pasadas). La huella registra dónde caen
//! los puntos, en una rejilla gruesa y cerrada sobre huecos pequeños, de modo que una superficie se
//! pueda recortar a ella.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::grid::Grid;

/// Celdas de la rejilla de huella, en celdas de rejilla de la superficie; al menos `MIN_SIZE`
/// unidades del mapa.
const SIZE_IN_CELLS: f64 = 6.0;
const MIN_SIZE: f64 = 3.0;
/// Celdas gruesas que puede abarcar un hueco y aun así contar como cubierto (cierre por dilatación
/// y erosión).
const CLOSE: usize = 2;
/// Máximo de celdas gruesas que guarda una huella.
const MAX_CELLS: usize = 1 << 24;

/// Qué celdas gruesas de un área contienen puntos. Compartida entre hilos: `add` toma `&self`.
pub struct Footprint {
    x0: f64,
    y0: f64,
    size: f64,
    cols: usize,
    rows: usize,
    seen: Vec<AtomicBool>,
}

impl Footprint {
    /// Una huella sobre `[x mínima, y mínima, x máxima, y máxima]` para superficies de `cell`.
    pub fn new(b: [f64; 4], cell: f64) -> Self {
        let mut size = (cell * SIZE_IN_CELLS).max(MIN_SIZE);
        let dims = |s: f64| {
            (
                ((b[2] - b[0]) / s).floor() as usize + 1,
                ((b[3] - b[1]) / s).floor() as usize + 1,
            )
        };
        let (mut cols, mut rows) = dims(size);
        while cols.saturating_mul(rows) > MAX_CELLS {
            size *= 2.0;
            (cols, rows) = dims(size);
        }
        Self {
            x0: b[0],
            y0: b[1],
            size,
            cols,
            rows,
            seen: (0..cols * rows).map(|_| AtomicBool::new(false)).collect(),
        }
    }

    /// Marca la celda gruesa que contiene `(x, y)`.
    #[inline]
    pub fn add(&self, x: f64, y: f64) {
        let c = ((x - self.x0) / self.size).floor();
        let r = ((y - self.y0) / self.size).floor();
        if c >= 0.0
            && r >= 0.0
            && c < self.cols as f64
            && r < self.rows as f64
            && let Some(s) = self.seen.get(r as usize * self.cols + c as usize)
        {
            s.store(true, Ordering::Relaxed);
        }
    }

    /// Las celdas cubiertas tras salvar los huecos más estrechos que `2 * CLOSE` celdas gruesas;
    /// las celdas más allá del borde del área cuentan como cubiertas mientras se erosiona, para que
    /// no se coma el borde.
    fn closed(&self) -> Vec<bool> {
        let mut m: Vec<bool> = self
            .seen
            .iter()
            .map(|s| s.load(Ordering::Relaxed))
            .collect();
        for _ in 0..CLOSE {
            m = self.morph(&m, true);
        }
        for _ in 0..CLOSE {
            m = self.morph(&m, false);
        }
        m
    }

    /// Un paso de dilatación (`grow`) o de erosión sobre los 8 vecinos.
    fn morph(&self, m: &[bool], grow: bool) -> Vec<bool> {
        let (cols, rows) = (self.cols as isize, self.rows as isize);
        (0..m.len())
            .map(|i| {
                let (c, r) = ((i % self.cols) as isize, (i / self.cols) as isize);
                let at = |dc: isize, dr: isize| {
                    let (nc, nr) = (c + dc, r + dr);
                    if nc < 0 || nr < 0 || nc >= cols || nr >= rows {
                        !grow
                    } else {
                        m.get((nr * cols + nc) as usize).copied().unwrap_or(false)
                    }
                };
                let around = (-1..=1).flat_map(|dr| (-1..=1).map(move |dc| (dc, dr)));
                if grow {
                    around.into_iter().any(|(dc, dr)| at(dc, dr))
                } else {
                    around.into_iter().all(|(dc, dr)| at(dc, dr))
                }
            })
            .collect()
    }

    /// Vacía las celdas de `g` que quedan fuera de la huella.
    pub fn apply(&self, g: &mut Grid) {
        let covered = self.closed();
        for row in 0..g.rows {
            for col in 0..g.cols {
                let (x, y) = g.center(col, row);
                let c = ((x - self.x0) / self.size).floor();
                let r = ((y - self.y0) / self.size).floor();
                let inside = c >= 0.0
                    && r >= 0.0
                    && c < self.cols as f64
                    && r < self.rows as f64
                    && covered
                        .get(r as usize * self.cols + c as usize)
                        .copied()
                        .unwrap_or(false);
                if !inside && let Some(v) = g.data.get_mut(row * g.cols + col) {
                    *v = f32::NAN;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;

    #[test]
    fn a_surface_is_cut_to_the_points_not_to_their_box() -> Result<()> {
        // una L: puntos a lo largo del borde inferior e izquierdo, la esquina superior derecha está
        // vacía
        let fp = Footprint::new([0.0, 0.0, 100.0, 100.0], 1.0);
        for k in 0..100 {
            let t = f64::from(k) + 0.5;
            for w in 0..20 {
                fp.add(t, f64::from(w));
                fp.add(f64::from(w), t);
            }
        }
        let mut g = Grid::covering([0.0, 0.0, 99.0, 99.0], 1.0)?;
        g.data.fill(1.0);
        fp.apply(&mut g);
        assert!(g.sample(5.0, 90.0).is_some());
        assert!(g.sample(90.0, 5.0).is_some());
        assert!(g.sample(90.0, 90.0).is_none());
        Ok(())
    }

    #[test]
    fn a_small_gap_inside_the_cloud_stays_covered() -> Result<()> {
        let fp = Footprint::new([0.0, 0.0, 60.0, 60.0], 1.0);
        for x in 0..60 {
            for y in 0..60 {
                if !(28..32).contains(&x) || !(28..32).contains(&y) {
                    fp.add(f64::from(x) + 0.5, f64::from(y) + 0.5);
                }
            }
        }
        let mut g = Grid::covering([0.0, 0.0, 59.0, 59.0], 1.0)?;
        g.data.fill(1.0);
        fp.apply(&mut g);
        assert!(g.sample(30.0, 30.0).is_some());
        Ok(())
    }
}
