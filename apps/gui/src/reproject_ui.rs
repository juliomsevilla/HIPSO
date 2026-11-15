//! Reproyectar (Ráster y nubes): moves clouds and rasters to another coordinate system and,
//! when asked, changes the vertical reference of their heights.

use super::*;
use hipso_core::paths;
use hipso_core::reproject::{self, Policy, Vertical, VerticalRef};

/// What the user picked for a vertical reference.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VChoice {
    /// Leave heights as they are (output) / the file's own declaration (input).
    Keep,
    Ellipsoidal,
    Code,
}

pub struct FileInfo {
    pub epsg: Option<u16>,
    pub vertical: Option<u16>,
}

pub struct ReprojectTool {
    pub open: bool,
    pub inputs: Vec<(PathBuf, FileInfo)>,
    pub to: String,
    pub from: String,
    pub v_in: VChoice,
    pub v_in_code: String,
    pub v_out: VChoice,
    pub v_out_code: String,
    pub allow_ballpark: bool,
    pub allow_fallback: bool,
    /// Rasters: 0 automatic, then the methods of `Resample::ALL` (see `align_ui::resample_picker`).
    pub resample: usize,
    /// Rasters: cell of the result as typed (empty: the same, or the equivalent in the new system).
    pub cell: String,
    /// Destination: 0 an EPSG code, 1 the system of a reference raster, 2 the cells of a reference
    /// raster (system, extent and cell; rasters only).
    pub dest: u8,
    pub reference: Option<PathBuf>,
}

impl Default for ReprojectTool {
    fn default() -> Self {
        Self {
            open: false,
            inputs: Vec::new(),
            to: String::new(),
            from: String::new(),
            v_in: VChoice::Keep,
            v_in_code: String::new(),
            v_out: VChoice::Keep,
            v_out_code: String::new(),
            allow_ballpark: false,
            allow_fallback: false,
            resample: 0,
            cell: String::new(),
            dest: 0,
            reference: None,
        }
    }
}

fn info(p: &Path) -> FileInfo {
    FileInfo {
        epsg: hipso_core::crs::epsg_of(p),
        vertical: hipso_core::crs::vertical_epsg_of(p),
    }
}

fn parse_code(s: &str) -> Option<u16> {
    s.trim()
        .trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == ':')
        .parse()
        .ok()
}

impl ReprojectTool {
    /// The EPSG code to write: the one typed, or that of the reference raster.
    fn target(&self) -> Option<u16> {
        match self.dest {
            0 => parse_code(&self.to),
            _ => self
                .reference
                .as_ref()
                .and_then(|r| hipso_core::crs::epsg_of(r)),
        }
    }

    fn vertical(&self) -> Result<Vertical, String> {
        let pick = |c: VChoice, code: &str| -> Result<Option<VerticalRef>, String> {
            match c {
                VChoice::Keep => Ok(None),
                VChoice::Ellipsoidal => Ok(Some(VerticalRef::Ellipsoidal)),
                VChoice::Code => parse_code(code)
                    .map(|c| Some(VerticalRef::Epsg(c)))
                    .ok_or_else(|| "código EPSG vertical no válido".to_string()),
            }
        };
        Ok(Vertical {
            input: pick(self.v_in, &self.v_in_code)?,
            output: pick(self.v_out, &self.v_out_code)?,
        })
    }

    fn policy(&self) -> Policy {
        let mut p = Policy::from_env();
        p.allow_ballpark |= self.allow_ballpark;
        p.only_best &= !self.allow_fallback;
        p
    }

    /// Why a file cannot be processed as set, if so.
    fn problem(&self, f: &FileInfo, v: &Vertical) -> Option<String> {
        if f.epsg.is_none() && parse_code(&self.from).is_none() {
            return Some("no declara su sistema: indica el de origen".into());
        }
        if v.output.is_some() && v.input.is_none() && f.vertical.is_none() {
            return Some(
                "no declara su referencia vertical: elige la de origen (elipsoidal o un código)"
                    .into(),
            );
        }
        None
    }
}

