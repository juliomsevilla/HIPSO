//! Grouping cells into objects.
//!
//! Two methods: connected cells whose value lies in a range ([`Method::Mask`]), and the
//! multiresolution merging of Baatz & Schäpe (2000) ([`Method::Merge`]). The second starts from
//! flat patches of the layers, builds the graph of neighbouring patches and merges, round after
//! round, every pair that is each other's best fit while the cost of merging (the growth of the
//! patches' heterogeneity in value and, if asked, in shape) stays under the scale. Merging only
//! mutual best pairs keeps a round parallel and the result independent of any visiting order.

use rayon::prelude::*;

use super::{Labels, Lattice, LayerStack};
use crate::{Error, Result, check_cancel};

#[derive(Clone, Debug, PartialEq)]
pub enum Method {
    /// The connected groups of cells whose value on `layer` is between `min` and `max`.
    Mask {
        layer: String,
        min: f32,
        max: f32,
        /// Cells touching at a corner belong together.
        diagonal: bool,
    },
    Merge(MergeParams),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MergeParams {
    /// Patches merge while the cost of doing so is under `scale` squared. The cost is measured in
    /// cells times standard deviations of the (standardised) layers, so a bigger scale gives
    /// bigger objects; start around 5 to 15.
    pub scale: f64,
    /// Weight of the shape (compactness and smoothness) against the value, 0 to 0.9.
    pub shape: f64,
    /// Inside the shape, the weight of compactness against smoothness, 0 to 1.
    pub compact: f64,
    /// A weight per layer (name, weight); the layers not named weigh 1 and a weight of 0 leaves a
    /// layer out.
    pub weights: Vec<(String, f32)>,
    /// Width, in standard deviations, of the value bins that decide which neighbouring cells
    /// start in the same patch: smaller is slower and more faithful.
    pub init: f64,
}

impl Default for MergeParams {
    fn default() -> Self {
        Self {
            scale: 8.0,
            shape: 0.1,
            compact: 0.5,
            weights: Vec::new(),
            init: 0.25,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SegmentParams {
    pub method: Method,
    /// Objects smaller than this are merged into their most similar neighbour (`Merge`) or
    /// dropped (`Mask`).
    pub min_cells: u32,
}

#[inline]
fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        let p = parent[x as usize];
        parent[x as usize] = parent[p as usize];
        x = parent[x as usize];
    }
    x
}

/// Joins the sets of `a` and `b`; the smaller root survives, so results do not depend on the
/// order of the unions.
#[inline]
fn union_min(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    match ra.cmp(&rb) {
        std::cmp::Ordering::Less => parent[rb as usize] = ra,
        std::cmp::Ordering::Greater => parent[ra as usize] = rb,
        std::cmp::Ordering::Equal => {}
    }
}

/// Connected components of the cells for which `valid` holds, two neighbours being joined when
/// `same` says so. Labels run from 1 in reading order of each component's first cell; 0 marks
/// the cells that are not valid. Returns the labels and how many components there are.
pub(crate) fn label_components(
    cols: usize,
    rows: usize,
    diagonal: bool,
    valid: impl Fn(usize) -> bool,
    same: impl Fn(usize, usize) -> bool,
) -> Result<(Vec<u32>, u32)> {
    let mut lab = vec![0u32; cols * rows];
    let mut parent: Vec<u32> = vec![0];
    for y in 0..rows {
        if y % 1024 == 0 {
            check_cancel()?;
        }
        for x in 0..cols {
            let i = y * cols + x;
            if !valid(i) {
                continue;
            }
            let mut cand = [0u32; 4];
            let mut m = 0;
            let mut look = |j: usize, lab: &[u32]| {
                if lab[j] != 0 && same(i, j) {
                    cand[m] = lab[j];
                    m += 1;
                }
            };
            if x > 0 {
                look(i - 1, &lab);
            }
            if y > 0 {
                look(i - cols, &lab);
                if diagonal {
                    if x > 0 {
                        look(i - cols - 1, &lab);
                    }
                    if x + 1 < cols {
                        look(i - cols + 1, &lab);
                    }
                }
            }
            if m == 0 {
                let l = u32::try_from(parent.len())
                    .map_err(|_| Error::Param("demasiados objetos iniciales".into()))?;
                parent.push(l);
                lab[i] = l;
            } else {
                for &c in &cand[1..m] {
                    union_min(&mut parent, cand[0], c);
                }
                lab[i] = find(&mut parent, cand[0]);
            }
        }
    }
    let mut dense = vec![0u32; parent.len()];
    let mut next = 0u32;
    for l in lab.iter_mut().filter(|l| **l != 0) {
        let r = find(&mut parent, *l) as usize;
        if dense[r] == 0 {
            next += 1;
            dense[r] = next;
        }
        *l = dense[r];
    }
    Ok((lab, next))
}

/// Renumbers the objects in reading order and turns those under `min_cells` into 0.
fn finish(mut ids: Vec<u32>, count: u32, min_cells: u32) -> (Vec<u32>, u32) {
    let mut size = vec![0u32; count as usize + 1];
    for &l in &ids {
        size[l as usize] += 1;
    }
    let mut map = vec![0u32; count as usize + 1];
    let mut next = 0u32;
    for l in ids.iter_mut().filter(|l| **l != 0) {
        let m = &mut map[*l as usize];
        if *m == 0 {
            if size[*l as usize] < min_cells.max(1) {
                *m = u32::MAX;
            } else {
                next += 1;
                *m = next;
            }
        }
        *l = if *m == u32::MAX { 0 } else { *m };
    }
    (ids, next)
}

/// Splits the cells of the stack into objects.
pub fn segment(stack: &LayerStack, p: &SegmentParams) -> Result<Labels> {
    let lat = stack.lat;
    let (ids, count) = match &p.method {
        Method::Mask {
            layer,
            min,
            max,
            diagonal,
        } => {
            let l = stack
                .index_of(layer)
                .ok_or_else(|| Error::Param(format!("no existe la capa '{layer}'")))?;
            let d = &stack.layers[l].data;
            let (lo, hi) = (*min, *max);
            let (lab, n) = label_components(
                lat.cols,
                lat.rows,
                *diagonal,
                |i| d[i] >= lo && d[i] <= hi,
                |_, _| true,
            )?;
            finish(lab, n, p.min_cells)
        }
        Method::Merge(m) => merge(stack, m, p.min_cells)?,
    };
    Ok(Labels { lat, ids, count })
}

/// A border of `len` cell edges between two patches, `a < b`.
#[derive(Clone, Copy)]
struct Edge {
    a: u32,
    b: u32,
    len: u32,
}

/// The patches and the borders between them.
struct Rag {
    /// Layers in use and their weights (summing to 1).
    k: usize,
    w: Vec<f64>,
    shape: f64,
    compact: f64,
    n: Vec<u32>,
    /// Per patch and layer: the sum of the values and of their squares.
    sum: Vec<f64>,
    sq: Vec<f64>,
    /// Edges of the patch's outline.
    per: Vec<u32>,
    /// First column, first row, last column, last row.
    bb: Vec<[u32; 4]>,
    edges: Vec<Edge>,
}

impl Rag {
    /// What merging patches `a` and `b`, sharing `shared` cell edges, would cost.
    fn cost(&self, a: usize, b: usize, shared: u32) -> f64 {
        let (na, nb) = (f64::from(self.n[a]), f64::from(self.n[b]));
        let nm = na + nb;
        let mut color = 0.0;
        for j in 0..self.k {
            let (sa, sb) = (self.sum[a * self.k + j], self.sum[b * self.k + j]);
            let (qa, qb) = (self.sq[a * self.k + j], self.sq[b * self.k + j]);
            // n times the standard deviation
            let hm = (nm * (qa + qb) - (sa + sb) * (sa + sb)).max(0.0).sqrt();
            let ha = (na * qa - sa * sa).max(0.0).sqrt();
            let hb = (nb * qb - sb * sb).max(0.0).sqrt();
            color += self.w[j] * (hm - ha - hb);
        }
        if self.shape <= 0.0 {
            return color;
        }
        let (la, lb) = (f64::from(self.per[a]), f64::from(self.per[b]));
        let lm = la + lb - 2.0 * f64::from(shared);
        let compact = lm * nm.sqrt() - la * na.sqrt() - lb * nb.sqrt();
        let outline = |bb: [u32; 4]| 2.0 * f64::from(bb[2] - bb[0] + 1 + bb[3] - bb[1] + 1);
        let (ba, bb) = (self.bb[a], self.bb[b]);
        let bm = [
            ba[0].min(bb[0]),
            ba[1].min(bb[1]),
            ba[2].max(bb[2]),
            ba[3].max(bb[3]),
        ];
        let smooth = nm * lm / outline(bm) - na * la / outline(ba) - nb * lb / outline(bb);
        (1.0 - self.shape) * color
            + self.shape * (self.compact * compact + (1.0 - self.compact) * smooth)
    }

