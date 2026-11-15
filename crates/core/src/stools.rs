//! Troncos que crecen juntos. En un monte bajo (eucaliptos que brotan de la cepa) salen varios
//! troncos de una misma cepa; algunos árboles se bifurcan más arriba. Se distinguen a la altura del
//! diámetro (1,3 m): los troncos que están separados a esa altura son los brotes de una cepa y cada
//! uno es un árbol con su propio diámetro; los troncos que son uno por debajo y se bifurcan por
//! encima son un solo árbol bifurcado, con el diámetro de su tronco y el volumen del tronco y de
//! cada rama.

use std::collections::{BTreeSet, HashMap};

use crate::cluster::Dsu;
use crate::tls::{Cubing, Log, Section, TlsParams, TreeResult, smalian};

/// Dos secciones son el mismo círculo: centros más cercanos que el 40 % del diámetro mayor y
/// diámetros dentro del 35 % de él.
fn same_circle(a: &Section, b: &Section) -> bool {
    let wide = a.d.max(b.d);
    (a.x - b.x).hypot(a.y - b.y) < 0.4 * wide && (a.d - b.d).abs() < 0.35 * wide
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Relation {
    /// Dos troncos, separados a la altura del diámetro.
    Apart,
    /// Un tronco a la altura del diámetro que se bifurca por encima de ella, a esta altura.
    Fork(f64),
    /// El mismo tronco encontrado dos veces.
    Twin,
}

/// Cómo se relacionan dos árboles de una cepa, a partir de las secciones que tienen ambos.
fn relation(a: &TreeResult, b: &TreeResult, p: &TlsParams) -> Relation {
    // las alturas donde ambos tienen una sección válida y si es el mismo círculo
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
    // el par más cercano a la altura del diámetro dice cuáles son (a un tronco suele faltarle justo
    // allí una sección); sin ninguna sección en común, lo dicen las bases: dos troncos no pueden
    // crecer en el mismo sitio
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
    // un tronco a la altura del diámetro: una bifurcación si se separan por encima, si no, un
    // gemelo
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
            // una bifurcación por debajo de la altura del diámetro es una cepa de brotes
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

/// El perfil (altura, diámetro) de las secciones válidas o rellenadas que acepta `keep`.
fn profile(secs: &[Section], keep: &dyn Fn(f64) -> bool) -> Vec<(f64, f64)> {
    secs.iter()
        .filter(|s| (s.ok || s.d.is_finite()) && keep(s.h))
        .map(|s| (s.h, s.d))
        .collect()
}

/// Un árbol a partir de los troncos de una bifurcación: el tronco bajo la bifurcación se cubica una
/// vez y cada rama por encima, desde la bifurcación hacia arriba.
fn merge_fork(mut members: Vec<TreeResult>, fork_h: f64, p: &TlsParams) -> TreeResult {
    // el tronco bajo la bifurcación: el miembro con más secciones válidas por debajo
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
        // cada miembro sigue por encima de la bifurcación como una rama propia
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
    // lo que depende de la cubicación se vuelve a calcular para todo el árbol
    for stale in ["SIN_DAP", "SIN_DIAM", "TRUNCADO", "SIN_VOLUMEN"] {
        flags.remove(stale);
    }
    if tree.dbh.is_none() {
        flags.insert("SIN_DIAM".into());
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
        estimated: false,
    };
    tree.sections = sections;
    tree.points = points;
    tree.quality = quality / branches.max(1) as f64;
    tree.height = height;
    tree.fork_h = Some(fork_h);
    tree.flags = flags.into_iter().collect::<Vec<_>>().join(" ");
    tree
}

/// Distingue los brotes de una cepa de las ramas de una bifurcación, a partir de los árboles
/// medidos uno a uno: los gemelos se descartan, las bifurcaciones se fusionan en un árbol y los
/// troncos de cada cepa se numeran juntos (`stool`, `sprouts`; la marca `BROTES` señala una cepa
/// con varios).
pub fn resolve(trees: Vec<TreeResult>, p: &TlsParams) -> Vec<TreeResult> {
    let n = trees.len();
    // troncos que crecen en una cepa: bases más cercanas que la distancia entre cepas
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
                    // el tronco encontrado dos veces: el medido menos se queda fuera
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
    // los miembros de cada bifurcación y su altura: la más baja a la que se separan sus pares
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
    // con cuántos árboles terminó cada cepa
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

    /// Un árbol a partir de secciones `(altura, x, y, diámetro)`, todas válidas.
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

    /// Secciones cada 0,25 m de `from` a `to` a lo largo de un eje vertical.
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

    /// Un tronco de 30 cm hasta 2 m que se bifurca en dos ramas de 20 cm.
    fn forked_at(fork: f64) -> (TreeResult, TreeResult) {
        let branch = |side: f64| {
            let mut s = column(0.1, fork, 0.0, 0.0, 0.30);
            // por encima de la bifurcación las ramas se separan del eje 20 cm por cada 25 cm de
            // altura
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
        // el diámetro es el del tronco bajo la bifurcación
        assert!(t.dbh.is_some_and(|d| (d - 0.30).abs() < 1e-9));
        assert!(t.fork_h.is_some_and(|h| (2.0..2.3).contains(&h)));
        // el tronco una vez y una rama cada una: trozas de la rama 0, 1 y 2
        let ids: BTreeSet<u8> = t.cubing.logs.iter().map(|l| l.branch).collect();
        assert_eq!(ids, BTreeSet::from([0, 1, 2]));
        let volume: f64 = t.cubing.logs.iter().map(|l| l.volume).sum();
        assert!((t.cubing.volume - volume).abs() < 1e-12);
        // más que el tronco solo, y el tronco no se cuenta dos veces
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
        // secciones de un tronco encontrado dos veces, a cada una le faltan las que tiene la otra,
        // y ninguna entre 0,85 y 1,9 m en común
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
        // sin nada en común en absoluto, solo el mismo lugar los delata
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
