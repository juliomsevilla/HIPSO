//! Lists every `assets/icons/*.svg` so a new icon only needs its file dropped in the folder.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // not canonicalized: on Windows that gives a `\\?\` path, which `include_str!` can fail on
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
