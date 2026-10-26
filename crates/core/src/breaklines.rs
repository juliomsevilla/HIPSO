//! Breaklines in a ground model: lines where the terrain changes slope sharply (the edge of a
//! road, a ditch, a bank, the shore of a lake), which the ground points alone round off. Each
//! line is densified to vertices half a cell apart that join the ground points of the
//! triangulation, and the ground points next to the line are left out, so the triangles follow
//! the line instead of crossing it.
//!
//! Lines with elevations (PolylineZ, PolygonZ) give their own heights: they are hard lines. Lines
//! without them (a stream, a ditch drawn on the orthophoto) take, at every vertex, the lowest
//! ground point within a cell of it, which carves the channel into the model.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{Error, Result};

/// The breaklines of a job, read once.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Breaklines {
    /// Each line as vertices (x, y, z); z is NaN for lines without elevations.
    pub lines: Vec<Vec<[f64; 3]>>,
}

impl Breaklines {
    /// Reads the lines (and polygon rings) of a shapefile.
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

    /// Whether every line carries its own elevations.
    pub fn has_z(&self) -> bool {
        self.lines.iter().flatten().all(|p| p[2].is_finite())
    }

    /// The vertices of the lines inside `area` (west, south, east, north), no more than `step`
    /// apart.
    fn vertices(&self, area: [f64; 4], step: f64) -> Vec<[f64; 3]> {
        let mut out = Vec::new();
        let inside =
            |p: &[f64; 3]| p[0] >= area[0] && p[0] <= area[2] && p[1] >= area[1] && p[1] <= area[3];
        for line in &self.lines {
            for w in line.windows(2) {
                let (a, b) = (w[0], w[1]);
                // a segment wholly outside the area on one side is skipped
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

    /// Adds the lines to the ground points of an area for a model of cell `cell`: their vertices
    /// join the points and the points within half a cell of a line leave. Returns how many
    /// vertices were added.
    pub fn apply(&self, ground: &mut Vec<[f64; 3]>, area: [f64; 4], cell: f64) -> usize {
        let step = (cell / 2.0).max(0.05);
        let mut verts = self.vertices(area, step);
        if verts.is_empty() {
            return 0;
        }
        let key = |x: f64, y: f64, s: f64| ((x / s).floor() as i64, (y / s).floor() as i64);
        // lines without elevations: the lowest ground point within a cell of each vertex
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
        // the ground points next to a line give way to it
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
        // flat ground at 100 m on a 1 m grid; a road edge at 103 m across it
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
        // the cells the line runs through are at its height; far away the ground is untouched
        let at = |x: f64, y: f64| g.sample(x, y).unwrap_or(f32::NAN);
        assert!((at(20.5, 25.5) - 103.0).abs() < 1e-3, "{}", at(20.5, 25.5));
        assert!((at(20.0, 5.0) - 100.0).abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn a_2d_line_takes_the_lowest_ground_near_it() {
        // a ditch: ground at 100 m except a narrow trench at 98 m along y = 10
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
