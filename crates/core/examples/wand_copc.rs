//! Varita sobre una ventana COPC a toda la densidad: `wand_copc ARCHIVO X Y [radio]`. Prepara la
//! copia COPC de la caché, lee la ventana alrededor de `(X, Y)` y hace crecer la región desde el
//! punto más cercano; imprime cuántos puntos tiene la ventana y la región.

use hipso_core::wand::{grow_in_store, WandParams};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = std::path::PathBuf::from(a.first().ok_or("FILE")?);
    let b = hipso_core::las::Reader::open(&file)?.header.bounds;
    let (x, y): (f64, f64) = match (a.get(1), a.get(2)) {
        (Some(x), Some(y)) => (x.parse()?, y.parse()?),
        _ => ((b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0),
    };
    let radius: f64 = a.get(3).map_or(Ok(30.0), |v| v.parse())?;
    let t = Instant::now();
    let copc = hipso_core::copc_cache::ensure(&file)?;
    println!("copia COPC lista en {:.1?}: {}", t.elapsed(), copc.display());
    let t = Instant::now();
    let r = radius + 4.0;
    let (s, _) = hipso_core::copc_detail::load_detail(
        &[copc],
        [x - r, y - r, x + r, y + r],
        30_000_000,
        0.0,
        [0.0; 3],
        &[String::new()],
    )?
    .ok_or("no es COPC")?;
    println!("{} puntos en la ventana, leídos en {:.1?}", s.len(), t.elapsed());
    let seed = (0..s.len())
        .min_by(|&i, &j| {
            let d = |k: usize| {
                let p = s.xyz(k);
                (p[0] - x).hypot(p[1] - y)
            };
            d(i).total_cmp(&d(j))
        })
        .ok_or("vacío")?;
    let prm = WandParams { link: 0.5, normal_radius: 0.5, max_radius: radius, ..WandParams::default() };
    let t = Instant::now();
    let (found, surf) = grow_in_store(&s, &|_| true, seed, &prm);
    println!("{} puntos ({}) en {:.2?}", found.len(), if surf { "superficie" } else { "volumen" }, t.elapsed());
    Ok(())
}