    /// For every patch, the cost of merging it with its best neighbour and who that is (the
    /// lower number wins a tie).
    fn best(&self) -> (Vec<f64>, Vec<u32>) {
        let costs: Vec<f64> = self
            .edges
            .par_iter()
            .map(|e| self.cost(e.a as usize, e.b as usize, e.len))
            .collect();
        let mut cost = vec![f64::INFINITY; self.n.len()];
        let mut nb = vec![u32::MAX; self.n.len()];
        for (e, &c) in self.edges.iter().zip(&costs) {
            for (x, y) in [(e.a, e.b), (e.b, e.a)] {
                let x = x as usize;
                if c < cost[x] || (c == cost[x] && y < nb[x]) {
                    cost[x] = c;
                    nb[x] = y;
                }
            }
        }
        (cost, nb)
    }

    /// Merges the patches joined in `parent` (a union-find with the smallest member as root);
    /// returns where each old patch went.
    fn merge(&mut self, parent: &mut [u32]) -> Vec<u32> {
        let old = self.n.len();
        let mut new_id = vec![0u32; old];
        let mut remap = vec![0u32; old];
        let mut count = 0u32;
        for (r, id) in new_id.iter_mut().enumerate() {
            if find(parent, r as u32) as usize == r {
                *id = count;
                count += 1;
            }
        }
        for (r, m) in remap.iter_mut().enumerate() {
            *m = new_id[find(parent, r as u32) as usize];
        }
        let cnt = count as usize;
        let k = self.k;
        let mut n = vec![0u32; cnt];
        let mut sum = vec![0f64; cnt * k];
        let mut sq = vec![0f64; cnt * k];
        let mut per = vec![0u32; cnt];
        let mut bb = vec![[u32::MAX, u32::MAX, 0, 0]; cnt];
        for (r, &t) in remap.iter().enumerate() {
            let t = t as usize;
            n[t] += self.n[r];
            per[t] += self.per[r];
            for j in 0..k {
                sum[t * k + j] += self.sum[r * k + j];
                sq[t * k + j] += self.sq[r * k + j];
            }
            let (s, o) = (&mut bb[t], self.bb[r]);
            *s = [
                s[0].min(o[0]),
                s[1].min(o[1]),
                s[2].max(o[2]),
                s[3].max(o[3]),
            ];
        }
        let mut edges: Vec<Edge> = Vec::with_capacity(self.edges.len());
        for e in &self.edges {
            let (a, b) = (remap[e.a as usize], remap[e.b as usize]);
            if a == b {
                per[a as usize] = per[a as usize].saturating_sub(2 * e.len);
            } else {
                edges.push(Edge {
                    a: a.min(b),
                    b: a.max(b),
                    len: e.len,
                });
            }
        }
        edges.par_sort_unstable_by_key(|e| (u64::from(e.a) << 32) | u64::from(e.b));
        let mut joined: Vec<Edge> = Vec::with_capacity(edges.len());
        for e in edges {
            match joined.last_mut() {
                Some(l) if l.a == e.a && l.b == e.b => l.len += e.len,
                _ => joined.push(e),
            }
        }
        self.n = n;
        self.sum = sum;
        self.sq = sq;
        self.per = per;
        self.bb = bb;
        self.edges = joined;
        remap
    }
}

fn merge(stack: &LayerStack, m: &MergeParams, min_cells: u32) -> Result<(Vec<u32>, u32)> {
    let Lattice { cols, rows, .. } = stack.lat;
    if cols > u32::MAX as usize / 2 || rows > u32::MAX as usize / 2 {
        return Err(Error::Param("la rejilla es demasiado grande".into()));
    }
    // layers in use and their weights
    let mut sel: Vec<usize> = Vec::new();
    let mut w: Vec<f64> = Vec::new();
    for (i, l) in stack.layers.iter().enumerate() {
        let weight = m
            .weights
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&l.name))
            .map_or(1.0, |(_, v)| f64::from(*v));
        if weight > 0.0 {
            sel.push(i);
            w.push(weight);
        }
    }
    for (name, _) in &m.weights {
        if stack.index_of(name).is_none() {
            return Err(Error::Param(format!("no existe la capa '{name}'")));
        }
    }
    if sel.is_empty() {
        return Err(Error::Param("hace falta al menos una capa con peso".into()));
    }
    if !(m.scale > 0.0) || !(m.init > 0.0) {
        return Err(Error::Param(
            "la escala y el paso inicial deben ser positivos".into(),
        ));
    }
    let total: f64 = w.iter().sum();
    for v in &mut w {
        *v /= total;
    }
    let k = sel.len();
    let data: Vec<&[f32]> = sel
        .iter()
        .map(|&i| stack.layers[i].data.as_slice())
        .collect();
    // standardise: mean and standard deviation of each layer over its valid cells
    let (mean, inv): (Vec<f64>, Vec<f64>) = data
        .iter()
        .map(|d| {
            let (mut s, mut q, mut c) = (0f64, 0f64, 0u64);
            let shift = d
                .iter()
                .find(|v| v.is_finite())
                .copied()
                .map_or(0.0, f64::from);
            for &v in d.iter().filter(|v| v.is_finite()) {
                let z = f64::from(v) - shift;
                s += z;
                q += z * z;
                c += 1;
            }
            if c == 0 {
                return (0.0, 1.0);
            }
            let mu = s / c as f64;
            let sd = (q / c as f64 - mu * mu).max(0.0).sqrt();
            (shift + mu, if sd > 1e-12 { 1.0 / sd } else { 1.0 })
        })
        .unzip();
    let z = |j: usize, i: usize| (f64::from(data[j][i]) - mean[j]) * inv[j];
    let valid = |i: usize| data.iter().all(|d| d[i].is_finite());
    // patches of cells in the same value bins
    let step: Vec<f64> = inv.iter().map(|v| v / m.init).collect();
    let bin = |j: usize, i: usize| ((f64::from(data[j][i]) - mean[j]) * step[j]).floor() as i64;
    let (lab, patches) = label_components(cols, rows, false, valid, |i, j| {
        (0..k).all(|l| bin(l, i) == bin(l, j))
    })?;
    if patches == 0 {
        return Ok((lab, 0));
    }
    let r0 = patches as usize;
    // the patches' statistics and borders, in one pass
    let mut rag = Rag {
        k,
        w,
        shape: m.shape.clamp(0.0, 0.9),
        compact: m.compact.clamp(0.0, 1.0),
        n: vec![0; r0],
        sum: vec![0.0; r0 * k],
        sq: vec![0.0; r0 * k],
        per: vec![0; r0],
        bb: vec![[u32::MAX, u32::MAX, 0, 0]; r0],
        edges: Vec::new(),
    };
    let mut keys: Vec<u64> = Vec::new();
    for y in 0..rows {
        if y % 1024 == 0 {
            check_cancel()?;
        }
        for x in 0..cols {
            let i = y * cols + x;
            let l = lab[i];
            if l == 0 {
                continue;
            }
            let r = (l - 1) as usize;
            rag.n[r] += 1;
            for j in 0..k {
                let v = z(j, i);
                rag.sum[r * k + j] += v;
                rag.sq[r * k + j] += v * v;
            }
            let bb = &mut rag.bb[r];
            *bb = [
                bb[0].min(x as u32),
                bb[1].min(y as u32),
                bb[2].max(x as u32),
                bb[3].max(y as u32),
            ];
            let mut outline = 0;
            let mut side = |j: Option<usize>, border: bool| match j.map(|j| lab[j]) {
                Some(o) if o == l => {}
                Some(o) if o != 0 => {
                    outline += 1;
                    if border {
                        let (a, b) = (l.min(o) - 1, l.max(o) - 1);
                        keys.push((u64::from(a) << 32) | u64::from(b));
                    }
                }
                _ => outline += 1,
            };
            // right and down count each border once; left and up only add to the outline
            side((x + 1 < cols).then_some(i + 1), true);
            side((y + 1 < rows).then_some(i + cols), true);
            side((x > 0).then(|| i - 1), false);
            side((y > 0).then(|| i - cols), false);
            rag.per[r] += outline;
        }
    }
    keys.par_sort_unstable();
    for key in keys {
        let (a, b) = ((key >> 32) as u32, key as u32);
        match rag.edges.last_mut() {
            Some(e) if e.a == a && e.b == b => e.len += 1,
            _ => rag.edges.push(Edge { a, b, len: 1 }),
        }
    }
    // where each initial patch is now
    let mut cur: Vec<u32> = (0..r0 as u32).collect();
    let limit = m.scale * m.scale;
    loop {
        check_cancel()?;
        let (cost, nb) = rag.best();
        let mut parent: Vec<u32> = (0..rag.n.len() as u32).collect();
        let mut merges = 0usize;
        for e in &rag.edges {
            let (a, b) = (e.a as usize, e.b as usize);
            if nb[a] == e.b && nb[b] == e.a && cost[a] < limit {
                union_min(&mut parent, e.a, e.b);
                merges += 1;
            }
        }
        if merges == 0 {
            break;
        }
        let remap = rag.merge(&mut parent);
        cur.par_iter_mut().for_each(|c| *c = remap[*c as usize]);
    }
    // patches under the minimum size join their most similar neighbour
    if min_cells > 1 {
        loop {
            check_cancel()?;
            let (_, nb) = rag.best();
            let mut parent: Vec<u32> = (0..rag.n.len() as u32).collect();
            let mut merges = 0usize;
            for (a, &b) in nb.iter().enumerate() {
                if rag.n[a] < min_cells && b != u32::MAX {
                    union_min(&mut parent, a as u32, b);
                    merges += 1;
                }
            }
            if merges == 0 {
                break;
            }
            let remap = rag.merge(&mut parent);
            cur.par_iter_mut().for_each(|c| *c = remap[*c as usize]);
        }
    }
    let groups = rag.n.len();
    let ids: Vec<u32> = lab
        .par_iter()
        .map(|&l| if l == 0 { 0 } else { cur[(l - 1) as usize] + 1 })
        .collect();
    Ok(finish(ids, groups as u32, min_cells))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obia::Lattice;

