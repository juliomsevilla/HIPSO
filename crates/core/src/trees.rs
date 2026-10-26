//! Individual trees from a canopy height model, on the raster, never on the points:
//! smoothing, tree tops by a local-maximum filter (Popescu & Wynne 2004), and crowns by
//! Dalponte & Coomes (2016), Silva et al. (2016) or a watershed. Results were checked against an independent
//! implementation (window edges, tie rules, id order).

use crate::grid::Grid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Circle,
    Square,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopParams {
    /// Lowest tree top (inclusive).
    pub hmin: f64,
    /// Window diameter = a + b h + c h² (map units); `b = c = 0` gives a fixed window.
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub shape: Shape,
}

impl Default for TopParams {
    fn default() -> Self {
        Self {
            hmin: 2.0,
            a: 3.0,
            b: 0.1,
            c: 0.0,
            shape: Shape::Circle,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segmenter {
    Dalponte,
    Silva,
    Watershed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrownParams {
    pub method: Segmenter,
    /// Lowest crown pixel (Dalponte, watershed).
    pub th_tree: f64,
    /// Dalponte: a pixel joins when higher than this share of its tree top...
    pub th_seed: f64,
    /// ...and of the crown's current mean height.
    pub th_cr: f64,
    /// Dalponte: crown reach in PIXELS (Chebyshev distance below this).
    pub max_cr: f64,
    /// Silva: crown radius as a share of the tree's height (map units).
    pub max_cr_factor: f64,
    /// Silva: pixels lower than this share of the tree's height are dropped.
    pub exclusion: f64,
    /// Watershed: minimum height difference between two crowns.
    pub tolerance: f64,
    /// Watershed: radius in pixels in which a peak must be the highest.
    pub ext: usize,
}

impl Default for CrownParams {
    fn default() -> Self {
        Self {
            method: Segmenter::Dalponte,
            th_tree: 2.0,
            th_seed: 0.45,
            th_cr: 0.55,
            max_cr: 10.0,
            max_cr_factor: 0.6,
            exclusion: 0.3,
            tolerance: 1.0,
            ext: 1,
        }
    }
}

/// One tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub height: f32,
    /// Crown area (map units²), once crowns are grown.
    pub area: f64,
}

/// Smooths a CHM with a mean, median or Gaussian filter over a window of `size` map units
/// (diameter); empty cells stay empty and are ignored by their neighbours.
pub fn smooth(chm: &Grid, size: f64, method: Smoothing, shape: Shape) -> Grid {
    let r = size / 2.0;
    let rr = (r / chm.cell + 1e-9).floor() as isize;
    let sigma = size / 6.0;
    let mut out = chm.like();
    let (cols, rows) = (chm.cols as isize, chm.rows as isize);
    let mut buf: Vec<(f32, f32)> = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let i = (row * cols + col) as usize;
            if chm.data[i].is_nan() {
                continue;
            }
            buf.clear();
            for dr in -rr..=rr {
                for dc in -rr..=rr {
                    let (nr, nc) = (row + dr, col + dc);
                    if nr < 0 || nc < 0 || nr >= rows || nc >= cols {
                        continue;
                    }
                    let (dx, dy) = (dc as f64 * chm.cell, dr as f64 * chm.cell);
                    let inside = match shape {
                        Shape::Circle => dx * dx + dy * dy <= r * r + 1e-9,
                        Shape::Square => dx.abs() <= r + 1e-9 && dy.abs() <= r + 1e-9,
                    };
                    let v = chm.data[(nr * cols + nc) as usize];
                    if inside && !v.is_nan() {
                        let w = ((-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()) as f32;
                        buf.push((v, w));
                    }
                }
            }
            out.data[i] = match method {
                Smoothing::Mean => buf.iter().map(|b| b.0).sum::<f32>() / buf.len() as f32,
                Smoothing::Gaussian => {
                    let s: f32 = buf.iter().map(|b| b.1).sum();
                    buf.iter().map(|b| b.0 * b.1).sum::<f32>() / s
                }
                Smoothing::Median => {
                    buf.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let n = buf.len();
                    if n % 2 == 1 {
                        buf[n / 2].0
                    } else {
                        (buf[n / 2 - 1].0 + buf[n / 2].0) / 2.0
                    }
                }
            };
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smoothing {
    Mean,
    Median,
    Gaussian,
}

/// Tree tops: cells at least `hmin` high and highest within the window their own height gives
/// (edge inclusive); equal cells: the first in reading order wins, and only a tie with an
/// already chosen top rules a cell out (a long equal ridge gives a top every window). Ids follow reading order.
pub fn tops(chm: &Grid, p: &TopParams) -> Vec<Tree> {
    let (cols, rows) = (chm.cols as isize, chm.rows as isize);
    let mut out = Vec::new();
    let mut is_top = vec![false; chm.data.len()];
    for r in 0..rows {
        for c in 0..cols {
            let i = (r * cols + c) as usize;
            let h = chm.data[i];
            if h.is_nan() || f64::from(h) < p.hmin {
                continue;
            }
            let hd = f64::from(h);
            let radius = (p.a + p.b * hd + p.c * hd * hd) / 2.0;
            let rr = (radius / chm.cell + 1e-9).floor() as isize;
            let mut top = true;
            'window: for dr in -rr..=rr {
                for dc in -rr..=rr {
                    let (dx, dy) = (dc as f64 * chm.cell, dr as f64 * chm.cell);
                    let inside = match p.shape {
                        Shape::Circle => dx * dx + dy * dy <= radius * radius + 1e-9,
                        Shape::Square => dx.abs() <= radius + 1e-9 && dy.abs() <= radius + 1e-9,
                    };
                    let (nr, nc) = (r + dr, c + dc);
                    if (dr == 0 && dc == 0)
                        || !inside
                        || nr < 0
                        || nc < 0
                        || nr >= rows
                        || nc >= cols
                    {
                        continue;
                    }
                    let j = (nr * cols + nc) as usize;
                    let v = chm.data[j];
                    if v > h || (v == h && j < i && is_top[j]) {
                        top = false;
                        break 'window;
                    }
                }
            }
            if top {
                is_top[i] = true;
                let (x, y) = chm.center(c as usize, r as usize);
                out.push(Tree {
                    id: out.len() as u32 + 1,
                    x,
                    y,
                    height: h,
                    area: 0.0,
                });
            }
        }
    }
    out
}

/// Cells holding a tree top (index into the grid) for each tree, in list order.
fn seeds(chm: &Grid, trees: &[Tree]) -> Vec<Option<usize>> {
    trees
        .iter()
        .map(|t| chm.col_row(t.x, t.y).map(|(c, r)| r * chm.cols + c))
        .collect()
}

/// Grows crowns from the tree tops with the method in `p`; the crown id of each cell (0: none).
/// Sets each tree's area. A watershed ignores `trees` and replaces it with its own.
pub fn crowns(chm: &Grid, trees: &mut Vec<Tree>, p: &CrownParams) -> Vec<u32> {
    let id = match p.method {
        Segmenter::Dalponte => dalponte(chm, trees, p),
        Segmenter::Silva => silva(chm, trees, p),
        Segmenter::Watershed => {
            let id = watershed(chm, p);
            *trees = trees_from_regions(chm, &id);
            id
        }
    };
    let mut count = vec![0u32; trees.iter().map(|t| t.id).max().unwrap_or(0) as usize + 1];
    for &v in &id {
        if let Some(c) = count.get_mut(v as usize) {
            *c += 1;
        }
    }
    for t in trees.iter_mut() {
        t.area = f64::from(count.get(t.id as usize).copied().unwrap_or(0)) * chm.cell * chm.cell;
    }
    id
}

/// Dalponte & Coomes: every round, each crown (in list order) offers its 4-neighbours to the
/// crown; a pixel is taken when it is above `th_tree`, above `th_seed` × the top, above `th_cr` ×
/// the crown mean, at most 5 % higher than the top, and closer than `max_cr` pixels (Chebyshev).
/// Two crowns reaching a pixel in the same round: the later one in the list wins.
fn dalponte(chm: &Grid, trees: &[Tree], p: &CrownParams) -> Vec<u32> {
    let cols = chm.cols;
    let mut id = vec![0u32; chm.data.len()];
    let mut stamp = vec![0u32; chm.data.len()];
    struct Crown {
        tree: u32,
        seed: (isize, isize),
        top: f32,
        sum: f64,
        n: u32,
        cells: Vec<usize>,
    }
    let mut cr: Vec<Crown> = Vec::new();
    for (t, s) in trees.iter().zip(seeds(chm, trees)) {
        let Some(i) = s.filter(|&i| id[i] == 0) else {
            continue;
        };
        id[i] = t.id;
        cr.push(Crown {
            tree: t.id,
            seed: ((i % cols) as isize, (i / cols) as isize),
            top: chm.data[i],
            sum: f64::from(chm.data[i]),
            n: 1,
            cells: vec![i],
        });
    }
    let slot: std::collections::HashMap<u32, usize> =
        cr.iter().enumerate().map(|(k, c)| (c.tree, k)).collect();
    let (icols, irows) = (cols as isize, chm.rows as isize);
    for round in 1u32.. {
        let mut changed = false;
        for k in 0..cr.len() {
            let mean = (cr[k].sum / f64::from(cr[k].n)) as f32;
            let (top, seed, me) = (cr[k].top, cr[k].seed, cr[k].tree);
            let known = cr[k].cells.len();
            for m in 0..known {
                let i = cr[k].cells[m];
                if id[i] != me {
                    continue;
                }
                let (c, r) = ((i % cols) as isize, (i / cols) as isize);
                for (nc, nr) in [(c, r - 1), (c - 1, r), (c + 1, r), (c, r + 1)] {
                    if nc < 0 || nr < 0 || nc >= icols || nr >= irows {
                        continue;
                    }
                    let j = nr as usize * cols + nc as usize;
                    let h = chm.data[j];
                    let free = id[j] == 0 || (stamp[j] == round && id[j] != me);
                    if !free
                        || h.is_nan()
                        || f64::from(h) <= p.th_tree
                        || f64::from(h) <= f64::from(top) * p.th_seed
                        || f64::from(h) <= f64::from(mean) * p.th_cr
                        || f64::from(h) > f64::from(top) * 1.05
                        || ((nc - seed.0).abs().max((nr - seed.1).abs()) as f64) >= p.max_cr
                    {
                        continue;
                    }
                    if id[j] != 0
                        && let Some(&o) = slot.get(&id[j])
                    {
                        cr[o].sum -= f64::from(h);
                        cr[o].n -= 1;
                    }
                    id[j] = me;
                    stamp[j] = round;
                    cr[k].sum += f64::from(h);
                    cr[k].n += 1;
                    cr[k].cells.push(j);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    id
}

/// Silva et al.: every pixel goes to its nearest tree top (Euclidean, ties to the first in the
/// list); it is dropped when farther than `max_cr_factor` × that tree's height or lower than
/// `exclusion` × that height (both edges inclusive).
fn silva(chm: &Grid, trees: &[Tree], p: &CrownParams) -> Vec<u32> {
    let cols = chm.cols;
    let mut id = vec![0u32; chm.data.len()];
    let mut best = vec![f64::INFINITY; chm.data.len()];
    let mut owner = vec![usize::MAX; chm.data.len()];
    let reach = trees
        .iter()
        .map(|t| f64::from(t.height) * p.max_cr_factor)
        .fold(0.0, f64::max);
    let rr = (reach / chm.cell + 1e-9).ceil() as isize;
    let (icols, irows) = (cols as isize, chm.rows as isize);
    for (k, s) in seeds(chm, trees).into_iter().enumerate() {
        let Some(s) = s else { continue };
        let (c0, r0) = ((s % cols) as isize, (s / cols) as isize);
        for r in (r0 - rr).max(0)..=(r0 + rr).min(irows - 1) {
            for c in (c0 - rr).max(0)..=(c0 + rr).min(icols - 1) {
                let j = r as usize * cols + c as usize;
                if chm.data[j].is_nan() {
                    continue;
                }
                let d = (((c - c0).pow(2) + (r - r0).pow(2)) as f64).sqrt() * chm.cell;
                if d < best[j] {
                    best[j] = d;
                    owner[j] = k;
                }
            }
        }
    }
    for j in 0..id.len() {
        let Some(t) = trees.get(owner[j]) else {
            continue;
        };
        let h = f64::from(t.height);
        if best[j] <= h * p.max_cr_factor + 1e-9 && f64::from(chm.data[j]) >= h * p.exclusion - 1e-9
        {
            id[j] = t.id;
        }
    }
    id
}

/// Watershed on the CHM turned upside down: each peak (highest within `ext` pixels, above
/// `th_tree`) floods downhill; two crowns join when the lower peak stands less than `tolerance`
/// above the pass between them. Ids follow reading order of the surviving peaks.
fn watershed(chm: &Grid, p: &CrownParams) -> Vec<u32> {
    let n = chm.data.len();
    let (cols, rows) = (chm.cols as isize, chm.rows as isize);
    let ext = p.ext as isize;
    let valid = |i: usize| !chm.data[i].is_nan() && f64::from(chm.data[i]) > p.th_tree;
    // label of each pixel: provisional peak index + 1
    let mut label = vec![0u32; n];
    let mut peak_h: Vec<f32> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let i = (r * cols + c) as usize;
            if !valid(i) {
                continue;
            }
            let h = chm.data[i];
            let mut top = true;
            'w: for dr in -ext..=ext {
                for dc in -ext..=ext {
                    let (nr, nc) = (r + dr, c + dc);
                    if (dr == 0 && dc == 0) || nr < 0 || nc < 0 || nr >= rows || nc >= cols {
                        continue;
                    }
                    let j = (nr * cols + nc) as usize;
                    if chm.data[j] > h || (chm.data[j] == h && j < i) {
                        top = false;
                        break 'w;
                    }
                }
            }
            if top {
                peak_h.push(h);
                label[i] = peak_h.len() as u32;
            }
        }
    }
    // union-find over peaks, the higher peak is the root
    let mut parent: Vec<usize> = (0..peak_h.len()).collect();
    fn root(parent: &mut [usize], mut a: usize) -> usize {
        while parent[a] != a {
            parent[a] = parent[parent[a]];
            a = parent[a];
        }
        a
    }
    let mut order: Vec<usize> = (0..n).filter(|&i| valid(i)).collect();
    order.sort_by(|&a, &b| chm.data[b].total_cmp(&chm.data[a]).then(a.cmp(&b)));
    for &i in &order {
        let (c, r) = ((i % chm.cols) as isize, (i / chm.cols) as isize);
        let mut seen: Vec<usize> = Vec::new();
        for (nc, nr) in [(c, r - 1), (c - 1, r), (c + 1, r), (c, r + 1)] {
            if nc < 0 || nr < 0 || nc >= cols || nr >= rows {
                continue;
            }
            let l = label[(nr * cols + nc) as usize];
            if l != 0 {
                let rt = root(&mut parent, l as usize - 1);
                if !seen.contains(&rt) {
                    seen.push(rt);
                }
            }
        }
        if label[i] == 0 {
            let Some(&first) = seen
                .iter()
                .max_by(|&&a, &&b| peak_h[a].total_cmp(&peak_h[b]))
            else {
                continue;
            };
            label[i] = first as u32 + 1;
        }
        // merge crowns whose lower peak is not `tolerance` above this pass
        let mut keep = root(&mut parent, label[i] as usize - 1);
        for &s in &seen {
            let s = root(&mut parent, s);
            if s == keep {
                continue;
            }
            let (hi, lo) = if peak_h[s] > peak_h[keep] || (peak_h[s] == peak_h[keep] && s < keep) {
                (s, keep)
            } else {
                (keep, s)
            };
            if f64::from(peak_h[lo]) - f64::from(chm.data[i]) < p.tolerance {
                parent[lo] = hi;
                keep = hi;
            }
        }
    }
    // renumber by reading order of each region's first pixel
    let mut new_id = vec![0u32; peak_h.len()];
    let mut next = 0u32;
    let mut out = vec![0u32; n];
    for i in 0..n {
        if label[i] == 0 {
            continue;
        }
        let rt = root(&mut parent, label[i] as usize - 1);
        if new_id[rt] == 0 {
            next += 1;
            new_id[rt] = next;
        }
        out[i] = new_id[rt];
    }
    out
}

/// One tree per region of `ids`: its highest cell (the first in reading order on a tie).
pub fn trees_from_regions(chm: &Grid, ids: &[u32]) -> Vec<Tree> {
    let mut best: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for (i, &v) in ids.iter().enumerate() {
        if v == 0 {
            continue;
        }
        let e = best.entry(v).or_insert(i);
        if chm.data[i] > chm.data[*e] {
            *e = i;
        }
    }
    best.into_iter()
        .map(|(id, i)| {
            let (x, y) = chm.center(i % chm.cols, i / chm.cols);
            Tree {
                id,
                x,
                y,
                height: chm.data[i],
                area: 0.0,
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unreachable)]
mod tests {
    use super::*;

    fn grid(m: &[&[f32]], cell: f64) -> Grid {
        let (rows, cols) = (m.len(), m[0].len());
        let mut g = Grid::new(0.0, 0.0, cell, cols, rows).unwrap_or_else(|_| unreachable!());
        for (r, row) in m.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                g.data[r * cols + c] = *v;
            }
        }
        g
    }

    fn cone(n: usize) -> (Grid, Vec<Tree>) {
        let mut g = Grid::new(0.0, 0.0, 1.0, n, n).unwrap_or_else(|_| unreachable!());
        let c = (n / 2) as f64;
        for r in 0..n {
            for k in 0..n {
                let d = ((r as f64 - c).powi(2) + (k as f64 - c).powi(2)).sqrt();
                g.data[r * n + k] = (10.0 - d).max(0.0) as f32;
            }
        }
        let (x, y) = g.center(n / 2, n / 2);
        let t = vec![Tree {
            id: 1,
            x,
            y,
            height: 10.0,
            area: 0.0,
        }];
        (g, t)
    }

    fn count(ids: &[u32]) -> usize {
        ids.iter().filter(|v| **v != 0).count()
    }

    #[test]
    fn dalponte_cone_counts() {
        let (g, t) = cone(41);
        let mk = |ts: f64, tc: f64, tt: f64| CrownParams {
            th_seed: ts,
            th_cr: tc,
            th_tree: tt,
            max_cr: 40.0,
            ..CrownParams::default()
        };
        assert_eq!(count(&dalponte(&g, &t, &mk(0.45, 0.55, 2.0))), 97);
        assert_eq!(count(&dalponte(&g, &t, &mk(0.45, 0.90, 2.0))), 1);
        assert_eq!(count(&dalponte(&g, &t, &mk(0.80, 0.55, 2.0))), 9);
        assert_eq!(count(&dalponte(&g, &t, &mk(0.80, 0.90, 2.0))), 1);
        assert_eq!(count(&dalponte(&g, &t, &mk(0.45, 0.55, 5.0))), 69);
    }

    #[test]
    fn dalponte_reach_and_equal_seeds() {
        let plateau = vec![10.0f32; 41];
        let rows: Vec<&[f32]> = (0..41).map(|_| plateau.as_slice()).collect();
        let g = grid(&rows, 1.0);
        let (x, y) = g.center(20, 20);
        let t = vec![Tree {
            id: 1,
            x,
            y,
            height: 10.0,
            area: 0.0,
        }];
        let p = |m: f64| CrownParams {
            max_cr: m,
            ..CrownParams::default()
        };
        assert_eq!(count(&dalponte(&g, &t, &p(3.0))), 25);
        assert_eq!(count(&dalponte(&g, &t, &p(5.0))), 81);
        // two equal seeds on a row: the second takes the shared pixel
        let row = vec![10.0f32; 21];
        let rows: Vec<&[f32]> = (0..9).map(|_| row.as_slice()).collect();
        let g = grid(&rows, 1.0);
        let (x1, y1) = g.center(5, 4);
        let (x2, y2) = g.center(15, 4);
        let t = vec![
            Tree {
                id: 2,
                x: x1,
                y: y1,
                height: 10.0,
                area: 0.0,
            },
            Tree {
                id: 1,
                x: x2,
                y: y2,
                height: 10.0,
                area: 0.0,
            },
        ];
        let id = dalponte(&g, &t, &p(20.0));
        let line: Vec<u32> = id[4 * 21..5 * 21].to_vec();
        assert_eq!(line.iter().filter(|v| **v == 2).count(), 10);
        assert_eq!(line.iter().filter(|v| **v == 1).count(), 11);
    }

    #[test]
    fn silva_radius_and_exclusion_edges() {
        let (g, t) = cone(41);
        let mut plateau = g.like();
        plateau.data.iter_mut().for_each(|v| *v = 10.0);
        let p = CrownParams {
            max_cr_factor: 0.3,
            exclusion: 0.3,
            ..CrownParams::default()
        };
        // radius 3 map units, edge inclusive: 29 pixels
        assert_eq!(count(&silva(&plateau, &t, &p)), 29);
    }

    #[test]
    fn lmf_ties_and_edges() {
        // 2x2 plateau: the first cell in reading order is the only top
        let mut m = vec![vec![0.0f32; 9]; 9];
        m[3][3] = 10.0;
        m[3][4] = 10.0;
        m[4][3] = 10.0;
        m[4][4] = 10.0;
        let rows: Vec<&[f32]> = m.iter().map(|r| r.as_slice()).collect();
        let g = grid(&rows, 1.0);
        let t = tops(
            &g,
            &TopParams {
                a: 5.0,
                b: 0.0,
                ..TopParams::default()
            },
        );
        assert_eq!(t.len(), 1);
        assert_eq!(g.col_row(t[0].x, t[0].y), Some((3, 3)));
        // peak 9 at distance 2 from peak 10: window 4 (radius 2) is inclusive, so it is not a top
        let mut m = vec![vec![0.0f32; 13]; 9];
        m[4][3] = 10.0;
        m[4][5] = 9.0;
        let rows: Vec<&[f32]> = m.iter().map(|r| r.as_slice()).collect();
        let g = grid(&rows, 1.0);
        let fixed = |a: f64| TopParams {
            a,
            b: 0.0,
            ..TopParams::default()
        };
        assert_eq!(tops(&g, &fixed(4.0)).len(), 1);
        assert_eq!(tops(&g, &fixed(3.9)).len(), 2);
    }

    #[test]
    fn watershed_splits_two_peaks() {
        let mut g = Grid::new(0.0, 0.0, 1.0, 30, 15).unwrap_or_else(|_| unreachable!());
        for r in 0..15 {
            for c in 0..30 {
                let (x, y) = g.center(c, r);
                let cn = |cx: f64, cy: f64, h: f64| {
                    (h - 2.0 * ((x - cx).powi(2) + (y - cy).powi(2)).sqrt()).max(0.0)
                };
                g.data[r * 30 + c] = cn(7.5, 7.5, 15.0).max(cn(22.5, 7.5, 12.0)) as f32;
            }
        }
        let mut t = Vec::new();
        let p = CrownParams {
            method: Segmenter::Watershed,
            ..CrownParams::default()
        };
        let id = crowns(&g, &mut t, &p);
        assert_eq!(t.len(), 2);
        assert!(count(&id) > 20);
    }
}
