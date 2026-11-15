//! Operaciones de edición de elementos vectoriales que no dependen de la interfaz: mover, girar y
//! escalar, reformar el contorno de un polígono con un trazo y pegar elementos de una capa en otra.
//! Las llama el editor vectorial del visor y se prueban aquí con geometrías pequeñas.

use crate::view::{GeomKind, VectorData};

/// Mueve, gira y escala las partes de un elemento: primero la escala y el giro alrededor de
/// `pivot` (grados, antihorario) y después el desplazamiento `(dx, dy)`.
pub fn transform(parts: &mut [Vec<[f64; 2]>], pivot: [f64; 2], dx: f64, dy: f64, angle_deg: f64, scale: f64) {
    let (s, c) = angle_deg.to_radians().sin_cos();
    for p in parts.iter_mut().flatten() {
        let (x, y) = ((p[0] - pivot[0]) * scale, (p[1] - pivot[1]) * scale);
        *p = [pivot[0] + x * c - y * s + dx, pivot[1] + x * s + y * c + dy];
    }
}

/// El centro de la caja de unas partes (el pivote por defecto de un giro).
pub fn center_of(parts: &[Vec<[f64; 2]>]) -> Option<[f64; 2]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in parts.iter().flatten() {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b.iter().all(|v| v.is_finite()).then(|| [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0])
}

