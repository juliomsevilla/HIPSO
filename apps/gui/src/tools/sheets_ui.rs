//! The tools that work sheet by sheet (classify, DTM, CHM, metrics), the coordinate-system tool
//! and the plot tool. Each loaded cloud is a sheet: it is processed on its own and gets its own
//! result; a mosaic of the results is made only when asked for.

use std::collections::HashMap;

use super::*;
use rustlidar_core::las::Reader;
use rustlidar_core::ops::{CrsSource, assign_projection};
use rustlidar_core::plots::{PlotShape, Sampling};
use rustlidar_core::sheets;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlotSource {
    File,
    Generate,
    Aois,
}

pub struct State {
    pub dtm_smooth: f64,
    pub dtm_smoothing: Smoothing,
    pub mosaic_dtm: bool,
    pub chm_smooth: f64,
    pub chm_smoothing: Smoothing,
    pub mosaic_chm: bool,
    /// 0 TIN, 1 highest per cell, 2 IDW, 3 pit-free (the last only with a DTM).
    pub surf_method: usize,
    /// Subtract the DTM (a CHM) or keep absolute elevations (a DSM).
    pub surf_dtm: bool,
    pub crs_from_file: bool,
    pub crs_file: String,
    pub crs_epsg: String,
    pub crs_replace: bool,
    // plots
    pub source: PlotSource,
    pub square: bool,
    pub radius: f64,
    pub side: f64,
    pub name_field: String,
    pub radius_field: String,
    pub side_x_field: String,
    pub side_y_field: String,
    pub where_: String,
    pub file: String,
    pub systematic: bool,
    pub count: usize,
    pub min_dist: f64,
    pub seed: u64,
    pub dx: f64,
    pub dy: f64,
    pub limit: String,
    /// 0: no clouds, 1: one per plot, 2: one with all the plots.
    pub clouds: u8,
    pub vector: bool,
    pub exclusive: bool,
    /// The user pressed Stop in a tool window.
    pub stop: bool,
    pub sel_note: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            dtm_smooth: 0.0,
            dtm_smoothing: Smoothing::Mean,
            mosaic_dtm: false,
            chm_smooth: 0.0,
            chm_smoothing: Smoothing::Mean,
            mosaic_chm: false,
            surf_method: 0,
            surf_dtm: true,
            crs_from_file: true,
            crs_file: String::new(),
            crs_epsg: String::new(),
            crs_replace: false,
            source: PlotSource::File,
            square: false,
            radius: 11.28,
            side: 20.0,
            name_field: String::new(),
            radius_field: String::new(),
            side_x_field: String::new(),
            side_y_field: String::new(),
            where_: String::new(),
            file: String::new(),
            systematic: false,
            count: 10,
            min_dist: 0.0,
            seed: 1,
            dx: 100.0,
            dy: 100.0,
            limit: String::new(),
            clouds: 0,
            vector: true,
            exclusive: false,
            stop: false,
            sel_note: None,
        }
    }
}

/// The files that belong to a result: a shapefile's companions, a metrics table's field list.
pub fn related(p: &Path) -> Vec<PathBuf> {
    let mut v = vec![p.to_path_buf()];
    let stem = paths::stem(p);
    for ext in ["dbf", "shx", "prj", "cpg", "csv"] {
        v.push(paths::swap_ext(p, ext));
    }
    v.push(p.with_file_name(format!("{stem}_campos.txt")));
    v.push(paths::swap_ext(p, "tmp.csv"));
    for suffix in ["intensity", "strata"] {
        for ext in ["shp", "dbf", "shx", "prj", "csv"] {
            v.push(p.with_file_name(format!("{stem}_{suffix}.{ext}")));
        }
        v.push(p.with_file_name(format!("{stem}_{suffix}_campos.txt")));
    }
    v
}

fn lines_of(inputs: &[PathBuf]) -> String {
    format!(
        "{} hoja(s) cargada(s): cada una se procesa por separado y tiene su propio resultado",
        inputs.len()
    )
}

impl Tools {
    pub(super) fn sheet_note(
        &self,
        ui: &mut egui::Ui,
        t: &Tokens,
        inputs: &[PathBuf],
        tag: &str,
        ext: &str,
    ) {
        ui.label(
            egui::RichText::new(lines_of(inputs))
                .color(t.weak)
                .size(11.0),
        );
        if let Some(f) = inputs.first() {
            ui.label(
                egui::RichText::new(format!("p. ej. {}", paths::product(f, tag, ext).display()))
                    .color(t.weak)
                    .size(11.0),
            );
        }
    }

