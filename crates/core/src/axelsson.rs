//! Suelo por densificación progresiva de TIN (Axelsson 2000, «Adaptive surface models for the
//! filtering of airborne laser scanner data», y los muchos filtros construidos sobre él).
//!
//! Se hace un TIN disperso con puntos semilla: el más bajo de cada celda grande, de modo que ningún
//! edificio cabe en una celda y toda celda tiene un punto en el suelo. Luego, una y otra vez, cada
//! punto dentro de un triángulo del TIN se comprueba contra el plano del triángulo: es suelo cuando
//! está cerca del plano (la distancia de iteración) y las líneas de él a los tres vértices forman
//! ángulos pequeños con el plano (el ángulo de iteración). Los puntos aceptados se añaden al TIN,
//! que se rehace, hasta que no se acepta ninguno. El suelo empinado recibe puntos en unos pocos
//! pasos, donde el ángulo permitido es pequeño en cada uno.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::las::Point;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct AxelssonParams {
    /// Lado (m) de las celdas cuyos puntos más bajos siembran el TIN: mayor que el mayor edificio o
    /// estructura de la zona que no es suelo.
    pub seed_cell: f64,
    /// Mayor ángulo (grados) entre el plano de un triángulo y la línea de un vértice a un punto que
    /// se añade al suelo.
    pub iter_angle: f64,
    /// Mayor distancia (m) del plano de un triángulo a un punto que se añade.
    pub iter_dist: f64,
    /// Los puntos tan cerca del plano (m, ruido del sensor) son suelo sea cual sea el ángulo, que
    /// cerca de un vértice es grande incluso para un punto que está sobre la superficie.
    pub flat_tolerance: f64,
    /// Los triángulos cuya arista más larga es más corta que esto (m) tienen el ángulo reducido en
    /// proporción (hasta la mitad), de modo que un TIN denso no acoge la vegetación baja.
    pub reduce_edge: f64,
    /// Los triángulos cuya arista más larga es más corta que esto (m) no toman más puntos: la
    /// superficie allí es suficientemente densa (0: siguen hasta que no se añade nada).
    pub min_edge: f64,
    pub max_iterations: u32,
}

impl Default for AxelssonParams {
    fn default() -> Self {
        Self {
            seed_cell: 20.0,
            iter_angle: 8.0,
            iter_dist: 1.4,
            flat_tolerance: 0.10,
            reduce_edge: 5.0,
            min_edge: 0.0,
            max_iterations: 30,
        }
    }
}

