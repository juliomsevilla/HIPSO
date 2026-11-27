//! El motor de HIPSO: productos forestales a partir de nubes de puntos LiDAR, leídas como flujo
//! para que la memoria dependa del área y del tamaño de celda, nunca del número de puntos.

pub mod aba;
pub mod alpha3d;
pub mod axelsson;
pub mod breaklines;
pub mod buildings;
pub mod canopygaps;
pub mod catalog;
pub mod change;
pub mod chm;
pub mod classes;
pub mod clip;
pub mod cluster;
pub mod cloudtools;
pub mod cog;
pub mod colorize;
pub mod copc;
pub mod copc_cache;
pub mod editops;
pub mod geoproc;
pub mod chartstats;
pub mod geomedit;
pub mod topofix;
pub mod topology;
pub mod flowrun;
pub mod model;
pub mod rasterproc;
pub mod copc_detail;
pub mod copc_view;
pub mod copc_tiled;
pub mod copc_write;
pub mod corridor;
pub mod crs;
pub mod csf;
pub mod cutfill;
pub mod dtmcheck;
pub mod e57;
pub mod edit;
pub mod export;
pub mod extras;
pub mod features;
pub mod fieldcalc;
pub mod fieldtype;
pub mod filter;
pub mod flightlines;
pub mod flood;
pub mod footprint;
pub mod forest;
pub mod geojson;
pub mod geomops;
pub mod gpkg;
pub mod gpkg_write;
pub mod grid;
pub mod ground;
pub mod hag;
pub mod histogram;
#[cfg(test)]
mod hostile;
pub mod hydro;
pub mod idw;
pub mod inspect;
pub mod intensity;
pub mod jobs;
pub mod jp2;
pub mod json;
pub mod kml;
pub mod las;
pub mod markup;
pub mod mcc;
pub mod metrics;
pub mod noise;
pub mod obia;
pub mod ponds;
pub mod rastervec;
pub mod reclass;
pub mod recipe;
pub mod merge;
pub mod ops;
pub mod overlap;
pub mod pathfind;
pub mod paths;
pub mod piles;
pub mod pipeline;
pub mod plots;
pub mod ply;
pub mod pmf;
pub mod pyramid;
pub mod raster;
pub mod rastercalc;
pub mod rasterfmt;
pub mod rastermask;
pub mod rasterops;
pub mod rastertools;
pub mod reproject;
pub mod rtree;
pub mod rules;
pub mod sheets;
pub mod smrf;
pub mod sqlite;
pub mod stools;
pub mod terrain;
pub mod terrainlines;
pub mod layeredit;
pub mod propagate;
pub mod thin;
pub(crate) mod tiffio;
pub mod tile;
pub mod tiled;
pub mod tilemap;
pub mod tin;
pub mod tls;
pub mod trees;
pub mod trees3d;
pub mod treemetrics;
pub mod vdensity;
pub mod vecops;
pub mod vector;
pub mod view;
pub mod viewshed;
pub mod volume;
pub mod voronoi;
pub mod voxelmetrics;
pub mod wand;
pub mod xlsx;
pub mod zonal;

/// Todo lo que puede salir mal; los archivos de entrada son hostiles, así que nada de aquí entra en
/// pánico.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Format(String),
    Unsupported(String),
    Param(String),
    /// El usuario detuvo el trabajo.
    Cancelled,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => f.write_str(&in_spanish(&e.to_string())),
            Error::Format(m) | Error::Unsupported(m) | Error::Param(m) => {
                f.write_str(&in_spanish(m))
            }
            Error::Cancelled => f.write_str("cancelado por el usuario"),
        }
    }
}

/// Los mensajes del sistema operativo y de las bibliotecas que leen archivos, que vienen en inglés,
/// dichos en español.
fn in_spanish(m: &str) -> String {
    const WORDS: &[(&str, &str)] = &[
        ("IoError: ", ""),
        (
            "failed to fill whole buffer",
            "el archivo se acaba antes de tiempo: está incompleto o dañado",
        ),
        (
            "unexpected end of file",
            "el archivo se acaba antes de tiempo: está incompleto o dañado",
        ),
        (
            "No such file or directory",
            "no existe el archivo o la carpeta",
        ),
        (
            "The system cannot find the file specified.",
            "no existe el archivo",
        ),
        (
            "The system cannot find the path specified.",
            "no existe la carpeta",
        ),
        // Windows en español ya lo dice en español, pero con sus propias palabras
        (
            "El sistema no puede encontrar el archivo especificado.",
            "no existe el archivo",
        ),
        (
            "El sistema no puede encontrar la ruta especificada.",
            "no existe la carpeta",
        ),
        ("Permission denied", "permiso denegado"),
        ("Access is denied.", "acceso denegado"),
        ("No space left on device", "no queda espacio en el disco"),
        (
            "There is not enough space on the disk.",
            "no queda espacio en el disco",
        ),
        ("Is a directory", "es una carpeta, no un archivo"),
        ("File exists", "el archivo ya existe"),
        ("Read-only file system", "el disco es de solo lectura"),
        ("Operation timed out", "la lectura ha tardado demasiado"),
    ];
    let mut out = m.to_string();
    for (en, es) in WORDS {
        if out.contains(en) {
            out = out.replace(en, es);
        }
    }
    out
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Abre un archivo para leerlo, con su ruta en el error (un «no existe el archivo» a secas no dice
/// cuál).
pub(crate) fn open_file(path: &std::path::Path) -> Result<std::fs::File> {
    std::fs::File::open(path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}{}", path.display(), cloud_hint(&e.to_string())),
        ))
    })
}

