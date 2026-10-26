//! A static R-tree of boxes (min x, min y, max x, max y), packed with the Sort-Tile-Recursive
//! method of Leutenegger et al. (1997): built in O(n log n), queried in O(log n + hits). It
//! answers which features of a layer lie inside a window without looking at all of them.

const FANOUT: usize = 16;

#[derive(Clone, Copy)]
struct Node {
    b: [f64; 4],
    /// First child: an index into the level below (or into `ids` for the lowest level).
    first: u32,
    n: u32,
}

#[derive(Default)]
pub struct RTree {
    /// Level 0 holds the leaves (ranges of `ids`); the last level has one node, the root.
    levels: Vec<Vec<Node>>,
    ids: Vec<u32>,
    boxes: Vec<[f64; 4]>,
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn meets(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[2] >= b[0] && a[0] <= b[2] && a[3] >= b[1] && a[1] <= b[3]
}

/// Puts the items in the order that makes runs of `FANOUT` of them close together: sorted by x
/// into vertical slabs, each slab sorted by y.
fn pack(items: &mut [(u32, [f64; 4])]) {
    let n = items.len();
    let leaves = n.div_ceil(FANOUT);
    let slabs = (leaves as f64).sqrt().ceil().max(1.0) as usize;
    let slab = slabs * FANOUT;
    items.sort_unstable_by(|a, b| (a.1[0] + a.1[2]).total_cmp(&(b.1[0] + b.1[2])));
    for s in items.chunks_mut(slab) {
        s.sort_unstable_by(|a, b| (a.1[1] + a.1[3]).total_cmp(&(b.1[1] + b.1[3])));
    }
}

impl RTree {
    /// Indexes the boxes; one with a value that is not finite is left out.
    pub fn new(boxes: &[[f64; 4]]) -> Self {
        let mut items: Vec<(u32, [f64; 4])> = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.iter().all(|v| v.is_finite()))
            .map(|(i, b)| (i as u32, *b))
            .collect();
        if items.is_empty() {
            return Self::default();
        }
        pack(&mut items);
        let ids: Vec<u32> = items.iter().map(|i| i.0).collect();
        let mut level: Vec<Node> = items
            .chunks(FANOUT)
            .enumerate()
            .map(|(k, c)| Node {
                b: c.iter().fold(c[0].1, |a, i| union(a, i.1)),
                first: (k * FANOUT) as u32,
                n: c.len() as u32,
            })
            .collect();
        let mut levels = Vec::new();
        while level.len() > 1 {
            let mut keyed: Vec<(u32, [f64; 4])> = level
                .iter()
                .enumerate()
                .map(|(k, n)| (k as u32, n.b))
                .collect();
            pack(&mut keyed);
            // the children of a parent are neighbours in the level below
            let ordered: Vec<Node> = keyed.iter().map(|k| level[k.0 as usize]).collect();
            let parents: Vec<Node> = ordered
                .chunks(FANOUT)
                .enumerate()
                .map(|(k, c)| Node {
                    b: c.iter().fold(c[0].b, |a, n| union(a, n.b)),
                    first: (k * FANOUT) as u32,
                    n: c.len() as u32,
                })
                .collect();
            levels.push(ordered);
            level = parents;
        }
        levels.push(level);
        Self {
            levels,
            ids,
            boxes: boxes.to_vec(),
        }
    }

    /// The indices of the boxes that meet the window, in ascending order.
    pub fn query(&self, w: [f64; 4]) -> Vec<usize> {
        let mut out = Vec::new();
        let Some(top) = self.levels.len().checked_sub(1) else {
            return out;
        };
        let mut stack = vec![(top, 0usize)];
        while let Some((l, k)) = stack.pop() {
            let node = self.levels[l][k];
            if !meets(&node.b, &w) {
                continue;
            }
            let range = node.first as usize..(node.first + node.n) as usize;
            if l == 0 {
                out.extend(
                    self.ids[range]
                        .iter()
                        .filter(|&&i| meets(&self.boxes[i as usize], &w))
                        .map(|&i| i as usize),
                );
            } else {
                stack.extend(range.map(|c| (l - 1, c)));
            }
        }
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_finds_what_a_scan_finds() {
        // boxes of varied size on a pseudo-random layout
        let mut s = 12345u64;
        let mut rnd = move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (s >> 33) as f64 / (1u64 << 31) as f64
        };
        let boxes: Vec<[f64; 4]> = (0..5000)
            .map(|_| {
                let (x, y, w, h) = (rnd() * 1000.0, rnd() * 1000.0, rnd() * 20.0, rnd() * 20.0);
                [x, y, x + w, y + h]
            })
            .collect();
        let t = RTree::new(&boxes);
        for _ in 0..50 {
            let (x, y) = (rnd() * 1000.0, rnd() * 1000.0);
            let w = [x, y, x + rnd() * 200.0, y + rnd() * 200.0];
            let want: Vec<usize> = (0..boxes.len()).filter(|&i| meets(&boxes[i], &w)).collect();
            assert_eq!(t.query(w), want);
        }
    }

    #[test]
    fn empty_and_unbounded_boxes_are_harmless() {
        assert!(RTree::new(&[]).query([0.0, 0.0, 1.0, 1.0]).is_empty());
        let t = RTree::new(&[[f64::INFINITY, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0]]);
        assert_eq!(t.query([0.5, 0.5, 2.0, 2.0]), [1]);
    }
}
