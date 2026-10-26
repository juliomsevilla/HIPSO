//! Igualar rásteres (Ráster): puts rasters on the cells of a reference, so that they coincide
//! cell for cell.

use super::*;
use rustlidar_core::paths;
use rustlidar_core::pyramid::Pyramid;
use rustlidar_core::rastertools::{self, AlignJob, Dtype, Resample};

/// What a raster says about its cells, for the list.
#[derive(Clone)]
pub struct Cells {
    pub text: String,
    pub cell: f64,
    pub size: (u32, u32),
    pub epsg: Option<u16>,
    pub origin: (f64, f64),
}

fn describe(path: &Path) -> Cells {
    match Pyramid::open(path) {
        Ok(p) => {
            let l = p.levels[0];
            let epsg = p.epsg;
            Cells {
                text: format!(
                    "celda {} · {} × {} · {}",
                    paths::num(p.cell),
                    l.cols,
                    l.rows,
                    epsg.map_or("sin sistema de coordenadas".to_string(), |e| format!(
                        "EPSG {e}"
                    ))
                ),
                cell: p.cell,
                size: (l.cols, l.rows),
                epsg,
                origin: (p.x0, p.y_top),
            }
        }
        Err(e) => Cells {
            text: format!("no se puede leer: {e}"),
            cell: 0.0,
            size: (0, 0),
            epsg: None,
            origin: (0.0, 0.0),
        },
    }
}

pub struct AlignTool {
    pub open: bool,
    pub reference: Option<(PathBuf, Cells)>,
    pub inputs: Vec<(PathBuf, Cells)>,
    /// 0 automatic, 1 nearest, 2 bilinear, 3 average.
    pub resample: usize,
    pub nodata: String,
}

impl Default for AlignTool {
    fn default() -> Self {
        Self {
            open: false,
            reference: None,
            inputs: Vec::new(),
            resample: 0,
            nodata: "-9999".into(),
        }
    }
}

const RESAMPLE: [&str; 4] = [
    "Automático (media si es más fino, si no bilineal)",
    "Vecino más cercano (clases)",
    "Bilineal",
    "Media",
];

impl AlignTool {
    /// What differs between a raster and the reference: nothing when it already matches.
    fn verdict(&self, c: &Cells) -> (bool, String) {
        let Some((_, r)) = &self.reference else {
            return (true, String::new());
        };
        if let (Some(a), Some(b)) = (r.epsg, c.epsg)
            && a != b
        {
            return (
                false,
                format!(
                    "otro sistema de coordenadas (EPSG {b}, la referencia EPSG {a}): pásalo antes al sistema de la referencia (easylidar reproject)"
                ),
            );
        }
        let same_cells = (c.cell - r.cell).abs() <= r.cell * 1e-9
            && c.size == r.size
            && (c.origin.0 - r.origin.0).abs() <= r.cell * 1e-6
            && (c.origin.1 - r.origin.1).abs() <= r.cell * 1e-6;
        if same_cells {
            (true, "ya coincide con la referencia".to_string())
        } else {
            (true, String::new())
        }
    }
}

