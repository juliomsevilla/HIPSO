//! The datum shift grids and geoid models Spain needs, built into the program so that the
//! transformations are exact without a download or an installation.
//!
//! - `es_ign_SPED2ETV2`: ED50 to ETRS89, mainland and Balearic Islands (IGN, CC-BY 4.0).
//! - `es_cat_icgc_100800401`: ED50 to ETRS89, Catalonia (ICGC, CC-BY 4.0).
//! - `es_ign_egm08-rednap` and `es_ign_egm08-rednap-canarias`: EGM08-REDNAP geoid, to turn
//!   orthometric heights into ellipsoidal ones and back (IGN, CC-BY 4.0).

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

/// Registers the built-in grids. Called by [`crate::Transformer`] constructors through
/// [`init`]; cheap after the first time.
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for (name, bytes) in GRIDS {
            crate::register_grid(name, bytes.to_vec());
        }
    });
}
