//! Vector (Capas vectoriales): statistics of a raster inside polygons, as a polygon layer with
//! new fields, and operations on polygon layers (check, repair, buffer, dissolve, union).

use super::*;
use hipso_core::{geomops, grid::Grid, paths, vector, zonal};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Zonal,
    Validate,
    Repair,
    Buffer,
    Dissolve,
    Union,
}

impl Op {
    const ALL: [Op; 6] = [Op::Zonal, Op::Validate, Op::Repair, Op::Buffer, Op::Dissolve, Op::Union];

    fn name(self) -> &'static str {
        match self {
            Op::Zonal => "Estadística zonal (ráster por polígono)",
            Op::Validate => "Comprobar si los polígonos son válidos",
            Op::Repair => "Reparar polígonos no válidos",
            Op::Buffer => "Búfer (ampliar o reducir)",
            Op::Dissolve => "Disolver (unir polígonos por un campo)",
            Op::Union => "Unión de dos capas",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Op::Zonal => "Para cada polígono calcula, con las celdas del ráster cuyo centro cae dentro: número de celdas, mínimo, máximo, media, suma, desviación típica y mediana. El resultado es una capa de polígonos nueva con los campos originales y un campo más por cada estadístico. Sirve, por ejemplo, para obtener el CHM medio de cada rodal.",
            Op::Validate => "Busca polígonos con anillos sin cerrar, autointersecciones, huecos fuera del polígono o partes solapadas.",
            Op::Repair => "Repara los polígonos no válidos con la regla par-impar (un lazo se convierte en dos partes, los bordes repetidos se cancelan). Los válidos no se tocan y la tabla de atributos se conserva.",
            Op::Buffer => "Amplía cada polígono la distancia indicada (negativa: lo reduce), con esquinas redondeadas. La distancia va en las unidades de la capa: metros en un sistema proyectado, grados en uno geográfico.",
            Op::Dissolve => "Une los polígonos que comparten el valor de un campo (o todos, si no eliges campo). El campo «n» cuenta cuántos se unieron.",
            Op::Union => "Superpone dos capas: da las piezas que están en las dos (con los campos de ambas, los de la segunda precedidos por «b_»), solo en la primera y solo en la segunda.",
        }
    }
}

pub struct VectorTool {
    pub open: bool,
    pub op: Op,
    pub layer: Option<PathBuf>,
    pub second: Option<PathBuf>,
    pub raster: Option<PathBuf>,
    pub prefix: String,
    pub distance: f64,
    pub field: String,
    pub repair_first: bool,
    pub report: String,
}

impl Default for VectorTool {
    fn default() -> Self {
        Self {
            open: false,
            op: Op::Zonal,
            layer: None,
            second: None,
            raster: None,
            prefix: String::new(),
            distance: 10.0,
            field: String::new(),
            repair_first: false,
            report: String::new(),
        }
    }
}

fn pick_vector(slot: &mut Option<PathBuf>, ui: &mut egui::Ui, label: &str, loaded: &[(String, PathBuf)]) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.label(slot.as_ref().map_or("(sin elegir)".to_string(), |p| paths::stem(p)));
        if ui.button("Archivo…").clicked()
            && let Some(p) = crate::prefs::dialog()
                .add_filter("Capa de polígonos", &["shp", "geojson", "json", "gpkg"])
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
                ui.label("No hay capas vectoriales cargadas");
            }
        });
    });
}

