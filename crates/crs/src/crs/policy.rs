//! What a transformation gave up to produce a value, and the policy that decides whether that is
//! acceptable.
//!
//! The engine falls back, silently, when the exact operation cannot run: no datum shift at all
//! (a "ballpark"), heights kept because no geoid model covers the point, or the next-best
//! operation because a grid is missing or does not cover the point. Each fallback is noted here,
//! per thread, while a point is transformed; [`crate::Transformer::transform_checked`] reads the
//! notes and applies a [`Policy`].

use std::cell::Cell;

/// One way a transformation can fall short of the best operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Approximation {
    /// No datum shift was applied: the horizontal coordinates are only moved between projections,
    /// an error of metres to hundreds of metres.
    NoDatumShift,
    /// The best listed operation could not run (grid missing, or point outside it) and a less
    /// accurate one was used.
    NextBestOperation,
    /// No geoid model or vertical transformation covers the point: the height was kept, only its
    /// unit changed. The error can be tens of metres.
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

    /// A sentence for the user.
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

/// How strict a transformation is about approximations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Accept [`Approximation::NoDatumShift`] and [`Approximation::HeightKept`].
    pub allow_ballpark: bool,
    /// Refuse [`Approximation::NextBestOperation`]: only the best operation counts.
    pub only_best: bool,
}

impl Default for Policy {
    /// The strict policy: no ballpark, only the best operation.
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

    /// Whether `a` is acceptable.
    pub fn accepts(self, a: Approximation) -> bool {
        match a {
            Approximation::NoDatumShift | Approximation::HeightKept => self.allow_ballpark,
            Approximation::NextBestOperation => !self.only_best,
        }
    }

    /// The policy from the environment, over the strict defaults: `HIPSO_ALLOW_BALLPARK` and
    /// `HIPSO_ONLY_BEST` (`YES`/`NO`, `1`/`0`, `TRUE`/`FALSE`).
    pub fn from_env() -> Policy {
        let flag = |name: &str, default: bool| match std::env::var(name) {
            Ok(v) => match v.trim().to_ascii_uppercase().as_str() {
                "YES" | "1" | "TRUE" | "ON" => true,
                "NO" | "0" | "FALSE" | "OFF" => false,
                _ => default,
            },
            Err(_) => default,
        };
        Policy {
            allow_ballpark: flag("HIPSO_ALLOW_BALLPARK", Policy::STRICT.allow_ballpark),
            only_best: flag("HIPSO_ONLY_BEST", Policy::STRICT.only_best),
        }
    }
}

/// The approximations a point went through.
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
    /// The first approximation the policy refuses.
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

/// Notes an approximation on this thread.
pub(crate) fn note(a: Approximation) {
    NOTES.with(|n| n.set(n.get() | a.bit()));
}

/// The notes of this thread, without clearing them.
pub(crate) fn peek() -> u8 {
    NOTES.with(Cell::get)
}

/// Clears the no-datum-shift note unless it was set in `before`.
pub(crate) fn restore_ballpark(before: u8) {
    if before & Approximation::NoDatumShift.bit() == 0 {
        NOTES.with(|n| n.set(n.get() & !Approximation::NoDatumShift.bit()));
    }
}

/// Takes (and clears) the notes of this thread.
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
        // Paris: the Spanish geoid does not cover it
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
        // A CRS defined by its ellipsoid alone has no datum to shift from.
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
        // and back
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
        // Tenerife: EGM08-REDNAP Canarias; ETRS89 and REGCAN95 coincide.
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
