//! Compares grid A with grid B at A's cell centres: `compare_grids a.tif b.asc [--b-nodes]`.
//! `--b-nodes` reads B's ASCII corner as its first node (FUSION's DTM2ASCII output).

use std::path::Path;

use rustlidar_core::grid::Grid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(a), Some(b)) = (args.get(1), args.get(2)) else {
        return Err("usage: compare_grids A B [--b-nodes]".into());
    };
    let a = Grid::read(Path::new(a))?;
    let mut b = Grid::read(Path::new(b))?;
    if args.iter().any(|s| s == "--b-nodes") {
        b.x0 -= b.cell / 2.0;
        b.y0 -= b.cell / 2.0;
    }
    let mut d: Vec<f64> = Vec::new();
    for r in 0..a.rows {
        for c in 0..a.cols {
            let (x, y) = a.center(c, r);
            let (Some(&va), Some(vb)) = (
                a.data.get(r * a.cols + c),
                b.index(x, y).and_then(|i| b.data.get(i)),
            ) else {
                continue;
            };
            if !va.is_nan() && !vb.is_nan() {
                d.push(f64::from(va - vb));
            }
        }
    }
    if d.is_empty() {
        return Err("no overlapping valued cells".into());
    }
    let n = d.len() as f64;
    let mean = d.iter().sum::<f64>() / n;
    let rmse = (d.iter().map(|v| v * v).sum::<f64>() / n).sqrt();
    let mut abs: Vec<f64> = d.iter().map(|v| v.abs()).collect();
    abs.sort_by(f64::total_cmp);
    let q = |p: f64| {
        abs.get(((abs.len() - 1) as f64 * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    println!(
        "cells {}  mean(A-B) {mean:+.3}  RMSE {rmse:.3}  |d| p50 {:.3} p90 {:.3} p99 {:.3} max {:.3}",
        d.len(),
        q(0.5),
        q(0.9),
        q(0.99),
        q(1.0)
    );
    Ok(())
}
