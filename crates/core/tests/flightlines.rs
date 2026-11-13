//! Flight lines from the gaps in the GPS time: the lines found, the small ones dropped, the
//! number written to the points, and the files that cannot be used.

mod common;

use common::write_las_with_gps;
use hipso_core::Result;
use hipso_core::flightlines::{FlightJob, FlightParams, Write, assign, detect, run};
use hipso_core::las::Reader;
use std::path::PathBuf;

/// Points every 0.1 s in each `(from, to)` block of time, along x, 10 m apart in y per block.
fn blocks(spans: &[(f64, f64)]) -> (Vec<[f64; 3]>, Vec<f64>) {
    let (mut pts, mut times) = (Vec::new(), Vec::new());
    for (b, (t0, t1)) in spans.iter().enumerate() {
        let n = ((t1 - t0) / 0.1).round() as usize;
        for k in 0..n {
            pts.push([100.0 + k as f64 * 0.5, 100.0 + b as f64 * 20.0 + (k % 2) as f64 * 6.0, 20.0]);
            times.push(t0 + k as f64 * 0.1);
        }
    }
    (pts, times)
}

fn params(gap: f64, min_points: u64) -> FlightParams {
    FlightParams {
        gap,
        min_points,
        ..Default::default()
    }
}

fn file(name: &str, spans: &[(f64, f64)]) -> Result<PathBuf> {
    let (pts, times) = blocks(spans);
    write_las_with_gps(name, &pts, |i| (1, 0, times[i]))
}

#[test]
fn the_gap_decides_how_many_lines() -> Result<()> {
    // times start at 1000: a time of 0 means no GPS time
    let f = file(
        "fl_gap",
        &[(1000.0, 1010.0), (1100.0, 1110.0), (1300.0, 1310.0)],
    )?;
    let three = detect(std::slice::from_ref(&f), &params(30.0, 1))?;
    assert_eq!(three.lines.len(), 3);
    assert_eq!(
        three.lines.iter().map(|l| l.points).collect::<Vec<_>>(),
        vec![100, 100, 100]
    );
    assert!((three.lines[1].t0 - 1100.0).abs() < 1e-6 && (three.lines[1].t1 - 1109.9).abs() < 1e-6);
    // the silences are 90 s and 190 s: a gap of 150 keeps the first two together
    let two = detect(std::slice::from_ref(&f), &params(150.0, 1))?;
    assert_eq!(two.lines.len(), 2);
    assert_eq!(two.lines[0].points, 200);
    let one = detect(std::slice::from_ref(&f), &params(200.0, 1))?;
    assert_eq!(one.lines.len(), 1);
    Ok(())
}

