//! Reglas de topología de una capa vectorial: polígonos que no se solapan, que no dejan huecos
//! entre sí y que no se cruzan consigo mismos, y líneas sin cabos sueltos. Cada incumplimiento se
//! devuelve como un `Issue` con el lugar donde ocurre, de modo que se pueda listar, centrar el mapa
//! en él o escribirlo en una tabla.

use geo::{Area, BooleanOps, MultiPolygon};

use crate::view::{GeomKind, VectorData};

/// Regla que se comprueba.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Dos polígonos comparten superficie.
    Overlap,
    /// Un hueco rodeado de polígonos que ninguno cubre.
    Gap,
    /// El contorno de un polígono se cruza consigo mismo.
    SelfIntersection,
    /// Extremo de una línea que no toca ninguna otra.
    Dangle,
}

impl Rule {
    /// Nombre corto en español, el que sale en el informe.
    pub fn label(self) -> &'static str {
        match self {
            Rule::Overlap => "solape",
            Rule::Gap => "hueco",
            Rule::SelfIntersection => "autointersección",
            Rule::Dangle => "cabo suelto",
        }
    }
}

/// Un incumplimiento: la regla, los elementos implicados (índice de fila), un punto donde ocurre y,
/// en solapes y huecos, la superficie afectada.
#[derive(Clone, Debug)]
pub struct Issue {
    pub rule: Rule,
    pub features: Vec<usize>,
    pub at: [f64; 2],
    pub area: f64,
}

/// Comprueba las reglas que valen para el tipo de geometría de la capa. `tol` es la tolerancia: en
/// polígonos, la superficie por debajo de la cual un solape o un hueco se ignora (ruido de
/// redondeo); en líneas, la distancia a la que dos extremos se consideran unidos.
pub fn check(d: &VectorData, kind: GeomKind, tol: f64) -> Vec<Issue> {
    match kind {
        GeomKind::Polygon => {
            let mut v = self_intersections(d);
            v.extend(overlaps(d, tol));
            v.extend(gaps(d, tol));
            v
        }
        GeomKind::Line => dangles(d, tol),
        GeomKind::Point => Vec::new(),
    }
}

fn bbox(f: &crate::view::Feature) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in f.parts.iter().flatten() {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

fn centroid_of(m: &MultiPolygon<f64>) -> [f64; 2] {
    use geo::InteriorPoint;
    m.interior_point().map_or([0.0, 0.0], |p| [p.x(), p.y()])
}

/// Pares de polígonos con superficie común mayor que `tol`.
pub fn overlaps(d: &VectorData, tol: f64) -> Vec<Issue> {
    let boxes: Vec<[f64; 4]> = d.features.iter().map(bbox).collect();
    let polys: Vec<MultiPolygon<f64>> = d.features.iter().map(|f| crate::geomops::multipolygon_of(&f.parts)).collect();
    let mut out = Vec::new();
    for i in 0..polys.len() {
        for j in i + 1..polys.len() {
            let (a, b) = (boxes[i], boxes[j]);
            if a[0] > b[2] || b[0] > a[2] || a[1] > b[3] || b[1] > a[3] {
                continue;
            }
            let common = polys[i].intersection(&polys[j]);
            let area = common.unsigned_area();
            if area > tol {
                out.push(Issue { rule: Rule::Overlap, features: vec![i, j], at: centroid_of(&common), area });
            }
        }
    }
    out
}

/// Huecos interiores de la unión de todos los polígonos con superficie mayor que `tol`.
pub fn gaps(d: &VectorData, tol: f64) -> Vec<Issue> {
    let polys: Vec<geo::Polygon<f64>> = d
        .features
        .iter()
        .flat_map(|f| crate::geomops::multipolygon_of(&f.parts).0)
        .collect();
    if polys.len() < 2 {
        return Vec::new();
    }
    let all = geo::unary_union(&polys);
    let mut out = Vec::new();
    for p in &all.0 {
        for h in p.interiors() {
            let hole = geo::Polygon::new(h.clone(), vec![]);
            let area = hole.unsigned_area();
            if area > tol {
                let c = centroid_of(&MultiPolygon::new(vec![hole]));
                out.push(Issue { rule: Rule::Gap, features: Vec::new(), at: c, area });
            }
        }
    }
    out
}

/// Cruce propio de segmentos no contiguos dentro de un mismo anillo.
pub fn self_intersections(d: &VectorData) -> Vec<Issue> {
    let mut out = Vec::new();
    for (row, f) in d.features.iter().enumerate() {
        'feature: for ring in &f.parts {
            let n = ring.len().saturating_sub(1); // el anillo viene cerrado
            for i in 0..n {
                for j in i + 2..n {
                    if i == 0 && j == n - 1 {
                        continue; // el último segmento enlaza con el primero
                    }
                    if let Some(p) = crossing(ring[i], ring[i + 1], ring[j], ring[j + 1]) {
                        out.push(Issue { rule: Rule::SelfIntersection, features: vec![row], at: p, area: 0.0 });
                        continue 'feature;
                    }
                }
            }
        }
    }
    out
}

/// Punto de cruce propio de dos segmentos (se excluyen los toques en sus extremos).
fn crossing(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let (r, q) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * q[1] - r[1] * q[0];
    if den.abs() < 1e-18 {
        return None;
    }
    let w = [c[0] - a[0], c[1] - a[1]];
    let t = (w[0] * q[1] - w[1] * q[0]) / den;
    let s = (w[0] * r[1] - w[1] * r[0]) / den;
    let eps = 1e-12;
    ((eps..1.0 - eps).contains(&t) && (eps..1.0 - eps).contains(&s)).then(|| [a[0] + r[0] * t, a[1] + r[1] * t])
}

