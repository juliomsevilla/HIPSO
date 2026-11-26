//! Las herramientas que escriben varios archivos a partir de un nombre base no pisan ninguna de sus
//! entradas.

// en las pruebas, un `unwrap` que falla es la prueba que falla
#![allow(clippy::unwrap_used)]

use hipso_core::change::{ChangeJob, change};
use hipso_core::paths::Formats;

#[test]
fn a_derived_output_never_replaces_an_input() {
    let dir = std::env::temp_dir().join(format!("hipso_guard_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // el CHM de antes se llama como la diferencia que saldría de la base «x»
    let before = dir.join("x_dchm.tif");
    std::fs::write(&before, "original").unwrap();
    let job = ChangeJob {
        before: before.clone(),
        after: dir.join("despues.tif"),
        output: dir.join("x.shp"),
        min_change: 2.0,
        cover_height: 2.0,
        min_patch: 10,
        years: None,
        regions: None,
        trees: None,
        formats: Formats::default(),
    };
    let e = change(&job).unwrap_err().to_string();
    assert!(e.contains("no se sobrescribe"), "{e}");
    assert_eq!(std::fs::read_to_string(&before).unwrap(), "original");
    std::fs::remove_dir_all(&dir).unwrap();
}
