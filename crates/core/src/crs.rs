//! Sistemas de coordenadas: en cuál está cada archivo y si los archivos se pueden combinar. No se
//! reproyecta nada; lo que importa es que dos conjuntos de datos en sistemas distintos nunca se
//! mezclen sin avisar: un polígono en UTM 30 sobre una nube en UTM 29 no recorta nada (o recorta el
//! sitio equivocado), y un ráster desplazado una zona está a cientos de kilómetros.

use std::path::{Path, PathBuf};

use crate::las::Reader;
use crate::{Error, Result};

/// Un archivo y el sistema que declara, si declara uno.
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

    /// El sistema que declara un archivo: la cabecera de una nube, las claves de un GeoTIFF, el
    /// `.prj` de un shapefile (o de una rejilla ASCII). Un archivo que no se puede leer no tiene
    /// ninguno.
    pub fn of(path: &Path) -> Self {
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self::new(label, epsg_of(path))
    }
}

/// El código EPSG que declara un archivo, si declara alguno.
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
        "kml" | "kmz" => Some(4326),
        _ => std::fs::read_to_string(path.with_extension("prj"))
            .ok()
            .and_then(|t| crate::vector::epsg_from_prj(&t)),
    }
}

/// El código EPSG de la referencia vertical que declara un archivo, si declara alguna (LAS/LAZ y
/// GeoTIFF).
pub fn vertical_epsg_of(path: &Path) -> Option<u16> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "las" | "laz" => Reader::open(path).ok().and_then(|r| r.header.vertical_epsg),
        "tif" | "tiff" => crate::grid::Grid::read_vertical_epsg(path),
        _ => None,
    }
}

/// Un nombre legible para un código, con el código: `ETRS89 / UTM 30N (EPSG:25830)`.
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

/// Cómo se relacionan dos sistemas declarados.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Same,
    /// ETRS89 y WGS 84 UTM de una misma zona: difieren en aproximadamente un metro en Europa, lo
    /// que no importa para nada a la escala de los productos forestales, pero conviene decirlo.
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

/// Comprueba que todo lo que se va a combinar está en un solo sistema.
///
/// - Dos elementos en sistemas distintos: un error que los nombra, pues el resultado sería erróneo.
/// - Elementos que no declaran nada: no se pueden comprobar, así que un aviso lo dice.
/// - ETRS89 contra WGS 84 de la misma zona: un aviso.
///
/// Devuelve los avisos, para que quien llama los ponga en su informe.
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
        // una lista larga no dice más que sus primeros nombres
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

/// Archivos en un solo sistema de coordenadas, listos para combinarse: los que estaban en otro se
/// convierten en una copia temporal (que se borra cuando esto se suelta).
#[derive(Debug, Default)]
pub struct Unified {
    moved: Vec<(PathBuf, PathBuf)>,
    /// El sistema en que está todo ahora, cuando algún archivo declara uno.
    pub target: Option<u16>,
    /// Lo que debe saber el usuario: qué se convirtió, qué no se pudo comprobar.
    pub warnings: Vec<String>,
    /// Archivos vectoriales en otro sistema, que convertir en memoria con [`Unified::regions`].
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
    /// El archivo que usar en lugar de `p`: su copia convertida o el propio `p`.
    pub fn path(&self, p: &Path) -> PathBuf {
        self.moved
            .iter()
            .find(|(from, _)| from == p)
            .map_or_else(|| p.to_path_buf(), |(_, to)| to.clone())
    }

    pub fn all(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        paths.iter().map(|p| self.path(p)).collect()
    }

    /// Las regiones leídas de `source`, en el sistema en que está ahora todo lo demás.
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

    /// `warnings` como líneas de informe.
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

/// Pone todo lo que se va a combinar en un solo sistema de coordenadas.
///
/// El sistema es el de la primera nube (o, si no, el del primer archivo que declara uno). Las nubes
/// y los rásteres en otro sistema se convierten en copias temporales, que cuestan una pasada sobre
/// ellos: no se convierte nada cuando los archivos ya concuerdan. Con `vectors`, los archivos de
/// polígonos en otro sistema se dejan para que [`Unified::regions`] los convierta en memoria, lo
/// que es casi gratis; sin él son un error, como todo archivo que no se puede convertir.
pub fn unify<P: AsRef<Path>>(paths: &[P], vectors: bool) -> Result<Unified> {
    unify_like(paths, vectors, None)
}

/// [`unify`] donde los rásteres que tienen que moverse caen sobre las celdas del ráster `like` (que
/// fija también el sistema): para trabajos que combinan rásteres celda a celda.
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
                            "hipso_crs_{}_{}",
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
                        (Some(l), Some(_)) => {
                            crate::reproject::grid_of_file_onto(p, &g, e, t, l.like())?
                        }
                        _ => {
                            let (b, cell) = crate::reproject::target_frame_of(
                                [g.x0, g.y0, g.cell], g.cols, g.rows, e, t,
                            )?;
                            let out = crate::grid::Grid::covering(b, cell)?;
                            crate::reproject::grid_of_file_onto(p, &g, e, t, out)?
                        }
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
    // lo que quede: un archivo que no se pudo convertir sigue siendo un error, y los sistemas casi
    // iguales o no declarados siguen avisando
    let rest = check(&items)?;
    u.warnings.extend(rest);
    Ok(u)
}

fn nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// `check` para archivos.
pub fn check_paths<P: AsRef<Path>>(paths: &[P]) -> Result<Vec<String>> {
    let items: Vec<Item> = paths.iter().map(|p| Item::of(p.as_ref())).collect();
    check(&items)
}

/// Las líneas de informe de algunos avisos: nada cuando no los hay.
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
            item("muestras.shp", Some(25830)),
        ])
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        assert!(e.contains("nube.laz") && e.contains("muestras.shp"), "{e}");
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
        // una cosa sola no tiene con qué compararse
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
