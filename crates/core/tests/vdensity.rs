//! Density by Voronoi cells: exact on a lattice, right on a random cloud, gaps counted apart,
//! and the same whether the cloud is one tile or many.

mod common;

use common::write_las;
use hipso_core::Result;
use hipso_core::grid::Grid;
use hipso_core::pipeline::Tiling;
use hipso_core::vdensity::{VDensityJob, density, run};

fn lattice(n: usize, s: f64) -> Vec<[f64; 3]> {
    let mut v = Vec::new();
    for r in 0..n {
        for c in 0..n {
            v.push([500_000.0 + c as f64 * s, 450_000.0 + r as f64 * s, 10.0]);
        }
    }
    v
}

fn random(n: usize, size: f64, seed: u64) -> Vec<[f64; 3]> {
    let mut state = seed;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..n)
        .map(|_| [500_000.0 + next() * size, 450_000.0 + next() * size, 10.0])
        .collect()
}

fn job(name: &str, pts: &[[f64; 3]], cell: f64) -> Result<VDensityJob> {
    let f = write_las(name, pts)?;
    let mut j = VDensityJob::new(
        vec![f],
        std::env::temp_dir().join(format!("hipso_{name}_{}", std::process::id())),
    );
    j.cell = cell;
    j.tiling = Tiling {
        size: 20.0,
        buffer: 8.0,
        threads: 0,
    };
    Ok(j)
}

/// The cells at least `margin` cells away from the edge of the grid.
fn interior(g: &Grid, margin: usize) -> Vec<f32> {
    let mut v = Vec::new();
    for r in margin..g.rows.saturating_sub(margin) {
        for c in margin..g.cols.saturating_sub(margin) {
            v.push(g.data[r * g.cols + c]);
        }
    }
    v
}

#[test]
fn a_lattice_of_half_a_metre_is_four_points_per_square_metre() -> Result<()> {
    let j = job("vd_lattice", &lattice(120, 0.5), 5.0)?;
    let (dens, _, rep, _) = density(&j)?;
    let inner = interior(&dens, 1);
    assert!(!inner.is_empty());
    for d in inner {
        assert!((d - 4.0).abs() < 1e-4, "{d}");
    }
    assert!(rep.points == 120 * 120);
    Ok(())
}

#[test]
fn a_random_cloud_has_the_density_it_was_made_with() -> Result<()> {
    // 10 points per square metre over 60 x 60 m
    let j = job("vd_poisson", &random(36_000, 60.0, 5), 10.0)?;
    let (dens, _, _, _) = density(&j)?;
    let inner = interior(&dens, 1);
    let mean = inner.iter().map(|v| f64::from(*v)).sum::<f64>() / inner.len() as f64;
    assert!((mean / 10.0 - 1.0).abs() < 0.03, "{mean}");
    Ok(())
}

#[test]
fn a_gap_is_void_area_and_its_cells_have_no_density() -> Result<()> {
    // a lattice with a 20 x 20 m hole in the middle
    let pts: Vec<[f64; 3]> = lattice(120, 0.5)
        .into_iter()
        .filter(|p| {
            !((20.0..40.0).contains(&(p[0] - 500_000.0))
                && (20.0..40.0).contains(&(p[1] - 450_000.0)))
        })
        .collect();
    let mut j = job("vd_gap", &pts, 5.0)?;
    j.max_cell_area = 3.0;
    let (dens, _, rep, _) = density(&j)?;
    let (x, y) = (500_030.0, 450_030.0);
    assert!(dens.index(x, y).is_some_and(|i| dens.data[i].is_nan()));
    assert!(rep.void_area > 0.0);
    Ok(())
}

#[test]
fn many_tiles_give_what_one_tile_does() -> Result<()> {
    let pts = random(8_000, 60.0, 11);
    let mut many = job("vd_many", &pts, 5.0)?;
    let mut one = job("vd_one", &pts, 5.0)?;
    many.tiling = Tiling {
        size: 15.0,
        buffer: 10.0,
        threads: 0,
    };
    one.tiling = Tiling {
        size: 400.0,
        buffer: 10.0,
        threads: 0,
    };
    let (a, ..) = density(&many)?;
    let (b, ..) = density(&one)?;
    assert_eq!((a.cols, a.rows), (b.cols, b.rows));
    // away from the border of the cloud, where the margin is what matters
    for (x, y) in a
        .data
        .iter()
        .zip(&b.data)
        .zip(0..)
        .filter_map(|((x, y), i)| {
            let (c, r) = (i % a.cols, i / a.cols);
            (c >= 1 && r >= 1 && c + 1 < a.cols && r + 1 < a.rows).then_some((x, y))
        })
    {
        assert!(
            (x.is_nan() && y.is_nan()) || (x - y).abs() < 1e-3 * y.abs().max(1.0),
            "{x} / {y}"
        );
    }
    Ok(())
}

#[test]
fn repeated_points_do_not_break_it_and_the_results_are_written() -> Result<()> {
    let mut pts = lattice(60, 0.5);
    let dup = pts[100];
    pts.push(dup);
    let mut j = job("vd_dup", &pts, 5.0)?;
    j.threshold = 3.0;
    let msg = run(&j)?;
    assert!(msg.starts_with("densidad de Voronoi en"), "{msg}");
    let base = j.output.clone();
    assert!(base.with_extension("tif").exists() && base.with_extension("csv").exists());
    assert!(
        base.with_file_name(format!(
            "{}_cumple.tif",
            base.file_name()
                .map_or(String::new(), |n| n.to_string_lossy().to_string())
        ))
        .exists()
    );
    let bad = VDensityJob {
        cell: 0.0,
        ..job("vd_bad", &lattice(10, 1.0), 5.0)?
    };
    assert!(density(&bad).is_err());
    Ok(())
}
