//! Cancelar o cerrar el programa detiene las órdenes en marcha y borra lo que anunciaron. Va en un
//! archivo aparte porque el cierre es global al proceso.

// en las pruebas, un `unwrap` que falla es la prueba que falla
#![allow(clippy::unwrap_used)]
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hipso_core::model::{shutdown, spawn};

/// Una orden que anuncia `product`, lo empieza a escribir y se queda trabajando.
fn start(product: &Path, cancel: Arc<AtomicBool>) -> std::thread::JoinHandle<bool> {
    let script = format!(
        "echo '» creará: {0}'; echo a > '{0}'; echo b > '{1}'; exec sleep 30",
        product.display(),
        product.with_extension("dbf").display()
    );
    let handle = std::thread::spawn(move || {
        let argv = ["-c".to_string(), script];
        spawn(Path::new("/bin/sh"), &argv, &cancel, &mut |_| {}).is_err()
    });
    let from = Instant::now();
    while !product.with_extension("dbf").exists() && from.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    handle
}

#[test]
fn cancelling_or_closing_stops_the_command_and_removes_what_it_announced() {
    let dir: PathBuf = std::env::temp_dir().join(format!("hipso_spawn_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // un archivo anterior, no anunciado, se conserva
    let old = dir.join("previo.tif");
    std::fs::write(&old, "x").unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let a = dir.join("a.shp");
    let run = start(&a, cancel.clone());
    let from = Instant::now();
    cancel.store(true, Ordering::Relaxed);
    assert!(run.join().unwrap());
    assert!(from.elapsed() < Duration::from_secs(5));
    assert!(!a.exists() && !a.with_extension("dbf").exists());

    let b = dir.join("b.shp");
    let run = start(&b, Arc::new(AtomicBool::new(false)));
    let from = Instant::now();
    shutdown(Duration::from_secs(5));
    assert!(run.join().unwrap());
    assert!(from.elapsed() < Duration::from_secs(5));
    assert!(!b.exists() && !b.with_extension("dbf").exists());
    assert!(old.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