impl AxelssonParams {
    fn validate(&self) -> Result<()> {
        if !(self.seed_cell > 0.0)
            || !(self.iter_angle > 0.0 && self.iter_angle < 90.0)
            || !(self.iter_dist > 0.0)
            || !(self.flat_tolerance >= 0.0)
            || !(self.reduce_edge >= 0.0)
            || !(self.min_edge >= 0.0)
            || self.max_iterations == 0
        {
            return Err(Error::Param(
                "the progressive TIN filter needs a positive seed cell, angle and distance, \
                 and at least 1 iteration"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Un vértice del TIN.
#[derive(Clone, Copy)]
struct V {
    x: f64,
    y: f64,
    z: f64,
}

/// Los puntos candidatos repartidos en una rejilla regular, de modo que los que están dentro de un
/// triángulo se encuentran mirando las celdas que cubre su caja.
struct Buckets {
    cell: f64,
    cols: usize,
    rows: usize,
    start: Vec<u32>,
    items: Vec<u32>,
}

impl Buckets {
    fn new(xy: &[(f64, f64)], ids: &[usize], extent: (f64, f64)) -> Self {
        // unos 3 puntos por celda
        let cell = ((extent.0 * extent.1 * 3.0 / ids.len().max(1) as f64).sqrt()).max(0.25);
        let cols = ((extent.0 / cell).floor() as usize + 1).min(1 << 14);
        let rows = ((extent.1 / cell).floor() as usize + 1).min(1 << 14);
        let at = |i: usize| -> usize {
            let (x, y) = xy[i];
            let c = ((x / cell).max(0.0) as usize).min(cols - 1);
            let r = ((y / cell).max(0.0) as usize).min(rows - 1);
            r * cols + c
        };
        let mut start = vec![0u32; cols * rows + 1];
        for &i in ids {
            start[at(i) + 1] += 1;
        }
        for k in 0..cols * rows {
            start[k + 1] += start[k];
        }
        let mut fill = start.clone();
        let mut items = vec![0u32; ids.len()];
        for &i in ids {
            let slot = &mut fill[at(i)];
            items[*slot as usize] = i as u32;
            *slot += 1;
        }
        Self {
            cell,
            cols,
            rows,
            start,
            items,
        }
    }

    /// Los candidatos de las celdas que cubre la caja.
    fn in_box(&self, lo: (f64, f64), hi: (f64, f64)) -> impl Iterator<Item = usize> + '_ {
        let c0 = ((lo.0 / self.cell).max(0.0) as usize).min(self.cols - 1);
        let c1 = ((hi.0 / self.cell).max(0.0) as usize).min(self.cols - 1);
        let r0 = ((lo.1 / self.cell).max(0.0) as usize).min(self.rows - 1);
        let r1 = ((hi.1 / self.cell).max(0.0) as usize).min(self.rows - 1);
        (r0..=r1).flat_map(move |r| {
            let (a, b) = (
                self.start[r * self.cols + c0] as usize,
                self.start[r * self.cols + c1 + 1] as usize,
            );
            self.items[a..b].iter().map(|&i| i as usize)
        })
    }
}

/// Los candidatos de un triángulo que son suelo: cerca de su plano, con ángulos pequeños a los
/// vértices.
fn accepted(
    tri: [V; 3],
    xy: &[(f64, f64)],
    z: &[f64],
    buckets: &Buckets,
    p: &AxelssonParams,
) -> Vec<usize> {
    let [a, b, c] = tri;
    let edge = |u: V, v: V| (u.x - v.x).hypot(u.y - v.y);
    let longest = edge(a, b).max(edge(b, c)).max(edge(c, a));
    if longest < p.min_edge {
        return Vec::new();
    }
    // el plano por los vértices: una normal n; uno vertical no tiene altura que comparar
    let (ux, uy, uz) = (b.x - a.x, b.y - a.y, b.z - a.z);
    let (vx, vy, vz) = (c.x - a.x, c.y - a.y, c.z - a.z);
    let n = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    if n.2.abs() < 1e-9 {
        return Vec::new();
    }
    let norm = (n.0 * n.0 + n.1 * n.1 + n.2 * n.2).sqrt();
    let reduce = if longest < p.reduce_edge && p.reduce_edge > 0.0 {
        (longest / p.reduce_edge).clamp(0.5, 1.0)
    } else {
        1.0
    };
    let max_angle = (p.iter_angle * reduce).to_radians();
    let lo = (a.x.min(b.x).min(c.x), a.y.min(b.y).min(c.y));
    let hi = (a.x.max(b.x).max(c.x), a.y.max(b.y).max(c.y));
    let area = ux * vy - uy * vx;
    let mut out = Vec::new();
    for q in buckets.in_box(lo, hi) {
        let (qx, qy) = xy[q];
        // dentro del triángulo: los tres pesos baricéntricos no son negativos
        let l1 = ((b.y - c.y) * (qx - c.x) + (c.x - b.x) * (qy - c.y)) / area;
        let l2 = ((c.y - a.y) * (qx - c.x) + (a.x - c.x) * (qy - c.y)) / area;
        if l1 < -1e-9 || l2 < -1e-9 || 1.0 - l1 - l2 < -1e-9 {
            continue;
        }
        // distancia al plano, medida perpendicularmente a él
        let zp = a.z - (n.0 * (qx - a.x) + n.1 * (qy - a.y)) / n.2;
        let d = (z[q] - zp) * n.2.abs() / norm;
        if d.abs() <= p.flat_tolerance {
            out.push(q);
            continue;
        }
        if d.abs() > p.iter_dist {
            continue;
        }
        let steep = [a, b, c].iter().any(|v| {
            let r = ((qx - v.x).powi(2) + (qy - v.y).powi(2) + (z[q] - v.z).powi(2)).sqrt();
            r > 1e-9 && (d.abs() / r).min(1.0).asin() > max_angle
        });
        if !steep {
            out.push(q);
        }
    }
    out
}

/// Qué puntos son suelo. El ruido y los puntos retenidos nunca son suelo y no participan.
pub fn ground(pts: &[Point], p: &AxelssonParams) -> Result<Vec<bool>> {
    p.validate()?;
    let n = pts.len();
    let mut ground = vec![false; n];
    let valid: Vec<usize> = (0..n)
        .filter(|&i| !pts[i].is_noise() && pts[i].x.is_finite() && pts[i].y.is_finite())
        .collect();
    if valid.len() < 3 {
        return Ok(ground);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &i in &valid {
        (x0, y0, x1, y1) = (
            x0.min(pts[i].x),
            y0.min(pts[i].y),
            x1.max(pts[i].x),
            y1.max(pts[i].y),
        );
    }
    // las coordenadas relativas a la esquina mantienen precisa la triangulación
    let xy: Vec<(f64, f64)> = pts.iter().map(|q| (q.x - x0, q.y - y0)).collect();
    let z: Vec<f64> = pts.iter().map(|q| q.z).collect();
    let extent = (x1 - x0, y1 - y0);
    // las semillas: el punto más bajo de cada celda
    let mut lowest: HashMap<(i64, i64), usize> = HashMap::new();
    for &i in &valid {
        let key = (
            (xy[i].0 / p.seed_cell).floor() as i64,
            (xy[i].1 / p.seed_cell).floor() as i64,
        );
        lowest
            .entry(key)
            .and_modify(|j| {
                if z[i] < z[*j] {
                    *j = i;
                }
            })
            .or_insert(i);
    }
    for &i in lowest.values() {
        ground[i] = true;
    }
    // Las semillas interiores a sus celdas dejan el borde de la nube fuera de su envolvente, donde
    // ningún triángulo comprueba un punto: a lo largo de cada lado de la extensión, el punto más
    // bajo de cada tramo de media celda de semilla, tomado en una banda en el borde, es también una
    // semilla.
    let band = (p.seed_cell / 10.0).max(1.0);
    let stretch = p.seed_cell / 2.0;
    let mut sides: HashMap<(u8, i64), usize> = HashMap::new();
    for &i in &valid {
        let (x, y) = xy[i];
        for (side, near, along) in [
            (0u8, x <= band, y),
            (1, extent.0 - x <= band, y),
            (2, y <= band, x),
            (3, extent.1 - y <= band, x),
        ] {
            if near {
                sides
                    .entry((side, (along / stretch).floor() as i64))
                    .and_modify(|j| {
                        if z[i] < z[*j] {
                            *j = i;
                        }
                    })
                    .or_insert(i);
            }
        }
    }
    for &i in sides.values() {
        ground[i] = true;
    }
    let mut candidates: Vec<usize> = valid.iter().copied().filter(|&i| !ground[i]).collect();
    for _ in 0..p.max_iterations {
        crate::check_cancel()?;
        if candidates.is_empty() {
            break;
        }
        let ids: Vec<usize> = valid.iter().copied().filter(|&i| ground[i]).collect();
        if ids.len() < 3 {
            break;
        }
        let mut corners: Vec<delaunator::Point> = ids
            .iter()
            .map(|&i| delaunator::Point {
                x: xy[i].0,
                y: xy[i].1,
            })
            .collect();
        // Las semillas son interiores a sus celdas, así que su envolvente deja una franja a lo
        // largo del borde de la nube que ningún triángulo cubre: cuatro vértices virtuales en las
        // esquinas de la extensión, a la altura del punto de suelo más cercano, extienden el TIN
        // sobre ella. Solo dan planos contra los que comprobar los puntos de la franja y nunca son
        // suelo ellos mismos.
        let real = ids.len();
        let mut corner_z = [0.0f64; 4];
        for (c, (cx, cy)) in [
            (0.0, 0.0),
            (extent.0, 0.0),
            (extent.0, extent.1),
            (0.0, extent.1),
        ]
        .into_iter()
        .enumerate()
        {
            let nearest = ids
                .iter()
                .min_by(|&&i, &&j| {
                    let di = (xy[i].0 - cx).powi(2) + (xy[i].1 - cy).powi(2);
                    let dj = (xy[j].0 - cx).powi(2) + (xy[j].1 - cy).powi(2);
                    di.total_cmp(&dj)
                })
                .copied();
            corner_z[c] = nearest.map_or(0.0, |i| z[i]);
            corners.push(delaunator::Point { x: cx, y: cy });
        }
        let t = delaunator::triangulate(&corners);
        if t.triangles.is_empty() {
            break;
        }
        let buckets = Buckets::new(&xy, &candidates, extent);
        let vertex = |k: usize| V {
            x: corners[k].x,
            y: corners[k].y,
            z: if k < real { z[ids[k]] } else { corner_z[k - real] },
        };
        let found: Vec<usize> = t
            .triangles
            .par_chunks_exact(3)
            .flat_map_iter(|tri| {
                accepted(
                    [vertex(tri[0]), vertex(tri[1]), vertex(tri[2])],
                    &xy,
                    &z,
                    &buckets,
                    p,
                )
            })
            .collect();
        let mut added = 0usize;
        for i in found {
            if !ground[i] {
                ground[i] = true;
                added += 1;
            }
        }
        if added == 0 {
            break;
        }
        candidates.retain(|&i| !ground[i]);
    }
    Ok(ground)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64) -> Point {
        Point {
            x,
            y,
            z,
            ..Default::default()
        }
    }

    /// Terreno en pendiente con una ligera ondulación, un edificio con tejado plano y árboles.
    fn scene() -> (Vec<Point>, Vec<bool>) {
        let terrain = |x: f64, y: f64| 0.2 * x + 0.1 * y + 0.4 * (x / 9.0).sin();
        let mut pts = Vec::new();
        let mut is_ground = Vec::new();
        let step = 0.5;
        let mut x = 0.0;
        while x <= 100.0 {
            let mut y = 0.0;
            while y <= 100.0 {
                let in_building = (40.0..58.0).contains(&x) && (40.0..52.0).contains(&y);
                let in_tree = [(15.0, 20.0), (75.0, 30.0), (30.0, 80.0), (80.0, 80.0)]
                    .iter()
                    .any(|t: &(f64, f64)| (x - t.0).hypot(y - t.1) < 3.0);
                if in_building {
                    // solo el tejado, a 6 m sobre el suelo
                    pts.push(pt(x, y, terrain(x, y) + 6.0));
                    is_ground.push(false);
                } else {
                    pts.push(pt(x, y, terrain(x, y)));
                    is_ground.push(true);
                    if in_tree {
                        // una copa: puntos a 6 a 12 m sobre el suelo
                        for k in 0..6 {
                            pts.push(pt(x, y, terrain(x, y) + 6.0 + f64::from(k)));
                            is_ground.push(false);
                        }
                    }
                }
                y += step;
            }
            x += step;
        }
        (pts, is_ground)
    }

    #[test]
    fn it_takes_the_ground_of_a_slope_and_leaves_roofs_and_crowns() -> Result<()> {
        let (pts, is_ground) = scene();
        let g = ground(&pts, &AxelssonParams::default())?;
        let (mut hit, mut total, mut wrong, mut others) = (0, 0, 0, 0);
        for (k, truth) in is_ground.iter().enumerate() {
            if *truth {
                total += 1;
                hit += usize::from(g[k]);
            } else {
                others += 1;
                wrong += usize::from(g[k]);
            }
        }
        // casi todo el suelo y ningún punto de tejado ni de copa
        assert!(hit as f64 / total as f64 > 0.99, "{hit} of {total}");
        assert_eq!(
            wrong, 0,
            "{wrong} of {others} points above the ground taken"
        );
        Ok(())
    }

    #[test]
    fn a_building_larger_than_the_seed_cell_is_taken_for_ground() -> Result<()> {
        // la celda de semilla es menor que el tejado de 18 x 12 m, así que una celda puede contener
        // solo puntos de tejado
        let (pts, is_ground) = scene();
        let small = AxelssonParams {
            seed_cell: 5.0,
            ..AxelssonParams::default()
        };
        let g = ground(&pts, &small)?;
        let roof_taken = is_ground
            .iter()
            .zip(&g)
            .zip(&pts)
            .filter(|((truth, got), q)| !**truth && **got && (40.0..58.0).contains(&q.x))
            .count();
        assert!(roof_taken > 0);
        Ok(())
    }

    #[test]
    fn the_strip_along_the_edge_that_the_seeds_do_not_cover_is_ground_too() -> Result<()> {
        // suelo llano en un retículo de 0,5 m sobre 50 x 50 m con celdas de semilla de 20 m: el
        // punto más bajo de cada celda está dentro de ella, así que la envolvente de las semillas
        // deja una franja a lo largo del borde
        let mut pts = Vec::new();
        let mut x = 0.0;
        while x <= 50.0 {
            let mut y = 0.0;
            while y <= 50.0 {
                // un poco de relieve para que el punto más bajo de una celda no esté en su esquina
                pts.push(pt(x, y, 0.05 * ((x * 0.7).sin() + (y * 0.9).cos())));
                y += 0.5;
            }
            x += 0.5;
        }
        let g = ground(&pts, &AxelssonParams::default())?;
        let edge = |q: &Point| q.x < 8.0 || q.y < 8.0 || q.x > 42.0 || q.y > 42.0;
        let (taken, total) = pts
            .iter()
            .zip(&g)
            .filter(|(q, _)| edge(q))
            .fold((0, 0), |(t, n), (_, g)| (t + usize::from(*g), n + 1));
        assert!(
            taken as f64 / total as f64 > 0.99,
            "{taken} of {total} edge points taken"
        );
        Ok(())
    }

    #[test]
    fn noise_and_a_cloud_too_small_to_triangulate_are_left_out() -> Result<()> {
        assert_eq!(
            ground(&[pt(0.0, 0.0, 0.0)], &AxelssonParams::default())?,
            [false]
        );
        let bad = AxelssonParams {
            iter_angle: 0.0,
            ..AxelssonParams::default()
        };
        assert!(matches!(ground(&[], &bad), Err(Error::Param(_))));
        Ok(())
    }
}
