//! Placing sample plots without a map to start from: at random or on a regular grid, circular or
//! rectangular, inside the extent of the cloud or of a stand polygon. The result is a list of
//! regions the clip tool cuts the cloud with, and a shapefile that records where they are.

use std::path::Path;

use crate::vector::{PolyTable, Polygon, Region, Shape2};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlotShape {
    Circle {
        radius: f64,
    },
    /// Sides along x and y, in metres.
    Rect {
        w: f64,
        h: f64,
    },
}

impl PlotShape {
    fn half_extent(self) -> (f64, f64) {
        match self {
            PlotShape::Circle { radius } => (radius, radius),
            PlotShape::Rect { w, h } => (w / 2.0, h / 2.0),
        }
    }

    fn at(self, x: f64, y: f64) -> Shape2 {
        match self {
            PlotShape::Circle { radius } => Shape2::Circle {
                cx: x,
                cy: y,
                r: radius,
            },
            PlotShape::Rect { w, h } => {
                Shape2::Rect([x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0])
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sampling {
    /// `count` plots at random, no closer than `min_dist` between centres.
    Random { count: usize, min_dist: f64 },
    /// A regular grid with `dx` x `dy` between centres; `random_start` shifts it by a random
    /// offset instead of centring it.
    Systematic {
        dx: f64,
        dy: f64,
        random_start: bool,
    },
}

/// A small, seedable generator (xorshift64*): the same seed gives the same plots.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Plots inside `bounds` (min x, min y, max x, max y) whose centre is also inside one of
/// `limit` when given, and which fit entirely within `bounds`.
pub fn generate(
    bounds: [f64; 4],
    limit: Option<&[Polygon]>,
    shape: PlotShape,
    sampling: Sampling,
    seed: u64,
) -> Result<Vec<Region>> {
    let (hx, hy) = shape.half_extent();
    if !(hx > 0.0 && hy > 0.0) {
        return Err(Error::Param(
            "las parcelas necesitan un tamaño mayor que 0".into(),
        ));
    }
    let area = [
        bounds[0] + hx,
        bounds[1] + hy,
        bounds[2] - hx,
        bounds[3] - hy,
    ];
    if area[2] <= area[0] || area[3] <= area[1] {
        return Err(Error::Param(
            "las parcelas son más grandes que la zona".into(),
        ));
    }
    let inside = |x: f64, y: f64| limit.is_none_or(|l| l.iter().any(|p| p.contains(x, y)));
    let mut centres: Vec<[f64; 2]> = Vec::new();
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    match sampling {
        Sampling::Random { count, min_dist } => {
            let mut tries = 0usize;
            while centres.len() < count && tries < count.saturating_mul(500).max(1000) {
                tries += 1;
                let (x, y) = (
                    area[0] + rng.next() * (area[2] - area[0]),
                    area[1] + rng.next() * (area[3] - area[1]),
                );
                if inside(x, y)
                    && centres
                        .iter()
                        .all(|c| (c[0] - x).hypot(c[1] - y) >= min_dist)
                {
                    centres.push([x, y]);
                }
            }
            if centres.len() < count {
                return Err(Error::Param(format!(
                    "solo caben {} de {count} parcelas: reduce el número, el tamaño o la distancia mínima",
                    centres.len()
                )));
            }
        }
        Sampling::Systematic {
            dx,
            dy,
            random_start,
        } => {
            if !(dx > 0.0 && dy > 0.0) {
                return Err(Error::Param(
                    "la malla necesita un espaciado mayor que 0".into(),
                ));
            }
            let (ox, oy) = if random_start {
                (rng.next() * dx, rng.next() * dy)
            } else {
                (dx / 2.0, dy / 2.0)
            };
            let mut y = bounds[1] + oy;
            while y <= area[3] + 1e-9 {
                let mut x = bounds[0] + ox;
                while x <= area[2] + 1e-9 {
                    if x >= area[0] && y >= area[1] && inside(x, y) {
                        centres.push([x, y]);
                    }
                    x += dx;
                }
                y += dy;
            }
            if centres.len() > 1_000_000 {
                return Err(Error::Param(
                    "demasiadas parcelas: aumenta el espaciado".into(),
                ));
            }
        }
    }
    Ok(centres
        .into_iter()
        .enumerate()
        .map(|(k, c)| Region {
            name: format!("P{:03}", k + 1),
            shape: shape.at(c[0], c[1]),
            attrs: Vec::new(),
        })
        .collect())
}

/// The rings that draw a region.
pub fn rings(shape: &Shape2) -> Vec<Vec<[f64; 2]>> {
    match shape {
        Shape2::Polygon(p) => p.rings.clone(),
        Shape2::Circle { cx, cy, r } => crate::edit::circle(*cx, *cy, *r).rings,
        Shape2::Rect(b) => vec![vec![[b[0], b[1]], [b[0], b[3]], [b[2], b[3]], [b[2], b[1]]]],
    }
}

/// Saves the plots as a polygon shapefile (name, shape, size, centre).
pub fn write_layout(path: &Path, regions: &[Region], prj: Option<&str>) -> Result<()> {
    let columns: Vec<String> = ["Parcela", "Forma", "Radio", "LadoX", "LadoY", "X", "Y"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut table: Option<PolyTable> = None;
    for r in regions {
        let b = r.bbox();
        let (form, radio, lx, ly) = match r.shape {
            Shape2::Circle { r, .. } => ("circular", r, 0.0, 0.0),
            _ => ("rectangular", 0.0, b[2] - b[0], b[3] - b[1]),
        };
        let row = vec![
            r.name.clone(),
            form.to_string(),
            format!("{radio:.3}"),
            format!("{lx:.3}"),
            format!("{ly:.3}"),
            format!("{:.3}", (b[0] + b[2]) / 2.0),
            format!("{:.3}", (b[1] + b[3]) / 2.0),
        ];
        if table.is_none() {
            table = Some(PolyTable::create(path, &columns, &row, prj)?);
        }
        if let Some(t) = table.as_mut() {
            t.push(&rings(&r.shape), &row)?;
        }
    }
    if let Some(t) = table {
        t.finish()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_plots_keep_their_distance_and_stay_inside() -> Result<()> {
        let b = [0.0, 0.0, 200.0, 100.0];
        let p = generate(
            b,
            None,
            PlotShape::Circle { radius: 10.0 },
            Sampling::Random {
                count: 12,
                min_dist: 25.0,
            },
            7,
        )?;
        assert_eq!(p.len(), 12);
        let c: Vec<[f64; 4]> = p.iter().map(|r| r.bbox()).collect();
        assert!(
            c.iter()
                .all(|b| b[0] >= 0.0 && b[2] <= 200.0 && b[1] >= 0.0 && b[3] <= 100.0)
        );
        let again = generate(
            b,
            None,
            PlotShape::Circle { radius: 10.0 },
            Sampling::Random {
                count: 12,
                min_dist: 25.0,
            },
            7,
        )?;
        assert_eq!(p[3].bbox(), again[3].bbox());
        Ok(())
    }

    #[test]
    fn systematic_plots_form_a_grid() -> Result<()> {
        let p = generate(
            [0.0, 0.0, 100.0, 100.0],
            None,
            PlotShape::Rect { w: 10.0, h: 10.0 },
            Sampling::Systematic {
                dx: 50.0,
                dy: 50.0,
                random_start: false,
            },
            1,
        )?;
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].bbox(), [20.0, 20.0, 30.0, 30.0]);
        Ok(())
    }
}
