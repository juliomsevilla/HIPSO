//! What the tools that group things share: the union-find that merges them, the chaining of
//! points that lie within a distance of each other, and the median.

use std::collections::HashMap;

/// Union-find over `n` items (indices kept as `u32`: it is used on rasters of hundreds of millions
/// of cells). The root of a set is its smallest index, so the groups come out the same whatever
/// the order the pairs were joined in.
pub struct Dsu(Vec<u32>);

impl Dsu {
    pub fn new(n: usize) -> Self {
        assert!(u32::try_from(n).is_ok(), "too many items for a union-find");
        Self((0..n as u32).collect())
    }

    /// A new item, alone in its set; returns its index.
    pub fn push(&mut self) -> usize {
        let i = u32::try_from(self.0.len()).expect("too many items for a union-find");
        self.0.push(i);
        i as usize
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn find(&mut self, i: usize) -> usize {
        let mut x = i as u32;
        while self.0[x as usize] != x {
            let p = self.0[x as usize];
            self.0[x as usize] = self.0[p as usize];
            x = self.0[x as usize];
        }
        x as usize
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b) as u32;
        }
    }
}

/// The group of each of `n` points (given by `at`): two points within `link` of each other are in
/// the same one, and so is everything chained to them. `zscale` weighs heights against distances
/// on the plan (1: the distance in space; 0: on the plan only; between, two points that are
/// `link / zscale` apart vertically are still linked). A group is named by its first point.
pub fn link_labels(
    n: usize,
    at: impl Fn(usize) -> [f64; 3] + Sync,
    link: f64,
    zscale: f64,
) -> Vec<usize> {
    let key = |p: &[f64; 3]| -> (i64, i64, i64) {
        (
            (p[0] / link).floor() as i64,
            (p[1] / link).floor() as i64,
            (p[2] * zscale / link).floor() as i64,
        )
    };
    let mut cells: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for j in 0..n {
        cells.entry(key(&at(j))).or_default().push(j);
    }
    let dz = i64::from(zscale > 0.0);
    let l2 = link * link;
    let mut dsu = Dsu::new(n);
    for j in 0..n {
        let q = at(j);
        let (cx, cy, cz) = key(&q);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for ddz in -dz..=dz {
                    for &o in cells
                        .get(&(cx + dx, cy + dy, cz + ddz))
                        .into_iter()
                        .flatten()
                    {
                        if o > j {
                            let w = at(o);
                            let d2 = (w[0] - q[0]).powi(2)
                                + (w[1] - q[1]).powi(2)
                                + ((w[2] - q[2]) * zscale).powi(2);
                            if d2 <= l2 {
                                dsu.union(j, o);
                            }
                        }
                    }
                }
            }
        }
    }
    (0..n).map(|j| dsu.find(j)).collect()
}

/// The groups of `labels` as lists of indices, ordered by their first point.
pub fn groups(labels: &[usize]) -> Vec<Vec<usize>> {
    let mut by: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &g) in labels.iter().enumerate() {
        by.entry(g).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = by.into_values().collect();
    out.sort_by_key(|g| g[0]);
    out
}

/// The median of `v` (the upper middle one with an even count); reorders `v`.
pub fn median(v: &mut [f64]) -> f64 {
    let m = v.len() / 2;
    v.select_nth_unstable_by(m, |a, b| a.total_cmp(b));
    v[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chained_points_share_a_group_and_a_far_one_does_not() {
        let p = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 0.0, 5.0],
        ];
        let at = |i: usize| p[i];
        let l = link_labels(5, at, 1.2, 1.0);
        assert_eq!(l[0], l[2]);
        assert_ne!(l[0], l[3]);
        assert_ne!(l[0], l[4], "5 m above is not linked in space");
        // on the plan only, the point 5 m above joins; with heights weighed down, too
        assert_eq!(link_labels(5, at, 1.2, 0.0)[4], l[0]);
        assert_eq!(link_labels(5, at, 1.2, 0.2)[4], l[0]);
        assert_eq!(groups(&l).len(), 3);
    }

    #[test]
    fn the_median_of_odd_and_even_counts() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 3.0);
    }
}
