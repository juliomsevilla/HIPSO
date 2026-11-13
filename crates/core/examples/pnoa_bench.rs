//! A recipe against the classification a file carries: `pnoa_bench --recipe R.txt FILE... [--size
//! 500] [--inner 400]`. For each file takes a square window of `--size` m around the middle of the
//! cloud, classifies it with the recipe and compares, in the inner `--inner` m (the rest is the
//! buffer, so the edge effects of the filters stay out), the classes it gives with the file's own,
//! class 12 (overlap between flight lines) included.
//!
//! `--analyze` instead prints, for each class of the file, how high its points are above the file's
//! own ground (class 2): the thresholds the producer used show in it.
//!
//! The reference is whatever classification the file carries (for PNOA sheets, the producer's):
//! the numbers measure agreement with it, not truth.

use std::collections::BTreeMap;
use std::sync::Mutex;

use hipso_core::grid::Grid;
use hipso_core::las::{Points, Reader};
use hipso_core::pipeline;
use hipso_core::recipe;
use hipso_core::tin;
use rayon::prelude::*;

const CLASSES: [u8; 13] = [1, 2, 3, 4, 5, 6, 7, 9, 11, 12, 14, 15, 17];

fn slot(c: u8) -> usize {
    CLASSES
        .iter()
        .position(|x| *x == c)
        .unwrap_or(CLASSES.len())
}

