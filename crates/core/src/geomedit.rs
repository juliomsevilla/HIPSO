//! Ediciones de la geometría de una capa vectorial: simplificar los contornos y las líneas, y
//! trazar la paralela de una línea a una distancia.
//!
//! * **Simplificar** quita vértices con el algoritmo de Douglas y Peucker (1973): un vértice se
//!   queda si se aleja de la recta que une los vecinos más de la tolerancia. En una capa de
//!   polígonos que comparten bordes, el borde común se simplifica una sola vez y los dos vecinos
//!   usan el mismo resultado, de modo que no se abren huecos ni solapes entre ellos; los puntos
//!   donde cambia quién comparte el borde (esquinas entre tres vecinos, extremos de un borde
//!   común) no se mueven. En líneas, los extremos y los vértices donde se encuentran dos líneas
//!   tampoco.
//! * **Paralela** desplaza cada línea a un lado: una distancia positiva a su izquierda (según
//!   se recorre) y negativa a su derecha. Las esquinas exteriores se resuelven con una inglete
//!   limitada (pasado el límite, con un bisel) y los lazos que aparecen en las interiores se
//!   recortan.

use std::collections::HashMap;

use crate::topofix::dist;
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result};

// ---------------------------------------------------------------------------------------------
// simplificar

/// Lo que se hizo al simplificar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimplifyReport {
    pub before: usize,
    pub after: usize,
    /// Elementos que se quedaron sin geometría (un polígono diminuto, por ejemplo) y se quitaron.
    pub dropped: usize,
}

/// Quita los vértices de las líneas o de los contornos de `d` a menos de `tol` de la recta que
/// une sus vecinos. Con `shared`, el borde que comparten dos elementos se simplifica una vez para
/// los dos.
pub fn simplify(d: &mut VectorData, kind: GeomKind, tol: f64, shared: bool) -> Result<SimplifyReport> {
    if !(tol >= 0.0 && tol.is_finite()) {
        return Err(Error::Param("la tolerancia debe ser un número no negativo".into()));
    }
    let closed = match kind {
        GeomKind::Polygon => true,
        GeomKind::Line => false,
        GeomKind::Point => return Err(Error::Param("solo se simplifican líneas y polígonos".into())),
    };
    let mut rep = SimplifyReport::default();
    // los vértices de cada camino sin el de cierre; a qué elementos pertenece cada coordenada
    let mut paths: Vec<(usize, Vec<[f64; 2]>)> = Vec::new();
    for (row, f) in d.features.iter().enumerate() {
        for part in &f.parts {
            let mut pts = part.clone();
            pts.dedup();
            if closed && pts.len() > 1 && pts.first() == pts.last() {
                pts.pop();
            }
            rep.before += pts.len();
            paths.push((row, pts));
        }
    }
    let owners: HashMap<[u64; 2], Vec<usize>> = if shared {
        let mut m: HashMap<[u64; 2], Vec<usize>> = HashMap::new();
        for (id, (row, pts)) in paths.iter().enumerate() {
            // en polígonos manda el elemento; en líneas, cada parte cuenta como un dueño
            let owner = if closed { *row } else { id };
            for p in pts {
                let v = m.entry(key(*p)).or_default();
                if !v.contains(&owner) {
                    v.push(owner);
                }
            }
        }
        m
    } else {
        HashMap::new()
    };
    let mut cache: HashMap<ArcKey, Vec<[f64; 2]>> = HashMap::new();
    let mut out: Vec<Vec<Vec<[f64; 2]>>> = vec![Vec::new(); d.features.len()];
    for (id, (row, pts)) in paths.iter().enumerate() {
        let owner = if closed { *row } else { id };
        let simplified = if closed {
            simplify_ring(pts, tol, owner, &owners, shared, &mut cache)
        } else {
            simplify_line(pts, tol, owner, &owners, shared, &mut cache)
        };
        if let Some(mut s) = simplified {
            rep.after += s.len();
            if closed {
                s.push(s[0]);
            }
            out[*row].push(s);
        }
    }
    let mut features = Vec::new();
    let mut text = Vec::new();
    let mut num = Vec::new();
    for (row, parts) in out.into_iter().enumerate() {
        if parts.is_empty() {
            rep.dropped += 1;
            continue;
        }
        features.push(Feature { parts });
        text.push(d.text.get(row).cloned().unwrap_or_default());
        num.push(d.num.get(row).cloned().unwrap_or_default());
    }
    d.features = features;
    d.text = text;
    d.num = num;
    refit(d);
    Ok(rep)
}