impl App {
    pub fn reproject_window(&mut self, ctx: &egui::Context) {
        if !self.rp.open {
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
        egui::Window::new("Reproyectar")
            .default_pos([(ctx.content_rect().right() - 560.0).max(320.0), 190.0])
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(600.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.rp;
                ui.label(
                    egui::RichText::new("Pasa nubes (LAS/LAZ) y rásteres a otro sistema de coordenadas y, si lo pides, cambia la referencia vertical de las alturas (por ejemplo, de elipsoidales a ortométricas con el modelo de geoide). Cada resultado es un archivo nuevo; los originales no se tocan. Si una transformación no puede hacerse con precisión (falta una rejilla o el punto cae fuera de su cobertura) se rechaza con un error, salvo que aceptes aproximaciones.")
                        .color(weak)
                        .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.label("Archivos");
                    if ui.button("Añadir archivos…").clicked()
                        && let Some(v) = crate::prefs::dialog()
                            .add_filter("Nubes y rásteres", &["las", "laz", "tif", "tiff"])
                            .pick_files()
                    {
                        for p in v {
                            if !t.inputs.iter().any(|i| i.0 == p) {
                                let i = info(&p);
                                t.inputs.push((p, i));
                            }
                        }
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                if !t.inputs.iter().any(|i| &i.0 == path) {
                                    let i = info(path);
                                    t.inputs.push((path.clone(), i));
                                }
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay capas cargadas");
                        }
                    });
                    if ui.button("Vaciar lista").clicked() {
                        t.inputs.clear();
                    }
                });
                let vertical = t.vertical();
                let mut remove = None;
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    for (k, (p, f)) in t.inputs.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.small_button("🗑").on_hover_text("Quitar de la lista").clicked() {
                                remove = Some(k);
                            }
                            ui.label(paths::stem(p));
                            let h = f.epsg.map_or("sin sistema".to_string(), |e| format!("EPSG {e}"));
                            let v = f.vertical.map_or(String::new(), |v| format!(" + vertical EPSG {v}"));
                            ui.label(egui::RichText::new(format!("{h}{v}")).color(weak).size(11.0));
                        });
                        if let Ok(v) = &vertical
                            && let Some(m) = t.problem(f, v)
                        {
                            ui.label(egui::RichText::new(format!("      {m}")).color(red).size(11.0));
                        }
                    }
                });
                if let Some(k) = remove {
                    t.inputs.remove(k);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Destino");
                    ui.radio_value(&mut t.dest, 0, "Código EPSG");
                    ui.radio_value(&mut t.dest, 1, "Sistema de un ráster");
                    ui.radio_value(&mut t.dest, 2, "Celdas de un ráster");
                });
                if t.dest != 0 {
                    ui.horizontal(|ui| {
                        ui.label("Ráster de referencia");
                        ui.label(t.reference.as_ref().map_or("(elige uno)".to_string(), |r| paths::stem(r)));
                        if ui.button("Archivo…").clicked()
                            && let Some(p) = crate::prefs::dialog().add_filter("Ráster", hipso_core::raster::EXTENSIONS).pick_file()
                        {
                            t.reference = Some(p);
                        }
                        ui.menu_button("De las capas cargadas", |ui| {
                            for (name, path) in &loaded {
                                if ui.button(name).clicked() {
                                    t.reference = Some(path.clone());
                                    ui.close();
                                }
                            }
                            if loaded.is_empty() {
                                ui.label("No hay rásteres cargados");
                            }
                        });
                    });
                    let note = match (&t.reference, t.target()) {
                        (Some(_), Some(e)) => format!("Sistema de destino: EPSG {e}."),
                        (Some(_), None) => "La referencia no declara su sistema de coordenadas.".to_string(),
                        _ => String::new(),
                    };
                    let extra = if t.dest == 2 { " Los rásteres toman también la extensión, el origen y la celda de la referencia; las nubes solo su sistema." } else { " Los rásteres conservan su extensión; elige «Celdas de un ráster» para igualarla también." };
                    ui.label(egui::RichText::new(format!("{note}{extra}")).color(weak).size(11.0));
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Sistema de destino (EPSG)");
                        ui.add(egui::TextEdit::singleline(&mut t.to).desired_width(80.0));
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Origen, si el archivo no lo declara");
                    ui.add(egui::TextEdit::singleline(&mut t.from).desired_width(80.0));
                });
                ui.separator();
                ui.label(egui::RichText::new("Rásteres").strong());
                crate::align_ui::resample_picker(ui, "rp_resample", &mut t.resample, weak);
                ui.horizontal(|ui| {
                    ui.label("Celda del resultado");
                    ui.add(egui::TextEdit::singleline(&mut t.cell).desired_width(70.0).hint_text("la misma"));
                    ui.label(egui::RichText::new("Con «Celdas de un ráster» el resultado coincide con él celda a celda (también en «Igualar rásteres»).").color(weak).size(11.0));
                });
                ui.separator();
                let combo = |ui: &mut egui::Ui, id: &str, label: &str, c: &mut VChoice, code: &mut String, keep: &str| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(id)
                            .selected_text(match c {
                                VChoice::Keep => keep,
                                VChoice::Ellipsoidal => "Elipsoidal",
                                VChoice::Code => "Código EPSG vertical",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(c, VChoice::Keep, keep);
                                ui.selectable_value(c, VChoice::Ellipsoidal, "Elipsoidal");
                                ui.selectable_value(c, VChoice::Code, "Código EPSG vertical");
                            });
                        if *c == VChoice::Code {
                            ui.add(egui::TextEdit::singleline(code).desired_width(70.0));
                        }
                    });
                };
                combo(ui, "rp_vin", "Alturas de entrada", &mut t.v_in, &mut t.v_in_code, "Las del archivo");
                combo(ui, "rp_vout", "Alturas de salida", &mut t.v_out, &mut t.v_out_code, "No cambiar las alturas");
                ui.checkbox(&mut t.allow_ballpark, "Aceptar aproximaciones (sin ajuste de datum, o altura sin corregir donde no hay geoide)");
                ui.checkbox(&mut t.allow_fallback, "Aceptar una transformación menos precisa si falta la mejor (por ejemplo, una rejilla)");
                ui.label(
                    egui::RichText::new("Las rejillas de geoide y de datum se buscan en HIPSO_GRID_DIR, PROJ_DATA y PROJ_LIB; las de España vienen incluidas. Cada resultado se guarda junto al original como «<nombre>_epsg<código>».")
                        .color(weak)
                        .size(11.0),
                );
                let to_ok = t.target().is_some();
                let problems = match &vertical {
                    Ok(v) => t.inputs.iter().any(|(_, f)| t.problem(f, v).is_some()),
                    Err(m) => {
                        ui.label(egui::RichText::new(m).color(red).size(11.0));
                        true
                    }
                };
                let ready = to_ok && !t.inputs.is_empty() && !problems;
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Reproyectar")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.rp;
            let (Some(to), Ok(vertical)) = (t.target(), t.vertical()) else {
                return;
            };
            let from = parse_code(&t.from);
            let policy = t.policy();
            let how = crate::align_ui::resample_of(t.resample);
            let cells_of = if t.dest == 2 { t.reference.clone() } else { None };
            let cell = t.cell.trim().replace(',', ".").parse::<f64>().ok().filter(|c| *c > 0.0);
            let jobs: Vec<(PathBuf, PathBuf)> = t
                .inputs
                .iter()
                .map(|(p, _)| {
                    let ext = p
                        .extension()
                        .map_or("las".into(), |e| e.to_string_lossy().to_ascii_lowercase());
                    (
                        p.clone(),
                        paths::unique(&paths::product(p, &format!("epsg{to}"), &ext)),
                    )
                })
                .collect();
            let outs: Vec<PathBuf> = jobs.iter().map(|j| j.1.clone()).collect();
            let task: tools::Task = Box::new(move || {
                let mut lines = Vec::new();
                for (src, dst) in &jobs {
                    let raster = src
                        .extension()
                        .is_some_and(|e| hipso_core::raster::is_raster_ext(&e.to_string_lossy().to_ascii_lowercase()));
                    lines.push(if raster && let Some(r) = &cells_of {
                        let mut j = hipso_core::rastertools::WarpJob::new(
                            src.clone(),
                            dst.with_extension("tif"),
                            hipso_core::rastertools::Target::Reference(r.clone()),
                        );
                        j.from = from;
                        j.resample = how;
                        hipso_core::rastertools::warp(&j)?
                    } else if raster && vertical.output.is_none() {
                        reproject::raster_file(src, &dst.with_extension("tif"), to, from, how, cell, None, policy)?
                    } else {
                        reproject::file_with(src, dst, to, from, vertical, policy)?
                    });
                }
                Ok(lines.join("\n"))
            });
            self.start(
                ctx,
                Launch {
                    label: "Reproyectar".to_string(),
                    task,
                    show: outs.clone(),
                    outputs: outs,
                },
            );
        }
        if !open {
            self.rp.open = false;
        }
    }
}
