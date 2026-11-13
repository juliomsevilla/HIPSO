//! Audit of the ground filters on synthetic scenes whose ground is known: `ground_audit [--size 200]`.
//!
//! For each filter (default parameters) and scene, prints the ground missed (type I) and the
//! non-ground taken (type II), the type I along the edge of the cloud (a strip of 10 m) against the
//! interior, and how many points change class when the same cloud is moved to UTM-sized
//! coordinates or its points are shuffled: a filter must not depend on where the cloud is or on
//! the order of its points.

use hipso_core::axelsson::{self, AxelssonParams};
use hipso_core::buildings::{self, BuildingParams};
use hipso_core::csf::{self, CsfParams};
use hipso_core::ground::{GroundFilter, GroundParams};
use hipso_core::las::{Point, Points};
use hipso_core::mcc::{self, MccParams};
use hipso_core::pmf::{self, PmfParams};
use hipso_core::smrf::{self, SmrfParams};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / f64::from(1u32 << 31)
    }
    fn gauss(&mut self) -> f64 {
        (0..12).map(|_| self.next()).sum::<f64>() - 6.0
    }
}

/// A scene: points and whether each is ground.
fn scene(kind: &str, size: f64, step: f64) -> (Vec<Point>, Vec<bool>) {
    let mut rng = Rng(42);
    let terrain: Box<dyn Fn(f64, f64) -> f64> = match kind {
        "flat" => Box::new(|_, _| 100.0),
        "slope25" => Box::new(|x, _| 100.0 + 25f64.to_radians().tan() * x),
        "hills" | "city" | "forest" => {
            Box::new(|x, y| 100.0 + 6.0 * (x / 30.0).sin() + 4.0 * (y / 25.0).cos())
        }
        _ => Box::new(|_, _| 100.0),
    };
    let mut pts = Vec::new();
    let mut truth = Vec::new();
    let mut push = |x: f64, y: f64, z: f64, g: bool, pts: &mut Vec<Point>| {
        pts.push(Point {
            x,
            y,
            z,
            return_number: 1,
            number_of_returns: 1,
            class: 1,
            ..Default::default()
        });
        truth.push(g);
    };
    let n = (size / step) as usize;
    for i in 0..=n {
        for j in 0..=n {
            // a jittered lattice, as a scanner gives
            let x = i as f64 * step + (rng.next() - 0.5) * step * 0.8;
            let y = j as f64 * step + (rng.next() - 0.5) * step * 0.8;
            let g = terrain(x, y) + rng.gauss() * 0.03;
            match kind {
                "city" => {
                    let b1 = (60.0..75.0).contains(&x) && (60.0..72.0).contains(&y);
                    let b2 = (120.0..135.0).contains(&x) && (30.0..45.0).contains(&y);
                    if b1 || b2 {
                        push(
                            x,
                            y,
                            terrain(x, y) + 7.0 + rng.gauss() * 0.03,
                            false,
                            &mut pts,
                        );
                    } else {
                        push(x, y, g, true, &mut pts);
                    }
                }
                "forest" => {
                    // crowns every ~8 m: 70 % of the cells under a tree give a canopy hit
                    let (cx, cy) = ((x / 8.0).round() * 8.0, (y / 8.0).round() * 8.0);
                    let under = (x - cx).hypot(y - cy) < 3.0;
                    if under && rng.next() < 0.7 {
                        let h = 10.0 + 4.0 * rng.next();
                        push(x, y, terrain(x, y) + h, false, &mut pts);
                    } else {
                        push(x, y, g, true, &mut pts);
                    }
                }
                _ => push(x, y, g, true, &mut pts),
            }
        }
    }
    (pts, truth)
}

type Filter = (
    &'static str,
    Box<dyn Fn(&Points) -> hipso_core::Result<Vec<bool>>>,
);

