//! Giving each object a class: by rules written as text, or by a random forest trained on
//! objects that sample polygons cover.

use std::collections::HashMap;

use rayon::prelude::*;

use super::Labels;
use super::attributes::AttrTable;
use crate::vector::Region;
use crate::{Error, Result, check_cancel};

/// The class of every object (0: none), how sure the choice is (1 for rules), the class names
/// (class `c` is `names[c - 1]`) and a text on how it went.
#[derive(Clone, Debug, Default)]
pub struct Classified {
    pub class: Vec<u16>,
    pub confidence: Vec<f32>,
    pub names: Vec<String>,
    pub report: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cond {
    pub column: String,
    pub op: Op,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub class: String,
    pub conds: Vec<Cond>,
}

/// Reads rules, one per line: `class: column >= value and column < value`. A class with several
/// lines takes an object that meets any of them; the first line that fits wins. Blank lines and
/// lines starting with `#` are skipped.
pub fn parse_rules(text: &str) -> Result<Vec<Rule>> {
    let mut rules = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |why: &str| Error::Param(format!("regla de la línea {}: {why}", n + 1));
        let (class, rest) = line
            .split_once(':')
            .ok_or_else(|| bad("falta «clase: condición»"))?;
        let class = class.trim();
        if class.is_empty() {
            return Err(bad("la clase no tiene nombre"));
        }
        let mut conds = Vec::new();
        for part in rest.split(" and ") {
            let part = part.trim();
            // two-character operators first
            let found = ["<=", ">=", "==", "!=", "<", ">", "="]
                .iter()
                .find_map(|op| part.find(op).map(|at| (*op, at)));
            let Some((op, at)) = found else {
                return Err(bad(&format!("no se entiende «{part}»")));
            };
            let column = part[..at].trim();
            let value: f32 = part[at + op.len()..]
                .trim()
                .replace(',', ".")
                .parse()
                .map_err(|_| bad(&format!("«{part}» no acaba en un número")))?;
            if column.is_empty() {
                return Err(bad(&format!("«{part}» no dice qué atributo mide")));
            }
            conds.push(Cond {
                column: column.to_string(),
                op: match op {
                    "<=" => Op::Le,
                    ">=" => Op::Ge,
                    "<" => Op::Lt,
                    ">" => Op::Gt,
                    "!=" => Op::Ne,
                    _ => Op::Eq,
                },
                value,
            });
        }
        if conds.is_empty() {
            return Err(bad("sin condiciones"));
        }
        rules.push(Rule {
            class: class.to_string(),
            conds,
        });
    }
    if rules.is_empty() {
        return Err(Error::Param("el archivo de reglas está vacío".into()));
    }
    Ok(rules)
}

impl Cond {
    #[inline]
    fn holds(&self, v: f32) -> bool {
        // a missing value fails every condition
        !v.is_nan()
            && match self.op {
                Op::Lt => v < self.value,
                Op::Le => v <= self.value,
                Op::Gt => v > self.value,
                Op::Ge => v >= self.value,
                Op::Eq => (v - self.value).abs() <= 1e-6 * self.value.abs().max(1.0),
                Op::Ne => (v - self.value).abs() > 1e-6 * self.value.abs().max(1.0),
            }
    }
}

/// A rule ready to run: its class code and, per condition, the column it reads.
type Compiled<'a> = (u16, Vec<(&'a [f32], &'a Cond)>);

/// Classifies the objects by rules.
pub fn classify_rules(table: &AttrTable, rules: &[Rule]) -> Result<Classified> {
    let mut names: Vec<String> = Vec::new();
    for r in rules {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&r.class)) {
            names.push(r.class.clone());
        }
    }
    // each rule: class code and the columns of its conditions
    let mut compiled: Vec<Compiled> = Vec::new();
    for r in rules {
        let code = names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(&r.class))
            .map_or(0, |p| p as u16 + 1);
        let mut conds = Vec::new();
        for c in &r.conds {
            let col = table.column(&c.column).ok_or_else(|| {
                Error::Param(format!(
                    "la regla de «{}» usa «{}», que no es un atributo (hay: {})",
                    r.class,
                    c.column,
                    table.names.join(", ")
                ))
            })?;
            conds.push((col, c));
        }
        compiled.push((code, conds));
    }
    let class: Vec<u16> = (0..table.objects)
        .into_par_iter()
        .map(|o| {
            compiled
                .iter()
                .find(|(_, conds)| conds.iter().all(|(col, c)| c.holds(col[o])))
                .map_or(0, |(code, _)| *code)
        })
        .collect();
    let mut count = vec![0usize; names.len() + 1];
    for &c in &class {
        count[c as usize] += 1;
    }
    let mut report = format!("{} objetos clasificados por reglas:", table.objects);
    for (i, n) in names.iter().enumerate() {
        report.push_str(&format!("\n  {n}: {}", count[i + 1]));
    }
    report.push_str(&format!("\n  sin clase: {}", count[0]));
    Ok(Classified {
        confidence: class.iter().map(|&c| f32::from(u8::from(c > 0))).collect(),
        class,
        names,
        report,
    })
}

