//! Prueba de la varita: `wand_bench ARCHIVO [--n 20]`. Carga la nube, construye la rejilla y
//! hace crecer la región desde `n` semillas repartidas, midiendo tiempos y tamaños.

use hipso_core::las::Reader;
use hipso_core::wand::{grow, Index, WandParams};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = a.first().ok_or("FILE")?;
    let n: usize = a.get(2).map_or(Ok(20), |v| v.parse())?;
    let t = Instant::now();
    let mut pts = Vec::new();
    Reader::open(std::path::Path::new(file))?.scan(|q| pts.push(*q))?;
    println!("{} puntos leídos en {:.1?}", pts.len(), t.elapsed());
    let mut all = std::collections::BTreeMap::new();
    for q in &pts {
        *all.entry(q.class).or_insert(0usize) += 1;
    }
    println!("clases de la nube {all:?}");
    let t = Instant::now();
    let mut prm = WandParams::default();
    if a.iter().any(|v| v == "--noclass") {
        prm.same_class = false;
    }
    if let Some(i) = a.iter().position(|v| v == "--maxr") {
        prm.max_radius = a.get(i + 1).ok_or("maxr")?.parse()?;
    }
    if let Some(i) = a.iter().position(|v| v == "--link") {
        prm.link = a.get(i + 1).ok_or("link")?.parse()?;
        prm.normal_radius = prm.link;
    }
    if let Some(i) = a.iter().position(|v| v == "--dz") {
        prm.max_dz = a.get(i + 1).ok_or("dz")?.parse()?;
    }
    let grid = Index::new(&pts, prm.link.max(prm.normal_radius));
    println!("rejilla en {:.1?}", t.elapsed());
    let mut seeds = Vec::new();
    for &c in all.keys() {
        let of: Vec<usize> = (0..pts.len()).filter(|&i| pts[i].class == c).collect();
        if of.len() > 1000 {
            for k in 0..n.min(3) {
                seeds.push(of[of.len() / (n.min(3) + 1) * (k + 1)]);
            }
        }
    }
    for i in seeds {
        let t = Instant::now();
        let (r, surf) = grow(&pts, &grid, i, &prm);
        let dt = t.elapsed();
        let mut hist = std::collections::BTreeMap::new();
        for &j in &r {
            *hist.entry(pts[j as usize].class).or_insert(0usize) += 1;
        }
        println!(
            "semilla {i} clase {} z {:.1} {}: {} puntos en {:.1?} ({:.2} µs/pt) clases {:?}",
            pts[i].class,
            pts[i].z,
            if surf { "superficie" } else { "volumen" },
            r.len(),
            dt,
            dt.as_secs_f64() * 1e6 / r.len() as f64,
            hist
        );
    }
    Ok(())
}