/// Una pista para los errores de lectura que vienen de un archivo que solo está en la nube
/// (OneDrive, iCloud…) y no se ha descargado: el sistema responde a las lecturas con un tiempo de
/// espera agotado o «no disponible».
pub(crate) fn cloud_hint(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    if m.contains("timed out") || m.contains("os error 60") || m.contains("os error 110") {
        " (¿el archivo está solo en la nube, en OneDrive o iCloud? Descárgalo en el equipo antes de abrirlo)"
    } else {
        ""
    }
}

/// Rechaza, con un mensaje claro, un ráster en un formato que aquí no se lee.
pub(crate) fn check_raster_format(path: &std::path::Path) -> Result<()> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let convert = "Conviértelo antes a GeoTIFF o JPEG 2000 con QGIS, GeoLibre o RustGis (en QGIS: Ráster → Conversión → Traducir)";
    let msg = match ext.as_str() {
        "ecw" => format!(
            "formato ECW, propietario: solo se puede leer con un SDK que no es de código abierto y no cabe en un programa en Rust puro. {convert}"
        ),
        "sid" => format!("formato MrSID, propietario: no se lee. {convert}"),
        "nc" | "hdf" | "h5" | "he5" | "grib" | "grb" => {
            format!("formato científico multidimensional, que no se lee. {convert}")
        }
        "img" if rasterfmt::kind_of(path).is_none() => {
            format!("formato Imagine (.img), que no se lee. {convert}")
        }
        "gpkg" | "vrt" | "kea" | "ntf" | "rsw" | "mrf" => {
            format!("formato ráster que no se lee. {convert}")
        }
        _ => return Ok(()),
    };
    Err(Error::Unsupported(format!("{}: {msg}", path.display())))
}

/// Rechaza, con un mensaje claro, una capa vectorial en un formato que aquí no se lee.
pub(crate) fn check_vector_format(path: &std::path::Path) -> Result<()> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name = match ext.as_str() {
        "gml" => "GML",
        "gdb" => "geodatabase de Esri",
        "dxf" | "dwg" => "CAD",
        "fgb" => "FlatGeobuf",
        _ => return Ok(()),
    };
    Err(Error::Unsupported(format!(
        "{}: formato {name}, que no se lee todavía. Expórtalo a shapefile o GeoJSON con QGIS, GeoLibre o RustGis (en QGIS: clic derecho en la capa → Exportar)",
        path.display()
    )))
}

pub type Result<T> = std::result::Result<T, Error>;

static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Pide a todo trabajo en marcha que se detenga en su siguiente lote de puntos o tesela.
pub fn request_cancel() {
    CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Borra una petición de parada (llámese antes de empezar un trabajo).
pub fn reset_cancel() {
    CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
}

pub fn is_cancelled() -> bool {
    CANCEL.load(std::sync::atomic::Ordering::Relaxed)
}

/// `Err(Cancelled)` cuando se pidió una parada.
pub fn check_cancel() -> Result<()> {
    if is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// Cuánto lleva el trabajo en marcha, para una estimación de tiempo: `done` de `total` unidades de
/// trabajo (puntos leídos, teselas hechas). Los trabajos que conocen su tamaño llaman a
/// [`progress::begin`] y [`progress::advance`]; los demás dejan el total en 0 y la interfaz no
/// muestra estimación.
pub mod progress {
    use std::sync::atomic::{AtomicU64, Ordering};

    static DONE: AtomicU64 = AtomicU64::new(0);
    static TOTAL: AtomicU64 = AtomicU64::new(0);

    /// Empieza a contar un trabajo de `total` unidades.
    pub fn begin(total: u64) {
        DONE.store(0, Ordering::Relaxed);
        TOTAL.store(total, Ordering::Relaxed);
    }

    /// Como [`begin`], salvo que el trabajo ya haya dicho su tamaño (un bucle sobre archivos que
    /// los declararon todos antes).
    pub fn begin_if_idle(total: u64) {
        if TOTAL.load(Ordering::Relaxed) == 0 {
            begin(total);
        }
    }

    /// Suma `units` de trabajo terminado.
    #[inline]
    pub fn advance(units: u64) {
        DONE.fetch_add(units, Ordering::Relaxed);
    }

    /// Olvida el último trabajo (llámese antes de empezar otro).
    pub fn reset() {
        begin(0);
    }

    /// La fracción hecha (0 a 1), cuando el trabajo en marcha dijo su tamaño.
    pub fn fraction() -> Option<f64> {
        let total = TOTAL.load(Ordering::Relaxed);
        (total > 0).then(|| (DONE.load(Ordering::Relaxed) as f64 / total as f64).min(1.0))
    }
}