/// The training sample: the objects the sample polygons cover, and their class codes.
pub struct Samples {
    pub names: Vec<String>,
    /// (object index, class code)
    pub objects: Vec<(usize, u16)>,
}

/// Which objects the sample polygons stand for. `regions` are named by their class. An object
/// takes the class that covers most of its cells, as long as the polygons cover at least
/// `min_cover` of it (half is a good value): a polygon drawn well inside an object is enough
/// only when the object is small, so draw samples that fill the objects they mean.
pub fn sample_objects(labels: &Labels, regions: &[Region], min_cover: f64) -> Result<Samples> {
    let lat = labels.lat;
    let mut names: Vec<String> = Vec::new();
    for r in regions {
        let n = r.name.trim();
        if !n.is_empty() && !names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
            names.push(n.to_string());
        }
    }
    names.sort_unstable_by_key(|n| n.to_lowercase());
    let code = |n: &str| {
        names
            .iter()
            .position(|x| x.eq_ignore_ascii_case(n.trim()))
            .map(|p| p as u16 + 1)
    };
    let mut covered: HashMap<(u32, u16), u32> = HashMap::new();
    for r in regions {
        let Some(c) = code(&r.name) else { continue };
        let b = r.bbox();
        let c0 = (((b[0] - lat.x0) / lat.cell).floor().max(0.0)) as usize;
        let c1 = (((b[2] - lat.x0) / lat.cell).ceil().max(0.0) as usize).min(lat.cols);
        let r0 = (lat.rows as f64 - ((b[3] - lat.y0) / lat.cell).ceil()).max(0.0) as usize;
        let r1 = ((lat.rows as f64 - ((b[1] - lat.y0) / lat.cell).floor()).max(0.0) as usize)
            .min(lat.rows);
        for row in r0..r1 {
            for col in c0..c1 {
                let id = labels.ids[row * lat.cols + col];
                if id == 0 {
                    continue;
                }
                let x = lat.x0 + (col as f64 + 0.5) * lat.cell;
                let y = lat.y0 + (lat.rows - row) as f64 * lat.cell - lat.cell / 2.0;
                if r.contains(x, y) {
                    *covered.entry((id, c)).or_insert(0) += 1;
                }
            }
        }
        check_cancel()?;
    }
    let mut size = vec![0u32; labels.count as usize + 1];
    for &l in &labels.ids {
        size[l as usize] += 1;
    }
    // for each object, the class with the most covered cells
    let mut best: HashMap<u32, (u16, u32)> = HashMap::new();
    for (&(id, c), &n) in &covered {
        let e = best.entry(id).or_insert((c, n));
        if n > e.1 || (n == e.1 && c < e.0) {
            *e = (c, n);
        }
    }
    let mut objects: Vec<(usize, u16)> = best
        .into_iter()
        .filter(|&(id, (_, n))| f64::from(n) >= min_cover * f64::from(size[id as usize]))
        .map(|(id, (c, _))| (id as usize - 1, c))
        .collect();
    objects.sort_unstable();
    Ok(Samples { names, objects })
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForestParams {
    pub trees: usize,
    pub max_depth: usize,
    /// Fewest samples in a leaf.
    pub min_leaf: usize,
    pub seed: u64,
    /// Attributes left out of the training (the position makes a forest learn places).
    pub exclude: Vec<String>,
}

