//! Road network from a classified cloud: a mask of passable ground (bare ground returns, low
//! slope, optionally the intensity of the surface), thinned to a skeleton and traced into lines
//! that carry the width of the road.
//!
//! Method: per cell, the share of returns classified as ground, their mean height and
//! intensity; slope from the ground surface (Horn); the mask keeps cells with a high ground
//! share, a slope under the limit and an intensity in range; patches and holes smaller than the
//! minimum road area are removed ([`crate::rasterops::sieve`]); Zhang and Suen's thinning (1984)
//! gives the skeleton; branches that end in a free tip and are shorter than `spur_length` are
//! pruned; lines are simplified with Douglas and Peucker's algorithm. The width of a line is
//! twice the distance from its skeleton to the nearest cell that is not road, less half a cell
//! (the skeleton of an even number of cells sits half a cell off the middle)
//! (exact Euclidean distance transform, [`crate::rasterops::proximity`]).

use crate::grid::Grid;
use crate::las::Point;
use crate::rasterops::{Connectivity, Targets, proximity, sieve};
use crate::terrain::{Edges, slope};
use crate::{Error, Result};

/// Parameters of the extraction.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadParams {
    /// Cell of the mask, metres. About a third of the narrowest road or finer.
    pub cell: f64,
    /// Smallest share of returns that are ground (class 2) for a cell to be road.
    pub min_ground_share: f64,
    /// Steepest slope of the ground surface, percent.
    pub max_slope: f64,
    /// Range of the mean intensity of the ground returns of a road cell, when roads differ from
    /// the bare ground around them.
    pub intensity: Option<(f64, f64)>,
    /// Narrowest road, metres: patches smaller than this wide by `min_length` are dropped.
    pub min_width: f64,
    /// Shortest road, metres.
    pub min_length: f64,
    /// Gaps in the road (under a crown, a car) up to this area, m², are closed; road patches
    /// smaller than it are dropped.
    pub fill_area: f64,
    /// Branches ending in a tip shorter than this are removed, metres.
    pub spur_length: f64,
    /// Tolerance of the line simplification, metres.
    pub simplify: f64,
}

impl Default for RoadParams {
    fn default() -> Self {
        Self {
            cell: 1.0,
            min_ground_share: 0.5,
            max_slope: 15.0,
            intensity: None,
            min_width: 2.0,
            min_length: 20.0,
            fill_area: 150.0,
            spur_length: 6.0,
            simplify: 1.0,
        }
    }
}

/// A road line.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadLine {
    pub points: Vec<[f64; 2]>,
    /// Mean width of the road along the line, metres.
    pub width: f64,
    pub length: f64,
}

/// The mask of road cells: 1 road, 0 not (and empty where no points fall).
pub fn road_mask(pts: &[Point], p: &RoadParams) -> Result<Grid> {
    if !(p.cell > 0.0) {
        return Err(Error::Param("la celda debe ser positiva".into()));
    }
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for q in pts.iter().filter(|q| !q.is_noise()) {
        b = [b[0].min(q.x), b[1].min(q.y), b[2].max(q.x), b[3].max(q.y)];
    }
    if !b[0].is_finite() {
        return Err(Error::Param("no hay puntos".into()));
    }
    let mut g = Grid::covering(b, p.cell)?;
    let n = g.data.len();
    let (mut total, mut ground) = (vec![0u32; n], vec![0u32; n]);
    let (mut zsum, mut isum) = (vec![0f64; n], vec![0f64; n]);
    for q in pts.iter().filter(|q| !q.is_noise()) {
        let (c, r) = (
            ((q.x - g.x0) / g.cell) as usize,
            g.rows - 1 - (((q.y - g.y0) / g.cell) as usize).min(g.rows - 1),
        );
        let i = r * g.cols + c.min(g.cols - 1);
        total[i] += 1;
        if q.class == 2 {
            ground[i] += 1;
            zsum[i] += q.z;
            isum[i] += f64::from(q.intensity);
        }
    }
    let mut dem = g.like();
    for i in 0..n {
        if ground[i] > 0 {
            dem.data[i] = (zsum[i] / f64::from(ground[i])) as f32;
        }
    }
    // the slope where the ground is bare is what matters: cells without ground are not road
    // the ground surface smoothed over 3x3 cells before the slope, so that the height noise of a
    // few returns per cell does not read as slope
    let mut smooth = dem.like();
    for r in 0..g.rows {
        for c in 0..g.cols {
            let (mut sum, mut cnt) = (0.0f64, 0u32);
            for rr in r.saturating_sub(1)..=(r + 1).min(g.rows - 1) {
                for cc in c.saturating_sub(1)..=(c + 1).min(g.cols - 1) {
                    let v = dem.data[rr * g.cols + cc];
                    if !v.is_nan() {
                        sum += f64::from(v);
                        cnt += 1;
                    }
                }
            }
            if cnt > 0 {
                smooth.data[r * g.cols + c] = (sum / f64::from(cnt)) as f32;
            }
        }
    }
    let slopes = slope(&smooth, 1.0, true, Edges::Center);
    for i in 0..n {
        if total[i] == 0 {
            continue;
        }
        let share = f64::from(ground[i]) / f64::from(total[i]);
        let mut road = share >= p.min_ground_share && slopes.data[i].is_finite() && f64::from(slopes.data[i]) <= p.max_slope;
        if road && let Some((lo, hi)) = p.intensity {
            let mean = isum[i] / f64::from(ground[i]);
            road = mean >= lo && mean <= hi;
        }
        g.data[i] = if road { 1.0 } else { 0.0 };
    }
    // cells without any point inside the cloud's extent are gaps: treat as not road
    for v in g.data.iter_mut() {
        if v.is_nan() {
            *v = 0.0;
        }
    }
    let min_cells = (((p.min_width * p.min_length).max(p.fill_area)) / (p.cell * p.cell)).ceil().max(2.0) as usize;
    Ok(sieve(&g, min_cells, Connectivity::Eight))
}

