//! Las filas que eligen un archivo en las ventanas de las herramientas: un ráster o una capa
//! vectorial, de un archivo o de las capas que ya están en el mapa.

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

/// Un ráster: su nombre, «Archivo…» y los rásteres cargados.
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

/// Una capa vectorial: su nombre, «Archivo…» y las capas cargadas.
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

/// Los rásteres y las capas vectoriales del mapa, como (nombre, ruta).
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
