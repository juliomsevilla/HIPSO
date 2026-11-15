//! Un árbol cortado por el borde entre dos modelos de copas lo encuentra una sola vez, entero, la
//! hoja que contiene su cima; por sí sola, la hoja que solo tiene su flanco encuentra un árbol en
//! el borde.

use hipso_core::Result;
use hipso_core::grid::Grid;
use hipso_core::jobs::{TreesJob, trees};
use hipso_core::paths::Formats;
use hipso_core::trees::{CrownParams, Smoothing, TopParams};
use std::path::{Path, PathBuf};

/// Conos: (x, y, altura, radio); la superficie es la más alta de ellos.
const TREES: [(f64, f64, f64, f64); 3] = [
    (48.0, 25.0, 20.0, 9.0),
    (75.0, 25.0, 18.0, 8.0),
    (20.0, 25.0, 15.0, 7.0),
];

fn sheet(x0: f64) -> Result<Grid> {
    let mut g = Grid::new(x0, 0.0, 0.5, 200, 100)?;
    for r in 0..g.rows {
        for c in 0..g.cols {
            let (x, y) = g.center(c, r);
            let h = TREES
                .iter()
                .map(|t| (t.2 - t.2 / t.3 * ((x - t.0).hypot(y - t.1))).max(0.0))
                .fold(0.0, f64::max);
            g.data[r * g.cols + c] = if h > 0.0 { h as f32 } else { 0.0 };
        }
    }
    Ok(g)
}

fn job(chm: PathBuf, out: PathBuf, around: Vec<PathBuf>) -> TreesJob {
    TreesJob {
        gpkg: false,
        crown_metrics: Vec::new(),
        chm,
        output: out,
        formats: Formats {
            vector: false,
            csv: true,
        },
        crowns: None,
        crowns_shp: None,
        tops_shp: None,
        cloud: None,
        tops_file: None,
        tops: TopParams::default(),
        crown: CrownParams::default(),
        smooth: 0.0,
        smoothing: Smoothing::Mean,
        field: None,
        around,
        margin: 20.0,
    }
}

fn count(csv: &Path) -> Result<Vec<(f64, f64)>> {
    let text = std::fs::read_to_string(csv)?;
    Ok(text
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            Some((f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?))
        })
        .collect())
}

#[test]
fn a_tree_on_the_edge_is_found_once() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("hipso_across_{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    // dos hojas lado a lado, de 50 m cada una: el primer árbol (48, 25) tiene su flanco en la del
    // este
    let (west, east) = (dir.join("oeste.tif"), dir.join("este.tif"));
    sheet(0.0)?
        .crop([0.0, 0.0, 50.0, 50.0])?
        .write_tiff(&west, None)?;
    sheet(0.0)?
        .crop([50.0, 0.0, 100.0, 50.0])?
        .write_tiff(&east, None)?;
    let run = |name: &str, chm: &Path, around: Vec<PathBuf>| -> Result<Vec<(f64, f64)>> {
        let out = dir.join(format!("{name}.shp"));
        trees(&job(chm.to_path_buf(), out.clone(), around))?;
        count(&out.with_extension("csv"))
    };
    // cada una por sí sola: la hoja del este se inventa un árbol a partir del flanco en su borde
    let alone = run("sola_oeste", &west, vec![])?.len() + run("sola_este", &east, vec![])?.len();
    assert!(alone > TREES.len(), "on their own: {alone}");
    // con las vecinas: los tres árboles, una vez cada uno, en sus propias cimas
    let w = run("con_oeste", &west, vec![east.clone()])?;
    let e = run("con_este", &east, vec![west.clone()])?;
    assert_eq!(w.len() + e.len(), TREES.len(), "west {w:?} east {e:?}");
    assert!(w.iter().all(|t| t.0 < 50.0) && e.iter().all(|t| t.0 >= 50.0));
    // el del borde está donde se puso, y está en la hoja del oeste
    assert!(
        w.iter().any(|t| (t.0 - 48.0).hypot(t.1 - 25.0) < 1.0),
        "{w:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}