/// Zhang and Suen's thinning of a boolean raster (true = foreground), in place.
fn thin(m: &mut [bool], cols: usize, rows: usize) {
    let at = |m: &[bool], r: isize, c: isize| -> bool {
        r >= 0 && c >= 0 && (r as usize) < rows && (c as usize) < cols && m[r as usize * cols + c as usize]
    };
    loop {
        let mut changed = false;
        for pass in 0..2 {
            let mut del = Vec::new();
            for r in 0..rows as isize {
                for c in 0..cols as isize {
                    if !m[r as usize * cols + c as usize] {
                        continue;
                    }
                    // P2..P9 clockwise from north
                    let n = [
                        at(m, r - 1, c),
                        at(m, r - 1, c + 1),
                        at(m, r, c + 1),
                        at(m, r + 1, c + 1),
                        at(m, r + 1, c),
                        at(m, r + 1, c - 1),
                        at(m, r, c - 1),
                        at(m, r - 1, c - 1),
                    ];
                    let b: usize = n.iter().filter(|v| **v).count();
                    if !(2..=6).contains(&b) {
                        continue;
                    }
                    let a = (0..8).filter(|&k| !n[k] && n[(k + 1) % 8]).count();
                    if a != 1 {
                        continue;
                    }
                    let ok = if pass == 0 {
                        !(n[0] && n[2] && n[4]) && !(n[2] && n[4] && n[6])
                    } else {
                        !(n[0] && n[2] && n[6]) && !(n[0] && n[4] && n[6])
                    };
                    if ok {
                        del.push(r as usize * cols + c as usize);
                    }
                }
            }
            if !del.is_empty() {
                changed = true;
                for i in del {
                    m[i] = false;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

const N8: [(isize, isize); 8] = [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)];

fn neighbours(s: &[bool], cols: usize, rows: usize, i: usize) -> Vec<usize> {
    let (r, c) = ((i / cols) as isize, (i % cols) as isize);
    N8.iter()
        .filter_map(|(dr, dc)| {
            let (rr, cc) = (r + dr, c + dc);
            (rr >= 0 && cc >= 0 && (rr as usize) < rows && (cc as usize) < cols)
                .then(|| rr as usize * cols + cc as usize)
                .filter(|&j| s[j])
        })
        .collect()
}

/// An edge of the skeleton graph: the cells between two vertices (junction or tip clusters).
struct Edge {
    a: usize,
    b: usize,
    cells: Vec<usize>,
}

/// The skeleton as a graph. Cells with other than two neighbours are nodes; touching nodes form
/// one vertex (a staircase corner or a thick junction is one place, not several).
struct Graph {
    /// Cell indices of each vertex's cluster.
    vertices: Vec<Vec<usize>>,
    edges: Vec<Edge>,
    /// Skeleton cells on no edge nor vertex: closed loops, as lists.
    loops: Vec<Vec<usize>>,
}

fn build_graph(s: &[bool], cols: usize, rows: usize) -> Graph {
    let n = s.len();
    let deg: Vec<usize> = (0..n).map(|i| if s[i] { neighbours(s, cols, rows, i).len() } else { 0 }).collect();
    let is_node = |i: usize| s[i] && deg[i] != 2;
    let mut cluster = vec![usize::MAX; n];
    let mut vertices: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        if !is_node(i) || cluster[i] != usize::MAX {
            continue;
        }
        let id = vertices.len();
        let mut stack = vec![i];
        cluster[i] = id;
        let mut cells = Vec::new();
        while let Some(c) = stack.pop() {
            cells.push(c);
            for j in neighbours(s, cols, rows, c) {
                if is_node(j) && cluster[j] == usize::MAX {
                    cluster[j] = id;
                    stack.push(j);
                }
            }
        }
        vertices.push(cells);
    }
    let mut seen = vec![false; n];
    let mut edges = Vec::new();
    for v in 0..vertices.len() {
        for &c in &vertices[v] {
            for j in neighbours(s, cols, rows, c) {
                if is_node(j) || seen[j] {
                    continue;
                }
                let (mut prev, mut cur) = (c, j);
                let mut cells = Vec::new();
                let end = loop {
                    seen[cur] = true;
                    cells.push(cur);
                    let nbs = neighbours(s, cols, rows, cur);
                    let next = nbs
                        .iter()
                        .copied()
                        .filter(|&q| q != prev && !cells.contains(&q))
                        .min_by_key(|&q| {
                            let (dr, dc) = ((q / cols) as isize - (cur / cols) as isize, (q % cols) as isize - (cur % cols) as isize);
                            (!is_node(q), usize::from(dr != 0 && dc != 0))
                        });
                    match next {
                        Some(q) if is_node(q) => break Some(cluster[q]),
                        Some(q) => {
                            prev = cur;
                            cur = q;
                        }
                        None => break None,
                    }
                };
                if let Some(b) = end {
                    edges.push(Edge { a: v, b, cells });
                }
            }
        }
    }
    let mut loops = Vec::new();
    for i in 0..n {
        if s[i] && !seen[i] && !is_node(i) && cluster[i] == usize::MAX {
            let (mut prev, mut cur) = (usize::MAX, i);
            let mut cells = Vec::new();
            loop {
                seen[cur] = true;
                cells.push(cur);
                let next = neighbours(s, cols, rows, cur).into_iter().find(|&q| q != prev && !seen[q]);
                match next {
                    Some(q) => {
                        prev = cur;
                        cur = q;
                    }
                    None => break,
                }
            }
            if cells.len() >= 4 {
                loops.push(cells);
            }
        }
    }
    Graph { vertices, edges, loops }
}

fn douglas_peucker(pts: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy);
    let mut worst = (0.0, 0usize);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = if len > 0.0 {
            ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len
        } else {
            (p[0] - a[0]).hypot(p[1] - a[1])
        };
        if d > worst.0 {
            worst = (d, i);
        }
    }
    if worst.0 > tol {
        let mut l = douglas_peucker(&pts[..=worst.1], tol);
        let r = douglas_peucker(&pts[worst.1..], tol);
        l.pop();
        l.extend(r);
        l
    } else {
        vec![a, b]
    }
}

/// The road lines of a classified cloud.
pub fn extract(pts: &[Point], p: &RoadParams) -> Result<Vec<RoadLine>> {
    let mask = road_mask(pts, p)?;
    lines_of_mask(&mask, p)
}

/// The lines of a road mask (1 road).
pub fn lines_of_mask(mask: &Grid, p: &RoadParams) -> Result<Vec<RoadLine>> {
    let (cols, rows) = (mask.cols, mask.rows);
    let mut s: Vec<bool> = mask.data.iter().map(|v| *v == 1.0).collect();
    if !s.iter().any(|v| *v) {
        return Ok(Vec::new());
    }
    // distance to the nearest non-road cell, for the width
    let dist = proximity(mask, &Targets::Values(vec![0.0]), None, false)
        .map_err(|_| Error::Param("la máscara no tiene fondo".into()))?;
    thin(&mut s, cols, rows);
    let spur_len = p.spur_length;
    let xy_of = |i: usize| {
        let (x, y) = mask.center(i % cols, i / cols);
        [x, y]
    };
    let cell_len = |cells: &[usize]| -> f64 {
        cells.windows(2).map(|w| { let (a, b) = (xy_of(w[0]), xy_of(w[1])); (b[0] - a[0]).hypot(b[1] - a[1]) }).sum()
    };
    // prune branches that end in a free tip and are shorter than the spur length
    let mut g = build_graph(&s, cols, rows);
    for _ in 0..6 {
        let mut degree = vec![0usize; g.vertices.len()];
        for e in &g.edges {
            degree[e.a] += 1;
            degree[e.b] += 1;
        }
        let mut drop = Vec::new();
        for (k, e) in g.edges.iter().enumerate() {
            let tip = |v: usize| degree[v] == 1;
            let junction = |v: usize| degree[v] >= 3;
            if (tip(e.a) && junction(e.b)) || (tip(e.b) && junction(e.a)) {
                // the length counts from the vertex it hangs on
                if cell_len(&e.cells) + p.cell <= spur_len {
                    drop.push(k);
                }
            }
        }
        if drop.is_empty() {
            break;
        }
        for k in drop {
            for &c in &g.edges[k].cells {
                s[c] = false;
            }
            // the tip cluster goes with it
            let e = &g.edges[k];
            let tip_v = {
                let mut d = vec![0usize; g.vertices.len()];
                for e in &g.edges {
                    d[e.a] += 1;
                    d[e.b] += 1;
                }
                if d[e.a] == 1 { e.a } else { e.b }
            };
            for &c in &g.vertices[tip_v] {
                s[c] = false;
            }
        }
        g = build_graph(&s, cols, rows);
    }
    // vertices of degree two join their two edges into one line
    let nv = g.vertices.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nv];
    for (k, e) in g.edges.iter().enumerate() {
        adj[e.a].push(k);
        if e.b != e.a {
            adj[e.b].push(k);
        }
    }
    let centroid = |v: usize| {
        let c = &g.vertices[v];
        let (mut x, mut y) = (0.0, 0.0);
        for &i in c {
            let q = xy_of(i);
            x += q[0];
            y += q[1];
        }
        [x / c.len() as f64, y / c.len() as f64]
    };
    let mut used = vec![false; g.edges.len()];
    let mut chains: Vec<(Vec<[f64; 2]>, Vec<usize>, Vec<usize>)> = Vec::new(); // points, cells, vertices
    let walk = |start_edge: usize, from: usize, used: &mut Vec<bool>| -> (Vec<[f64; 2]>, Vec<usize>, Vec<usize>) {
        let mut pts = vec![centroid(from)];
        let mut cells = Vec::new();
        let mut verts = vec![from];
        let (mut e, mut at) = (start_edge, from);
        loop {
            used[e] = true;
            let edge = &g.edges[e];
            let forward = edge.a == at;
            let seq: Vec<usize> = if forward { edge.cells.clone() } else { edge.cells.iter().rev().copied().collect() };
            for &c in &seq {
                pts.push(xy_of(c));
                cells.push(c);
            }
            let to = if forward { edge.b } else { edge.a };
            pts.push(centroid(to));
            verts.push(to);
            if adj[to].len() == 2 {
                if let Some(&nx) = adj[to].iter().find(|&&q| q != e && !used[q]) {
                    e = nx;
                    at = to;
                    continue;
                }
            }
            break;
        }
        (pts, cells, verts)
    };
    for v in 0..nv {
        if adj[v].len() != 2 {
            for &e in &adj[v].clone() {
                if !used[e] {
                    chains.push(walk(e, v, &mut used));
                }
            }
        }
    }
    for e in 0..g.edges.len() {
        if !used[e] {
            // a loop of degree-two vertices
            let a = g.edges[e].a;
            chains.push(walk(e, a, &mut used));
        }
    }
    for l in &g.loops {
        let mut pts: Vec<[f64; 2]> = l.iter().map(|&i| xy_of(i)).collect();
        pts.push(pts[0]);
        chains.push((pts, l.clone(), Vec::new()));
    }
    // networks (lines sharing vertices) shorter than the minimum road are dropped
    let mut comp: Vec<usize> = (0..nv).collect();
    fn root(c: &mut [usize], mut i: usize) -> usize {
        while c[i] != i {
            c[i] = c[c[i]];
            i = c[i];
        }
        i
    }
    for (_, _, vs) in &chains {
        for w in vs.windows(2) {
            let (a, b) = (root(&mut comp, w[0]), root(&mut comp, w[1]));
            comp[a.max(b)] = a.min(b);
        }
    }
    let line_len = |pts: &[[f64; 2]]| -> f64 { pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum() };
    let mut net_len = std::collections::HashMap::<usize, f64>::new();
    for (pts, _, vs) in &chains {
        let key = vs.first().map_or(usize::MAX, |&v| root(&mut comp, v));
        *net_len.entry(key).or_default() += line_len(pts);
    }
    let mut out = Vec::new();
    for (pts, cells, vs) in &chains {
        let key = vs.first().map_or(usize::MAX, |&v| root(&mut comp, v));
        let length = line_len(pts);
        if net_len[&key] < p.min_length || length <= 0.0 {
            continue;
        }
        let width = if cells.is_empty() {
            p.cell
        } else {
            cells.iter().map(|&i| f64::from(dist.data[i]) * 2.0 - 0.5 * p.cell).sum::<f64>() / cells.len() as f64
        };
        out.push(RoadLine {
            points: douglas_peucker(pts, p.simplify),
            width: width.max(p.cell),
            length,
        });
    }
    Ok(out)
}

