//! Operaciones ráster sobre una rejilla: tamizado (quitar manchas pequeñas de un ráster de clases),
//! proximidad (distancia a las celdas de algunas clases) y relleno de celdas vacías por pesos del
//! inverso de la distancia.
//!
//! Escritas a partir de los algoritmos publicados: etiquetado de componentes conexas con fusión de
//! componentes pequeñas en su vecina mayor (tamiz), la transformada exacta de distancia euclídea de
//! Felzenszwalb y Huttenlocher (2012) (proximidad) y ponderación por el inverso de la distancia en
//! ocho direcciones (relleno). Las celdas vacías son `NaN`.

use rayon::prelude::*;

use crate::grid::Grid;
use crate::{Error, Result};

/// Qué celdas se tocan: las cuatro que comparten una arista o las ocho de alrededor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connectivity {
    Four,
    Eight,
}

fn same(a: f32, b: f32) -> bool {
    a == b
}

/// Sustituye las manchas (celdas conectadas de igual valor) menores que `threshold` celdas por el
/// valor de su mancha vecina mayor: se prefiere una vecina mayor a una que a su vez es pequeña. Las
/// celdas vacías no forman parte de ninguna mancha y nunca reciben un valor. Las manchas se visitan
/// en orden del ráster y una mancha fusionada cuenta su tamaño nuevo, de modo que una cadena de
/// manchas pequeñas termina en la vecina que por fin las contiene.
pub fn sieve(g: &Grid, threshold: usize, conn: Connectivity) -> Grid {
    let (cols, rows) = (g.cols, g.rows);
    let n = cols * rows;
    // etiquetar las componentes conexas con unión-búsqueda
    let mut dsu = crate::cluster::Dsu::new(n);
    let offs: &[(isize, isize)] = match conn {
        Connectivity::Four => &[(0, -1), (-1, 0)],
        Connectivity::Eight => &[(0, -1), (-1, -1), (-1, 0), (-1, 1)],
    };
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            let v = g.data[i];
            if v.is_nan() {
                continue;
            }
            for &(dr, dc) in offs {
                let (rr, cc) = (r as isize + dr, c as isize + dc);
                if rr < 0 || cc < 0 || cc >= cols as isize {
                    continue;
                }
                let j = rr as usize * cols + cc as usize;
                if same(g.data[j], v) {
                    dsu.union(i, j);
                }
            }
        }
    }
    let mut root: Vec<u32> = (0..n).map(|i| dsu.find(i) as u32).collect();
    let mut size = vec![0usize; n];
    for (i, v) in g.data.iter().enumerate() {
        if !v.is_nan() {
            size[root[i] as usize] += 1;
        }
    }
    let mut value: Vec<f32> = g.data.clone();
    // valor de la mancha por raíz
    let mut patch_value = vec![f32::NAN; n];
    for i in 0..n {
        if !g.data[i].is_nan() {
            patch_value[root[i] as usize] = g.data[i];
        }
    }
    // las celdas de cada mancha, para sus vecinas
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); n];
    for i in 0..n {
        if !g.data[i].is_nan() {
            members[root[i] as usize].push(i as u32);
        }
    }
    let all8: [(isize, isize); 8] = [
        (-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1),
    ];
    let four: [(isize, isize); 4] = [(-1, 0), (0, -1), (0, 1), (1, 0)];
    let around: &[(isize, isize)] = match conn {
        Connectivity::Four => &four,
        Connectivity::Eight => &all8,
    };
    for start in 0..n {
        if root[start] as usize != start || g.data[start].is_nan() {
            continue;
        }
        let me = start;
        if size[me] >= threshold || size[me] == 0 {
            continue;
        }
        // manchas vecinas: la mayor de las grandes o, si no, la mayor de las pequeñas
        let mut best: Option<(bool, usize, usize)> = None; // (grande, tamaño, raíz)
        for &cell in &members[me] {
            let (r, c) = ((cell as usize / cols) as isize, (cell as usize % cols) as isize);
            for &(dr, dc) in around {
                let (rr, cc) = (r + dr, c + dc);
                if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                    continue;
                }
                let j = rr as usize * cols + cc as usize;
                if value[j].is_nan() {
                    continue;
                }
                let nr = root[j] as usize;
                if nr == me {
                    continue;
                }
                let cand = (size[nr] >= threshold, size[nr], nr);
                if best.is_none_or(|b| (cand.0, cand.1) > (b.0, b.1)) {
                    best = Some(cand);
                }
            }
        }
        let Some((_, _, target)) = best else {
            continue;
        };
        let v = patch_value[target];
        let moved = std::mem::take(&mut members[me]);
        for &cell in &moved {
            value[cell as usize] = v;
            root[cell as usize] = target as u32;
        }
        size[target] += size[me];
        size[me] = 0;
        members[target].extend(moved);
    }
    let mut out = g.like();
    out.data = value;
    out
}