impl App {
    pub fn align_window(&mut self, ctx: &egui::Context) {
        if !self.at.open {
            return;
        }
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. } | RefKind::Image(_)))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Igualar rásteres")
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(600.0)
            .show(ctx, |ui| {
                let t = &mut self.at;
                ui.label(
                    egui::RichText::new("Pone varios rásteres sobre las celdas de uno de referencia (mismo origen, tamaño de celda y dimensiones), para poder apilarlos o usarlos juntos celda a celda, por ejemplo en el análisis por objetos. Cada ráster da un archivo nuevo; los originales no se tocan. Todos deben estar en el mismo sistema de coordenadas; si no, pásalos antes al de la referencia (easylidar reproject).")
                        .color(weak)
                        .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.label("Referencia");
                    ui.label(
                        t.reference
                            .as_ref()
                            .map_or("(elige el ráster cuyas celdas se copian)".to_string(), |r| paths::stem(&r.0)),
                    );
                    if ui.button("Archivo…").clicked()
                        && let Some(p) = crate::prefs::dialog().add_filter("Ráster", rustlidar_core::raster::EXTENSIONS).pick_file()
                    {
                        let c = describe(&p);
                        t.reference = Some((p, c));
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                let c = describe(path);
                                t.reference = Some((path.clone(), c));
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                });
                if let Some((_, c)) = &t.reference {
                    ui.label(egui::RichText::new(format!("   {}", c.text)).color(weak).size(11.0));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Rásteres a igualar");
                    if ui.button("➕ Añadir…").clicked()
                        && let Some(v) = crate::prefs::dialog().add_filter("Ráster", rustlidar_core::raster::EXTENSIONS).pick_files()
                    {
                        for p in v {
                            if !t.inputs.iter().any(|i| i.0 == p) {
                                let c = describe(&p);
                                t.inputs.push((p, c));
                            }
                        }
                    }
                    ui.menu_button("➕ De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                if !t.inputs.iter().any(|i| &i.0 == path) {
                                    let c = describe(path);
                                    t.inputs.push((path.clone(), c));
                                }
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                    if ui.button("Vaciar lista").clicked() {
                        t.inputs.clear();
                    }
                });
                let mut remove = None;
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for (k, (p, c)) in t.inputs.iter().enumerate() {
                        let (ok, note) = t.verdict(c);
                        ui.horizontal(|ui| {
                            if ui
.small_button("🗑")
.on_hover_text("Quitar este ráster de la lista")
.clicked() {
                                remove = Some(k);
                            }
                            ui.label(paths::stem(p));
                            ui.label(egui::RichText::new(&c.text).color(weak).size(11.0));
                        });
                        if !note.is_empty() {
                            let color = if ok { weak } else { egui::Color32::from_rgb(200, 60, 40) };
                            ui.label(egui::RichText::new(format!("      {note}")).color(color).size(11.0));
                        }
                    }
                });
                if let Some(k) = remove {
                    t.inputs.remove(k);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Remuestreo");
                    egui::ComboBox::from_id_salt("Remuestreo")
                    .selected_text(RESAMPLE[t.resample])
                    .show_ui(ui, |ui| {
                        for (k, n) in RESAMPLE.iter().enumerate() {
                            ui.selectable_value(&mut t.resample, k, *n);
                        }
                    });
                });
                ui.label(
                    egui::RichText::new("Usa «Vecino más cercano» si el ráster tiene clases o identificadores (promediarlos los estropea).")
                        .color(weak)
                        .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.label("Valor de «sin datos» en la salida");
                    ui.add(egui::TextEdit::singleline(&mut t.nodata).desired_width(70.0));
                });
                let blocked = t.inputs.iter().any(|(_, c)| !t.verdict(c).0);
                let ready = t.reference.is_some() && !t.inputs.is_empty() && !blocked;
                ui.label(
                    egui::RichText::new("Cada resultado se guarda junto al original como «<nombre>_alineado.tif» (nunca pisa uno anterior).")
                        .color(weak)
                        .size(11.0),
                );
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Igualar")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.at;
            let Some((reference, _)) = t.reference.clone() else {
                return;
            };
            let pairs: Vec<(PathBuf, PathBuf)> = t
                .inputs
                .iter()
                .map(|(p, _)| {
                    (
                        p.clone(),
                        paths::unique(&paths::product(p, "alineado", "tif")),
                    )
                })
                .collect();
            let outs: Vec<PathBuf> = pairs.iter().map(|p| p.1.clone()).collect();
            let job = AlignJob {
                reference,
                inputs: pairs,
                resample: [
                    None,
                    Some(Resample::Nearest),
                    Some(Resample::Bilinear),
                    Some(Resample::Average),
                ][t.resample],
                dtype: None::<Dtype>,
                nodata: t.nodata.trim().replace(',', ".").parse().unwrap_or(-9999.0),
                src_nodata: None,
                compression: None,
            };
            let task: tools::Task = Box::new(move || rastertools::align(&job));
            self.start(
                ctx,
                Launch {
                    label: "Igualar rásteres".to_string(),
                    task,
                    show: outs.clone(),
                    outputs: outs,
                },
            );
        }
        if !open {
            self.at.open = false;
        }
    }
}
