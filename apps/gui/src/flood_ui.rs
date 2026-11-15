//! Inundación (Ráster › Análisis del terreno › Hidrología): el agua de uno o varios niveles sobre
//! un MDT, desde semillas.

use super::*;
use crate::numfield::NumField;
use hipso_core::flood::{self, FloodJob, Relative, Seeds};
use hipso_core::paths;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Points,
    Border,
    Bathtub,
}

pub struct FloodTool {
    pub open: bool,
    dtm: Option<PathBuf>,
    source: Source,
    seeds: String,
    relative: u8,
    from: f64,
    to: f64,
    step: f64,
    series: bool,
    eight: bool,
    depth: bool,
    extent: bool,
    csv: bool,
}

impl Default for FloodTool {
    fn default() -> Self {
        Self {
            open: false,
            dtm: None,
            source: Source::Points,
            seeds: String::new(),
            relative: 0,
            from: 0.0,
            to: 2.0,
            step: 0.5,
            series: false,
            eight: true,
            depth: true,
            extent: true,
            csv: true,
        }
    }
}

/// Los puntos `x,y` de un texto, uno por línea.
fn points(text: &str) -> Option<Vec<[f64; 2]>> {
    let v: Option<Vec<[f64; 2]>> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (x, y) = l.split_once([',', ';', ' ', '\t'])?;
            Some([
                x.trim().replace(',', ".").parse().ok()?,
                y.trim().replace(',', ".").parse().ok()?,
            ])
        })
        .collect();
    v.filter(|p| !p.is_empty())
}

impl App {
    pub fn flood_window(&mut self, ctx: &egui::Context) {
        if !self.fl.open {
            return;
        }
        let rasters = self.loaded_rasters();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Inundación")
            .default_pos([(ctx.content_rect().right() - 560.0).max(320.0), 190.0])
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(540.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.fl;
                ui.label(
                    egui::RichText::new("Simula el agua de uno o varios niveles sobre un MDT: área inundada, volumen, profundidad y masas de agua. El agua solo llena lo que puede alcanzar desde el origen pasando por celdas que no superan el nivel: un hoyo detrás de una loma queda seco. Con «bañera» se inunda todo lo que está bajo el nivel, llegue o no el agua. Metros y sistema métrico.")
                        .color(weak)
                        .size(11.0),
                );
                crate::pick::raster(ui, "MDT", &mut t.dtm, &rasters);
                ui.separator();
                ui.label("Origen del agua");
                ui.radio_value(&mut t.source, Source::Points, "Uno o varios puntos (un río, una rotura, el mar)");
                ui.radio_value(&mut t.source, Source::Border, "El borde del modelo");
                ui.radio_value(&mut t.source, Source::Bathtub, "Bañera: sin conectividad");
                if t.source == Source::Points {
                    ui.label(egui::RichText::new("Un punto por línea, «x y» con las coordenadas del mapa.").color(weak).size(11.0));
                    ui.add(
                        egui::TextEdit::multiline(&mut t.seeds)
                            .hint_text("512345.5 4678123.0")
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Niveles");
                    ui.radio_value(&mut t.series, false, "Uno");
                    ui.radio_value(&mut t.series, true, "Una serie");
                });
                ui.horizontal(|ui| {
                    ui.label(if t.series { "Desde" } else { "Nivel" });
                    ui.add(NumField::new(&mut t.from).range(-10000.0..=100000.0).suffix(" m"));
                    if t.series {
                        ui.label("hasta");
                        ui.add(NumField::new(&mut t.to).range(-10000.0..=100000.0).suffix(" m"));
                        ui.label("cada");
                        ui.add(NumField::new(&mut t.step).range(0.001..=1000.0).suffix(" m"));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Medidos como");
                    ui.radio_value(&mut t.relative, 0, "Cota");
                    ui.radio_value(&mut t.relative, 1, "Altura sobre la celda más baja");
                    ui.radio_value(&mut t.relative, 2, "Altura sobre el origen");
                });
                ui.checkbox(&mut t.eight, "El agua pasa también por las esquinas de las celdas (8 vecinas)");
                ui.horizontal(|ui| {
                    ui.label("Resultados");
                    ui.checkbox(&mut t.depth, "profundidad por nivel");
                    ui.checkbox(&mut t.extent, "polígonos de la extensión");
                    ui.checkbox(&mut t.csv, "tabla (CSV)");
                });
                ui.label(egui::RichText::new("Se guarda también el ráster de alcance: la cota mínima del agua que llega a cada celda, útil como mapa de peligro.").color(weak).size(11.0));
                let seeds_ok = t.source != Source::Points || points(&t.seeds).is_some();
                if !seeds_ok {
                    ui.label(egui::RichText::new("Escribe al menos un punto, «x y», uno por línea.").color(weak).size(11.0));
                }
                let series_ok = !t.series || (t.step > 0.0 && t.to >= t.from && (t.to - t.from) / t.step <= 1000.0);
                if !series_ok {
                    ui.label(egui::RichText::new("La serie va de menor a mayor, con paso positivo y 1000 niveles como máximo.").color(weak).size(11.0));
                }
                let ready = t.dtm.is_some() && seeds_ok && series_ok;
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Simular")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.fl;
            let Some(dtm) = t.dtm.clone() else {
                return;
            };
            let seeds = match t.source {
                Source::Points => match points(&t.seeds) {
                    Some(p) => Seeds::Points(p),
                    None => return,
                },
                Source::Border => Seeds::Border,
                Source::Bathtub => Seeds::All,
            };
            let levels: Vec<f64> = if t.series {
                let n = ((t.to - t.from) / t.step + 1e-9).floor() as usize;
                (0..=n).map(|k| t.from + k as f64 * t.step).collect()
            } else {
                vec![t.from]
            };
            let out = paths::unique(&paths::product(&dtm, "inundacion", "shp"));
            let mut job = FloodJob::new(dtm, seeds, levels, out.clone());
            job.relative = match t.relative {
                1 => Relative::MinZ,
                2 => Relative::Seed,
                _ => Relative::Absolute,
            };
            job.connectivity = if t.eight { 8 } else { 4 };
            (job.depth, job.extent, job.csv) = (t.depth, t.extent, t.csv);
            let mut show = Vec::new();
            show.push(
                paths::swap_ext(&out, "tif")
                    .with_file_name(format!("{}_alcance.tif", paths::stem(&out))),
            );
            let task: tools::Task = Box::new(move || flood::run(&job));
            self.start(
                ctx,
                Launch {
                    label: "Inundación".into(),
                    task,
                    show,
                    outputs: vec![out],
                },
            );
        }
        if !open {
            self.fl.open = false;
        }
    }
}
