//! Baseline of the road extraction on synthetic scenes of known geometry (level 1) and of its
//! stability under changes that should not matter (level 2).
//!
//!     cargo run --release -p hipso-core --example roads_bench -- benchmarks/roads
//!
//! The scenes are generated from a fixed seed, so every run gives the same clouds and the same
//! numbers. Writes `baseline.json` and `baseline.md` into the folder given. No success threshold
//! is applied here: thresholds are set from the baseline (see benchmarks/roads/README.md).

use hipso_core::las::Point;
use hipso_core::roads::{RoadLine, RoadParams, extract};

const SEED: u64 = 20_261_102;
const SIZE: f64 = 200.0;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut z = self.0;
        z ^= z >> 33;
        z = z.wrapping_mul(0xff51afd7ed558ccd);
        z ^= z >> 33;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
    fn normal(&mut self) -> f64 {
        let (a, b) = (self.next().max(1e-12), self.next());
        (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }
}

#[derive(Clone)]
struct Case {
    name: String,
    width: f64,
    slope_pct: f64,
    density: f64,
    noise: f64,
}

/// Truth: centre lines as dense polylines (2 m steps).
fn roads() -> Vec<Vec<[f64; 2]>> {
    // a winding east-west road and a straight north-south one that crosses it
    let a: Vec<[f64; 2]> = (0..=95).map(|i| {
        let x = 5.0 + f64::from(i) * 2.0;
        [x, 100.0 + 18.0 * (x / 38.0).sin()]
    }).collect();
    let b: Vec<[f64; 2]> = (0..=95).map(|i| [118.0 - 0.1 * f64::from(i), 5.0 + f64::from(i) * 2.0]).collect();
    vec![a, b]
}

fn dist_to_polyline(p: [f64; 2], l: &[[f64; 2]]) -> f64 {
    let mut best = f64::INFINITY;
    for w in l.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
        best = best.min((p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy));
    }
    best
}

fn generate(c: &Case, truth: &[Vec<[f64; 2]>], rng: &mut Rng) -> Vec<Point> {
    let n = (c.density * SIZE * SIZE) as usize;
    let slope = c.slope_pct / 100.0;
    let ang = 0.6f64;
    // crowns along the road edges: (x, y, radius)
    let mut crowns = Vec::new();
    for l in truth {
        let mut s = 8.0;
        while s < (l.len() - 1) as f64 * 2.0 {
            let i = (s / 2.0) as usize;
            let (p, q) = (l[i], l[(i + 1).min(l.len() - 1)]);
            let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
            let len = dx.hypot(dy).max(1e-9);
            let side = if rng.next() < 0.5 { 1.0 } else { -1.0 };
            let off = c.width / 2.0 + 1.0;
            crowns.push((p[0] - dy / len * off * side, p[1] + dx / len * off * side, 3.0 + 1.5 * rng.next()));
            s += 14.0 + 20.0 * rng.next();
        }
    }
    let mut pts = Vec::with_capacity(n);
    for _ in 0..n {
        let (x, y) = (rng.next() * SIZE, rng.next() * SIZE);
        let d = truth.iter().map(|l| dist_to_polyline([x, y], l)).fold(f64::INFINITY, f64::min);
        let ground = slope * (x * ang.cos() + y * ang.sin()) + 1.5 * (x / 30.0).sin() * slope.min(0.05) * 10.0;
        let on_road = d <= c.width / 2.0;
        let in_ditch = !on_road && d <= c.width / 2.0 + 0.8;
        let under_crown = crowns.iter().any(|k| (x - k.0).hypot(y - k.1) <= k.2);
        let (class, z, intensity) = if on_road && !(under_crown && rng.next() < 0.7) {
            (2, ground + c.noise * rng.normal(), 120.0 + 10.0 * rng.normal())
        } else if in_ditch && rng.next() < 0.8 {
            (2, ground - 0.3 + c.noise * rng.normal(), 80.0 + 10.0 * rng.normal())
        } else if !on_road && !in_ditch && rng.next() < 0.12 {
            (2, ground + c.noise * rng.normal(), 40.0 + 10.0 * rng.normal())
        } else {
            (5, ground + 4.0 + 18.0 * rng.next(), 60.0 + 10.0 * rng.normal())
        };
        pts.push(Point {
            x,
            y,
            z,
            class,
            intensity: intensity.clamp(0.0, 65535.0) as u16,
            return_number: 1,
            number_of_returns: 1,
            ..Default::default()
        });
    }
    pts
}

