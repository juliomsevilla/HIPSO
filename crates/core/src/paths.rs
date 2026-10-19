//! Where results go. A product's path is worked out from the input it is made from and the
//! settings that shape it, so a run never inherits the path of an earlier run or of another
//! tool, and two settings of the same tool never overwrite each other.
//!
//! Folders follow the project layout many forestry workflows use: when the input sits under a
//! folder that has both `data` and `results`, products go to its `results`; otherwise to a
//! `resultados` folder next to the input.

use std::path::{Path, PathBuf};

/// The folder products of `input` go to.
pub fn results_dir(input: &Path) -> PathBuf {
    let dir = input.parent().unwrap_or_else(|| Path::new("."));
    for anc in dir.ancestors() {
        if anc.join("data").is_dir() && anc.join("results").is_dir() {
            return anc.join("results");
        }
    }
    dir.join("resultados")
}

/// The name of an input without its extension, safe in a file name.
pub fn stem(input: &Path) -> String {
    input
        .file_stem()
        .map_or("datos".into(), |s| s.to_string_lossy().into_owned())
}

/// `<results>/<stem>_<tag>.<ext>`.
pub fn product(input: &Path, tag: &str, ext: &str) -> PathBuf {
    results_dir(input).join(format!(
        "{}_{}.{}",
        stem(input),
        tag,
        ext.trim_start_matches('.')
    ))
}

/// Creates the folder a product is written into.
pub fn ensure_parent(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => std::fs::create_dir_all(d),
        _ => Ok(()),
    }
}

/// The formats a table result is written in: a vector layer (shapefile), a text table, or both.
/// Text tables stay available because very large runs make a table with a polygon per cell
/// slow to write and to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Formats {
    pub vector: bool,
    pub csv: bool,
}

impl Default for Formats {
    fn default() -> Self {
        Self {
            vector: true,
            csv: false,
        }
    }
}

impl Formats {
    /// From a path: `.csv` means a text table only, anything else a vector layer.
    pub fn from_path(p: &Path) -> Self {
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
            Self {
                vector: false,
                csv: true,
            }
        } else {
            Self::default()
        }
    }

    /// At least one format is always written.
    pub fn fixed(self) -> Self {
        if self.vector || self.csv {
            self
        } else {
            Self::default()
        }
    }
}

/// `path` with its `.shp` or `.csv` extension (if it has one) replaced by `ext`; any other dot
/// in the name (`chm_0.5m`) is kept.
pub fn swap_ext(path: &Path, ext: &str) -> PathBuf {
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let lower = name.to_ascii_lowercase();
    let stem = if lower.ends_with(".shp") || lower.ends_with(".csv") {
        &name[..name.len() - 4]
    } else {
        &name
    };
    path.with_file_name(format!("{stem}.{ext}"))
}

/// A number in a file name: `1` for 1.0, `0.5` for 0.5.
pub fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The first path like `path` that is free: `a.tif`, then `a_2.tif`, `a_3.tif`…
pub fn unique(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 2..10_000 {
        let name = match &ext {
            Some(e) => format!("{stem}_{n}.{e}"),
            None => format!("{stem}_{n}"),
        };
        let p = path.with_file_name(name);
        if !p.exists() {
            return p;
        }
    }
    path.to_path_buf()
}

/// The newest raster `<stem>_<tag>…` made from `input`, if any: never one of another input.
fn find_raster(input: &Path, tag: &str) -> Option<PathBuf> {
    let prefix = format!("{}_{tag}", stem(input)).to_lowercase();
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in std::fs::read_dir(results_dir(input)).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        let ok = name.starts_with(&prefix)
            && (name.ends_with(".tif") || name.ends_with(".tiff") || name.ends_with(".asc"));
        let time = e.metadata().and_then(|m| m.modified()).ok();
        if let (true, Some(t)) = (ok, time)
            && best.as_ref().is_none_or(|b| t > b.0)
        {
            best = Some((t, e.path()));
        }
    }
    best.map(|b| b.1)
}

/// The newest DTM made from `input`, if any: the one a CHM or metrics run should use by
/// default, never a DTM of another input.
pub fn find_dtm(input: &Path) -> Option<PathBuf> {
    find_raster(input, "mdt")
}

/// The newest CHM made from `input`, if any.
pub fn find_chm(input: &Path) -> Option<PathBuf> {
    find_raster(input, "chm")
}

/// The newest DSM made from `input`, if any.
pub fn find_dsm(input: &Path) -> Option<PathBuf> {
    find_raster(input, "mds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn products_are_named_from_the_input_and_tag() {
        let p = product(Path::new("C:/x/datos_1.las"), "mdt_1m", "tif");
        assert!(
            p.ends_with("resultados/datos_1_mdt_1m.tif")
                || p.ends_with("resultados\\datos_1_mdt_1m.tif")
        );
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(0.125), "0.125");
    }

    #[test]
    fn project_layout_sends_results_to_results() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!("rl_paths_{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/LiDAR"))?;
        std::fs::create_dir_all(root.join("results"))?;
        let input = root.join("data/LiDAR/a.las");
        assert_eq!(results_dir(&input), root.join("results"));
        let first = root.join("results/a_mdt.tif");
        std::fs::write(&first, b"x")?;
        assert_eq!(unique(&first), root.join("results/a_mdt_2.tif"));
        assert_eq!(find_dtm(&input), Some(first));
        assert_eq!(find_dtm(&root.join("data/LiDAR/other.las")), None);
        std::fs::remove_dir_all(&root)
    }
}
