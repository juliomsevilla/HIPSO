//! Operaciones ráster: tamizar un ráster de clases, medir la distancia a unas celdas y rellenar
//! las celdas vacías con las de alrededor.

use super::*;
use hipso_core::paths;
use hipso_core::{grid::Grid, rasterops};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Sieve,
    Proximity,
    Fill,
}

impl Op {
    pub const ALL: [Op; 3] = [Op::Sieve, Op::Proximity, Op::Fill];

    pub fn label(self) -> &'static str {
        match self {
            Op::Sieve => "Tamizar",
            Op::Proximity => "Proximidad",
            Op::Fill => "Rellenar huecos",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Op::Sieve => "@sieve",
            Op::Proximity => "@proximity",
            Op::Fill => "@fillholes",
        }
    }
}

/// One tool (and window) per operation, in the order of [`Op::ALL`].
pub fn tools() -> Vec<RasterOpsTool> {
    Op::ALL.iter().map(|op| RasterOpsTool::new(*op)).collect()
}

pub struct RasterOpsTool {
    pub open: bool,
    pub op: Op,
    pub input: String,
    /// Sieve: patches with fewer cells than this are merged.
    pub threshold: f64,
    pub eight: bool,
    /// Proximity: the values that count as targets, separated by commas (empty: every non-zero cell).
    pub values: String,
    pub max_distance: String,
    pub in_cells: bool,
    /// Fill: how far to look, in cells, and the passes of smoothing.
    pub fill_distance: f64,
    pub smoothing: f64,
}

impl RasterOpsTool {
    fn new(op: Op) -> Self {
        Self {
            open: false,
            op,
            input: String::new(),
            threshold: 2.0,
            eight: false,
            values: String::new(),
            max_distance: String::new(),
            in_cells: false,
            fill_distance: 100.0,
            smoothing: 0.0,
        }
    }
}

impl RasterOpsTool {
    fn targets(&self) -> Result<rasterops::Targets, String> {
        let t = self.values.trim();
        if t.is_empty() {
            return Ok(rasterops::Targets::NonZero);
        }
        t.split([',', ' ', ';'])
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.replace(',', ".")
                    .parse::<f32>()
                    .map_err(|_| format!("«{s}» no es un número"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(rasterops::Targets::Values)
    }

    fn max_distance(&self) -> Result<Option<f64>, String> {
        let t = self.max_distance.trim();
        if t.is_empty() {
            return Ok(None);
        }
        t.replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|d| *d > 0.0)
            .map(Some)
            .ok_or_else(|| "la distancia máxima debe ser un número positivo".to_string())
    }
}

impl App {
    /// The windows of the raster operations: each one opens its own.
    pub fn rasterops_windows(&mut self, ctx: &egui::Context) {
        for i in 0..self.ro.len() {
            self.rasterops_window(ctx, i);
        }
    }

