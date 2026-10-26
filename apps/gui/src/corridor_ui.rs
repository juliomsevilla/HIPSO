//! Corredores eléctricos (Inventario forestal): conductors, poles and vegetation that comes too
//! close or could fall on the line.

use super::*;
use crate::numfield::NumField;
use rustlidar_core::corridor::{CorridorJob, CorridorParams};
use rustlidar_core::filter::ClassSel;
use rustlidar_core::paths::{self, Formats};

pub struct CorridorTool {
    pub open: bool,
    pub dtm: Option<PathBuf>,
    pub min_distance: f64,
    pub fall_margin: f64,
    pub half_width: f64,
    pub link: f64,
    pub gap_max: f64,
    pub min_height: f64,
    pub vegetation: String,
    pub vector: bool,
    pub out: String,
    pub out_manual: bool,
}

impl Default for CorridorTool {
    fn default() -> Self {
        let d = CorridorParams::default();
        Self {
            open: false,
            dtm: None,
            min_distance: d.min_distance,
            fall_margin: d.fall_margin,
            half_width: d.half_width,
            link: d.link,
            gap_max: d.gap_max,
            min_height: d.min_height,
            vegetation: "3,4,5".into(),
            vector: true,
            out: String::new(),
            out_manual: false,
        }
    }
}

impl App {
    pub fn corridor_window(&mut self, ctx: &egui::Context) {
        if !self.co.open {
            return;
        }
        let clouds = self.clouds.clone();
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Corredores eléctricos")
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                let t = &mut self.co;
                ui.label(
                    egui::RichText::new("Reconstruye los conductores con los puntos de cable (clase 14), localiza los postes (clase 15) y marca la vegetación que queda más cerca del conductor que la distancia de seguridad o que podría alcanzarlo si cayera. Clasifica antes «Cables y postes» en Clasificar. El resultado trae un informe por vano.")
                        .color(weak)
                        .size(11.0),
                );
                if clouds.is_empty() {
                    ui.label("Abre una o varias nubes ya clasificadas (cables y postes).");
                } else {
                    ui.label(egui::RichText::new(format!("Se analizan {} nube(s) cargada(s).", clouds.len())).color(weak).size(11.0));
                }
                ui.horizontal(|ui| {
                    ui.label("MDT");
                    let auto = clouds.first().and_then(|c| paths::find_dtm(c));
                    let shown = t
                        .dtm
                        .as_deref()
                        .map(paths::stem)
                        .or_else(|| auto.as_deref().map(|a| format!("{} (automático)", paths::stem(a))))
                        .unwrap_or("(hace falta un MDT)".to_string());
                    ui.label(shown);
                    if ui.button("Archivo…").clicked()
                        && let Some(p) = crate::prefs::dialog().add_filter("Ráster", rustlidar_core::raster::EXTENSIONS).pick_file()
                    {
                        t.dtm = Some(p);
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                t.dtm = Some(path.clone());
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Distancia de seguridad");
                    ui.add(NumField::new(&mut t.min_distance).range(0.1..=50.0).suffix(" m"));
                    ui.label("Margen de caída");
                    ui.add(NumField::new(&mut t.fall_margin).range(0.0..=20.0).suffix(" m"));
                });
                ui.label(egui::RichText::new("La distancia de seguridad depende de la tensión de la línea y de la normativa aplicable: ajústala a tu caso.").color(weak).size(11.0));
                ui.horizontal(|ui| {
                    ui.label("Franja a cada lado");
                    ui.add(NumField::new(&mut t.half_width).range(2.0..=500.0).suffix(" m"));
                    ui.label("Altura mínima de la vegetación");
                    ui.add(NumField::new(&mut t.min_height).range(0.0..=50.0).suffix(" m"));
                });
                ui.horizontal(|ui| {
                    ui.label("Unir puntos de cable a menos de");
                    ui.add(NumField::new(&mut t.link).range(0.5..=20.0).suffix(" m"));
                    ui.label("Rellenar huecos de hasta");
                    ui.add(NumField::new(&mut t.gap_max).range(0.0..=200.0).suffix(" m"));
                });
                ui.horizontal(|ui| {
                    ui.label("Clases de vegetación");
                    ui.add(egui::TextEdit::singleline(&mut t.vegetation).desired_width(90.0));
                    ui.checkbox(&mut t.vector, "capas (shp)");
                });
                if !t.out_manual {
                    t.out = clouds.first().map_or(String::new(), |f| {
                        paths::unique(&paths::product(f, "corredor", "shp")).display().to_string()
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
                let have_dtm = t.dtm.is_some() || clouds.first().and_then(|c| paths::find_dtm(c)).is_some();
                let ready = !clouds.is_empty() && have_dtm && !t.out.trim().is_empty() && ClassSel::parse(&t.vegetation).is_ok();
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Calcular")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.co;
            let dtm = t
                .dtm
                .clone()
                .or_else(|| clouds.first().and_then(|c| paths::find_dtm(c)));
            let (Some(dtm), Ok(vegetation)) = (dtm, ClassSel::parse(&t.vegetation)) else {
                return;
            };
            let job = CorridorJob {
                inputs: clouds,
                dtm,
                output: {
                    let o = PathBuf::from(t.out.trim());
                    if t.out_manual { o } else { paths::unique(&o) }
                },
                params: CorridorParams {
                    link: t.link,
                    gap_max: t.gap_max,
                    half_width: t.half_width,
                    min_distance: t.min_distance,
                    fall_margin: t.fall_margin,
                    min_height: t.min_height,
                    vegetation,
                    ..Default::default()
                },
                formats: Formats {
                    vector: t.vector,
                    csv: true,
                },
            };
            let o = job.outputs();
            let mut show: Vec<PathBuf> = Vec::new();
            show.extend(o.conductors.clone());
            show.extend(o.poles.clone());
            show.extend(o.hazards.clone());
            let mut outputs = show.clone();
            outputs.push(o.spans);
            outputs.push(o.hazards_table);
            let task: tools::Task = Box::new(move || rustlidar_core::corridor::run(&job));
            self.start(
                ctx,
                Launch {
                    label: "Corredores eléctricos".to_string(),
                    task,
                    show,
                    outputs,
                },
            );
        }
        if !open {
            self.co.open = false;
        }
    }
}
