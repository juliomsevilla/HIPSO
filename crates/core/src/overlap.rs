//! Overlap between flight lines. Where two strips of a survey cover the same ground, their points
//! describe it twice; the strip scanned closest to the nadir (the smallest scan angle) sees it best,
//! and the points of the others are set aside as class 12 (ASPRS: overlap points), so that the
//! steps that follow work on one cover of the ground.
//!
//! The ground is cut into square cells; in a cell reached by more than one flight line, the line
//! with the smallest mean absolute scan angle keeps its points and the points of the other lines
//! are overlap. A cell with one line, or a file with no flight line or scan angle, has none.

use std::collections::HashMap;

use crate::las::Point;

/// One flight line in a cell: its point source ID, the sum of the |scan angles| and the count.
type Line = (u16, f64, u32);

/// The class of overlap points.
pub const OVERLAP: u8 = 12;

/// Which points are overlap, with cells of `cell` metres. Noise and withheld points take no part.
pub fn find(pts: &[Point], cell: f64) -> Vec<bool> {
    let mut out = vec![false; pts.len()];
    if !(cell > 0.0) {
        return out;
    }
    let key = |q: &Point| ((q.x / cell).floor() as i64, (q.y / cell).floor() as i64);
    // per cell, per flight line: sum of |scan angle| and count
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
    // the line to keep in each cell reached by several
    let best: HashMap<(i64, i64), u16> = cells
        .iter()
        .filter(|(_, lines)| lines.len() > 1)
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
        // two lines over the same 5 m cell, one at 4° and one at 18°; a lone line next door
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
    fn noise_and_a_single_line_have_no_overlap() {
        let mut noise = pt(1.0, 1.0, 12, 30.0);
        noise.class = 7;
        let pts = [pt(1.0, 1.0, 10, 4.0), noise, pt(9.0, 9.0, 10, 4.0)];
        assert_eq!(find(&pts, 5.0), [false, false, false]);
        assert!(find(&pts, 0.0).iter().all(|o| !o));
    }
}
