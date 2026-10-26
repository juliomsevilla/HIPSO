//! Coordinate systems: which one each file is in, and whether files can be combined. Nothing is
//! reprojected; what matters is that two data sets in different systems are never mixed without a
//! word: a polygon in UTM 30 over a cloud in UTM 29 clips nothing (or clips the wrong place),
//! and a raster off by a zone is hundreds of kilometres away.

use std::path::{Path, PathBuf};

use crate::las::Reader;
use crate::{Error, Result};

/// A file and the system it declares, if it declares one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub epsg: Option<u16>,
}

impl Item {
    pub fn new(label: impl Into<String>, epsg: Option<u16>) -> Self {
        Self {
            label: label.into(),
            epsg,
        }
    }

    /// The system a file declares: a cloud's header, a GeoTIFF's keys, a shapefile's (or ASCII
    /// grid's) `.prj`. A file that cannot be read has none.
    pub fn of(path: &Path) -> Self {
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self::new(label, epsg_of(path))
    }
}

/// The EPSG code a file declares, if any.
pub fn epsg_of(path: &Path) -> Option<u16> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "las" | "laz" => Reader::open(path).ok().and_then(|r| r.header.epsg),
        e if crate::raster::is_raster_ext(e) && e != "asc" => crate::grid::Grid::read_epsg(path),
        "geojson" | "json" => crate::geojson::epsg_hint(path),
        "gpkg" => crate::gpkg::epsg_hint(path),
        _ => std::fs::read_to_string(path.with_extension("prj"))
            .ok()
            .and_then(|t| crate::vector::epsg_from_prj(&t)),
    }
}

/// A readable name for a code, with the code: `ETRS89 / UTM 30N (EPSG:25830)`.
pub fn name(epsg: u16) -> String {
    let utm = |datum: &str, zone: u16, south: bool| {
        format!(
            "{datum} / UTM {zone}{} (EPSG:{epsg})",
            if south { "S" } else { "N" }
        )
    };
    match epsg {
        25828..=25838 => utm("ETRS89", epsg - 25800, false),
        32601..=32660 => utm("WGS 84", epsg - 32600, false),
        32701..=32760 => utm("WGS 84", epsg - 32700, true),
        23028..=23038 => utm("ED50", epsg - 23000, false),
        4326 => "WGS 84 geográficas (EPSG:4326)".into(),
        4258 => "ETRS89 geográficas (EPSG:4258)".into(),
        4230 => "ED50 geográficas (EPSG:4230)".into(),
        3857 => "WGS 84 / Pseudo-Mercator (EPSG:3857)".into(),
        _ => format!("EPSG:{epsg}"),
    }
}

/// How two declared systems relate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Same,
    /// ETRS89 and WGS 84 UTM of one zone: they differ by about a metre in Europe, which matters
    /// for nothing at the scale of forest products but is worth saying.
    NearlyEqual,
    Different,
}

pub fn relation(a: u16, b: u16) -> Relation {
    if a == b {
        return Relation::Same;
    }
    // ETRS89 UTM 28-38N (25828-25838) against WGS 84 UTM 28-38N (32628-32638)
    let zone = |c: u16| match c {
        25828..=25838 => Some(c - 25800),
        32628..=32638 => Some(c - 32600),
        _ => None,
    };
    let etrs = |c: u16| (25828..=25838).contains(&c);
    match (zone(a), zone(b)) {
        (Some(x), Some(y)) if x == y && etrs(a) != etrs(b) => Relation::NearlyEqual,
        _ => Relation::Different,
    }
}

/// Checks that everything that is going to be combined is in one system.
///
/// - Two items in different systems: an error that names them, since the result would be wrong.
/// - Items that declare nothing: they cannot be checked, so a warning says so.
/// - ETRS89 against WGS 84 of the same zone: a warning.
///
/// Returns the warnings, for the caller to put in its report.
pub fn check(items: &[Item]) -> Result<Vec<String>> {
    let known: Vec<(&Item, u16)> = items
        .iter()
        .filter_map(|i| i.epsg.map(|e| (i, e)))
        .collect();
    let mut warnings = Vec::new();
    if let Some((first, e0)) = known.first() {
        for (item, e) in &known[1..] {
            match relation(*e0, *e) {
                Relation::Same => {}
                Relation::NearlyEqual => warnings.push(format!(
                    "«{}» está en {} y «{}» en {}: difieren en torno a un metro",
                    first.label,
                    name(*e0),
                    item.label,
                    name(*e)
                )),
                Relation::Different => {
                    return Err(Error::Param(format!(
                        "no están en el mismo sistema de coordenadas: «{}» está en {} y «{}» en {}. Este archivo no se puede pasar al mismo sistema automáticamente: conviértelo antes.",
                        first.label,
                        name(*e0),
                        item.label,
                        name(*e)
                    )));
                }
            }
        }
    }
    let unknown: Vec<&str> = items
        .iter()
        .filter(|i| i.epsg.is_none())
        .map(|i| i.label.as_str())
        .collect();
    if !unknown.is_empty() && items.len() > 1 {
        // a long list says nothing more than its first names
        let shown: Vec<String> = unknown.iter().take(3).map(|l| format!("«{l}»")).collect();
        warnings.push(format!(
            "no se ha podido comprobar el sistema de coordenadas de {}{} (no lo declara): se da por bueno",
            shown.join(", "),
            if unknown.len() > 3 {
                format!(" y {} archivos más", unknown.len() - 3)
            } else {
                String::new()
            }
        ));
    }
    Ok(warnings)
}

