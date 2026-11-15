//! Superficies de ponderación por el inverso de la distancia: cada celda toma la media de sus
//! puntos más cercanos ponderada por 1/d². Los puntos se reducen primero a uno por media celda (la
//! media, o el más alto para superficies de dosel), de modo que el tiempo y la memoria siguen a la
//! rejilla y no a la densidad de puntos, y la búsqueda mira anillo a anillo en ese retículo hasta
//! tener `k` puntos.

use rayon::prelude::*;

use crate::grid::Grid;

/// Vecinos promediados por celda.
pub const NEIGHBOURS: usize = 8;

/// Lo más lejos que puede estar un punto para contar (unidades del mapa), como mucho; el alcance se
/// limita además a 30 celdas. Las celdas sin ninguno se dejan vacías (y se rellenan después).
pub const MAX_DIST: f64 = 50.0;

/// Escribe en las celdas vacías de `grid` la superficie IDW por `pts` (x, y, z). Con `highest` cada
/// intervalo del retículo conserva su punto más alto en lugar de la media.
pub fn rasterize(pts: &[[f64; 3]], grid: &mut Grid, highest: bool) {
    let reach = (grid.cell * 30.0).clamp(10.0, MAX_DIST);
    let bin = (grid.cell / 2.0).max(1e-6);
    let (x0, y0) = (grid.x0 - reach, grid.y0 - reach);
    let (x1, y1) = (
        grid.x0 + grid.cols as f64 * grid.cell + reach,
        grid.y0 + grid.rows as f64 * grid.cell + reach,
    );
    let (bc, br) = (
        (((x1 - x0) / bin).ceil() as usize).max(1),
        (((y1 - y0) / bin).ceil() as usize).max(1),
    );
    if bc.saturating_mul(br) > 1 << 27 {
        return;
    }
    // por intervalo: sumas de x, y, z (o el punto más alto) y el recuento
    let mut lattice = vec![[0f64; 4]; bc * br];
    for q in pts {
        let (c, r) = (((q[0] - x0) / bin).floor(), ((q[1] - y0) / bin).floor());
        if !(c >= 0.0 && r >= 0.0 && (c as usize) < bc && (r as usize) < br) {
            continue;
        }
        let b = &mut lattice[r as usize * bc + c as usize];
        if highest {
            if b[3] == 0.0 || q[2] > b[2] {
                *b = [q[0], q[1], q[2], 1.0];
            }
        } else {
            b[0] += q[0];
            b[1] += q[1];
            b[2] += q[2];
            b[3] += 1.0;
        }
    }
    let at = |c: usize, r: usize| -> Option<[f64; 3]> {
        let b = lattice[r * bc + c];
        (b[3] > 0.0).then(|| [b[0] / b[3], b[1] / b[3], b[2] / b[3]])
    };
    // intervalos ocupados en cualquier cuadrado, en tiempo constante (tabla de áreas acumuladas)
    let w = bc + 1;
    let mut sat = vec![0u32; w * (br + 1)];
    for r in 0..br {
        let mut run = 0u32;
        for c in 0..bc {
            run += u32::from(lattice[r * bc + c][3] > 0.0);
            sat[(r + 1) * w + c + 1] = sat[r * w + c + 1] + run;
        }
    }
    let count = |c0: i64, r0: i64, c1: i64, r1: i64| -> u32 {
        let (c0, r0) = (
            c0.clamp(0, bc as i64) as usize,
            r0.clamp(0, br as i64) as usize,
        );
        let (c1, r1) = (
            (c1 + 1).clamp(0, bc as i64) as usize,
            (r1 + 1).clamp(0, br as i64) as usize,
        );
        if c1 <= c0 || r1 <= r0 {
            return 0;
        }
        sat[r1 * w + c1] + sat[r0 * w + c0] - sat[r0 * w + c1] - sat[r1 * w + c0]
    };
    let cols = grid.cols;
    let (gx0, gy_top, cell) = (grid.x0, grid.y0 + grid.rows as f64 * grid.cell, grid.cell);
    let max_ring = (reach / bin).ceil() as i64;
    let rows: Vec<Vec<f32>> = (0..grid.rows)
        .into_par_iter()
        .map(|row| {
            (0..cols)
                .map(|col| {
                    let (x, y) = (
                        gx0 + (col as f64 + 0.5) * cell,
                        gy_top - (row as f64 + 0.5) * cell,
                    );
                    let (cc, cr) = (((x - x0) / bin) as i64, ((y - y0) / bin) as i64);
                    let sq = |r: i64| count(cc - r, cr - r, cc + r, cr + r);
                    if sq(max_ring) == 0 {
                        return f32::NAN;
                    }
                    // el cuadrado más pequeño con suficientes puntos, ampliado un poco para que los
                    // más cercanos, que pueden estar fuera de sus esquinas, queden dentro
                    let (mut lo, mut hi) = (0i64, max_ring);
                    while lo < hi {
                        let mid = (lo + hi) / 2;
                        if sq(mid) as usize >= NEIGHBOURS {
                            hi = mid;
                        } else {
                            lo = mid + 1;
                        }
                    }
                    let r = ((lo as f64 * 1.42).ceil() as i64 + 1).min(max_ring);
                    let mut found: Vec<(f64, f64)> = Vec::with_capacity(64);
                    for rr in (cr - r).max(0)..=(cr + r).min(br as i64 - 1) {
                        for c in (cc - r).max(0)..=(cc + r).min(bc as i64 - 1) {
                            if let Some(p) = at(c as usize, rr as usize) {
                                let d2 = (p[0] - x).powi(2) + (p[1] - y).powi(2);
                                if d2 <= reach * reach {
                                    found.push((d2, p[2]));
                                }
                            }
                        }
                    }
                    if found.is_empty() {
                        return f32::NAN;
                    }
                    found.sort_by(|a, b| a.0.total_cmp(&b.0));
                    found.truncate(NEIGHBOURS);
                    let (mut sw, mut sz) = (0.0, 0.0);
                    for (d2, z) in found {
                        // un punto en el centro de la celda es el valor de la celda
                        if d2 < 1e-12 {
                            return z as f32;
                        }
                        let w = 1.0 / d2;
                        sw += w;
                        sz += w * z;
                    }
                    (sz / sw) as f32
                })
                .collect()
        })
        .collect();
    for (row, vals) in rows.into_iter().enumerate() {
        for (col, v) in vals.into_iter().enumerate() {
            let slot = &mut grid.data[row * cols + col];
            if slot.is_nan() {
                *slot = v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;

    #[test]
    fn plane_is_reproduced_and_highest_wins() -> Result<()> {
        let mut pts = Vec::new();
        for i in 0..40 {
            for j in 0..40 {
                let (x, y) = (f64::from(i) * 0.5, f64::from(j) * 0.5);
                pts.push([x, y, 10.0 + 0.2 * x]);
            }
        }
        let mut g = Grid::new(5.0, 5.0, 1.0, 10, 10)?;
        rasterize(&pts, &mut g, false);
        let v = g.sample(10.0, 10.0).unwrap_or(f32::NAN);
        assert!((v - 12.0).abs() < 0.15, "{v}");
        // un pico en el mismo intervalo: la superficie del punto más alto lo sigue
        pts.push([10.6, 10.6, 30.0]);
        let (mut h, mut m) = (
            Grid::new(5.0, 5.0, 1.0, 10, 10)?,
            Grid::new(5.0, 5.0, 1.0, 10, 10)?,
        );
        rasterize(&pts, &mut h, true);
        rasterize(&pts, &mut m, false);
        let at = |g: &Grid| g.data[g.index(10.55, 10.55).unwrap_or(0)];
        assert!(at(&h) > at(&m) + 0.5, "{} {}", at(&h), at(&m));
        Ok(())
    }
}
