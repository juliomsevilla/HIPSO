//! A qué renunció una transformación para producir un valor, y la política que decide si eso es
//! aceptable.
//!
//! El motor recurre, en silencio, a una alternativa cuando la operación exacta no puede ejecutarse:
//! ningún ajuste de datum (una «aproximación»), alturas conservadas porque ningún modelo de geoide
//! cubre el punto, o la siguiente mejor operación porque falta una rejilla o no cubre el punto.
//! Cada alternativa se anota aquí, por hilo, mientras se transforma un punto;
//! [`crate::Transformer::transform_checked`] lee las notas y aplica una [`Policy`].

use std::cell::Cell;

/// Una forma en que una transformación puede quedarse corta respecto a la mejor operación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Approximation {
    /// No se aplicó ningún ajuste de datum: las coordenadas horizontales solo se mueven entre
    /// proyecciones, un error de metros a cientos de metros.
    NoDatumShift,
    /// La mejor operación listada no pudo ejecutarse (falta la rejilla o el punto está fuera de
    /// ella) y se usó una menos precisa.
    NextBestOperation,
    /// Ningún modelo de geoide ni transformación vertical cubre el punto: la altura se conservó y
    /// solo cambió su unidad. El error puede ser de decenas de metros.
    HeightKept,
}

impl Approximation {
    pub const ALL: [Approximation; 3] = [
        Approximation::NoDatumShift,
        Approximation::NextBestOperation,
        Approximation::HeightKept,
    ];

    const fn bit(self) -> u8 {
        match self {
            Approximation::NoDatumShift => 1,
            Approximation::NextBestOperation => 2,
            Approximation::HeightKept => 4,
        }
    }

    /// Una frase para el usuario.
    pub fn describe(self) -> &'static str {
        match self {
            Approximation::NoDatumShift => {
                "sin transformación de datum (se mueven solo las proyecciones): error de metros o más"
            }
            Approximation::NextBestOperation => {
                "la mejor transformación no pudo aplicarse (rejilla ausente o punto fuera de ella): se usó otra menos precisa"
            }
            Approximation::HeightKept => {
                "sin modelo de geoide para el punto: la altura se conserva sin corregir"
            }
        }
    }
}

/// Cuán estricta es una transformación con las aproximaciones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Acepta [`Approximation::NoDatumShift`] y [`Approximation::HeightKept`].
    pub allow_ballpark: bool,
    /// Rechaza [`Approximation::NextBestOperation`]: solo cuenta la mejor operación.
    pub only_best: bool,
}

impl Default for Policy {
    /// La política estricta: sin aproximación, solo la mejor operación.
    fn default() -> Self {
        Policy::STRICT
    }
}

impl Policy {
    pub const STRICT: Policy = Policy {
        allow_ballpark: false,
        only_best: true,
    };
    pub const LENIENT: Policy = Policy {
        allow_ballpark: true,
        only_best: false,
    };

    /// Si `a` es aceptable.
    pub fn accepts(self, a: Approximation) -> bool {
        match a {
            Approximation::NoDatumShift | Approximation::HeightKept => self.allow_ballpark,
            Approximation::NextBestOperation => !self.only_best,
        }
    }

    /// La política del entorno, sobre los valores estrictos por defecto: `HIPSO_ALLOW_BALLPARK` y
    /// `HIPSO_ONLY_BEST` (`YES`/`NO`, `1`/`0`, `TRUE`/`FALSE`).
    pub fn from_env() -> Policy {
        let set = overrides().lock().map(|o| *o).unwrap_or_default();
        let flag = |name: &str, default: bool, fixed: Option<bool>| match fixed.map_or_else(|| std::env::var(name), |b| Ok(b.to_string())) {
            Ok(v) => match v.trim().to_ascii_uppercase().as_str() {
                "YES" | "1" | "TRUE" | "ON" => true,
                "NO" | "0" | "FALSE" | "OFF" => false,
                _ => default,
            },
            Err(_) => default,
        };
        Policy {
            allow_ballpark: flag("HIPSO_ALLOW_BALLPARK", Policy::STRICT.allow_ballpark, set.0),
            only_best: flag("HIPSO_ONLY_BEST", Policy::STRICT.only_best, set.1),
        }
    }
}

/// Los valores que la aplicación fija en lugar de las variables de entorno (`None`: la variable).
fn overrides() -> &'static std::sync::Mutex<(Option<bool>, Option<bool>)> {
    static O: std::sync::OnceLock<std::sync::Mutex<(Option<bool>, Option<bool>)>> =
        std::sync::OnceLock::new();
    O.get_or_init(|| std::sync::Mutex::new((None, None)))
}

/// Fija la política de aproximaciones de la aplicación en lugar de `HIPSO_ALLOW_BALLPARK` y
/// `HIPSO_ONLY_BEST`; `None` en un campo deja actuar a la variable de entorno.
pub fn set_policy_overrides(allow_ballpark: Option<bool>, only_best: Option<bool>) {
    if let Ok(mut o) = overrides().lock() {
        *o = (allow_ballpark, only_best);
    }
}

/// Las aproximaciones por las que pasó un punto.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Approximations(u8);

