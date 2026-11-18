//! Cubicación de pilas (Ráster › Volúmenes): el material sobre una superficie base dentro del
//! contorno de cada pila.

use super::*;
use crate::numfield::NumField;
use hipso_core::paths;
use hipso_core::piles::{self, Base, PileJob};

#[derive(Clone, Copy, PartialEq, Eq)]
enum BaseKind {
    Plane,
    Lowest,
    Median,
    Interpolated,
    Level,
    Dtm,
}

pub struct PileTool {
    pub open: bool,
    surface: Option<PathBuf>,
    piles: Option<PathBuf>,
    kind: BaseKind,
    level: f64,
    dtm: Option<PathBuf>,
    min_height: f64,
    density_on: bool,
    density: f64,
    csv: bool,
    base_raster: bool,
    height_raster: bool,
}

impl Default for PileTool {
    fn default() -> Self {
        Self {
            open: false,
            surface: None,
            piles: None,
            kind: BaseKind::Plane,
            level: 0.0,
            dtm: None,
            min_height: 0.05,
            density_on: false,
            density: 1.6,
            csv: true,
            base_raster: false,
            height_raster: false,
        }
    }
}

impl App {
    pub fn piles_window(&mut self, ctx: &egui::Context) {
        if !self.pl.open {
            return;
        }
        let (rasters, vectors) = (self.loaded_rasters(), self.loaded_vectors());
        let weak = self.tokens().weak;
        let mut open = true;
        let accent = self.tokens().accent;
        let mut go = false;
        crate::procwin::window(ctx, "Cubicación de pilas")
            .open(&mut open)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.pl;
                ui.label(
                    egui::RichText::new("Mide el material que hay sobre una superficie base dentro del contorno de cada pila: volumen, área en planta y sobre la pendiente, altura media y máxima y, con la densidad, la masa. El modelo de superficie (MDS) debe estar en un sistema métrico. El volumen se decide por el centro de cada celda: el error es del orden del perímetro por media celda por la altura del borde, y baja con celdas más finas.")
                        .color(weak)
                        .size(11.0),
                );
                crate::pick::raster(ui, "Modelo de superficie", &mut t.surface, &rasters);
                crate::pick::vector(ui, "Contornos de las pilas", &mut t.piles, &vectors);
                ui.separator();
                ui.label("Superficie base");
                for (k, label) in [
                    (BaseKind::Plane, "Plano ajustado al contorno (mínimos cuadrados)"),
                    (BaseKind::Lowest, "Cota más baja del contorno"),
                    (BaseKind::Median, "Cota mediana del contorno"),
                    (BaseKind::Interpolated, "Superficie interpolada desde el contorno"),
                    (BaseKind::Level, "Cota fija"),
                    (BaseKind::Dtm, "MDT del terreno de partida"),
                ] {
                    ui.radio_value(&mut t.kind, k, label);
                }
                match t.kind {
                    BaseKind::Level => {
                        ui.horizontal(|ui| {
                            ui.label("Cota");
                            ui.add(NumField::new(&mut t.level).range(-10000.0..=100000.0).suffix(" m"));
                        });
                    }
                    BaseKind::Dtm => crate::pick::raster(ui, "MDT", &mut t.dtm, &rasters),
                    _ => {
                        ui.label(egui::RichText::new("El contorno debe rodear el pie de la pila: lo que mida a lo largo de él se toma como suelo.").color(weak).size(11.0));
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Altura mínima");
                    ui.add(NumField::new(&mut t.min_height).range(0.0..=100.0).suffix(" m"));
                    ui.checkbox(&mut t.density_on, "Densidad del material");
                    if t.density_on {
                        ui.add(NumField::new(&mut t.density).range(0.01..=30.0).suffix(" t/m³"));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Resultados");
                    ui.checkbox(&mut t.csv, "tabla (CSV)");
                    ui.checkbox(&mut t.base_raster, "superficie base");
                    ui.checkbox(&mut t.height_raster, "altura sobre la base");
                });
                let ready = t.surface.is_some()
                    && t.piles.is_some()
                    && (t.kind != BaseKind::Dtm || t.dtm.is_some());
                if crate::procwin::run_bar(ui, accent, "Cubicar", ready, self.running.is_some(), "Faltan datos por elegir", &mut self.tools.s2.stop) {
                    go = true;
                }
            });
        if go {
            let t = &self.pl;
            let (Some(surface), Some(pl)) = (t.surface.clone(), t.piles.clone()) else {
                return;
            };
            let out = paths::unique(&paths::product(&surface, "pilas", "shp"));
            let mut job = PileJob::new(surface, pl, out.clone());
            job.base = match t.kind {
                BaseKind::Plane => Base::Plane,
                BaseKind::Lowest => Base::LowestPerimeter,
                BaseKind::Median => Base::MedianPerimeter,
                BaseKind::Interpolated => Base::Interpolated,
                BaseKind::Level => Base::Level(t.level),
                BaseKind::Dtm => match t.dtm.clone() {
                    Some(d) => Base::Dtm(d),
                    None => return,
                },
            };
            job.min_height = t.min_height;
            job.density = t.density_on.then_some(t.density);
            (job.csv, job.base_raster, job.height_raster) = (t.csv, t.base_raster, t.height_raster);
            let show = vec![out.clone()];
            let task: tools::Task = Box::new(move || piles::run(&job));
            self.start(
                ctx,
                Launch {
                    label: "Cubicación de pilas".into(),
                    task,
                    show,
                    outputs: vec![out],
                },
            );
        }
        if !open {
            self.pl.open = false;
        }
    }
}