impl Default for ForestParams {
    fn default() -> Self {
        Self {
            trees: 100,
            max_depth: 24,
            min_leaf: 1,
            seed: 1,
            exclude: vec!["x".into(), "y".into()],
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[derive(Clone, Copy)]
struct Node {
    /// `u32::MAX` for a leaf.
    feature: u32,
    threshold: f32,
    /// Child nodes; in a leaf, `left` is the class.
    left: u32,
    right: u32,
}

struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    #[inline]
    fn predict(&self, row: &[f32]) -> u16 {
        let mut at = 0usize;
        loop {
            let n = self.nodes[at];
            if n.feature == u32::MAX {
                return n.left as u16;
            }
            at = if row[n.feature as usize] <= n.threshold {
                n.left as usize
            } else {
                n.right as usize
            };
        }
    }
}

struct Builder<'a> {
    x: &'a [f32],
    f: usize,
    y: &'a [u16],
    classes: usize,
    mtry: usize,
    max_depth: usize,
    min_leaf: usize,
    rng: Rng,
    nodes: Vec<Node>,
    importance: Vec<f64>,
}

impl Builder<'_> {
    fn gini(counts: &[u32], n: f64) -> f64 {
        1.0 - counts
            .iter()
            .map(|&c| (f64::from(c) / n).powi(2))
            .sum::<f64>()
    }

    fn build(&mut self, idx: &mut [u32], depth: usize) -> u32 {
        let at = self.nodes.len();
        self.nodes.push(Node {
            feature: u32::MAX,
            threshold: 0.0,
            left: 0,
            right: 0,
        });
        let mut counts = vec![0u32; self.classes];
        for &s in idx.iter() {
            counts[self.y[s as usize] as usize] += 1;
        }
        let majority = counts
            .iter()
            .enumerate()
            .max_by_key(|&(c, &n)| (n, std::cmp::Reverse(c)))
            .map_or(0, |(c, _)| c);
        self.nodes[at].left = majority as u32;
        let n = idx.len() as f64;
        let pure = counts[majority] as usize == idx.len();
        if pure || depth >= self.max_depth || idx.len() < 2 * self.min_leaf.max(1) {
            return at as u32;
        }
        let parent = Self::gini(&counts, n);
        // best split among a few attributes drawn at random
        let mut order: Vec<usize> = (0..self.f).collect();
        for i in 0..self.mtry.min(self.f) {
            let j = i + self.rng.below(self.f - i);
            order.swap(i, j);
        }
        let mut best: Option<(f64, usize, f32)> = None;
        let mut tmp: Vec<(f32, u16)> = Vec::with_capacity(idx.len());
        for &feat in order.iter().take(self.mtry) {
            tmp.clear();
            tmp.extend(
                idx.iter()
                    .map(|&s| (self.x[s as usize * self.f + feat], self.y[s as usize])),
            );
            tmp.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            let mut left = vec![0u32; self.classes];
            let mut right = counts.clone();
            for i in 0..tmp.len() - 1 {
                let c = tmp[i].1 as usize;
                left[c] += 1;
                right[c] -= 1;
                if tmp[i].0 == tmp[i + 1].0 {
                    continue;
                }
                let (nl, nr) = ((i + 1) as f64, n - (i + 1) as f64);
                if (i + 1) < self.min_leaf || (tmp.len() - i - 1) < self.min_leaf {
                    continue;
                }
                let gain =
                    parent - nl / n * Self::gini(&left, nl) - nr / n * Self::gini(&right, nr);
                if gain > 1e-12 && best.is_none_or(|b| gain > b.0) {
                    best = Some((gain, feat, tmp[i].0 / 2.0 + tmp[i + 1].0 / 2.0));
                }
            }
        }
        let Some((gain, feat, thr)) = best else {
            return at as u32;
        };
        self.importance[feat] += gain * n;
        // partition in place
        let mut mid = 0;
        for i in 0..idx.len() {
            if self.x[idx[i] as usize * self.f + feat] <= thr {
                idx.swap(i, mid);
                mid += 1;
            }
        }
        let (l, r) = idx.split_at_mut(mid);
        let left = self.build(l, depth + 1);
        let right = self.build(r, depth + 1);
        self.nodes[at] = Node {
            feature: feat as u32,
            threshold: thr,
            left,
            right,
        };
        at as u32
    }
}

