//! Ground filter benchmark against a reference classification: `ground_bench FILE... [--size 500]
//! [--cell 1]`. For each file, takes a square window around the middle of the cloud (the whole
//! file with `--size 0`), runs every ground filter with its default parameters and reports the
//! time, the agreement with the file's own class 2 (overall accuracy, kappa, type I and II errors)
//! and the difference between the DTMs (a TIN of each ground) in metres. `gross II` is the share
//! of the points a filter calls ground that lie more than 0.5 m above the reference ground: real
//! vegetation or structures taken for ground, as opposed to low points the reference leaves out.
//!
//! `--axelsson-flat M` sets Axelsson's tolerance to the sensor noise (default 0.10 m).
//! `--elm` takes the low noise out first (extended local minimum, 10 m cells, 3 m).
//! `--spacing S` sets the scales that depend on the point spacing (cells, windows) to S metres,
//! the same for every sample. `--isprs DIR` runs the ISPRS filter test instead: the `sampNN.txt` reference files of
//! the test (X Y Z and 0 ground / 1 not), with the average over the samples at the end.
//!
//! The reference is whatever class 2 the file carries (for PNOA sheets, the producer's
//! classification, itself not free of errors): the numbers measure agreement with it, not truth.

use std::time::Instant;

use hipso_core::axelsson::{self, AxelssonParams};
use hipso_core::csf::{self, CsfParams};
use hipso_core::grid::Grid;
use hipso_core::ground::{GroundFilter, GroundParams};
use hipso_core::las::{Point, Points, Reader};
use hipso_core::mcc::{self, MccParams};
use hipso_core::pmf::{self, PmfParams};
use hipso_core::smrf::{self, SmrfParams};
use hipso_core::tin;

type Filter = (
    &'static str,
    Box<dyn Fn(&Points) -> hipso_core::Result<Vec<bool>>>,
);

/// The filters, with their default parameters or (`spacing`: the point spacing in metres) with the
/// scales that depend on it set to it, the same for every sample: no tuning to any one.
fn filters(spacing: Option<f64>, axelsson_flat: Option<f64>) -> Vec<Filter> {
    let sp = spacing.unwrap_or(0.0);
    let cell = |d: f64| if sp > 0.0 { sp } else { d };
    let gp = GroundParams {
        filter_cell: if sp > 0.0 { 5.0 * sp } else { 5.0 },
        ..GroundParams::default()
    };
    let mp = MccParams {
        spacing: cell(1.5),
        ..MccParams::default()
    };
    let cp = CsfParams {
        resolution: cell(0.5),
        rigidness: if sp > 0.0 { 2 } else { 1 },
        ..CsfParams::default()
    };
    let pp = PmfParams {
        cell: cell(1.0),
        ..PmfParams::default()
    };
    let sm = SmrfParams {
        cell: cell(1.0),
        ..SmrfParams::default()
    };
    let ap = AxelssonParams {
        seed_cell: if sp > 0.0 { 15.0 * sp } else { 20.0 },
        flat_tolerance: axelsson_flat.unwrap_or(0.10),

        ..AxelssonParams::default()
    };
    vec![
        (
            "fusion (Kraus-Pfeifer)",
            Box::new(move |w: &Points| {
                let (f, _) = GroundFilter::run(w, &gp)?;
                Ok(w.pts
                    .iter()
                    .map(|q| !q.is_noise() && f.is_ground(q))
                    .collect())
            }),
        ),
        (
            "mcc",
            Box::new(move |w: &Points| Ok(mcc::ground(&w.pts, &mp))),
        ),
        ("csf", Box::new(move |w: &Points| csf::ground(&w.pts, &cp))),
        (
            "pmf",
            Box::new(move |w: &Points| Ok(pmf::ground(&w.pts, &pp))),
        ),
        (
            "smrf",
            Box::new(move |w: &Points| Ok(smrf::ground(&w.pts, &sm))),
        ),
        (
            "axelsson (ptin)",
            Box::new(move |w: &Points| axelsson::ground(&w.pts, &ap)),
        ),
    ]
}

