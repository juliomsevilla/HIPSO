//! El sistema de coordenadas del trabajo cuando lo que se carga no declara ninguno: si HIPSO aún no
//! tiene un sistema definido, se pide el código EPSG en una ventana. La respuesta vale para la
//! sesión y se guarda en el proyecto; los archivos no se modifican.

use super::*;

/// Los códigos que se ofrecen en la lista de la ventana.
const COMMON: [u16; 10] = [
    25828, 25829, 25830, 25831, 4258, 32628, 32630, 32631, 4326, 3857,
];

/// La ventana abierta: los elementos sin sistema y lo que se lleva escrito.
pub struct CrsAsk {
    paths: Vec<PathBuf>,
    code: String,
}

/// El código escrito, si es un sistema que se sabe transformar.
fn known(text: &str) -> Option<u16> {
    let t = text.trim();
    let t = t
        .strip_prefix("EPSG:")
        .or_else(|| t.strip_prefix("epsg:"))
        .unwrap_or(t);
    let code: u16 = t.trim().parse().ok()?;
    hipso_core::reproject::transformer(code, 4326)
        .is_ok()
        .then_some(code)
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

impl App {
    /// Da un sistema a las nubes que no declaran ninguno.
    pub fn give_clouds_crs(&mut self, code: u16) {
        for p in self.clouds.clone() {
            if self.crs_of(&p).is_none() {
                self.crs_given.insert(p, code);
            }
        }
    }

    /// Da por definido el sistema que ya se deduce de lo cargado y, si no hay de dónde, devuelve
    /// los elementos que no declaran ninguno.
    fn settle_crs(&mut self) -> Vec<PathBuf> {
        // mientras algo se está cargando no se sabe todavía si declara un sistema
        if self.project_crs().is_some() || self.loading.is_some() || !self.img_jobs.is_empty() {
            return Vec::new();
        }
        let layers: Vec<(PathBuf, Option<[f64; 4]>)> = (0..self.refs.len())
            .map(|k| (self.refs[k].path.clone(), self.layer_extent(k)))
            .collect();
        let cloud = self.store.as_ref().map(|s| {
            let b = s.bounds;
            [b[0], b[1], b[3], b[4]]
        });
        for (path, extent) in &layers {
            let Some(code) = self.crs_of(path) else {
                continue;
            };
            if self.clouds.is_empty() {
                self.work_crs = Some(code);
                return Vec::new();
            }
            // una capa con sistema que cae sobre una nube sin él dice en cuál está la nube
            if let (Some(c), Some(e)) = (cloud, extent)
                && overlaps(c, *e)
            {
                self.give_clouds_crs(code);
                self.log(format!(
                    "La nube no declara su sistema de coordenadas: se toma {}, el de la capa «{}», que cae sobre ella.",
                    hipso_core::crs::name(code),
                    path.display()
                ));
                return Vec::new();
            }
        }
        let mut out: Vec<PathBuf> = Vec::new();
        let clouds = if self.store.is_some() {
            self.clouds.clone()
        } else {
            Vec::new()
        };
        for p in clouds.into_iter().chain(layers.into_iter().map(|l| l.0)) {
            if !out.contains(&p)
                && !self.crs_skipped.contains(&p)
                && p.is_file()
                && self.crs_of(&p).is_none()
            {
                out.push(p);
            }
        }
        out
    }

    /// La respuesta de la ventana: todos los elementos preguntados pasan a ese sistema.
    pub fn accept_crs(&mut self, paths: &[PathBuf], code: u16) {
        for p in paths {
            self.crs_given.insert(p.clone(), code);
        }
        if self.clouds.is_empty() {
            self.work_crs = Some(code);
        }
        self.log(format!(
            "Sistema de coordenadas definido: {}. Vale para esta sesión y el proyecto; los archivos no se modifican.",
            hipso_core::crs::name(code)
        ));
    }

    pub fn crs_window(&mut self, ctx: &egui::Context) {
        let pending = self.settle_crs();
        if pending.is_empty() {
            self.crs_ask = None;
            return;
        }
        let mut ask = self.crs_ask.take().unwrap_or_else(|| CrsAsk {
            paths: Vec::new(),
            code: String::new(),
        });
        ask.paths = pending;
        let (mut open, mut accept, mut skip) = (true, None, false);
        egui::Window::new("Sistema de coordenadas")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label("No declara su sistema de coordenadas, y todavía no hay ninguno definido:");
                for p in ask.paths.iter().take(6) {
                    let name = p
                        .file_name()
                        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
                    ui.strong(name);
                }
                if ask.paths.len() > 6 {
                    ui.label(format!("… y {} más", ask.paths.len() - 6));
                }
                ui.add_space(6.0);
                let typed = known(&ask.code);
                ui.horizontal(|ui| {
                    ui.label("EPSG");
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut ask.code)
                            .hint_text("25830")
                            .desired_width(80.0),
                    );
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        accept = typed;
                    }
                    egui::ComboBox::from_id_salt("crs_common")
                        .selected_text("Habituales")
                        .width(250.0)
                        .show_ui(ui, |ui| {
                            for c in COMMON {
                                if ui
                                    .selectable_label(typed == Some(c), hipso_core::crs::name(c))
                                    .clicked()
                                {
                                    ask.code = c.to_string();
                                }
                            }
                        });
                });
                match known(&ask.code) {
                    Some(c) => ui.label(hipso_core::crs::name(c)),
                    None if ask.code.trim().is_empty() => ui.weak("Escribe un código o elige uno de la lista."),
                    None => ui.weak("Ese código no se reconoce."),
                };
                ui.add_space(6.0);
                ui.weak("Vale para esta sesión y para el proyecto. El archivo no se modifica: en una nube se escribe con LiDAR > Asignar proyección.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ready = known(&ask.code);
                    if ui
                        .add_enabled(ready.is_some(), egui::Button::new("Usar este sistema"))
                        .clicked()
                    {
                        accept = ready;
                    }
                    if ui.button("Seguir sin sistema").clicked() {
                        skip = true;
                    }
                });
            });
        match accept {
            Some(code) => self.accept_crs(&ask.paths, code),
            None if skip || !open => self.crs_skipped.extend(ask.paths),
            None => self.crs_ask = Some(ask),
        }
    }
}
