//! The TLS cubing on a cloud made of cylinders: a tree on its own, a stool with two sprouts and a
//! tree that forks above the height of the diameter.

use rustlidar_core::Result;
use rustlidar_core::tls::{TlsParams, analyse};

/// Points on the surface of a vertical (or leaning) cylinder, every ~1.5 cm.
fn cylinder(out: &mut Vec<[f64; 3]>, c: (f64, f64), r: f64, z0: f64, z1: f64, lean: (f64, f64)) {
    let step = 0.015;
    let rings = ((z1 - z0) / step).round() as usize;
    let around = ((std::f64::consts::TAU * r / step).round() as usize).max(8);
    for k in 0..=rings {
        let z = z0 + k as f64 * step;
        let (cx, cy) = (c.0 + lean.0 * (z - z0), c.1 + lean.1 * (z - z0));
        // every other ring is turned half a step, so the surface has no stripes
        let phase = if k % 2 == 0 { 0.0 } else { 0.5 };
        for a in 0..around {
            let t = (a as f64 + phase) * std::f64::consts::TAU / around as f64;
            out.push([cx + r * t.cos(), cy + r * t.sin(), z]);
        }
    }
}

/// A LAS 1.2 file of point format 1 (scale 1 mm) written to the temporary folder.
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
    let path = std::env::temp_dir().join(format!("rustlidar_{name}_{}.las", std::process::id()));
    std::fs::write(&path, h)?;
    Ok(path)
}

fn params() -> TlsParams {
    TlsParams {
        voxel: 0.02,
        // the cylinders stand on a ground at z = 0: the cloud is already heights
        normalized: true,
        // a perfect cylinder has a disc for neighbourhood, as tall as it is wide: the
        // verticality of real stems (rough, seen from one side) is not what is tested here
        vert_min: 0.0,
        ..TlsParams::default()
    }
}

#[test]
fn a_stem_a_stool_and_a_fork() -> Result<()> {
    let mut pts = Vec::new();
    // a tree on its own, 24 cm across
    cylinder(&mut pts, (0.0, 0.0), 0.12, 0.0, 7.0, (0.0, 0.0));
    // a stool: two sprouts 35 cm apart, 16 cm across
    cylinder(&mut pts, (6.0, 0.0), 0.08, 0.0, 7.0, (0.0, 0.0));
    cylinder(&mut pts, (6.35, 0.0), 0.08, 0.0, 7.0, (0.0, 0.0));
    // a tree 30 cm across that forks at 2.5 m into two branches 18 cm across
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
    let near = |x: f64| -> Vec<&rustlidar_core::tls::TreeResult> {
        trees
            .iter()
            .filter(|(rx, _)| (rx - x).abs() <= 1.0)
            .map(|(_, t)| *t)
            .collect()
    };
    // the tree on its own
    let single = near(near_origin(&r));
    assert_eq!(single.len(), 1);
    assert!(single[0].sprouts == 1 && single[0].fork_h.is_none());
    assert!(
        single[0].dbh.is_some_and(|d| (d - 0.24).abs() < 0.03),
        "{:?}",
        single[0].dbh
    );
    // the stool: two trees of one stool, each with its own diameter
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
    // the fork: one tree, with the diameter of its stem and the fork where it is
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

/// The x of the leftmost tree, where the tree on its own stands.
fn near_origin(r: &rustlidar_core::tls::TlsResult) -> f64 {
    r.trees
        .iter()
        .map(|t| t.x.round())
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_stem_with_an_unmeasured_stretch_is_followed_through_it() -> Result<()> {
    let mut pts = Vec::new();
    cylinder(&mut pts, (0.0, 0.0), 0.12, 0.0, 9.0, (0.0, 0.0));
    // an occlusion: nothing of the stem between 4.0 and 5.5 m
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
    let top = |r: &rustlidar_core::tls::TlsResult| {
        r.trees
            .iter()
            .flat_map(|t| t.sections.iter().filter(|s| s.ok).map(|s| s.h))
            .fold(0.0, f64::max)
    };
    // with the default stretch the stem goes on above the occlusion; with a short one it is lost
    assert!(top(&followed) > 8.0, "{}", top(&followed));
    assert!(top(&lost) < 4.5, "{}", top(&lost));
    Ok(())
}
