//! The point-cloud tools beyond the forestry chain: thinning, duplicates, comparison of two
//! clouds, alignment, swath separation, colouring from an image and the extra classifiers.
//! Each loaded cloud is a sheet and gets its own result, like the other tools.

use std::collections::HashMap;

use super::*;
use rustlidar_core::cloudtools::{self, DupFields};
use rustlidar_core::thin::{self, ThinMethod, ThinParams};
use rustlidar_core::tls::{self, TlsJob, TlsParams};

pub struct State {
    pub thin_method: usize,
    pub thin_cell: f64,
    pub thin_vcell: f64,
    pub thin_pct: f64,
    pub thin_n: f64,
    pub thin_fraction: f64,
    pub thin_density: f64,
    pub thin_seed: f64,
    pub tls: TlsParams,
    pub tls_clouds: bool,
    pub dup: DupFields,
    pub dup_all: bool,
    pub cmp_ref: String,
    pub cmp_dist: f64,
    pub cmp_neigh: f64,
    pub fit_ref: String,
    pub fit_thin: f64,
    pub fit_xy: f64,
    pub fit_z: f64,
    pub fit_iter: f64,
    pub fit_stop: f64,
    pub fit_shift: bool,
    pub sw_cell: f64,
    pub sw_returns: usize,
    pub sw_by_cloud: bool,
    pub dsm_cell: f64,
    pub dsm_method: usize,
    pub dsm_fill: bool,
    pub dsm_smooth: f64,
    pub dsm_smoothing: Smoothing,
}

impl Default for State {
    fn default() -> Self {
        Self {
            thin_method: 0,
            thin_cell: 1.0,
            thin_vcell: 1.0,
            thin_pct: 50.0,
            thin_n: 2.0,
            thin_fraction: 0.5,
            thin_density: 1.0,
            thin_seed: 1.0,
            tls: TlsParams::default(),
            tls_clouds: false,
            dup: DupFields::default(),
            dup_all: false,
            cmp_ref: String::new(),
            cmp_dist: 0.5,
            cmp_neigh: 3.0,
            fit_ref: String::new(),
            fit_thin: 1.0,
            fit_xy: 5.0,
            fit_z: 2.0,
            fit_iter: 30.0,
            fit_stop: 0.01,
            fit_shift: false,
            sw_cell: 1.0,
            sw_returns: 0,
            sw_by_cloud: false,
            dsm_cell: 1.0,
            dsm_method: 0,
            dsm_fill: true,
            dsm_smooth: 0.0,
            dsm_smoothing: Smoothing::Mean,
        }
    }
}

const THIN_METHODS: [&str; 9] = [
    "Densidad objetivo (puntos/m², al azar)",
    "Un punto por celda: el más cercano al centro",
    "Un punto por celda: el más bajo",
    "Un punto por celda: el más alto",
    "Un punto por celda: percentil de la elevación (50 = mediana)",
    "Celdas 3D (conserva paredes y pendientes)",
    "Conservar un punto de cada n",
    "Eliminar un punto de cada n",
    "Muestreo al azar (fracción)",
];

impl Tools {
    fn lx_ext(first: &Path) -> String {
        first
            .extension()
            .map_or("las".to_string(), |e| e.to_string_lossy().into_owned())
    }

