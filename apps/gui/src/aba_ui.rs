//! Modelos de transición (Inventario forestal): regression of a field variable on the metrics of
//! the plots, its validation, and the map of the variable.

use super::*;
use crate::numfield::NumField;
use rustlidar_core::aba::{self, AbaJob, Criterion, Spec, Transform};
use rustlidar_core::paths;

use crate::aba_plots::Diag;

pub struct ModelTool {
    pub open: bool,
    /// Diagnostic graphics of the last model, and the model that is being waited for.
    diag: Option<Diag>,
    diag_open: bool,
    diag_pending: Option<(PathBuf, String, std::time::SystemTime)>,
    plots: String,
    field: String,
    plots_cols: (String, Vec<String>),
    field_cols: (String, Vec<String>),
    plot_key: String,
    field_key: String,
    response: String,
    transform: usize,
    manual: bool,
    criterion: usize,
    max_vars: usize,
    max_vif: f64,
    max_corr: f64,
    intensity: bool,
    /// The analysis before the fit, the candidates in it and which are ticked.
    screening: Option<(String, aba::Screening)>,
    cand_on: Vec<bool>,
    screen_msg: String,
    map: bool,
    cells: Vec<PathBuf>,
    interval: bool,
    extrapolate: bool,
    out: String,
    out_manual: bool,
}

impl Default for ModelTool {
    fn default() -> Self {
        Self {
            open: false,
            diag: None,
            diag_open: false,
            diag_pending: None,
            plots: String::new(),
            field: String::new(),
            plots_cols: (String::new(), Vec::new()),
            field_cols: (String::new(), Vec::new()),
            plot_key: String::new(),
            field_key: String::new(),
            response: String::new(),
            transform: 0,
            manual: false,
            criterion: 0,
            max_vars: 0,
            max_vif: 5.0,
            max_corr: 0.8,
            intensity: false,
            screening: None,
            cand_on: Vec::new(),
            screen_msg: String::new(),
            map: false,
            cells: Vec::new(),
            interval: true,
            extrapolate: false,
            out: String::new(),
            out_manual: false,
        }
    }
}

const TRANSFORMS: [&str; 3] = [
    "Ninguna",
    "Logaritmo de la variable (errores proporcionales)",
    "Logaritmo de la variable y de las métricas (relación potencial)",
];

/// The columns of a table, read again only when the path changes.
fn refresh(slot: &mut (String, Vec<String>), path: &str) {
    if slot.0 == path {
        return;
    }
    slot.0 = path.to_string();
    slot.1 = if Path::new(path.trim()).is_file() {
        aba::columns(Path::new(path.trim())).unwrap_or_default()
    } else {
        Vec::new()
    };
}

/// The column that most likely names the plot.
fn guess_key(cols: &[String]) -> String {
    cols.iter()
        .find(|c| {
            let l = c.to_lowercase();
            l.contains("parcela") || l.contains("plot") || l == "id"
        })
        .or(cols.first())
        .cloned()
        .unwrap_or_default()
}

fn file_row(ui: &mut egui::Ui, label: &str, text: &mut String, exts: &[(&str, &[&str])]) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(text).desired_width(360.0));
        if ui.button("…").clicked() {
            let mut d = crate::prefs::dialog();
            for (n, e) in exts {
                d = d.add_filter(*n, e);
            }
            if let Some(p) = d.pick_file() {
                *text = p.display().to_string();
            }
        }
    });
}

fn column_combo(ui: &mut egui::Ui, id: &str, value: &mut String, cols: &[String], skip: &str) {
    egui::ComboBox::from_id_salt(id)
        .width(220.0)
        .selected_text(if value.is_empty() {
            "(elige)".to_string()
        } else {
            value.clone()
        })
        .show_ui(ui, |ui| {
            for c in cols.iter().filter(|c| c.as_str() != skip) {
                ui.selectable_value(value, c.clone(), c);
            }
        });
}