    fn rasterops_window(&mut self, ctx: &egui::Context, i: usize) {
        if !self.ro[i].open {
            return;
        }
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let weak = self.tokens().weak;
        let red = egui::Color32::from_rgb(200, 60, 40);
        let mut open = true;
        let mut go = false;
        egui::Window::new(self.ro[i].op.label())
            .default_pos([(ctx.content_rect().right() - 560.0).max(320.0), 190.0])
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.ro[i];
                let help = match t.op {
                    Op::Sieve => "Quita los parches pequeños de un ráster de clases: las celdas conexas de un mismo valor que son menos que el umbral toman el valor del parche vecino más grande. Las celdas vacías no se tocan.",
                    Op::Proximity => "Calcula para cada celda la distancia a la celda más cercana de unos valores (distancia euclídea exacta). Las celdas objetivo valen 0.",
                    Op::Fill => "Rellena las celdas vacías con las celdas con valor de alrededor, ponderadas por el inverso del cuadrado de la distancia en ocho direcciones. Lo que no tiene ninguna a su alcance sigue vacío.",
                };
                ui.label(egui::RichText::new(help).color(weak).size(11.0));
                ui.horizontal(|ui| {
                    ui.label("Ráster");
                    ui.add(
                        egui::TextEdit::singleline(&mut t.input)
                            .hint_text("archivo GeoTIFF")
                            .desired_width(300.0),
                    );
                    if ui.button("…").clicked()
                        && let Some(p) = crate::prefs::dialog()
                            .add_filter("Ráster", &["tif", "tiff"])
                            .pick_file()
                    {
                        t.input = p.display().to_string();
                    }
                    ui.menu_button("De las capas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                t.input = path.display().to_string();
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                });
                ui.separator();
                let mut problem: Option<String> = None;
                match t.op {
                    Op::Sieve => {
                        crate::tools::num(ui, "Umbral (celdas)", &mut t.threshold, 2.0..=100000.0, 1.0);
                        ui.checkbox(&mut t.eight, "Las celdas se tocan también por las esquinas (8 vecinas)");
                    }
                    Op::Proximity => {
                        ui.horizontal(|ui| {
                            ui.label("Valores objetivo");
                            ui.add(
                                egui::TextEdit::singleline(&mut t.values)
                                    .hint_text("2, 5 (vacío: toda celda distinta de 0)")
                                    .desired_width(200.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Distancia máxima");
                            ui.add(
                                egui::TextEdit::singleline(&mut t.max_distance)
                                    .hint_text("sin límite")
                                    .desired_width(80.0),
                            );
                        });
                        ui.checkbox(&mut t.in_cells, "Distancias en celdas (si no, en unidades del mapa)");
                        if let Err(m) = t.targets() {
                            problem = Some(m);
                        } else if let Err(m) = t.max_distance() {
                            problem = Some(m);
                        }
                    }
                    Op::Fill => {
                        crate::tools::num(ui, "Alcance (celdas)", &mut t.fill_distance, 1.0..=2000.0, 1.0);
                        crate::tools::num(ui, "Pasadas de suavizado", &mut t.smoothing, 0.0..=20.0, 1.0);
                    }
                }
                if t.input.trim().is_empty() {
                    problem = Some("Elige un ráster.".into());
                } else if !Path::new(t.input.trim()).is_file() {
                    problem = Some("El archivo no existe.".into());
                }
                if let Some(m) = &problem {
                    ui.label(egui::RichText::new(m).color(red).size(11.0));
                }
                if ui
                    .add_enabled(
                        problem.is_none() && self.running.is_none(),
                        egui::Button::new("▶ Ejecutar"),
                    )
                    .clicked()
                {
                    go = true;
                }
            });
        if go {
            let t = &self.ro[i];
            let src = PathBuf::from(t.input.trim());
            let (op, threshold, eight) = (t.op, t.threshold.round().max(1.0) as usize, t.eight);
            let (targets, maxd, cells) = (t.targets(), t.max_distance(), t.in_cells);
            let (reach, smooth) = (t.fill_distance.round().max(1.0) as usize, t.smoothing.round() as usize);
            let (tag, label) = match op {
                Op::Sieve => ("sieve", "Tamizar"),
                Op::Proximity => ("proximity", "Proximidad"),
                Op::Fill => ("relleno", "Rellenar huecos"),
            };
            let dst = paths::unique(&paths::product(&src, tag, "tif"));
            let out = dst.clone();
            let task: tools::Task = Box::new(move || {
                let g = Grid::read(&src)?;
                let msg;
                let r = match op {
                    Op::Sieve => {
                        let conn = if eight {
                            rasterops::Connectivity::Eight
                        } else {
                            rasterops::Connectivity::Four
                        };
                        let r = rasterops::sieve(&g, threshold, conn);
                        let changed = g
                            .data
                            .iter()
                            .zip(&r.data)
                            .filter(|(a, b)| a.to_bits() != b.to_bits())
                            .count();
                        msg = format!("{changed} celdas cambiadas");
                        r
                    }
                    Op::Proximity => {
                        let targets = targets.map_err(hipso_core::Error::Param)?;
                        let maxd = maxd.map_err(hipso_core::Error::Param)?;
                        msg = "distancias calculadas".to_string();
                        rasterops::proximity(&g, &targets, maxd, cells)?
                    }
                    Op::Fill => {
                        let r = rasterops::fill_nodata(&g, reach, smooth);
                        let empty = |g: &Grid| g.data.iter().filter(|v| v.is_nan()).count();
                        let (before, after) = (empty(&g), empty(&r));
                        msg = format!("{} celdas rellenadas, {after} siguen vacías", before - after);
                        r
                    }
                };
                paths::ensure_parent(&dst)?;
                r.write_tiff(&dst, Grid::read_epsg(&src))?;
                Ok(format!("{msg} -> {}", dst.display()))
            });
            self.start(
                ctx,
                Launch {
                    label: label.to_string(),
                    task,
                    show: vec![out.clone()],
                    outputs: vec![out],
                },
            );
        }
        if !open {
            self.ro[i].open = false;
        }
    }
}