fn refit(d: &mut VectorData) {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in d.features.iter().flat_map(|f| f.parts.iter().flatten()) {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    d.bounds = b;
}

fn key(p: [f64; 2]) -> [u64; 2] {
    [p[0].to_bits(), p[1].to_bits()]
}

/// Un borde común: su primer y último punto y su vértice del medio, en el orden canónico (el que
/// empieza por la coordenada menor), de modo que lo reconozcan los dos elementos que lo recorren
/// cada uno en un sentido.
type ArcKey = ([u64; 2], [u64; 2], usize, [u64; 2]);

/// La distancia de `p` a la recta que pasa por `a` y `b` (al punto `a` si coinciden).
fn off_line(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = dx.hypot(dy);
    if l == 0.0 {
        return dist(p, a);
    }
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / l
}

/// Douglas y Peucker sobre un tramo cuyos extremos se conservan.
fn douglas_peucker(pts: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let n = pts.len();
    if n <= 2 {
        return pts.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (mut far, mut at) = (0.0, None);
        for k in a + 1..b {
            let dk = off_line(pts[k], pts[a], pts[b]);
            if dk > far {
                far = dk;
                at = Some(k);
            }
        }
        if let Some(k) = at
            && far > tol
        {
            keep[k] = true;
            stack.push((a, k));
            stack.push((k, b));
        }
    }
    pts.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect()
}

/// Los elementos que comparten cada coordenada, sin el `owner` propio: si queda alguno, el vértice
/// es compartido.
fn shared_with(owners: &HashMap<[u64; 2], Vec<usize>>, p: [f64; 2]) -> &[usize] {
    owners.get(&key(p)).map_or(&[], Vec::as_slice)
}

/// El tramo `arc` (con sus dos extremos fijos) simplificado, reutilizando el resultado cuando otro
/// elemento recorre el mismo borde.
fn simplify_arc(arc: &[[f64; 2]], tol: f64, shared: bool, cache: &mut HashMap<ArcKey, Vec<[f64; 2]>>) -> Vec<[f64; 2]> {
    if arc.len() <= 2 || !shared {
        return douglas_peucker(arc, tol);
    }
    let (first, last) = (arc[0], arc[arc.len() - 1]);
    let reversed = key(first) > key(last);
    let canon: Vec<[f64; 2]> = if reversed { arc.iter().rev().copied().collect() } else { arc.to_vec() };
    let k: ArcKey = (key(canon[0]), key(canon[canon.len() - 1]), canon.len(), key(canon[canon.len() / 2]));
    let mut s = cache.entry(k).or_insert_with(|| douglas_peucker(&canon, tol)).clone();
    if reversed {
        s.reverse();
    }
    s
}

/// Qué vértices de un contorno no se tocan: aquellos donde cambia el conjunto de elementos que
/// comparten el tramo que llega y el que sale.
fn ring_pins(pts: &[[f64; 2]], owner: usize, owners: &HashMap<[u64; 2], Vec<usize>>) -> Vec<bool> {
    let n = pts.len();
    let seg = |a: [f64; 2], b: [f64; 2]| -> Vec<usize> {
        let (sa, sb) = (shared_with(owners, a), shared_with(owners, b));
        let mut v: Vec<usize> = sa.iter().copied().filter(|x| sb.contains(x)).collect();
        v.sort_unstable();
        v
    };
    (0..n)
        .map(|i| {
            let (p, c, nx) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let _ = owner;
            seg(p, c) != seg(c, nx)
        })
        .collect()
}

fn simplify_ring(
    pts: &[[f64; 2]],
    tol: f64,
    owner: usize,
    owners: &HashMap<[u64; 2], Vec<usize>>,
    shared: bool,
    cache: &mut HashMap<ArcKey, Vec<[f64; 2]>>,
) -> Option<Vec<[f64; 2]>> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let mut pin = if shared { ring_pins(pts, owner, owners) } else { vec![false; n] };
    // sin ningún vértice fijo, el contorno se parte por el vértice 0 y el más lejano a él
    if !pin.iter().any(|p| *p) {
        pin[0] = true;
        let far = (1..n).max_by(|&a, &b| dist(pts[0], pts[a]).total_cmp(&dist(pts[0], pts[b])))?;
        pin[far] = true;
    } else if pin.iter().filter(|p| **p).count() == 1 {
        // un único vértice fijo: el tramo sería un lazo; se añade el más lejano a él
        let at = pin.iter().position(|p| *p)?;
        let far = (0..n).filter(|&k| k != at).max_by(|&a, &b| dist(pts[at], pts[a]).total_cmp(&dist(pts[at], pts[b])))?;
        pin[far] = true;
    }
    let pins: Vec<usize> = (0..n).filter(|&k| pin[k]).collect();
    let mut out: Vec<[f64; 2]> = Vec::new();
    for (w, &a) in pins.iter().enumerate() {
        let b = pins[(w + 1) % pins.len()];
        let mut arc = Vec::new();
        let mut k = a;
        loop {
            arc.push(pts[k]);
            if k == b && arc.len() > 1 {
                break;
            }
            k = (k + 1) % n;
            if arc.len() > n + 1 {
                break;
            }
        }
        let s = simplify_arc(&arc, tol, shared, cache);
        out.extend_from_slice(&s[..s.len() - 1]);
    }
    out.dedup();
    (out.len() >= 3).then_some(out)
}