/// Qué cuenta como objetivo para [`proximity`].
#[derive(Clone, Debug, PartialEq)]
pub enum Targets {
    /// Celdas con estos valores.
    Values(Vec<f32>),
    /// Toda celda con un valor distinto de cero (no vacía).
    NonZero,
}

/// Para cada celda, la distancia euclídea a la celda objetivo más cercana, en unidades del mapa (o
/// en celdas cuando `in_cells`); los objetivos están a 0. Las distancias mayores que `max_distance`
/// quedan vacías. Exacto para cualquier forma de los objetivos (una pasada por eje).
pub fn proximity(g: &Grid, targets: &Targets, max_distance: Option<f64>, in_cells: bool) -> Result<Grid> {
    let (cols, rows) = (g.cols, g.rows);
    let is_target = |v: f32| {
        !v.is_nan()
            && match targets {
                Targets::Values(list) => list.contains(&v),
                Targets::NonZero => v != 0.0,
            }
    };
    const INF: f64 = 1e20;
    // distancias al cuadrado en celdas: primero columnas, luego filas (Felzenszwalb y Huttenlocher)
    let mut d2 = vec![INF; cols * rows];
    for (i, v) in g.data.iter().enumerate() {
        if is_target(*v) {
            d2[i] = 0.0;
        }
    }
    if !d2.contains(&0.0) {
        return Err(Error::Param("el ráster no tiene ninguna celda de las clases pedidas".into()));
    }
    fn edt1(f: &[f64], out: &mut [f64], v: &mut [usize], z: &mut [f64]) {
        let n = f.len();
        let mut k = 0usize;
        v[0] = 0;
        z[0] = -INF;
        z[1] = INF;
        for q in 1..n {
            loop {
                let p = v[k];
                let s = ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * q as f64 - 2.0 * p as f64);
                if s <= z[k] {
                    if k == 0 {
                        break;
                    }
                    k -= 1;
                    continue;
                }
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = INF;
                break;
            }
            if k == 0 && f[v[0]] >= INF && f[q] < INF {
                v[0] = q;
                z[0] = -INF;
                z[1] = INF;
            }
        }
        let mut k = 0usize;
        for (q, o) in out.iter_mut().enumerate() {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            *o = (q as f64 - p as f64).powi(2) + f[p];
        }
    }
    // filas: cada columna por separado
    let mut tmp = vec![INF; cols * rows];
    let cols_out: Vec<Vec<f64>> = (0..cols)
        .into_par_iter()
        .map(|c| {
            let f: Vec<f64> = (0..rows).map(|r| d2[r * cols + c]).collect();
            let mut o = vec![0.0; rows];
            let (mut v, mut z) = (vec![0usize; rows], vec![0.0; rows + 1]);
            edt1(&f, &mut o, &mut v, &mut z);
            o
        })
        .collect();
    for (c, col) in cols_out.iter().enumerate() {
        for r in 0..rows {
            tmp[r * cols + c] = col[r];
        }
    }
    let mut out = g.like();
    let unit = if in_cells { 1.0 } else { g.cell };
    out.data
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(r, line)| {
            let f = &tmp[r * cols..(r + 1) * cols];
            let mut o = vec![0.0; cols];
            let (mut v, mut z) = (vec![0usize; cols], vec![0.0; cols + 1]);
            edt1(f, &mut o, &mut v, &mut z);
            for (c, cell) in line.iter_mut().enumerate() {
                let d = o[c].sqrt() * unit;
                *cell = if d.is_finite() && o[c] < INF / 2.0 && max_distance.is_none_or(|m| d <= m) {
                    d as f32
                } else {
                    f32::NAN
                };
            }
        });
    Ok(out)
}

