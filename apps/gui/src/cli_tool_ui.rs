//! Las herramientas que no tienen una ventana propia: una ventana genérica que dibuja las
//! opciones de la orden de `hipso` a partir de su esquema (el mismo que usa el constructor de
//! flujos), las ejecuta en segundo plano y abre en el mapa lo que producen. Cada herramienta nueva
//! nace como orden de la línea de comandos y entra aquí sin escribir una ventana.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hipso_core::model::{self, Kind};

use super::*;

/// Las herramientas de esta ventana: orden, icono, rótulo de botón grande y de botón pequeño.
pub const TOOLS: [(&str, &str, &str, &str); 11] = [
    ("extract", "@extract", "Extraer\nvalores", "Extraer valores"),
    ("join-table", "@jointable", "Unir\ntabla", "Unir tabla"),
    ("merge", "@merge", "Combinar\ncapas", "Combinar capas"),
    ("split", "@split", "Dividir\npor campo", "Dividir por campo"),
    ("clip-vector", "@clipvec", "Recortar\ncapa", "Recortar capa"),
    ("near", "@near", "Cercanía", "Cercanía"),
    ("summary", "@summary", "Resumen\nestadístico", "Resumen estadístico"),
    ("rasterize", "@rasterize", "Rasterizar", "Rasterizar"),
    ("focal", "@focal", "Filtro\nfocal", "Filtro focal"),
    ("rstats", "@rstats", "Estadísticas\nráster", "Estadísticas ráster"),
    ("tabulate", "@tabulate", "Tabular\náreas", "Tabular áreas"),
];

enum Msg {
    Line(String),
    Done(Result<(), String>),
}

struct Running {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    lines: Vec<String>,
}

/// Lo que guarda la ventana entre fotogramas.
#[derive(Default)]
pub struct CliTool {
    /// La herramienta cuya ventana está abierta (índice de [`TOOLS`]).
    pub open: Option<usize>,
    args: Vec<BTreeMap<String, Vec<String>>>,
    running: Option<Running>,
    message: String,
}

impl App {
    /// Abre la ventana de la herramienta `k` de [`TOOLS`].
    pub fn open_cli_tool(&mut self, k: usize) {
        if self.cli.args.len() < TOOLS.len() {
            self.cli.args.resize(TOOLS.len(), BTreeMap::new());
        }
        self.cli.open = Some(k);
        self.cli.message.clear();
    }

    /// Si la herramienta `k` tiene su ventana abierta (para iluminar su botón).
    pub fn cli_tool_open(&self, k: usize) -> bool {
        self.cli.open == Some(k)
    }

    /// Lo que han dicho las órdenes en marcha: líneas al registro y, al terminar, los archivos
    /// nombrados tras `->` abiertos en el mapa.
    fn cli_tool_poll(&mut self, ctx: &egui::Context) {
        let Some(run) = &mut self.cli.running else { return };
        let mut done = None;
        while let Ok(m) = run.rx.try_recv() {
            match m {
                Msg::Line(l) => {
                    run.lines.push(l.clone());
                    self.log.push(l);
                }
                Msg::Done(r) => done = Some(r),
            }
        }
        let Some(r) = done else { return };
        let lines = self.cli.running.take().map(|r| r.lines).unwrap_or_default();
        match r {
            Ok(()) => {
                let mut open = Vec::new();
                for l in &lines {
                    if let Some(p) = l.rsplit_once("-> ").map(|(_, p)| PathBuf::from(p.trim())) {
                        if p.is_dir() {
                            let mut files: Vec<PathBuf> = std::fs::read_dir(&p)
                                .into_iter()
                                .flatten()
                                .flatten()
                                .map(|e| e.path())
                                .filter(|f| [Kind::Raster, Kind::Vector].iter().any(|k| k.accepts(f)))
                                .collect();
                            files.sort();
                            open.extend(files);
                        } else if [Kind::Raster, Kind::Vector, Kind::Cloud].iter().any(|k| k.accepts(&p)) {
                            open.push(p);
                        }
                    }
                }
                self.cli.message = lines.last().cloned().unwrap_or_else(|| "Terminado.".into());
                if !open.is_empty() {
                    self.open_files(ctx, open);
                }
            }
            Err(e) => {
                self.cli.message = format!("Falló: {e}");
                self.notice = Some(format!("La herramienta falló: {e}"));
            }
        }
        self.show_log = true;
    }

