//! Forma alfa tridimensional de una nube de puntos: la triangulación de Delaunay en 3D (Bowyer
//! 1981, Watson 1981: se insertan los puntos de uno en uno y se rehace la cavidad de tetraedros cuya
//! esfera circunscrita los contiene) y, de ella, el complejo alfa (Edelsbrunner y Mücke 1994): los
//! tetraedros cuya esfera circunscrita tiene un radio menor que `alfa`. Su volumen es el de la forma
//! alfa y su superficie, la de las caras que limitan con el exterior.
//!
//! Con un `alfa` muy grande la forma es la envolvente convexa. Un `alfa` pequeño sigue el contorno de
//! la nube y deja fuera los huecos entre copas, lo que da un volumen de copa más realista que la
//! envolvente.
//!
//! Para que los puntos casi cosféricos o coplanares (una retícula, un plano de suelo) no den
//! cavidades mal formadas, los puntos se mueven una fracción ínfima de la extensión de manera
//! determinista; el error de volumen es del orden de esa fracción.

use std::collections::HashMap;

/// El volumen de la forma alfa y de la envolvente convexa, y la superficie de la forma alfa.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub hull_volume: f64,
    pub alpha_volume: f64,
    pub alpha_area: f64,
    /// Tetraedros del complejo alfa.
    pub tetrahedra: usize,
    /// Puntos usados tras reducir la nube.
    pub points: usize,
}

