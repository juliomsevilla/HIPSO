//! The rows that choose a file in the windows of the tools: a raster or a vector layer, either
//! from a file or from the layers already on the map.

use super::*;
use hipso_core::paths;

fn pick(
    ui: &mut egui::Ui,
    label: &str,
    slot: &mut Option<PathBuf>,
    loaded: &[(String, PathBuf)],
    filter: (&str, &[&str]),
    none: &str,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.label(
            slot.as_deref()
                .map_or("(sin elegir)".to_string(), paths::stem),
        );
        if ui.button("Archivo…").clicked()
            && let Some(p) = crate::prefs::dialog()
                .add_filter(filter.0, filter.1)
                .pick_file()
        {
            *slot = Some(p);
        }
        ui.menu_button("De las capas cargadas", |ui| {
            for (name, path) in loaded {
                if ui.button(name).clicked() {
                    *slot = Some(path.clone());
                    ui.close();
                }
            }
            if loaded.is_empty() {
                ui.label(none);
            }
        });
    });
}

/// A raster: its name, "Archivo…" and the loaded rasters.
pub fn raster(
    ui: &mut egui::Ui,
    label: &str,
    slot: &mut Option<PathBuf>,
    loaded: &[(String, PathBuf)],
) {
    pick(
        ui,
        label,
        slot,
        loaded,
        ("Ráster", hipso_core::raster::EXTENSIONS),
        "No hay rásteres cargados",
    );
}

/// A vector layer: its name, "Archivo…" and the loaded layers.
pub fn vector(
    ui: &mut egui::Ui,
    label: &str,
    slot: &mut Option<PathBuf>,
    loaded: &[(String, PathBuf)],
) {
    pick(
        ui,
        label,
        slot,
        loaded,
        (
            "Capa vectorial",
            &["shp", "geojson", "json", "gpkg", "kml", "kmz"],
        ),
        "No hay capas vectoriales cargadas",
    );
}

/// The rasters and the vector layers on the map, as (name, path).
impl App {
    pub fn loaded_rasters(&self) -> Vec<(String, PathBuf)> {
        self.refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect()
    }

    pub fn loaded_vectors(&self) -> Vec<(String, PathBuf)> {
        self.refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Vector { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect()
    }
}
