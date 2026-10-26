//! Cambio forestal (Inventario forestal): the difference between the canopy height models of two
//! dates.

use super::*;
use crate::numfield::NumField;
use rustlidar_core::change::{ChangeJob, TreeChange};
use rustlidar_core::paths::{self, Formats};
use rustlidar_core::trees::{Shape, TopParams};

pub struct ChangeTool {
    pub open: bool,
    pub before: Option<PathBuf>,
    pub after: Option<PathBuf>,
    pub min_change: f64,
    pub cover_height: f64,
    pub min_patch: u32,
    pub years: String,
    pub stands: Option<PathBuf>,
    pub stand_field: String,
    pub trees: bool,
    pub hmin: f64,
    pub window_a: f64,
    pub window_b: f64,
    pub smooth: f64,
    pub match_dist: f64,
    pub vector: bool,
    pub csv: bool,
    pub out: String,
    pub out_manual: bool,
}

impl Default for ChangeTool {
    fn default() -> Self {
        Self {
            open: false,
            before: None,
            after: None,
            min_change: 1.0,
            cover_height: 2.0,
            min_patch: 10,
            years: String::new(),
            stands: None,
            stand_field: String::new(),
            trees: false,
            hmin: 2.0,
            window_a: 3.0,
            window_b: 0.1,
            smooth: 0.0,
            match_dist: 3.0,
            vector: true,
            csv: false,
            out: String::new(),
            out_manual: false,
        }
    }
}

fn pick_raster(
    ui: &mut egui::Ui,
    label: &str,
    slot: &mut Option<PathBuf>,
    loaded: &[(String, PathBuf)],
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.label(
            slot.as_deref()
                .map_or("(elige un CHM)".to_string(), paths::stem),
        );
        if ui.button("Archivo…").clicked()
            && let Some(p) = crate::prefs::dialog()
                .add_filter("Ráster", rustlidar_core::raster::EXTENSIONS)
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
                ui.label("No hay rásteres cargados");
            }
        });
    });
}

impl App {
    pub fn change_window(&mut self, ctx: &egui::Context) {
        if !self.ct.open {
            return;
        }
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Cambio forestal")
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(580.0)
            .show(ctx, |ui| {
                let t = &mut self.ct;
                ui.label(
                    egui::RichText::new("Compara los modelos de alturas (CHM) de dos fechas: cuánto ha crecido o bajado cada celda, dónde se ha perdido o ganado copa, las manchas perturbadas, el resumen por rodal y el crecimiento de cada árbol. Los dos CHM deben coincidir celda a celda (Ráster → Igualar rásteres).")
                        .color(weak)
                        .size(11.0),
                );
                pick_raster(ui, "CHM de la primera fecha", &mut t.before, &loaded);
                pick_raster(ui, "CHM de la segunda fecha", &mut t.after, &loaded);
                ui.horizontal(|ui| {
                    ui.label("Años entre fechas");
                    ui.add(egui::TextEdit::singleline(&mut t.years).desired_width(50.0).hint_text("opcional"));
                    ui.label("Cambio mínimo");
                    ui.add(NumField::new(&mut t.min_change).range(0.0..=20.0).suffix(" m"));
                    ui.label("Copa desde");
                    ui.add(NumField::new(&mut t.cover_height).range(0.0..=50.0).suffix(" m"));
                });
                ui.label(egui::RichText::new("Un cambio de altura menor que el mínimo se considera ruido. El error vertical entre fechas (MDT distintos) puede ser mayor que el crecimiento anual: el informe lo mide.").color(weak).size(11.0));
                ui.horizontal(|ui| {
                    ui.label("Mancha mínima de pérdida");
                    ui.add(NumField::new(&mut t.min_patch).range(1.0..=1.0e6).suffix(" celdas"));
                });
                ui.horizontal(|ui| {
                    ui.label("Rodales");
                    ui.label(t.stands.as_deref().map_or("(opcional)".to_string(), paths::stem));
                    if ui.button("Archivo…").clicked()
                        && let Some(p) = crate::prefs::dialog().add_filter("Rodales", &["shp", "geojson", "json", "gpkg"]).pick_file()
                    {
                        t.stands = Some(p);
                    }
                    if t.stands.is_some() && ui.button("Quitar").clicked() {
                        t.stands = None;
                    }
                    ui.label("Campo del nombre");
                    ui.add(egui::TextEdit::singleline(&mut t.stand_field).desired_width(80.0));
                });
                ui.checkbox(&mut t.trees, "Emparejar los árboles de las dos fechas y medir su crecimiento");
                if t.trees {
                    ui.horizontal(|ui| {
                        ui.label("Altura mínima");
                        ui.add(NumField::new(&mut t.hmin).range(0.0..=50.0).suffix(" m"));
                        ui.label("Ventana a + b·h");
                        ui.add(NumField::new(&mut t.window_a).range(0.5..=50.0));
                        ui.add(NumField::new(&mut t.window_b).range(0.0..=2.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Suavizar");
                        ui.add(NumField::new(&mut t.smooth).range(0.0..=20.0).suffix(" m"));
                        ui.label("Mismo árbol si dista menos de");
                        ui.add(NumField::new(&mut t.match_dist).range(0.1..=50.0).suffix(" m"));
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Salida");
                    ui.checkbox(&mut t.vector, "manchas (shp)");
                    ui.checkbox(&mut t.csv, "manchas (csv)");
                });
                if !t.out_manual {
                    t.out = t.after.as_ref().map_or(String::new(), |f| {
                        paths::unique(&paths::product(f, "cambio", "shp")).display().to_string()
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Base del nombre");
                    if ui.add(egui::TextEdit::singleline(&mut t.out).desired_width(380.0)).changed() {
                        t.out_manual = true;
                    }
                    if t.out_manual && ui.button("auto").clicked() {
                        t.out_manual = false;
                    }
                });
                let ready = t.before.is_some() && t.after.is_some() && !t.out.trim().is_empty() && (t.vector || t.csv);
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Calcular")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.ct;
            let (Some(before), Some(after)) = (t.before.clone(), t.after.clone()) else {
                return;
            };
            let job = ChangeJob {
                before,
                after,
                output: {
                    let o = PathBuf::from(t.out.trim());
                    if t.out_manual { o } else { paths::unique(&o) }
                },
                min_change: t.min_change,
                cover_height: t.cover_height,
                min_patch: t.min_patch,
                years: t.years.trim().replace(',', ".").parse().ok(),
                regions: t.stands.clone().map(|f| {
                    let field = t.stand_field.trim();
                    (f, (!field.is_empty()).then(|| field.to_string()))
                }),
                trees: t.trees.then_some(TreeChange {
                    tops: TopParams {
                        hmin: t.hmin,
                        a: t.window_a,
                        b: t.window_b,
                        c: 0.0,
                        shape: Shape::Circle,
                    },
                    smooth: t.smooth,
                    max_dist: t.match_dist,
                }),
                formats: Formats {
                    vector: t.vector,
                    csv: t.csv,
                },
            };
            let o = job.outputs();
            let mut show = vec![o.classes.clone(), o.difference.clone()];
            show.extend(o.patches.clone());
            let mut outputs = show.clone();
            outputs.extend(o.patches_table);
            outputs.extend(o.stands);
            outputs.extend(o.trees);
            let task: tools::Task = Box::new(move || rustlidar_core::change::change(&job));
            self.start(
                ctx,
                Launch {
                    label: "Cambio forestal".to_string(),
                    task,
                    show,
                    outputs,
                },
            );
        }
        if !open {
            self.ct.open = false;
        }
    }
}
