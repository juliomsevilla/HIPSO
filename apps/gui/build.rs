//! Lista cada `assets/icons/*.svg` para que un icono nuevo solo necesite que se deje su archivo en
//! la carpeta.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // no canonicalizado: en Windows eso da una ruta `\\?\`, con la que `include_str!` puede fallar
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../../assets/icons");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "svg").then(|| p.file_stem()?.to_str().map(String::from))?
        })
        .collect();
    names.sort();
    let mut out = String::from("const SOURCES: &[(&str, &str)] = &[\n");
    for n in &names {
        let path = dir.join(format!("{n}.svg"));
        writeln!(
            out,
            "    ({n:?}, include_str!({:?})),",
            path.display().to_string()
        )?;
    }
    out.push_str("];\n");
    std::fs::write(
        PathBuf::from(std::env::var("OUT_DIR")?).join("icons_table.rs"),
        out,
    )?;
    Ok(())
}
