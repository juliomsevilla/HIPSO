//! Zonas encharcadas (Ráster › Análisis del terreno › Hidrología): dónde se queda el agua sobre un
//! MDT.

use super::*;
use crate::numfield::NumField;
use hipso_core::paths;
use hipso_core::ponds::{self, Method, PondJob};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Terreno nivelado: el agua poco profunda cuenta.
    Level,
    /// Terreno rugoso (bosque, matorral): el modelo tiene hoyos que no son charcas.
    Rough,
    Custom,
}

pub struct PondTool {
    pub open: bool,
    pub input: Option<PathBuf>,
    pub preset: Preset,
    pub method: Method,
    pub smooth: f64,
    pub min_depth: f64,
    pub min_area: f64,
    pub max_area: f64,
    pub max_depth: f64,
    pub limit_size: bool,
    pub slope_on: bool,
    pub max_slope: f64,
    pub raster: bool,
    pub depth_raster: bool,
    pub shp: bool,
    pub gpkg: bool,
}

impl Default for PondTool {
    fn default() -> Self {
        let mut t = Self {
            open: false,
            input: None,
            preset: Preset::Level,
            method: Method::Depression,
            smooth: 0.0,
            min_depth: 0.05,
            min_area: 1.0,
            max_area: 5000.0,
            max_depth: 1.0,
            limit_size: true,
            slope_on: false,
            max_slope: 8.0,
            raster: true,
            depth_raster: false,
            shp: true,
            gpkg: false,
        };
        t.apply(Preset::Level);
        t
    }
}

impl PondTool {
    fn apply(&mut self, p: Preset) {
        self.preset = p;
        match p {
            Preset::Level => {
                self.min_depth = 0.05;
                self.min_area = 1.0;
                self.slope_on = false;
            }
            Preset::Rough => {
                self.min_depth = 0.15;
                self.min_area = 4.0;
                self.slope_on = true;
                self.max_slope = 8.0;
            }
            Preset::Custom => {}
        }
    }
}

impl App {
    pub fn ponds_window(&mut self, ctx: &egui::Context) {
        if !self.pt.open {
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
        egui::Window::new("Zonas encharcadas")
            .default_pos([(ctx.content_rect().right() - 560.0).max(320.0), 190.0])
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.pt;
                ui.label(
                    egui::RichText::new("Delimita donde se queda el agua en un MDT: se rellenan las depresiones hasta su punto de desborde y cada mancha de agua más honda que el mínimo es un charco, con su área, profundidad, volumen y cota de desborde. Metros y sistema métrico (UTM).")
                        .color(weak)
                        .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.label("MDT");
                    ui.label(t.input.as_deref().map_or("(elige un MDT)".to_string(), paths::stem));
                    if ui.button("Archivo…").clicked()
                        && let Some(p) = crate::prefs::dialog()
                            .add_filter("Ráster", hipso_core::raster::EXTENSIONS)
                            .pick_file()
                    {
                        t.input = Some(p);
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                t.input = Some(path.clone());
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Terreno");
                    let mut p = t.preset;
                    ui.radio_value(&mut p, Preset::Level, "Nivelado (cultivo)");
                    ui.radio_value(&mut p, Preset::Rough, "Irregular (monte)");
                    ui.radio_value(&mut p, Preset::Custom, "A medida");
                    if p != t.preset {
                        t.apply(p);
                    }
                });
                ui.label(
                    egui::RichText::new("Nivelado: 5 cm y 1 m². Irregular: un MDT de monte tiene hoyos que no son charcos, así que pide 15 cm, 4 m² y que el suelo bajo el charco sea casi llano. Son puntos de partida: ajústalos a tu MDT.")
                        .color(weak)
                        .size(11.0),
                );
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("Profundidad mínima");
                    changed |= ui.add(NumField::new(&mut t.min_depth).range(0.0..=10.0).suffix(" m")).changed();
                    ui.label("Área mínima");
                    changed |= ui.add(NumField::new(&mut t.min_area).range(0.0..=1.0e6).suffix(" m²")).changed();
                });
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut t.slope_on, "Suelo casi llano: pendiente media máxima").changed();
                    if t.slope_on {
                        changed |= ui.add(NumField::new(&mut t.max_slope).range(0.0..=90.0).suffix("°")).changed();
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut t.limit_size, "Descartar balsas y zanjas: área máxima");
                    if t.limit_size {
                        ui.add(NumField::new(&mut t.max_area).range(1.0..=1.0e9).suffix(" m²"));
                        ui.label("profundidad máxima");
                        ui.add(NumField::new(&mut t.max_depth).range(0.01..=1000.0).suffix(" m"));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Suavizar el MDT antes");
                    ui.add(NumField::new(&mut t.smooth).range(0.0..=20.0).suffix(" m"));
                    ui.label(egui::RichText::new("0 = no. En campos con caballones, no suavices: borra las calles.").color(weak).size(11.0));
                });
                if changed && t.preset != Preset::Custom {
                    t.preset = Preset::Custom;
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Resultados");
                    ui.checkbox(&mut t.raster, "ráster de charcos");
                    ui.checkbox(&mut t.depth_raster, "profundidad del agua");
                    ui.checkbox(&mut t.shp, "polígonos (shp)");
                    ui.checkbox(&mut t.gpkg, "GeoPackage");
                });
                let ready = t.input.is_some()
                    && (t.raster || t.depth_raster || t.shp || t.gpkg);
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Delimitar charcos")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.pt;
            let Some(input) = t.input.clone() else {
                return;
            };
            let out = paths::unique(&paths::product(&input, "charcos", "shp"));
            let mut job = PondJob::new(input, out.clone());
            job.method = t.method;
            job.smooth = t.smooth;
            job.min_depth = t.min_depth;
            job.min_area = t.min_area;
            job.max_slope = t.slope_on.then_some(t.max_slope);
            if t.limit_size {
                job.max_area = Some(t.max_area);
                job.max_depth = Some(t.max_depth);
            }
            (job.raster, job.depth_raster, job.shp, job.gpkg) = (t.raster, t.depth_raster, t.shp, t.gpkg);
            let mut show = Vec::new();
            if t.shp {
                show.push(paths::swap_ext(&out, "shp"));
            } else if t.gpkg {
                show.push(paths::swap_ext(&out, "gpkg"));
            }
            if t.raster {
                show.push(paths::swap_ext(&out, "tif"));
            }
            let task: tools::Task = Box::new(move || ponds::run(&job));
            self.start(
                ctx,
                Launch {
                    label: "Zonas encharcadas".into(),
                    task,
                    show,
                    outputs: vec![out],
                },
            );
        }
        if !open {
            self.pt.open = false;
        }
    }
}