fn sample(l: &[[f64; 2]], step: f64) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    let mut carry = 0.0;
    for w in l.windows(2) {
        let len = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        let mut t = carry;
        while t < len {
            let f = t / len;
            out.push([w[0][0] + f * (w[1][0] - w[0][0]), w[0][1] + f * (w[1][1] - w[0][1])]);
            t += step;
        }
        carry = t - len;
    }
    out
}

#[derive(Clone, Debug, Default)]
struct Metrics {
    width_error: f64,
    hausdorff: f64,
    deviation_mean: f64,
    recall_5m: f64,
    precision_5m: f64,
    missed: usize,
    invented: usize,
    lines: usize,
}

fn evaluate(truth: &[Vec<[f64; 2]>], width: f64, found: &[RoadLine]) -> Metrics {
    let lines: Vec<Vec<[f64; 2]>> = found.iter().map(|l| l.points.clone()).collect();
    let near_found = |p: [f64; 2]| lines.iter().map(|l| dist_to_polyline(p, l)).fold(f64::INFINITY, f64::min);
    let near_truth = |p: [f64; 2]| truth.iter().map(|l| dist_to_polyline(p, l)).fold(f64::INFINITY, f64::min);
    let (mut ts, mut tm) = (0usize, 0usize);
    let mut missed = 0;
    for l in truth {
        let s = sample(l, 1.0);
        let hit = s.iter().filter(|p| near_found(**p) <= 5.0).count();
        ts += s.len();
        tm += hit;
        if (hit as f64) < 0.5 * s.len() as f64 {
            missed += 1;
        }
    }
    let (mut es, mut em) = (0usize, 0usize);
    let (mut dev_sum, mut dev_max, mut wsum) = (0.0, 0.0f64, 0.0);
    let mut invented = 0;
    for (l, f) in lines.iter().zip(found) {
        let s = sample(l, 1.0);
        let mut hit = 0;
        for p in &s {
            let d = near_truth(*p);
            if d <= 5.0 {
                hit += 1;
                dev_sum += d;
                dev_max = dev_max.max(d);
                wsum += (f.width - width).abs();
            }
        }
        es += s.len();
        em += hit;
        if (hit as f64) < 0.5 * s.len() as f64 {
            invented += 1;
        }
    }
    // Hausdorff: also the truth samples with no road within reach count at their distance
    let mut h = dev_max;
    for l in truth {
        for p in sample(l, 1.0) {
            let d = near_found(p);
            if d.is_finite() {
                h = h.max(d);
            } else {
                h = f64::INFINITY;
            }
        }
    }
    Metrics {
        width_error: if em > 0 { wsum / em as f64 } else { f64::NAN },
        hausdorff: h,
        deviation_mean: if em > 0 { dev_sum / em as f64 } else { f64::NAN },
        recall_5m: 100.0 * tm as f64 / ts.max(1) as f64,
        precision_5m: if es > 0 { 100.0 * em as f64 / es as f64 } else { 0.0 },
        missed,
        invented,
        lines: found.len(),
    }
}

fn fmt(v: f64) -> String {
    if v.is_finite() { format!("{v:.3}") } else { "null".into() }
}

