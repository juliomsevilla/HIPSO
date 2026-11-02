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
                    if ui.button("➕ Añadir…").clicked()
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
                    ui.menu_button("➕ De las capas cargadas", |ui| {
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
                    ui.label("Sistema de destino (EPSG)");
                    ui.add(egui::TextEdit::singleline(&mut t.to).desired_width(80.0));
                    ui.label("Origen, si el archivo no lo declara");
                    ui.add(egui::TextEdit::singleline(&mut t.from).desired_width(80.0));
                });
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
                let to_ok = parse_code(&t.to).is_some();
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
            let (Some(to), Ok(vertical)) = (parse_code(&t.to), t.vertical()) else {
                return;
            };
            let from = parse_code(&t.from);
            let policy = t.policy();
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
                    lines.push(reproject::file_with(src, dst, to, from, vertical, policy)?);
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
