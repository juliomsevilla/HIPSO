//! El cubicado TLS sobre una nube hecha de cilindros: un árbol solo, una cepa con dos brotes y un
//! árbol que se bifurca por encima de la altura del diámetro.

use hipso_core::Result;
use hipso_core::tls::{TlsParams, analyse};

/// Puntos sobre la superficie de un cilindro vertical (o inclinado), cada ~1,5 cm.
fn cylinder(out: &mut Vec<[f64; 3]>, c: (f64, f64), r: f64, z0: f64, z1: f64, lean: (f64, f64)) {
    let step = 0.015;
    let rings = ((z1 - z0) / step).round() as usize;
    let around = ((std::f64::consts::TAU * r / step).round() as usize).max(8);
    for k in 0..=rings {
        let z = z0 + k as f64 * step;
        let (cx, cy) = (c.0 + lean.0 * (z - z0), c.1 + lean.1 * (z - z0));
        // un anillo de cada dos se gira medio paso, para que la superficie no tenga rayas
        let phase = if k % 2 == 0 { 0.0 } else { 0.5 };
        for a in 0..around {
            let t = (a as f64 + phase) * std::f64::consts::TAU / around as f64;
            out.push([cx + r * t.cos(), cy + r * t.sin(), z]);
        }
    }
}