fn read_window(f: &str, size: f64) -> Result<Points, Box<dyn std::error::Error>> {
    let r = Reader::open(std::path::Path::new(f))?;
    let hb = r.header.bounds;
    let (cx, cy) = ((hb[0] + hb[3]) / 2.0, (hb[1] + hb[4]) / 2.0);
    let half = if size > 0.0 {
        size / 2.0
    } else {
        f64::INFINITY
    };
    let mut pts = Vec::new();
    r.scan(|q| {
        if (q.x - cx).abs() <= half && (q.y - cy).abs() <= half {
            pts.push(*q);
        }
    })?;
    Ok(Points::new(pts))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut files, mut recipe_path) = (Vec::new(), None);
    let (mut size, mut inner, mut analyze) = (500.0_f64, 400.0_f64, false);
    let (mut wire_spec, mut pole_spec) = (String::new(), String::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--recipe" => recipe_path = Some(it.next().ok_or("--recipe needs a file")?.clone()),
            "--size" => size = it.next().ok_or("--size needs a value")?.parse()?,
            "--inner" => inner = it.next().ok_or("--inner needs a value")?.parse()?,
            "--analyze" => analyze = true,
            "--wire" => wire_spec = it.next().ok_or("--wire needs k=v")?.clone(),
            "--pole" => pole_spec = it.next().ok_or("--pole needs k=v")?.clone(),
            f => files.push(f.to_string()),
        }
    }
    if files.is_empty() {
        return Err("usage: pnoa_bench --recipe R.txt FILE... [--size 500] [--inner 400] | --analyze FILE...".into());
    }

    if analyze {
        let mut heights: BTreeMap<u8, Vec<f32>> = BTreeMap::new();
        for f in &files {
            let w = read_window(f, size)?;
            let g: Vec<[f64; 3]> = w
                .pts
                .iter()
                .filter(|q| q.class == 2)
                .map(|q| [q.x, q.y, q.z])
                .collect();
            let b = [w.bounds[0], w.bounds[1], w.bounds[3], w.bounds[4]];
            let mut dtm = Grid::covering(b, 0.5)?;
            tin::rasterize(&g, &mut dtm);
            for q in &w.pts {
                if let Some(h) = dtm.sample(q.x, q.y) {
                    heights
                        .entry(q.class)
                        .or_default()
                        .push((q.z - f64::from(h)) as f32);
                }
            }
        }
        println!("| class | points | p1 | p5 | p25 | median | p75 | p95 | p99 |");
        println!("|---|---|---|---|---|---|---|---|---|");
        for (c, mut v) in heights {
            v.sort_by(f32::total_cmp);
            let pc = |p: f64| v[((v.len() - 1) as f64 * p) as usize];
            println!(
                "| {c} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |",
                v.len(),
                pc(0.01),
                pc(0.05),
                pc(0.25),
                pc(0.5),
                pc(0.75),
                pc(0.95),
                pc(0.99)
            );
        }
        return Ok(());
    }

    let text = std::fs::read_to_string(recipe_path.ok_or("--recipe needs a file")?)?;
    let loaded = recipe::parse(&text)?;
    if !loaded.ignored.is_empty() {
        eprintln!("ignored in the recipe: {}", loaded.ignored.join(" "));
    }
    let mut steps = loaded.recipe.steps;
    // `--wire k=v,...` / `--pole k=v,...` change the parameters of those steps (to tune them)
    for (kind, spec) in [("wire", &wire_spec), ("pole", &pole_spec)] {
        for kv in spec.split(',').filter(|t| !t.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or("--wire/--pole take k=v,k=v")?;
            let v: f64 = v.parse()?;
            if let (Some(w), "wire") = (steps.extras.wires.as_mut(), kind) {
                match k {
                    "min_height" => w.min_height = v,
                    "radius" => w.radius = v,
                    "linearity" => w.linearity = v,
                    "max_slope" => w.max_slope = v,
                    "max_neighbours" => w.max_neighbours = v as usize,
                    "min_length" => w.min_length = v,
                    _ => return Err(format!("unknown wire parameter {k}").into()),
                }
            }
            if let (Some(w), "pole") = (steps.extras.poles.as_mut(), kind) {
                match k {
                    "min_height" => w.min_height = v,
                    "radius" => w.radius = v,
                    "max_width" => w.max_width = v,
                    "min_extent" => w.min_extent = v,
                    "head_length" => w.head_length = v,
                    _ => return Err(format!("unknown pole parameter {k}").into()),
                }
            }
        }
    }
    let n = CLASSES.len() + 1;
    let total = Mutex::new(vec![vec![0u64; n]; n]);
    let rows: Mutex<Vec<(String, Vec<Vec<u64>>)>> = Mutex::new(Vec::new());
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
    pool.install(|| {
        files.par_iter().for_each(|f| {
            let Ok(mut w) = read_window(f, size) else {
                eprintln!("{f}: not read");
                return;
            };
            let b = w.bounds;
            let (cx, cy) = ((b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0);
            let reference: Vec<u8> = w.pts.iter().map(|q| q.class).collect();
            // the recipe must find every class itself: none of the file's survives into its input
            for q in &mut w.pts {
                q.class = 1;
            }
            let Ok(pred) = pipeline::classify_points(&mut w, &steps) else {
                eprintln!("{f}: classification failed");
                return;
            };
            let mut m = vec![vec![0u64; n]; n];
            for (k, q) in w.pts.iter().enumerate() {
                if (q.x - cx).abs() > inner / 2.0 || (q.y - cy).abs() > inner / 2.0 {
                    continue;
                }
                m[slot(reference[k])][slot(pred[k])] += 1;
            }
            if let (Ok(mut t), Ok(mut r)) = (total.lock(), rows.lock()) {
                for i in 0..n {
                    for j in 0..n {
                        t[i][j] += m[i][j];
                    }
                }
                let name = std::path::Path::new(f)
                    .file_stem()
                    .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                r.push((name, m));
            }
        });
    });
    let report = |name: &str, m: &[Vec<u64>]| -> (f64, f64) {
        let all: u64 = m.iter().flatten().sum();
        let ok: u64 = (0..n).map(|i| m[i][i]).sum();
        let mut f1s = Vec::new();
        let mut cells = Vec::new();
        for (i, c) in CLASSES.iter().enumerate() {
            let tp = m[i][i] as f64;
            let rsum: f64 = m[i].iter().sum::<u64>() as f64;
            let psum: f64 = (0..n).map(|k| m[k][i]).sum::<u64>() as f64;
            let (p, r) = (tp / psum.max(1.0), tp / rsum.max(1.0));
            let f1 = if p + r > 0.0 {
                2.0 * p * r / (p + r)
            } else {
                0.0
            };
            cells.push(format!("{c}:{:.0}/{:.0}", 100.0 * p, 100.0 * r));
            if rsum / all.max(1) as f64 >= 0.002
                && [2u8, 3, 4, 5, 6, 9, 11, 12, 14, 15, 17].contains(c)
            {
                f1s.push(f1);
            }
        }
        let macro_f1 = f1s.iter().sum::<f64>() / f1s.len().max(1) as f64;
        let acc = ok as f64 / all.max(1) as f64;
        println!(
            "| {name} | {:.1} | {:.3} | {} |",
            100.0 * acc,
            macro_f1,
            cells.join("  ")
        );
        (acc, macro_f1)
    };
    println!("| window | accuracy % | macro F1 (classes present) | precision/recall % by class |");
    println!("|---|---|---|---|");
    if let Ok(mut r) = rows.lock() {
        r.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, m) in r.iter() {
            report(name, m);
        }
    }
    if let Ok(t) = total.lock() {
        report("**pooled**", &t);
        println!();
        println!("confusion (rows: file, columns: recipe; classes {CLASSES:?} and other):");
        for (i, row) in t.iter().enumerate() {
            let name = CLASSES
                .get(i)
                .map_or("other".to_string(), ToString::to_string);
            println!(
                "{name:>6}: {}",
                row.iter()
                    .map(|v| format!("{v:>9}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }
    Ok(())
}