fn json(m: &Metrics) -> String {
    format!(
        "{{\"width_error_m\":{},\"hausdorff_m\":{},\"mean_deviation_m\":{},\"recall_5m_pct\":{},\"precision_5m_pct\":{},\"missed\":{},\"invented\":{},\"lines\":{}}}",
        fmt(m.width_error), fmt(m.hausdorff), fmt(m.deviation_mean), fmt(m.recall_5m), fmt(m.precision_5m), m.missed, m.invented, m.lines
    )
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "benchmarks/roads".into()));
    std::fs::create_dir_all(&out).expect("folder");
    let truth = roads();
    let params = RoadParams::default();
    let mut json_cases = Vec::new();
    let mut md = String::from("# Road extraction baseline (level 1)\n\nSynthetic scenes, 200 x 200 m: a winding road and a straight one that cross, with ditches and crowns that overhang the edges. Seed 20261102. Default parameters.\n\n| case | width m | slope % | pts/m² | noise m | lines | width err m | Hausdorff m | mean dev m | recall 5 m % | precision 5 m % | missed | invented |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    let mut rng = Rng(SEED);
    let mut cases = Vec::new();
    for &w in &[3.0, 5.0, 8.0] {
        for &s in &[0.0, 6.0, 12.0] {
            for &d in &[1.0, 5.0] {
                for &nz in &[0.05, 0.3] {
                    cases.push(Case { name: format!("w{w}_s{s}_d{d}_n{nz}"), width: w, slope_pct: s, density: d, noise: nz });
                }
            }
        }
    }
    let mut clouds = Vec::new();
    for c in &cases {
        let pts = generate(c, &truth, &mut rng);
        let found = extract(&pts, &params).unwrap_or_default();
        let m = evaluate(&truth, c.width, &found);
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            c.name, c.width, c.slope_pct, c.density, c.noise, m.lines, fmt(m.width_error), fmt(m.hausdorff), fmt(m.deviation_mean), fmt(m.recall_5m), fmt(m.precision_5m), m.missed, m.invented
        ));
        json_cases.push(format!("{{\"case\":\"{}\",\"width_m\":{},\"slope_pct\":{},\"density\":{},\"noise_m\":{},\"metrics\":{}}}", c.name, c.width, c.slope_pct, c.density, c.noise, json(&m)));
        clouds.push(pts);
    }
    // level 2: stability of the middle case under changes that should not matter
    let k = cases.iter().position(|c| c.name == "w5_s6_d5_n0.05").expect("case");
    let (c, base) = (&cases[k], &clouds[k]);
    let mut variants: Vec<(String, Vec<Point>)> = vec![("original".into(), base.clone())];
    let mut r2 = Rng(SEED ^ 0x9e37);
    variants.push(("half the points".into(), base.iter().filter(|_| r2.next() < 0.5).copied().collect()));
    variants.push(("a fifth of the points".into(), base.iter().filter(|_| r2.next() < 0.2).copied().collect()));
    let mut shuffled = base.clone();
    for i in (1..shuffled.len()).rev() {
        let j = (r2.next() * (i + 1) as f64) as usize;
        shuffled.swap(i, j.min(i));
    }
    variants.push(("shuffled order".into(), shuffled));
    variants.push(("shifted 0.37 m, 0.61 m".into(), base.iter().map(|p| Point { x: p.x + 0.37, y: p.y + 0.61, ..*p }).collect()));
    let mut md2 = String::from("\n# Stability (level 2)\n\nCase w5_s6_d5_n0.05 under changes that should not matter.\n\n| variant | lines | width err m | Hausdorff m | mean dev m | recall 5 m % | precision 5 m % | missed | invented |\n|---|---|---|---|---|---|---|---|---|\n");
    let mut json_var = Vec::new();
    for (name, pts) in &variants {
        let found = extract(pts, &params).unwrap_or_default();
        let shift = name.starts_with("shifted");
        // the shifted cloud is compared to the shifted truth
        let t: Vec<Vec<[f64; 2]>> = if shift { truth.iter().map(|l| l.iter().map(|p| [p[0] + 0.37, p[1] + 0.61]).collect()).collect() } else { truth.clone() };
        let m = evaluate(&t, c.width, &found);
        md2.push_str(&format!("| {name} | {} | {} | {} | {} | {} | {} | {} | {} |\n", m.lines, fmt(m.width_error), fmt(m.hausdorff), fmt(m.deviation_mean), fmt(m.recall_5m), fmt(m.precision_5m), m.missed, m.invented));
        json_var.push(format!("{{\"variant\":\"{name}\",\"metrics\":{}}}", json(&m)));
    }
    // tiles: two halves with 20 m overlap, each extracted separately, lines pooled
    {
        let half = |lo: f64, hi: f64| -> Vec<Point> { base.iter().filter(|p| p.x >= lo && p.x < hi).copied().collect() };
        let mut pooled = extract(&half(0.0, 120.0), &params).unwrap_or_default();
        pooled.extend(extract(&half(100.0, 200.0), &params).unwrap_or_default());
        let m = evaluate(&truth, c.width, &pooled);
        md2.push_str(&format!("| two tiles, 20 m overlap | {} | {} | {} | {} | {} | {} | {} | {} |\n", m.lines, fmt(m.width_error), fmt(m.hausdorff), fmt(m.deviation_mean), fmt(m.recall_5m), fmt(m.precision_5m), m.missed, m.invented));
        json_var.push(format!("{{\"variant\":\"two tiles\",\"metrics\":{}}}", json(&m)));
    }
    std::fs::write(out.join("baseline.json"), format!("{{\"seed\":{SEED},\"params\":\"default\",\"cases\":[{}],\"stability\":[{}]}}\n", json_cases.join(","), json_var.join(","))).expect("json");
    md.push_str(&md2);
    std::fs::write(out.join("baseline.md"), md).expect("md");
    println!("{} cases and {} variants -> {}", cases.len(), variants.len() + 1, out.display());
}
