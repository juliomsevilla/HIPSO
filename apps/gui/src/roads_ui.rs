//! Carreteras y vías (Infraestructuras): the road network of a cloud with classified ground, as
//! centre lines that carry the width of the road.

use super::*;
use crate::numfield::NumField;
use hipso_core::paths;
use hipso_core::roads::{self, RoadParams};

pub struct RoadTool {
    pub open: bool,
    pub p: RoadParams,
    pub use_intensity: bool,
    pub intensity: (f64, f64),
    pub ground_pct: f64,
}

impl Default for RoadTool {
    fn default() -> Self {
        Self {
            open: false,
            p: RoadParams::default(),
            use_intensity: false,
            intensity: (80.0, 400.0),
            ground_pct: 50.0,
        }
    }
}

impl App {
    pub fn roads_window(&mut self, ctx: &egui::Context) {
        if !self.rd.open {
            return;
        }
        let clouds = self.clouds.clone();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Carreteras y vías")
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.rd;
                ui.label(
                    egui::RichText::new("Localiza el terreno transitable (celdas donde la mayoría de los retornos son suelo, de poca pendiente y, si lo pides, con la intensidad de la superficie de la vía), lo adelgaza hasta su eje y lo traza en líneas con la anchura de la vía. Clasifica antes el suelo (clase 2). El resultado es una capa de líneas con los campos WIDTH y LENGTH, en metros.")
                        .color(weak)
                        .size(11.0),
                );
                if clouds.is_empty() {
                    ui.label("Abre una o varias nubes con el suelo clasificado.");
                } else {
                    ui.label(egui::RichText::new(format!("Se analizan {} nube(s) cargada(s); cada una da su capa.", clouds.len())).color(weak).size(11.0));
                }
                ui.horizontal(|ui| {
                    ui.label("Celda");
                    ui.add(NumField::new(&mut t.p.cell).range(0.25..=5.0).suffix(" m"));
                    ui.label("Suelo mínimo en la celda");
                    ui.add(NumField::new(&mut t.ground_pct).range(5.0..=100.0).suffix(" %"));
                });
                ui.horizontal(|ui| {
                    ui.label("Pendiente máxima");
                    ui.add(NumField::new(&mut t.p.max_slope).range(1.0..=100.0).suffix(" %"));
                    ui.label("Anchura mínima");
                    ui.add(NumField::new(&mut t.p.min_width).range(0.5..=30.0).suffix(" m"));
                });
                ui.horizontal(|ui| {
                    ui.label("Longitud mínima");
                    ui.add(NumField::new(&mut t.p.min_length).range(2.0..=2000.0).suffix(" m"));
                    ui.label("Quitar ramales de menos de");
                    ui.add(NumField::new(&mut t.p.spur_length).range(0.0..=100.0).suffix(" m"));
                });
                ui.horizontal(|ui| {
                    ui.label("Cerrar huecos de hasta");
                    ui.add(NumField::new(&mut t.p.fill_area).range(0.0..=5000.0).suffix(" m²"));
                    ui.label(egui::RichText::new("(copas, vehículos)").color(weak).size(11.0));
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut t.use_intensity, "Exigir intensidad entre");
                    ui.add_enabled(t.use_intensity, NumField::new(&mut t.intensity.0).range(0.0..=65535.0));
                    ui.label("y");
                    ui.add_enabled(t.use_intensity, NumField::new(&mut t.intensity.1).range(0.0..=65535.0));
                });
                ui.label(
                    egui::RichText::new("La intensidad ayuda a separar la vía del suelo desnudo que la rodea (cortafuegos, canteras, cauces secos), que por su forma podría confundirse con ella.")
                        .color(weak)
                        .size(11.0),
                );
                ui.label(
                    egui::RichText::new("Con menos de unos 2 puntos/m² y ruido en las alturas el método pierde tramos y da líneas cortadas; la validación está en benchmarks/roads.")
                        .color(weak)
                        .size(11.0),
                );
                if ui.add_enabled(!clouds.is_empty() && self.running.is_none(), egui::Button::new("▶ Extraer")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.rd;
            let mut p = t.p.clone();
            p.min_ground_share = t.ground_pct / 100.0;
            p.intensity = t.use_intensity.then_some(t.intensity);
            let jobs: Vec<(PathBuf, PathBuf)> = clouds
                .iter()
                .map(|c| (c.clone(), paths::unique(&paths::product(c, "viales", "shp"))))
                .collect();
            let outs: Vec<PathBuf> = jobs.iter().map(|j| j.1.clone()).collect();
            let task: tools::Task = Box::new(move || {
                let mut lines = Vec::new();
                for (src, dst) in &jobs {
                    let r = roads::extract_file(src, &p)?;
                    roads::write_lines(&r, dst, hipso_core::crs::epsg_of(src))?;
                    let km: f64 = r.iter().map(|l| l.length).sum::<f64>() / 1000.0;
                    lines.push(format!("{}: {} tramos, {km:.2} km -> {}", paths::stem(src), r.len(), dst.display()));
                }
                Ok(lines.join("\n"))
            });
            self.start(
                ctx,
                Launch {
                    label: "Carreteras y vías".to_string(),
                    task,
                    show: outs.clone(),
                    outputs: outs,
                },
            );
        }
        if !open {
            self.rd.open = false;
        }
    }
}
