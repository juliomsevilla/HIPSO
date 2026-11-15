//! Utilidades compartidas por las pruebas de integración: archivos LAS sintéticos en la carpeta
//! temporal.
#![allow(dead_code)]

use hipso_core::Result;
use std::path::PathBuf;

/// Un archivo LAS 1.2 de formato de punto 1 (escala 1 mm); cada punto es un retorno único de clase
/// 1.
pub fn write_las(name: &str, pts: &[[f64; 3]]) -> Result<PathBuf> {
    write_las_with(name, pts, |_| (1, 0))
}

/// Como [`write_las`], con la clase y el identificador de fuente del punto de cada punto dados por
/// `attrs`.
pub fn write_las_with(
    name: &str,
    pts: &[[f64; 3]],
    attrs: impl Fn(usize) -> (u8, u16),
) -> Result<PathBuf> {
    write_las_with_gps(name, pts, |i| {
        let (class, source) = attrs(i);
        (class, source, 0.0)
    })
}

/// Como [`write_las_with`], con también el tiempo GPS de cada punto (los demás escritores dejan 0).
pub fn write_las_with_gps(
    name: &str,
    pts: &[[f64; 3]],
    attrs: impl Fn(usize) -> (u8, u16, f64),
) -> Result<PathBuf> {
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
    for (i, p) in pts.iter().enumerate() {
        let (class, source, gps) = attrs(i);
        let mut rec = [0u8; 28];
        for k in 0..3 {
            rec[4 * k..4 * k + 4].copy_from_slice(&((p[k] * 1000.0).round() as i32).to_le_bytes());
        }
        rec[14] = 0x09;
        rec[15] = class;
        rec[18..20].copy_from_slice(&source.to_le_bytes());
        rec[20..28].copy_from_slice(&gps.to_le_bytes());
        h.extend_from_slice(&rec);
    }
    let path = std::env::temp_dir().join(format!("hipso_{name}_{}.las", std::process::id()));
    std::fs::write(&path, h)?;
    Ok(path)
}

/// Un terreno ondulado: una superficie con pendientes en todas direcciones, de modo que un
/// movimiento en cualquier dirección cambia el ajuste.
pub fn terrain(x: f64, y: f64) -> f64 {
    3.0 * (x / 12.0).sin() + 2.0 * (y / 9.0).cos() + 0.05 * x + 0.03 * y
}

/// El terreno muestreado una vez en cada celda de una rejilla cuadrada de `step` metros, en un
/// lugar aleatorio de la celda (a partir de `seed`): un retículo regular fijaría los ajustes punto
/// a punto al retículo.
pub fn terrain_points(size: f64, step: f64, seed: u64) -> Vec<[f64; 3]> {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let n = (size / step) as usize;
    let mut v = Vec::with_capacity(n * n);
    for i in 0..n {
        for j in 0..n {
            let (x, y) = ((i as f64 + next()) * step, (j as f64 + next()) * step);
            v.push([x, y, terrain(x, y)]);
        }
    }
    v
}
