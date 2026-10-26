//! Coordinate reference systems and reprojection for EasyLiDAR.
//!
//! The transformation engine (projections, EPSG/ESRI tables, WKT and PROJ strings, datum shift
//! grids) is adapted from the `terrio` project (MIT, Qiusheng Wu), itself ported from PROJ (MIT).
//! The Spanish IGN and ICGC grids and the EGM08-REDNAP geoid are built in (CC-BY 4.0): see
//! [`spain`]. Credits are in the repository's `NOTICE`.

#![allow(dead_code, unused_imports)]

mod crs;
pub mod spain;

pub use crs::*;

/// Errors of this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Crs(String),
    #[error("{0}")]
    Param(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<wbprojection::ProjectionError> for Error {
    fn from(e: wbprojection::ProjectionError) -> Self {
        Error::Crs(e.to_string())
    }
}
