//! Rejillas de polo girado: `+proj=ob_tran +o_proj=longlat` de PROJ con `+o_lat_p`, `+o_lon_p` y
//! `+lon_0`, la forma que toman `rotated_latitude_longitude` de CF y las rejillas giradas de GRIB.
//!
//! Como en PROJ, la rotación se aplica a la longitud y la latitud tal como están, sin ninguna
//! conversión de datum ni de geodésica a esférica: sobre una esfera y sobre WGS 84, las coordenadas
//! de una rejilla son las mismas.

use super::math::*;
use super::params::Params;
use crate::{Error, Result};

/// La rotación de una rejilla de polo girado. Las longitudes son relativas a `+lon_0`, que aplica
/// quien llama, como el meridiano central de una proyección.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rotation {
    /// El seno y el coseno de la latitud del polo de la rejilla (`+o_lat_p`).
    sphip: f64,
    cphip: f64,
    /// La longitud de rejilla del polo geográfico (`+o_lon_p`), en radianes.
    lamp: f64,
}

/// Los parámetros `ob_tran` que aplica terrio: `o_proj` y el polo nuevo. Los demás (`o_lon_c`,
/// `o_lat_c`, `o_alpha`, `o_lon_1`...) definen el polo de otras maneras.
const KNOWN: [&str; 3] = ["o_proj", "o_lat_p", "o_lon_p"];

impl Rotation {
    /// La rotación de una cadena PROJ `ob_tran`. Falla con las formas que terrio no aplica: un
    /// `o_proj` distinto de `longlat` o un polo no dado por `o_lat_p` y `o_lon_p`.
    pub fn from_params(p: &Params) -> Result<Self> {
        let o_proj = p
            .str("o_proj")
            .ok_or_else(|| err("+proj=ob_tran without +o_proj"))?;
        if !matches!(o_proj, "longlat" | "latlong" | "lonlat" | "latlon") {
            return Err(err(&format!(
                "+proj=ob_tran with +o_proj={o_proj} (only +o_proj=longlat is)"
            )));
        }
        if let Some(other) = p.keys().find(|k| k.starts_with("o_") && !KNOWN.contains(k)) {
            return Err(err(&format!(
                "+proj=ob_tran with +{other} (only +o_lat_p and +o_lon_p are)"
            )));
        }
        let phip = p
            .angle("o_lat_p")
            .ok_or_else(|| err("+proj=ob_tran without +o_lat_p"))?;
        let (sphip, cphip) = phip.sin_cos();
        Ok(Self {
            sphip,
            cphip,
            lamp: p.angle("o_lon_p").unwrap_or(0.0),
        })
    }

    /// La latitud del polo de la rejilla (`+o_lat_p`) y la longitud de rejilla del polo geográfico
    /// (`+o_lon_p`), en radianes.
    pub fn pole(&self) -> (f64, f64) {
        (self.sphip.atan2(self.cphip), self.lamp)
    }

    /// Las coordenadas de rejilla (radianes) de una longitud, relativa a `+lon_0`, y una latitud
    /// (`o_forward` de PROJ: las fórmulas 5-7 y 5-8b de Snyder).
    pub fn fwd(&self, lam: f64, phi: f64) -> (f64, f64) {
        let (sinphi, cosphi) = phi.sin_cos();
        let (sinlam, coslam) = lam.sin_cos();
        let lam = adjlon(
            (cosphi * sinlam).atan2(self.sphip * cosphi * coslam + self.cphip * sinphi) + self.lamp,
        );
        (
            lam,
            aasin(self.sphip * sinphi - self.cphip * cosphi * coslam),
        )
    }

    /// La longitud, relativa a `+lon_0`, y la latitud (radianes) de unas coordenadas de rejilla
    /// (`o_inverse` de PROJ).
    pub fn inv(&self, lam: f64, phi: f64) -> (f64, f64) {
        let (sinphi, cosphi) = phi.sin_cos();
        let (sinlam, coslam) = (lam - self.lamp).sin_cos();
        let phi = aasin(self.sphip * sinphi + self.cphip * cosphi * coslam);
        (
            (cosphi * sinlam).atan2(self.sphip * cosphi * coslam - self.cphip * sinphi),
            phi,
        )
    }
}

fn err(msg: &str) -> Error {
    Error::Crs(format!("unsupported projection: {msg}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_undoes_forward() {
        let r = Rotation::from_params(&Params::parse(
            "+proj=ob_tran +o_proj=longlat +o_lat_p=39.25 +o_lon_p=10",
        ))
        .unwrap();
        for (lam, phi) in [(0.3, 0.7), (-2.5, -1.2), (3.0, 0.0)] {
            let (x, y) = r.fwd(lam, phi);
            let (l, p) = r.inv(x, y);
            assert!((adjlon(l - lam)).abs() < 1e-12 && (p - phi).abs() < 1e-12);
        }
    }

    #[test]
    fn refuses_other_forms() {
        for s in [
            "+proj=ob_tran +o_proj=merc +o_lat_p=40",
            "+proj=ob_tran +o_proj=longlat +o_lon_c=10 +o_lat_c=40 +o_alpha=5",
        ] {
            assert!(Rotation::from_params(&Params::parse(s)).is_err(), "{s}");
        }
    }
}