fn simplify_line(
    pts: &[[f64; 2]],
    tol: f64,
    owner: usize,
    owners: &HashMap<[u64; 2], Vec<usize>>,
    shared: bool,
    cache: &mut HashMap<ArcKey, Vec<[f64; 2]>>,
) -> Option<Vec<[f64; 2]>> {
    let n = pts.len();
    if n < 2 {
        return None;
    }
    // se fijan los extremos y los vértices que comparte con otra línea
    let pins: Vec<usize> = (0..n)
        .filter(|&k| k == 0 || k == n - 1 || (shared && shared_with(owners, pts[k]).iter().any(|o| *o != owner)))
        .collect();
    let mut out: Vec<[f64; 2]> = vec![pts[0]];
    for w in pins.windows(2) {
        let s = simplify_arc(&pts[w[0]..=w[1]], tol, shared, cache);
        out.extend_from_slice(&s[1..]);
    }
    out.dedup();
    (out.len() >= 2).then_some(out)
}

// ---------------------------------------------------------------------------------------------
// paralela

/// Lo que se hizo al trazar paralelas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OffsetReport {
    pub parts: usize,
    /// Lazos de las esquinas interiores que se recortaron.
    pub loops_clipped: usize,
    /// Partes que desaparecen (la distancia las deja sin sentido, o eran de un solo punto).
    pub dropped: usize,
}

/// Cuántos tramos hacia atrás se miran al buscar el cruce de un lazo.
const LOOK_BACK: usize = 64;

/// Sustituye cada línea de `d` por su paralela a `distance` (positiva a la izquierda, negativa a la
/// derecha). `miter` es el límite de la inglete en esquinas exteriores, en múltiplos de la
/// distancia (2 por defecto en la herramienta); más allá se corta con un bisel.
pub fn parallel(d: &mut VectorData, distance: f64, miter: f64) -> Result<OffsetReport> {
    if !distance.is_finite() || distance == 0.0 {
        return Err(Error::Param("la distancia de la paralela debe ser un número distinto de cero".into()));
    }
    if !(miter >= 1.0) {
        return Err(Error::Param("el límite de la inglete debe ser al menos 1".into()));
    }
    let mut rep = OffsetReport::default();
    let mut features = Vec::new();
    let mut text = Vec::new();
    let mut num = Vec::new();
    for (row, f) in d.features.iter().enumerate() {
        let mut parts = Vec::new();
        for part in &f.parts {
            match offset_part(part, distance, miter, &mut rep.loops_clipped) {
                Some(p) => {
                    rep.parts += 1;
                    parts.push(p);
                }
                None => rep.dropped += 1,
            }
        }
        if parts.is_empty() {
            continue;
        }
        features.push(Feature { parts });
        text.push(d.text.get(row).cloned().unwrap_or_default());
        num.push(d.num.get(row).cloned().unwrap_or_default());
    }
    d.features = features;
    d.text = text;
    d.num = num;
    refit(d);
    Ok(rep)
}

