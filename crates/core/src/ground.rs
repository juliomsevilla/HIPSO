//! Extracción de suelo desnudo con el filtro de Kraus y Pfeifer (1998), hecha en pasadas en flujo:
//! cada pasada lee la nube una vez y guarda dos sumas por nodo de la superficie, de modo que la
//! memoria depende del área, nunca del número de puntos.
//!
//! La superficie intermedia es una rejilla de nodos separados `filter_cell`: cada nodo guarda la
//! elevación media ponderada de los puntos más cercanos a él, los huecos cortos se rellenan desde
//! los nodos más cercanos en 8 direcciones y el residuo de un punto es su elevación menos la
//! superficie interpolada bilinealmente bajo él.

use crate::footprint::Footprint;
use crate::grid::{Grid, MeanGrid};
use crate::las::{Cloud, Point};
use crate::{Error, Result};

/// Los parámetros del filtro (valores por defecto).
#[derive(Clone, Debug, PartialEq)]
pub struct GroundParams {
    /// Separación de los nodos de la superficie intermedia.
    pub filter_cell: f64,
    /// Residuo igual o inferior al cual un punto recibe peso completo.
    pub g: f64,
    /// Anchura por encima de `g` en la que el peso cae a 0.
    pub w: f64,
    pub a: f64,
    pub b: f64,
    pub iterations: u32,
    /// Suelo = a esta distancia de la superficie final (por defecto: residuo <= g + w).
    pub tolerance: Option<f64>,
}

impl Default for GroundParams {
    fn default() -> Self {
        Self {
            filter_cell: 5.0,
            g: -2.0,
            w: 2.5,
            a: 1.0,
            b: 4.0,
            iterations: 5,
            tolerance: None,
        }
    }
}

impl GroundParams {
    /// El peso de un residuo `v` (punto menos superficie), por la función de peso del filtro.
    #[inline]
    pub fn weight(&self, v: f64) -> f64 {
        if v <= self.g {
            1.0
        } else if v > self.g + self.w {
            0.0
        } else {
            1.0 / (self.a * (v - self.g).powf(self.b) + 1.0)
        }
    }