    pub(super) fn smoothing_row(ui: &mut egui::Ui, size: &mut f64, kind: &mut Smoothing) {
        num(ui, "Suavizado (m, 0 = sin suavizar)", size, 0.0..=50.0, 0.1);
        if *size > 0.0 {
            ui.horizontal(|ui| {
                ui.radio_value(kind, Smoothing::Mean, "Media");
                ui.radio_value(kind, Smoothing::Median, "Mediana");
                ui.radio_value(kind, Smoothing::Gaussian, "Gauss");
            });
        }
    }

    /// Only the ground method: a DTM is made from ground points, so no noise or building steps.
    fn ground_only(&mut self, ui: &mut egui::Ui) {
        let names = [
            (
                GroundChoice::Classes,
                "Clase 2 que trae la nube (por defecto)",
            ),
            (GroundChoice::Fusion, "Kraus y Pfeifer (iterativo)"),
            (GroundChoice::Mcc, "MCC (curvatura multiescala)"),
            (GroundChoice::Csf, "CSF (simulación de tela)"),
            (GroundChoice::Pmf, "PMF (morfológico progresivo)"),
            (
                GroundChoice::Axelsson,
                "Axelsson (TIN progresivo, ángulo y distancia)",
            ),
        ];
        egui::ComboBox::from_label("Suelo")
            .selected_text(
                names
                    .iter()
                    .find(|n| n.0 == self.ground)
                    .map_or("", |n| n.1),
            )
            .show_ui(ui, |ui| {
                for (c, n) in names {
                    ui.selectable_value(&mut self.ground, c, n);
                }
            });
        ui.label(egui::RichText::new("Otro método vuelve a buscar el suelo solo para este MDT: sirve para probar varios modelos con distinta clasificación. No modifica la nube.").size(11.0));
        match self.ground {
            GroundChoice::Fusion => {
                num(
                    ui,
                    "Celda del filtro",
                    &mut self.gp.filter_cell,
                    0.5..=50.0,
                    0.1,
                );
                let mut it = f64::from(self.gp.iterations);
                num(ui, "Iteraciones", &mut it, 1.0..=50.0, 1.0);
                self.gp.iterations = it as u32;
            }
            GroundChoice::Mcc => {
                num(ui, "Espaciado (s)", &mut self.mcc.spacing, 0.1..=20.0, 0.1);
                num(
                    ui,
                    "Umbral de curvatura (t)",
                    &mut self.mcc.threshold,
                    0.0..=5.0,
                    0.01,
                );
            }
            GroundChoice::Csf => {
                num(
                    ui,
                    "Resolución de la tela",
                    &mut self.csf.resolution,
                    0.1..=10.0,
                    0.1,
                );
                num(ui, "Umbral", &mut self.csf.threshold, 0.05..=5.0, 0.05);
            }
            GroundChoice::Pmf => {
                num(
                    ui,
                    "Ventana máxima",
                    &mut self.pmf.max_window,
                    3.0..=200.0,
                    1.0,
                );
                num(ui, "Pendiente", &mut self.pmf.slope, 0.0..=10.0, 0.1);
            }
            GroundChoice::Axelsson => axelsson_fields(ui, &mut self.ax),
            GroundChoice::Classes => {}
        }
    }