/// Un archivo LAS 1.2 de formato de punto 1 (escala 1 mm) escrito en la carpeta temporal.
fn write_las(name: &str, pts: &[[f64; 3]]) -> Result<std::path::PathBuf> {
    let min = pts.iter().fold([f64::INFINITY; 3], |m, p| {
        [m[0].min(p[0]), m[1].min(p[1]), m[2].min(p[2])]
    });
    let max = pts.iter().fold([f64::NEG_INFINITY; 3], |m, p| {
        [m[0].max(p[0]), m[1].max(p[1]), m[2].max(p[2])]
    });
    let mut h = vec![0u8; 227];
    h[..4].copy_from_slice(b"LASF");
    h[24] = 1;
    h[25] = 2;
    h[94..96].copy_from_slice(&227u16.to_le_bytes());
    h[96..100].copy_from_slice(&227u32.to_le_bytes());
    h[104] = 1;
    h[105..107].copy_from_slice(&28u16.to_le_bytes());
    h[107..111].copy_from_slice(&(pts.len() as u32).to_le_bytes());
    for at in [131, 139, 147] {
        h[at..at + 8].copy_from_slice(&0.001f64.to_le_bytes());
    }
    for (at, v) in [
        (179, max[0]),
        (187, min[0]),
        (195, max[1]),
        (203, min[1]),
        (211, max[2]),
        (219, min[2]),
    ] {
        h[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    for p in pts {
        let mut rec = [0u8; 28];
        for k in 0..3 {
            rec[4 * k..4 * k + 4].copy_from_slice(&((p[k] * 1000.0).round() as i32).to_le_bytes());
        }
        rec[14] = 0x09;
        rec[15] = 1;
        h.extend_from_slice(&rec);
    }
    let path = std::env::temp_dir().join(format!("hipso_{name}_{}.las", std::process::id()));
    std::fs::write(&path, h)?;
    Ok(path)
}

fn params() -> TlsParams {
    TlsParams {
        voxel: 0.02,
        // los cilindros se apoyan en un suelo en z = 0: la nube ya son alturas
        normalized: true,
        // un cilindro perfecto tiene un disco por entorno, tan alto como ancho: la verticalidad de
        // los troncos reales (rugosos, vistos desde un lado) no es lo que se prueba aquí
        vert_min: 0.0,
        ..TlsParams::default()
    }
}

#[test]
fn a_stem_a_stool_and_a_fork() -> Result<()> {
    let mut pts = Vec::new();
    // un árbol solo, de 24 cm de diámetro
    cylinder(&mut pts, (0.0, 0.0), 0.12, 0.0, 7.0, (0.0, 0.0));
    // a stool: two sprouts 35 cm apart, 16 cm across
    cylinder(&mut pts, (6.0, 0.0), 0.08, 0.0, 7.0, (0.0, 0.0));
    cylinder(&mut pts, (6.35, 0.0), 0.08, 0.0, 7.0, (0.0, 0.0));
    // un árbol de 30 cm de diámetro que se bifurca a 2,5 m en dos ramas de 18 cm de diámetro
    cylinder(&mut pts, (12.0, 0.0), 0.15, 0.0, 2.5, (0.0, 0.0));
    cylinder(&mut pts, (12.0, 0.0), 0.09, 2.5, 7.0, (0.25, 0.0));
    cylinder(&mut pts, (12.0, 0.0), 0.09, 2.5, 7.0, (-0.25, 0.0));
    let input = write_las("stools", &pts)?;
    let r = analyse(&input, &params());
    let _ = std::fs::remove_file(&input);
    let r = r?;
    let trees: Vec<_> = r.trees.iter().map(|t| (t.x.round(), t)).collect();
    assert_eq!(
        trees.len(),
        4,
        "{:?}",
        r.trees
            .iter()
            .map(|t| (t.x, t.dbh, t.flags.clone()))
            .collect::<Vec<_>>()
    );
    let near = |x: f64| -> Vec<&hipso_core::tls::TreeResult> {
        trees
            .iter()
            .filter(|(rx, _)| (rx - x).abs() <= 1.0)
            .map(|(_, t)| *t)
            .collect()
    };
    // el árbol solo
    let single = near(near_origin(&r));
    assert_eq!(single.len(), 1);
    assert!(single[0].sprouts == 1 && single[0].fork_h.is_none());
    assert!(
        single[0].dbh.is_some_and(|d| (d - 0.24).abs() < 0.03),
        "{:?}",
        single[0].dbh
    );
    // la cepa: dos árboles de una cepa, cada uno con su propio diámetro
    let xs: Vec<f64> = r.trees.iter().map(|t| t.x).collect();
    let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let sprouts: Vec<_> = r
        .trees
        .iter()
        .filter(|t| (t.x - (min_x + 6.0)).abs() < 1.0)
        .collect();
    assert_eq!(sprouts.len(), 2);
    assert!(
        sprouts
            .iter()
            .all(|t| t.sprouts == 2 && t.flags.contains("BROTES"))
    );
    assert_eq!(sprouts[0].stool, sprouts[1].stool);
    assert!(
        sprouts
            .iter()
            .all(|t| t.dbh.is_some_and(|d| (d - 0.16).abs() < 0.03))
    );
    // la bifurcación: un árbol, con el diámetro de su tronco y la bifurcación donde está
    let forked: Vec<_> = r
        .trees
        .iter()
        .filter(|t| (t.x - (min_x + 12.0)).abs() < 1.0)
        .collect();
    assert_eq!(forked.len(), 1);
    assert!(forked[0].flags.contains("BIFURCADO") && forked[0].sprouts == 1);
    assert!(
        forked[0].dbh.is_some_and(|d| (d - 0.30).abs() < 0.03),
        "{:?}",
        forked[0].dbh
    );
    assert!(
        forked[0].fork_h.is_some_and(|h| (2.0..3.2).contains(&h)),
        "{:?}",
        forked[0].fork_h
    );
    Ok(())
}

/// La x del árbol más a la izquierda, donde está el árbol solo.
fn near_origin(r: &hipso_core::tls::TlsResult) -> f64 {
    r.trees
        .iter()
        .map(|t| t.x.round())
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_stem_with_an_unmeasured_stretch_is_followed_through_it() -> Result<()> {
    let mut pts = Vec::new();
    cylinder(&mut pts, (0.0, 0.0), 0.12, 0.0, 9.0, (0.0, 0.0));
    // una oclusión: nada del tronco entre 4,0 y 5,5 m
    pts.retain(|p| !(4.0..5.5).contains(&p[2]));
    let input = write_las("gap", &pts)?;
    let followed = analyse(&input, &params());
    let lost = analyse(
        &input,
        &TlsParams {
            max_gap: 0.5,
            ..params()
        },
    );
    let _ = std::fs::remove_file(&input);
    let (followed, lost) = (followed?, lost?);
    let top = |r: &hipso_core::tls::TlsResult| {
        r.trees
            .iter()
            .flat_map(|t| t.sections.iter().filter(|s| s.ok).map(|s| s.h))
            .fold(0.0, f64::max)
    };
    // con el tramo por defecto el tronco sigue por encima de la oclusión; con uno corto se pierde
    assert!(top(&followed) > 8.0, "{}", top(&followed));
    assert!(top(&lost) < 4.5, "{}", top(&lost));
    Ok(())
}
