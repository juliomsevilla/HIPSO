//! Solape entre pasadas. Donde dos franjas de un vuelo cubren el mismo terreno, sus puntos lo
//! describen dos veces; la franja escaneada más cerca del nadir (el menor ángulo de barrido) lo ve
//! mejor, y los puntos de las demás se apartan como clase 12 (ASPRS: puntos de solape), de modo que
//! los pasos siguientes trabajen sobre una sola cobertura del terreno.
//!
//! El terreno se corta en celdas cuadradas; en una celda alcanzada por más de una pasada, la pasada
//! con el menor ángulo de barrido absoluto medio conserva sus puntos y los puntos de las demás
//! pasadas son solape. Una celda con una pasada, o un archivo sin pasada ni ángulo de barrido, no
//! tiene ninguno.
//!
//! El solape lateral entre franjas pone dos pasadas sobre una celda (tres en las esquinas). Una
//! celda alcanzada por más pasadas que [`MAX_LINES`] es un vuelo que pasa por el mismo terreno una
//! y otra vez (un dron con varias pasadas, vuelos cruzados): allí las pasadas se suman unas a otras
//! y, sobre todo, añaden puntos a cables y estructuras finas, así que no se aparta nada.

use std::collections::HashMap;

use crate::las::Point;

/// Una pasada en una celda: su identificador de fuente del punto, la suma de los |ángulos de
/// barrido| y el recuento.
type Line = (u16, f64, u32);

/// La clase de los puntos de solape.
pub const OVERLAP: u8 = 12;

/// Máximo de pasadas sobre una celda para que las sobrantes sean solape.
pub const MAX_LINES: usize = 2;

/// Qué puntos son solape, con celdas de `cell` metros. El ruido y los puntos retenidos no
/// participan.
pub fn find(pts: &[Point], cell: f64) -> Vec<bool> {
    let mut out = vec![false; pts.len()];
    if !(cell > 0.0) {
        return out;
    }
    let key = |q: &Point| ((q.x / cell).floor() as i64, (q.y / cell).floor() as i64);
    // por celda, por pasada: suma de |ángulo de barrido| y recuento
    let mut cells: HashMap<(i64, i64), Vec<Line>> = HashMap::new();
    for q in pts.iter().filter(|q| !q.is_noise()) {
        let lines = cells.entry(key(q)).or_default();
        let a = f64::from(q.scan_angle.abs());
        match lines.iter_mut().find(|l| l.0 == q.source_id) {
            Some(l) => {
                l.1 += a;
                l.2 += 1;
            }
            None => lines.push((q.source_id, a, 1)),
        }
    }
    // la pasada que se conserva en cada celda alcanzada por varias
    let best: HashMap<(i64, i64), u16> = cells
        .iter()
        .filter(|(_, lines)| lines.len() > 1 && lines.len() <= MAX_LINES)
        .filter_map(|(k, lines)| {
            lines
                .iter()
                .min_by(|a, b| {
                    (a.1 / f64::from(a.2))
                        .total_cmp(&(b.1 / f64::from(b.2)))
                        .then(a.0.cmp(&b.0))
                })
                .map(|l| (*k, l.0))
        })
        .collect();
    for (k, q) in pts.iter().enumerate() {
        if !q.is_noise()
            && let Some(keep) = best.get(&key(q))
        {
            out[k] = q.source_id != *keep;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, source: u16, angle: f32) -> Point {
        Point {
            x,
            y,
            source_id: source,
            scan_angle: angle,
            ..Default::default()
        }
    }

    #[test]
    fn the_line_nearest_the_nadir_keeps_the_cell() {
        // dos pasadas sobre la misma celda de 5 m, una a 4° y otra a 18°; una pasada sola al lado
        let pts = [
            pt(1.0, 1.0, 10, 4.0),
            pt(2.0, 2.0, 10, -6.0),
            pt(1.5, 1.5, 11, 18.0),
            pt(3.0, 3.0, 11, 20.0),
            pt(7.0, 1.0, 11, 19.0),
        ];
        assert_eq!(find(&pts, 5.0), [false, false, true, true, false]);
    }

    #[test]
    fn the_angle_is_compared_in_absolute_value_and_ties_go_to_the_lower_line() {
        let pts = [pt(1.0, 1.0, 20, -8.0), pt(2.0, 2.0, 21, 8.0)];
        assert_eq!(find(&pts, 5.0), [false, true]);
    }

    #[test]
    fn a_cell_flown_over_many_times_keeps_every_pass() {
        let pts = [
            pt(1.0, 1.0, 10, 4.0),
            pt(2.0, 2.0, 11, 5.0),
            pt(1.5, 1.5, 12, 6.0),
            pt(1.2, 1.2, 13, 7.0),
        ];
        assert_eq!(find(&pts, 5.0), [false, false, false, false]);
    }

    #[test]
    fn noise_and_a_single_line_have_no_overlap() {
        let mut noise = pt(1.0, 1.0, 12, 30.0);
        noise.class = 7;
        let pts = [pt(1.0, 1.0, 10, 4.0), noise, pt(9.0, 9.0, 10, 4.0)];
        assert_eq!(find(&pts, 5.0), [false, false, false]);
        assert!(find(&pts, 0.0).iter().all(|o| !o));
    }
}