/// Files in one coordinate system, ready to be combined: those that were in another one are
/// converted into a temporary copy (removed when this is dropped).
#[derive(Debug, Default)]
pub struct Unified {
    moved: Vec<(PathBuf, PathBuf)>,
    /// The system everything is in now, when any file declares one.
    pub target: Option<u16>,
    /// What the user should know: what was converted, what could not be checked.
    pub warnings: Vec<String>,
    /// Vector files in another system, to convert in memory with [`Unified::regions`].
    vectors: Vec<(PathBuf, u16)>,
    dir: Option<PathBuf>,
}

impl Drop for Unified {
    fn drop(&mut self) {
        if let Some(d) = &self.dir {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

impl Unified {
    /// The file to use in place of `p`: its converted copy, or `p` itself.
    pub fn path(&self, p: &Path) -> PathBuf {
        self.moved
            .iter()
            .find(|(from, _)| from == p)
            .map_or_else(|| p.to_path_buf(), |(_, to)| to.clone())
    }

    pub fn all(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        paths.iter().map(|p| self.path(p)).collect()
    }

    /// The regions read from `source`, in the system everything else is in now.
    pub fn regions(
        &self,
        list: &[crate::vector::Region],
        source: Option<&Path>,
    ) -> Result<Vec<crate::vector::Region>> {
        let from = source.and_then(|s| {
            self.vectors
                .iter()
                .find(|(p, _)| p == s)
                .map(|(_, from)| *from)
        });
        match (from, self.target) {
            (Some(from), Some(to)) => crate::reproject::regions(list, from, to),
            _ => Ok(list.to_vec()),
        }
    }

    /// `warnings` as report lines.
    pub fn notes(&self) -> String {
        notes(&self.warnings)
    }
}

fn is_cloud(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("las") || e.eq_ignore_ascii_case("laz"))
}

fn is_raster(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| crate::raster::is_raster_ext(&e.to_string_lossy().to_ascii_lowercase()))
}

/// Puts everything that is going to be combined in one coordinate system.
///
/// The system is the first cloud's (else the first file's that declares one). Clouds and rasters
/// in another system are converted into temporary copies, which cost a pass over them: nothing
/// is converted when the files already agree. With `vectors`, polygon files in another system
/// are left for [`Unified::regions`] to convert in memory, which is nearly free; without it they
/// are an error, as any file that cannot be converted.
pub fn unify<P: AsRef<Path>>(paths: &[P], vectors: bool) -> Result<Unified> {
    unify_like(paths, vectors, None)
}

/// [`unify`] where the rasters that have to move land on the cells of the raster `like` (which
/// sets the system too): for jobs that combine rasters cell by cell.
pub fn unify_like<P: AsRef<Path>>(
    paths: &[P],
    vectors: bool,
    like: Option<&Path>,
) -> Result<Unified> {
    let paths: Vec<&Path> = paths.iter().map(AsRef::as_ref).collect();
    let items: Vec<Item> = paths.iter().map(|p| Item::of(p)).collect();
    let like_grid = match like {
        Some(p) => Some(crate::grid::Grid::read(p)?),
        None => None,
    };
    let target = like.and_then(epsg_of).or_else(|| {
        paths
            .iter()
            .zip(&items)
            .find(|(p, i)| is_cloud(p) && i.epsg.is_some())
            .or_else(|| paths.iter().zip(&items).find(|(_, i)| i.epsg.is_some()))
            .and_then(|(_, i)| i.epsg)
    });
    let mut u = Unified::default();
    u.target = target;
    let mut items = items;
    if let Some(t) = target {
        for (k, (p, item)) in paths.iter().zip(items.iter_mut()).enumerate() {
            let Some(e) = item.epsg else { continue };
            if relation(t, e) != Relation::Different {
                continue;
            }
            if is_raster(p) && crate::pyramid::Pyramid::open(p).is_ok_and(|y| y.bands > 1) {
                return Err(Error::Param(format!(
                    "«{}» es una imagen de varias bandas en {} y las demás capas están en {}: pásala antes al mismo sistema",
                    item.label,
                    name(e),
                    name(t)
                )));
            }
            if is_cloud(p) || is_raster(p) {
                let dir = match &u.dir {
                    Some(d) => d.clone(),
                    None => {
                        let d = std::env::temp_dir().join(format!(
                            "easylidar_crs_{}_{}",
                            std::process::id(),
                            nonce()
                        ));
                        std::fs::create_dir_all(&d)?;
                        u.dir = Some(d.clone());
                        d
                    }
                };
                let file = p
                    .file_name()
                    .map_or_else(|| "datos".to_string(), |n| n.to_string_lossy().into_owned());
                let out = dir.join(format!("{k}_{file}"));
                let what = if is_cloud(p) {
                    let (read, dropped) = crate::reproject::cloud(p, &out, e, t)?;
                    if dropped > 0 {
                        u.warnings.push(format!(
                            "«{}»: {dropped} de {read} puntos caen fuera del sistema {} y se han descartado",
                            item.label,
                            name(t)
                        ));
                    }
                    out
                } else {
                    let g = crate::grid::Grid::read(p)?;
                    let moved = match (&like_grid, like.and_then(epsg_of)) {
                        (Some(l), Some(_)) => crate::reproject::grid_onto(&g, e, t, l.like())?,
                        _ => crate::reproject::grid(&g, e, t)?,
                    };
                    let out = out.with_extension("tif");
                    moved.write_tiff(&out, Some(t))?;
                    out
                };
                u.warnings.push(format!(
                    "«{}» estaba en {} y se ha pasado a {}",
                    item.label,
                    name(e),
                    name(t)
                ));
                u.moved.push((p.to_path_buf(), what));
                item.epsg = Some(t);
            } else if vectors {
                u.warnings.push(format!(
                    "«{}» estaba en {} y se pasa a {}",
                    item.label,
                    name(e),
                    name(t)
                ));
                u.vectors.push((p.to_path_buf(), e));
                item.epsg = Some(t);
            }
        }
    }
    // whatever is left: a file that could not be converted is still an error, and near-equal
    // or undeclared systems still warn
    let rest = check(&items)?;
    u.warnings.extend(rest);
    Ok(u)
}

fn nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// `check` for files.
pub fn check_paths<P: AsRef<Path>>(paths: &[P]) -> Result<Vec<String>> {
    let items: Vec<Item> = paths.iter().map(|p| Item::of(p.as_ref())).collect();
    check(&items)
}

/// The report lines for some warnings: nothing when there are none.
pub fn notes(warnings: &[String]) -> String {
    warnings
        .iter()
        .map(|w| format!("\nAviso: {w}."))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(l: &str, e: Option<u16>) -> Item {
        Item::new(l, e)
    }

    #[test]
    fn names_read_well() {
        assert_eq!(name(25830), "ETRS89 / UTM 30N (EPSG:25830)");
        assert_eq!(name(32719), "WGS 84 / UTM 19S (EPSG:32719)");
        assert_eq!(name(23029), "ED50 / UTM 29N (EPSG:23029)");
        assert_eq!(name(2154), "EPSG:2154");
    }

    #[test]
    fn systems_relate() {
        assert_eq!(relation(25830, 25830), Relation::Same);
        assert_eq!(relation(25830, 32630), Relation::NearlyEqual);
        assert_eq!(relation(32630, 25830), Relation::NearlyEqual);
        assert_eq!(relation(25830, 32629), Relation::Different);
        assert_eq!(relation(25830, 25829), Relation::Different);
        assert_eq!(relation(25830, 23030), Relation::Different);
        assert_eq!(relation(25830, 4258), Relation::Different);
    }

    #[test]
    fn mixing_systems_is_refused_and_says_who() {
        let e = check(&[
            item("nube.laz", Some(25829)),
            item("parcelas.shp", Some(25830)),
        ])
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        assert!(e.contains("nube.laz") && e.contains("parcelas.shp"), "{e}");
        assert!(e.contains("EPSG:25829") && e.contains("EPSG:25830"), "{e}");
    }

    #[test]
    fn unknown_and_near_systems_only_warn() -> Result<()> {
        let w = check(&[item("a.laz", Some(25830)), item("b.tif", None)])?;
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("b.tif"));
        let w = check(&[item("a.laz", Some(25830)), item("b.tif", Some(32630))])?;
        assert_eq!(w.len(), 1);
        assert!(check(&[item("a.laz", Some(25830)), item("b.tif", Some(25830))])?.is_empty());
        // one thing alone has nothing to be compared with
        assert!(check(&[item("a.laz", None)])?.is_empty());
        assert!(notes(&[]).is_empty());
        Ok(())
    }

    #[test]
    fn files_declare_their_system() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_crs_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let mut g = crate::grid::Grid::new(0.0, 0.0, 1.0, 2, 2)?;
        g.data.fill(1.0);
        g.write_tiff(&dir.join("a.tif"), Some(25829))?;
        std::fs::write(
            dir.join("p.prj"),
            crate::vector::prj_for_epsg(25830).unwrap_or_default(),
        )?;
        let w = check_paths(&[dir.join("a.tif"), dir.join("p.shp")]);
        let msg = w.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(msg.contains("a.tif") && msg.contains("p.shp"), "{msg}");
        assert_eq!(epsg_of(&dir.join("p.shp")), Some(25830));
        assert_eq!(epsg_of(&dir.join("nada.shp")), None);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