/// La intersección propia de dos segmentos (sin contar los toques por los extremos), si cruzan.
fn cross_point(a: [f64; 2], b: [f64; 2], c: [f64; 2], e: [f64; 2]) -> Option<[f64; 2]> {
    let (r, q) = ([b[0] - a[0], b[1] - a[1]], [e[0] - c[0], e[1] - c[1]]);
    let den = r[0] * q[1] - r[1] * q[0];
    if den.abs() < 1e-18 {
        return None;
    }
    let w = [c[0] - a[0], c[1] - a[1]];
    let t = (w[0] * q[1] - w[1] * q[0]) / den;
    let s = (w[0] * r[1] - w[1] * r[0]) / den;
    ((1e-12..1.0 - 1e-12).contains(&t) && (1e-12..1.0 - 1e-12).contains(&s)).then(|| [a[0] + r[0] * t, a[1] + r[1] * t])
}

fn offset_part(part: &[[f64; 2]], dist_: f64, miter: f64, clipped: &mut usize) -> Option<Vec<[f64; 2]>> {
    let mut pts = part.to_vec();
    pts.dedup();
    let closed = pts.len() > 3 && pts.first() == pts.last();
    if closed {
        pts.pop();
    }
    let n = pts.len();
    if n < 2 {
        return None;
    }
    let segs = if closed { n } else { n - 1 };
    // dirección y normal a la izquierda de cada tramo
    let unit: Vec<[f64; 2]> = (0..segs)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let l = dist(a, b);
            [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
        })
        .collect();
    let shift = |p: [f64; 2], u: [f64; 2]| [p[0] - u[1] * dist_, p[1] + u[0] * dist_];
    let mut out: Vec<[f64; 2]> = Vec::new();
    // el vértice `k` une el tramo `prev` con el tramo `next`
    let join = |k: usize, prev: usize, next: usize, out: &mut Vec<[f64; 2]>, clipped: &mut usize| {
        let (u, v) = (unit[prev], unit[next]);
        let p = pts[k];
        let (a, b) = (shift(p, u), shift(p, v));
        let cross = u[0] * v[1] - u[1] * v[0];
        let dot = u[0] * v[0] + u[1] * v[1];
        if cross.abs() < 1e-12 && dot > 0.0 {
            out.push(a); // siguen en línea recta
            return;
        }
        // es esquina exterior cuando se gira hacia el lado contrario al del desplazamiento
        let outer = cross * dist_ < 0.0;
        if outer {
            // inglete: el punto a `1 / cos(medio ángulo)` de la esquina por la bisectriz
            let half = ((1.0 + dot) / 2.0).max(0.0).sqrt();
            if half > 1e-9 && 1.0 / half <= miter {
                let m = [(a[0] + b[0]) / 2.0 - p[0], (a[1] + b[1]) / 2.0 - p[1]];
                let ml = m[0].hypot(m[1]);
                if ml > 0.0 {
                    let k2 = dist_.abs() / half / ml;
                    out.push([p[0] + m[0] * k2, p[1] + m[1] * k2]);
                    return;
                }
            }
            out.push(a);
            out.push(b);
        } else {
            // esquina interior: se dejan los dos puntos y el lazo se recorta al añadirlos
            for q in [a, b] {
                push_clipping(out, q, clipped);
            }
        }
    };
    if closed {
        for k in 0..n {
            join(k, (k + n - 1) % n, k, &mut out, clipped);
        }
    } else {
        out.push(shift(pts[0], unit[0]));
        for k in 1..n - 1 {
            join(k, k - 1, k, &mut out, clipped);
        }
        out.push(shift(pts[n - 1], unit[segs - 1]));
    }
    out.dedup();
    if closed && out.len() > 1 {
        out.push(out[0]);
    }
    (out.len() >= 2).then_some(out)
}

