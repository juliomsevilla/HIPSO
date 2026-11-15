//! Líneas de ruptura en un modelo del suelo: líneas donde el terreno cambia bruscamente de
//! pendiente (el borde de una carretera, una zanja, un talud, la orilla de un lago), que los puntos
//! de suelo solos redondean. Cada línea se densifica en vértices separados media celda que se unen
//! a los puntos de suelo de la triangulación, y los puntos de suelo junto a la línea se dejan
//! fuera, de modo que los triángulos siguen la línea en lugar de cruzarla.
//!
//! Las líneas con elevaciones (PolylineZ, PolygonZ) dan sus propias alturas: son líneas duras. Las
//! líneas sin ellas (un arroyo, una zanja dibujada en la ortofoto) toman, en cada vértice, el punto
//! de suelo más bajo a menos de una celda de él, lo que excava el cauce en el modelo.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{Error, Result};

/// Las líneas de ruptura de un trabajo, leídas una vez.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Breaklines {
    /// Cada línea como vértices (x, y, z); z es NaN para las líneas sin elevaciones.
    pub lines: Vec<Vec<[f64; 3]>>,
}

impl Breaklines {
    /// Lee las líneas (y los anillos de polígonos) de un shapefile.
    pub fn read(path: &Path) -> Result<Self> {
        crate::check_vector_format(path)?;
        let d = crate::vector::read_drawing(path)?;
        let lines: Vec<Vec<[f64; 3]>> = d
            .lines
            .into_iter()
            .map(|(l, _)| l)
            .filter(|l| l.len() >= 2)
            .collect();
        if lines.is_empty() {
            return Err(Error::Param(format!(
                "{}: no tiene líneas ni polígonos que usar como líneas de rotura",
                path.display()
            )));
        }
        Ok(Self { lines })
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Si todas las líneas llevan sus propias elevaciones.
    pub fn has_z(&self) -> bool {
        self.lines.iter().flatten().all(|p| p[2].is_finite())
    }

    /// Los vértices de las líneas dentro de `area` (oeste, sur, este, norte), a no más de `step`
    /// unos de otros.
    fn vertices(&self, area: [f64; 4], step: f64) -> Vec<[f64; 3]> {
        let mut out = Vec::new();
        let inside =
            |p: &[f64; 3]| p[0] >= area[0] && p[0] <= area[2] && p[1] >= area[1] && p[1] <= area[3];
        for line in &self.lines {
            for w in line.windows(2) {
                let (a, b) = (w[0], w[1]);
                // un segmento enteramente fuera del área por un lado se salta
                if (a[0] < area[0] && b[0] < area[0])
                    || (a[0] > area[2] && b[0] > area[2])
                    || (a[1] < area[1] && b[1] < area[1])
                    || (a[1] > area[3] && b[1] > area[3])
                {
                    continue;
                }
                let len = (b[0] - a[0]).hypot(b[1] - a[1]);
                let n = ((len / step).ceil() as usize).clamp(1, 1_000_000);
                for k in 0..n {
                    let t = k as f64 / n as f64;
                    let p = [
                        a[0] + (b[0] - a[0]) * t,
                        a[1] + (b[1] - a[1]) * t,
                        a[2] + (b[2] - a[2]) * t,
                    ];
                    if inside(&p) {
                        out.push(p);
                    }
                }
            }
            if let Some(last) = line.last()
                && inside(last)
            {
                out.push(*last);
            }
        }
        out
    }

    /// Añade las líneas a los puntos de suelo de un área para un modelo de celda `cell`: sus
    /// vértices se unen a los puntos y los puntos a menos de media celda de una línea se van.
    /// Devuelve cuántos vértices se añadieron.
    pub fn apply(&self, ground: &mut Vec<[f64; 3]>, area: [f64; 4], cell: f64) -> usize {
        let step = (cell / 2.0).max(0.05);
        let mut verts = self.vertices(area, step);
        if verts.is_empty() {
            return 0;
        }
        let key = |x: f64, y: f64, s: f64| ((x / s).floor() as i64, (y / s).floor() as i64);
        // líneas sin elevaciones: el punto de suelo más bajo a menos de una celda de cada vértice
        if verts.iter().any(|v| !v[2].is_finite()) {
            let mut low: HashMap<(i64, i64), f64> = HashMap::new();
            for p in ground.iter() {
                let k = key(p[0], p[1], cell);
                let e = low.entry(k).or_insert(f64::INFINITY);
                *e = e.min(p[2]);
            }
            for v in verts.iter_mut().filter(|v| !v[2].is_finite()) {
                let (cx, cy) = key(v[0], v[1], cell);
                let mut z = f64::INFINITY;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        if let Some(&h) = low.get(&(cx + dx, cy + dy)) {
                            z = z.min(h);
                        }
                    }
                }
                v[2] = z;
            }
            verts.retain(|v| v[2].is_finite());
        }
        // los puntos de suelo junto a una línea le dejan paso
        let near: HashSet<(i64, i64)> = verts.iter().map(|v| key(v[0], v[1], step)).collect();
        ground.retain(|p| !near.contains(&key(p[0], p[1], step)));
        let n = verts.len();
        ground.extend(verts);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_3d_line_sets_the_heights_along_it() -> Result<()> {
        // suelo llano a 100 m en una rejilla de 1 m; un borde de carretera a 103 m a través de él
        let mut ground: Vec<[f64; 3]> = (0..50)
            .flat_map(|i| (0..50).map(move |j| [f64::from(i) + 0.3, f64::from(j) + 0.3, 100.0]))
            .collect();
        let b = Breaklines {
            lines: vec![vec![[0.0, 25.5, 103.0], [50.0, 25.5, 103.0]]],
        };
        let n = b.apply(&mut ground, [0.0, 0.0, 50.0, 50.0], 1.0);
        assert!(n >= 100, "{n}");
        let mut g = crate::grid::Grid::new(0.0, 0.0, 1.0, 50, 50)?;
        crate::tin::rasterize(&ground, &mut g);
        // las celdas por las que pasa la línea están a su altura; lejos el suelo no se toca
        let at = |x: f64, y: f64| g.sample(x, y).unwrap_or(f32::NAN);
        assert!((at(20.5, 25.5) - 103.0).abs() < 1e-3, "{}", at(20.5, 25.5));
        assert!((at(20.0, 5.0) - 100.0).abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn a_2d_line_takes_the_lowest_ground_near_it() {
        // una zanja: suelo a 100 m salvo una trinchera estrecha a 98 m a lo largo de y = 10
        let mut ground: Vec<[f64; 3]> = (0..20)
            .flat_map(|i| {
                (0..20).map(move |j| {
                    let z = if j == 10 { 98.0 } else { 100.0 };
                    [f64::from(i) + 0.5, f64::from(j) + 0.2, z]
                })
            })
            .collect();
        let b = Breaklines {
            lines: vec![vec![[0.0, 10.4, f64::NAN], [20.0, 10.4, f64::NAN]]],
        };
        b.apply(&mut ground, [0.0, 0.0, 20.0, 20.0], 1.0);
        let on_line: Vec<f64> = ground
            .iter()
            .filter(|p| (p[1] - 10.4).abs() < 1e-9)
            .map(|p| p[2])
            .collect();
        assert!(!on_line.is_empty());
        assert!(on_line.iter().all(|z| (*z - 98.0).abs() < 1e-9));
    }
}
