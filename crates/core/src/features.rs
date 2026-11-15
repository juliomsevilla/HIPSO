//! Características geométricas del entorno de un punto: cómo se disponen los puntos a su alrededor,
//! a partir de los autovalores l1 ≥ l2 ≥ l3 de su covarianza (Demantké et al. 2011; Weinmann et al.
//! 2015).
//!
//! * linealidad (l1 − l2) / l1: cercana a 1 en cables y bordes;
//! * planaridad (l2 − l3) / l1: cercana a 1 en tejados, carreteras y suelo desnudo;
//! * esfericidad l3 / l1: cercana a 1 donde los puntos llenan un volumen (copas);
//! * pendiente: ángulo (grados) entre el plano local y la horizontal, 0 llano, 90 una pared;
//! * rugosidad: desviación típica de la distancia de los vecinos al plano local (m);
//! * recuento: cuántos puntos están dentro del radio, incluido el propio punto.
//!
//! El entorno son los puntos a menos de `radius` (una esfera), hallados en una rejilla hash de
//! celdas de un radio; un punto con menos de [`MIN_NEIGHBOURS`] no tiene características. Solo se
//! dan características a los puntos pedidos, de modo que una regla que mira unos pocos no paga por
//! el resto.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::las::Point;
use crate::tls::eigen;

/// Menor número de puntos en un entorno (el punto incluido) para describir su forma.
pub const MIN_NEIGHBOURS: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Features {
    pub count: u32,
    pub linearity: f32,
    pub planarity: f32,
    pub sphericity: f32,
    pub slope: f32,
    pub roughness: f32,
}

type Cell = (i32, i32, i32);

/// Características de los puntos de `pts` que marca `wanted` (todos los demás reciben `None`), con
/// sus vecinos tomados de todo punto que no es ruido.
pub fn compute(pts: &[Point], wanted: &[bool], radius: f64) -> Vec<Option<Features>> {
    let mut out = vec![None; pts.len()];
    if !(radius > 0.0) || !wanted.iter().any(|w| *w) {
        return out;
    }
    let key = |x: f64, y: f64, z: f64| -> Cell {
        (
            (x / radius).floor() as i32,
            (y / radius).floor() as i32,
            (z / radius).floor() as i32,
        )
    };
    let mut cells: HashMap<Cell, Vec<u32>> = HashMap::new();
    for (k, q) in pts.iter().enumerate().filter(|(_, q)| !q.is_noise()) {
        cells.entry(key(q.x, q.y, q.z)).or_default().push(k as u32);
    }
    let r2 = radius * radius;
    let todo: Vec<usize> = (0..pts.len()).filter(|&k| wanted[k]).collect();
    let computed: Vec<Option<Features>> = todo
        .par_iter()
        .map(|&k| {
            let q = &pts[k];
            let (cx, cy, cz) = key(q.x, q.y, q.z);
            let mut nb: Vec<[f64; 3]> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(list) = cells.get(&(cx + dx, cy + dy, cz + dz)) else {
                            continue;
                        };
                        for &j in list {
                            let o = &pts[j as usize];
                            let d = (o.x - q.x).powi(2) + (o.y - q.y).powi(2) + (o.z - q.z).powi(2);
                            if d <= r2 {
                                nb.push([o.x - q.x, o.y - q.y, o.z - q.z]);
                            }
                        }
                    }
                }
            }
            describe(&nb)
        })
        .collect();
    for (k, f) in todo.into_iter().zip(computed) {
        out[k] = f;
    }
    out
}

/// Las características de un entorno dado como desplazamientos desde su centro; `None` cuando es
/// demasiado pequeño.
fn describe(nb: &[[f64; 3]]) -> Option<Features> {
    let n = nb.len();
    if n < MIN_NEIGHBOURS {
        return None;
    }
    let inv = 1.0 / n as f64;
    let mut mean = [0.0; 3];
    for p in nb {
        for k in 0..3 {
            mean[k] += p[k] * inv;
        }
    }
    let mut c = [[0.0; 3]; 3];
    for p in nb {
        let d = [p[0] - mean[0], p[1] - mean[1], p[2] - mean[2]];
        for i in 0..3 {
            for j in i..3 {
                c[i][j] += d[i] * d[j] * inv;
            }
        }
    }
    c[1][0] = c[0][1];
    c[2][0] = c[0][2];
    c[2][1] = c[1][2];
    let (vals, _) = eigen(c);
    let [l1, l2, l3] = [vals[0].max(0.0), vals[1].max(0.0), vals[2].max(0.0)];
    if l1 <= 1e-12 {
        // todos los puntos en un mismo sitio: nada que decir de su forma
        return None;
    }
    let normal = smallest_direction(c, l3);
    Some(Features {
        count: n as u32,
        linearity: ((l1 - l2) / l1) as f32,
        planarity: ((l2 - l3) / l1) as f32,
        sphericity: (l3 / l1) as f32,
        slope: normal[2].abs().clamp(0.0, 1.0).acos().to_degrees() as f32,
        roughness: l3.sqrt() as f32,
    })
}