impl ModelTool {
    /// The job with the settings of the window.
    fn job(&self) -> AbaJob {
        let output = if self.map {
            PathBuf::from(self.out.trim())
        } else {
            // without a map the report goes next to the plots table
            let p = PathBuf::from(self.plots.trim());
            paths::unique_with(
                &paths::product(&p, "modelo", "tif"),
                rustlidar_core::aba::COMPANIONS,
            )
        };
        // the metrics ticked in the table are the model's
        let predictors: Vec<String> = match &self.screening {
            Some((_, sc)) if self.manual => sc
                .vars
                .iter()
                .zip(&self.cand_on)
                .filter(|(_, on)| **on)
                .map(|(v, _)| v.name.clone())
                .collect(),
            _ => Vec::new(),
        };
        AbaJob {
            plots: PathBuf::from(self.plots.trim()),
            field: PathBuf::from(self.field.trim()),
            response: self.response.clone(),
            plot_key: self.plot_key.clone(),
            field_key: self.field_key.clone(),
            spec: Spec {
                transform: [Transform::None, Transform::LogResponse, Transform::LogBoth]
                    [self.transform.min(2)],
                criterion: [Criterion::Bic, Criterion::Aicc][self.criterion.min(1)],
                predictors,
                max_vars: self.max_vars,
                max_vif: self.max_vif,
                candidates: Vec::new(),
                max_corr: self.max_corr,
                intensity: self.intensity,
            },
            cells: if self.map {
                self.cells.clone()
            } else {
                Vec::new()
            },
            output,
            interval_map: self.map && self.interval,
            mask_extrapolation: !self.extrapolate,
            epsg: None,
        }
    }

    /// What the analysis depends on: if it changes, the analysis is out of date.
    fn screening_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.plots,
            self.field,
            self.response,
            self.plot_key,
            self.field_key,
            self.transform,
            self.max_corr,
            self.intensity,
            self.cells.len()
        )
    }
}