    /// Las capas del mapa que admite una opción de archivos de tipo `kind`, como (nombre, ruta).
    fn map_files(&self, kind: Kind) -> Vec<(String, PathBuf)> {
        let mut v: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| kind.accepts(&r.path))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        if matches!(kind, Kind::Cloud | Kind::Any) {
            v.extend(self.clouds.iter().map(|p| {
                (p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), p.clone())
            }));
        }
        v
    }

    /// La ventana de la herramienta abierta.
    pub fn cli_tool_window(&mut self, ctx: &egui::Context) {
        self.cli_tool_poll(ctx);
        let Some(k) = self.cli.open else { return };
        let Some(schema) = self.tool_schema() else {
            self.cli.open = None;
            self.notice = Some("No se encuentra el ejecutable «hipso» junto al programa.".into());
            return;
        };
        let name = TOOLS[k].0;
        let Some(spec) = schema.tool(name) else { return };
        let title = model::entry(name).map_or(name, |e| e.title);
        let t = self.tokens();
        let mut open = true;
        let running = self.cli.running.is_some();
        let mut run = false;
        let mut stop = false;
        let mut args = std::mem::take(&mut self.cli.args[k]);
        // lo que falta antes de poder ejecutar
        let missing: Vec<String> = spec
            .args
            .iter()
            .filter(|a| a.required && !a.flag && a.default.is_none() && a.id != "results_dir")
            .filter(|a| args.get(&a.id).is_none_or(|v| v.iter().all(|s| s.trim().is_empty())))
            .map(|a| crate::flow_ui::label_es(name, &a.id).map_or_else(|| a.long.clone().map_or(a.id.clone(), |l| format!("--{l}")), str::to_string))
            .collect();
        egui::Window::new(title)
            .id(egui::Id::new(("cli_tool", name)))
            .open(&mut open)
            .default_width(430.0)
            .default_pos(egui::pos2(ctx.content_rect().right() - 460.0, 215.0))
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(&spec.about).size(11.5).color(t.weak));
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                    for a in spec.args.iter().filter(|a| a.id != "results_dir") {
                        // las opciones de archivo ofrecen primero las capas del mapa
                        if model::wirable(name, a) || (a.path && a.id != "output") {
                            let files = self.map_files(model::arg_kind(name, &a.id));
                            if !files.is_empty() {
                                ui.add_space(4.0);
                                let label = crate::flow_ui::label_es(name, &a.id).map_or_else(|| a.long.clone().map_or(a.id.clone(), |l| if a.positional { l } else { format!("--{l}") }), str::to_string);
                                ui.label(if a.required { format!("{label} *") } else { label });
                                egui::ComboBox::from_id_salt(("cli_map", name, &a.id))
                                    .selected_text("Del mapa…")
                                    .width(ui.available_width() - 8.0)
                                    .show_ui(ui, |ui| {
                                        for (n, p) in &files {
                                            if ui.selectable_label(false, n).clicked() {
                                                let v = args.entry(a.id.clone()).or_default();
                                                if a.multiple {
                                                    v.push(p.display().to_string());
                                                } else {
                                                    *v = vec![p.display().to_string()];
                                                }
                                            }
                                        }
                                    });
                            }
                        }
                        crate::flow_ui::arg_widget(ui, &t, name, a, &mut args, false);
                    }
                });
                ui.add_space(8.0);
                if !missing.is_empty() {
                    ui.label(egui::RichText::new(format!("Falta: {}", missing.join(", "))).size(11.5).color(t.weak));
                }
                ui.horizontal(|ui| {
                    if running {
                        ui.spinner();
                        if ui.button("Cancelar").clicked() {
                            stop = true;
                        }
                    } else if ui.add_enabled(missing.is_empty(), egui::Button::new("Ejecutar")).clicked() {
                        run = true;
                    }
                });
                if !self.cli.message.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(&self.cli.message).size(11.5).color(theme::brand_text(&t)));
                }
            });
        self.cli.args[k] = args;
        if !open {
            self.cli.open = None;
        }
        if stop && let Some(r) = &self.cli.running {
            r.cancel.store(true, Ordering::Relaxed);
        }
        if run {
            self.cli_tool_run(ctx, k);
        }
    }

    fn cli_tool_run(&mut self, ctx: &egui::Context, k: usize) {
        let Some(schema) = self.tool_schema() else { return };
        let Some(exe) = crate::flow_ui::hipso_exe() else { return };
        let Some(spec) = schema.tool(TOOLS[k].0) else { return };
        let argv = model::tool_argv(spec, &self.cli.args[k]);
        self.log(format!("» hipso {}", argv.join(" ")));
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let c2 = c.clone();
            let r = model::spawn(&exe, &argv, &flag, &mut |l| {
                let _ = tx2.send(Msg::Line(l.to_string()));
                c2.request_repaint();
            });
            let _ = tx.send(Msg::Done(r.map_err(|e| e.to_string())));
            c.request_repaint();
        });
        self.cli.running = Some(Running { rx, cancel, lines: Vec::new() });
        self.cli.message = "Ejecutando…".into();
    }
}
