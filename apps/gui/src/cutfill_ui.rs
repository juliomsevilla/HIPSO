//! Desmonte y terraplén (Ráster › Volúmenes): la tierra que quitar y añadir para llevar un terreno
//! a un diseño.

use super::*;
use crate::numfield::NumField;
use hipso_core::cutfill::{self, CutFillJob, Design};
use hipso_core::paths;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DesignKind {
    Surface,
    Level,
    Plane,
}

pub struct CutFillTool {
    pub open: bool,
    existing: Option<PathBuf>,
    kind: DesignKind,
    design: Option<PathBuf>,
    level: f64,
    z0: f64,
    slope_pct: f64,
    aspect: f64,
    origin_x: String,
    origin_y: String,
    regions: Option<PathBuf>,
    tolerance: f64,
    csv: bool,
    classified: bool,
}

impl Default for CutFillTool {
    fn default() -> Self {
        Self {
            open: false,
            existing: None,
            kind: DesignKind::Surface,
            design: None,
            level: 0.0,
            z0: 0.0,
            slope_pct: 2.0,
            aspect: 90.0,
            origin_x: String::new(),
            origin_y: String::new(),
            regions: None,
            tolerance: 0.05,
            csv: true,
            classified: false,
        }
    }
}

fn number(t: &str) -> Option<f64> {
    t.trim().replace(',', ".").parse().ok()
}

impl App {
    pub fn cutfill_window(&mut self, ctx: &egui::Context) {
        if !self.cf.open {
            return;
        }
        let (rasters, vectors) = (self.loaded_rasters(), self.loaded_vectors());
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Desmonte y terraplén")
            .default_pos([(ctx.content_rect().right() - 560.0).max(320.0), 190.0])
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.cf;
                ui.label(
                    egui::RichText::new("Calcula la tierra que hay que quitar (desmonte) y la que hay que añadir (terraplén) para llevar un terreno existente a un diseño. La diferencia es diseño menos terreno: positiva es terraplén, negativa es desmonte. Las diferencias de hasta la tolerancia cuentan como sin cambio. Metros y sistema métrico.")
                        .color(weak)
                        .size(11.0),
                );
                crate::pick::raster(ui, "Terreno existente", &mut t.existing, &rasters);
                ui.separator();
                ui.label("Diseño");
                ui.radio_value(&mut t.kind, DesignKind::Surface, "Otro modelo (un MDT de diseño)");
                ui.radio_value(&mut t.kind, DesignKind::Level, "Plano horizontal a una cota");
                ui.radio_value(&mut t.kind, DesignKind::Plane, "Plano inclinado");
                match t.kind {
                    DesignKind::Surface => crate::pick::raster(ui, "Diseño", &mut t.design, &rasters),
                    DesignKind::Level => {
                        ui.horizontal(|ui| {
                            ui.label("Cota");
                            ui.add(NumField::new(&mut t.level).range(-10000.0..=100000.0).suffix(" m"));
                        });
                    }
                    DesignKind::Plane => {
                        ui.horizontal(|ui| {
                            ui.label("Cota en el origen");
                            ui.add(NumField::new(&mut t.z0).range(-10000.0..=100000.0).suffix(" m"));
                            ui.label("Pendiente");
                            ui.add(NumField::new(&mut t.slope_pct).range(0.0..=1000.0).suffix(" %"));
                            ui.label("Hacia");
                            ui.add(NumField::new(&mut t.aspect).range(0.0..=360.0).suffix("°"));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Origen X");
                            ui.add(egui::TextEdit::singleline(&mut t.origin_x).desired_width(90.0));
                            ui.label("Y");
                            ui.add(egui::TextEdit::singleline(&mut t.origin_y).desired_width(90.0));
                        });
                        ui.label(egui::RichText::new("El plano baja hacia la orientación indicada (grados desde el norte, en sentido horario), como correría el agua. Origen: las coordenadas del mapa.").color(weak).size(11.0));
                    }
                }
                ui.separator();
                crate::pick::vector(ui, "Polígonos (opcional)", &mut t.regions, &vectors);
                ui.horizontal(|ui| {
                    ui.label("Tolerancia");
                    ui.add(NumField::new(&mut t.tolerance).range(0.0..=10.0).suffix(" m"));
                    ui.checkbox(&mut t.csv, "tabla (CSV)");
                    ui.checkbox(&mut t.classified, "ráster de clases");
                });
                let plane_ok = t.kind != DesignKind::Plane
                    || (number(&t.origin_x).is_some() && number(&t.origin_y).is_some());
                if !plane_ok {
                    ui.label(egui::RichText::new("Escribe las coordenadas X e Y del origen del plano.").color(weak).size(11.0));
                }
                let ready = t.existing.is_some()
                    && (t.kind != DesignKind::Surface || t.design.is_some())
                    && plane_ok;
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Calcular")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.cf;
            let Some(existing) = t.existing.clone() else {
                return;
            };
            let design = match t.kind {
                DesignKind::Surface => match t.design.clone() {
                    Some(d) => Design::Surface(d),
                    None => return,
                },
                DesignKind::Level => Design::Level(t.level),
                DesignKind::Plane => {
                    let (Some(x), Some(y)) = (number(&t.origin_x), number(&t.origin_y)) else {
                        return;
                    };
                    Design::Plane {
                        z0: t.z0,
                        slope_pct: t.slope_pct,
                        aspect_deg: t.aspect,
                        origin: [x, y],
                    }
                }
            };
            let out = paths::unique(&paths::product(&existing, "cutfill", "tif"));
            let mut job = CutFillJob::new(existing, design, out.clone());
            job.regions = t.regions.clone();
            job.tolerance = t.tolerance;
            (job.csv, job.classified) = (t.csv, t.classified);
            let task: tools::Task = Box::new(move || cutfill::run(&job));
            self.start(
                ctx,
                Launch {
                    label: "Desmonte y terraplén".into(),
                    task,
                    show: vec![out.clone()],
                    outputs: vec![out],
                },
            );
        }
        if !open {
            self.cf.open = false;
        }
    }
}
