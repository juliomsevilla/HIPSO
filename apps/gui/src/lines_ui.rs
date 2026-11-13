//! Crestas y drenaje (Ráster › Análisis del terreno › Forma del terreno): ridge or drainage
//! lines traced from a DTM.

use super::*;
use crate::numfield::NumField;
use hipso_core::paths;
use hipso_core::terrainlines::{self, Kind, LinesJob};

pub struct LinesTool {
    pub open: bool,
    dtm: Option<PathBuf>,
    drainage: bool,
    min_area: f64,
    min_length: f64,
    smooth: f64,
}

impl Default for LinesTool {
    fn default() -> Self {
        Self {
            open: false,
            dtm: None,
            drainage: false,
            min_area: 1000.0,
            min_length: 30.0,
            smooth: 0.0,
        }
    }
}

impl App {
    pub fn lines_window(&mut self, ctx: &egui::Context) {
        if !self.ln.open {
            return;
        }
        let rasters = self.loaded_rasters();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Crestas y drenaje")
            .default_pos([(ctx.content_rect().right() - 560.0).max(320.0), 190.0])
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.ln;
                ui.label(
                    egui::RichText::new("Traza las líneas de cresta o de drenaje de un MDT como líneas 3D, con el orden de Strahler (las cabeceras son 1; dos tramos del mismo orden que se juntan dan el siguiente), la longitud y la pendiente media de cada tramo. Una línea nace donde se recoge la superficie mínima. En una cresta o un cauce perfectamente llano la línea sale en dos mitades que se tocan en el centro. Metros y sistema métrico.")
                        .color(weak)
                        .size(11.0),
                );
                crate::pick::raster(ui, "MDT", &mut t.dtm, &rasters);
                ui.horizontal(|ui| {
                    ui.label("Líneas de");
                    ui.radio_value(&mut t.drainage, false, "cresta");
                    ui.radio_value(&mut t.drainage, true, "drenaje");
                });
                ui.horizontal(|ui| {
                    ui.label("Superficie mínima que recoge");
                    ui.add(NumField::new(&mut t.min_area).range(1.0..=1.0e9).suffix(" m²"));
                    ui.label("Longitud mínima");
                    ui.add(NumField::new(&mut t.min_length).range(0.0..=1.0e6).suffix(" m"));
                });
                ui.horizontal(|ui| {
                    ui.label("Suavizado");
                    ui.add(NumField::new(&mut t.smooth).range(0.0..=5.0).speed(0.2));
                    ui.label(egui::RichText::new("pasadas de Chaikin; 0 sigue las celdas").color(weak).size(11.0));
                });
                if ui.add_enabled(t.dtm.is_some() && self.running.is_none(), egui::Button::new("▶ Trazar")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.ln;
            let Some(dtm) = t.dtm.clone() else {
                return;
            };
            let (kind, tag) = if t.drainage {
                (Kind::Drainage, "drenaje")
            } else {
                (Kind::Ridge, "crestas")
            };
            let out = paths::unique(&paths::product(&dtm, tag, "shp"));
            let mut job = LinesJob::new(dtm, kind, out.clone());
            job.min_area = t.min_area;
            job.min_length = t.min_length;
            job.smooth = t.smooth.round() as u8;
            let task: tools::Task = Box::new(move || terrainlines::run(&job));
            self.start(
                ctx,
                Launch {
                    label: "Crestas y drenaje".into(),
                    task,
                    show: vec![out.clone()],
                    outputs: vec![out],
                },
            );
        }
        if !open {
            self.ln.open = false;
        }
    }
}