/// A random forest (Breiman 2001): trees grown on resamples of the training objects, each split
/// chosen among a few attributes drawn at random, and the class the trees vote for.
pub struct Forest {
    trees: Vec<Tree>,
    classes: usize,
    /// The attributes used, by name, and the value that stands for a missing one.
    features: Vec<String>,
    fill: Vec<f32>,
}

/// How well a forest does on the objects it did not see in training.
pub struct Accuracy {
    /// `matrix[reference][predicted]`.
    pub matrix: Vec<Vec<u32>>,
    pub overall: f64,
    pub kappa: f64,
}

impl Accuracy {
    fn from_matrix(matrix: Vec<Vec<u32>>) -> Self {
        let total: f64 = matrix.iter().flatten().map(|&v| f64::from(v)).sum();
        let diag: f64 = (0..matrix.len()).map(|i| f64::from(matrix[i][i])).sum();
        let overall = if total > 0.0 { diag / total } else { 0.0 };
        let chance: f64 = (0..matrix.len())
            .map(|i| {
                let row: f64 = matrix[i].iter().map(|&v| f64::from(v)).sum();
                let col: f64 = matrix.iter().map(|r| f64::from(r[i])).sum();
                row * col
            })
            .sum::<f64>()
            / (total * total).max(1.0);
        let kappa = if chance < 1.0 {
            (overall - chance) / (1.0 - chance)
        } else {
            1.0
        };
        Self {
            matrix,
            overall,
            kappa,
        }
    }
}

/// A forest, how well it does and the attributes ranked by how much they count.
pub type Trained = (Forest, Accuracy, Vec<(String, f64)>);