impl App {
    pub fn model_window(&mut self, ctx: &egui::Context) {
        if !self.mo.open {
            return;
        }
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| {
                matches!(r.kind, RefKind::Vector(_))
                    && r.path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("shp"))
            })
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        // a model that has just finished: its graphics open by themselves
        if self.running.is_none()
            && let Some((path, response, since)) = self.mo.diag_pending.clone()
            && std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .is_ok_and(|m| m >= since)
        {
            self.mo.diag_pending = None;
            if let Some(d) = Diag::read(&path, &response) {
                self.mo.diag = Some(d);
                self.mo.diag_open = true;
            }
        } else if self.running.is_none() {
            // it did not finish well: nothing to wait for
            self.mo.diag_pending = None;
        }
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Modelos de transición")
            .open(&mut open)
            .default_width(620.0)
            .show(ctx, |ui| {
                let t = &mut self.mo;
                // the button stays on top, whatever the form below is like
                let missing = if t.plots.trim().is_empty() {
                    "Falta la capa de parcelas con métricas"
                } else if t.field.trim().is_empty() {
                    "Faltan los datos de campo"
                } else if t.response.is_empty() {
                    "Elige la variable a estimar"
                } else if t.manual && !t.cand_on.iter().any(|b| *b) {
                    "Marca las variables del modelo (Elegir variables…)"
                } else if t.map && t.cells.is_empty() {
                    "Elige las tablas de métricas de las celdas, o quita el mapa"
                } else {
                    ""
                };
                let busy = self.running.is_some();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(missing.is_empty() && !busy, egui::Button::new("▶ Ajustar y validar"))
                        .clicked()
                    {
                        go = true;
                    }
                    if t.diag.is_some() && ui.button("Gráficas").on_hover_text("Gráficas de diagnóstico del último modelo").clicked() {
                        t.diag_open = true;
                    }
                    if busy {
                        ui.spinner();
                        ui.label("Hay otro proceso en marcha: espera a que termine");
                    } else if !missing.is_empty() {
                        ui.colored_label(egui::Color32::from_rgb(230, 160, 60), missing);
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 200.0).max(220.0))
                    .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Un modelo de transición relaciona una variable medida en campo (volumen, área basimétrica, biomasa…) con las métricas LiDAR de las mismas parcelas y, si se quiere, dibuja su mapa. Se ajusta una regresión lineal y se valida dejando fuera una parcela cada vez, repitiendo la elección de métricas: esa es la cifra de error que cabe esperar en parcelas nuevas. Hacen falta al menos unas 20 parcelas; con menos, el resultado es muy inestable.")
                        .color(weak)
                        .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.label("Capa de parcelas con métricas");
                    ui.add(egui::TextEdit::singleline(&mut t.plots).desired_width(330.0));
                    if ui.button("…").clicked()
                        && let Some(p) = crate::prefs::dialog()
                            .add_filter("Capa o tabla de parcelas", &["shp", "csv", "txt"])
                            .pick_file()
                    {
                        t.plots = p.display().to_string();
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                t.plots = path.display().to_string();
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay capas de polígonos cargadas");
                        }
                    });
                });
                file_row(
                    ui,
                    "Datos de campo",
                    &mut t.field,
                    &[("Tabla (una fila por parcela)", &["csv", "txt", "xlsx"])],
                );
                ui.label(egui::RichText::new("La capa de parcelas es la que da «Recortar parcelas» (modo solo métricas, formato shapefile): un polígono por parcela con sus métricas; puede estar ya cargada en el programa. El campo, un Excel o CSV con una fila por parcela y su nombre igual al de la parcela.").color(weak).size(11.0));
                refresh(&mut t.plots_cols, &t.plots);
                refresh(&mut t.field_cols, &t.field);
                if t.plot_key.is_empty() || !t.plots_cols.1.contains(&t.plot_key) {
                    t.plot_key = guess_key(&t.plots_cols.1);
                }
                if t.field_key.is_empty() || !t.field_cols.1.contains(&t.field_key) {
                    t.field_key = guess_key(&t.field_cols.1);
                }
                if !t.field_cols.1.contains(&t.response) {
                    t.response.clear();
                }
                ui.horizontal(|ui| {
                    ui.label("Parcela en métricas");
                    let cols = t.plots_cols.1.clone();
                    column_combo(ui, "mo_pk", &mut t.plot_key, &cols, "");
                    ui.label("en campo");
                    let cols = t.field_cols.1.clone();
                    column_combo(ui, "mo_fk", &mut t.field_key, &cols, "");
                });
                ui.horizontal(|ui| {
                    ui.label("Variable a estimar");
                    let cols = t.field_cols.1.clone();
                    let key = t.field_key.clone();
                    column_combo(ui, "mo_resp", &mut t.response, &cols, &key);
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Transformación");
                    egui::ComboBox::from_id_salt("mo_tr")
                        .width(380.0)
                        .selected_text(TRANSFORMS[t.transform.min(2)])
                        .show_ui(ui, |ui| {
                            for (k, n) in TRANSFORMS.iter().enumerate() {
                                ui.selectable_value(&mut t.transform, k, *n);
                            }
                        });
                });
                ui.add_space(2.0);
                ui.label(egui::RichText::new("Variables del modelo").strong());
                ui.horizontal(|ui| {
                    ui.radio_value(&mut t.manual, false, "Automático")
                        .on_hover_text("El programa elige las métricas solo");
                    ui.radio_value(&mut t.manual, true, "Elegir variables")
                        .on_hover_text("Tú marcas las métricas del modelo en una tabla");
                });
                if !t.manual {
                    ui.horizontal(|ui| {
                        ui.label("Criterio");
                        egui::ComboBox::from_id_salt("mo_cr")
                            .width(90.0)
                            .selected_text(["BIC", "AICc"][t.criterion.min(1)])
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut t.criterion, 0, "BIC");
                                ui.selectable_value(&mut t.criterion, 1, "AICc");
                            });
                        ui.label("Variables como mucho");
                        ui.add(NumField::new(&mut t.max_vars).range(0..=10).suffix(" (0: auto)"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Colinealidad máxima |r|");
                        ui.add(NumField::new(&mut t.max_corr).range(0.3..=1.0).fixed_decimals(2));
                        ui.label("VIF máximo");
                        ui.add(NumField::new(&mut t.max_vif).range(1.5..=100.0));
                    });
                    ui.checkbox(&mut t.intensity, "Probar también las métricas de intensidad");
                    ui.label(egui::RichText::new("El programa deja de lado las métricas que repiten a otra (correlación mayor que el límite: se queda la más relacionada con la variable) y elige entre el resto por el criterio, con una variable por cada diez parcelas como mucho (entre dos y cinco). El informe trae el análisis previo de normalidad y de candidatas. La intensidad de casi todos los vuelos no está calibrada entre pasadas y se deja fuera salvo que se pida.").color(weak).size(11.0));
                } else {
                    ui.horizontal(|ui| {
                        let ready = !t.plots.trim().is_empty() && !t.field.trim().is_empty() && !t.response.is_empty();
                        if ui.add_enabled(ready, egui::Button::new("Elegir variables…")).on_hover_text("Analiza la normalidad de la variable y las métricas, y las muestra para marcar las del modelo").clicked() {
                            let key = t.screening_key();
                            // the table of an earlier analysis must not limit this one
                            t.screening = None;
                            match aba::analyze(&t.job()) {
                                Ok((sc, notes)) => {
                                    t.cand_on = vec![false; sc.vars.len()];
                                    t.screen_msg = notes.join(" ");
                                    t.screening = Some((key, sc));
                                }
                                Err(e) => {
                                    t.screening = None;
                                    t.screen_msg = e.to_string();
                                }
                            }
                        }
                        ui.checkbox(&mut t.intensity, "Con las de intensidad");
                    });
                    if !t.screen_msg.is_empty() {
                        ui.label(egui::RichText::new(t.screen_msg.clone()).color(weak).size(11.0));
                    }
                    // an analysis of other settings is stale
                    let key = t.screening_key();
                    if t.screening.as_ref().is_some_and(|(k, _)| *k != key) {
                        t.screening = None;
                    }
                    if let Some((_, sc)) = &t.screening {
                        let line = |n: &aba::Normality| {
                            if n.p.is_finite() {
                                format!("W = {:.3}, p = {:.3} ({})", n.w, n.p, if n.p < 0.05 { "no normal" } else { "normal" })
                            } else {
                                "no calculable".to_string()
                            }
                        };
                        ui.label(format!("«{}»: Shapiro–Wilk {}", t.response, line(&sc.response)));
                        if let Some(l) = &sc.response_ln {
                            ui.label(format!("Con el logaritmo: Shapiro–Wilk {}", line(l)));
                        }
                        for a in &sc.advice {
                            ui.colored_label(egui::Color32::from_rgb(230, 160, 60), a);
                        }
                        let ticked = t.cand_on.iter().filter(|b| **b).count();
                        let most = (sc.response.n / 10).clamp(2, 5);
                        ui.horizontal(|ui| {
                            ui.label(format!("{} métricas, {} sin repetir a otra · marcadas: {ticked}", sc.vars.len(), sc.kept));
                            if ui.button("Sugeridas").on_hover_text("Marca las mejor relacionadas que no se repiten").clicked() {
                                t.cand_on = sc.vars.iter().enumerate().map(|(i, v)| v.dropped_by.is_none() && i < most).collect();
                            }
                            if ui.button("Ninguna").clicked() {
                                t.cand_on.iter_mut().for_each(|b| *b = false);
                            }
                        });
                        if ticked > most {
                            ui.colored_label(egui::Color32::from_rgb(230, 160, 60), format!("Con {} parcelas, más de {most} variables arriesga el sobreajuste.", sc.response.n));
                        }
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            egui::Grid::new("mo_cands").striped(true).show(ui, |ui| {
                                for h in ["", "Métrica", "r", "ρ", "Normal p", "Asim.", "Estado"] {
                                    ui.label(egui::RichText::new(h).strong());
                                }
                                ui.end_row();
                                for (i, v) in sc.vars.iter().enumerate() {
                                    if let Some(on) = t.cand_on.get_mut(i) {
                                        ui.checkbox(on, "");
                                    }
                                    ui.label(&v.name);
                                    ui.label(format!("{:.2}", v.r));
                                    ui.label(format!("{:.2}", v.rho));
                                    ui.label(format!("{:.3}", v.normal_p));
                                    ui.label(format!("{:.2}", v.skew));
                                    ui.label(match &v.dropped_by {
                                        Some((by, r)) => format!("repite a {by} (r = {r:.2})"),
                                        None => "no se repite".to_string(),
                                    });
                                    ui.end_row();
                                }
                            });
                        });
                        ui.label(egui::RichText::new("r y ρ: correlación de Pearson y de Spearman con la variable. Normal p: Shapiro–Wilk de la métrica. No marques métricas que se repiten entre sí. La regresión pide residuos normales, no variables normales; se comprueban tras el ajuste.").color(weak).size(11.0));
                    }
                }
                ui.separator();
                ui.checkbox(&mut t.map, "Hacer el mapa de la variable");
                if t.map {
                    ui.horizontal(|ui| {
                        ui.label(format!("Métricas de las celdas: {} archivo(s)", t.cells.len()));
                        if ui.button("Elegir…").clicked()
                            && let Some(f) = crate::prefs::dialog()
                                .add_filter("Tabla de texto", &["csv", "txt"])
                                .pick_files()
                        {
                            t.cells = f;
                        }
                        if !t.cells.is_empty() && ui.button("Quitar").clicked() {
                            t.cells.clear();
                        }
                    });
                    ui.label(egui::RichText::new("Son las tablas de texto (.csv) de «Métricas de rodal» (una por hoja). Deben calcularse igual que las de las parcelas: mismo corte de altura, mismas clases y retornos, y celdas de tamaño parecido al de las parcelas. Solo se ofrecen las métricas que están en las dos tablas.").color(weak).size(11.0));
                    ui.checkbox(&mut t.interval, "Guardar también el intervalo de predicción del 95 % (semiancho)");
                    ui.checkbox(&mut t.extrapolate, "Estimar también las celdas fuera del rango de las parcelas");
                    ui.label(egui::RichText::new("Por defecto esas celdas quedan vacías: el modelo no ha visto nada parecido y extrapolar da cifras sin respaldo.").color(weak).size(11.0));
                    if !t.out_manual {
                        t.out = if t.plots.trim().is_empty() {
                            String::new()
                        } else {
                            let p = PathBuf::from(t.plots.trim());
                            let resp: String = t.response.chars().take_while(|c| c.is_alphanumeric()).collect();
                            paths::unique_with(&paths::product(&p, &format!("mapa_{resp}"), "tif"), rustlidar_core::aba::COMPANIONS).display().to_string()
                        };
                    }
                    ui.horizontal(|ui| {
                        ui.label("Mapa");
                        if ui.add(egui::TextEdit::singleline(&mut t.out).desired_width(430.0)).changed() {
                            t.out_manual = true;
                        }
                        if t.out_manual && ui.button("auto").clicked() {
                            t.out_manual = false;
                        }
                    });
                } else if !t.plots.trim().is_empty() {
                    let p = PathBuf::from(t.plots.trim());
                    t.out = p.with_file_name("modelo.tif").display().to_string();
                }
                    });
            });
        if go {
            let job = self.mo.job();
            self.mo.diag_pending = Some((
                job.plots_path(),
                job.response.clone(),
                std::time::SystemTime::now(),
            ));
            let t = &self.mo;
            let mut outputs = vec![job.report_path(), job.plots_path()];
            let mut show = Vec::new();
            if t.map {
                outputs.push(job.output.clone());
                show.push(job.output.clone());
                if job.interval_map {
                    outputs.push(job.interval_path());
                    show.push(job.interval_path());
                }
            }
            let task: tools::Task = Box::new(move || aba::run(&job));
            self.start(
                ctx,
                Launch {
                    label: "Modelos de transición".to_string(),
                    task,
                    show,
                    outputs,
                },
            );
        }
        if self.mo.diag_open {
            let mut keep = true;
            let h = (ctx.content_rect().height() - 90.0).max(300.0);
            egui::Window::new("Diagnóstico del modelo")
                .open(&mut keep)
                .default_width(680.0)
                .default_height(h.min(640.0))
                .vscroll(true)
                .show(ctx, |ui| {
                    if let Some(d) = &self.mo.diag {
                        d.show(ui);
                    }
                });
            self.mo.diag_open = keep;
        }
        if !open {
            self.mo.open = false;
        }
    }
}
