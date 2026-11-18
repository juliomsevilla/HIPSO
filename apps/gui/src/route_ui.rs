//! Ruta de menor coste (Ráster › Análisis del terreno › Visibilidad y rutas): el camino más barato
//! sobre un MDT entre puntos.

use super::*;
use crate::numfield::NumField;
use hipso_core::pathfind::{self, CostModel, PathJob};
use hipso_core::paths;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Model {
    Slope,
    Tobler,
    Uniform,
    Raster,
}

pub struct RouteTool {
    pub open: bool,
    dtm: Option<PathBuf>,
    from: String,
    via: String,
    to: String,
    nearest: bool,
    model: Model,
    power: f64,
    reference: f64,
    cost_raster: Option<PathBuf>,
    max_slope_on: bool,
    max_slope: f64,
    barriers: Option<PathBuf>,
    prefer: Option<PathBuf>,
    prefer_factor: f64,
    cost_distance: bool,
}

impl Default for RouteTool {
    fn default() -> Self {
        Self {
            open: false,
            dtm: None,
            from: String::new(),
            via: String::new(),
            to: String::new(),
            nearest: false,
            model: Model::Slope,
            power: 2.0,
            reference: 10.0,
            cost_raster: None,
            max_slope_on: false,
            max_slope: 30.0,
            barriers: None,
            prefer: None,
            prefer_factor: 0.5,
            cost_distance: false,
        }
    }
}

/// Los puntos `x y` de un texto, uno por línea.
fn points(text: &str) -> Option<Vec<[f64; 2]>> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (x, y) = l.trim().split_once([',', ';', ' ', '\t'])?;
            Some([
                x.trim().replace(',', ".").parse().ok()?,
                y.trim().replace(',', ".").parse().ok()?,
            ])
        })
        .collect()
}

fn one(text: &str) -> Option<[f64; 2]> {
    points(text).filter(|p| p.len() == 1).map(|p| p[0])
}

