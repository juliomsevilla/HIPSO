//! Stems that grow together. In a coppice stand (eucalyptus that sprout from the stump) several
//! stems come out of one stool; some trees fork higher up. They are told apart at the height of
//! the diameter (1.3 m): stems that are apart at that height are the sprouts of one stool, and
//! each is a tree with its own diameter; stems that are one below it and fork above it are one
//! forked tree, with the diameter of its stem and the volume of the stem and of every branch.

use std::collections::{BTreeSet, HashMap};

use crate::tls::{Cubing, Log, Section, TlsParams, TreeResult, smalian};

/// Union-find over the trees.
struct Dsu(Vec<usize>);

impl Dsu {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[b.max(a)] = b.min(a);
        }
    }
}

/// Two sections are the same circle: centres closer than 40 % of the wider diameter, and
/// diameters within 35 % of it.
fn same_circle(a: &Section, b: &Section) -> bool {
    let wide = a.d.max(b.d);
    (a.x - b.x).hypot(a.y - b.y) < 0.4 * wide && (a.d - b.d).abs() < 0.35 * wide
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Relation {
    /// Two stems, apart at the height of the diameter.
    Apart,
    /// One stem at the height of the diameter that forks above it, at this height.
    Fork(f64),
    /// The same stem found twice.
    Twin,
}

/// How two trees of a stool relate, from the sections both have.
fn relation(a: &TreeResult, b: &TreeResult, p: &TlsParams) -> Relation {
    // the heights where both have a valid section, and whether it is the same circle
    let common: Vec<(f64, bool)> = a
        .sections
        .iter()
        .filter(|s| s.ok)
        .filter_map(|sa| {
            b.sections
                .iter()
                .find(|s| s.ok && (s.h - sa.h).abs() < 0.01)
                .map(|sb| (sa.h, same_circle(sa, sb)))
        })
        .collect();
    // the pair nearest to the height of the diameter says which they are (a stem is often
    // missing a section right there); with no section in common, the bases tell: two stems
    // cannot grow in the same place
    let away = |h: f64| (h - p.fork_height).abs();
    let Some(&(h_ref, same)) = common.iter().min_by(|x, y| away(x.0).total_cmp(&away(y.0))) else {
        return if (a.x - b.x).hypot(a.y - b.y) < 0.05 {
            Relation::Twin
        } else {
            Relation::Apart
        };
    };
    if !same {
        return Relation::Apart;
    }
    // one stem at the height of the diameter: a fork if they part above, else a twin
    match common
        .iter()
        .filter(|(h, same)| *h > h_ref && !*same)
        .map(|(h, _)| *h)
        .min_by(f64::total_cmp)
    {
        Some(h_apart) => {
            let h_same = common
                .iter()
                .filter(|(h, same)| *same && *h < h_apart)
                .map(|(h, _)| *h)
                .fold(h_ref, f64::max);
            let fork = (h_same + h_apart) / 2.0;
            // a fork under the height of the diameter is a stool of sprouts
            if fork < p.fork_height {
                Relation::Apart
            } else {
                Relation::Fork(fork)
            }
        }
        None => Relation::Twin,
    }
}

fn valid_sections(t: &TreeResult) -> usize {
    t.sections.iter().filter(|s| s.ok).count()
}

/// The profile (height, diameter) of the valid or filled sections that `keep` accepts.
fn profile(secs: &[Section], keep: &dyn Fn(f64) -> bool) -> Vec<(f64, f64)> {
    secs.iter()
        .filter(|s| (s.ok || s.d.is_finite()) && keep(s.h))
        .map(|s| (s.h, s.d))
        .collect()
}

/// One tree from the stems of a fork: the stem under the fork is cubed once, and every branch
/// above it from the fork up.
fn merge_fork(mut members: Vec<TreeResult>, fork_h: f64, p: &TlsParams) -> TreeResult {
    // the stem below the fork: the member with most valid sections under it
    let under = |t: &TreeResult| t.sections.iter().filter(|s| s.ok && s.h <= fork_h).count();
    let first = (0..members.len())
        .max_by_key(|&i| under(&members[i]))
        .unwrap_or(0);
    let mut tree = members[first].clone();
    let stem = smalian(
        &profile(&tree.sections, &|h| h <= fork_h),
        p.base_height,
        p.log_len,
        p.top_diam,
    );
    let mut logs = stem.logs;
    let (mut top_h, mut truncated, mut branches) = (stem.top_h, true, 0);
    let mut sections: Vec<Section> = tree
        .sections
        .iter()
        .filter(|s| s.h <= fork_h)
        .cloned()
        .collect();
    let mut flags: BTreeSet<String> = BTreeSet::new();
    let mut quality = 0.0;
    let mut points = 0;
    let mut height = 0.0f64;
    for m in &members {
        // every member goes on above the fork as a branch of its own
        let above = smalian(
            &profile(&m.sections, &|h| h >= fork_h),
            fork_h,
            p.log_len,
            p.top_diam,
        );
        branches += 1;
        logs.extend(above.logs.into_iter().map(|l| Log {
            branch: branches,
            ..l
        }));
        top_h = top_h.max(above.top_h);
        truncated &= above.truncated;
        sections.extend(m.sections.iter().filter(|s| s.h > fork_h).cloned());
        flags.extend(m.flags.split_whitespace().map(str::to_string));
        quality += m.quality;
        points += m.points;
        height = height.max(m.height);
    }
    members.clear();
    sections.sort_by(|a, b| a.h.total_cmp(&b.h));
    let volume = logs.iter().map(|l| l.volume).sum();
    // what depends on the cubing is worked out again for the whole tree
    for stale in ["SIN_DAP", "TRUNCADO", "SIN_VOLUMEN"] {
        flags.remove(stale);
    }
    if tree.dbh.is_none() {
        flags.insert("SIN_DAP".into());
    }
    if truncated {
        flags.insert("TRUNCADO".into());
    }
    if logs.is_empty() {
        flags.insert("SIN_VOLUMEN".into());
    }
    flags.insert("BIFURCADO".into());
    tree.cubing = Cubing {
        logs,
        volume,
        top_h,
        truncated,
    };
    tree.sections = sections;
    tree.points = points;
    tree.quality = quality / branches.max(1) as f64;
    tree.height = height;
    tree.fork_h = Some(fork_h);
    tree.flags = flags.into_iter().collect::<Vec<_>>().join(" ");
    tree
}

/// Tells the sprouts of a stool from the branches of a fork, from the trees measured one by one:
/// twins are dropped, forks are merged into one tree, and the stems of each stool are numbered
/// together (`stool`, `sprouts`; the flag `BROTES` marks a stool with several).
pub fn resolve(trees: Vec<TreeResult>, p: &TlsParams) -> Vec<TreeResult> {
    let n = trees.len();
    // stems that grow on one stool: bases closer than the distance between stools
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| trees[a].x.total_cmp(&trees[b].x));
    let mut stool = Dsu::new(n);
    for (k, &i) in order.iter().enumerate() {
        for &j in &order[k + 1..] {
            if trees[j].x - trees[i].x > p.stool_dist {
                break;
            }
            if (trees[i].x - trees[j].x).hypot(trees[i].y - trees[j].y) < p.stool_dist {
                stool.union(i, j);
            }
        }
    }
    let mut dropped = vec![false; n];
    let mut fork = Dsu::new(n);
    let mut fork_at: HashMap<(usize, usize), f64> = HashMap::new();
    for i in 0..n {
        for j in i + 1..n {
            if dropped[i] || dropped[j] || stool.find(i) != stool.find(j) {
                continue;
            }
            let rel = relation(&trees[i], &trees[j], p);
            match rel {
                Relation::Apart => {}
                Relation::Twin => {
                    // the stem found twice: the one measured less stays out
                    let weaker = if valid_sections(&trees[i]) < valid_sections(&trees[j]) {
                        i
                    } else {
                        j
                    };
                    dropped[weaker] = true;
                }
                Relation::Fork(h) => {
                    fork.union(i, j);
                    fork_at.insert((i, j), h);
                }
            }
        }
    }
    // the members of each fork, and its height: the lowest the pairs of it part at
    let mut sets: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, gone) in dropped.iter().enumerate() {
        if !*gone {
            sets.entry(fork.find(i)).or_default().push(i);
        }
    }
    let mut out: Vec<(usize, TreeResult)> = Vec::new();
    let mut slots: Vec<Option<TreeResult>> = trees.into_iter().map(Some).collect();
    for i in 0..n {
        if dropped[i] {
            continue;
        }
        let root = fork.find(i);
        let members = &sets[&root];
        if members.len() < 2 {
            if let Some(t) = slots[i].take() {
                out.push((stool.find(i), t));
            }
            continue;
        }
        if members[0] != i {
            continue;
        }
        let h = fork_at
            .iter()
            .filter(|((a, b), _)| members.contains(a) && members.contains(b))
            .map(|(_, h)| *h)
            .fold(f64::INFINITY, f64::min);
        let parts: Vec<TreeResult> = members.iter().filter_map(|&m| slots[m].take()).collect();
        out.push((stool.find(i), merge_fork(parts, h, p)));
    }
    // how many trees each stool ended with
    let mut count: HashMap<usize, u32> = HashMap::new();
    for (s, _) in &out {
        *count.entry(*s).or_default() += 1;
    }
    out.into_iter()
        .map(|(s, mut t)| {
            t.stool = s as u32;
            t.sprouts = count[&s];
            if t.sprouts > 1 {
                let mut flags: BTreeSet<&str> = t.flags.split_whitespace().collect();
                flags.insert("BROTES");
                t.flags = flags.into_iter().collect::<Vec<_>>().join(" ");
            }
            t
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::diam_at;

    /// A tree from `(height, x, y, diameter)` sections, all valid.
    fn tree(x: f64, y: f64, secs: &[(f64, f64, f64, f64)]) -> TreeResult {
        let p = TlsParams::default();
        let sections: Vec<Section> = secs
            .iter()
            .map(|&(h, x, y, d)| Section {
                h,
                x,
                y,
                d,
                ok: true,
                points: 40,
                sectors: 12,
            })
            .collect();
        let prof: Vec<(f64, f64)> = sections.iter().map(|s| (s.h, s.d)).collect();
        TreeResult {
            id: 0,
            x,
            y,
            ground_z: 0.0,
            dbh: diam_at(&prof, 1.3),
            height: sections.last().map_or(0.0, |s| s.h),
            tilt: 0.0,
            cubing: smalian(&prof, p.base_height, p.log_len, p.top_diam),
            sections,
            points: 400,
            quality: 80.0,
            d_base: 0.3,
            flags: "TRUNCADO".into(),
            slope: [0.0, 0.0],
            stool: 0,
            sprouts: 1,
            fork_h: None,
        }
    }

    /// Sections every 0.25 m from `from` to `to` along a vertical axis.
    fn column(from: f64, to: f64, x: f64, y: f64, d: f64) -> Vec<(f64, f64, f64, f64)> {
        let n = ((to - from) / 0.25).round() as usize;
        (0..=n).map(|k| (from + 0.25 * k as f64, x, y, d)).collect()
    }

    #[test]
    fn sprouts_that_are_apart_at_the_height_of_the_diameter_are_trees_of_one_stool() {
        let p = TlsParams::default();
        let a = tree(0.0, 0.0, &column(0.1, 4.0, 0.0, 0.0, 0.20));
        let b = tree(0.4, 0.0, &column(0.1, 4.0, 0.4, 0.0, 0.22));
        let far = tree(5.0, 0.0, &column(0.1, 4.0, 5.0, 0.0, 0.25));
        let out = resolve(vec![a, b, far], &p);
        assert_eq!(out.len(), 3);
        let stool_of_a = out[0].stool;
        assert_eq!((out[0].sprouts, out[1].sprouts, out[2].sprouts), (2, 2, 1));
        assert_eq!(out[1].stool, stool_of_a);
        assert_ne!(out[2].stool, stool_of_a);
        assert!(out[0].flags.contains("BROTES") && !out[2].flags.contains("BROTES"));
        assert!(out.iter().all(|t| t.fork_h.is_none()));
    }

    /// A trunk of 30 cm up to 2 m that forks into two branches of 20 cm.
    fn forked_at(fork: f64) -> (TreeResult, TreeResult) {
        let branch = |side: f64| {
            let mut s = column(0.1, fork, 0.0, 0.0, 0.30);
            // above the fork the branches leave the axis at 20 cm per 25 cm of height
            for k in 1..=8 {
                let h = fork + 0.25 * k as f64;
                s.push((h, side * 0.2 * k as f64, 0.0, 0.20));
            }
            s
        };
        (tree(0.0, 0.0, &branch(1.0)), tree(0.0, 0.0, &branch(-1.0)))
    }

    #[test]
    fn a_fork_above_the_height_of_the_diameter_is_one_tree() {
        let p = TlsParams::default();
        let (a, b) = forked_at(2.0);
        let out = resolve(vec![a, b], &p);
        assert_eq!(out.len(), 1);
        let t = &out[0];
        assert!(t.flags.contains("BIFURCADO") && !t.flags.contains("BROTES"));
        // the diameter is that of the stem under the fork
        assert!(t.dbh.is_some_and(|d| (d - 0.30).abs() < 1e-9));
        assert!(t.fork_h.is_some_and(|h| (2.0..2.3).contains(&h)));
        // the stem once, and a branch each: logs of branch 0, 1 and 2
        let ids: BTreeSet<u8> = t.cubing.logs.iter().map(|l| l.branch).collect();
        assert_eq!(ids, BTreeSet::from([0, 1, 2]));
        let volume: f64 = t.cubing.logs.iter().map(|l| l.volume).sum();
        assert!((t.cubing.volume - volume).abs() < 1e-12);
        // more than the stem alone, and the stem is not counted twice
        let one = forked_at(2.0).0.cubing.volume;
        assert!(t.cubing.volume > one && t.cubing.volume < 2.0 * one);
    }

    #[test]
    fn a_fork_below_the_height_of_the_diameter_is_a_stool_of_sprouts() {
        let p = TlsParams::default();
        let (a, b) = forked_at(0.9);
        let out = resolve(vec![a, b], &p);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|t| t.sprouts == 2 && t.fork_h.is_none()));
        assert!(out.iter().all(|t| t.flags.contains("BROTES")));
    }

    #[test]
    fn a_twin_with_no_section_near_the_height_of_the_diameter_is_still_found() {
        let p = TlsParams::default();
        // sections of one stem found twice, each missing the ones the other has, and none
        // between 0.85 and 1.9 m in common
        let a = tree(
            0.0,
            0.0,
            &[
                (0.35, 0.0, 0.0, 0.15),
                (0.85, 0.0, 0.0, 0.16),
                (1.9, 0.0, 0.0, 0.14),
                (2.1, 0.0, 0.0, 0.14),
            ],
        );
        let b = tree(
            0.0,
            0.0,
            &[
                (0.35, 0.0, 0.0, 0.15),
                (0.6, 0.0, 0.0, 0.15),
                (0.85, 0.0, 0.0, 0.16),
                (1.4, 0.0, 0.0, 0.14),
            ],
        );
        assert_eq!(resolve(vec![a, b], &p).len(), 1);
        // with nothing in common at all, only the same place gives them away
        let c = tree(0.0, 0.0, &[(0.35, 0.0, 0.0, 0.15)]);
        let d = tree(0.01, 0.0, &[(0.6, 0.0, 0.0, 0.15)]);
        let e = tree(0.4, 0.0, &[(0.6, 0.4, 0.0, 0.15)]);
        assert_eq!(resolve(vec![c.clone(), d], &p).len(), 1);
        assert_eq!(resolve(vec![c, e], &p).len(), 2);
    }

    #[test]
    fn the_same_stem_found_twice_is_kept_once() {
        let p = TlsParams::default();
        let a = tree(0.0, 0.0, &column(0.1, 4.0, 0.0, 0.0, 0.25));
        let mut shorter = column(0.1, 3.0, 0.01, 0.0, 0.25);
        shorter.truncate(10);
        let b = tree(0.01, 0.0, &shorter);
        let out = resolve(vec![a, b], &p);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].sprouts, 1);
        assert!((out[0].height - 4.1).abs() < 0.2);
    }
}