struct Tet {
    v: [usize; 4],
    c: [f64; 3],
    r2: f64,
    alive: bool,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Centro y radio al cuadrado de la esfera circunscrita; el radio es negativo si el tetraedro es
/// plano (nunca contiene un punto).
fn circumsphere(p: [[f64; 3]; 4]) -> ([f64; 3], f64) {
    let u = sub(p[1], p[0]);
    let v = sub(p[2], p[0]);
    let w = sub(p[3], p[0]);
    let det = dot(u, cross(v, w));
    let scale = dot(u, u).max(dot(v, v)).max(dot(w, w));
    if det.abs() <= 1e-13 * scale * scale.sqrt() || !det.is_finite() {
        return ([0.0; 3], -1.0);
    }
    let (vw, wu, uv) = (cross(v, w), cross(w, u), cross(u, v));
    let (a, b, c) = (dot(u, u), dot(v, v), dot(w, w));
    let x = [
        (a * vw[0] + b * wu[0] + c * uv[0]) / (2.0 * det),
        (a * vw[1] + b * wu[1] + c * uv[1]) / (2.0 * det),
        (a * vw[2] + b * wu[2] + c * uv[2]) / (2.0 * det),
    ];
    ([p[0][0] + x[0], p[0][1] + x[1], p[0][2] + x[2]], dot(x, x))
}

fn volume(p: [[f64; 3]; 4]) -> f64 {
    dot(sub(p[1], p[0]), cross(sub(p[2], p[0]), sub(p[3], p[0]))).abs() / 6.0
}

fn face_area(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let n = cross(sub(b, a), sub(c, a));
    dot(n, n).sqrt() / 2.0
}

/// Deja como mucho `cap` puntos: uno por vóxel, con un vóxel que crece hasta que caben.
fn thinned(pts: &[[f64; 3]], cap: usize) -> Vec<[f64; 3]> {
    if pts.len() <= cap {
        return pts.to_vec();
    }
    let mut size = 0.05;
    loop {
        let mut seen: HashMap<(i64, i64, i64), [f64; 3]> = HashMap::new();
        for p in pts {
            let key = (
                (p[0] / size).floor() as i64,
                (p[1] / size).floor() as i64,
                (p[2] / size).floor() as i64,
            );
            seen.entry(key).or_insert(*p);
        }
        if seen.len() <= cap {
            let mut v: Vec<_> = seen.into_iter().collect();
            v.sort_by_key(|(k, _)| *k);
            return v.into_iter().map(|(_, p)| p).collect();
        }
        size *= 1.4;
    }
}

/// La forma alfa de `pts`; con menos de 4 puntos o todos en un plano, todo vale 0. `cap` limita los
/// puntos que se triangulan (la triangulación crece con ellos).
pub fn shape(pts: &[[f64; 3]], alpha: f64, cap: usize) -> Shape {
    let empty = Shape {
        hull_volume: 0.0,
        alpha_volume: 0.0,
        alpha_area: 0.0,
        tetrahedra: 0,
        points: 0,
    };
    let mut p = thinned(pts, cap.max(8));
    let n = p.len();
    if n < 4 {
        return Shape { points: n, ..empty };
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for q in &p {
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    let ext = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    if !(ext > 0.0) {
        return Shape { points: n, ..empty };
    }
    // un movimiento ínfimo y determinista
    for (i, q) in p.iter_mut().enumerate() {
        let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for c in q.iter_mut() {
            h ^= h >> 29;
            h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
            *c += ((h >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 1e-9 * ext;
        }
    }
    let mid = [
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    ];
    let s = 50.0 * ext;
    for sign in [
        [1.0, 1.0, 1.0],
        [1.0, -1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
    ] {
        p.push([
            mid[0] + s * sign[0],
            mid[1] + s * sign[1],
            mid[2] + s * sign[2],
        ]);
    }
    let corners = |v: [usize; 4]| [p[v[0]], p[v[1]], p[v[2]], p[v[3]]];
    let mut tets: Vec<Tet> = Vec::new();
    let first = [n, n + 1, n + 2, n + 3];
    let (c, r2) = circumsphere(corners(first));
    tets.push(Tet {
        v: first,
        c,
        r2,
        alive: true,
    });
    let mut alive: Vec<usize> = vec![0];
    let sites: Vec<[f64; 3]> = p[..n].to_vec();
    for (i, &q) in sites.iter().enumerate() {
        let mut faces: HashMap<[usize; 3], u32> = HashMap::new();
        let mut bad = Vec::new();
        for &t in &alive {
            let tet = &tets[t];
            let d = sub(q, tet.c);
            if tet.r2 > 0.0 && dot(d, d) < tet.r2 {
                bad.push(t);
            }
        }
        for &t in &bad {
            let v = tets[t].v;
            for skip in 0..4 {
                let mut f: Vec<usize> = (0..4).filter(|&k| k != skip).map(|k| v[k]).collect();
                f.sort_unstable();
                *faces.entry([f[0], f[1], f[2]]).or_insert(0) += 1;
            }
            tets[t].alive = false;
        }
        let first_new = tets.len();
        for (f, count) in faces {
            if count != 1 {
                continue;
            }
            let v = [f[0], f[1], f[2], i];
            let (c, r2) = circumsphere(corners(v));
            tets.push(Tet {
                v,
                c,
                r2,
                alive: true,
            });
        }
        alive.retain(|&t| tets[t].alive);
        alive.extend(first_new..tets.len());
    }
    let mut out = empty;
    out.points = n;
    let mut face_count: HashMap<[usize; 3], u32> = HashMap::new();
    for t in tets
        .iter()
        .filter(|t| t.alive && t.v.iter().all(|&v| v < n))
    {
        let pts4 = corners(t.v);
        let vol = volume(pts4);
        out.hull_volume += vol;
        if t.r2 >= 0.0 && t.r2 < alpha * alpha {
            out.alpha_volume += vol;
            out.tetrahedra += 1;
            for skip in 0..4 {
                let mut f: Vec<usize> = (0..4).filter(|&k| k != skip).map(|k| t.v[k]).collect();
                f.sort_unstable();
                *face_count.entry([f[0], f[1], f[2]]).or_insert(0) += 1;
            }
        }
    }
    out.alpha_area = face_count
        .iter()
        .filter(|(_, c)| **c == 1)
        .map(|(f, _)| face_area(p[f[0]], p[f[1]], p[f[2]]))
        .sum();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lattice(nx: usize, ny: usize, nz: usize, step: f64, at: [f64; 3]) -> Vec<[f64; 3]> {
        let mut v = Vec::new();
        for i in 0..nx {
            for j in 0..ny {
                for k in 0..nz {
                    v.push([
                        at[0] + i as f64 * step,
                        at[1] + j as f64 * step,
                        at[2] + k as f64 * step,
                    ]);
                }
            }
        }
        v
    }

    #[test]
    fn lattice_cube_has_the_volume_of_a_cube() {
        // la retícula es el caso más degenerado: todo cosférico
        let s = shape(&lattice(4, 4, 4, 1.0, [0.0; 3]), f64::INFINITY, 1000);
        assert!((s.hull_volume - 27.0).abs() < 1e-4, "{}", s.hull_volume);
        assert!((s.alpha_volume - 27.0).abs() < 1e-4);
    }

    #[test]
    fn sphere_hull_is_close_to_the_sphere() {
        let n = 600;
        let golden = std::f64::consts::PI * (3.0 - 5.0f64.sqrt());
        let pts: Vec<[f64; 3]> = (0..n)
            .map(|i| {
                let y = 1.0 - 2.0 * (f64::from(i) + 0.5) / f64::from(n);
                let r = (1.0 - y * y).sqrt();
                let t = golden * f64::from(i);
                [5.0 * r * t.cos(), 5.0 * y, 5.0 * r * t.sin()]
            })
            .collect();
        let s = shape(&pts, f64::INFINITY, 1000);
        let exact = 4.0 / 3.0 * std::f64::consts::PI * 125.0;
        assert!(
            s.hull_volume < exact && s.hull_volume > 0.97 * exact,
            "{} {}",
            s.hull_volume,
            exact
        );
        // con un alfa pequeño no queda nada del interior vacío
        let small = shape(&pts, 1.0, 1000);
        assert!(small.alpha_volume < 0.5 * s.hull_volume);
    }

    #[test]
    fn alpha_drops_the_gap_between_two_clusters() {
        let mut pts = lattice(3, 3, 3, 1.0, [0.0; 3]);
        pts.extend(lattice(3, 3, 3, 1.0, [20.0, 0.0, 0.0]));
        let all = shape(&pts, f64::INFINITY, 1000);
        let alpha = shape(&pts, 2.0, 1000);
        assert!(
            (alpha.alpha_volume - 16.0).abs() < 1e-3,
            "{}",
            alpha.alpha_volume
        );
        assert!(all.hull_volume > 5.0 * alpha.alpha_volume);
        // la superficie de dos cubos de lado 2
        assert!(
            (alpha.alpha_area - 48.0).abs() < 1e-3,
            "{}",
            alpha.alpha_area
        );
    }

    #[test]
    fn too_few_or_flat_points_have_no_volume() {
        assert_eq!(
            shape(&[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], 5.0, 100).hull_volume,
            0.0
        );
        let flat = lattice(5, 5, 1, 1.0, [0.0; 3]);
        assert!(shape(&flat, 5.0, 100).hull_volume < 1e-6);
    }

    #[test]
    fn thinning_keeps_the_shape() {
        let pts = lattice(10, 10, 10, 0.1, [0.0; 3]);
        let s = shape(&pts, f64::INFINITY, 300);
        assert!(s.points <= 300);
        // la reducción a un punto por vóxel acorta algo la extensión
        assert!(
            s.hull_volume > 0.4 && s.hull_volume < 0.75,
            "{}",
            s.hull_volume
        );
    }
}