/// Reads a cloud and extracts its roads: the ground must be classified (class 2).
pub fn extract_file(path: &std::path::Path, p: &RoadParams) -> Result<Vec<RoadLine>> {
    let mut pts = Vec::new();
    crate::las::Reader::open(path)?.scan(|q| pts.push(*q))?;
    if !pts.iter().any(|q| q.class == 2) {
        return Err(Error::Param(
            "la nube no tiene suelo clasificado (clase 2): clasifica el suelo antes".into(),
        ));
    }
    extract(&pts, p)
}

/// Writes the lines as a polyline shapefile with `WIDTH` and `LENGTH` fields (metres).
pub fn write_lines(lines: &[RoadLine], path: &std::path::Path, epsg: Option<u16>) -> Result<()> {
    use crate::vector::{ColKind, Table, Val, prj_for_epsg, write_polylines};
    let table = Table {
        columns: vec![
            ("WIDTH".into(), ColKind::Num(10, 2)),
            ("LENGTH".into(), ColKind::Num(12, 2)),
        ],
        rows: lines.iter().map(|l| vec![Val::Num(l.width), Val::Num(l.length)]).collect(),
    };
    let geom: Vec<Vec<[f64; 3]>> = lines
        .iter()
        .map(|l| l.points.iter().map(|p| [p[0], p[1], 0.0]).collect())
        .collect();
    crate::paths::ensure_parent(path)?;
    write_polylines(path, &geom, false, &table, epsg.and_then(prj_for_epsg).as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64, class: u8) -> Point {
        Point { x, y, z, class, intensity: 100, return_number: 1, number_of_returns: 1, ..Default::default() }
    }

    /// A 100 x 60 m scene with a horizontal road 6 m wide at y = 30 and canopy elsewhere.
    fn scene() -> Vec<Point> {
        let mut v = Vec::new();
        let mut k = 7u64;
        for i in 0..100 * 4 {
            for j in 0..60 * 4 {
                k = k.wrapping_mul(6364136223846793005).wrapping_add(1);
                let (x, y) = (f64::from(i) * 0.25 + 0.1, f64::from(j) * 0.25 + 0.1);
                if (y - 30.0).abs() <= 3.0 {
                    v.push(pt(x, y, 0.0, 2));
                } else if (k >> 40) % 10 == 0 {
                    v.push(pt(x, y, 0.0, 2));
                } else {
                    v.push(pt(x, y, 12.0, 5));
                }
            }
        }
        v
    }

    #[test]
    fn a_straight_road_is_found_with_its_width() -> Result<()> {
        let lines = extract(&scene(), &RoadParams::default())?;
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        assert!(l.length > 85.0, "{lines:?}");
        assert!((l.width - 6.0).abs() < 1.0, "{}", l.width);
        assert!(l.points.iter().all(|p| (p[1] - 30.0).abs() < 1.0), "{:?}", l.points);
        Ok(())
    }

    #[test]
    fn no_road_gives_no_lines() -> Result<()> {
        let pts: Vec<Point> = (0..2000).map(|i| pt(f64::from(i % 50), f64::from(i / 50), 10.0, 5)).collect();
        assert!(extract(&pts, &RoadParams::default())?.is_empty());
        Ok(())
    }
}
