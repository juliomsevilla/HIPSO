//! Sistemas de referencia de coordenadas y reproyección para HIPSO.
//!
//! El motor de transformación (proyecciones, tablas EPSG/ESRI, WKT y cadenas PROJ, rejillas de
//! ajuste de datum) está adaptado del proyecto `terrio` (MIT, Qiusheng Wu), a su vez portado de
//! PROJ (MIT). Las rejillas del IGN y del ICGC y el geoide EGM08-REDNAP están incorporados (CC-BY
//! 4.0): véase [`spain`]. Los créditos están en el `NOTICE` del repositorio.

#![allow(dead_code, unused_imports)]

mod crs;
pub mod spain;

pub use crs::*;

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Crs(String),
    #[error("{0}")]
    Param(String),
    /// La [`Policy`] rechazó la transformación de un punto (una aproximación o una operación
    /// inferior).
    #[error("{0}")]
    Policy(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<wbprojection::ProjectionError> for Error {
    fn from(e: wbprojection::ProjectionError) -> Self {
        Error::Crs(e.to_string())
    }
}
