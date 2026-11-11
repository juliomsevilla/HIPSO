//! Fitting one cloud onto another: the known motion is recovered by every variant, and the
//! trimmed fit ignores a block of changed ground that drags the plain one off.

mod common;

use common::{terrain_points, write_las};
use hipso_core::Result;
use hipso_core::cloudtools::{FitParams, FitReport, fit_cloud};

const MOVE: [f64; 3] = [0.4, -0.3, 0.2];

fn run(name: &str, source: &[[f64; 3]], p: &FitParams) -> Result<FitReport> {
    let reference = write_las(&format!("{name}_ref"), &terrain_points(60.0, 0.25, 1))?;
    let moved = write_las(name, source)?;
    let out = std::env::temp_dir().join(format!("hipso_{name}_out_{}.las", std::process::id()));
    let rep = fit_cloud(std::slice::from_ref(&reference), &moved, &out, p);
    for f in [reference, moved, out] {
        let _ = std::fs::remove_file(f);
    }
    rep
}

/// The same terrain sampled between the reference points, then moved by `MOVE`.
fn displaced() -> Vec<[f64; 3]> {
    terrain_points(56.0, 0.25, 2)
        .into_iter()
        .map(|p| [p[0] + MOVE[0], p[1] + MOVE[1], p[2] + MOVE[2]])
        .collect()
}

fn params() -> FitParams {
    FitParams {
        thin: 0.25,
        max_xy: 2.0,
        max_z: 1.5,
        max_iter: 60,
        stop_dist: 0.001,
        ..FitParams::default()
    }
}

/// How far the recovered shift is from the inverse of `MOVE`, in metres.
fn error(r: &FitReport) -> f64 {
    (0..3)
        .map(|k| (r.shift[k] + MOVE[k]).powi(2))
        .sum::<f64>()
        .sqrt()
}

#[test]
fn point_to_point_recovers_the_motion() -> Result<()> {
    let r = run("fit_pp", &displaced(), &params())?;
    // it creeps towards the answer (the motion is 0.54 m) and has not arrived in 60 iterations
    assert!(r.after < r.before, "{r:?}");
    assert!(error(&r) < 0.4, "error {} {r:?}", error(&r));
    Ok(())
}

#[test]
fn point_to_plane_recovers_the_motion_faster() -> Result<()> {
    let p2p = run("fit_pp2", &displaced(), &params())?;
    let r = run(
        "fit_pl",
        &displaced(),
        &FitParams {
            plane: true,
            ..params()
        },
    )?;
    assert!(error(&r) < 0.05, "error {} {r:?}", error(&r));
    assert!(
        error(&r) < error(&p2p) / 2.0,
        "plane {} point {}",
        error(&r),
        error(&p2p)
    );
    Ok(())
}

#[test]
fn shift_only_does_not_turn() -> Result<()> {
    let r = run(
        "fit_sh",
        &displaced(),
        &FitParams {
            shift_only: true,
            ..params()
        },
    )?;
    assert!(r.rotation.iter().all(|a| a.abs() < 1e-9), "{r:?}");
    assert!(error(&r) < 0.3, "error {} {r:?}", error(&r));
    Ok(())
}

#[test]
fn trimming_sets_aside_changed_ground() -> Result<()> {
    // a 14 x 14 m block raised 3 m: a landslide, a felled stand, a building
    let changed: Vec<[f64; 3]> = displaced()
        .into_iter()
        .map(|p| {
            if (20.0..34.0).contains(&p[0]) && (20.0..34.0).contains(&p[1]) {
                [p[0], p[1], p[2] + 3.0]
            } else {
                p
            }
        })
        .collect();
    let wide = FitParams {
        max_z: 4.0,
        plane: true,
        ..params()
    };
    let plain = run("fit_plain", &changed, &wide)?;
    let trimmed = run("fit_trim", &changed, &FitParams { trim: 0.8, ..wide })?;
    assert!(
        error(&trimmed) < error(&plain),
        "trimmed {} plain {}",
        error(&trimmed),
        error(&plain)
    );
    assert!(error(&trimmed) < 0.1, "trimmed {}", error(&trimmed));
    Ok(())
}

#[test]
fn bad_trim_is_refused() {
    for trim in [0.0, 1.5, -1.0] {
        assert!(run("fit_bad", &displaced(), &FitParams { trim, ..params() }).is_err());
    }
}
