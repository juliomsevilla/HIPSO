//! Sheets of one block: the catalogue reads their headers and finds what is wrong with the
//! tiling, and a DTM made with the neighbours of each sheet has no seam at its edges.

mod common;

use common::{terrain_points, write_las_with};
use hipso_core::Result;
use hipso_core::catalog::{Kind, catalog};
use hipso_core::grid::Grid;
use hipso_core::jobs::DtmJob;
use hipso_core::pipeline::{Ground, Steps, Surface, Tiling};
use hipso_core::sheets::dtm_each;
use std::path::PathBuf;

/// The terrain cut in two sheets at x = 30 (ground class), written to a folder of its own.
fn two_sheets(tag: &str) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let dir = std::env::temp_dir().join(format!("hipso_seam_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let all = terrain_points(60.0, 0.5, 7);
    let west: Vec<[f64; 3]> = all.iter().copied().filter(|p| p[0] < 30.0).collect();
    let east: Vec<[f64; 3]> = all.iter().copied().filter(|p| p[0] >= 30.0).collect();
    for (name, pts) in [("oeste", &west), ("este", &east)] {
        let f = write_las_with(&format!("seam_{tag}_{name}"), pts, |_| (2, 0))?;
        std::fs::rename(&f, dir.join(format!("{name}.las")))?;
    }
    Ok((dir.clone(), dir.join("oeste.las"), dir.join("este.las")))
}

fn base() -> DtmJob {
    DtmJob {
        inputs: Vec::new(),
        output: PathBuf::new(),
        cell: 1.0,
        surface: Surface::Tin,
        steps: Steps {
            noise: None,
            ground: Ground::Classes,
            buildings: None,
            extras: Default::default(),
            rules: Vec::new(),
        },
        tiling: Tiling::default(),
        smooth: 0.0,
        smoothing: hipso_core::trees::Smoothing::Mean,
        breaklines: None,
    }
}

#[test]
fn the_catalogue_reads_headers_and_a_folder() -> Result<()> {
    let (dir, west, east) = two_sheets("cat")?;
    let c = catalog(std::slice::from_ref(&dir))?;
    assert_eq!(c.entries.len(), 2);
    assert_eq!(c.points(), 120 * 120);
    // two sheets side by side, no CRS declared: that is all that is wrong
    let kinds: Vec<Kind> = c.issues(1.0).iter().map(|i| i.kind).collect();
    assert!(kinds.contains(&Kind::NoCrs), "{kinds:?}");
    assert!(
        !kinds.contains(&Kind::Overlap) && !kinds.contains(&Kind::Gap),
        "{kinds:?}"
    );
    // the same sheet twice is caught
    let twice = catalog(&[west.clone(), east, west])?;
    assert!(twice.issues(1.0).iter().any(|i| i.kind == Kind::Duplicate));
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn an_unreadable_file_is_listed_and_the_rest_is_catalogued() -> Result<()> {
    let (dir, west, _) = two_sheets("bad")?;
    let broken = dir.join("roto.laz");
    std::fs::write(&broken, b"no es un LAZ")?;
    let c = catalog(&[west, broken])?;
    assert_eq!((c.entries.len(), c.failed.len()), (1, 1));
    assert!(c.issues(1.0).iter().any(|i| i.kind == Kind::Unreadable));
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn with_neighbours_the_edge_of_a_sheet_has_the_dtm_of_the_block() -> Result<()> {
    let (dir, west, east) = two_sheets("dtm")?;
    let out_for = |p: &std::path::Path| dir.join(format!("{}_mdt.tif", hipso_core::paths::stem(p)));
    let inputs = [west.clone(), east.clone()];
    // the block as a whole is the truth
    let whole = DtmJob {
        inputs: inputs.to_vec(),
        output: dir.join("todo.tif"),
        ..base()
    };
    hipso_core::jobs::dtm(&whole)?;
    let truth = Grid::read(&whole.output)?;
    // each sheet alone
    dtm_each(&inputs, &base(), &out_for, None, 0.0)?;
    let alone = Grid::read(&out_for(&west))?;
    // each sheet with its neighbour
    let buffered_dir = dir.join("con_vecinas");
    std::fs::create_dir_all(&buffered_dir)?;
    let out_b =
        |p: &std::path::Path| buffered_dir.join(format!("{}_mdt.tif", hipso_core::paths::stem(p)));
    dtm_each(&inputs, &base(), &out_b, None, 5.0)?;
    let joined = Grid::read(&out_b(&west))?;
    // same cells as the sheet's own DTM
    assert_eq!((joined.cols, joined.rows), (alone.cols, alone.rows));
    // how far each is from the block's DTM, over the cells both have
    let worst = |g: &Grid| {
        let (mut worst, mut checked) = (0.0f32, 0);
        for r in 0..g.rows {
            for c in 0..g.cols {
                let (x, y) = g.center(c, r);
                let v = g.data[r * g.cols + c];
                let Some(i) = truth.index(x, y) else { continue };
                let t = truth.data[i];
                if v.is_finite() && v > -9000.0 && t.is_finite() && t > -9000.0 {
                    worst = worst.max((v - t).abs());
                    checked += 1;
                }
            }
        }
        (worst, checked)
    };
    let (w_joined, n) = worst(&joined);
    let (w_alone, _) = worst(&alone);
    assert!(n > 1000, "{n}");
    assert!(w_joined < 1e-3, "con vecinas: {w_joined}");
    assert!(
        w_alone > 10.0 * w_joined.max(1e-4),
        "sola {w_alone} con vecinas {w_joined}"
    );
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// Points on a half-metre lattice over `[x0, x1) x [0, 40)`, with heights that vary from point to
/// point (so a cell's statistics tell its points apart).
fn lattice(x0: f64, x1: f64) -> Vec<[f64; 3]> {
    let mut v = Vec::new();
    let mut x = x0;
    while x < x1 {
        let mut y = 0.0;
        while y < 40.0 {
            let k = ((x * 2.0) as i64 * 7 + (y * 2.0) as i64 * 13).rem_euclid(11) as f64;
            v.push([x, y, 3.0 + k * 0.7]);
            y += 0.5;
        }
        x += 0.5;
    }
    v
}

/// The rows of a metrics table by cell centre: `(x, y) -> (points, mean elevation)`.
fn cells(csv: &std::path::Path) -> Result<std::collections::BTreeMap<(i64, i64), (i64, String)>> {
    let text = std::fs::read_to_string(csv)?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
    let at = |name: &str| header.iter().position(|h| *h == name).unwrap_or(usize::MAX);
    let (cx, cy, n, mean) = (
        at("center X"),
        at("center Y"),
        at("Total return count"),
        at("Elev mean"),
    );
    let mut out = std::collections::BTreeMap::new();
    for l in lines {
        let f: Vec<&str> = l.split(',').collect();
        let key = (
            f[cx].parse::<f64>().unwrap_or(0.0).round() as i64,
            f[cy].parse::<f64>().unwrap_or(0.0).round() as i64,
        );
        // a cell with no row of its own twice would overwrite: keep the larger count
        let count: i64 = f[n].parse().unwrap_or(0);
        if out
            .get(&key)
            .is_none_or(|(c, _): &(i64, String)| count > *c)
        {
            out.insert(key, (count, f[mean].to_string()));
        }
    }
    Ok(out)
}

fn rows(csv: &std::path::Path) -> Result<usize> {
    Ok(std::fs::read_to_string(csv)?.lines().count() - 1)
}

#[test]
fn metrics_cells_on_the_seam_are_written_once_with_all_their_points() -> Result<()> {
    use hipso_core::jobs::MetricsJob;
    use hipso_core::metrics::MetricParams;
    use hipso_core::paths::Formats;
    use hipso_core::sheets::metrics_each;
    let dir = std::env::temp_dir().join(format!("hipso_mseam_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    // the block is 60 x 40 m; the sheets are cut at x = 30, which is the middle of the 10 m cell
    // centred on 30 (cells are centred on multiples of the cell size)
    let all = lattice(0.0, 60.0);
    let write = |name: &str, pts: &[[f64; 3]]| -> Result<PathBuf> {
        let f = write_las_with(&format!("mseam_{name}"), pts, |_| (1, 0))?;
        let to = dir.join(format!("{name}.las"));
        std::fs::rename(&f, &to)?;
        Ok(to)
    };
    let west = write("oeste", &lattice(0.0, 30.0))?;
    let east = write("este", &lattice(30.0, 60.0))?;
    let block = write("bloque", &all)?;
    // a flat ground at 0 under everything
    let mut dtm = Grid::new(-10.0, -10.0, 1.0, 90, 70)?;
    dtm.data.iter_mut().for_each(|v| *v = 0.0);
    let dtm_path = dir.join("mdt.tif");
    dtm.write_tiff(&dtm_path, None)?;
    let base = MetricsJob {
        input: PathBuf::new(),
        dtm: PathBuf::new(),
        output: PathBuf::new(),
        params: MetricParams {
            cell: 10.0,
            ..MetricParams::default()
        },
        intensity: false,
        strata: false,
        formats: Formats {
            vector: false,
            csv: true,
        },
    };
    let out_for = |p: &std::path::Path| dir.join(format!("{}_m.csv", hipso_core::paths::stem(p)));
    let dtm_for = |_: &std::path::Path| Ok(dtm_path.clone());
    let inputs = [west.clone(), east.clone()];
    // the truth: the block as one file
    metrics_each(std::slice::from_ref(&block), &base, &out_for, &dtm_for, 0.0)?;
    let truth = cells(&out_for(&block))?;
    // alone: the cell on the seam is in both tables, with part of its points each
    metrics_each(&inputs, &base, &out_for, &dtm_for, 0.0)?;
    let (w, e) = (cells(&out_for(&west))?, cells(&out_for(&east))?);
    let seam = (30, 20);
    let whole = truth[&seam].0;
    assert!(
        w[&seam].0 < whole && e[&seam].0 < whole,
        "{} {} {whole}",
        w[&seam].0,
        e[&seam].0
    );
    assert_eq!(w[&seam].0 + e[&seam].0, whole);
    assert!(rows(&out_for(&west))? + rows(&out_for(&east))? > truth.len());
    // with neighbours: every cell once, with the values of the block
    metrics_each(&inputs, &base, &out_for, &dtm_for, 10.0)?;
    let (w, e) = (cells(&out_for(&west))?, cells(&out_for(&east))?);
    assert_eq!(rows(&out_for(&west))? + rows(&out_for(&east))?, truth.len());
    let mut joined = w.clone();
    for (k, v) in e {
        assert!(
            joined.insert(k, v).is_none(),
            "cell {k:?} is in both tables"
        );
    }
    assert_eq!(joined, truth);
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

fn write_in(dir: &std::path::Path, name: &str, pts: &[[f64; 3]]) -> Result<PathBuf> {
    // tests run side by side in one process: the temporary name carries the test's folder
    let tag = dir
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let f = write_las_with(&format!("sseam_{tag}_{name}"), pts, |_| (1, 0))?;
    let to = dir.join(format!("{name}.las"));
    std::fs::rename(&f, &to)?;
    Ok(to)
}

fn class_count(path: &std::path::Path, class: u8) -> Result<(u64, u64)> {
    let r = hipso_core::las::Reader::open(path)?;
    let (mut all, mut some) = (0, 0);
    r.scan(|p| {
        all += 1;
        some += u64::from(p.class == class);
    })?;
    Ok((all, some))
}

#[test]
fn classifying_with_neighbours_sees_across_the_edge_and_writes_only_the_sheet() -> Result<()> {
    use hipso_core::noise::NoiseParams;
    use hipso_core::pipeline::Noise;
    use hipso_core::sheets::classify_each;
    let dir = std::env::temp_dir().join(format!("hipso_cseam_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    // a ground at 0 and, 30 m above it across the cut at x = 30, eight points of a bird or a
    // wire: the noise filter calls isolated what has fewer than five points around it
    let flock: Vec<[f64; 3]> = (0..8)
        .map(|i| [28.0 + 0.5 * i as f64, 20.0, 30.0])
        .collect();
    let mut west: Vec<[f64; 3]> = lattice(0.0, 30.0)
        .into_iter()
        .map(|p| [p[0], p[1], 0.0])
        .collect();
    let mut east: Vec<[f64; 3]> = lattice(30.0, 60.0)
        .into_iter()
        .map(|p| [p[0], p[1], 0.0])
        .collect();
    west.extend(flock.iter().filter(|p| p[0] < 30.0));
    east.extend(flock.iter().filter(|p| p[0] >= 30.0));
    let (w, e) = (
        write_in(&dir, "oeste", &west)?,
        write_in(&dir, "este", &east)?,
    );
    let steps = Steps {
        noise: Some(Noise::Isolated(NoiseParams::default())),
        ground: Ground::Classes,
        buildings: None,
        extras: Default::default(),
        rules: Vec::new(),
    };
    let out_for = |p: &std::path::Path| dir.join(format!("{}_c.las", hipso_core::paths::stem(p)));
    let inputs = [w.clone(), e.clone()];
    // alone: each half of the flock is four points, and isolated
    classify_each(&inputs, &out_for, &steps, &Tiling::default(), 0.0)?;
    let alone = class_count(&out_for(&w), 7)?.1 + class_count(&out_for(&e), 7)?.1;
    assert_eq!(alone, 8);
    // with the neighbour: eight points together are not isolated, and each sheet keeps its own
    classify_each(&inputs, &out_for, &steps, &Tiling::default(), 10.0)?;
    let (w_all, w_noise) = class_count(&out_for(&w), 7)?;
    let (e_all, e_noise) = class_count(&out_for(&e), 7)?;
    assert_eq!((w_noise, e_noise), (0, 0));
    assert_eq!((w_all, e_all), (west.len() as u64, east.len() as u64));
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

/// How far `g` is from `truth` over the cells both have, and how many that is.
fn worst_vs(g: &Grid, truth: &Grid) -> (f32, usize) {
    let (mut worst, mut n) = (0.0f32, 0);
    for r in 0..g.rows {
        for c in 0..g.cols {
            let (x, y) = g.center(c, r);
            let v = g.data[r * g.cols + c];
            let Some(i) = truth.index(x, y) else { continue };
            let t = truth.data[i];
            if v.is_finite() && v > -9000.0 && t.is_finite() && t > -9000.0 {
                worst = worst.max((v - t).abs());
                n += 1;
            }
        }
    }
    (worst, n)
}

#[test]
fn surfaces_with_neighbours_have_no_seam_at_the_edge() -> Result<()> {
    use hipso_core::jobs::{ChmJob, ChmMethod, DsmJob, DsmMethod};
    use hipso_core::pipeline::{self, PitFree};
    use hipso_core::sheets::{chm_each, dsm_each};
    let dir = std::env::temp_dir().join(format!("hipso_sseam_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let all = terrain_points(60.0, 0.5, 11);
    let (w, e) = (
        write_in(
            &dir,
            "oeste",
            &all.iter()
                .copied()
                .filter(|p| p[0] < 30.0)
                .collect::<Vec<_>>(),
        )?,
        write_in(
            &dir,
            "este",
            &all.iter()
                .copied()
                .filter(|p| p[0] >= 30.0)
                .collect::<Vec<_>>(),
        )?,
    );
    let inputs = [w.clone(), e.clone()];
    let tiling = Tiling::default();
    let out_a = |p: &std::path::Path| dir.join(format!("{}_alone.tif", hipso_core::paths::stem(p)));
    let out_b = |p: &std::path::Path| dir.join(format!("{}_nb.tif", hipso_core::paths::stem(p)));
    // MDS by triangulation
    let dsm = DsmJob {
        input: PathBuf::new(),
        output: PathBuf::new(),
        cell: 1.0,
        method: DsmMethod::Tin,
        fill: true,
        tiling: tiling.clone(),
        smooth: 0.0,
        smoothing: hipso_core::trees::Smoothing::Mean,
    };
    let (truth, _) = pipeline::dsm(&inputs, 1.0, Surface::Tin, &tiling)?;
    dsm_each(&inputs, &dsm, &out_a, None, 0.0)?;
    dsm_each(&inputs, &dsm, &out_b, None, 5.0)?;
    let (alone, with) = (
        worst_vs(&Grid::read(&out_a(&w))?, &truth),
        worst_vs(&Grid::read(&out_b(&w))?, &truth),
    );
    assert!(
        with.1 > 1000 && with.0 < 1e-3,
        "dsm with neighbours: {with:?}"
    );
    assert!(
        alone.0 > 10.0 * with.0.max(1e-4),
        "dsm alone {alone:?} with {with:?}"
    );
    // CHM by pit-free stack over a flat ground at 0
    let mut ground = Grid::new(-10.0, -10.0, 1.0, 90, 70)?;
    ground.data.iter_mut().for_each(|v| *v = 0.0);
    let ground_path = dir.join("suelo.tif");
    ground.write_tiff(&ground_path, None)?;
    let dtm_for = |_: &std::path::Path| Ok(ground_path.clone());
    let chm = ChmJob {
        input: PathBuf::new(),
        dtm: PathBuf::new(),
        output: PathBuf::new(),
        cell: 1.0,
        method: ChmMethod::PitFree(PitFree::default()),
        tiling: tiling.clone(),
        smooth: 0.0,
        smoothing: hipso_core::trees::Smoothing::Mean,
    };
    let (truth, _) = pipeline::chm_pitfree(&inputs, &ground, 1.0, &PitFree::default(), &tiling)?;
    chm_each(&inputs, &chm, &out_a, &dtm_for, None, 0.0)?;
    chm_each(&inputs, &chm, &out_b, &dtm_for, None, 5.0)?;
    let (alone, with) = (
        worst_vs(&Grid::read(&out_a(&w))?, &truth),
        worst_vs(&Grid::read(&out_b(&w))?, &truth),
    );
    assert!(
        with.1 > 1000 && with.0 < 1e-3,
        "chm with neighbours: {with:?}"
    );
    assert!(
        alone.0 > 10.0 * with.0.max(1e-4),
        "chm alone {alone:?} with {with:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

#[test]
fn the_highest_point_surface_only_needs_neighbours_to_fill_holes() -> Result<()> {
    use hipso_core::jobs::{DsmJob, DsmMethod};
    use hipso_core::sheets::dsm_each;
    let dir = std::env::temp_dir().join(format!("hipso_hseam_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    // a point every 2.5 m: with 1 m cells most cells are empty and have to be filled
    let sparse = |x0: f64, x1: f64| -> Vec<[f64; 3]> {
        terrain_points(60.0, 2.5, 5)
            .into_iter()
            .filter(|p| p[0] >= x0 && p[0] < x1)
            .collect()
    };
    let (w, e) = (
        write_in(&dir, "oeste", &sparse(0.0, 30.0))?,
        write_in(&dir, "este", &sparse(30.0, 60.0))?,
    );
    let block = write_in(&dir, "bloque", &sparse(0.0, 60.0))?;
    let inputs = [w.clone(), e.clone()];
    let job = |fill, smooth| DsmJob {
        input: PathBuf::new(),
        output: PathBuf::new(),
        cell: 1.0,
        method: DsmMethod::Max,
        fill,
        tiling: Tiling::default(),
        smooth,
        smoothing: hipso_core::trees::Smoothing::Mean,
    };
    let out_a = |p: &std::path::Path| dir.join(format!("{}_a.tif", hipso_core::paths::stem(p)));
    let out_b = |p: &std::path::Path| dir.join(format!("{}_b.tif", hipso_core::paths::stem(p)));
    let truth = hipso_core::chm::dsm(&hipso_core::las::Reader::open(&block)?, 1.0, true)?;
    dsm_each(&inputs, &job(true, 0.0), &out_a, None, 0.0)?;
    dsm_each(&inputs, &job(true, 0.0), &out_b, None, 8.0)?;
    let on = |g: &Grid| {
        // the cells within 4 m of the seam, where the filling looks across it
        let mut worst = 0.0f32;
        for r in 0..g.rows {
            for c in 0..g.cols {
                let (x, y) = g.center(c, r);
                let v = g.data[r * g.cols + c];
                let Some(i) = truth.index(x, y) else { continue };
                // away from the outer edge of the block, where nothing is looked across
                if (x - 30.0).abs() < 4.0
                    && (8.0..52.0).contains(&y)
                    && v.is_finite()
                    && truth.data[i].is_finite()
                {
                    worst = worst.max((v - truth.data[i]).abs());
                }
            }
        }
        worst
    };
    let (alone, with) = (on(&Grid::read(&out_a(&w))?), on(&Grid::read(&out_b(&w))?));
    assert!(with < 1e-3, "with neighbours {with}");
    assert!(alone > 0.1, "alone {alone}");
    // no filling and no smoothing: nothing to look across, the same surface either way
    dsm_each(&inputs, &job(false, 0.0), &out_a, None, 0.0)?;
    dsm_each(&inputs, &job(false, 0.0), &out_b, None, 8.0)?;
    assert_eq!(
        Grid::read(&out_a(&w))?.data.len(),
        Grid::read(&out_b(&w))?.data.len()
    );
    assert!(worst_vs(&Grid::read(&out_a(&w))?, &Grid::read(&out_b(&w))?).0 == 0.0);
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
