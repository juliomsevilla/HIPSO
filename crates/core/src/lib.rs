//! EasyLiDAR's engine: forestry products from LiDAR point clouds, read as a stream so that
//! memory depends on the area and cell size, never on the number of points.

pub mod axelsson;
pub mod buildings;
pub mod chm;
pub mod classes;
pub mod clip;
pub mod cloudtools;
pub mod csf;
pub mod edit;
pub mod export;
pub mod extras;
pub mod filter;
pub mod forest;
pub mod grid;
pub mod ground;
pub mod histogram;
pub mod idw;
pub mod inspect;
pub mod jobs;
pub mod las;
pub mod markup;
pub mod mcc;
pub mod metrics;
pub mod noise;
pub mod ops;
pub mod paths;
pub mod pipeline;
pub mod plots;
pub mod pmf;
pub mod pyramid;
pub mod raster;
pub mod rastertools;
pub mod rules;
pub mod sheets;
pub mod stools;
pub mod thin;
pub mod tile;
pub mod tin;
pub mod tls;
pub mod trees;
pub mod vector;
pub mod view;
pub mod volume;

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
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(m) | Error::Unsupported(m) | Error::Param(m) => f.write_str(m),
            Error::Cancelled => f.write_str("cancelado por el usuario"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
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