    fn stack(cols: usize, rows: usize, f: impl Fn(usize, usize) -> f32) -> LayerStack {
        let lat = Lattice {
            x0: 0.0,
            y0: 0.0,
            cell: 1.0,
            cols,
            rows,
        };
        let mut s = LayerStack::new(lat, None);
        let data = (0..cols * rows).map(|i| f(i % cols, i / cols)).collect();
        assert!(s.push("v", data).is_ok());
        s
    }

    #[test]
    fn a_mask_gives_its_connected_groups() -> Result<()> {
        // two blocks of 3 x 3 apart, a single cell touching the first at a corner
        let s = stack(12, 6, |x, y| {
            let block = (x < 3 && y < 3) || (x >= 8 && y < 3);
            let corner = x == 3 && y == 3;
            if block || corner { 1.0 } else { 0.0 }
        });
        let run = |diagonal, min_cells| {
            segment(
                &s,
                &SegmentParams {
                    method: Method::Mask {
                        layer: "v".into(),
                        min: 0.5,
                        max: 2.0,
                        diagonal,
                    },
                    min_cells,
                },
            )
        };
        assert_eq!(run(false, 1)?.count, 3);
        assert_eq!(run(true, 1)?.count, 2);
        // small groups are dropped and the rest numbered again
        let l = run(false, 2)?;
        assert_eq!(l.count, 2);
        assert_eq!(l.ids[3 * 12 + 3], 0);
        assert_eq!(l.ids[0], 1);
        assert_eq!(l.ids[8], 2);
        Ok(())
    }

