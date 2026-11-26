//! Unir nubes: dos nubes con puntos repetidos se unen y los duplicados se mantienen, marcan o
//! eliminan.

// en las pruebas, un `unwrap` que falla es la prueba que falla (clippy.toml no cubre los
// ayudantes de las pruebas de integración)
#![allow(clippy::unwrap_used)]

mod common;

use common::write_las;
use hipso_core::cloudtools::DupFields;
use hipso_core::las::Reader;
use hipso_core::merge::{DupAction, merge};
use std::path::PathBuf;

fn two() -> (PathBuf, PathBuf) {
    // tres puntos en cada nube; el segundo de A y el primero de B son el mismo punto
    let a = write_las("merge_a", &[[0.0, 0.0, 1.0], [5.0, 5.0, 2.0], [9.0, 1.0, 3.0]]).unwrap();
    let b = write_las("merge_b", &[[5.0, 5.0, 2.0], [20.0, 20.0, 4.0], [30.0, 5.0, 6.0]]).unwrap();
    (a, b)
}

fn run(tag: &str, action: DupAction) -> (u64, u64, u64) {
    let (a, b) = two();
    let out = std::env::temp_dir().join(format!("merge_out_{tag}_{}.las", std::process::id()));
    let r = merge(&[a, b], &out, action, DupFields::default()).unwrap();
    let rd = Reader::open(&out).unwrap();
    let mut withheld = 0;
    rd.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(usize::from(rd.header.record_len)) {
            if rd.decode(rec).is_some_and(|p| p.withheld) {
                withheld += 1;
            }
        }
        Ok(())
    })
    .unwrap();
    let _ = std::fs::remove_file(&out);
    (rd.header.count, r.duplicates, withheld)
}

#[test]
fn keep_mark_and_delete() {
    assert_eq!(run("keep", DupAction::Keep), (6, 0, 0));
    assert_eq!(run("mark", DupAction::Mark), (6, 1, 1));
    assert_eq!(run("del", DupAction::Delete), (5, 1, 0));
}

#[test]
fn one_cloud_is_refused() {
    let (a, _) = two();
    let out = std::env::temp_dir().join("merge_one.las");
    assert!(merge(&[a], &out, DupAction::Keep, DupFields::default()).is_err());
}