impl Forest {
    /// Trains on the rows `samples` of the table (object, class code starting at 1), and tells how
    /// it does on the samples each tree left out.
    pub fn train(
        table: &AttrTable,
        samples: &[(usize, u16)],
        n_classes: usize,
        p: &ForestParams,
    ) -> Result<Trained> {
        let used: Vec<usize> = (0..table.names.len())
            .filter(|&c| {
                !p.exclude
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(&table.names[c]))
            })
            .collect();
        let f = used.len();
        if f == 0 || samples.len() < 2 || n_classes < 2 {
            return Err(Error::Param(
                "hacen falta al menos dos clases con muestras y algún atributo".into(),
            ));
        }
        let mut present = vec![0usize; n_classes];
        for &(_, c) in samples {
            present[c as usize - 1] += 1;
        }
        if present.iter().filter(|&&n| n > 0).count() < 2 {
            return Err(Error::Param(
                "las muestras cubren objetos de una sola clase: dibuja muestras de otra clase"
                    .into(),
            ));
        }
        let fill: Vec<f32> = used
            .iter()
            .map(|&c| {
                let (s, n) = samples
                    .iter()
                    .filter_map(|&(o, _)| Some(table.cols[c][o]).filter(|v| !v.is_nan()))
                    .fold((0f64, 0u32), |(s, n), v| (s + f64::from(v), n + 1));
                if n > 0 {
                    (s / f64::from(n)) as f32
                } else {
                    0.0
                }
            })
            .collect();
        let fill_ref = &fill;
        let x: Vec<f32> = samples
            .iter()
            .flat_map(|&(o, _)| {
                used.iter().enumerate().map(move |(j, &c)| {
                    let v = table.cols[c][o];
                    if v.is_nan() { fill_ref[j] } else { v }
                })
            })
            .collect();
        let y: Vec<u16> = samples.iter().map(|&(_, c)| c - 1).collect();
        let s = samples.len();
        let mtry = ((f as f64).sqrt().round() as usize).max(1);
        let grown: Vec<(Tree, Vec<bool>, Vec<f64>)> = (0..p.trees.max(1))
            .into_par_iter()
            .map(|t| {
                let mut rng = Rng(p
                    .seed
                    .wrapping_add((t as u64 + 1).wrapping_mul(0xD134_2543_DE82_EF95)));
                let mut bag: Vec<u32> = (0..s).map(|_| rng.below(s) as u32).collect();
                let mut in_bag = vec![false; s];
                for &i in &bag {
                    in_bag[i as usize] = true;
                }
                let mut b = Builder {
                    x: &x,
                    f,
                    y: &y,
                    classes: n_classes,
                    mtry,
                    max_depth: p.max_depth.max(1),
                    min_leaf: p.min_leaf,
                    rng,
                    nodes: Vec::new(),
                    importance: vec![0.0; f],
                };
                b.build(&mut bag, 0);
                (Tree { nodes: b.nodes }, in_bag, b.importance)
            })
            .collect();
        check_cancel()?;
        // votes of the trees that did not train on each sample
        let mut votes = vec![0u32; s * n_classes];
        for (tree, in_bag, _) in &grown {
            for i in 0..s {
                if !in_bag[i] {
                    let c = tree.predict(&x[i * f..(i + 1) * f]) as usize;
                    votes[i * n_classes + c] += 1;
                }
            }
        }
        let mut matrix = vec![vec![0u32; n_classes]; n_classes];
        for i in 0..s {
            let v = &votes[i * n_classes..(i + 1) * n_classes];
            if v.iter().all(|&n| n == 0) {
                continue;
            }
            let pred = v
                .iter()
                .enumerate()
                .max_by_key(|&(c, &n)| (n, std::cmp::Reverse(c)))
                .map_or(0, |(c, _)| c);
            matrix[y[i] as usize][pred] += 1;
        }
        let mut importance = vec![0f64; f];
        for (_, _, imp) in &grown {
            for (a, b) in importance.iter_mut().zip(imp) {
                *a += b;
            }
        }
        let total: f64 = importance.iter().sum::<f64>().max(1e-12);
        let mut ranked: Vec<(String, f64)> = used
            .iter()
            .zip(&importance)
            .map(|(&c, v)| (table.names[c].clone(), v / total))
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let forest = Self {
            trees: grown.into_iter().map(|g| g.0).collect(),
            classes: n_classes,
            features: used.iter().map(|&c| table.names[c].clone()).collect(),
            fill,
        };
        Ok((forest, Accuracy::from_matrix(matrix), ranked))
    }

    /// The class of every object of the table and the share of the trees that chose it.
    pub fn predict(&self, table: &AttrTable) -> Result<(Vec<u16>, Vec<f32>)> {
        let cols: Vec<&[f32]> = self
            .features
            .iter()
            .map(|n| {
                table
                    .column(n)
                    .ok_or_else(|| Error::Param(format!("falta el atributo «{n}» de la tabla")))
            })
            .collect::<Result<_>>()?;
        let f = cols.len();
        let n_trees = self.trees.len() as f32;
        let out: Vec<(u16, f32)> = (0..table.objects)
            .into_par_iter()
            .map_init(
                || (vec![0f32; f], vec![0u32; self.classes]),
                |(row, votes), o| {
                    for j in 0..f {
                        let v = cols[j][o];
                        row[j] = if v.is_nan() { self.fill[j] } else { v };
                    }
                    votes.fill(0);
                    for t in &self.trees {
                        votes[t.predict(row) as usize] += 1;
                    }
                    let (c, n) = votes
                        .iter()
                        .enumerate()
                        .max_by_key(|&(c, &n)| (n, std::cmp::Reverse(c)))
                        .map_or((0, 0), |(c, &n)| (c, n));
                    (c as u16 + 1, n as f32 / n_trees)
                },
            )
            .collect();
        Ok(out.into_iter().unzip())
    }
}

/// Trains a forest on the objects the `regions` (named by class) cover and classifies every
/// object with it.
pub fn classify_samples(
    labels: &Labels,
    table: &AttrTable,
    regions: &[Region],
    p: &ForestParams,
) -> Result<Classified> {
    let s = sample_objects(labels, regions, 0.5)?;
    if s.objects.is_empty() {
        return Err(Error::Param(
            "ninguna muestra cubre un objeto (al menos la mitad de sus celdas): comprueba que las muestras caen sobre la zona segmentada".into(),
        ));
    }
    let (forest, acc, ranked) = Forest::train(table, &s.objects, s.names.len(), p)?;
    let (class, confidence) = forest.predict(table)?;
    let mut per_class = vec![0usize; s.names.len()];
    for &(_, c) in &s.objects {
        per_class[c as usize - 1] += 1;
    }
    let mut report = format!(
        "{} objetos de entrenamiento ({}) y {} árboles.\nPrecisión con los objetos que cada árbol no vio: {:.1} % (kappa {:.2})",
        s.objects.len(),
        s.names
            .iter()
            .zip(&per_class)
            .map(|(n, c)| format!("{n}: {c}"))
            .collect::<Vec<_>>()
            .join(", "),
        p.trees,
        acc.overall * 100.0,
        acc.kappa
    );
    report.push_str("\nPor clase (precisión del usuario / del productor):");
    for (i, n) in s.names.iter().enumerate() {
        let row: u32 = acc.matrix[i].iter().sum();
        let col: u32 = acc.matrix.iter().map(|r| r[i]).sum();
        let hit = f64::from(acc.matrix[i][i]);
        let pct = |d: u32| {
            if d == 0 {
                "—".to_string()
            } else {
                format!("{:.0} %", hit / f64::from(d) * 100.0)
            }
        };
        report.push_str(&format!("\n  {n}: {} / {}", pct(col), pct(row)));
    }
    report.push_str("\nAtributos que más pesan: ");
    report.push_str(
        &ranked
            .iter()
            .take(5)
            .map(|(n, v)| format!("{n} ({:.0} %)", v * 100.0))
            .collect::<Vec<_>>()
            .join(", "),
    );
    Ok(Classified {
        class,
        confidence,
        names: s.names,
        report,
    })
}