impl App {
    pub fn route_window(&mut self, ctx: &egui::Context) {
        if !self.rt_route.open {
            return;
        }
        let (rasters, vectors) = (self.loaded_rasters(), self.loaded_vectors());
        let weak = self.tokens().weak;
        let mut open = true;
        let accent = self.tokens().accent;
        let mut go = false;
        crate::procwin::window(ctx, "Ruta de menor coste")
            .open(&mut open)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.rt_route;
                ui.label(
                    egui::RichText::new("Busca el camino más barato sobre un MDT entre un origen y un destino, con paradas si se quiere. Se mueve por las ocho celdas vecinas, así que sigue algo la retícula (hasta un 8 % más larga que la recta). Da la ruta como una línea 3D con su longitud, pendientes, subida y bajada. Metros y sistema métrico; las coordenadas son las del mapa.")
                        .color(weak)
                        .size(11.0),
                );
                crate::pick::raster(ui, "MDT", &mut t.dtm, &rasters);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Origen, «x y»");
                    ui.add(egui::TextEdit::singleline(&mut t.from).desired_width(220.0));
                });
                ui.label("Paradas intermedias (una por línea, opcional)");
                ui.add(egui::TextEdit::multiline(&mut t.via).desired_rows(2).desired_width(f32::INFINITY));
                ui.checkbox(&mut t.nearest, "Varios destinos: ir al más barato");
                ui.label(if t.nearest { "Destinos (uno por línea)" } else { "Destino, «x y»" });
                if t.nearest {
                    ui.add(egui::TextEdit::multiline(&mut t.to).desired_rows(2).desired_width(f32::INFINITY));
                } else {
                    ui.add(egui::TextEdit::singleline(&mut t.to).desired_width(220.0));
                }
                ui.separator();
                ui.label("Qué cuesta");
                ui.radio_value(&mut t.model, Model::Slope, "La pendiente: 1 + (pendiente / referencia) ^ potencia por metro");
                ui.radio_value(&mut t.model, Model::Tobler, "Tiempo de marcha a pie (función de Tobler, distingue subida y bajada)");
                ui.radio_value(&mut t.model, Model::Uniform, "Nada: la ruta más corta");
                ui.radio_value(&mut t.model, Model::Raster, "Un ráster de costes por metro, con las celdas del MDT");
                match t.model {
                    Model::Slope => {
                        ui.horizontal(|ui| {
                            ui.label("Potencia");
                            ui.add(NumField::new(&mut t.power).range(0.1..=10.0));
                            ui.label("Pendiente de referencia");
                            ui.add(NumField::new(&mut t.reference).range(0.1..=1000.0).suffix(" %"));
                        });
                    }
                    Model::Raster => crate::pick::raster(ui, "Costes", &mut t.cost_raster, &rasters),
                    _ => {}
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.checkbox(&mut t.max_slope_on, "No cruzar pendientes mayores de");
                    if t.max_slope_on {
                        ui.add(NumField::new(&mut t.max_slope).range(1.0..=1000.0).suffix(" %"));
                    }
                });
                crate::pick::vector(ui, "Barreras (opcional)", &mut t.barriers, &vectors);
                crate::pick::vector(ui, "Zonas preferidas (opcional)", &mut t.prefer, &vectors);
                if t.prefer.is_some() {
                    ui.horizontal(|ui| {
                        ui.label("El coste ahí se multiplica por");
                        ui.add(NumField::new(&mut t.prefer_factor).range(0.01..=100.0));
                    });
                }
                ui.checkbox(&mut t.cost_distance, "Guardar también el ráster de coste acumulado desde el origen");
                let from_ok = one(&t.from).is_some();
                let via_ok = points(&t.via).is_some();
                let to_ok = if t.nearest { points(&t.to).is_some_and(|p| !p.is_empty()) } else { one(&t.to).is_some() };
                if !(from_ok && via_ok && to_ok) {
                    ui.label(egui::RichText::new("Escribe las coordenadas «x y» del origen y del destino.").color(weak).size(11.0));
                }
                let ready = t.dtm.is_some()
                    && from_ok
                    && via_ok
                    && to_ok
                    && (t.model != Model::Raster || t.cost_raster.is_some());
                if crate::procwin::run_bar(ui, accent, "Buscar la ruta", ready, self.running.is_some(), "Faltan datos por elegir", &mut self.tools.s2.stop) {
                    go = true;
                }
            });
        if go {
            let t = &self.rt_route;
            let Some(dtm) = t.dtm.clone() else {
                return;
            };
            let (Some(from), Some(via), Some(to)) = (one(&t.from), points(&t.via), points(&t.to))
            else {
                return;
            };
            let mut pts = vec![from];
            pts.extend(via);
            pts.extend(to);
            let out = paths::unique(&paths::product(&dtm, "ruta", "shp"));
            let mut job = PathJob::new(dtm, pts, out.clone());
            job.model = match t.model {
                Model::Slope => CostModel::Slope {
                    power: t.power,
                    reference_pct: t.reference,
                },
                Model::Tobler => CostModel::Tobler,
                Model::Uniform => CostModel::Uniform,
                Model::Raster => match t.cost_raster.clone() {
                    Some(r) => CostModel::Raster(r),
                    None => return,
                },
            };
            job.constraints.max_slope_pct = t.max_slope_on.then_some(t.max_slope);
            job.constraints.barriers = t.barriers.clone();
            job.constraints.prefer = t.prefer.clone().map(|p| (p, t.prefer_factor));
            job.nearest = t.nearest;
            job.cost_distance = t.cost_distance;
            let task: tools::Task = Box::new(move || pathfind::run(&job));
            self.start(
                ctx,
                Launch {
                    label: "Ruta de menor coste".into(),
                    task,
                    show: vec![out.clone()],
                    outputs: vec![out],
                },
            );
        }
        if !open {
            self.rt_route.open = false;
        }
    }
}