    /// A noisy step: two plateaus 10 apart.
    fn plateaus() -> LayerStack {
        stack(40, 30, |x, y| {
            let noise = (((x * 31 + y * 17) % 7) as f32 - 3.0) * 0.05;
            (if x < 20 { 2.0 } else { 12.0 }) + noise
        })
    }

    #[test]
    fn merging_finds_the_plateaus() -> Result<()> {
        let p = |scale| SegmentParams {
            method: Method::Merge(MergeParams {
                scale,
                ..Default::default()
            }),
            min_cells: 4,
        };
        let l = segment(&plateaus(), &p(8.0))?;
        assert_eq!(l.count, 2);
        let (a, b) = (l.ids[0], l.ids[39]);
        assert!(a != 0 && b != 0 && a != b);
        for (i, &id) in l.ids.iter().enumerate() {
            assert_eq!(id, if i % 40 < 20 { a } else { b });
        }
        // a larger scale than the step is worth merges everything
        let all = segment(&plateaus(), &p(1000.0))?;
        assert_eq!(all.count, 1);
        // the result does not depend on the thread schedule
        assert_eq!(segment(&plateaus(), &p(8.0))?.ids, l.ids);
        Ok(())
    }

    #[test]
    fn merging_leaves_empty_cells_out() -> Result<()> {
        let s = stack(
            10,
            10,
            |x, _| if x == 5 { f32::NAN } else { x as f32 * 10.0 },
        );
        let l = segment(
            &s,
            &SegmentParams {
                method: Method::Merge(MergeParams::default()),
                min_cells: 1,
            },
        )?;
        assert!(
            l.ids
                .iter()
                .enumerate()
                .all(|(i, &v)| (v == 0) == (i % 10 == 5))
        );
        assert!(l.count >= 2);
        Ok(())
    }

    #[test]
    fn unknown_layers_are_reported() {
        let r = segment(
            &plateaus(),
            &SegmentParams {
                method: Method::Mask {
                    layer: "nada".into(),
                    min: 0.0,
                    max: 1.0,
                    diagonal: false,
                },
                min_cells: 1,
            },
        );
        assert!(matches!(r, Err(Error::Param(_))));
    }
}
