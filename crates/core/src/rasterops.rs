//! Raster operations on one grid: sieve (remove small patches of a class raster), proximity
//! (distance to the cells of some classes) and fill of empty cells by inverse distance weights.
//!
//! Written from the published algorithms: connected-component labelling with merging of small
//! components into their largest neighbour (sieve), the exact Euclidean distance transform of
//! Felzenszwalb and Huttenlocher (2012) (proximity) and inverse distance weighting along eight
//! directions (fill). Empty cells are `NaN`.

use rayon::prelude::*;

use crate::grid::Grid;
use crate::{Error, Result};

/// Which cells touch: the four sharing an edge, or all eight around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connectivity {
    Four,
    Eight,
}

fn same(a: f32, b: f32) -> bool {
    a == b
}

/// Replaces patches (connected cells of equal value) smaller than `threshold` cells by the value
/// of their largest neighbouring patch: a bigger neighbour is preferred to one that is itself
/// small. Empty cells are not part of any patch and never receive a value. Patches are visited
/// in raster order and a merged patch counts its new size, so a chain of small patches ends in
/// the neighbour that finally holds them.
pub fn sieve(g: &Grid, threshold: usize, conn: Connectivity) -> Grid {
    let (cols, rows) = (g.cols, g.rows);
    let n = cols * rows;
    // label connected components with union-find
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(p: &mut [u32], mut i: u32) -> u32 {
        while p[i as usize] != i {
            p[i as usize] = p[p[i as usize] as usize];
            i = p[i as usize];
        }
        i
    }
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
                    let (a, b) = (find(&mut parent, i as u32), find(&mut parent, j as u32));
                    if a != b {
                        parent[a.max(b) as usize] = a.min(b);
                    }
                }
            }
        }
    }
    let mut root: Vec<u32> = (0..n as u32).map(|i| find(&mut parent, i)).collect();
    let mut size = vec![0usize; n];
    for (i, v) in g.data.iter().enumerate() {
        if !v.is_nan() {
            size[root[i] as usize] += 1;
        }
    }
    let mut value: Vec<f32> = g.data.clone();
    // patch value by root
    let mut patch_value = vec![f32::NAN; n];
    for i in 0..n {
        if !g.data[i].is_nan() {
            patch_value[root[i] as usize] = g.data[i];
        }
    }
    // the cells of each patch, for its neighbours
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
        // neighbouring patches: the largest big one, else the largest small one
        let mut best: Option<(bool, usize, usize)> = None; // (big, size, root)
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

/// What counts as a target for [`proximity`].
#[derive(Clone, Debug, PartialEq)]
pub enum Targets {
    /// Cells with these values.
    Values(Vec<f32>),
    /// Every cell with a value other than zero (not empty).
    NonZero,
}

/// For each cell, the Euclidean distance to the nearest target cell, in map units (or cells when
/// `in_cells`); targets are at 0. Distances beyond `max_distance` are empty. Exact for any shape
/// of targets (one pass per axis).
pub fn proximity(g: &Grid, targets: &Targets, max_distance: Option<f64>, in_cells: bool) -> Result<Grid> {
    let (cols, rows) = (g.cols, g.rows);
    let is_target = |v: f32| {
        !v.is_nan()
            && match targets {
                Targets::Values(list) => list.iter().any(|t| *t == v),
                Targets::NonZero => v != 0.0,
            }
    };
    const INF: f64 = 1e20;
    // squared distances in cells: columns first, then rows (Felzenszwalb and Huttenlocher)
    let mut d2 = vec![INF; cols * rows];
    for (i, v) in g.data.iter().enumerate() {
        if is_target(*v) {
            d2[i] = 0.0;
        }
    }
    if !d2.iter().any(|v| *v == 0.0) {
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
    // rows: each column independently
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

/// Fills empty cells from the valued cells around them: for each, the nearest valued cell found
/// along each of the eight directions within `max_distance` cells, weighted by the inverse of
/// the squared distance. Cells with none in reach stay empty. `smoothing` passes of a 3x3 mean
/// over the filled cells only soften the seams.
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
        // an isolated cell with threshold 1 stays; empty cells are never filled
        let g = grid(&["11.2", "1.22"]);
        let s = sieve(&g, 2, Connectivity::Four);
        assert_eq!(text(&s), ["11.2", "1.22"]);
        // diagonal neighbours join only with eight-connectivity
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
        // in map units (cell 2 m) and with a limit
        let q = proximity(&g, &Targets::Values(vec![1.0]), Some(4.5), false)?;
        assert_eq!(q.data[2 * 7 + 5], 4.0);
        assert!(q.data[2 * 7 + 6].is_nan());
        // brute force on a scattered raster
        let mut g = Grid::new(0.0, 0.0, 1.0, 23, 17)?;
        let mut k = 12345u64;
        for v in g.data.iter_mut() {
            k = k.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *v = if (k >> 33) % 40 == 0 { 1.0 } else { 0.0 };
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
        // too far to reach anything
        let f = fill_nodata(&g, 1, 0);
        assert!(f.data[4 * 9 + 4].is_nan() && f.data[3 * 9 + 3].is_finite());
        // valued cells never change
        assert_eq!(f.data[0], 10.0);
    }
}