/// Rellena las celdas vacías a partir de las celdas con valor que las rodean: para cada una, la
/// celda con valor más cercana hallada en cada una de las ocho direcciones a menos de
/// `max_distance` celdas, ponderada por el inverso de la distancia al cuadrado. Las celdas sin
/// ninguna a su alcance quedan vacías. `smoothing` pasadas de una media 3x3 sobre las celdas
/// rellenadas solo suavizan las costuras.
pub fn fill_nodata(g: &Grid, max_distance: usize, smoothing: usize) -> Grid {
    let (cols, rows) = (g.cols as isize, g.rows as isize);
    let dirs: [(isize, isize); 8] = [
        (-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1),
    ];
    let mut out = g.clone();
    out.data
        .par_chunks_mut(g.cols)
        .enumerate()
        .for_each(|(r, line)| {
            for (c, cell) in line.iter_mut().enumerate() {
                if !cell.is_nan() {
                    continue;
                }
                let (mut sum, mut wsum) = (0.0f64, 0.0f64);
                for (dr, dc) in dirs {
                    let step = if dr != 0 && dc != 0 { std::f64::consts::SQRT_2 } else { 1.0 };
                    for k in 1..=max_distance as isize {
                        let (rr, cc) = (r as isize + dr * k, c as isize + dc * k);
                        if rr < 0 || cc < 0 || rr >= rows || cc >= cols {
                            break;
                        }
                        let v = g.data[rr as usize * g.cols + cc as usize];
                        if !v.is_nan() {
                            let d = k as f64 * step;
                            let w = 1.0 / (d * d);
                            sum += f64::from(v) * w;
                            wsum += w;
                            break;
                        }
                    }
                }
                if wsum > 0.0 {
                    *cell = (sum / wsum) as f32;
                }
            }
        });
    for _ in 0..smoothing {
        let prev = out.data.clone();
        out.data
            .par_chunks_mut(g.cols)
            .enumerate()
            .for_each(|(r, line)| {
                for (c, cell) in line.iter_mut().enumerate() {
                    if !g.data[r * g.cols + c].is_nan() || cell.is_nan() {
                        continue;
                    }
                    let (mut s, mut n) = (0.0f64, 0u32);
                    for dr in -1..=1isize {
                        for dc in -1..=1isize {
                            let (rr, cc) = (r as isize + dr, c as isize + dc);
                            if rr < 0 || cc < 0 || rr >= rows || cc >= cols {
                                continue;
                            }
                            let v = prev[rr as usize * g.cols + cc as usize];
                            if !v.is_nan() {
                                s += f64::from(v);
                                n += 1;
                            }
                        }
                    }
                    if n > 0 {
                        *cell = (s / f64::from(n)) as f32;
                    }
                }
            });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> Grid {
        let (r, c) = (rows.len(), rows[0].len());
        let mut g = Grid::new(0.0, 0.0, 2.0, c, r).unwrap();
        for (i, line) in rows.iter().enumerate() {
            for (j, ch) in line.chars().enumerate() {
                g.data[i * c + j] = match ch {
                    '.' => f32::NAN,
                    d => d.to_digit(10).unwrap() as f32,
                };
            }
        }
        g
    }

    fn text(g: &Grid) -> Vec<String> {
        g.data
            .chunks(g.cols)
            .map(|l| {
                l.iter()
                    .map(|v| if v.is_nan() { '.' } else { char::from_digit(*v as u32, 10).unwrap() })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn sieve_merges_small_patches_into_the_largest_neighbour() {
        let g = grid(&["11122", "11122", "13322", "11122"]);
        let s = sieve(&g, 3, Connectivity::Four);
        assert_eq!(text(&s), ["11122", "11122", "11122", "11122"]);
        // una celda aislada con umbral 1 se queda; las celdas vacías nunca se rellenan
        let g = grid(&["11.2", "1.22"]);
        let s = sieve(&g, 2, Connectivity::Four);
        assert_eq!(text(&s), ["11.2", "1.22"]);
        // las vecinas diagonales se unen solo con conectividad ocho
        let g = grid(&["1112", "1121", "2111"]);
        let four = sieve(&g, 2, Connectivity::Four);
        let eight = sieve(&g, 2, Connectivity::Eight);
        assert_ne!(text(&four), text(&eight));
    }

    #[test]
    fn proximity_is_the_exact_euclidean_distance() -> Result<()> {
        let g = grid(&["0000000", "0000000", "0001000", "0000000", "0000000"]);
        let p = proximity(&g, &Targets::NonZero, None, true)?;
        assert_eq!(p.data[2 * 7 + 3], 0.0);
        assert_eq!(p.data[2 * 7 + 6], 3.0);
        assert!((f64::from(p.data[0]) - 13f64.sqrt()).abs() < 1e-5);
        // en unidades del mapa (celda de 2 m) y con un límite
        let q = proximity(&g, &Targets::Values(vec![1.0]), Some(4.5), false)?;
        assert_eq!(q.data[2 * 7 + 5], 4.0);
        assert!(q.data[2 * 7 + 6].is_nan());
        // fuerza bruta sobre un ráster disperso
        let mut g = Grid::new(0.0, 0.0, 1.0, 23, 17)?;
        let mut k = 12345u64;
        for v in g.data.iter_mut() {
            k = k.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *v = if (k >> 33).is_multiple_of(40) { 1.0 } else { 0.0 };
        }
        g.data[5] = 1.0;
        let p = proximity(&g, &Targets::NonZero, None, true)?;
        for r in 0..17usize {
            for c in 0..23usize {
                let mut best = f64::INFINITY;
                for rr in 0..17usize {
                    for cc in 0..23usize {
                        if g.data[rr * 23 + cc] == 1.0 {
                            best = best.min(((r as f64 - rr as f64).powi(2) + (c as f64 - cc as f64).powi(2)).sqrt());
                        }
                    }
                }
                assert!((f64::from(p.data[r * 23 + c]) - best).abs() < 1e-4, "({c},{r})");
            }
        }
        assert!(proximity(&grid(&["00"]), &Targets::NonZero, None, true).is_err());
        Ok(())
    }

    #[test]
    fn fill_nodata_interpolates_a_hole_and_respects_the_reach() {
        let mut g = Grid::new(0.0, 0.0, 1.0, 9, 9).unwrap();
        for r in 0..9 {
            for c in 0..9 {
                g.data[r * 9 + c] = 10.0;
            }
        }
        for r in 3..6 {
            for c in 3..6 {
                g.data[r * 9 + c] = f32::NAN;
            }
        }
        let f = fill_nodata(&g, 10, 0);
        assert!(f.data.iter().all(|v| (*v - 10.0).abs() < 1e-5));
        // demasiado lejos para alcanzar nada
        let f = fill_nodata(&g, 1, 0);
        assert!(f.data[4 * 9 + 4].is_nan() && f.data[3 * 9 + 3].is_finite());
        // las celdas con valor nunca cambian
        assert_eq!(f.data[0], 10.0);
    }
}