impl App {
    pub fn vector_window(&mut self, ctx: &egui::Context) {
        if !self.vt.open {
            return;
        }
        let vectors: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Vector { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let rasters: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Capas vectoriales: operaciones")
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.vt;
                ui.horizontal(|ui| {
                    ui.label("Operación");
                    egui::ComboBox::from_id_salt("vector_op")
                        .selected_text(t.op.name())
                        .show_ui(ui, |ui| {
                            for o in Op::ALL {
                                ui.selectable_value(&mut t.op, o, o.name());
                            }
                        });
                });
                ui.label(egui::RichText::new(t.op.help()).color(weak).size(11.0));
                ui.separator();
                if t.op == Op::Zonal {
                    ui.horizontal(|ui| {
                        ui.label("Ráster");
                        ui.label(t.raster.as_ref().map_or("(sin elegir)".to_string(), |p| paths::stem(p)));
                        if ui.button("Archivo…").clicked()
                            && let Some(p) = crate::prefs::dialog().add_filter("Ráster", hipso_core::raster::EXTENSIONS).pick_file()
                        {
                            t.raster = Some(p);
                        }
                        ui.menu_button("De las capas cargadas", |ui| {
                            for (name, path) in &rasters {
                                if ui.button(name).clicked() {
                                    t.raster = Some(path.clone());
                                    ui.close();
                                }
                            }
                            if rasters.is_empty() {
                                ui.label("No hay rásteres cargados");
                            }
                        });
                    });
                }
                let label = if t.op == Op::Union { "Primera capa" } else { "Polígonos" };
                pick_vector(&mut t.layer, ui, label, &vectors);
                if t.op == Op::Union {
                    pick_vector(&mut t.second, ui, "Segunda capa", &vectors);
                }
                match t.op {
                    Op::Zonal => {
                        ui.horizontal(|ui| {
                            ui.label("Prefijo de los campos nuevos");
                            ui.add(egui::TextEdit::singleline(&mut t.prefix).desired_width(70.0));
                            ui.label(egui::RichText::new("p. ej. «chm_» da chm_mean; un campo de shapefile tiene 10 caracteres como máximo").color(weak).size(11.0));
                        });
                    }
                    Op::Buffer => {
                        ui.horizontal(|ui| {
                            ui.label("Distancia");
                            ui.add(crate::numfield::NumField::new(&mut t.distance).range(-100000.0..=100000.0).suffix(" m"));
                        });
                    }
                    Op::Dissolve => {
                        ui.horizontal(|ui| {
                            ui.label("Campo (vacío: todos en uno)");
                            ui.add(egui::TextEdit::singleline(&mut t.field).desired_width(120.0));
                        });
                    }
                    _ => {}
                }
                if matches!(t.op, Op::Buffer | Op::Dissolve | Op::Union) {
                    ui.checkbox(&mut t.repair_first, "Reparar antes los polígonos no válidos");
                }
                let ready = t.layer.is_some()
                    && (t.op != Op::Zonal || t.raster.is_some())
                    && (t.op != Op::Union || t.second.is_some());
                ui.horizontal(|ui| {
                    if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Ejecutar")).clicked() {
                        go = true;
                    }
                    ui.label(egui::RichText::new("El resultado es una capa nueva junto a la original; los originales no se tocan.").color(weak).size(11.0));
                });
                if !t.report.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&t.report).size(12.0));
                }
            });
        if go {
            let t = &mut self.vt;
            let (Some(layer), op) = (t.layer.clone(), t.op) else {
                return;
            };
            if op == Op::Validate {
                // quick: no new file
                t.report = match geomops::read_polygons(&layer).map(|l| (geomops::invalid(&l), l.features.len())) {
                    Ok((bad, n)) if bad.is_empty() => format!("{n} polígonos, todos válidos."),
                    Ok((bad, n)) => format!(
                        "{} de {n} polígonos no son válidos:\n{}",
                        bad.len(),
                        bad.iter().take(12).map(|(i, w)| format!("  polígono {}: {w}", i + 1)).collect::<Vec<_>>().join("\n")
                    ),
                    Err(e) => format!("Error: {e}"),
                };
            } else {
                let tag = match op {
                    Op::Zonal => "zonal",
                    Op::Repair => "valido",
                    Op::Buffer => "buffer",
                    Op::Dissolve => "disuelto",
                    Op::Union => "union",
                    Op::Validate => unreachable!(),
                };
                let out = paths::unique(&paths::product(&layer, tag, "shp"));
                let (raster, second) = (t.raster.clone(), t.second.clone());
                let (prefix, distance, field, repair) = (t.prefix.clone(), t.distance, t.field.trim().to_string(), t.repair_first);
                let target = out.clone();
                let task: tools::Task = Box::new(move || {
                    let mut l = geomops::read_polygons(&layer)?;
                    if repair {
                        l = geomops::make_valid(&l)?.0;
                    }
                    let (result, note) = match op {
                        Op::Zonal => {
                            let g = Grid::read(raster.as_deref().expect("raster"))?;
                            let regions = vector::read_regions(&layer, &vector::ReadOptions::default())?;
                            let mut r = zonal::zonal_layer(&g, &regions, &prefix)?;
                            r.prj = l.prj.clone();
                            let n = r.features.len();
                            (r, format!("{n} polígonos con {} campos nuevos", zonal::FIELDS.len()))
                        }
                        Op::Repair => {
                            let (r, n) = geomops::make_valid(&l)?;
                            let total = r.features.len();
                            (r, format!("{n} de {total} polígonos reparados"))
                        }
                        Op::Buffer => (geomops::buffer(&l, distance)?, format!("buffer de {distance}")),
                        Op::Dissolve => {
                            let r = geomops::dissolve(&l, (!field.is_empty()).then_some(field.as_str()))?;
                            let n = r.features.len();
                            (r, format!("{n} polígonos tras disolver"))
                        }
                        Op::Union => {
                            let mut b = geomops::read_polygons(second.as_deref().expect("segunda"))?;
                            if repair {
                                b = geomops::make_valid(&b)?.0;
                            }
                            let r = geomops::union(&l, &b)?;
                            let n = r.features.len();
                            (r, format!("{n} piezas"))
                        }
                        Op::Validate => unreachable!(),
                    };
                    geomops::write_polygons(&result, &target)?;
                    Ok(format!("{note} -> {}", target.display()))
                });
                self.start(
                    ctx,
                    Launch {
                        label: op.name().to_string(),
                        task,
                        show: vec![out.clone()],
                        outputs: vec![out],
                    },
                );
            }
        }
        if !open {
            self.vt.open = false;
        }
    }
}
