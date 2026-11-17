//! Adónde van los resultados. La ruta de un producto se calcula a partir de la entrada de la que se
//! hace y de los ajustes que lo configuran, de modo que una ejecución nunca hereda la ruta de una
//! ejecución anterior o de otra herramienta, y dos ajustes de la misma herramienta nunca se
//! sobrescriben.
//!
//! Salvo que el usuario elija una carpeta, los productos se escriben junto a la entrada de la que
//! proceden, con otro nombre.

use std::path::{Path, PathBuf};

static OUTPUT_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Envía todo producto a `dir` (la elección del usuario) en lugar de a la carpeta calculada a
/// partir de su entrada; `None` vuelve a la de por defecto.
pub fn set_output_dir(dir: Option<PathBuf>) {
    if let Ok(mut d) = OUTPUT_DIR.write() {
        *d = dir;
    }
}

/// La carpeta que eligió el usuario para los resultados, si la hay.
pub fn output_dir() -> Option<PathBuf> {
    OUTPUT_DIR.read().ok().and_then(|d| d.clone())
}

/// La carpeta adonde van los productos de `input`.
pub fn results_dir(input: &Path) -> PathBuf {
    output_dir().unwrap_or_else(|| default_results_dir(input))
}

/// La carpeta adonde van los productos de `input` cuando el usuario no ha elegido: la carpeta de la
/// propia entrada, de modo que no se crea ninguna carpeta a espaldas del usuario.
pub fn default_results_dir(input: &Path) -> PathBuf {
    match input.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// El nombre de una entrada sin su extensión, seguro como nombre de archivo.
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

/// Crea la carpeta en la que se escribe un producto.
pub fn ensure_parent(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => std::fs::create_dir_all(d),
        _ => Ok(()),
    }
}

/// Los formatos en que se escribe un resultado de tabla: una capa vectorial (shapefile), una tabla
/// de texto o ambos. Las tablas de texto siguen disponibles porque las ejecuciones muy grandes
/// hacen lenta de escribir y de abrir una tabla con un polígono por celda.
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
    /// A partir de una ruta: `.csv` significa solo tabla de texto, cualquier otra cosa una capa
    /// vectorial.
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

    /// Siempre se escribe al menos un formato.
    pub fn fixed(self) -> Self {
        if self.vector || self.csv {
            self
        } else {
            Self::default()
        }
    }
}

/// `path` con su extensión `.shp`, `.csv`, `.tif`, `.tiff` o `.gpkg` (si la tiene) sustituida por
/// `ext`; cualquier otro punto del nombre (`chm_0.5m`) se conserva.
pub fn swap_ext(path: &Path, ext: &str) -> PathBuf {
    let name = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let lower = name.to_ascii_lowercase();
    let stem = [".shp", ".csv", ".tif", ".gpkg", ".tiff"]
        .iter()
        .find(|e| lower.ends_with(**e))
        .map_or(&name[..], |e| &name[..name.len() - e.len()]);
    path.with_file_name(format!("{stem}.{ext}"))
}

/// Un número en un nombre de archivo: `1` para 1,0, `0.5` para 0,5.
pub fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// La primera ruta como `path` que está libre: `a.tif`, luego `a_2.tif`, `a_3.tif`…
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

/// Como [`unique`], para un producto escrito con acompañantes a su lado (`<nombre>_<sufijo>.<ext>`,
/// como el informe y la tabla de un modelo): el primer nombre para el que no existen ni el producto
/// ni ningún acompañante, de modo que una ejecución nueva nunca sobrescribe los archivos de una
/// anterior.
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

/// Palabras que marcan un ráster derivado de un MDT, CHM o MDS (productos del terreno, copas de
/// árboles): el nombre del origen empieza el nombre de su producto, pero el producto no es el
/// origen. Un ráster de copas guarda identificadores de árbol, así que tomarlo por un CHM hizo
/// alturas de cientos de miles de metros.
const DERIVED: &[&str] = &[
    "arboles",
    "copas",
    "individuos",
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

/// El ráster `<nombre>_<etiqueta>…` más reciente hecho a partir de `input`, si lo hay: nunca uno de
/// otra entrada.
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

/// El MDT más reciente hecho a partir de `input`, si lo hay: el que debe usar por defecto una
/// ejecución de CHM o de métricas, nunca un MDT de otra entrada.
pub fn find_dtm(input: &Path) -> Option<PathBuf> {
    find_raster(input, "mdt")
}

/// El CHM más reciente hecho a partir de `input`, si lo hay.
pub fn find_chm(input: &Path) -> Option<PathBuf> {
    find_raster(input, "chm")
}

/// El MDS más reciente hecho a partir de `input`, si lo hay.
pub fn find_dsm(input: &Path) -> Option<PathBuf> {
    find_raster(input, "mds")
}

#[cfg(test)]
mod tests {
    #[test]
    fn swapping_an_extension_replaces_the_known_ones_only() {
        // se comparan como rutas, no como texto: Windows escribe «\» donde aquí hay «/»
        let sw = |p: &str, e: &str| swap_ext(Path::new(p), e);
        assert_eq!(sw("a/chm_0.5m.shp", "csv"), Path::new("a/chm_0.5m.csv"));
        assert_eq!(sw("a/x_vdensidad.tif", "csv"), Path::new("a/x_vdensidad.csv"));
        assert_eq!(sw("a/x_vdensidad.TIF", "tif"), Path::new("a/x_vdensidad.tif"));
        assert_eq!(sw("a/x.gpkg", "shp"), Path::new("a/x.shp"));
        // un punto que no es una extensión se queda
        assert_eq!(sw("a/chm_0.5m", "tif"), Path::new("a/chm_0.5m.tif"));
    }

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
