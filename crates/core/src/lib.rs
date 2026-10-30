//! EasyLiDAR's engine: forestry products from LiDAR point clouds, read as a stream so that
//! memory depends on the area and cell size, never on the number of points.

pub mod aba;
pub mod axelsson;
pub mod breaklines;
pub mod buildings;
pub mod change;
pub mod chm;
pub mod classes;
pub mod clip;
pub mod cloudtools;
pub mod cog;
pub mod colorize;
pub mod copc;
pub mod copc_cache;
pub mod copc_detail;
pub mod copc_tiled;
pub mod copc_write;
pub mod corridor;
pub mod crs;
pub mod csf;
pub mod e57;
pub mod edit;
pub mod export;
pub mod extras;
pub mod filter;
pub mod footprint;
pub mod forest;
pub mod geojson;
pub mod gpkg;
pub mod gpkg_write;
pub mod grid;
pub mod ground;
pub mod hag;
pub mod histogram;
pub mod hydro;
pub mod idw;
pub mod inspect;
pub mod intensity;
pub mod jobs;
pub mod jp2;
pub mod json;
pub mod las;
pub mod markup;
pub mod mcc;
pub mod metrics;
pub mod noise;
pub mod obia;
pub mod ops;
pub mod paths;
pub mod pipeline;
pub mod plots;
pub mod ply;
pub mod pmf;
pub mod pyramid;
pub mod raster;
pub mod rasterfmt;
pub mod rastertools;
pub mod reproject;
pub mod rtree;
pub mod rules;
pub mod sheets;
pub mod smrf;
pub mod sqlite;
pub mod stools;
pub mod terrain;
pub mod thin;
pub mod tile;
pub mod tiled;
pub mod tilemap;
pub mod tin;
pub mod tls;
pub mod trees;
pub mod vector;
pub mod view;
pub mod viewshed;
pub mod volume;
pub mod xlsx;

/// Everything that can go wrong; input files are hostile, so nothing here panics.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Format(String),
    Unsupported(String),
    Param(String),
    /// The user stopped the job.
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

/// The messages of the operating system and of the libraries that read files, which come in
/// English, said in Spanish.
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
        // Windows in Spanish already says it in Spanish, but with its own words
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

/// Opens a file to read, with its path in the error (a bare "no such file" does not say which).
pub(crate) fn open_file(path: &std::path::Path) -> Result<std::fs::File> {
    std::fs::File::open(path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}{}", path.display(), cloud_hint(&e.to_string())),
        ))
    })
}

/// A hint for read errors that come from a file that is only in the cloud (OneDrive, iCloud…) and
/// has not been downloaded: the system answers the reads with a timeout or "not available".
pub(crate) fn cloud_hint(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    if m.contains("timed out") || m.contains("os error 60") || m.contains("os error 110") {
        " (¿el archivo está solo en la nube, en OneDrive o iCloud? Descárgalo en el equipo antes de abrirlo)"
    } else {
        ""
    }
}

/// Refuses, with a clear message, a raster in a format that is not read here.
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

/// Refuses, with a clear message, a vector layer in a format that is not read here.
pub(crate) fn check_vector_format(path: &std::path::Path) -> Result<()> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name = match ext.as_str() {
        "kml" | "kmz" => "KML",
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

/// Asks every running job to stop at its next batch of points or tile.
pub fn request_cancel() {
    CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Clears a stop request (call before starting a job).
pub fn reset_cancel() {
    CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
}

pub fn is_cancelled() -> bool {
    CANCEL.load(std::sync::atomic::Ordering::Relaxed)
}

/// `Err(Cancelled)` when a stop was requested.
pub fn check_cancel() -> Result<()> {
    if is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// How far the running job is, for a time estimate: `done` of `total` units of work (points read,
/// tiles done). Jobs that know their size call [`progress::begin`] and [`progress::advance`];
/// the others leave the total at 0, and the interface shows no estimate.
pub mod progress {
    use std::sync::atomic::{AtomicU64, Ordering};

    static DONE: AtomicU64 = AtomicU64::new(0);
    static TOTAL: AtomicU64 = AtomicU64::new(0);

    /// Starts counting a job of `total` units.
    pub fn begin(total: u64) {
        DONE.store(0, Ordering::Relaxed);
        TOTAL.store(total, Ordering::Relaxed);
    }

    /// Like [`begin`], unless the job already said its size (a loop over files that declared them
    /// all first).
    pub fn begin_if_idle(total: u64) {
        if TOTAL.load(Ordering::Relaxed) == 0 {
            begin(total);
        }
    }

    /// Adds `units` of finished work.
    #[inline]
    pub fn advance(units: u64) {
        DONE.fetch_add(units, Ordering::Relaxed);
    }

    /// Forgets the last job (call before starting another).
    pub fn reset() {
        begin(0);
    }

    /// The fraction done (0 to 1), when the running job said its size.
    pub fn fraction() -> Option<f64> {
        let total = TOTAL.load(Ordering::Relaxed);
        (total > 0).then(|| (DONE.load(Ordering::Relaxed) as f64 / total as f64).min(1.0))
    }
}