/// Añade `q` a la línea; si el nuevo tramo cruza uno de los últimos, se recorta el lazo que
/// forman, dejando el punto de cruce.
fn push_clipping(out: &mut Vec<[f64; 2]>, q: [f64; 2], clipped: &mut usize) {
    if let Some(&last) = out.last() {
        let len = out.len();
        let from = len.saturating_sub(LOOK_BACK + 1);
        for i in from..len.saturating_sub(2) {
            if let Some(x) = cross_point(out[i], out[i + 1], last, q) {
                out.truncate(i + 1);
                out.push(x);
                out.push(q);
                *clipped += 1;
                return;
            }
        }
    }
    out.push(q);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geomops::multipolygon_of;
    use geo::Area;

    fn layer(kind: GeomKind, feats: Vec<Vec<Vec<[f64; 2]>>>) -> VectorData {
        let mut d = VectorData { polygons: kind == GeomKind::Polygon, ..VectorData::default() };
        d.fields = vec!["id".into()];
        for (k, parts) in feats.into_iter().enumerate() {
            d.features.push(Feature { parts });
            d.text.push(vec![k.to_string()]);
            d.num.push(vec![k as f64]);
        }
        d
    }

    fn sq(x: f64, y: f64, w: f64, h: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x, y + h], [x + w, y + h], [x + w, y], [x, y]]
    }

    #[test]
    fn a_nearly_straight_line_loses_its_middle_vertices_but_not_its_ends() {
        let mut d = layer(GeomKind::Line, vec![vec![vec![[0.0, 0.0], [1.0, 0.01], [2.0, -0.01], [3.0, 0.0], [4.0, 3.0]]]]);
        let r = simplify(&mut d, GeomKind::Line, 0.1, true).unwrap();
        assert_eq!(r.before, 5);
        assert_eq!(d.features[0].parts[0], vec![[0.0, 0.0], [3.0, 0.0], [4.0, 3.0]]);
        // con tolerancia cero no se quita nada que no esté alineado
        let mut e = layer(GeomKind::Line, vec![vec![vec![[0.0, 0.0], [1.0, 0.01], [2.0, 0.0]]]]);
        simplify(&mut e, GeomKind::Line, 0.0, true).unwrap();
        assert_eq!(e.features[0].parts[0].len(), 3);
    }

    #[test]
    fn neighbours_keep_exactly_the_same_border_after_simplifying() {
        // dos polígonos con un borde común ondulado (x = 10 con pequeñas desviaciones)
        let border = [[10.0, 0.0], [10.02, 2.0], [9.98, 4.0], [10.02, 6.0], [10.0, 8.0], [10.01, 10.0]];
        let mut left = vec![[0.0, 0.0], [0.0, 10.0]];
        left.extend(border.iter().rev().copied());
        left.push([0.0, 0.0]);
        let mut right = vec![[20.0, 0.0], [10.0, 0.0]];
        right.extend(border.iter().copied().skip(1));
        right.push([20.0, 10.0]);
        right.push([20.0, 0.0]);
        let mut d = layer(GeomKind::Polygon, vec![vec![left], vec![right]]);
        let before: f64 = (0..2).map(|k| multipolygon_of(&d.features[k].parts).unsigned_area()).sum();
        let r = simplify(&mut d, GeomKind::Polygon, 0.1, true).unwrap();
        assert!(r.after < r.before, "{r:?}");
        let after: f64 = (0..2).map(|k| multipolygon_of(&d.features[k].parts).unsigned_area()).sum();
        assert!((before - after).abs() < 0.5);
        // sin huecos ni solapes entre ellos
        assert!(crate::topology::gaps(&d, 1e-9).is_empty());
        assert!(crate::topology::overlaps(&d, 1e-9).is_empty());
        // y los vértices del borde son los mismos en los dos
        let on_border = |k: usize| -> Vec<[f64; 2]> {
            let mut v: Vec<[f64; 2]> = d.features[k].parts[0].iter().copied().filter(|p| p[0] > 9.5 && p[0] < 10.5).collect();
            v.sort_by(|a, b| a[1].total_cmp(&b[1]));
            v.dedup();
            v
        };
        assert_eq!(on_border(0), on_border(1));
    }

    #[test]
    fn a_tiny_polygon_disappears_and_its_row_with_it() {
        let mut d = layer(GeomKind::Polygon, vec![vec![sq(0.0, 0.0, 10.0, 10.0)], vec![sq(20.0, 0.0, 0.05, 0.05)]]);
        let r = simplify(&mut d, GeomKind::Polygon, 1.0, false).unwrap();
        assert_eq!(r.dropped, 1);
        assert_eq!(d.features.len(), 1);
        assert_eq!(d.text.len(), 1);
    }

    #[test]
    fn a_line_parallel_to_a_straight_one_is_shifted_to_the_left() {
        let mut d = layer(GeomKind::Line, vec![vec![vec![[0.0, 0.0], [10.0, 0.0]]]]);
        let r = parallel(&mut d, 2.0, 2.0).unwrap();
        assert_eq!(r.parts, 1);
        assert_eq!(d.features[0].parts[0], vec![[0.0, 2.0], [10.0, 2.0]]);
        let mut e = layer(GeomKind::Line, vec![vec![vec![[0.0, 0.0], [10.0, 0.0]]]]);
        parallel(&mut e, -2.0, 2.0).unwrap();
        assert_eq!(e.features[0].parts[0], vec![[0.0, -2.0], [10.0, -2.0]]);
    }

    #[test]
    fn an_outer_corner_gets_a_miter_and_an_inner_one_loses_its_loop() {
        // una L: va al este y luego al norte (gira a la izquierda)
        let l = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        // a la derecha es el lado exterior: inglete en (12, -2)
        let mut right = layer(GeomKind::Line, vec![vec![l.clone()]]);
        parallel(&mut right, -2.0, 2.0).unwrap();
        let want = [[0.0, -2.0], [12.0, -2.0], [12.0, 10.0]];
        let got = &right.features[0].parts[0];
        assert_eq!(got.len(), 3);
        for (g, w) in got.iter().zip(want) {
            assert!(dist(*g, w) < 1e-9, "{g:?} frente a {w:?}");
        }
        // a la izquierda es el lado interior: las dos paralelas se cruzan en (8, 2)
        let mut left = layer(GeomKind::Line, vec![vec![l]]);
        let r = parallel(&mut left, 2.0, 2.0).unwrap();
        assert_eq!(r.loops_clipped, 0);
        let p = &left.features[0].parts[0];
        assert!(dist(p[0], [0.0, 2.0]) < 1e-9);
        assert!(dist(p[p.len() - 1], [8.0, 10.0]) < 1e-9);
    }

    #[test]
    fn an_inner_loop_of_a_sharp_turn_is_clipped() {
        // un zigzag cerrado: la paralela interior forma lazos que hay que recortar
        let z = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 1.0], [0.0, 1.0], [0.0, 2.0], [10.0, 2.0]];
        let mut d = layer(GeomKind::Line, vec![vec![z]]);
        let r = parallel(&mut d, 3.0, 2.0).unwrap();
        assert!(r.loops_clipped >= 1, "{r:?}");
        // ninguna pareja de tramos no contiguos se cruza ya
        let p = &d.features[0].parts[0];
        for i in 0..p.len() - 1 {
            for j in i + 2..p.len() - 1 {
                assert!(cross_point(p[i], p[i + 1], p[j], p[j + 1]).is_none(), "tramos {i} y {j}");
            }
        }
    }

    #[test]
    fn a_closed_ring_offset_outwards_grows_by_the_distance() {
        // un cuadrado antihorario a la izquierda es hacia dentro; a la derecha, hacia fuera
        let ring = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]];
        let mut d = layer(GeomKind::Line, vec![vec![ring]]);
        parallel(&mut d, -1.0, 2.0).unwrap();
        let p = &d.features[0].parts[0];
        assert_eq!(p.first(), p.last());
        let xs: Vec<f64> = p.iter().map(|q| q[0]).collect();
        let ys: Vec<f64> = p.iter().map(|q| q[1]).collect();
        assert!((xs.iter().cloned().fold(f64::INFINITY, f64::min) + 1.0).abs() < 1e-9);
        assert!((xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - 11.0).abs() < 1e-9);
        assert!((ys.iter().cloned().fold(f64::INFINITY, f64::min) + 1.0).abs() < 1e-9);
        assert!((ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - 11.0).abs() < 1e-9);
    }

    #[test]
    fn bad_arguments_are_refused() {
        let mut d = layer(GeomKind::Line, vec![vec![vec![[0.0, 0.0], [1.0, 0.0]]]]);
        assert!(parallel(&mut d, 0.0, 2.0).is_err());
        assert!(parallel(&mut d, 1.0, 0.5).is_err());
        assert!(simplify(&mut d, GeomKind::Line, -1.0, true).is_err());
        assert!(simplify(&mut d, GeomKind::Point, 1.0, true).is_err());
    }
}