fn filters() -> Vec<Filter> {
    vec![
        (
            "fusion",
            Box::new(|w: &Points| {
                let (f, _) = GroundFilter::run(w, &GroundParams::default())?;
                Ok(w.pts.iter().map(|q| f.is_ground(q)).collect())
            }),
        ),
        (
            "mcc",
            Box::new(|w: &Points| Ok(mcc::ground(&w.pts, &MccParams::default()))),
        ),
        (
            "csf",
            Box::new(|w: &Points| csf::ground(&w.pts, &CsfParams::default())),
        ),
        (
            "pmf",
            Box::new(|w: &Points| Ok(pmf::ground(&w.pts, &PmfParams::default()))),
        ),
        (
            "smrf",
            Box::new(|w: &Points| Ok(smrf::ground(&w.pts, &SmrfParams::default()))),
        ),
        (
            "axelsson",
            Box::new(|w: &Points| axelsson::ground(&w.pts, &AxelssonParams::default())),
        ),
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut size = 200.0;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(p) = args.iter().position(|a| a == "--size") {
        size = args.get(p + 1).ok_or("--size needs a value")?.parse()?;
    }
    let (ux, uy) = (512_345.0, 4_410_678.0); // UTM-sized offsets
    let dump = args
        .iter()
        .position(|a| a == "--dump")
        .and_then(|p| args.get(p + 1))
        .cloned();
    println!(
        "| scene | filter | type I % | type II % | type I edge % | type I interior % | moved to UTM: changed % | shuffled: changed % |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for kind in ["flat", "slope25", "hills", "city", "forest"] {
        let (pts, truth) = scene(kind, size, 0.7);
        let w = Points::new(pts.clone());
        let moved = Points::new(
            pts.iter()
                .map(|q| Point {
                    x: q.x + ux,
                    y: q.y + uy,
                    ..*q
                })
                .collect(),
        );
        // the same points in another order
        let mut order: Vec<usize> = (0..pts.len()).collect();
        let mut rng = Rng(7);
        for i in (1..order.len()).rev() {
            order.swap(i, (rng.next() * (i + 1) as f64) as usize % (i + 1));
        }
        let shuffled = Points::new(order.iter().map(|&k| pts[k]).collect());
        let mut preds: Vec<(String, Vec<bool>)> = Vec::new();
        for (name, run) in filters() {
            let g = run(&w)?;
            preds.push((name.to_string(), g.clone()));
            let (mut m, mut gt, mut t, mut nt) = (0u64, 0u64, 0u64, 0u64);
            let (mut em, mut eg, mut im, mut ig) = (0u64, 0u64, 0u64, 0u64);
            for (k, q) in pts.iter().enumerate() {
                let edge = q.x < 10.0 || q.y < 10.0 || q.x > size - 10.0 || q.y > size - 10.0;
                if truth[k] {
                    gt += 1;
                    m += u64::from(!g[k]);
                    if edge {
                        eg += 1;
                        em += u64::from(!g[k]);
                    } else {
                        ig += 1;
                        im += u64::from(!g[k]);
                    }
                } else {
                    nt += 1;
                    t += u64::from(g[k]);
                }
            }
            let gm = run(&moved)?;
            let diff_moved = g.iter().zip(&gm).filter(|(a, b)| a != b).count();
            let gs = run(&shuffled)?;
            let mut back = vec![false; g.len()];
            for (pos, &k) in order.iter().enumerate() {
                back[k] = gs[pos];
            }
            let diff_shuffled = g.iter().zip(&back).filter(|(a, b)| a != b).count();
            println!(
                "| {kind} | {name} | {:.2} | {:.2} | {:.2} | {:.2} | {:.3} | {:.3} |",
                100.0 * m as f64 / gt.max(1) as f64,
                100.0 * t as f64 / nt.max(1) as f64,
                100.0 * em as f64 / eg.max(1) as f64,
                100.0 * im as f64 / ig.max(1) as f64,
                100.0 * diff_moved as f64 / g.len() as f64,
                100.0 * diff_shuffled as f64 / g.len() as f64,
            );
        }
        if let Some(dir) = &dump {
            use std::fmt::Write as _;
            let mut text = String::new();
            for (k, q) in pts.iter().enumerate() {
                let _ = write!(
                    text,
                    "{:.3} {:.3} {:.3} {}",
                    q.x,
                    q.y,
                    q.z,
                    u8::from(truth[k])
                );
                for (_, g) in &preds {
                    let _ = write!(text, " {}", u8::from(g[k]));
                }
                text.push('\n');
            }
            std::fs::write(format!("{dir}/scene_{kind}.txt"), text)?;
        }
    }
    // Buildings: on the cities, the share of roof points called building; on the forests, the share
    // of crown points called building (it should be none). With the true ground and with SMRF's.
    println!();
    println!(
        "| buildings | ground from | roofs/crowns called building % | ground called building % |"
    );
    println!("|---|---|---|---|");
    for kind in ["city", "forest"] {
        let (pts, truth) = scene(kind, size, 0.7);
        let w = Points::new(pts.clone());
        let smrf_ground = smrf::ground(&w.pts, &SmrfParams::default());
        for (label, ground) in [("truth", truth.clone()), ("smrf", smrf_ground)] {
            let out = buildings::classify(&pts, &ground, &BuildingParams::default());
            let (mut hit, mut n, mut bad, mut gn) = (0u64, 0u64, 0u64, 0u64);
            let (mut ebad, mut egn) = (0u64, 0u64);
            for (k, c) in out.iter().enumerate() {
                let q = &pts[k];
                let edge = q.x < 10.0 || q.y < 10.0 || q.x > size - 10.0 || q.y > size - 10.0;
                if truth[k] && edge {
                    egn += 1;
                    ebad += u64::from(*c == Some(buildings::BUILDING));
                }
                if truth[k] {
                    gn += 1;
                    bad += u64::from(*c == Some(buildings::BUILDING));
                } else {
                    n += 1;
                    hit += u64::from(*c == Some(buildings::BUILDING));
                }
            }
            if std::env::var("HIPSO_DIAG").is_ok() && kind == "forest" && label == "truth" {
                // where the ground called building lies, by distance to the nearest tree centre
                let mut hist = [0u64; 8];
                let mut inter = 0u64;
                for (k, c) in out.iter().enumerate() {
                    if truth[k] && *c == Some(buildings::BUILDING) {
                        let q = &pts[k];
                        let (cx, cy) = ((q.x / 8.0).round() * 8.0, (q.y / 8.0).round() * 8.0);
                        hist[((q.x - cx).hypot(q.y - cy).floor() as usize).min(7)] += 1;
                        inter += u64::from(
                            q.x > 20.0 && q.y > 20.0 && q.x < size - 20.0 && q.y < size - 20.0,
                        );
                    }
                }
                eprintln!(
                    "DIAG ground called building by distance to tree centre (m): {hist:?}; deep interior: {inter}"
                );
            }
            println!(
                "| {kind} | {label} | {:.1} | {:.2} (edge strip {:.2}) |",
                100.0 * hit as f64 / n.max(1) as f64,
                100.0 * bad as f64 / gn.max(1) as f64,
                100.0 * ebad as f64 / egn.max(1) as f64
            );
        }
    }
    Ok(())
}
