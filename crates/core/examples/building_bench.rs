//! Building classification against a reference classification: `building_bench FILE... [--ground
//! classes|smrf] [--size 0]`. Takes the whole file (or a square window of `--size` m around its
//! middle), uses the file's own class 2 as ground (`classes`) or SMRF's, runs the building step and
//! compares what it calls building (class 6) with the class 6 of the file.
//!
//! `--center x,y --size M` takes a window around a point; `--dump FILE` writes its points with the
//! reference and the new class, to draw them.
//!
//! The reference is whatever class 6 the file carries (for PNOA sheets, the producer's
//! classification): the numbers measure agreement with it, not truth. Points of class 12 (overlap
//! between flight lines) and noise are left out.

use std::time::Instant;

use hipso_core::buildings::{self, BuildingParams};
use hipso_core::las::{Points, Reader};
use hipso_core::smrf::{self, SmrfParams};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut files = Vec::new();
    let mut size = 0.0_f64;
    let mut ground_kind = "classes".to_string();
    let mut dump: Option<String> = None;
    let mut center: Option<(f64, f64)> = None;
    let mut params = BuildingParams::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => size = it.next().ok_or("--size needs a value")?.parse()?,
            "--ground" => ground_kind = it.next().ok_or("--ground needs a value")?.clone(),
            "--offset" => params.offset = it.next().ok_or("--offset needs a value")?.parse()?,
            "--window" => params.window = it.next().ok_or("--window needs a value")?.parse()?,
            "--area" => params.min_area = it.next().ok_or("--area needs a value")?.parse()?,
            "--seeds" => {
                params.seeds_per_cell = it.next().ok_or("--seeds needs a value")?.parse()?
            }
            "--eaves" => params.eaves = it.next().ok_or("--eaves needs a value")?.parse()?,
            "--seed-planarity" => {
                params.seed_planarity =
                    it.next().ok_or("--seed-planarity needs a value")?.parse()?
            }
            "--radius" => params.radius = it.next().ok_or("--radius needs a value")?.parse()?,
            "--penetrating" => {
                params.penetrating = it.next().ok_or("--penetrating needs a value")?.parse()?
            }
            "--roof-depth" => {
                params.roof_depth = it.next().ok_or("--roof-depth needs a value")?.parse()?
            }
            "--sharp" => params.sharp = it.next().ok_or("--sharp needs a value")?.parse()?,
            "--dump" => dump = Some(it.next().ok_or("--dump needs a file")?.clone()),
            "--center" => {
                let v: Vec<f64> = it
                    .next()
                    .ok_or("--center needs x,y")?
                    .split(',')
                    .filter_map(|t| t.parse().ok())
                    .collect();
                if let [x, y] = v[..] {
                    center = Some((x, y));
                }
            }
            "--planar" => params.planar = it.next().ok_or("--planar needs a value")?.parse()?,
            f => files.push(f.to_string()),
        }
    }
    if files.is_empty() {
        return Err("usage: building_bench FILE... [--ground classes|smrf] [--size M]".into());
    }
    println!(
        "| file | points | ref % | precision % | recall % | F1 % | false building from ref class | missed building to class | s |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    let (mut tp_all, mut fp_all, mut fn_all) = (0u64, 0u64, 0u64);
    for f in &files {
        let r = Reader::open(std::path::Path::new(f))?;
        let hb = r.header.bounds;
        let (cx, cy) = center.unwrap_or(((hb[0] + hb[3]) / 2.0, (hb[1] + hb[4]) / 2.0));
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
        let w = Points::new(pts);
        let ground: Vec<bool> = if ground_kind == "smrf" {
            smrf::ground(&w.pts, &SmrfParams::default())
        } else {
            w.pts
                .iter()
                .map(|q| q.class == 2 && !q.is_noise())
                .collect()
        };
        let t = Instant::now();
        let out = buildings::classify(&w.pts, &ground, &params);
        let secs = t.elapsed().as_secs_f64();
        let (mut tp, mut fp, mut fneg, mut n, mut nref) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let mut fp_from = [0u64; 256];
        let mut fn_to = [0u64; 256];
        for (k, q) in w.pts.iter().enumerate() {
            if q.is_noise() || q.class == 12 {
                continue;
            }
            n += 1;
            let truth = q.class == 6;
            let pred = out[k] == Some(buildings::BUILDING);
            nref += u64::from(truth);
            match (truth, pred) {
                (true, true) => tp += 1,
                (false, true) => {
                    fp += 1;
                    fp_from[usize::from(q.class)] += 1;
                }
                (true, false) => {
                    fneg += 1;
                    // what the step made of it: another class, or left as it was
                    let to = match out[k] {
                        Some(c) => c,
                        None => {
                            if ground[k] {
                                2
                            } else {
                                1
                            }
                        }
                    };
                    fn_to[usize::from(to)] += 1;
                }
                (false, false) => {}
            }
        }
        if let Some(path) = &dump {
            use std::fmt::Write as _;
            let mut text = String::from("x y z ref pred ground\n");
            for (k, q) in w.pts.iter().enumerate() {
                let pred = out[k].unwrap_or(if ground[k] { 2 } else { 1 });
                let _ = writeln!(
                    text,
                    "{:.2} {:.2} {:.2} {} {} {}",
                    q.x,
                    q.y,
                    q.z,
                    q.class,
                    pred,
                    u8::from(ground[k])
                );
            }
            std::fs::write(path, text)?;
        }
        tp_all += tp;
        fp_all += fp;
        fn_all += fneg;
        let top = |h: &[u64; 256], total: u64| -> String {
            let mut v: Vec<(usize, u64)> = h
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, c)| *c > 0)
                .collect();
            v.sort_by_key(|a| std::cmp::Reverse(a.1));
            v.iter()
                .take(3)
                .map(|(c, k)| format!("c{c}:{:.0}%", 100.0 * *k as f64 / total.max(1) as f64))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let prec = 100.0 * tp as f64 / (tp + fp).max(1) as f64;
        let rec = 100.0 * tp as f64 / (tp + fneg).max(1) as f64;
        let name = std::path::Path::new(f)
            .file_stem()
            .map_or_else(|| f.clone(), |s| s.to_string_lossy().into_owned());
        println!(
            "| {name} | {n} | {:.2} | {prec:.1} | {rec:.1} | {:.1} | {} | {} | {secs:.1} |",
            100.0 * nref as f64 / n.max(1) as f64,
            2.0 * prec * rec / (prec + rec).max(1e-9),
            top(&fp_from, fp),
            top(&fn_to, fneg)
        );
    }
    if files.len() > 1 {
        let prec = 100.0 * tp_all as f64 / (tp_all + fp_all).max(1) as f64;
        let rec = 100.0 * tp_all as f64 / (tp_all + fn_all).max(1) as f64;
        println!(
            "| all (points pooled) | | | {prec:.1} | {rec:.1} | {:.1} | | | |",
            2.0 * prec * rec / (prec + rec).max(1e-9)
        );
    }
    Ok(())
}
