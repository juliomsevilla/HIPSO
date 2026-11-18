//! Igualar rásteres (Ráster): pone rásteres sobre las celdas de una referencia, de modo que
//! coincidan celda a celda.

use super::*;
use hipso_core::paths;
use hipso_core::pyramid::Pyramid;
use hipso_core::rastertools::{self, AlignJob, Dtype, Resample};

/// Lo que dice un ráster de sus celdas, para la lista.
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
    /// 0 automático, luego [`Resample::ALL`].
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

/// El índice 0 es automático; los demás siguen [`Resample::ALL`].
pub fn resample_of(k: usize) -> Option<Resample> {
    k.checked_sub(1).and_then(|k| Resample::ALL.get(k).copied())
}

const AUTO_HELP: &str = "Elige según los datos: si el ráster son números enteros (clases, identificadores) conserva los valores (vecino más cercano, o moda si reduce la resolución); si no, usa la media cuando las celdas de salida son mayores que las de entrada y bilineal en los demás casos.";

/// La lista de métodos de remuestreo con para qué sirve cada uno, compartida por las herramientas
/// que remuestrean.
pub fn resample_picker(ui: &mut egui::Ui, id: &str, k: &mut usize, weak: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label("Remuestreo");
        let name = |k: usize| resample_of(k).map_or("Automático", |r| r.name());
        egui::ComboBox::from_id_salt(id)
            .selected_text(name(*k))
            .show_ui(ui, |ui| {
                ui.selectable_value(k, 0, "Automático");
                for (i, r) in Resample::ALL.iter().enumerate() {
                    ui.selectable_value(k, i + 1, r.name());
                }
            });
    });
    ui.label(
        egui::RichText::new(resample_of(*k).map_or(AUTO_HELP, Resample::describe))
            .color(weak)
            .size(11.0),
    );
}

impl AlignTool {
    /// En qué se diferencia un ráster de la referencia: nada cuando ya coincide.
    fn verdict(&self, c: &Cells) -> (bool, String) {
        let Some((_, r)) = &self.reference else {
            return (true, String::new());
        };
        if let (Some(a), Some(b)) = (r.epsg, c.epsg)
            && a != b
        {
            return (
                true,
                format!("está en EPSG {b} y la referencia en EPSG {a}: se reproyectará a la referencia"),
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
        let accent = self.tokens().accent;
        let mut go = false;
        crate::procwin::window(ctx, "Igualar rásteres")
            .open(&mut open)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.at;
                ui.label(
                    egui::RichText::new("Pone varios rásteres sobre las celdas de uno de referencia (mismo origen, tamaño de celda y dimensiones), para poder apilarlos o usarlos juntos celda a celda, por ejemplo en el análisis por objetos. Cada ráster da un archivo nuevo; los originales no se tocan. Si un ráster está en otro sistema de coordenadas, se reproyecta al de la referencia; así un solo paso lo reproyecta, lo recorta o amplía a la extensión de la referencia y lo remuestrea a su celda.")
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
                        && let Some(p) = crate::prefs::dialog().add_filter("Ráster", hipso_core::raster::EXTENSIONS).pick_file()
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
                    if ui.button("Añadir archivos…").clicked()
                        && let Some(v) = crate::prefs::dialog().add_filter("Ráster", hipso_core::raster::EXTENSIONS).pick_files()
                    {
                        for p in v {
                            if !t.inputs.iter().any(|i| i.0 == p) {
                                let c = describe(&p);
                                t.inputs.push((p, c));
                            }
                        }
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
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
                resample_picker(ui, "Remuestreo", &mut t.resample, weak);
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
                if crate::procwin::run_bar(ui, accent, "Igualar", ready, self.running.is_some(), "Faltan datos por elegir", &mut self.tools.s2.stop) {
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
                resample: resample_of(t.resample),
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
