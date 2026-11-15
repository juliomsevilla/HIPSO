//! Qué aspecto tienen los puntos de cada clase de un archivo: `class_profile [--size 500] [--step
//! 10] ARCHIVO...`. Para cada clase imprime percentiles de intensidad, rugosidad, planaridad,
//! pendiente y ángulo de barrido, y la proporción de retornos únicos, tomando uno de cada `--step`
//! puntos (características de un entorno de 1 m de todos los puntos).

use std::collections::BTreeMap;

use hipso_core::features;
use hipso_core::las::{Points, Reader};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut files, mut size, mut step, mut radius) = (Vec::new(), 500.0_f64, 10usize, 1.0_f64);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => size = it.next().ok_or("--size needs a value")?.parse()?,
            "--step" => step = it.next().ok_or("--step needs a value")?.parse()?,
            "--radius" => radius = it.next().ok_or("--radius needs a value")?.parse()?,
            f => files.push(f.to_string()),
        }
    }
    let mut cols: BTreeMap<u8, [Vec<f32>; 6]> = BTreeMap::new();
    let mut single: BTreeMap<u8, (u64, u64)> = BTreeMap::new();
    for f in &files {
        let r = Reader::open(std::path::Path::new(f))?;
        let hb = r.header.bounds;
        let (cx, cy) = ((hb[0] + hb[3]) / 2.0, (hb[1] + hb[4]) / 2.0);
        let mut pts = Vec::new();
        r.scan(|q| {
            if (q.x - cx).abs() <= size / 2.0 && (q.y - cy).abs() <= size / 2.0 {
                pts.push(*q);
            }
        })?;
        let w = Points::new(pts);
        let mut seen: BTreeMap<u8, usize> = BTreeMap::new();
        let wanted: Vec<bool> = w
            .pts
            .iter()
            .map(|q| {
                let n = seen.entry(q.class).or_default();
                *n += 1;
                n.is_multiple_of(step)
            })
            .collect();
        let fe = features::compute(&w.pts, &wanted, radius);
        for (k, q) in w.pts.iter().enumerate() {
            if !wanted[k] {
                continue;
            }
            let s = single.entry(q.class).or_default();
            s.1 += 1;
            s.0 += u64::from(q.number_of_returns == 1);
            if let Some(ft) = fe[k] {
                let c = cols.entry(q.class).or_default();
                c[0].push(f32::from(q.intensity));
                c[1].push(ft.roughness);
                c[2].push(ft.planarity);
                c[3].push(ft.slope);
                c[4].push(q.scan_angle.abs());
                c[5].push(ft.count as f32);
            }
        }
    }
    println!(
        "| class | n | single % | intensity p10/50/90 | roughness p10/50/90 | planarity p10/50/90 | slope p10/50/90 | |angle| p50 | neighbours p50 |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for (c, mut v) in cols {
        let n = v[0].len();
        for col in &mut v {
            col.sort_by(f32::total_cmp);
        }
        let p = |i: usize, q: f64| v[i][((n - 1) as f64 * q) as usize];
        let (s, t) = single.get(&c).copied().unwrap_or((0, 1));
        println!(
            "| {c} | {n} | {:.0} | {:.0}/{:.0}/{:.0} | {:.3}/{:.3}/{:.3} | {:.2}/{:.2}/{:.2} | {:.1}/{:.1}/{:.1} | {:.1} | {:.0} |",
            100.0 * s as f64 / t.max(1) as f64,
            p(0, 0.1),
            p(0, 0.5),
            p(0, 0.9),
            p(1, 0.1),
            p(1, 0.5),
            p(1, 0.9),
            p(2, 0.1),
            p(2, 0.5),
            p(2, 0.9),
            p(3, 0.1),
            p(3, 0.5),
            p(3, 0.9),
            p(4, 0.5),
            p(5, 0.5)
        );
    }
    Ok(())
}
