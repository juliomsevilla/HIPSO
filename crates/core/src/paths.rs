//! Where results go. A product's path is worked out from the input it is made from and the
//! settings that shape it, so a run never inherits the path of an earlier run or of another
//! tool, and two settings of the same tool never overwrite each other.
//!
//! Unless the user chooses a folder, products are written next to the input they come from,
//! under another name.

use std::path::{Path, PathBuf};

static OUTPUT_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Sends every product to `dir` (the user's choice) instead of the folder worked out from its
/// input; `None` goes back to the default.
pub fn set_output_dir(dir: Option<PathBuf>) {
    if let Ok(mut d) = OUTPUT_DIR.write() {
        *d = dir;
    }
}

/// The folder the user chose for results, if any.
pub fn output_dir() -> Option<PathBuf> {
    OUTPUT_DIR.read().ok().and_then(|d| d.clone())
}

/// The folder products of `input` go to.
pub fn results_dir(input: &Path) -> PathBuf {
    output_dir().unwrap_or_else(|| default_results_dir(input))
}

/// The folder products of `input` go to when the user has not chosen one: the folder of the
/// input itself, so no folder is created behind the user's back.
pub fn default_results_dir(input: &Path) -> PathBuf {
    match input.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    }
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

/// Like [`unique`], for a product written with companions next to it (`<stem>_<suffix>.<ext>`,
/// such as a model's report and table): the first name for which neither the product nor any
/// companion exists, so a new run never overwrites the files of an earlier one.
pub fn unique_with(path: &Path, companions: &[(&str, &str)]) -> PathBuf {
    let free = |p: &Path| {
        let stem = p
            .file_stem()
            .map_or(String::new(), |s| s.to_string_lossy().into_owned());
        !p.exists()
            && companions
                .iter()
                .all(|(suffix, ext)| !p.with_file_name(format!("{stem}_{suffix}.{ext}")).exists())
    };
    if free(path) {
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
        if free(&p) {
            return p;
        }
    }
    path.to_path_buf()
}

/// Words that mark a raster derived from a DTM, CHM or DSM (terrain products, tree crowns): the
/// name of the source starts the name of its product, but the product is not the source. A crown
/// raster holds tree ids, so taking it for a CHM made heights of hundreds of thousands of metres.
const DERIVED: &[&str] = &[
    "arboles",
    "copas",
    "apices",
    "pendiente",
    "orientacion",
    "sombreado",
    "curv",
    "curvatura",
    "rugosidad",
    "tri",
    "tpi",
    "depresiones",
    "acumulacion",
    "twi",
    "drenaje",
    "subcuencas",
    "visibilidad",
    "volumen",
];

fn is_derived(name: &str) -> bool {
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .split('_')
        .any(|w| DERIVED.contains(&w))
}

/// The newest raster `<stem>_<tag>…` made from `input`, if any: never one of another input.
fn find_raster(input: &Path, tag: &str) -> Option<PathBuf> {
    let prefix = format!("{}_{tag}", stem(input)).to_lowercase();
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut dirs = vec![results_dir(input)];
    let default = default_results_dir(input);
    if !dirs.contains(&default) {
        dirs.push(default);
    }
    for e in dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().to_lowercase();
        let ok = name.starts_with(&prefix)
            && !is_derived(name.strip_prefix(&prefix).unwrap_or(&name))
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
    fn a_product_made_from_a_chm_is_not_a_chm() {
        assert!(is_derived("_0.5m_arboles_dalponte_copas.tif"));
        assert!(is_derived("_1m_pendiente_2.tif"));
        assert!(!is_derived("_0.5m.tif"));
        assert!(!is_derived("_1m_tin_2.tif"));
    }

    #[test]
    fn products_are_named_from_the_input_and_tag() {
        let p = product(Path::new("C:/x/datos_1.las"), "mdt_1m", "tif");
        assert_eq!(p, Path::new("C:/x/datos_1_mdt_1m.tif"));
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(0.125), "0.125");
    }

    #[test]
    fn products_go_next_to_the_input() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!("rl_paths_{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/LiDAR"))?;
        std::fs::create_dir_all(root.join("results"))?;
        let input = root.join("data/LiDAR/a.las");
        assert_eq!(results_dir(&input), root.join("data/LiDAR"));
        let first = root.join("data/LiDAR/a_mdt.tif");
        std::fs::write(&first, b"x")?;
        assert_eq!(unique(&first), root.join("data/LiDAR/a_mdt_2.tif"));
        assert_eq!(find_dtm(&input), Some(first));
        assert_eq!(find_dtm(&root.join("data/LiDAR/other.las")), None);
        std::fs::remove_dir_all(&root)
    }
}