    pub fn classify_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        let tag = format!("clasif{}", self.ground_tag());
        let ext = inputs
            .first()
            .and_then(|p| p.extension())
            .map_or("las".to_string(), |e| e.to_string_lossy().into_owned());
        self.sheet_note(ui, t, inputs, &tag, &ext);
        ui.label(egui::RichText::new("Las hojas no se unen: se clasifica una a una y cada una se escribe en su propio archivo.").color(t.weak).size(11.0));
        self.steps_form(ui);
    }

    pub fn dtm_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        let tin = self.surface_tag();
        let tag = format!(
            "mdt_{}m{tin}{}",
            paths::num(self.dtm_cell),
            self.ground_tag()
        );
        self.sheet_note(ui, t, inputs, &tag, "tif");
        num(ui, "Celda", &mut self.dtm_cell, 0.1..=100.0, 0.1);
        egui::ComboBox::from_label("Método")
            .selected_text(match self.dtm_surface {
                Surface::Tin => "Triangulación (TIN)",
                Surface::Mean => "Media por celda",
                Surface::Idw => "Ponderado por distancia (IDW)",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.dtm_surface,
                    Surface::Tin,
                    "Triangulación (TIN): superficie continua, el mejor equilibrio",
                );
                ui.selectable_value(
                    &mut self.dtm_surface,
                    Surface::Idw,
                    "Ponderado por distancia (IDW): suave y rápido",
                );
                ui.selectable_value(
                    &mut self.dtm_surface,
                    Surface::Mean,
                    "Media por celda: la más rápida, en una sola pasada",
                );
            });
        self.ground_only(ui);
        Self::smoothing_row(ui, &mut self.s2.dtm_smooth, &mut self.s2.dtm_smoothing);
        ui.checkbox(
            &mut self.s2.mosaic_dtm,
            "Hacer además un mosaico con todas las hojas",
        );
    }

    /// Whether the surface tool subtracts the DTM: asked for, and every sheet has one.
    pub fn surface_with_dtm(&self, inputs: &[PathBuf]) -> bool {
        self.s2.surf_dtm && inputs.iter().all(|p| paths::find_dtm(p).is_some())
    }

    fn surface_name(&self, with_dtm: bool) -> String {
        let name = ["tin", "max", "idw", "pitfree"][self.s2.surf_method.min(3)];
        let sm = if self.s2.chm_smooth > 0.0 {
            format!("_s{}", paths::num(self.s2.chm_smooth))
        } else {
            String::new()
        };
        let cell = paths::num(self.chm_cell);
        if with_dtm {
            format!("chm_{cell}m_{name}{sm}")
        } else {
            format!("mds_{cell}m_{name}{sm}")
        }
    }

    pub fn chm_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        let missing: Vec<String> = inputs
            .iter()
            .filter(|p| paths::find_dtm(p).is_none())
            .map(|p| paths::stem(p))
            .collect();
        ui.label(egui::RichText::new("Superficie de lo más alto que ve el sensor (copas, tejados). Con el MDT se resta y sale la altura sobre el suelo (CHM); sin él, la elevación absoluta (MDS).").color(t.weak).size(11.0));
        ui.checkbox(
            &mut self.s2.surf_dtm,
            "Restar el MDT (altura sobre el suelo, CHM)",
        );
        if self.s2.surf_dtm {
            if missing.is_empty() {
                ui.label(
                    egui::RichText::new("Cada hoja usa el último MDT hecho con ella.")
                        .color(t.weak)
                        .size(11.0),
                );
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 160, 60),
                    format!(
                        "Sin MDT: {}. Se hará el MDS (elevación absoluta); genera antes el MDT para obtener el CHM.",
                        missing.join(", ")
                    ),
                );
            }
        }
        let with_dtm = self.surface_with_dtm(inputs);
        if !with_dtm && self.s2.surf_method == 3 {
            self.s2.surf_method = 0;
        }
        let tag = self.surface_name(with_dtm);
        self.sheet_note(ui, t, inputs, &tag, "tif");
        num(ui, "Celda", &mut self.chm_cell, 0.1..=100.0, 0.1);
        let names = [
            "Triangulación (TIN) de los puntos más altos: superficie continua",
            "Máximo por celda: el más rápido, en una sola pasada",
            "Ponderado por distancia (IDW) de los puntos más altos",
            "Sin huecos (pit-free, Khosravipour et al. 2014)",
        ];
        egui::ComboBox::from_label("Método")
            .selected_text(names[self.s2.surf_method.min(3)])
            .show_ui(ui, |ui| {
                for (k, n) in names.iter().enumerate() {
                    if k == 3 && !with_dtm {
                        ui.add_enabled(false, egui::Button::selectable(false, *n))
                            .on_disabled_hover_text(
                                "El pit-free trabaja con alturas sobre el suelo: necesita el MDT.",
                            );
                    } else {
                        ui.selectable_value(&mut self.s2.surf_method, k, *n);
                    }
                }
            });
        match self.s2.surf_method {
            1 => {
                ui.checkbox(&mut self.fill, "Rellenar celdas vacías");
            }
            3 => num(ui, "Arista máxima", &mut self.max_edge, 0.1..=20.0, 0.1),
            _ => {}
        }
        Self::smoothing_row(ui, &mut self.s2.chm_smooth, &mut self.s2.chm_smoothing);
        ui.checkbox(
            &mut self.s2.mosaic_chm,
            "Hacer además un mosaico con todas las hojas",
        );
    }

    pub fn metrics_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        let tag = format!("metricas_{}m", paths::num(self.mp.cell));
        self.sheet_note(
            ui,
            t,
            inputs,
            &tag,
            if self.fmt_vector { "shp" } else { "csv" },
        );
        let missing: Vec<String> = inputs
            .iter()
            .filter(|p| paths::find_dtm(p).is_none())
            .map(|p| paths::stem(p))
            .collect();
        if !missing.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(230, 160, 60),
                format!(
                    "Sin MDT: {}. Genera antes el MDT de esas hojas.",
                    missing.join(", ")
                ),
            );
        }
        self.formats_row(ui);
        num(ui, "Celda", &mut self.mp.cell, 1.0..=1000.0, 0.5);
        num(
            ui,
            "Altura de corte (cobertura)",
            &mut self.mp.heightbreak,
            0.0..=50.0,
            0.1,
        );
        ui.checkbox(&mut self.intensity, "Métricas de intensidad (_intensity)");
        ui.checkbox(&mut self.strata, "Métricas por estratos (_strata)");
        let mut mb = (self.mp.memory >> 20) as f64;
        num(ui, "Memoria máxima (MB)", &mut mb, 16.0..=65536.0, 16.0);
        self.mp.memory = (mb as u64) << 20;
    }

    pub fn crs_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        ui.label(egui::RichText::new("Da a las nubes cargadas que no tienen sistema de coordenadas el de otra nube, o un código EPSG. Se escribe una copia nueva (…_crs); los puntos no cambian.").color(t.weak).size(11.0));
        for p in inputs {
            let has = Reader::open(p)
                .ok()
                .is_some_and(|r| r.header.epsg.is_some() || r.meta.wkt.is_some());
            ui.label(format!(
                "{} {}",
                if has {
                    "✔ con sistema:"
                } else {
                    "✖ sin sistema:"
                },
                paths::stem(p)
            ));
        }
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.s2.crs_from_file, true, "Copiar de otra nube");
            ui.radio_value(&mut self.s2.crs_from_file, false, "Código EPSG");
        });
        if self.s2.crs_from_file {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.s2.crs_file).desired_width(360.0));
                if ui.button("…").clicked()
                    && let Some(p) = pick_file("Nube", &["las", "laz"])
                {
                    self.s2.crs_file = p.display().to_string();
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.label("EPSG");
                ui.add(
                    egui::TextEdit::singleline(&mut self.s2.crs_epsg)
                        .hint_text("25830")
                        .desired_width(80.0),
                );
            });
        }
        ui.checkbox(
            &mut self.s2.crs_replace,
            "Reemplazar también el de las nubes que ya lo tienen",
        );
    }

    // ------------------------------------------------------------------------ plots

    pub fn plots_form(&mut self, ui: &mut egui::Ui, t: &Tokens, aois: &[Aoi]) {
        let weak = |s: &str| egui::RichText::new(s.to_string()).color(t.weak).size(11.0);
        ui.label(egui::RichText::new("1 · De dónde salen las parcelas").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.s2.source, PlotSource::File, "Archivo vectorial");
            ui.radio_value(
                &mut self.s2.source,
                PlotSource::Generate,
                "Generarlas (muestreo)",
            );
            ui.radio_value(
                &mut self.s2.source,
                PlotSource::Aois,
                format!("AOI dibujadas ({})", aois.len()),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Forma");
            ui.radio_value(&mut self.s2.square, false, "Circular");
            ui.radio_value(&mut self.s2.square, true, "Cuadrada / rectangular");
        });
        match self.s2.source {
            PlotSource::File => {
                ui.horizontal(|ui| {
                    ui.label("Archivo");
                    ui.add(egui::TextEdit::singleline(&mut self.s2.file).desired_width(330.0));
                    if ui.button("…").clicked()
                        && let Some(p) = pick_file("Parcelas", &["shp", "csv", "txt"])
                    {
                        self.s2.file = p.display().to_string();
                    }
                });
                ui.label(weak("Polígonos (se usan tal cual) o puntos con el centro de cada parcela (la forma y el tamaño se dan abajo)."));
                ui.horizontal(|ui| {
                    ui.label("Campo con el número de parcela");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.s2.name_field).desired_width(110.0),
                    );
                });
                if self.s2.square {
                    ui.horizontal(|ui| {
                        ui.label("Campo con el lado X (m)");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.s2.side_x_field)
                                .desired_width(90.0),
                        );
                        ui.label("lado Y (m)");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.s2.side_y_field)
                                .desired_width(90.0),
                        );
                    });
                    num(ui, "o lado fijo (m)", &mut self.s2.side, 0.5..=1000.0, 0.5);
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Campo con el radio (m)");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.s2.radius_field)
                                .desired_width(110.0),
                        );
                    });
                    num(
                        ui,
                        "o radio fijo (m)",
                        &mut self.s2.radius,
                        0.5..=500.0,
                        0.5,
                    );
                }
                ui.horizontal(|ui| {
                    ui.label("Solo las parcelas con CAMPO=VALOR");
                    ui.add(egui::TextEdit::singleline(&mut self.s2.where_).desired_width(110.0));
                });
            }
            PlotSource::Generate => {
                if self.s2.square {
                    num(ui, "Lado (m)", &mut self.s2.side, 0.5..=1000.0, 0.5);
                } else {
                    num(ui, "Radio (m)", &mut self.s2.radius, 0.5..=500.0, 0.5);
                }
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.s2.systematic, false, "Aleatorio");
                    ui.radio_value(&mut self.s2.systematic, true, "Sistemático (malla regular)");
                });
                if self.s2.systematic {
                    num(
                        ui,
                        "Separación en X (m)",
                        &mut self.s2.dx,
                        1.0..=100000.0,
                        1.0,
                    );
                    num(
                        ui,
                        "Separación en Y (m)",
                        &mut self.s2.dy,
                        1.0..=100000.0,
                        1.0,
                    );
                } else {
                    let mut c = self.s2.count as f64;
                    num(ui, "Número de parcelas", &mut c, 1.0..=100000.0, 1.0);
                    self.s2.count = c as usize;
                    num(
                        ui,
                        "Distancia mínima entre centros (m, 0 = el doble del tamaño)",
                        &mut self.s2.min_dist,
                        0.0..=100000.0,
                        1.0,
                    );
                    let mut s = self.s2.seed as f64;
                    num(
                        ui,
                        "Semilla (la misma semilla da las mismas parcelas)",
                        &mut s,
                        0.0..=1e9,
                        1.0,
                    );
                    self.s2.seed = s as u64;
                }
                ui.horizontal(|ui| {
                    ui.label("Limitar a los polígonos de");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.s2.limit)
                            .hint_text("rodal.shp (opcional)")
                            .desired_width(220.0),
                    );
                    if ui.button("…").clicked()
                        && let Some(p) = pick_file("Polígonos", &["shp"])
                    {
                        self.s2.limit = p.display().to_string();
                    }
                });
                ui.label(weak(
                    "La ubicación de las parcelas generadas se guarda como capa vectorial.",
                ));
            }
            PlotSource::Aois => {
                for a in aois {
                    ui.label(format!("• {} ({} vértices)", a.name, a.points.len()));
                }
                if aois.is_empty() {
                    ui.label("Dibuja polígonos con la herramienta AOI (pestaña Inicio).");
                }
            }
        }
        ui.separator();
        ui.label(egui::RichText::new("2 · Qué se obtiene").strong());
        ui.checkbox(
            &mut self.s2.vector,
            "Capa vectorial con las parcelas y sus métricas",
        );
        if self.s2.vector {
            self.formats_row(ui);
        }
        ui.horizontal(|ui| {
            ui.label("Nubes recortadas");
            ui.radio_value(&mut self.s2.clouds, 0, "ninguna");
            ui.radio_value(&mut self.s2.clouds, 1, "una por parcela");
            ui.radio_value(&mut self.s2.clouds, 2, "una con todas");
        });
        egui::CollapsingHeader::new("Filtros y opciones").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Clases");
                ui.add(
                    egui::TextEdit::singleline(&mut self.clip_class)
                        .hint_text("2,5  o  ~7,18")
                        .desired_width(90.0),
                );
                ui.label("Retornos");
                ui.add(
                    egui::TextEdit::singleline(&mut self.clip_returns)
                        .hint_text("A, F, L, 1,2")
                        .desired_width(90.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Z mín");
                ui.add(egui::TextEdit::singleline(&mut self.clip_zmin).desired_width(60.0));
                ui.label("Z máx");
                ui.add(egui::TextEdit::singleline(&mut self.clip_zmax).desired_width(60.0));
            });
            ui.checkbox(
                &mut self.clip_normalise,
                "Alturas sobre el suelo (normalizar con el MDT de la primera hoja)",
            );
            ui.checkbox(
                &mut self.s2.exclusive,
                "Parcelas solapadas: cada punto va solo a la primera",
            );
        });
    }

    // ------------------------------------------------------------------------ jobs

    pub fn build_sheets(
        &mut self,
        tool: ToolId,
        inputs: &[PathBuf],
        log: &mut Vec<String>,
    ) -> Option<Launch> {
        let first = inputs.first()?.clone();
        let label = tool.title().to_string();
        let ext = first
            .extension()
            .map_or("las".to_string(), |e| e.to_string_lossy().into_owned());
        let names = |tag: &str, ext: &str| -> HashMap<PathBuf, PathBuf> {
            inputs
                .iter()
                .map(|p| (p.clone(), paths::unique(&paths::product(p, tag, ext))))
                .collect()
        };
        let missing: Vec<String> = inputs
            .iter()
            .filter(|p| paths::find_dtm(p).is_none())
            .map(|p| paths::stem(p))
            .collect();
        let need_dtm = |log: &mut Vec<String>| -> bool {
            if missing.is_empty() {
                true
            } else {
                log.push(format!(
                    "Falta el MDT de: {}. Genéralo antes con la herramienta MDT.",
                    missing.join(", ")
                ));
                false
            }
        };
        let dtm_for = |p: &Path| {
            paths::find_dtm(p).ok_or_else(|| {
                rustlidar_core::Error::Param(format!("no hay MDT de {}", p.display()))
            })
        };
        let inputs_v = inputs.to_vec();
        let (task, show, outputs): (Task, Vec<PathBuf>, Vec<PathBuf>) = match tool {
            ToolId::Classify => {
                let tag = format!("clasif{}", self.ground_tag());
                let outs = names(&tag, &ext);
                let (steps, tiling) = (self.steps(), self.tiling());
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let out_for = |p: &Path| outs.get(p).cloned().unwrap_or_default();
                        sheets::classify_each(&inputs_v, &out_for, &steps, &tiling)
                    }),
                    vec![],
                    outputs,
                )
            }
            ToolId::Dtm => {
                let tin = self.surface_tag();
                let sm = if self.s2.dtm_smooth > 0.0 {
                    format!("_s{}", paths::num(self.s2.dtm_smooth))
                } else {
                    String::new()
                };
                let tag = format!(
                    "mdt_{}m{tin}{}{sm}",
                    paths::num(self.dtm_cell),
                    self.ground_tag()
                );
                let outs = names(&tag, "tif");
                let mosaic = (self.s2.mosaic_dtm && inputs.len() > 1).then(|| {
                    paths::unique(&paths::product(&first, &format!("mosaico_{tag}"), "tif"))
                });
                let mut steps = self.steps();
                // a DTM is made from ground points only
                steps.noise = None;
                steps.buildings = None;
                let base = jobs::DtmJob {
                    inputs: Vec::new(),
                    output: PathBuf::new(),
                    cell: self.dtm_cell,
                    surface: self.dtm_surface,
                    steps,
                    tiling: self.tiling(),
                    smooth: self.s2.dtm_smooth,
                    smoothing: self.s2.dtm_smoothing,
                };
                let mut outputs: Vec<PathBuf> = outs.values().cloned().collect();
                outputs.extend(mosaic.clone());
                let show = outputs.clone();
                (
                    Box::new(move || {
                        let out_for = |p: &Path| outs.get(p).cloned().unwrap_or_default();
                        sheets::dtm_each(&inputs_v, &base, &out_for, mosaic.as_deref()).map(|r| r.0)
                    }),
                    show,
                    outputs,
                )
            }
            ToolId::Chm => {
                let with_dtm = self.surface_with_dtm(inputs);
                let tag = self.surface_name(with_dtm);
                let outs = names(&tag, "tif");
                let mosaic = (self.s2.mosaic_chm && inputs.len() > 1).then(|| {
                    paths::unique(&paths::product(&first, &format!("mosaico_{tag}"), "tif"))
                });
                let tiling = Tiling {
                    buffer: 5.0,
                    ..self.tiling.clone()
                };
                let (smooth, smoothing) = (self.s2.chm_smooth, self.s2.chm_smoothing);
                let (cell, fill, kind) = (self.chm_cell, self.fill, self.s2.surf_method);
                let max_edge = self.max_edge;
                let mut outputs: Vec<PathBuf> = outs.values().cloned().collect();
                outputs.extend(mosaic.clone());
                let show = outputs.clone();
                let task: Task = if with_dtm {
                    let method = match kind {
                        0 => ChmMethod::Tin,
                        1 => ChmMethod::Max { fill },
                        2 => ChmMethod::Idw,
                        _ => ChmMethod::PitFree(PitFree {
                            max_edge,
                            ..Default::default()
                        }),
                    };
                    let base = jobs::ChmJob {
                        input: PathBuf::new(),
                        dtm: PathBuf::new(),
                        output: PathBuf::new(),
                        cell,
                        method,
                        tiling,
                        smooth,
                        smoothing,
                    };
                    Box::new(move || {
                        let out_for = |p: &Path| outs.get(p).cloned().unwrap_or_default();
                        sheets::chm_each(&inputs_v, &base, &out_for, &dtm_for, mosaic.as_deref())
                            .map(|r| r.0)
                    })
                } else {
                    let base = jobs::DsmJob {
                        input: PathBuf::new(),
                        output: PathBuf::new(),
                        cell,
                        method: [
                            jobs::DsmMethod::Tin,
                            jobs::DsmMethod::Max,
                            jobs::DsmMethod::Idw,
                        ][kind.min(2)],
                        fill,
                        tiling,
                        smooth,
                        smoothing,
                    };
                    Box::new(move || {
                        let out_for = |p: &Path| outs.get(p).cloned().unwrap_or_default();
                        sheets::dsm_each(&inputs_v, &base, &out_for, mosaic.as_deref()).map(|r| r.0)
                    })
                };
                (task, show, outputs)
            }
            ToolId::Metrics => {
                if !need_dtm(log) {
                    return None;
                }
                let tag = format!("metricas_{}m", paths::num(self.mp.cell));
                let formats = self.formats();
                let outs = names(&tag, "shp");
                let base = jobs::MetricsJob {
                    input: PathBuf::new(),
                    dtm: PathBuf::new(),
                    output: PathBuf::new(),
                    params: self.mp.clone(),
                    intensity: self.intensity,
                    strata: self.strata,
                    formats,
                };
                let mut outputs = Vec::new();
                let mut show = Vec::new();
                for o in outs.values() {
                    outputs.push(o.clone());
                    if formats.vector {
                        show.push(paths::swap_ext(o, "shp"));
                        for (on, suffix) in [(self.intensity, "intensity"), (self.strata, "strata")]
                        {
                            if on {
                                show.push(
                                    o.with_file_name(format!("{}_{suffix}.shp", paths::stem(o))),
                                );
                            }
                        }
                    }
                }
                (
                    Box::new(move || {
                        let out_for = |p: &Path| outs.get(p).cloned().unwrap_or_default();
                        sheets::metrics_each(&inputs_v, &base, &out_for, &dtm_for)
                    }),
                    show,
                    outputs,
                )
            }
            ToolId::Crs => {
                let source = if self.s2.crs_from_file {
                    if !Path::new(self.s2.crs_file.trim()).is_file() {
                        log.push(
                            "Elige la nube de la que copiar el sistema de coordenadas.".into(),
                        );
                        return None;
                    }
                    CrsSource::Cloud(PathBuf::from(self.s2.crs_file.trim()))
                } else {
                    match self.s2.crs_epsg.trim().parse::<u16>() {
                        Ok(c) => CrsSource::Epsg(c),
                        Err(_) => {
                            log.push(
                                "El código EPSG debe ser un número (por ejemplo 25830).".into(),
                            );
                            return None;
                        }
                    }
                };
                let outs = names("crs", &ext);
                let replace = self.s2.crs_replace;
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for p in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let has = Reader::open(p)
                                .ok()
                                .is_some_and(|r| r.header.epsg.is_some() || r.meta.wkt.is_some());
                            if has && !replace {
                                lines.push(format!(
                                    "{}: ya tiene sistema de coordenadas, se deja como está",
                                    p.display()
                                ));
                                continue;
                            }
                            let out = outs.get(p).cloned().unwrap_or_default();
                            lines.push(assign_projection(p, &source, &out, replace)?);
                        }
                        Ok(lines.join("\n"))
                    }),
                    vec![],
                    outputs,
                )
            }
            _ => return None,
        };
        Some(Launch {
            label,
            task,
            show,
            outputs,
        })
    }

    /// The plot job: gets or generates the plots, saves where they are, and cuts the clouds.
    pub fn build_clip(
        &mut self,
        inputs: &[PathBuf],
        aois: &[Aoi],
        log: &mut Vec<String>,
    ) -> Option<Launch> {
        use rustlidar_core::clip::{ClipJob, ClipMode, SampleMetrics};
        let first = inputs.first()?.clone();
        let ext = first
            .extension()
            .map_or("las".to_string(), |e| e.to_string_lossy().into_owned());
        let s = &self.s2;
        let mut show: Vec<PathBuf> = Vec::new();
        let mut outputs: Vec<PathBuf> = Vec::new();
        let regions = match s.source {
            PlotSource::File => {
                if s.file.trim().is_empty() {
                    log.push("Elige el archivo con las parcelas.".into());
                    return None;
                }
                let opts = vector::ReadOptions {
                    name_field: (!s.name_field.trim().is_empty())
                        .then(|| s.name_field.trim().to_string()),
                    radius: Some(s.radius),
                    radius_field: (!s.radius_field.trim().is_empty())
                        .then(|| s.radius_field.trim().to_string()),
                    square: s.square,
                    side: Some(s.side),
                    side_x_field: (!s.side_x_field.trim().is_empty())
                        .then(|| s.side_x_field.trim().to_string()),
                    side_y_field: (!s.side_y_field.trim().is_empty())
                        .then(|| s.side_y_field.trim().to_string()),
                };
                match vector::read_regions(Path::new(s.file.trim()), &opts) {
                    Ok(mut list) => {
                        if let Some((f, v)) = s.where_.split_once('=') {
                            list.retain(|r| {
                                r.attrs.iter().any(|(k, val)| {
                                    k.eq_ignore_ascii_case(f.trim())
                                        && val.eq_ignore_ascii_case(v.trim())
                                })
                            });
                        }
                        list
                    }
                    Err(e) => {
                        log.push(format!("Parcelas: {e}"));
                        return None;
                    }
                }
            }
            PlotSource::Aois => aois
                .iter()
                .filter(|a| a.points.len() >= 3)
                .map(|a| Region {
                    name: a.name.clone(),
                    shape: Shape2::Polygon(Polygon::new(vec![a.points.clone()])),
                    attrs: vec![],
                })
                .collect(),
            PlotSource::Generate => {
                let mut b = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for p in inputs {
                    if let Ok(r) = Reader::open(p) {
                        let h = r.header.bounds;
                        b = [
                            b[0].min(h[0]),
                            b[1].min(h[1]),
                            b[2].max(h[3]),
                            b[3].max(h[4]),
                        ];
                    }
                }
                let shape = if s.square {
                    PlotShape::Rect {
                        w: s.side,
                        h: s.side,
                    }
                } else {
                    PlotShape::Circle { radius: s.radius }
                };
                let size = if s.square { s.side } else { 2.0 * s.radius };
                let sampling = if s.systematic {
                    Sampling::Systematic {
                        dx: s.dx,
                        dy: s.dy,
                        random_start: false,
                    }
                } else {
                    Sampling::Random {
                        count: s.count,
                        min_dist: if s.min_dist > 0.0 {
                            s.min_dist
                        } else {
                            2.0 * size
                        },
                    }
                };
                let limit: Option<Vec<Polygon>> = if s.limit.trim().is_empty() {
                    None
                } else {
                    match vector::read_regions(
                        Path::new(s.limit.trim()),
                        &vector::ReadOptions::default(),
                    ) {
                        Ok(l) => Some(
                            l.into_iter()
                                .filter_map(|r| {
                                    if let Shape2::Polygon(p) = r.shape {
                                        Some(p)
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                        ),
                        Err(e) => {
                            log.push(format!("Polígonos límite: {e}"));
                            return None;
                        }
                    }
                };
                match rustlidar_core::plots::generate(b, limit.as_deref(), shape, sampling, s.seed)
                {
                    Ok(plots) => {
                        let layout = paths::unique(&paths::product(&first, "parcelas", "shp"));
                        let prj = Reader::open(&first)
                            .ok()
                            .and_then(|r| r.header.epsg)
                            .and_then(vector::prj_for_epsg);
                        if let Err(e) =
                            rustlidar_core::plots::write_layout(&layout, &plots, prj.as_deref())
                        {
                            log.push(format!("No se pudo guardar la capa de parcelas: {e}"));
                            return None;
                        }
                        log.push(format!(
                            "{} parcelas generadas: {}",
                            plots.len(),
                            layout.display()
                        ));
                        show.push(layout.clone());
                        outputs.push(layout);
                        plots
                    }
                    Err(e) => {
                        log.push(format!("Muestreo: {e}"));
                        return None;
                    }
                }
            }
        };
        if regions.is_empty() {
            log.push("No hay parcelas.".into());
            return None;
        }
        let classes = match class_mask(&self.clip_class) {
            Ok(c) => c,
            Err(e) => {
                log.push(format!("Clases: {e}"));
                return None;
            }
        };
        let returns = if self.clip_returns.trim().is_empty() {
            ReturnSel::all()
        } else {
            match ReturnSel::parse(&self.clip_returns) {
                Ok(r) => r,
                Err(e) => {
                    log.push(format!("Retornos: {e}"));
                    return None;
                }
            }
        };
        if !self.s2.vector && self.s2.clouds == 0 {
            log.push(
                "Elige qué quieres obtener: la capa de parcelas, nubes recortadas, o ambas.".into(),
            );
            return None;
        }
        let (mode, tag) = match self.s2.clouds {
            1 => (ClipMode::PerRegion, "parcela"),
            2 => (ClipMode::Merge, "recorte"),
            _ => (ClipMode::MetricsOnly, "parcelas"),
        };
        let cloud_out = paths::unique(&paths::product(&first, tag, &ext));
        if self.s2.clouds != 0 {
            outputs.push(cloud_out.clone());
        }
        let mut job = ClipJob::new(inputs.to_vec(), regions, cloud_out);
        job.mode = mode;
        job.filter = Filter {
            classes,
            returns,
            zmin: parse_opt(&self.clip_zmin),
            zmax: parse_opt(&self.clip_zmax),
            decimate: 1,
            skip_noise: false,
        };
        if self.clip_normalise {
            match paths::find_dtm(&first) {
                Some(d) => job.dtm = Some(d),
                None => {
                    log.push("Falta el MDT de la primera hoja para normalizar las alturas.".into());
                    return None;
                }
            }
        }
        job.exclusive = self.s2.exclusive;
        if self.s2.vector {
            let base = paths::unique(&paths::product(&first, "parcelas_metricas", "shp"));
            let formats = self.formats();
            if formats.vector {
                show.push(paths::swap_ext(&base, "shp"));
            }
            outputs.push(base.clone());
            job.metrics = Some(SampleMetrics {
                formats,
                csv: base,
                heightbreak: 2.0,
                min_height: 0.0,
            });
        }
        Some(Launch {
            label: ToolId::Clip.title().to_string(),
            task: Box::new(move || jobs::clip(&job)),
            show,
            outputs,
        })
    }
}