fn dtm(pts: &[Point], ground: &[bool], b: [f64; 4], cell: f64) -> Option<Grid> {
    let g: Vec<[f64; 3]> = pts
        .iter()
        .zip(ground)
        .filter(|(q, g)| **g && !q.is_noise())
        .map(|(q, _)| [q.x, q.y, q.z])
        .collect();
    if g.len() < 3 {
        return None;
    }
    let mut grid = Grid::covering(b, cell).ok()?;
    tin::rasterize(&g, &mut grid);
    Some(grid)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut files = Vec::new();
    let (mut size, mut cell) = (500.0_f64, 1.0_f64);
    let mut isprs: Option<String> = None;
    let mut use_elm = false;
    let mut axelsson_flat: Option<f64> = None;
    let mut spacing: Option<f64> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => size = it.next().ok_or("--size needs a value")?.parse()?,
            "--cell" => cell = it.next().ok_or("--cell needs a value")?.parse()?,
            "--spacing" => spacing = Some(it.next().ok_or("--spacing needs a value")?.parse()?),
            "--axelsson-flat" => axelsson_flat = Some(it.next().ok_or("needs a value")?.parse()?),
            "--elm" => use_elm = true,
            "--isprs" => isprs = Some(it.next().ok_or("--isprs needs a folder")?.clone()),
            f => files.push(f.to_string()),
        }
    }
    if files.is_empty() && isprs.is_none() {
        return Err("usage: ground_bench FILE... [--size 500] [--cell 1] | --isprs DIR".into());
    }
    let mut samples: Vec<(String, Points)> = Vec::new();
    if let Some(dir) = &isprs {
        let mut names: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("samp") && n.ends_with(".txt"))
            })
            .collect();
        names.sort();
        for path in names {
            let mut pts = Vec::new();
            for line in std::fs::read_to_string(&path)?.lines() {
                let v: Vec<f64> = line
                    .split_whitespace()
                    .filter_map(|t| t.parse().ok())
                    .collect();
                if let [x, y, z, label] = v[..] {
                    pts.push(Point {
                        x,
                        y,
                        z,
                        class: if label == 0.0 { 2 } else { 1 },
                        return_number: 1,
                        number_of_returns: 1,
                        ..Default::default()
                    });
                }
            }
            let name = path
                .file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
            samples.push((name, Points::new(pts)));
        }
    }
    for f in &files {
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
        let name = std::path::Path::new(f)
            .file_stem()
            .map_or_else(|| f.clone(), |s| s.to_string_lossy().into_owned());
        samples.push((name, Points::new(pts)));
    }
    let mut totals: Vec<[f64; 6]> = vec![[0.0; 6]; filters(spacing, axelsson_flat).len()];
    let n_samples = samples.len();
    let list = filters(spacing, axelsson_flat);
    println!(
        "| file | filter | s | OA % | kappa | type I % | type II % | gross II % | DTM bias m | DTM RMSE m | DTM MAE m |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for (name, w) in &samples {
        let bb = [w.bounds[0], w.bounds[1], w.bounds[3], w.bounds[4]];
        let refg: Vec<bool> = w
            .pts
            .iter()
            .map(|q| q.class == 2 && !q.is_noise())
            .collect();
        // class 12 (overlap between flight lines) carries no ground reference: left out
        let valid: Vec<bool> = w
            .pts
            .iter()
            .map(|q| !q.is_noise() && q.class != 12)
            .collect();
        let ref_dtm = dtm(&w.pts, &refg, bb, cell);
        eprintln!(
            "{name}: {} points, {:.0} x {:.0} m, reference ground {:.1} %",
            w.pts.len(),
            bb[2] - bb[0],
            bb[3] - bb[1],
            100.0 * refg.iter().filter(|g| **g).count() as f64
                / valid.iter().filter(|v| **v).count().max(1) as f64
        );
        for (fi, (label, run)) in list.iter().enumerate() {
            let t = Instant::now();
            let pred = if use_elm {
                // low noise out first (class 7: the filters leave it out), as the pipeline can
                let flags = hipso_core::noise::elm(&w.pts, 10.0, 3.0);
                let mut clean = Points::new(w.pts.clone());
                for (q, f) in clean.pts.iter_mut().zip(&flags) {
                    if *f {
                        q.class = 7;
                    }
                }
                run(&clean)?
                    .into_iter()
                    .zip(&flags)
                    .map(|(g, f)| g && !f)
                    .collect()
            } else {
                run(w)?
            };
            let secs = t.elapsed().as_secs_f64();
            let (mut tp, mut tn, mut fp, mut fneg) = (0u64, 0u64, 0u64, 0u64);
            let mut fp_by_class = [0u64; 256];
            let (mut gross, mut pred_n) = (0u64, 0u64);
            for k in 0..w.pts.len() {
                if !valid[k] {
                    continue;
                }
                if pred[k] {
                    pred_n += 1;
                    let q = &w.pts[k];
                    if let Some(h) = ref_dtm.as_ref().and_then(|d| d.sample(q.x, q.y))
                        && q.z - f64::from(h) > 0.5
                    {
                        gross += 1;
                    }
                }
                match (refg[k], pred[k]) {
                    (true, true) => tp += 1,
                    (false, false) => tn += 1,
                    (false, true) => {
                        fp += 1;
                        fp_by_class[usize::from(w.pts[k].class)] += 1;
                    }
                    (true, false) => fneg += 1,
                }
            }
            let n = (tp + tn + fp + fneg) as f64;
            let oa = (tp + tn) as f64 / n;
            let pe = ((tp + fneg) as f64 * (tp + fp) as f64
                + (tn + fp) as f64 * (tn + fneg) as f64)
                / (n * n);
            let kappa = (oa - pe) / (1.0 - pe);
            let t1 = fneg as f64 / (tp + fneg).max(1) as f64;
            let t2 = fp as f64 / (tn + fp).max(1) as f64;
            let (mut bias, mut rmse, mut mae, mut m) = (0.0, 0.0, 0.0, 0u64);
            if let (Some(a), Some(b)) = (&ref_dtm, dtm(&w.pts, &pred, bb, cell)) {
                for (x, y) in a.data.iter().zip(&b.data) {
                    if !x.is_nan() && !y.is_nan() {
                        let d = f64::from(*y - *x);
                        bias += d;
                        rmse += d * d;
                        mae += d.abs();
                        m += 1;
                    }
                }
            }
            let mf = m.max(1) as f64;
            let t = &mut totals[fi];
            t[0] += secs;
            t[1] += 100.0 * oa;
            t[2] += kappa;
            t[3] += 100.0 * t1;
            t[4] += 100.0 * t2;
            t[5] += (rmse / mf).sqrt();
            println!(
                "| {name} | {label} | {secs:.2} | {:.2} | {kappa:.3} | {:.2} | {:.2} | {:.2} | {:+.3} | {:.3} | {:.3} |",
                100.0 * oa,
                100.0 * t1,
                100.0 * t2,
                100.0 * gross as f64 / pred_n.max(1) as f64,
                bias / mf,
                (rmse / mf).sqrt(),
                mae / mf
            );
            let top: Vec<String> = {
                let mut v: Vec<(usize, u64)> = fp_by_class
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|(_, c)| *c > 0)
                    .collect();
                v.sort_by(|a, b| b.1.cmp(&a.1));
                v.iter()
                    .take(4)
                    .map(|(c, k)| format!("c{c}:{:.0}%", 100.0 * *k as f64 / fp.max(1) as f64))
                    .collect()
            };
            eprintln!(
                "    {label}: false ground by reference class {}",
                top.join(" ")
            );
        }
    }
    if n_samples > 1 {
        let k = n_samples as f64;
        println!();
        println!(
            "| average of {n_samples} | s | OA % | kappa | type I % | type II % | DTM RMSE m |"
        );
        println!("|---|---|---|---|---|---|---|");
        for ((label, _), t) in list.iter().zip(&totals) {
            println!(
                "| {label} | {:.2} | {:.2} | {:.3} | {:.2} | {:.2} | {:.3} |",
                t[0] / k,
                t[1] / k,
                t[2] / k,
                t[3] / k,
                t[4] / k,
                t[5] / k
            );
        }
    }
    Ok(())
}
