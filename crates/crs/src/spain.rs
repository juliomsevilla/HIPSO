//! Las rejillas de ajuste de datum y los modelos de geoide que necesita España, incorporados al
//! programa para que las transformaciones sean exactas sin descarga ni instalación.
//!
//! - `es_ign_SPED2ETV2`: ED50 a ETRS89, península y Baleares (IGN, CC-BY 4.0).
//! - `es_cat_icgc_100800401`: ED50 a ETRS89, Cataluña (ICGC, CC-BY 4.0).
//! - `es_ign_egm08-rednap` y `es_ign_egm08-rednap-canarias`: geoide EGM08-REDNAP, para convertir
//!   alturas ortométricas en elipsoidales y a la inversa (IGN, CC-BY 4.0).

use std::sync::Once;

const GRIDS: &[(&str, &[u8])] = &[
    (
        "es_ign_SPED2ETV2.tif",
        include_bytes!("../data/grids/es_ign_SPED2ETV2.tif"),
    ),
    (
        "es_cat_icgc_100800401.tif",
        include_bytes!("../data/grids/es_cat_icgc_100800401.tif"),
    ),
    (
        "es_ign_egm08-rednap.tif",
        include_bytes!("../data/grids/es_ign_egm08-rednap.tif"),
    ),
    (
        "es_ign_egm08-rednap-canarias.tif",
        include_bytes!("../data/grids/es_ign_egm08-rednap-canarias.tif"),
    ),
];

/// Registra las rejillas incorporadas. Lo llaman los constructores de [`crate::Transformer`] a
/// través de [`init`]; es barato a partir de la primera vez.
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for (name, bytes) in GRIDS {
            crate::register_grid(name, bytes.to_vec());
        }
    });
}