/// Autovector unitario del menor autovalor `l3` de una matriz simétrica: una dirección en el
/// espacio nulo de `c − l3·I`, el mayor producto vectorial de dos de sus filas.
fn smallest_direction(c: [[f64; 3]; 3], l3: f64) -> [f64; 3] {
    let r = [
        [c[0][0] - l3, c[0][1], c[0][2]],
        [c[1][0], c[1][1] - l3, c[1][2]],
        [c[2][0], c[2][1], c[2][2] - l3],
    ];
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let mut best = [0.0, 0.0, 1.0];
    let mut big = 0.0;
    for (i, j) in [(0, 1), (0, 2), (1, 2)] {
        let v = cross(r[i], r[j]);
        let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if m > big {
            big = m;
            best = [v[0] / m, v[1] / m, v[2] / m];
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64) -> Point {
        Point {
            x,
            y,
            z,
            return_number: 1,
            number_of_returns: 1,
            class: 1,
            ..Default::default()
        }
    }

    fn one(pts: &[Point], radius: f64) -> Features {
        let mut wanted = vec![false; pts.len()];
        wanted[0] = true;
        compute(pts, &wanted, radius)[0].unwrap_or_default()
    }

    #[test]
    fn a_flat_patch_is_planar_with_no_slope() {
        let mut pts = vec![pt(0.0, 0.0, 0.0)];
        for i in -5..=5 {
            for j in -5..=5 {
                if i != 0 || j != 0 {
                    pts.push(pt(f64::from(i) * 0.1, f64::from(j) * 0.1, 0.0));
                }
            }
        }
        let f = one(&pts, 0.6);
        assert!(f.planarity > 0.95, "{f:?}");
        assert!(
            f.slope < 1.0 && f.roughness < 1e-3 && f.linearity < 0.05,
            "{f:?}"
        );
    }

    #[test]
    fn a_tilted_plane_has_its_slope_and_a_wall_is_vertical() {
        let slope = 30f64.to_radians();
        let mut tilted = vec![pt(0.0, 0.0, 0.0)];
        let mut wall = vec![pt(0.0, 0.0, 0.0)];
        for i in -5..=5 {
            for j in -5..=5 {
                if i != 0 || j != 0 {
                    let (u, v) = (f64::from(i) * 0.1, f64::from(j) * 0.1);
                    tilted.push(pt(u, v, u * slope.tan()));
                    wall.push(pt(u, 0.0, v));
                }
            }
        }
        let t = one(&tilted, 0.6);
        assert!((f64::from(t.slope) - 30.0).abs() < 1.0, "{t:?}");
        let w = one(&wall, 0.6);
        assert!(w.slope > 89.0 && w.planarity > 0.9, "{w:?}");
    }

    #[test]
    fn a_wire_is_linear_and_a_ball_is_spherical() {
        let mut wire = vec![pt(0.0, 0.0, 0.0)];
        for i in 1..=20 {
            wire.push(pt(f64::from(i) * 0.05, 0.0, 0.0));
            wire.push(pt(-f64::from(i) * 0.05, 0.0, 0.0));
        }
        let w = one(&wire, 1.2);
        assert!(w.linearity > 0.95 && w.planarity < 0.1, "{w:?}");
        // pseudo-random filled cube
        let mut ball = vec![pt(0.0, 0.0, 0.0)];
        let mut s = 12345u64;
        let mut next = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as f64 / f64::from(1u32 << 31)) - 0.5
        };
        for _ in 0..400 {
            ball.push(pt(next(), next(), next()));
        }
        let b = one(&ball, 2.0);
        assert!(b.sphericity > 0.7 && b.linearity < 0.3, "{b:?}");
    }

    #[test]
    fn few_neighbours_or_noise_give_nothing() {
        let pts = [pt(0.0, 0.0, 0.0), pt(0.1, 0.0, 0.0), pt(5.0, 5.0, 5.0)];
        let mut wanted = vec![false; 3];
        wanted[0] = true;
        assert_eq!(compute(&pts, &wanted, 1.0)[0], None);
        // solo los puntos pedidos reciben características
        assert!(compute(&pts, &[false; 3], 1.0).iter().all(Option::is_none));
    }
}