/// Extremos de líneas abiertas que no están a menos de `tol` de otro extremo ni de un segmento de
/// otra línea.
pub fn dangles(d: &VectorData, tol: f64) -> Vec<Issue> {
    let tol = tol.max(1e-9);
    // extremos de cada parte abierta: (fila, parte, punto)
    let mut ends: Vec<(usize, usize, [f64; 2])> = Vec::new();
    for (row, f) in d.features.iter().enumerate() {
        for (k, part) in f.parts.iter().enumerate() {
            if part.len() < 2 || part.first() == part.last() {
                continue;
            }
            ends.push((row, k, part[0]));
            ends.push((row, k, part[part.len() - 1]));
        }
    }
    let near_seg = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
        (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy) <= tol
    };
    let mut out = Vec::new();
    for (idx, &(row, k, p)) in ends.iter().enumerate() {
        // otro extremo (de otra parte, o el opuesto de una parte de dos puntos que se cerrase)
        let joined_end = ends
            .iter()
            .enumerate()
            .any(|(j, &(r2, k2, q))| j != idx && (r2, k2) != (row, k) && (p[0] - q[0]).hypot(p[1] - q[1]) <= tol);
        let on_line = || {
            d.features.iter().enumerate().any(|(r2, f)| {
                f.parts.iter().enumerate().any(|(k2, part)| {
                    (r2, k2) != (row, k) && part.windows(2).any(|w| near_seg(p, w[0], w[1]))
                })
            })
        };
        if !joined_end && !on_line() {
            out.push(Issue { rule: Rule::Dangle, features: vec![row], at: p, area: 0.0 });
        }
    }
    out
}

/// El informe de incumplimientos como CSV: regla, elementos, x, y y superficie.
pub fn report_csv(issues: &[Issue]) -> String {
    let mut s = String::from("regla,elementos,x,y,superficie\n");
    for i in issues {
        let rows: Vec<String> = i.features.iter().map(|r| (r + 1).to_string()).collect();
        s.push_str(&format!("{},{},{:.3},{:.3},{:.3}\n", i.rule.label(), rows.join(" "), i.at[0], i.at[1], i.area));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Feature;

    fn sq(x: f64, y: f64, s: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y], [x, y]]
    }

    fn polys(rings: Vec<Vec<[f64; 2]>>) -> VectorData {
        let mut d = VectorData { polygons: true, ..VectorData::default() };
        for r in rings {
            d.features.push(Feature { parts: vec![r] });
            d.text.push(Vec::new());
            d.num.push(Vec::new());
        }
        d
    }

    #[test]
    fn finds_overlaps_and_ignores_neighbours_that_only_touch() {
        let d = polys(vec![sq(0.0, 0.0, 10.0), sq(5.0, 5.0, 10.0), sq(20.0, 0.0, 10.0), sq(30.0, 0.0, 10.0)]);
        let v = overlaps(&d, 1e-6);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].features, vec![0, 1]);
        assert!((v[0].area - 25.0).abs() < 1e-9);
    }

    #[test]
    fn finds_the_gap_between_four_polygons_around_a_hole() {
        // un cuadro de 3 x 3 sin su centro
        let mut rings = Vec::new();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (0.0, 1.0), (2.0, 1.0), (0.0, 2.0), (1.0, 2.0), (2.0, 2.0)] {
            rings.push(sq(x, y, 1.0));
        }
        let d = polys(rings);
        let g = gaps(&d, 1e-6);
        assert_eq!(g.len(), 1);
        assert!((g[0].area - 1.0).abs() < 1e-9);
        assert!((g[0].at[0] - 1.5).abs() < 1e-9 && (g[0].at[1] - 1.5).abs() < 1e-9);
        // sin hueco, nada
        assert!(gaps(&polys(vec![sq(0.0, 0.0, 1.0), sq(1.0, 0.0, 1.0)]), 1e-6).is_empty());
    }

    #[test]
    fn finds_a_bow_tie_but_not_a_simple_ring() {
        let bow = vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0], [0.0, 0.0]];
        let v = self_intersections(&polys(vec![bow, sq(20.0, 0.0, 5.0)]));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].features, vec![0]);
        assert!((v[0].at[0] - 5.0).abs() < 1e-9 && (v[0].at[1] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn finds_dangling_ends_only() {
        let mut d = VectorData::default();
        // una T: el palo toca el travesaño; los extremos libres son los dos del travesaño y el
        // de arriba no existe porque el palo acaba sobre el travesaño
        for part in [vec![[0.0, 10.0], [10.0, 10.0]], vec![[5.0, 0.0], [5.0, 10.0]], vec![[10.0, 10.0], [10.0, 20.0]]] {
            d.features.push(Feature { parts: vec![part] });
            d.text.push(Vec::new());
            d.num.push(Vec::new());
        }
        let v = dangles(&d, 1e-6);
        let at: Vec<[f64; 2]> = v.iter().map(|i| i.at).collect();
        assert_eq!(v.len(), 3, "{at:?}");
        assert!(at.contains(&[0.0, 10.0]) && at.contains(&[5.0, 0.0]) && at.contains(&[10.0, 20.0]));
    }
}