#[test]
fn small_stretches_are_not_lines() -> Result<()> {
    let f = file(
        "fl_small",
        &[(1000.0, 1010.0), (1100.0, 1100.3), (1300.0, 1310.0)],
    )?;
    let rep = detect(&[f], &params(30.0, 50))?;
    assert_eq!(rep.lines.len(), 2);
    assert_eq!(rep.discarded, 3);
    // the lines are numbered by time, without a hole
    assert_eq!(
        rep.lines.iter().map(|l| l.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(rep.index.line_at(1100.1), 0);
    assert_eq!(rep.index.line_at(1305.0), 2);
    Ok(())
}

#[test]
fn a_file_without_gps_time_is_refused() -> Result<()> {
    let f = file("fl_nogps", &[(1000.0, 1010.0)])?;
    // the same file declared as point format 0, which has no time
    let mut bytes = std::fs::read(&f)?;
    bytes[104] = 0;
    let g = f.with_file_name("hipso_fl_nogps0.las");
    std::fs::write(&g, bytes)?;
    let e = detect(&[g], &params(30.0, 1))
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(e.contains("no tiene tiempo GPS"), "{e}");
    Ok(())
}

#[test]
fn the_order_of_the_points_does_not_matter() -> Result<()> {
    let spans = [(1000.0, 1010.0), (1100.0, 1110.0), (1300.0, 1310.0)];
    let (pts, times) = blocks(&spans);
    // the same points back to front
    let n = pts.len();
    let back = write_las_with_gps(
        "fl_back",
        &pts.iter().rev().copied().collect::<Vec<_>>(),
        |i| (1, 0, times[n - 1 - i]),
    )?;
    let fwd = file("fl_fwd", &spans)?;
    let a = detect(&[fwd], &params(30.0, 1))?;
    let b = detect(&[back], &params(30.0, 1))?;
    assert_eq!(a.lines.len(), b.lines.len());
    for (x, y) in a.lines.iter().zip(&b.lines) {
        assert_eq!((x.points, x.t0, x.t1), (y.points, y.t0, y.t1));
    }
    Ok(())
}

#[test]
fn lines_continue_from_one_file_to_the_next() -> Result<()> {
    let a = file("fl_a", &[(1000.0, 1010.0)])?;
    let b = file("fl_b", &[(1020.0, 1030.0), (1200.0, 1210.0)])?;
    let rep = detect(&[a, b], &params(30.0, 1))?;
    assert_eq!(rep.lines.len(), 2);
    assert_eq!(rep.lines[0].points, 200);
    Ok(())
}

#[test]
fn the_line_number_is_written_and_read_back() -> Result<()> {
    let spans = [(1000.0, 1010.0), (1100.0, 1110.0)];
    let f = file("fl_assign", &spans)?;
    let rep = detect(std::slice::from_ref(&f), &params(30.0, 1))?;
    let dir = std::env::temp_dir().join(format!("hipso_fl_out_{}", std::process::id()));
    let out = assign(std::slice::from_ref(&f), &dir, &rep, Write::SourceId)?;
    assert_eq!(out.len(), 1);
    let r = Reader::open(&out[0])?;
    let mut ids = Vec::new();
    let rec_len = usize::from(r.header.record_len);
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            ids.push(r.decode_extra(rec).source_id);
        }
        Ok(())
    })?;
    assert_eq!(ids.len(), 200);
    assert!(ids[..100].iter().all(|i| *i == 1) && ids[100..].iter().all(|i| *i == 2));
    // in the user data byte
    let out = assign(
        std::slice::from_ref(&f),
        &dir.join("u"),
        &rep,
        Write::UserData,
    )?;
    let r = Reader::open(&out[0])?;
    let mut data = Vec::new();
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            data.push(r.decode_extra(rec).user_data);
        }
        Ok(())
    })?;
    assert!(data[..100].iter().all(|i| *i == 1) && data[100..].iter().all(|i| *i == 2));
    // more lines than the field holds
    let mut many = rep.clone();
    many.lines = (1..=300)
        .map(|k| hipso_core::flightlines::Line {
            id: k,
            ..rep.lines[0].clone()
        })
        .collect();
    assert!(assign(&[f], &dir, &many, Write::UserData).is_err());
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn the_job_reports_the_lines_and_writes_the_table() -> Result<()> {
    let f = file("fl_job", &[(1000.0, 1010.0), (1100.0, 1110.0)])?;
    let dir = std::env::temp_dir().join(format!("hipso_fl_job_{}", std::process::id()));
    let msg = run(&FlightJob {
        inputs: vec![f],
        params: params(30.0, 1),
        into: Write::None,
        output_dir: dir.clone(),
        csv: true,
        vector: true,
    })?;
    assert!(msg.starts_with("2 líneas de vuelo en 200 puntos"), "{msg}");
    let csv = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().ends_with(".csv"))
        .map(|e| std::fs::read_to_string(e.path()));
    assert!(csv.is_some_and(|c| c.is_ok_and(|c| c.lines().count() == 3)));
    let shp = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().ends_with(".shp"))
        .map(|e| hipso_core::view::read_vector(&e.path()));
    let shp = shp.ok_or(hipso_core::Error::Param("sin capa".into()))??;
    assert_eq!(shp.features.len(), 2);
    assert!(shp.fields.iter().any(|f| f == "puntos"));
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn bad_settings_are_refused() -> Result<()> {
    let f = file("fl_bad", &[(1000.0, 1010.0)])?;
    assert!(detect(std::slice::from_ref(&f), &params(0.0, 1)).is_err());
    assert!(detect(&[], &params(30.0, 1)).is_err());
    Ok(())
}

#[test]
fn the_separation_between_passes_can_use_the_gaps_in_time() -> Result<()> {
    use hipso_core::cloudtools::{SwathBy, SwathReturns, swath_separation_by};
    // two passes over the same ground 0.2 m apart, both with source id 0
    let (mut pts, mut times) = (Vec::new(), Vec::new());
    for (pass, t0) in [(0.0, 1000.0), (0.2, 1200.0)] {
        for k in 0..100 {
            pts.push([100.0 + k as f64, 100.0, 20.0 + pass]);
            times.push(t0 + k as f64 * 0.1);
        }
    }
    let f = write_las_with_gps("fl_swath", &pts, |i| (1, 0, times[i]))?;
    let out = std::env::temp_dir().join(format!("hipso_fl_swath_{}.tif", std::process::id()));
    let by_id = swath_separation_by(
        std::slice::from_ref(&f),
        &out,
        1.0,
        SwathReturns::All,
        SwathBy::SourceId,
    )?;
    assert_eq!(by_id.overlap_cells, 0);
    let by_time = swath_separation_by(
        std::slice::from_ref(&f),
        &out,
        1.0,
        SwathReturns::All,
        SwathBy::GpsGap(30.0),
    )?;
    assert!(by_time.overlap_cells > 50, "{}", by_time.overlap_cells);
    assert!(
        (by_time.mean_abs - 0.2).abs() < 1e-6,
        "{}",
        by_time.mean_abs
    );
    let _ = std::fs::remove_file(out);
    Ok(())
}