impl Approximations {
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn contains(self, a: Approximation) -> bool {
        self.0 & a.bit() != 0
    }
    pub fn iter(self) -> impl Iterator<Item = Approximation> {
        Approximation::ALL.into_iter().filter(move |a| self.contains(*a))
    }
    /// La primera aproximación que rechaza la política.
    pub fn refused_by(self, policy: Policy) -> Option<Approximation> {
        self.iter().find(|a| !policy.accepts(*a))
    }
    pub(crate) fn from_bits(b: u8) -> Self {
        Approximations(b)
    }
}

thread_local! {
    static NOTES: Cell<u8> = const { Cell::new(0) };
}

/// Anota una aproximación en este hilo.
pub(crate) fn note(a: Approximation) {
    NOTES.with(|n| n.set(n.get() | a.bit()));
}

/// Las notas de este hilo, sin borrarlas.
pub(crate) fn peek() -> u8 {
    NOTES.with(Cell::get)
}

/// Borra la nota de ausencia de ajuste de datum salvo que estuviera fijada en `before`.
pub(crate) fn restore_ballpark(before: u8) {
    if before & Approximation::NoDatumShift.bit() == 0 {
        NOTES.with(|n| n.set(n.get() & !Approximation::NoDatumShift.bit()));
    }
}

/// Toma (y borra) las notas de este hilo.
pub(crate) fn take() -> Approximations {
    Approximations(NOTES.with(|n| n.replace(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SpatialRef, Transformer};

    fn t(from: &str, to: &str) -> Transformer {
        Transformer::new(
            &SpatialRef::parse(from).unwrap(),
            &SpatialRef::parse(to).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn madrid_geoid_is_exact_and_clean() {
        let tr = t("EPSG:4937", "EPSG:25830+5782");
        let ((_, _, h), notes) = tr
            .transform_checked(-3.7038, 40.4168, 0.0, Policy::STRICT)
            .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert!((h + 51.11).abs() < 0.01, "{h}");
    }

    #[test]
    fn outside_geoid_coverage_strict_fails_lenient_warns() {
        let tr = t("EPSG:4937", "EPSG:25831+5782");
        // París: el geoide español no lo cubre
        let strict = tr.transform_checked(2.3522, 48.8566, 0.0, Policy::STRICT);
        assert!(strict.is_err(), "{strict:?}");
        let ((_, _, h), notes) = tr
            .transform_checked(2.3522, 48.8566, 0.0, Policy::LENIENT)
            .unwrap();
        assert!(notes.contains(Approximation::HeightKept));
        assert_eq!(h, 0.0);
    }

    #[test]
    fn unknown_datum_is_noted_and_refused_when_strict() {
        // Un sistema definido solo por su elipsoide no tiene datum desde el que desplazar.
        let tr = t("+proj=utm +zone=30 +ellps=intl +units=m +no_defs", "EPSG:25830");
        let (x, y) = (500_000.0, 4_400_000.0);
        let (_, n) = tr.transform_checked(x, y, 0.0, Policy::LENIENT).unwrap();
        assert!(n.contains(Approximation::NoDatumShift), "{n:?}");
        assert!(tr.transform_checked(x, y, 0.0, Policy::STRICT).is_err());
    }
}

#[cfg(test)]
mod ellipsoidal {
    use super::*;
    use crate::{SpatialRef, TransformOptions, Transformer};

    #[test]
    fn projected_ellipsoidal_height_to_orthometric() {
        let a = SpatialRef::parse("EPSG:25830").unwrap();
        let b = SpatialRef::parse("EPSG:25830+5782").unwrap();
        let opts = TransformOptions {
            source_ellipsoidal_height: true,
            ..Default::default()
        };
        let tr = Transformer::with_options(&a, &b, &opts).unwrap();
        // Madrid, UTM 30N
        let ((_, _, h), n) = tr
            .transform_checked(440_295.0, 4_474_235.0, 700.0, Policy::STRICT)
            .unwrap();
        assert!(n.is_empty());
        assert!((h - 648.89).abs() < 0.02, "{h}");
        // y de vuelta
        let back = Transformer::with_options(
            &b,
            &a,
            &TransformOptions {
                target_ellipsoidal_height: true,
                ..Default::default()
            },
        )
        .unwrap();
        let ((_, _, z), _) = back
            .transform_checked(440_295.0, 4_474_235.0, h, Policy::STRICT)
            .unwrap();
        assert!((z - 700.0).abs() < 1e-3, "{z}");
    }
}

#[cfg(test)]
mod canarias {
    use super::*;
    use crate::{SpatialRef, Transformer};

    #[test]
    fn canarias_geoid_is_clean_under_strict_policy() {
        // Tenerife: EGM08-REDNAP Canarias; ETRS89 y REGCAN95 coinciden.
        let a = SpatialRef::parse("EPSG:4937").unwrap();
        let b = SpatialRef::parse("EPSG:25828+9398").unwrap();
        let tr = Transformer::new(&a, &b).unwrap();
        let ((_, _, h), n) = tr
            .transform_checked(-16.25, 28.47, 0.0, Policy::STRICT)
            .unwrap();
        assert!(n.is_empty(), "{n:?}");
        assert!((h + 44.07).abs() < 0.05, "{h}");
    }
}