    fn validate(&self) -> Result<()> {
        if !(self.filter_cell > 0.0)
            || !(self.w > 0.0)
            || self.iterations == 0
            || !self.a.is_finite()
            || !self.b.is_finite()
        {
            return Err(Error::Param(
                "el filtro de suelo necesita una celda y un w positivos y al menos 1 iteración"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Los huecos de hasta tantos nodos de un nodo con valor se rellenan antes de cada pasada.
const FILL_STEPS: usize = 2;

/// La superficie intermedia: valores de los nodos, NaN donde ningún punto pesó.
struct Surface {
    x0: f64,
    y0: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    z: Vec<f32>,
}

impl Surface {
    /// Nodos desde la esquina suroeste de la nube, que la cubren.
    fn new(b: [f64; 6], cell: f64) -> Result<Self> {
        let cols = ((b[3] - b[0] + cell / 2.0) / cell) as usize + 1;
        let rows = ((b[4] - b[1] + cell / 2.0) / cell) as usize + 1;
        let n = cols
            .checked_mul(rows)
            .filter(|n| *n <= 1 << 29)
            .ok_or_else(|| {
                Error::Param("la celda del filtro es demasiado pequeña para esta zona".into())
            })?;
        Ok(Self {
            x0: b[0],
            y0: b[1],
            cell,
            cols,
            rows,
            z: vec![f32::NAN; n],
        })
    }

    /// El nodo más cercano a un punto (la fila 0 es el sur).
    #[inline]
    fn node(&self, x: f64, y: f64) -> usize {
        let c = (((x - self.x0) / self.cell + 0.5).max(0.0) as usize).min(self.cols - 1);
        let r = (((y - self.y0) / self.cell + 0.5).max(0.0) as usize).min(self.rows - 1);
        r * self.cols + c
    }

    /// La superficie bajo un punto, bilineal entre los 4 nodos de alrededor; None sobre un hueco.
    #[inline]
    fn at(&self, x: f64, y: f64) -> Option<f64> {
        let (fx, fy) = ((x - self.x0) / self.cell, (y - self.y0) / self.cell);
        let (c, r) = (fx.floor(), fy.floor());
        if c < 0.0 || r < 0.0 || c > (self.cols - 1) as f64 || r > (self.rows - 1) as f64 {
            return None;
        }
        let (c, r) = (c as usize, r as usize);
        let (c1, r1) = ((c + 1).min(self.cols - 1), (r + 1).min(self.rows - 1));
        let v = |c: usize, r: usize| self.z.get(r * self.cols + c).copied().map(f64::from);
        let (z00, z10, z01, z11) = (v(c, r)?, v(c1, r)?, v(c, r1)?, v(c1, r1)?);
        if [z00, z10, z01, z11].iter().any(|z| z.is_nan()) {
            return None;
        }
        let (tx, ty) = (fx - c as f64, fy - r as f64);
        let south = z00 + (z10 - z00) * tx;
        let north = z01 + (z11 - z01) * tx;
        Some(south + (north - south) * ty)
    }

    /// Rellena los huecos desde el primer nodo con valor dentro de [`FILL_STEPS`] en cada una de 8
    /// direcciones, ponderado por el inverso de la distancia; los huecos más anchos quedan vacíos.
    fn fill_gaps(&mut self) {
        let snap = self.z.clone();
        let (cols, rows) = (self.cols as isize, self.rows as isize);
        for r in 0..rows {
            for c in 0..cols {
                let i = (r * cols + c) as usize;
                if !snap.get(i).is_some_and(|v| v.is_nan()) {
                    continue;
                }
                let (mut sw, mut swz) = (0.0f32, 0.0f32);
                for (dc, dr) in [
                    (0, -1),
                    (0, 1),
                    (-1, 0),
                    (1, 0),
                    (-1, -1),
                    (1, -1),
                    (-1, 1),
                    (1, 1),
                ] {
                    let unit = if dc != 0 && dr != 0 {
                        std::f32::consts::SQRT_2
                    } else {
                        1.0
                    };
                    for step in 1..=FILL_STEPS as isize {
                        let (nc, nr) = (c + dc * step, r + dr * step);
                        if nc < 0 || nr < 0 || nc >= cols || nr >= rows {
                            break;
                        }
                        if let Some(&v) =
                            snap.get((nr * cols + nc) as usize).filter(|v| !v.is_nan())
                        {
                            let w = 1.0 / (step as f32 * unit);
                            sw += w;
                            swz += w * v;
                            break;
                        }
                    }
                }
                if sw > 0.0
                    && let Some(z) = self.z.get_mut(i)
                {
                    *z = swz / sw;
                }
            }
        }
    }
}

/// Lo que hizo el filtro.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroundReport {
    pub points: u64,
    pub ground: u64,
    pub passes: u32,
}

/// Filtra el suelo y lo rasteriza en un MDT con celdas de `cell`: cada celda es la elevación media
/// de sus puntos de suelo, y las celdas vacías se rellenan desde sus vecinas.
pub fn dtm<C: Cloud>(cloud: &C, cell: f64, p: &GroundParams) -> Result<(Grid, GroundReport)> {
    let (filter, passes) = GroundFilter::run(cloud, p)?;
    let b = cloud.bounds();
    let mut mean = MeanGrid::new(Grid::covering([b[0], b[1], b[3], b[4]], cell)?, b[2])?;
    let fp = Footprint::new([b[0], b[1], b[3], b[4]], cell);
    let mut report = GroundReport {
        passes: passes + 1,
        ..Default::default()
    };
    report.points = cloud.scan(|pt| {
        if !pt.is_noise() {
            fp.add(pt.x, pt.y);
        }
        if !pt.is_noise() && filter.is_ground(pt) {
            mean.add(pt.x, pt.y, pt.z);
            report.ground += 1;
        }
    })?;
    if report.ground == 0 {
        return Err(Error::Param(
            "ningún punto ha quedado como suelo: revisa g, w y la celda del filtro".into(),
        ));
    }
    let mut dtm = mean.finish();
    dtm.fill_holes();
    fp.apply(&mut dtm);
    Ok((dtm, report))
}

/// La superficie final del filtro, que distingue los puntos de suelo.
pub struct GroundFilter {
    s: Surface,
    p: GroundParams,
}

impl GroundFilter {
    /// Ejecuta el filtro sobre una nube; el filtro y cuántas pasadas leyó.
    pub fn run<C: Cloud>(cloud: &C, p: &GroundParams) -> Result<(Self, u32)> {
        p.validate()?;
        let mut s = Surface::new(cloud.bounds(), p.filter_cell)?;
        let mut sum_wz = vec![0.0f64; s.z.len()];
        let mut sum_w = vec![0.0f32; s.z.len()];
        let mut passes = 0;
        // La pasada 0 pesa igual todos los puntos; luego cada pasada vuelve a pesar contra la
        // última superficie.
        for pass in 0..=p.iterations {
            if pass > 0 {
                s.fill_gaps();
            }
            sum_wz.fill(0.0);
            sum_w.fill(0.0);
            cloud.scan(|pt| {
                if pt.is_noise() {
                    return;
                }
                let w = if pass == 0 {
                    1.0
                } else {
                    match s.at(pt.x, pt.y) {
                        Some(sz) => p.weight(pt.z - sz),
                        None => 0.0,
                    }
                };
                if w > 0.0 {
                    let i = s.node(pt.x, pt.y);
                    if let (Some(a), Some(b)) = (sum_wz.get_mut(i), sum_w.get_mut(i)) {
                        *a += w * pt.z;
                        *b += w as f32;
                    }
                }
            })?;
            passes += 1;
            for ((z, a), b) in s.z.iter_mut().zip(&sum_wz).zip(&sum_w) {
                *z = if *b > 0.0 {
                    (*a / f64::from(*b)) as f32
                } else {
                    f32::NAN
                };
            }
        }
        s.fill_gaps();
        Ok((Self { s, p: p.clone() }, passes))
    }

    /// Si un punto está en el suelo: como mucho a g + w por encima de la superficie (o dentro de la
    /// tolerancia de ella).
    #[inline]
    pub fn is_ground(&self, pt: &Point) -> bool {
        match self.s.at(pt.x, pt.y) {
            Some(sz) => match self.p.tolerance {
                Some(t) => (pt.z - sz).abs() <= t,
                None => pt.z - sz <= self.p.g + self.p.w,
            },
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_function_values() {
        let p = GroundParams::default();
        assert_eq!(p.weight(-3.0), 1.0);
        assert_eq!(p.weight(-2.0), 1.0);
        assert_eq!(p.weight(0.6), 0.0);
        // Halfway: 1 / (1 * 1^4 + 1).
        assert!((p.weight(-1.0) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn surface_interpolates_and_fills_short_gaps() -> Result<()> {
        let mut s = Surface::new([0.0, 0.0, 0.0, 20.0, 0.0, 0.0], 5.0)?;
        assert_eq!((s.cols, s.rows), (5, 1));
        s.z = vec![10.0, f32::NAN, f32::NAN, f32::NAN, 20.0];
        s.fill_gaps();
        // El nodo 1 ve el nodo 0 a 1 paso y el nodo 4 a 3 (más allá del alcance); el nodo 2 ve
        // ambos a 2.
        assert_eq!(s.z[1], 10.0);
        assert_eq!(s.z[2], 15.0);
        assert_eq!(s.z[3], 20.0);
        assert_eq!(s.at(2.5, 0.0), Some(10.0));
        assert_eq!(s.node(2.6, 0.0), 1);
        Ok(())
    }
}