/// Cruce de dos segmentos: `(t, s, punto)` con `t` a lo largo de `a→b` y `s` a lo largo de `c→d`.
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<(f64, f64, [f64; 2])> {
    let (r, q) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * q[1] - r[1] * q[0];
    if den.abs() < 1e-15 {
        return None;
    }
    let w = [c[0] - a[0], c[1] - a[1]];
    let t = (w[0] * q[1] - w[1] * q[0]) / den;
    let s = (w[0] * r[1] - w[1] * r[0]) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&s)).then(|| (t, s, [a[0] + r[0] * t, a[1] + r[1] * t]))
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Reforma el anillo exterior de un polígono con un trazo que lo cruza: entre el primer y el último
/// cruce, la parte más corta del contorno se sustituye por el trazo (se corta una esquina o se
/// añade una mordida según por dónde pase). Devuelve el anillo nuevo, cerrado, o `None` cuando el
/// trazo no cruza el contorno al menos dos veces.
pub fn reshape_ring(ring: &[[f64; 2]], line: &[[f64; 2]]) -> Option<Vec<[f64; 2]>> {
    let mut v: Vec<[f64; 2]> = ring.to_vec();
    if v.len() > 1 && v.first() == v.last() {
        v.pop();
    }
    let n = v.len();
    if n < 3 || line.len() < 2 {
        return None;
    }
    // cruces: (posición a lo largo del trazo, arista, parámetro en la arista, punto)
    let mut hits: Vec<(f64, usize, f64, [f64; 2])> = Vec::new();
    for (k, w) in line.windows(2).enumerate() {
        for e in 0..n {
            if let Some((t, s, p)) = cross(w[0], w[1], v[e], v[(e + 1) % n]) {
                hits.push((k as f64 + t, e, s, p));
            }
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    hits.dedup_by(|a, b| dist(a.3, b.3) < 1e-12);
    if hits.len() < 2 {
        return None;
    }
    let (a, b) = (hits[0], hits[hits.len() - 1]);
    // el trazo entre los dos cruces
    let mut sub: Vec<[f64; 2]> = vec![a.3];
    for (k, p) in line.iter().enumerate().skip(1) {
        if (k as f64) > a.0 && (k as f64) < b.0 {
            sub.push(*p);
        }
    }
    sub.push(b.3);
    // contorno de un cruce a otro, hacia delante (los vértices intermedios, en orden)
    let along = |from: &(f64, usize, f64, [f64; 2]), to: &(f64, usize, f64, [f64; 2])| -> Vec<[f64; 2]> {
        let mut path = vec![from.3];
        let (pf, pt) = (from.1 as f64 + from.2, to.1 as f64 + to.2);
        let mut i = from.1;
        // si el destino queda antes en la vuelta, se da la vuelta entera
        let mut steps = if pt >= pf { to.1 - from.1 } else { n - from.1 + to.1 };
        while steps > 0 {
            i = (i + 1) % n;
            path.push(v[i]);
            steps -= 1;
        }
        path.push(to.3);
        path
    };
    let len = |p: &[[f64; 2]]| p.windows(2).map(|w| dist(w[0], w[1])).sum::<f64>();
    let fwd = along(&a, &b);
    let back = along(&b, &a);
    let mut out = if len(&fwd) >= len(&back) {
        // se conserva a→b y se vuelve de b a a por el trazo, al revés
        let mut o = fwd;
        o.extend(sub.iter().rev().skip(1));
        o
    } else {
        let mut o = back;
        o.extend(sub.iter().skip(1));
        o
    };
    out.dedup_by(|p, q| dist(*p, *q) < 1e-12);
    if out.first() != out.last() {
        out.push(out[0]);
    }
    (out.len() >= 4).then_some(out)
}

/// Pega en `dest` los elementos `rows` de `src`: los campos se emparejan por nombre (sin
/// distinguir mayúsculas) y los que `dest` no tiene se descartan; los de `dest` que `src` no tiene
/// quedan vacíos. Devuelve los índices de los elementos añadidos, o un error si los tipos de
/// geometría no coinciden.
pub fn paste(dest: &mut VectorData, dest_kind: GeomKind, src: &VectorData, src_kind: GeomKind, rows: &[usize]) -> Result<Vec<usize>, String> {
    if dest_kind != src_kind {
        return Err("solo se pega en una capa del mismo tipo de geometría (puntos, líneas o polígonos)".into());
    }
    let map: Vec<Option<usize>> = dest
        .fields
        .iter()
        .map(|f| src.fields.iter().position(|g| g.eq_ignore_ascii_case(f)))
        .collect();
    let mut added = Vec::new();
    for &r in rows {
        let Some(f) = src.features.get(r) else { continue };
        dest.features.push(f.clone());
        dest.text.push(vec![String::new(); dest.fields.len()]);
        dest.num.push(vec![f64::NAN; dest.fields.len()]);
        let row = dest.features.len() - 1;
        for (c, m) in map.iter().enumerate() {
            if let Some(m) = m
                && let Some(v) = src.text.get(r).and_then(|t| t.get(*m))
                && !v.is_empty()
            {
                // un valor que no encaja en el tipo del campo de destino se deja vacío
                let _ = dest.set_value(row, c, v);
            }
        }
        added.push(row);
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fieldtype::FieldType;
    use crate::view::Feature;

    fn area(r: &[[f64; 2]]) -> f64 {
        r.windows(2).map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1]).sum::<f64>().abs() / 2.0
    }

    #[test]
    fn transform_scales_and_turns_about_the_pivot_then_moves() {
        let mut p = vec![vec![[1.0, 0.0], [2.0, 0.0]]];
        transform(&mut p, [0.0, 0.0], 10.0, 0.0, 90.0, 2.0);
        assert!((p[0][0][0] - 10.0).abs() < 1e-9 && (p[0][0][1] - 2.0).abs() < 1e-9);
        assert!((p[0][1][0] - 10.0).abs() < 1e-9 && (p[0][1][1] - 4.0).abs() < 1e-9);
        assert_eq!(center_of(&p), Some([10.0, 3.0]));
    }

    #[test]
    fn reshaping_replaces_the_shorter_part_of_the_outline_with_the_stroke() {
        let sq = vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0], [0.0, 0.0]];
        // cortar la esquina de abajo a la derecha: del lado de abajo al lado de la derecha
        let cut = reshape_ring(&sq, &[[5.0, -2.0], [5.0, 5.0], [12.0, 5.0]]).expect("reforma");
        assert!((area(&cut) - 75.0).abs() < 1e-9, "{}", area(&cut));
        // añadir una mordida hacia fuera: del lado de arriba, saliendo y volviendo a entrar
        let bite = reshape_ring(&sq, &[[3.0, 8.0], [3.0, 14.0], [7.0, 14.0], [7.0, 8.0]]).expect("reforma");
        assert!((area(&bite) - 116.0).abs() < 1e-9, "{}", area(&bite));
        // un trazo que no cruza el contorno no reforma nada
        assert!(reshape_ring(&sq, &[[2.0, 2.0], [8.0, 8.0]]).is_none());
        // el anillo se mantiene cerrado
        assert_eq!(cut.first(), cut.last());
    }

    #[test]
    fn pasting_matches_fields_by_name_and_refuses_other_kinds() {
        let mut src = VectorData { points: true, ..VectorData::default() };
        src.add_field("Sp", FieldType::Text);
        src.add_field("solo_origen", FieldType::Text);
        src.features.push(Feature { parts: vec![vec![[1.0, 1.0]]] });
        src.text.push(vec!["pino".into(), "x".into()]);
        src.num.push(vec![f64::NAN; 2]);
        let mut dest = VectorData { points: true, ..VectorData::default() };
        dest.add_field("sp", FieldType::Text);
        dest.add_field("vol", FieldType::Real);
        let added = paste(&mut dest, GeomKind::Point, &src, GeomKind::Point, &[0]).expect("pega");
        assert_eq!(added, vec![0]);
        assert_eq!(dest.text[0], vec!["pino".to_string(), String::new()]);
        assert!(paste(&mut dest, GeomKind::Polygon, &src, GeomKind::Point, &[0]).is_err());
    }
}