    pub fn thin_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        ui.label(egui::RichText::new("Reduce la densidad de la nube sin cambiar su extensión. Sirve para llevar a la misma densidad nubes de coberturas distintas (por ejemplo la primera y la tercera del PNOA) antes de compararlas. Se escribe una copia nueva por hoja (…_delgada_…); la original no cambia.").color(t.weak).size(11.0));
        ui.label(format!(
            "{} hoja(s): cada una se adelgaza por separado",
            inputs.len()
        ));
        for p in inputs {
            if let Ok(r) = rustlidar_core::las::Reader::open(p) {
                let b = r.header.bounds;
                let area = (b[3] - b[0]).max(0.0) * (b[4] - b[1]).max(0.0);
                if area > 0.0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "• {}: {} puntos, {:.2} puntos/m² (sobre el rectángulo envolvente)",
                            paths::stem(p),
                            r.header.count,
                            r.header.count as f64 / area
                        ))
                        .size(11.0),
                    );
                }
            }
        }
        let s = &mut self.lx;
        egui::ComboBox::from_id_salt("thin_method")
            .width(380.0)
            .selected_text(THIN_METHODS[s.thin_method])
            .show_ui(ui, |ui| {
                for (k, n) in THIN_METHODS.iter().enumerate() {
                    ui.selectable_value(&mut s.thin_method, k, *n);
                }
            });
        match s.thin_method {
            0 => {
                num(
                    ui,
                    "Puntos por m² que quedan",
                    &mut s.thin_density,
                    0.01..=10000.0,
                    0.1,
                );
                num(ui, "Semilla", &mut s.thin_seed, 0.0..=1e9, 1.0);
                ui.label(
                    egui::RichText::new("Una nube que ya es menos densa se deja como está.")
                        .color(t.weak)
                        .size(11.0),
                );
            }
            1..=3 => num(ui, "Celda (m)", &mut s.thin_cell, 0.05..=1000.0, 0.1),
            4 => {
                num(ui, "Celda (m)", &mut s.thin_cell, 0.05..=1000.0, 0.1);
                num(ui, "Percentil", &mut s.thin_pct, 0.0..=100.0, 1.0);
            }
            5 => {
                num(
                    ui,
                    "Celda horizontal (m)",
                    &mut s.thin_cell,
                    0.05..=1000.0,
                    0.1,
                );
                num(
                    ui,
                    "Celda vertical (m)",
                    &mut s.thin_vcell,
                    0.05..=1000.0,
                    0.1,
                );
            }
            6 | 7 => num(ui, "n", &mut s.thin_n, 2.0..=1e6, 1.0),
            _ => {
                num(
                    ui,
                    "Fracción que se conserva (0–1)",
                    &mut s.thin_fraction,
                    0.001..=1.0,
                    0.01,
                );
                num(ui, "Semilla", &mut s.thin_seed, 0.0..=1e9, 1.0);
            }
        }
    }

    pub fn tls_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let weak = |s: &str| egui::RichText::new(s.to_string()).color(t.weak).size(11.0);
        ui.label(weak("Mide el diámetro del tronco a lo largo de todo el fuste en nubes de escáner terrestre, móvil o fotogramétrico, y cubica cada árbol con la fórmula de Smalian: V = (A₁ + A₂)/2 · L por cada troza, desde la altura de la base hasta donde el diámetro baja al diámetro de punta delgada. Cada nube es una hoja y da su propio resultado."));
        let s = &mut self.lx.tls;
        ui.label(egui::RichText::new("Cubicación").strong());
        let (mut base_cm, mut top_cm) = (s.base_height * 100.0, s.top_diam * 100.0);
        num(
            ui,
            "Altura de la base sobre el suelo (cm)",
            &mut base_cm,
            0.0..=300.0,
            1.0,
        );
        num(
            ui,
            "Diámetro en punta delgada (cm)",
            &mut top_cm,
            1.0..=100.0,
            0.5,
        );
        num(
            ui,
            "Longitud de la troza (m): distancia entre diámetros",
            &mut s.log_len,
            0.1..=10.0,
            0.05,
        );
        (s.base_height, s.top_diam) = (base_cm / 100.0, top_cm / 100.0);
        ui.separator();
        ui.checkbox(
            &mut s.normalized,
            "La nube ya tiene alturas sobre el suelo (no hay que calcular el terreno)",
        );
        egui::CollapsingHeader::new("Búsqueda de troncos").show(ui, |ui| {
            num(
                ui,
                "Franja: desde (m sobre el suelo)",
                &mut s.stripe.0,
                0.0..=10.0,
                0.1,
            );
            num(ui, "hasta (m)", &mut s.stripe.1, 0.5..=20.0, 0.1);
            let mut v = s.voxel * 100.0;
            num(
                ui,
                "Vóxel (cm): la nube se reduce a un punto por vóxel",
                &mut v,
                0.5..=10.0,
                0.1,
            );
            s.voxel = v / 100.0;
            num(
                ui,
                "Diámetro máximo esperado (m)",
                &mut s.max_diam,
                0.2..=3.0,
                0.05,
            );
            let mut peel = s.peel as f64;
            num(ui, "Pasadas de poda de ramas", &mut peel, 1.0..=5.0, 1.0);
            s.peel = peel as usize;
            num(
                ui,
                "Verticalidad mínima (0–1)",
                &mut s.vert_min,
                0.3..=0.99,
                0.01,
            );
            let mut np = s.min_stem_points as f64;
            num(
                ui,
                "Vóxeles mínimos de un tronco en la franja",
                &mut np,
                20.0..=100000.0,
                10.0,
            );
            s.min_stem_points = np as usize;
        });
        egui::CollapsingHeader::new("Secciones y suelo").show(ui, |ui| {
            let mut sl = s.slice * 100.0;
            num(
                ui,
                "Semigrosor de la sección (cm)",
                &mut sl,
                1.0..=20.0,
                0.5,
            );
            s.slice = sl / 100.0;
            let mut a = [s.min_section_points as f64, s.min_sectors as f64];
            num(
                ui,
                "Puntos mínimos por sección",
                &mut a[0],
                8.0..=2000.0,
                1.0,
            );
            num(ui, "Sectores ocupados (de 16)", &mut a[1], 3.0..=16.0, 1.0);
            (s.min_section_points, s.min_sectors) = (a[0] as usize, a[1] as usize);
            num(
                ui,
                "Malla del terreno simulado (m)",
                &mut s.cloth,
                0.2..=5.0,
                0.05,
            );
            num(
                ui,
                "Altura máxima medida (m)",
                &mut s.max_height,
                5.0..=80.0,
                1.0,
            );
        });
        ui.separator();
        ui.checkbox(
            &mut self.lx.tls_clouds,
            "Guardar también la nube de puntos de cada árbol",
        );
        if self.lx.tls_clouds {
            num(
                ui,
                "Distancia máxima al eje (m)",
                &mut self.lx.tls.assign_dist,
                1.0..=50.0,
                0.5,
            );
        }
        self.formats_row(ui);
        ui.label(weak("Resultado: puntos en la base de cada tronco con DAP, alturas, volumen, nº de trozas, inclinación y calidad (.shp y/o .csv) y <nombre>_trozas.csv con el detalle de cada troza."));
    }

    fn reference_row(ui: &mut egui::Ui, text: &mut String) {
        ui.horizontal(|ui| {
            ui.label("Nube de referencia");
            ui.add(egui::TextEdit::singleline(text).desired_width(330.0));
            if ui.button("…").clicked()
                && let Some(p) = pick_file("Nube", &["las", "laz"])
            {
                *text = p.display().to_string();
            }
        });
    }

    pub fn dups_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Quita los puntos que están en el mismo sitio (x, y, z iguales). Marca los atributos que también deben coincidir para que dos puntos cuenten como repetidos. Se escribe una copia nueva (…_sin_duplicados).").color(t.weak).size(11.0));
        ui.checkbox(&mut s.dup.class, "Misma clase");
        ui.checkbox(&mut s.dup.gps_time, "Mismo tiempo GPS");
        ui.checkbox(&mut s.dup.intensity, "Misma intensidad");
        ui.checkbox(&mut s.dup.return_number, "Mismo número de retorno");
        ui.checkbox(&mut s.dup.return_count, "Mismo número de retornos");
        ui.checkbox(&mut s.dup.source_id, "Mismo ID de fuente (pasada)");
        ui.checkbox(
            &mut s.dup_all,
            "Quitar todas las copias (si no, se deja un punto de cada grupo)",
        );
    }

    pub fn compare_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Detección de cambios: escribe los puntos de la nube cargada que están a más de la distancia indicada de todos los puntos de la referencia (por ejemplo, la primera y la tercera cobertura del PNOA adelgazadas a la misma densidad). Da también la distancia media y la compleción.").color(t.weak).size(11.0));
        Self::reference_row(ui, &mut s.cmp_ref);
        num(
            ui,
            "Distancia mínima de cambio (m)",
            &mut s.cmp_dist,
            0.02..=100.0,
            0.05,
        );
        num(
            ui,
            "Vecinos cambiados que necesita un punto",
            &mut s.cmp_neigh,
            0.0..=1000.0,
            1.0,
        );
    }

    pub fn fit_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Mueve la nube cargada para que encaje con la de referencia (punto más cercano iterativo, ICP) y escribe una copia (…_ajustada). Usa los últimos retornos de ambas.").color(t.weak).size(11.0));
        Self::reference_row(ui, &mut s.fit_ref);
        num(
            ui,
            "Adelgazar para calcular (m)",
            &mut s.fit_thin,
            0.1..=50.0,
            0.1,
        );
        num(
            ui,
            "Distancia horizontal máxima (m)",
            &mut s.fit_xy,
            0.1..=500.0,
            0.5,
        );
        num(
            ui,
            "Distancia vertical máxima (m)",
            &mut s.fit_z,
            0.1..=500.0,
            0.5,
        );
        num(ui, "Iteraciones máximas", &mut s.fit_iter, 1.0..=200.0, 1.0);
        num(
            ui,
            "Parar con distancia media menor de (m)",
            &mut s.fit_stop,
            0.001..=5.0,
            0.005,
        );
        ui.checkbox(&mut s.fit_shift, "Solo desplazar (sin girar)");
    }

    pub fn swath_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Diferencia de altura entre pasadas de vuelo: por celda, la mayor distancia entre las elevaciones medias de las pasadas (ID de fuente) que la cubren. Se guarda un ráster (…_pasadas) y se da el porcentaje de celdas que pasa de 8 y 16 cm.").color(t.weak).size(11.0));
        num(ui, "Celda (m)", &mut s.sw_cell, 0.1..=100.0, 0.1);
        egui::ComboBox::from_label("Retornos")
            .selected_text(["Últimos y únicos", "Todos", "Solo únicos"][s.sw_returns])
            .show_ui(ui, |ui| {
                for (k, n) in ["Últimos y únicos", "Todos", "Solo únicos"]
                    .iter()
                    .enumerate()
                {
                    ui.selectable_value(&mut s.sw_returns, k, *n);
                }
            });
        ui.checkbox(
            &mut s.sw_by_cloud,
            "Cada nube cargada es una pasada (en vez del ID de fuente)",
        );
    }

    pub fn dsm_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        let name = ["tin", "max", "idw"][self.lx.dsm_method];
        let tag = format!("mds_{}m_{name}", paths::num(self.lx.dsm_cell));
        self.sheet_note(ui, t, inputs, &tag, "tif");
        ui.label(egui::RichText::new("Superficie de lo más alto que ve el sensor (copas, tejados): el punto más alto de cada celda. Cada hoja da su ráster; con el MDT se obtiene la altura de copas.").color(t.weak).size(11.0));
        num(ui, "Celda", &mut self.lx.dsm_cell, 0.1..=100.0, 0.1);
        egui::ComboBox::from_label("Método")
            .selected_text(
                [
                    "Triangulación (TIN)",
                    "Máximo por celda",
                    "Ponderado por distancia (IDW)",
                ][self.lx.dsm_method],
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.lx.dsm_method,
                    0,
                    "Triangulación (TIN) de los puntos más altos: superficie continua",
                );
                ui.selectable_value(
                    &mut self.lx.dsm_method,
                    1,
                    "Máximo por celda: el más rápido, en una sola pasada",
                );
                ui.selectable_value(
                    &mut self.lx.dsm_method,
                    2,
                    "Ponderado por distancia (IDW) de los puntos más altos",
                );
            });
        if self.lx.dsm_method == 1 {
            ui.checkbox(&mut self.lx.dsm_fill, "Rellenar las celdas sin puntos");
        }
        Self::smoothing_row(ui, &mut self.lx.dsm_smooth, &mut self.lx.dsm_smoothing);
    }

    fn thin_params(&self) -> ThinParams {
        let s = &self.lx;
        let seed = s.thin_seed as u64;
        ThinParams {
            method: match s.thin_method {
                0 => ThinMethod::Density {
                    per_m2: s.thin_density,
                    seed,
                },
                1 => ThinMethod::Nearest,
                2 => ThinMethod::Lowest,
                3 => ThinMethod::Highest,
                4 => ThinMethod::Percentile(s.thin_pct),
                5 => ThinMethod::Voxel {
                    vertical: s.thin_vcell,
                },
                6 => ThinMethod::EveryNth(s.thin_n as u64),
                7 => ThinMethod::RemoveNth(s.thin_n as u64),
                _ => ThinMethod::Random {
                    fraction: s.thin_fraction,
                    seed,
                },
            },
            cell: s.thin_cell,
        }
    }

    pub fn build_lidar(
        &mut self,
        tool: ToolId,
        inputs: &[PathBuf],
        _log: &mut Vec<String>,
    ) -> Option<Launch> {
        let first = inputs.first()?.clone();
        let label = tool.title().to_string();
        let ext = Self::lx_ext(&first);
        let names = |tag: &str| -> HashMap<PathBuf, PathBuf> {
            inputs
                .iter()
                .map(|p| (p.clone(), paths::unique(&paths::product(p, tag, &ext))))
                .collect()
        };
        let names_ext = |tag: &str, e: &str| -> HashMap<PathBuf, PathBuf> {
            inputs
                .iter()
                .map(|p| (p.clone(), paths::unique(&paths::product(p, tag, e))))
                .collect()
        };
        let inputs_v = inputs.to_vec();
        let (task, outputs): (Task, Vec<PathBuf>) = match tool {
            ToolId::Thin => {
                let params = self.thin_params();
                let outs = names(&format!("delgada_{}", params.tag()));
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for p in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let out = outs.get(p).cloned().unwrap_or_default();
                            let (r, k) = thin::thin_cloud(
                                std::slice::from_ref(p),
                                &out,
                                &params,
                                &Filter::default(),
                            )?;
                            lines.push(thin::report(r, k, &out));
                        }
                        Ok(lines.join("\n"))
                    }),
                    outputs,
                )
            }
            ToolId::Dups => {
                let (f, all) = (self.lx.dup, self.lx.dup_all);
                let outs = names("sin_duplicados");
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for p in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let out = outs.get(p).cloned().unwrap_or_default();
                            let (read, gone) = cloudtools::remove_duplicates(p, &out, f, all)?;
                            lines.push(format!(
                                "{read} puntos leídos, {gone} duplicados quitados: {}",
                                out.display()
                            ));
                        }
                        Ok(lines.join("\n"))
                    }),
                    outputs,
                )
            }
            ToolId::Compare | ToolId::Fit => {
                let reference = PathBuf::from(if tool == ToolId::Compare {
                    self.lx.cmp_ref.trim()
                } else {
                    self.lx.fit_ref.trim()
                });
                if !reference.is_file() {
                    _log.push("Elige la nube de referencia.".into());
                    return None;
                }
                let s = &self.lx;
                let (cp, fp) = (
                    cloudtools::CompareParams {
                        min_dist: s.cmp_dist,
                        min_neighbors: s.cmp_neigh as usize,
                    },
                    cloudtools::FitParams {
                        thin: s.fit_thin,
                        max_xy: s.fit_xy,
                        max_z: s.fit_z,
                        max_iter: s.fit_iter as usize,
                        stop_dist: s.fit_stop,
                        shift_only: s.fit_shift,
                    },
                );
                let is_cmp = tool == ToolId::Compare;
                let outs = names(if is_cmp { "cambios" } else { "ajustada" });
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for p in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let out = outs.get(p).cloned().unwrap_or_default();
                            let refs = std::slice::from_ref(&reference);
                            lines.push(if is_cmp {
                                cloudtools::compare_text(
                                    &cloudtools::compare_clouds(refs, p, &out, &cp)?,
                                    &out,
                                )
                            } else {
                                cloudtools::fit_text(
                                    &cloudtools::fit_cloud(refs, p, &out, &fp)?,
                                    &out,
                                )
                            });
                        }
                        Ok(lines.join("\n"))
                    }),
                    outputs,
                )
            }
            ToolId::Swath => {
                let s = &self.lx;
                let (cell, by_cloud) = (s.sw_cell, s.sw_by_cloud);
                let returns = [
                    cloudtools::SwathReturns::Last,
                    cloudtools::SwathReturns::All,
                    cloudtools::SwathReturns::Single,
                ][s.sw_returns];
                let out = paths::unique(&paths::product(&first, "pasadas", "tif"));
                let outputs = vec![out.clone()];
                let show = vec![out.clone()];
                let task: Task = Box::new(move || {
                    let rep =
                        cloudtools::swath_separation(&inputs_v, &out, cell, returns, by_cloud)?;
                    Ok(cloudtools::swath_text(&rep, &out))
                });
                return Some(Launch {
                    label,
                    task,
                    show,
                    outputs,
                });
            }
            ToolId::Dsm => {
                let s = &self.lx;
                let method = [
                    jobs::DsmMethod::Tin,
                    jobs::DsmMethod::Max,
                    jobs::DsmMethod::Idw,
                ][s.dsm_method];
                let name = ["tin", "max", "idw"][s.dsm_method];
                let sm = if s.dsm_smooth > 0.0 {
                    format!("_s{}", paths::num(s.dsm_smooth))
                } else {
                    String::new()
                };
                let outs = names_ext(
                    &format!("mds_{}m_{name}{sm}", paths::num(s.dsm_cell)),
                    "tif",
                );
                let (cell, fill, smooth, smoothing) =
                    (s.dsm_cell, s.dsm_fill, s.dsm_smooth, s.dsm_smoothing);
                let tiling = self.tiling();
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                let show = outputs.clone();
                let task: Task = Box::new(move || {
                    let mut lines = Vec::new();
                    for p in &inputs_v {
                        rustlidar_core::check_cancel()?;
                        lines.push(jobs::dsm(&jobs::DsmJob {
                            input: p.clone(),
                            output: outs.get(p).cloned().unwrap_or_default(),
                            cell,
                            method,
                            fill,
                            tiling: tiling.clone(),
                            smooth,
                            smoothing,
                        })?);
                    }
                    Ok(lines.join("\n"))
                });
                return Some(Launch {
                    label,
                    task,
                    show,
                    outputs,
                });
            }
            ToolId::Tls => {
                let formats = self.formats();
                let params = self.lx.tls.clone();
                let clouds = self.lx.tls_clouds;
                let outs = names_ext("cubicado", "shp");
                let mut outputs: Vec<PathBuf> = Vec::new();
                let mut show = Vec::new();
                for o in outs.values() {
                    outputs.extend(super::related(o));
                    outputs.push(o.with_file_name(format!("{}_trozas.csv", paths::stem(o))));
                    if clouds {
                        outputs.push(o.with_file_name(format!("{}_arboles", paths::stem(o))));
                    }
                    if formats.vector {
                        show.push(o.clone());
                    }
                }
                let task: Task = Box::new(move || {
                    let mut lines = Vec::new();
                    for p in &inputs_v {
                        rustlidar_core::check_cancel()?;
                        let job = TlsJob {
                            input: p.clone(),
                            output: outs.get(p).cloned().unwrap_or_default(),
                            params: params.clone(),
                            formats,
                            tree_clouds: clouds,
                        };
                        lines.push(tls::run(&job)?);
                    }
                    Ok(lines.join("\n"))
                });
                return Some(Launch {
                    label,
                    task,
                    show,
                    outputs,
                });
            }
            _ => return None,
        };
        Some(Launch {
            label,
            task,
            show: vec![],
            outputs,
        })
    }
}