/// Groups the objects into `k` strata by how alike they are, without rules or samples: k-means
/// (Lloyd 1982, seeded as in Arthur & Vassilvitskii 2007) on the standardised `features`, each
/// object weighing its area so that big objects set the strata. With no `features`, the means
/// and 95th percentiles of the layers and the canopy cover are used. Strata are numbered from the
/// lowest value of the first feature to the highest.
pub fn classify_kmeans(
    table: &AttrTable,
    k: usize,
    features: &[String],
    seed: u64,
) -> Result<Classified> {
    let names: Vec<String> = if features.is_empty() {
        table
            .names
            .iter()
            .filter(|n| n.ends_with("_media") || n.ends_with("_p95") || n.as_str() == "cobertura")
            .cloned()
            .collect()
    } else {
        features.to_vec()
    };
    let mut cols: Vec<&[f32]> = Vec::new();
    for n in &names {
        cols.push(table.column(n).ok_or_else(|| {
            Error::Param(format!(
                "«{n}» no es un atributo (hay: {})",
                table.names.join(", ")
            ))
        })?);
    }
    let f = cols.len();
    let n = table.objects;
    if f == 0 || !(2..=64).contains(&k) || n < k {
        return Err(Error::Param(
            "hacen falta entre 2 y 64 estratos, no más que objetos, y algún atributo".into(),
        ));
    }
    let weight: Vec<f64> = match table.column("area") {
        Some(a) => a.iter().map(|&v| f64::from(v).max(1e-9)).collect(),
        None => vec![1.0; n],
    };
    let total_w: f64 = weight.iter().sum();
    // weighted mean and spread of each feature, and the objects in those units
    let mut x = vec![0f64; n * f];
    let mut mean = vec![0f64; f];
    let mut spread = vec![1f64; f];
    for (j, col) in cols.iter().enumerate() {
        let (mut sw, mut sv) = (0f64, 0f64);
        for (o, &v) in col.iter().enumerate() {
            if !v.is_nan() {
                sw += weight[o];
                sv += weight[o] * f64::from(v);
            }
        }
        mean[j] = if sw > 0.0 { sv / sw } else { 0.0 };
        let var = col
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.is_nan())
            .map(|(o, &v)| weight[o] * (f64::from(v) - mean[j]).powi(2))
            .sum::<f64>()
            / sw.max(1e-12);
        if var.sqrt() > 1e-12 {
            spread[j] = var.sqrt();
        }
        for (o, &v) in col.iter().enumerate() {
            x[o * f + j] = if v.is_nan() {
                0.0
            } else {
                (f64::from(v) - mean[j]) / spread[j]
            };
        }
    }
    let dist2 = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(p, q)| (p - q) * (p - q)).sum::<f64>();
    let mut rng = Rng(seed ^ 0x5DEE_CE66_D1CE_4E5B);
    let unit = |r: &mut Rng| (r.next() >> 11) as f64 / (1u64 << 53) as f64;
    // seeding: far-apart objects, more likely when they weigh more
    let mut centres: Vec<f64> = Vec::with_capacity(k * f);
    let pick = |r: &mut Rng, w: &[f64]| {
        let total: f64 = w.iter().sum();
        let mut t = unit(r) * total;
        for (o, &v) in w.iter().enumerate() {
            t -= v;
            if t <= 0.0 {
                return o;
            }
        }
        w.len() - 1
    };
    let first = pick(&mut rng, &weight);
    centres.extend_from_slice(&x[first * f..(first + 1) * f]);
    let mut near: Vec<f64> = (0..n)
        .map(|o| dist2(&x[o * f..(o + 1) * f], &centres[..f]))
        .collect();
    while centres.len() < k * f {
        let w: Vec<f64> = near.iter().zip(&weight).map(|(d, w)| d * w).collect();
        let o = if w.iter().sum::<f64>() > 0.0 {
            pick(&mut rng, &w)
        } else {
            rng.below(n)
        };
        let c = centres.len();
        centres.extend_from_slice(&x[o * f..(o + 1) * f]);
        for (q, d) in near.iter_mut().enumerate() {
            *d = d.min(dist2(&x[q * f..(q + 1) * f], &centres[c..c + f]));
        }
    }
    let mut label = vec![u32::MAX; n];
    for _ in 0..100 {
        check_cancel()?;
        let next: Vec<u32> = (0..n)
            .into_par_iter()
            .map(|o| {
                let row = &x[o * f..(o + 1) * f];
                (0..k)
                    .min_by(|&a, &b| {
                        dist2(row, &centres[a * f..(a + 1) * f])
                            .total_cmp(&dist2(row, &centres[b * f..(b + 1) * f]))
                    })
                    .map_or(0, |c| c as u32)
            })
            .collect();
        let changed = next != label;
        label = next;
        let mut sum = vec![0f64; k * f];
        let mut sw = vec![0f64; k];
        for o in 0..n {
            let c = label[o] as usize;
            sw[c] += weight[o];
            for j in 0..f {
                sum[c * f + j] += weight[o] * x[o * f + j];
            }
        }
        for c in 0..k {
            if sw[c] > 0.0 {
                for j in 0..f {
                    centres[c * f + j] = sum[c * f + j] / sw[c];
                }
            } else {
                // an empty stratum starts again at the object farthest from its centre
                let far = (0..n)
                    .max_by(|&a, &b| {
                        let da = dist2(
                            &x[a * f..(a + 1) * f],
                            &centres[label[a] as usize * f..(label[a] as usize + 1) * f],
                        );
                        let db = dist2(
                            &x[b * f..(b + 1) * f],
                            &centres[label[b] as usize * f..(label[b] as usize + 1) * f],
                        );
                        da.total_cmp(&db)
                    })
                    .unwrap_or(0);
                centres[c * f..(c + 1) * f].copy_from_slice(&x[far * f..(far + 1) * f]);
            }
        }
        if !changed {
            break;
        }
    }
    // number the strata from the lowest value of the first feature
    let mut order: Vec<usize> = (0..k).collect();
    order.sort_by(|&a, &b| centres[a * f].total_cmp(&centres[b * f]));
    let mut code = vec![0u16; k];
    for (rank, &c) in order.iter().enumerate() {
        code[c] = rank as u16 + 1;
    }
    let mut class = vec![0u16; n];
    let mut confidence = vec![1f32; n];
    for o in 0..n {
        let row = &x[o * f..(o + 1) * f];
        let mut d: Vec<f64> = (0..k)
            .map(|c| dist2(row, &centres[c * f..(c + 1) * f]).sqrt())
            .collect();
        let c = label[o] as usize;
        class[o] = code[c];
        let d1 = d[c];
        d.swap_remove(c);
        let d2 = d.into_iter().fold(f64::INFINITY, f64::min);
        if d2.is_finite() && d1 + d2 > 0.0 {
            confidence[o] = (d2 / (d1 + d2)) as f32;
        }
    }
    let names_out: Vec<String> = (1..=k).map(|i| format!("estrato {i}")).collect();
    let mut report = format!(
        "{n} objetos agrupados en {k} estratos según: {}",
        names.join(", ")
    );
    for (rank, &c) in order.iter().enumerate() {
        let w: f64 = (0..n)
            .filter(|&o| label[o] as usize == c)
            .map(|o| weight[o])
            .sum();
        let count = (0..n).filter(|&o| label[o] as usize == c).count();
        report.push_str(&format!(
            "\n  estrato {}: {count} objetos, {:.1} % de la superficie, {} ≈ {:.2}",
            rank + 1,
            100.0 * w / total_w,
            names[0],
            centres[c * f] * spread[0] + mean[0]
        ));
    }
    Ok(Classified {
        class,
        confidence,
        names: names_out,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(cols: Vec<(&str, Vec<f32>)>) -> AttrTable {
        let objects = cols.first().map_or(0, |c| c.1.len());
        AttrTable {
            names: cols.iter().map(|c| c.0.to_string()).collect(),
            cols: cols.into_iter().map(|c| c.1).collect(),
            objects,
            centroid: Vec::new(),
        }
    }

    #[test]
    fn rules_apply_in_order_and_missing_values_fail() -> Result<()> {
        let t = table(vec![
            ("chm_media", vec![12.0, 0.2, 3.0, f32::NAN]),
            ("area", vec![30.0, 500.0, 10.0, 10.0]),
        ]);
        let rules = parse_rules(
            "# alturas\narbol: chm_media >= 5 and area < 100\nsuelo: chm_media < 0,5\nmatorral: chm_media >= 1 and chm_media < 5",
        )?;
        let c = classify_rules(&t, &rules)?;
        assert_eq!(c.names, vec!["arbol", "suelo", "matorral"]);
        assert_eq!(c.class, vec![1, 2, 3, 0]);
        assert!(parse_rules("sin dos puntos").is_err());
        assert!(parse_rules("a: nada").is_err());
        let bad = parse_rules("a: inexistente > 1")?;
        assert!(classify_rules(&t, &bad).is_err());
        Ok(())
    }

    #[test]
    fn a_forest_learns_two_separable_classes() -> Result<()> {
        // class 1 where h is low, class 2 where it is high; "ruido" carries no signal
        let n = 200;
        let h: Vec<f32> = (0..n)
            .map(|i| (i % 100) as f32 / 10.0 + if i < 100 { 0.0 } else { 20.0 })
            .collect();
        let noise: Vec<f32> = (0..n).map(|i| ((i * 7919) % 13) as f32).collect();
        let t = table(vec![("h", h), ("ruido", noise)]);
        let samples: Vec<(usize, u16)> = (0..n)
            .filter(|i| i % 4 == 0)
            .map(|i| (i, if i < 100 { 1 } else { 2 }))
            .collect();
        let (forest, acc, ranked) = Forest::train(&t, &samples, 2, &ForestParams::default())?;
        assert!(acc.overall > 0.95 && acc.kappa > 0.9);
        assert_eq!(ranked[0].0, "h");
        let (class, conf) = forest.predict(&t)?;
        for i in 0..n {
            assert_eq!(class[i], if i < 100 { 1 } else { 2 }, "object {i}");
            assert!(conf[i] > 0.5);
        }
        // the same seed gives the same forest
        let (again, _, _) = Forest::train(&t, &samples, 2, &ForestParams::default())?;
        assert_eq!(again.predict(&t)?.0, class);
        Ok(())
    }

    #[test]
    fn kmeans_finds_the_groups_and_numbers_them_by_height() -> Result<()> {
        // three groups of objects by height; the area weights are equal
        let h: Vec<f32> = (0..90)
            .map(|i| [20.0, 2.0, 9.0][i / 30] + (i % 5) as f32 * 0.1)
            .collect();
        let t = table(vec![("h_media", h), ("area", vec![100.0; 90])]);
        let a = classify_kmeans(&t, 3, &[], 7)?;
        // lowest group (objects 30..60) is stratum 1, highest (0..30) stratum 3
        assert!(a.class[30..60].iter().all(|&c| c == 1));
        assert!(a.class[60..90].iter().all(|&c| c == 2));
        assert!(a.class[0..30].iter().all(|&c| c == 3));
        assert_eq!(a.names, vec!["estrato 1", "estrato 2", "estrato 3"]);
        // same seed, same strata
        assert_eq!(classify_kmeans(&t, 3, &[], 7)?.class, a.class);
        assert!(classify_kmeans(&t, 1, &[], 7).is_err());
        assert!(classify_kmeans(&t, 3, &["nada".into()], 7).is_err());
        Ok(())
    }

    #[test]
    fn one_class_only_is_refused() {
        let t = table(vec![("h", vec![1.0, 2.0, 3.0])]);
        let r = Forest::train(&t, &[(0, 1), (1, 1)], 2, &ForestParams::default());
        assert!(r.is_err());
    }
}
