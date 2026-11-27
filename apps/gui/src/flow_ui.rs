//! El constructor de flujos: un lienzo donde las herramientas de HIPSO son nodos conectados de modo
//! que lo que produce uno lo recibe el siguiente, y que se ejecutan en orden en segundo plano. El
//! modelo y el motor están en `hipso_core::model`; este archivo es el lienzo, la paleta de
//! herramientas, las opciones del nodo seleccionado y la ejecución.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hipso_core::model::{self, Edge, Kind, Model, Node, NodeKind, Schema, State};

use super::*;

const NODE_W: f32 = 210.0;
const INPUT_W: f32 = 170.0;
const HEAD: f32 = 28.0;
const ROW: f32 = 20.0;
const PORT: f32 = 5.5;

/// Lo que el hilo que ejecuta el flujo comunica a la ventana.
enum Msg {
    Log(String),
    State(String, State),
    Done(Result<model::Report, String>),
}

struct Running {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
}

/// Todo lo que la vista de flujos conserva entre fotogramas.
pub struct Flow {
    pub model: Model,
    schema: Option<Result<Arc<Schema>, String>>,
    pan: Vec2,
    zoom: f32,
    canvas: Rect,
    selected: BTreeSet<String>,
    sel_edge: Option<usize>,
    undo: Vec<Model>,
    redo: Vec<Model>,
    last_push: Option<Instant>,
    search: String,
    /// El nodo del que se está arrastrando un hilo.
    wiring: Option<String>,
    states: HashMap<String, State>,
    running: Option<Running>,
    /// El archivo en el que se guarda el flujo.
    path: Option<PathBuf>,
    started: bool,
    fit_next: bool,
    add_to_map: bool,
    /// Cuántas teselas tratar a la vez (0: automático).
    jobs: usize,
    /// Hasta qué nodo fue la última ejecución parcial, para reanudar la misma.
    until: Option<String>,
    saved: Option<(Model, Instant)>,
    status: String,
}

impl Flow {
    pub fn new() -> Self {
        Self {
            model: Model::default(),
            schema: None,
            pan: Vec2::ZERO,
            zoom: 1.0,
            canvas: Rect::NOTHING,
            selected: BTreeSet::new(),
            sel_edge: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_push: None,
            search: String::new(),
            wiring: None,
            states: HashMap::new(),
            running: None,
            path: None,
            started: false,
            fit_next: false,
            add_to_map: true,
            jobs: 0,
            until: None,
            saved: None,
            status: String::new(),
        }
    }

    /// Recuerda el modelo tal como estaba, para que Deshacer pueda recuperarlo.
    fn push_undo(&mut self) {
        self.undo.push(self.model.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last_push = Some(Instant::now());
    }

    /// Como [`Flow::push_undo`], pero como mucho una vez por segundo, para lo que se escribe.
    fn push_undo_typing(&mut self, before: &Model) {
        if self.last_push.is_none_or(|t| t.elapsed().as_secs_f32() > 1.0) {
            self.undo.push(before.clone());
            self.redo.clear();
            self.last_push = Some(Instant::now());
        }
    }
}

/// El ejecutable `hipso` que viaja con este programa.
pub fn hipso_exe() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "hipso.exe" } else { "hipso" };
    let p = me.parent()?.join(name);
    p.is_file().then_some(p)
}

/// Cuántos resultados se abren como mucho en el mapa.
const MAX_TO_MAP: usize = 60;
/// Cuántas líneas de registro se guardan en memoria.
const MAX_LOG: usize = 20_000;

/// La carpeta de resultados de un flujo: la elegida o `resultados_flujo` junto a la primera
/// entrada.
fn results_of(model: &Model) -> PathBuf {
    if !model.results.trim().is_empty() {
        return PathBuf::from(&model.results);
    }
    model
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Input { path } => Some(PathBuf::from(path)),
            _ => None,
        })
        .and_then(|p| if p.is_dir() { Some(p) } else { p.parent().map(PathBuf::from) })
        .unwrap_or_else(std::env::temp_dir)
        .join("resultados_flujo")
}

fn group_color(group: &str) -> Color32 {
    match group {
        "Nubes" | "Preparar" => Color32::from_rgb(0x3A, 0x6F, 0xB5),
        "Superficies" => Color32::from_rgb(0x3A, 0x8E, 0x4A),
        "Inventario" => Color32::from_rgb(0xC7, 0x7A, 0x1E),
        "Terreno" => Color32::from_rgb(0x8A, 0x6A, 0x3C),
        "Ráster" => Color32::from_rgb(0x7A, 0x52, 0xA8),
        "Vectorial" => Color32::from_rgb(0x2E, 0x8C, 0x8C),
        _ => Color32::from_rgb(0x6B, 0x74, 0x80),
    }
}

fn kind_color(k: Kind) -> Color32 {
    match k {
        Kind::Cloud => Color32::from_rgb(0x4A, 0x86, 0xE0),
        Kind::Raster => Color32::from_rgb(0x46, 0xB0, 0x56),
        Kind::Vector => Color32::from_rgb(0xE8, 0x91, 0x2D),
        Kind::Table => Color32::from_rgb(0x9A, 0xA3, 0xAE),
        Kind::Any => Color32::from_rgb(0xB5, 0xBB, 0xC4),
    }
}

/// Dónde están las partes de un nodo, en el espacio del lienzo.
struct Lay {
    id: String,
    rect: Rect,
    /// Las opciones de archivo que puede recibir: la opción, el texto que muestra y si es
    /// necesaria.
    ins: Vec<(String, String, bool, Kind)>,
}

impl Lay {
    fn in_pos(&self, k: usize) -> Pos2 {
        Pos2::new(self.rect.min.x, self.rect.min.y + HEAD + 6.0 + k as f32 * ROW + ROW / 2.0)
    }

    fn out_pos(&self) -> Pos2 {
        Pos2::new(self.rect.max.x, self.rect.min.y + HEAD + 6.0 + ROW / 2.0)
    }
}

/// El nombre de una entrada tal como lo muestra su puerto: lo que se conecta en él, en español; las
/// que no tienen uno propio, la opción de la línea de órdenes.
fn port_label(tool: &str, a: &model::ArgSpec) -> String {
    let name = match (tool, a.id.as_str()) {
        ("zonal" | "extract", "raster") => "Ráster",
        ("model", "field") => "Datos de campo",
        ("trees", "field") => "Árboles de campo",
        (_, "inputs" | "input" | "rasters") => match model::arg_kind(tool, &a.id) {
            Kind::Cloud if a.multiple => "Nubes",
            Kind::Cloud => "Nube",
            Kind::Raster if a.multiple => "Rásteres",
            Kind::Raster => "Ráster",
            Kind::Vector if a.multiple => "Capas",
            Kind::Vector => "Capa",
            _ => "Entrada",
        },
        (_, "dtm") => "MDT",
        (_, "chm") => "CHM",
        (_, "around") => "CHM vecinos",
        (_, "cloud") => "Nube",
        (_, "breaklines") => "Líneas de rotura",
        (_, "recipe") => "Receta",
        (_, "tops_file") => "Cimas",
        (_, "regions") => "Polígonos",
        (_, "mask") => "Máscara",
        (_, "reference") => "Referencia",
        (_, "surface") => "Superficie",
        (_, "piles") => "Contornos",
        (_, "before") => "Antes",
        (_, "after") => "Después",
        (_, "existing") => "Terreno actual",
        (_, "design") => "Diseño",
        (_, "layer") => "Capa",
        (_, "table") => "Tabla",
        (_, "zones") => "Zonas",
        (_, "values") => "Clases",
        (_, "plots") => "Parcelas",
        (_, "cells") => "Celdas",
        (_, "first") => "Primera capa",
        (_, "second") => "Segunda capa",
        (_, "to") => "Capa cercana",
        (_, "image") => "Ortofoto",
        (_, "trajectory") => "Trayectoria",
        (_, "stands") => "Rodales",
        (_, "samples") => "Muestras",
        (_, "rules" | "rules_file") => "Reglas",
        (_, "barriers") => "Barreras",
        (_, "prefer") => "Preferentes",
        (_, "lines") => "Líneas",
        (_, "edits") => "Ediciones",
        (_, "limit") => "Límite",
        (_, "from") => "Nube modelo",
        _ => "",
    };
    match &a.long {
        _ if !name.is_empty() => name.to_string(),
        Some(l) if !a.positional => format!("--{l}"),
        _ => a.id.replace('_', " "),
    }
}

fn layout_of(m: &Model, schema: &Schema) -> Vec<Lay> {
    m.nodes
        .iter()
        .map(|n| {
            let (w, ins) = match &n.kind {
                NodeKind::Input { .. } => (INPUT_W, Vec::new()),
                NodeKind::Tool { tool, .. } => {
                    let mut ins: Vec<(String, String, bool, Kind)> = schema
                        .tool(tool)
                        .map(|s| {
                            s.args
                                .iter()
                                .filter(|a| model::wirable(tool, a))
                                .map(|a| {
                                    (
                                        a.id.clone(),
                                        port_label(tool, a),
                                        a.required,
                                        model::arg_kind(tool, &a.id),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    // primero las entradas principales, en el orden en que las toma la orden
                    ins.sort_by_key(|(id, _, req, _)| {
                        let pos = schema
                            .tool(tool)
                            .and_then(|s| s.arg(id))
                            .is_some_and(|a| a.positional);
                        (!pos, !req)
                    });
                    (NODE_W, ins)
                }
            };
            let rows = ins.len().max(1) as f32;
            Lay {
                id: n.id.clone(),
                rect: Rect::from_min_size(
                    Pos2::new(n.x, n.y),
                    Vec2::new(w, HEAD + 12.0 + rows * ROW),
                ),
                ins,
            }
        })
        .collect()
}

/// Si al soltar un hilo sobre un nodo conviene esta entrada: la admite y, cuando lo que llega es un
/// archivo, es de su tipo.
fn fits(src: &Node, tool: &str, port: &str, kind: Kind) -> bool {
    match &src.kind {
        NodeKind::Input { path } => {
            let p = std::path::Path::new(path);
            !p.is_file() || kind.accepts(p)
        }
        NodeKind::Tool { .. } => model::can_feed(src, tool, port),
    }
}

/// Puntos del hilo entre dos puertos, en el espacio de la pantalla.
fn wire(a: Pos2, b: Pos2) -> Vec<Pos2> {
    let dx = ((b.x - a.x).abs() * 0.5).max(40.0);
    let (c1, c2) = (Pos2::new(a.x + dx, a.y), Pos2::new(b.x - dx, b.y));
    (0..=24)
        .map(|k| {
            let t = k as f32 / 24.0;
            let u = 1.0 - t;
            let f = |p0: f32, p1: f32, p2: f32, p3: f32| {
                u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
            };
            Pos2::new(f(a.x, c1.x, c2.x, b.x), f(a.y, c1.y, c2.y, b.y))
        })
        .collect()
}

fn dist_to_polyline(p: Pos2, pts: &[Pos2]) -> f32 {
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let t = if ab.length_sq() == 0.0 {
                0.0
            } else {
                ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
            };
            (a + ab * t).distance(p)
        })
        .fold(f32::MAX, f32::min)
}

impl App {
    /// Da a la vista de flujos lo que necesita la primera vez: el esquema de las órdenes y el
    /// borrador de la última sesión.
    pub fn flow_start(&mut self) {
        if self.flow.started {
            return;
        }
        self.flow.started = true;
        self.flow.schema = Some(
            hipso_exe()
                .ok_or_else(|| "No se encuentra el ejecutable «hipso» junto al programa.".to_string())
                .and_then(|exe| {
                    std::process::Command::new(exe)
                        .arg("schema")
                        .output()
                        .map_err(|e| e.to_string())
                })
                .and_then(|o| String::from_utf8(o.stdout).map_err(|e| e.to_string()))
                .and_then(|t| Schema::from_json(&t).map_err(|e| e.to_string()))
                .map(Arc::new),
        );
        if let Some(text) = prefs::draft().and_then(|f| std::fs::read_to_string(f).ok())
            && let Ok(m) = Model::from_json(&text)
        {
            self.flow.model = m;
            self.flow.fit_next = true;
            self.flow_place();
        }
        self.flow.saved = Some((self.flow.model.clone(), Instant::now()));
    }

    /// Coloca un flujo cuyos nodos todavía no tienen posición (escrito a mano o por otro programa).
    fn flow_place(&mut self) {
        let unplaced = self.flow.model.nodes.len() > 1
            && self.flow.model.nodes.iter().all(|n| n.x == 0.0 && n.y == 0.0);
        if let (true, Some(Ok(schema))) = (unplaced, self.flow.schema.clone()) {
            self.flow_arrange(&schema);
            self.flow.undo.clear();
        }
    }

    /// La vista de flujos: paleta a la izquierda, lienzo en el centro, opciones del nodo
    /// seleccionado a la derecha y los problemas debajo.
    pub fn flow_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.flow_start();
        self.flow_poll(ctx);
        let t = self.tokens();
        let schema = match &self.flow.schema {
            Some(Ok(s)) => s.clone(),
            Some(Err(e)) => {
                let e = e.clone();
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(t.canvas).inner_margin(24))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Los flujos no están disponibles").strong());
                        ui.label(e);
                    });
                return;
            }
            None => return,
        };
        let before = self.flow.model.clone();
        egui::Panel::left("flow_palette")
            .exact_size(232.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(t.pane).inner_margin(egui::Margin::symmetric(10, 10)))
            .show(ui, |ui| self.flow_palette(ui, &schema));
        egui::Panel::right("flow_params")
            .exact_size(316.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(t.pane).inner_margin(egui::Margin::symmetric(12, 10)))
            .show(ui, |ui| self.flow_params(ui, &schema));
        let problems = self.flow.model.problems(&schema);
        let warnings = self.flow.model.warnings();
        egui::Panel::bottom("flow_problems")
            .exact_size(if problems.is_empty() && warnings.is_empty() { 30.0 } else { 104.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(t.pane)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .stroke(Stroke::new(1.0, t.border)),
            )
            .show(ui, |ui| {
                if problems.is_empty() && warnings.is_empty() {
                    let text = if self.flow.status.is_empty() {
                        "Sin problemas: el flujo se puede ejecutar.".to_string()
                    } else {
                        self.flow.status.clone()
                    };
                    ui.label(egui::RichText::new(text).color(t.weak));
                    return;
                }
                if !problems.is_empty() {
                    ui.label(egui::RichText::new(format!("Problemas ({})", problems.len())).strong());
                } else if !self.flow.status.is_empty() {
                    ui.label(egui::RichText::new(&self.flow.status).color(t.weak));
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let lines = problems.iter().map(|p| ("•", p)).chain(warnings.iter().map(|p| ("Aviso:", p)));
                    for (mark, p) in lines {
                        let r = ui.add(
                            egui::Label::new(egui::RichText::new(format!("{mark} {}", p.text)).color(t.text))
                                .sense(Sense::click()),
                        );
                        if r.clicked()
                            && let Some(id) = &p.node
                        {
                            self.flow.selected = BTreeSet::from([id.clone()]);
                        }
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.canvas))
            .show(ui, |ui| self.flow_canvas(ui, ctx, &schema));
        // lo que han cambiado los paneles y el lienzo
        if self.flow.model != before {
            self.flow.push_undo_typing(&before);
            self.flow.status.clear();
        }
        self.flow_keys(ctx);
        self.flow_draft();
    }

    /// Escribe el flujo en el archivo de borrador, para que esté ahí la próxima vez que se abra el
    /// programa.
    fn flow_draft(&mut self) {
        let due = match &self.flow.saved {
            Some((m, at)) => *m != self.flow.model && at.elapsed().as_secs_f32() > 1.0,
            None => true,
        };
        if due {
            if let Some(f) = prefs::draft() {
                let _ = hipso_core::paths::write_atomic(&f, self.flow.model.to_json());
            }
            self.flow.saved = Some((self.flow.model.clone(), Instant::now()));
        }
    }

    fn flow_palette(&mut self, ui: &mut egui::Ui, schema: &Schema) {
        let t = self.tokens();
        ui.label(egui::RichText::new("Herramientas").size(15.0).strong().color(t.text));
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.flow.search)
                .hint_text("Buscar herramienta")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(6.0);
        let mut add: Option<NodeKind> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::CollapsingHeader::new(egui::RichText::new("Entradas").strong())
                .default_open(true)
                .show(ui, |ui| {
                    if ui.button("Archivo…").on_hover_text("Una nube, un ráster o una capa").clicked()
                        && let Some(f) = prefs::dialog().pick_file()
                    {
                        add = Some(NodeKind::Input { path: f.display().to_string() });
                    }
                    if ui
                        .button("Carpeta de hojas…")
                        .on_hover_text("Todos los archivos de una carpeta, como una entrega de hojas")
                        .clicked()
                        && let Some(d) = rfd::FileDialog::new().pick_folder()
                    {
                        add = Some(NodeKind::Input { path: d.display().to_string() });
                    }
                    if !self.clouds.is_empty() || !self.refs.is_empty() {
                        ui.menu_button("Del mapa…", |ui| {
                            for p in &self.clouds {
                                let name = p
                                    .file_name()
                                    .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                                if ui.button(name).clicked() {
                                    add = Some(NodeKind::Input { path: p.display().to_string() });
                                    ui.close();
                                }
                            }
                            for l in &self.refs {
                                if ui.button(&l.name).clicked() {
                                    add = Some(NodeKind::Input { path: l.path.display().to_string() });
                                    ui.close();
                                }
                            }
                        });
                    }
                });
            let q = self.flow.search.trim().to_lowercase();
            let mut groups: Vec<&str> = Vec::new();
            for e in model::CATALOG {
                if !groups.contains(&e.group) {
                    groups.push(e.group);
                }
            }
            for g in groups {
                let tools: Vec<&model::Entry> = model::CATALOG
                    .iter()
                    .filter(|e| e.group == g && schema.tool(e.tool).is_some())
                    .filter(|e| {
                        q.is_empty() || e.title.to_lowercase().contains(&q) || e.tool.contains(&q)
                    })
                    .collect();
                if tools.is_empty() {
                    continue;
                }
                let head = egui::RichText::new(g).strong().color(group_color(g));
                egui::CollapsingHeader::new(head)
                    .id_salt(("flow_group", g))
                    .open(if q.is_empty() { None } else { Some(true) })
                    .show(ui, |ui| {
                        for e in tools {
                            let about = schema.tool(e.tool).map(|s| s.about.clone()).unwrap_or_default();
                            if ui.selectable_label(false, e.title).on_hover_text(about).clicked() {
                                add = Some(NodeKind::Tool {
                                    tool: e.tool.to_string(),
                                    args: Default::default(),
                                });
                            }
                        }
                    });
            }
        });
        if let Some(kind) = add {
            self.flow_add(kind);
        }
    }

    /// Añade como entradas los archivos y carpetas soltados sobre la ventana.
    pub fn flow_drop(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            self.flow_add(NodeKind::Input { path: p.display().to_string() });
        }
    }

    /// Añade un nodo en el centro de lo que muestra el lienzo.
    fn flow_add(&mut self, kind: NodeKind) {
        self.flow.push_undo();
        let z = self.flow.zoom.max(0.1);
        let c = self.flow.canvas;
        let centre = if c.is_positive() {
            let p = (c.center().to_vec2() - c.min.to_vec2() - self.flow.pan) / z;
            Pos2::new(p.x, p.y)
        } else {
            Pos2::ZERO
        };
        let k = self.flow.model.nodes.len() as f32 % 6.0;
        let id = self.flow.model.new_id();
        self.flow.model.nodes.push(Node {
            id: id.clone(),
            label: String::new(),
            kind,
            keep: false,
            x: centre.x - NODE_W / 2.0 + k * 18.0,
            y: centre.y - 40.0 + k * 18.0,
        });
        self.flow.selected = BTreeSet::from([id]);
        self.flow.sel_edge = None;
    }

    fn flow_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, schema: &Schema) {
        let t = self.tokens();
        let rect = ui.available_rect_before_wrap();
        self.flow.canvas = rect;
        let bg = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let lays = layout_of(&self.flow.model, schema);
        if self.flow.fit_next {
            self.flow.fit_next = false;
            self.flow_fit(&lays);
        }
        // zoom con Ctrl y la rueda o con un gesto de pellizco, alrededor del puntero; la rueda sola
        // desplaza
        if let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)) {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if zoom != 1.0 {
                let z0 = self.flow.zoom;
                let z1 = (z0 * zoom).clamp(0.3, 2.0);
                let local = p - rect.min - self.flow.pan;
                self.flow.pan += local - local * (z1 / z0);
                self.flow.zoom = z1;
            } else if scroll != Vec2::ZERO {
                self.flow.pan += scroll;
            }
        }
        if bg.dragged() {
            self.flow.pan += bg.drag_delta();
        }
        let z = self.flow.zoom;
        let origin = rect.min.to_vec2() + self.flow.pan;
        let to_s = |p: Pos2| Pos2::new(p.x * z, p.y * z) + origin;
        let rect_s = |r: Rect| Rect::from_min_max(to_s(r.min), to_s(r.max));
        // un fondo de puntos
        let step = 24.0 * z;
        if step > 8.0 {
            let (ox, oy) = (origin.x.rem_euclid(step), origin.y.rem_euclid(step));
            let mut x = rect.min.x + ox;
            while x < rect.max.x {
                let mut y = rect.min.y + oy;
                while y < rect.max.y {
                    painter.circle_filled(Pos2::new(x, y), 1.0, t.border);
                    y += step;
                }
                x += step;
            }
        }
        let index: HashMap<&str, usize> = lays.iter().enumerate().map(|(i, l)| (l.id.as_str(), i)).collect();
        // los hilos, bajo los nodos
        let mut wires: Vec<Vec<Pos2>> = Vec::new();
        for (k, e) in self.flow.model.edges.iter().enumerate() {
            let (Some(&a), Some(&b)) = (index.get(e.from.as_str()), index.get(e.to.as_str())) else {
                wires.push(Vec::new());
                continue;
            };
            let Some(port) = lays[b].ins.iter().position(|p| p.0 == e.port) else {
                wires.push(Vec::new());
                continue;
            };
            let pts = wire(to_s(lays[a].out_pos()), to_s(lays[b].in_pos(port)));
            let on = self.flow.sel_edge == Some(k);
            painter.add(egui::Shape::line(
                pts.clone(),
                Stroke::new(if on { 3.0 } else { 2.0 }, if on { t.accent } else { t.weak }),
            ));
            wires.push(pts);
        }
        if bg.clicked()
            && let Some(p) = bg.interact_pointer_pos()
        {
            self.flow.sel_edge = wires
                .iter()
                .position(|w| !w.is_empty() && dist_to_polyline(p, w) < 6.0);
            if self.flow.sel_edge.is_some() || !ui.input(|i| i.modifiers.shift) {
                self.flow.selected.clear();
            }
        }
        // los nodos
        let font = |s: f32| egui::FontId::proportional((s * z).clamp(5.0, 22.0));
        let mut connect: Option<Edge> = None;
        let mut moved: Option<(String, Vec2)> = None;
        let mut clicked: Option<(String, bool)> = None;
        let mut toggle_keep: Option<String> = None;
        let mut wire_from: Option<String> = None;
        let pointer = ui.input(|i| i.pointer.interact_pos());
        for (n, lay) in self.flow.model.nodes.iter().zip(&lays) {
            let r = rect_s(lay.rect);
            if !r.intersects(rect) {
                continue;
            }
            let selected = self.flow.selected.contains(&n.id);
            let (color, title) = match &n.kind {
                NodeKind::Input { .. } => (group_color(""), self.flow.model.name(n)),
                NodeKind::Tool { tool, .. } => (
                    group_color(model::entry(tool).map_or("", |e| e.group)),
                    self.flow.model.name(n),
                ),
            };
            painter.rect_filled(r.translate(Vec2::new(0.0, 2.0)), 6.0 * z, Color32::from_black_alpha(40));
            painter.rect_filled(r, 6.0 * z, t.ribbon);
            let head = Rect::from_min_size(r.min, Vec2::new(r.width(), HEAD * z));
            painter.rect_filled(head, egui::CornerRadius { nw: (6.0 * z) as u8, ne: (6.0 * z) as u8, sw: 0, se: 0 }, color);
            painter.rect_stroke(
                r,
                6.0 * z,
                Stroke::new(if selected { 2.5 } else { 1.0 }, if selected { t.accent } else { t.border }),
                egui::StrokeKind::Inside,
            );
            // el estado de la ejecución
            let lamp = match self.flow.states.get(&n.id) {
                Some(State::Running) => Some(Color32::from_rgb(0xFF, 0xB0, 0x30)),
                Some(State::Done) => Some(Color32::from_rgb(0x6C, 0xE0, 0x6C)),
                Some(State::Failed) => Some(Color32::from_rgb(0xFF, 0x60, 0x60)),
                _ => None,
            };
            if let Some(c) = lamp {
                painter.circle_filled(Pos2::new(head.min.x + 11.0 * z, head.center().y), 4.5 * z, c);
            }
            let tx = head.min.x + if lamp.is_some() { 22.0 * z } else { 10.0 * z };
            let clip = painter.with_clip_rect(Rect::from_min_max(
                Pos2::new(tx, head.min.y),
                Pos2::new(head.max.x - 26.0 * z, head.max.y),
            ));
            clip.text(
                Pos2::new(tx, head.center().y),
                egui::Align2::LEFT_CENTER,
                &title,
                font(12.5),
                Color32::WHITE,
            );
            // el nodo se mueve por su cabecera y su cuerpo; un clic lo selecciona
            let resp = ui.interact(r, egui::Id::new(("flow_node", &n.id)), Sense::click_and_drag());
            if resp.dragged() {
                moved = Some((n.id.clone(), resp.drag_delta() / z));
            }
            if resp.clicked() || resp.drag_started() {
                clicked = Some((n.id.clone(), ui.input(|i| i.modifiers.shift)));
            }
            // conservar los productos; los de un nodo final se guardan siempre
            let kept = self.flow.model.kept(n);
            let is_final = !self.flow.model.edges.iter().any(|e| e.from == n.id);
            // las entradas no producen nada que guardar
            if matches!(n.kind, NodeKind::Tool { .. }) {
                let star = Rect::from_center_size(
                    Pos2::new(head.max.x - 13.0 * z, head.center().y),
                    Vec2::splat(18.0 * z),
                );
                let sr = ui.interact(star, egui::Id::new(("flow_keep", &n.id)), Sense::click());
                painter.text(
                    star.center(),
                    egui::Align2::CENTER_CENTER,
                    if kept { "★" } else { "☆" },
                    font(15.0),
                    if kept { Color32::from_rgb(0xFF, 0xE0, 0x6A) } else { Color32::from_white_alpha(170) },
                );
                if sr.clicked() && !is_final {
                    toggle_keep = Some(n.id.clone());
                }
                sr.on_hover_text(if is_final {
                    "Resultado final del flujo: se guarda siempre"
                } else if n.keep {
                    "Se conservan los resultados de este nodo"
                } else {
                    "Conservar los resultados de este nodo (los demás se borran al acabar)"
                });
            }
            // el cuerpo
            match &n.kind {
                NodeKind::Input { path } => {
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map_or(path.clone(), |f| f.to_string_lossy().into_owned());
                    if z > 0.45 {
                        painter.with_clip_rect(r).text(
                            Pos2::new(r.min.x + 10.0 * z, r.min.y + (HEAD + 6.0 + ROW / 2.0) * z),
                            egui::Align2::LEFT_CENTER,
                            if std::path::Path::new(path).is_dir() { format!("📁 {name}") } else { name },
                            font(11.0),
                            t.weak,
                        );
                    }
                }
                NodeKind::Tool { .. } => {
                    for (k, (id, label, required, kind)) in lay.ins.iter().enumerate() {
                        let p = to_s(lay.in_pos(k));
                        let wired = self.flow.model.edges.iter().any(|e| e.to == n.id && e.port == *id);
                        let mut ring = kind_color(*kind);
                        // mientras se arrastra un hilo, solo quedan iluminados los puertos a los
                        // que puede llegar
                        if let Some(w) = &self.flow.wiring
                            && let (Some(src), NodeKind::Tool { tool, .. }) = (self.flow.model.node(w), &n.kind)
                            && !model::can_feed(src, tool, id)
                        {
                            ring = ring.gamma_multiply(0.25);
                        }
                        painter.circle_filled(p, PORT * z, if wired { ring } else { t.ribbon });
                        painter.circle_stroke(p, PORT * z, Stroke::new(1.6, ring));
                        if z > 0.45 {
                            painter.text(
                                p + Vec2::new(10.0 * z, 0.0),
                                egui::Align2::LEFT_CENTER,
                                if *required { format!("{label} *") } else { label.clone() },
                                font(10.5),
                                t.text,
                            );
                        }
                    }
                }
            }
            // el puerto de salida y el hilo que se arrastra desde él
            let o = to_s(lay.out_pos());
            let makes = match &n.kind {
                NodeKind::Tool { tool, .. } => model::entry(tool).and_then(|e| e.makes.first().copied()).unwrap_or(Kind::Any),
                NodeKind::Input { path } => {
                    let p = std::path::Path::new(path);
                    [Kind::Cloud, Kind::Raster, Kind::Vector, Kind::Table]
                        .into_iter()
                        .find(|k| k.accepts(p))
                        .unwrap_or(Kind::Any)
                }
            };
            painter.circle_filled(o, PORT * z, kind_color(makes));
            painter.circle_stroke(o, PORT * z, Stroke::new(1.6, t.ribbon));
            let pr = ui.interact(
                Rect::from_center_size(o, Vec2::splat(18.0)),
                egui::Id::new(("flow_out", &n.id)),
                Sense::drag(),
            );
            pr.clone().on_hover_text("Arrastra hasta una entrada para conectar");
            if pr.drag_started() {
                wire_from = Some(n.id.clone());
            }
            if pr.drag_stopped()
                && let (Some(from), Some(p)) = (self.flow.wiring.clone(), pointer)
                && let Some(src) = self.flow.model.node(&from)
            {
                // el puerto de entrada que hay bajo el puntero, si el hilo puede ir allí
                'ports: for (m, l2) in self.flow.model.nodes.iter().zip(&lays) {
                    let NodeKind::Tool { tool, .. } = &m.kind else { continue };
                    for (k, (id, _, _, _)) in l2.ins.iter().enumerate() {
                        if to_s(l2.in_pos(k)).distance(p) < 12.0 && m.id != from && model::can_feed(src, tool, id) {
                            connect = Some(Edge { from: from.clone(), to: m.id.clone(), port: id.clone() });
                            break 'ports;
                        }
                    }
                    // soltado sobre el nodo: la primera entrada libre que lo admite
                    if m.id != from && rect_s(l2.rect).contains(p) {
                        let free = l2.ins.iter().find(|(id, _, _, kind)| {
                            let used = self.flow.model.edges.iter().any(|e| e.to == m.id && e.port == *id);
                            !used && fits(src, tool, id, *kind)
                        });
                        if let Some((id, _, _, _)) = free {
                            connect = Some(Edge { from: from.clone(), to: m.id.clone(), port: id.clone() });
                        }
                        break 'ports;
                    }
                }
            }
        }
        // el hilo en el aire
        if let (Some(from), Some(p)) = (&self.flow.wiring, pointer)
            && let Some(&a) = index.get(from.as_str())
        {
            painter.add(egui::Shape::line(
                wire(to_s(lays[a].out_pos()), p),
                Stroke::new(2.0, t.accent),
            ));
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.pointer.any_released()) {
            self.flow.wiring = None;
        }
        if let Some(f) = wire_from {
            self.flow.wiring = Some(f);
        }
        if let Some((id, shift)) = clicked {
            self.flow.sel_edge = None;
            if shift {
                if !self.flow.selected.remove(&id) {
                    self.flow.selected.insert(id);
                }
            } else if !self.flow.selected.contains(&id) {
                self.flow.selected = BTreeSet::from([id]);
            }
        }
        if let Some((id, d)) = moved {
            if self.flow.selected.contains(&id) {
                let ids = self.flow.selected.clone();
                for n in self.flow.model.nodes.iter_mut().filter(|n| ids.contains(&n.id)) {
                    n.x += d.x;
                    n.y += d.y;
                }
            } else if let Some(n) = self.flow.model.nodes.iter_mut().find(|n| n.id == id) {
                n.x += d.x;
                n.y += d.y;
            }
        }
        if let Some(id) = toggle_keep {
            self.flow.push_undo();
            if let Some(n) = self.flow.model.nodes.iter_mut().find(|n| n.id == id) {
                n.keep = !n.keep;
            }
        }
        if let Some(e) = connect
            && !self.flow.model.edges.contains(&e)
        {
            let mut next = self.flow.model.clone();
            next.edges.push(e);
            if next.order().is_none() {
                self.flow.status = "Ese hilo cerraría un bucle.".into();
            } else {
                self.flow.push_undo();
                self.flow.model = next;
            }
        }
        if self.flow.model.nodes.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Añade una entrada y herramientas desde la paleta, y conecta sus puertos.",
                egui::FontId::proportional(14.0),
                t.weak,
            );
        }
        let _ = ctx;
    }

    /// Encuadra el flujo entero en el lienzo.
    fn flow_fit(&mut self, lays: &[Lay]) {
        let c = self.flow.canvas;
        if lays.is_empty() || !c.is_positive() {
            self.flow.zoom = 1.0;
            self.flow.pan = Vec2::new(40.0, 40.0);
            return;
        }
        let mut all = lays[0].rect;
        for l in lays {
            all = all.union(l.rect);
        }
        let z = ((c.width() - 80.0) / all.width()).min((c.height() - 80.0) / all.height()).clamp(0.3, 1.2);
        self.flow.zoom = z;
        self.flow.pan = (c.size() - all.size() * z) / 2.0 - all.min.to_vec2() * z;
    }

    /// Coloca los nodos en columnas según su distancia a las entradas.
    fn flow_arrange(&mut self, schema: &Schema) {
        self.flow.push_undo();
        let depth = self.flow.model.depths();
        let heights: Vec<f32> = layout_of(&self.flow.model, schema).iter().map(|l| l.rect.height()).collect();
        let mut next_y: HashMap<usize, f32> = HashMap::new();
        for (i, n) in self.flow.model.nodes.iter_mut().enumerate() {
            let d = depth[i];
            let y = next_y.entry(d).or_insert(0.0);
            n.x = d as f32 * (NODE_W + 70.0);
            n.y = *y;
            *y += heights[i] + 28.0;
        }
        self.flow.fit_next = true;
    }

    fn flow_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (del, dup, undo, redo) = ctx.input(|i| {
            (
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                i.modifiers.command && i.key_pressed(Key::D),
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(Key::Z),
                i.modifiers.command && (i.key_pressed(Key::Y) || (i.modifiers.shift && i.key_pressed(Key::Z))),
            )
        });
        if del {
            self.flow_delete();
        }
        if dup {
            self.flow_duplicate();
        }
        if undo {
            self.flow_undo();
        }
        if redo {
            self.flow_redo();
        }
    }

    fn flow_delete(&mut self) {
        if let Some(k) = self.flow.sel_edge.take() {
            self.flow.push_undo();
            if k < self.flow.model.edges.len() {
                self.flow.model.edges.remove(k);
            }
            return;
        }
        if self.flow.selected.is_empty() {
            return;
        }
        self.flow.push_undo();
        let ids = std::mem::take(&mut self.flow.selected);
        self.flow.model.nodes.retain(|n| !ids.contains(&n.id));
        self.flow.model.edges.retain(|e| !ids.contains(&e.from) && !ids.contains(&e.to));
        // un nodo nuevo puede reutilizar el identificador: no debe heredar sus parámetros
        self.flow.model.params.retain(|p| !ids.contains(&p.node));
    }

    fn flow_duplicate(&mut self) {
        if self.flow.selected.is_empty() {
            return;
        }
        self.flow.push_undo();
        let ids = self.flow.selected.clone();
        let mut fresh = BTreeSet::new();
        let mut map: HashMap<String, String> = HashMap::new();
        for n in self.flow.model.nodes.clone().into_iter().filter(|n| ids.contains(&n.id)) {
            let id = self.flow.model.new_id();
            map.insert(n.id.clone(), id.clone());
            let mut c = n;
            c.id = id.clone();
            c.x += 30.0;
            c.y += 30.0;
            self.flow.model.nodes.push(c);
            fresh.insert(id);
        }
        // los hilos entre los nodos copiados también se copian
        for e in self.flow.model.edges.clone() {
            if let (Some(a), Some(b)) = (map.get(&e.from), map.get(&e.to)) {
                self.flow.model.edges.push(Edge { from: a.clone(), to: b.clone(), port: e.port });
            }
        }
        self.flow.selected = fresh;
    }

    fn flow_undo(&mut self) {
        if let Some(m) = self.flow.undo.pop() {
            self.flow.redo.push(std::mem::replace(&mut self.flow.model, m));
            self.flow.selected.clear();
            self.flow.sel_edge = None;
        }
    }

    fn flow_redo(&mut self) {
        if let Some(m) = self.flow.redo.pop() {
            self.flow.undo.push(std::mem::replace(&mut self.flow.model, m));
            self.flow.selected.clear();
            self.flow.sel_edge = None;
        }
    }

    /// Las opciones del nodo seleccionado o, si no hay ninguno, las del flujo.
    fn flow_params(&mut self, ui: &mut egui::Ui, schema: &Schema) {
        let t = self.tokens();
        let sel = if self.flow.selected.len() == 1 { self.flow.selected.iter().next().cloned() } else { None };
        self.flow_output(ui);
        ui.separator();
        let Some(id) = sel else {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.flow_exposed(ui, schema);
                self.flow_settings(ui);
            });
            return;
        };
        let Some(idx) = self.flow.model.nodes.iter().position(|n| n.id == id) else {
            return;
        };
        let edges: Vec<Edge> = self.flow.model.edges.iter().filter(|e| e.to == id).cloned().collect();
        // por cada opción conectada, el archivo de entrada que la alimenta si es un nodo de entrada con
        // un archivo (de él se ven los campos); si viene de otra herramienta, aún no existe
        let inputs: std::collections::HashMap<String, Option<String>> = edges
            .iter()
            .map(|e| {
                let path = self.flow.model.nodes.iter().find(|n| n.id == e.from).and_then(|n| match &n.kind {
                    NodeKind::Input { path } if std::path::Path::new(path).is_file() => Some(path.clone()),
                    _ => None,
                });
                (e.port.clone(), path)
            })
            .collect();
        let is_final = !self.flow.model.edges.iter().any(|e| e.from == id);
        // las opciones que ya son parámetros, y lo que el panel pida cambiar en ellos
        let exposed: HashMap<String, String> = self
            .flow
            .model
            .params
            .iter()
            .filter(|p| p.node == id)
            .map(|p| (p.arg.clone(), p.name.clone()))
            .collect();
        let mut expose: Option<(String, Option<String>)> = None;
        let mut run_here = false;
        let idle = self.flow.running.is_none();
        let node = &mut self.flow.model.nodes[idx];
        let before: Option<std::collections::BTreeMap<String, Vec<String>>> = match &node.kind {
            NodeKind::Tool { args, .. } => Some(args.clone()),
            NodeKind::Input { .. } => None,
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(egui::RichText::new("Nodo").size(15.0).strong().color(t.text));
            ui.add_space(4.0);
            ui.label("Nombre");
            ui.add(egui::TextEdit::singleline(&mut node.label).hint_text("(el de la herramienta)").desired_width(f32::INFINITY));
            if is_final {
                ui.label(egui::RichText::new("Resultado final: se guarda siempre.").size(11.5).color(t.weak));
            } else {
                ui.checkbox(&mut node.keep, "Conservar los resultados")
                    .on_hover_text("Los resultados de un nodo que no se conserva se borran cuando nadie los necesita.");
            }
            ui.add_space(6.0);
            match &mut node.kind {
                NodeKind::Input { path } => {
                    ui.label("Archivo o carpeta");
                    ui.add(egui::TextEdit::singleline(path).desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        if ui.button("Archivo…").clicked()
                            && let Some(f) = prefs::dialog().pick_file()
                        {
                            *path = f.display().to_string();
                        }
                        if ui.button("Carpeta…").clicked()
                            && let Some(d) = rfd::FileDialog::new().pick_folder()
                        {
                            *path = d.display().to_string();
                        }
                    });
                    ui.label(
                        egui::RichText::new("Una carpeta entrega todos sus archivos, como una entrega de hojas.")
                            .size(11.5)
                            .color(t.weak),
                    );
                }
                NodeKind::Tool { tool, args } => {
                    let Some(spec) = schema.tool(tool) else {
                        ui.label("Herramienta desconocida.");
                        return;
                    };
                    if let Some(e) = model::entry(tool) {
                        ui.label(egui::RichText::new(e.title).strong());
                    }
                    if ui
                        .add_enabled(idle, egui::Button::new("Ejecutar hasta aquí"))
                        .on_hover_text("Ejecuta este nodo y los que lo alimentan; su resultado se guarda.")
                        .clicked()
                    {
                        run_here = true;
                    }
                    ui.label(egui::RichText::new(&spec.about).size(11.5).color(t.weak));
                    ui.add_space(8.0);
                    let wired = |a: &str| edges.iter().any(|e| e.port == a);
                    // primero lo necesario y lo conectado; el resto, plegado
                    // una opción de un método o de una casilla solo sale cuando se usa, como en las
                    // ventanas de cada herramienta del programa
                    let main: Vec<&model::ArgSpec> = spec
                        .args
                        .iter()
                        .filter(|a| a.id != "output" && a.id != "results_dir")
                        .filter(|a| a.required || wired(&a.id))
                        .collect();
                    let more: Vec<&model::ArgSpec> = spec
                        .args
                        .iter()
                        .filter(|a| a.id != "output" && a.id != "results_dir")
                        .filter(|a| !(a.required || wired(&a.id)))
                        .filter(|a| spec.shown(a, args))
                        .collect();
                    // bajo cada opción que no llega por un hilo, si es un parámetro del flujo
                    let mut parameter = |ui: &mut egui::Ui, a: &model::ArgSpec| {
                        let mut on = exposed.contains_key(&a.id);
                        let text = egui::RichText::new("Parámetro del flujo").size(10.5).color(t.weak);
                        if ui
                            .checkbox(&mut on, text)
                            .on_hover_text(
                                "Se puede cambiar sin abrir el nodo. Las opciones con el mismo nombre de parámetro cambian a la vez.",
                            )
                            .changed()
                        {
                            let name = on.then(|| a.long.clone().unwrap_or_else(|| a.id.clone()));
                            expose = Some((a.id.clone(), name));
                        }
                        if let Some(name) = exposed.get(&a.id) {
                            // se aplica al salir del cuadro: a medio escribir, un nombre puede
                            // coincidir con el de otro parámetro y tomar su valor
                            let key = egui::Id::new(("flow_param_name", &id, &a.id));
                            let mut text = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| name.clone());
                            let r = ui
                                .add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY))
                                .on_hover_text("Nombre del parámetro");
                            if r.changed() {
                                ui.data_mut(|d| d.insert_temp(key, text.clone()));
                            }
                            if r.lost_focus() {
                                ui.data_mut(|d| d.remove::<String>(key));
                                let text = text.trim().to_string();
                                if !text.is_empty() && text != *name {
                                    expose = Some((a.id.clone(), Some(text)));
                                }
                            }
                        }
                    };
                    for a in main {
                        arg_widget(ui, &t, tool, a, args, wired(&a.id), &inputs);
                        if !wired(&a.id) {
                            parameter(ui, a);
                        }
                    }
                    if !more.is_empty() {
                        egui::CollapsingHeader::new(format!("Más opciones ({})", more.len()))
                            .id_salt(("flow_more", &id))
                            .show(ui, |ui| {
                                for a in more {
                                    arg_widget(ui, &t, tool, a, args, false, &inputs);
                                    parameter(ui, a);
                                }
                            });
                    }
                }
            }
        });
        // lo escrito en una opción que es parámetro pasa a las que lo comparten
        if let Some(before) = before
            && let Some(NodeKind::Tool { args, .. }) = self.flow.model.node(&id).map(|n| &n.kind)
        {
            let changed: Vec<String> = self
                .flow
                .model
                .params
                .iter()
                .filter(|p| p.node == id && args.get(&p.arg) != before.get(&p.arg))
                .map(|p| p.arg.clone())
                .collect();
            for a in changed {
                self.flow.model.share(&id, &a);
            }
        }
        match expose {
            Some((arg, Some(name))) => {
                self.flow.push_undo();
                // al renombrar, el valor del nodo es el que vale si el nombre es nuevo
                self.flow.model.expose(&id, &arg, &name);
            }
            Some((arg, None)) => {
                self.flow.push_undo();
                self.flow.model.unexpose(&id, &arg);
            }
            None => {}
        }
        if run_here {
            let ctx = ui.ctx().clone();
            let schema = self.flow.schema.clone();
            if let Some(Ok(schema)) = schema {
                self.flow.until = Some(id);
                self.flow_run(&ctx, schema, false);
            }
        }
    }

    /// Los parámetros del flujo: las entradas y las opciones expuestas, juntas y editables.
    fn flow_exposed(&mut self, ui: &mut egui::Ui, schema: &Schema) {
        let t = self.tokens();
        let m = &mut self.flow.model;
        let before = m.clone();
        let names: Vec<String> = m.parameter_names().iter().map(|s| s.to_string()).collect();
        let inputs: Vec<usize> = (0..m.nodes.len())
            .filter(|&i| matches!(m.nodes[i].kind, NodeKind::Input { .. }))
            .collect();
        if names.is_empty() && inputs.is_empty() {
            return;
        }
        ui.label(egui::RichText::new("Parámetros").size(15.0).strong().color(t.text));
        ui.add_space(4.0);
        for i in inputs {
            let title = m.name(&m.nodes[i]);
            ui.label(title);
            let id = m.nodes[i].id.clone();
            if let NodeKind::Input { path } = &mut m.nodes[i].kind {
                ui.push_id(("flow_input", id), |ui| {
                    ui.add(egui::TextEdit::singleline(path).desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        if ui.button("Archivo…").clicked()
                            && let Some(f) = prefs::dialog().pick_file()
                        {
                            *path = f.display().to_string();
                        }
                        if ui.button("Carpeta…").clicked()
                            && let Some(d) = rfd::FileDialog::new().pick_folder()
                        {
                            *path = d.display().to_string();
                        }
                    });
                });
            }
        }
        for name in names {
            let Some((node, arg)) = m
                .parameter_targets(&name)
                .next()
                .map(|(n, a)| (n.id.clone(), a.to_string()))
            else {
                continue;
            };
            let places: Vec<String> = m.parameter_targets(&name).map(|(n, _)| m.name(n)).collect();
            let Some(n) = m.nodes.iter_mut().find(|n| n.id == node) else { continue };
            let NodeKind::Tool { tool, args } = &mut n.kind else { continue };
            let Some(a) = schema.tool(tool).and_then(|s| s.arg(&arg)) else { continue };
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&name).strong())
                .on_hover_text(format!("En: {}", places.join(", ")));
            ui.push_id(("flow_param", &name), |ui| {
                arg_widget(ui, &t, tool, a, args, false, &HashMap::new());
            });
            if m.value(&node, &arg) != before.value(&node, &arg) {
                m.share(&node, &arg);
            }
        }
        if self.flow.model != before {
            self.flow.push_undo_typing(&before);
        }
        ui.separator();
    }

    /// La salida del flujo, siempre a la vista: la carpeta, qué se guarda y si se abre en el mapa.
    fn flow_output(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        ui.label(egui::RichText::new("Salida").size(15.0).strong().color(t.text));
        ui.add_space(4.0);
        ui.label("Carpeta de resultados");
        ui.add(
            egui::TextEdit::singleline(&mut self.flow.model.results)
                .hint_text("(resultados_flujo, junto a la primera entrada)")
                .desired_width(f32::INFINITY),
        );
        if ui.button("Elegir carpeta…").clicked()
            && let Some(d) = rfd::FileDialog::new().pick_folder()
        {
            self.flow.model.results = d.display().to_string();
        }
        ui.label(
            egui::RichText::new("Cada ejecución crea dentro su propia carpeta: no se sobrescribe nada.")
                .size(11.5)
                .color(t.weak),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Guardar:");
            // los dos botones solo ponen las estrellas, que siguen decidiendo nodo a nodo
            let finals = ui.button("Solo finales").on_hover_text("Guarda solo los nodos de los que no sale ningún hilo.");
            let all = ui.button("Todo").on_hover_text("Conserva también los resultados intermedios.");
            if finals.clicked() || all.clicked() {
                self.flow.push_undo();
                for n in &mut self.flow.model.nodes {
                    n.keep = all.clicked() && matches!(n.kind, NodeKind::Tool { .. });
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Teselas a la vez:");
            ui.add(
                egui::DragValue::new(&mut self.flow.jobs)
                    .range(0..=64)
                    .custom_formatter(|v, _| if v < 1.0 { "automático".into() } else { format!("{v:.0}") }),
            )
            .on_hover_text(
                "Automático: según los núcleos, la memoria y el tamaño de los archivos. En un disco lento, o si falta memoria, pon 1.",
            );
        });
        ui.checkbox(&mut self.flow.add_to_map, "Abrir los resultados en el mapa al terminar");
        if ui
            .add_enabled(self.flow.running.is_none(), egui::Button::new("Reanudar la última ejecución"))
            .on_hover_text(
                "Sigue donde se quedó tras un corte, una cancelación o unos fallos: lo ya hecho no se repite. \
                 Vale mientras no cambien las opciones ni los hilos.",
            )
            .clicked()
            && let Some(Ok(schema)) = self.flow.schema.clone()
        {
            let ctx = ui.ctx().clone();
            self.flow_run(&ctx, schema, true);
        }
    }

    /// Los datos del flujo, sin ningún nodo seleccionado.
    fn flow_settings(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        ui.label(egui::RichText::new("Flujo").size(15.0).strong().color(t.text));
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Selecciona un nodo para ver sus opciones. Las estrellas marcan los resultados que se conservan; \
                 los de los demás nodos se borran cuando ya nadie los necesita.",
            )
            .size(11.5)
            .color(t.weak),
        );
        ui.add_space(10.0);
        ui.label(egui::RichText::new(format!("{} nodos, {} hilos", self.flow.model.nodes.len(), self.flow.model.edges.len())).color(t.weak));
        if let Some(p) = &self.flow.path {
            ui.label(egui::RichText::new(format!("Guardado en {}", p.display())).size(11.5).color(t.weak));
        }
    }

    /// Lo que la cinta pide a la vista de flujos.
    pub fn flow_action(&mut self, ctx: &egui::Context, a: Action) {
        self.flow_start();
        let Some(Ok(schema)) = self.flow.schema.clone() else { return };
        match a {
            Action::FlowNew => {
                if self.flow.running.is_some() {
                    return;
                }
                self.flow.push_undo();
                self.flow.model = Model::default();
                self.flow.path = None;
                self.flow.selected.clear();
                self.flow.states.clear();
            }
            Action::FlowOpen => {
                if let Some(f) = prefs::dialog().add_filter("Flujo de HIPSO", &["json"]).pick_file() {
                    match std::fs::read_to_string(&f)
                        .map_err(|e| e.to_string())
                        .and_then(|t| Model::from_json(&t).map_err(|e| e.to_string()))
                    {
                        Ok(m) => {
                            self.flow.push_undo();
                            self.flow.model = m;
                            self.flow.path = Some(f);
                            self.flow.selected.clear();
                            self.flow.states.clear();
                            self.flow.fit_next = true;
                            self.flow_place();
                        }
                        Err(e) => self.notice = Some(format!("No se pudo abrir el flujo: {e}")),
                    }
                }
            }
            Action::FlowSave | Action::FlowSaveAs => {
                let target = match (&self.flow.path, a) {
                    (Some(p), Action::FlowSave) => Some(p.clone()),
                    _ => prefs::dialog()
                        .add_filter("Flujo de HIPSO", &["json"])
                        .set_file_name("flujo.model.json")
                        .save_file(),
                };
                if let Some(p) = target {
                    match hipso_core::paths::write_atomic(&p, self.flow.model.to_json()) {
                        Ok(()) => {
                            self.flow.status = format!("Guardado en {}", p.display());
                            self.flow.path = Some(p);
                        }
                        Err(e) => self.notice = Some(format!("No se pudo guardar el flujo: {e}")),
                    }
                }
            }
            Action::FlowExample(k) => {
                if self.flow.running.is_some() {
                    return;
                }
                if let Some((_, text)) = EXAMPLES.get(usize::from(k))
                    && let Ok(m) = Model::from_json(text)
                {
                    self.flow.push_undo();
                    self.flow.model = m;
                    self.flow.path = None;
                    self.flow.selected.clear();
                    self.flow.states.clear();
                    self.flow.fit_next = true;
                    self.flow_place();
                }
            }
            Action::FlowUndo => self.flow_undo(),
            Action::FlowRedo => self.flow_redo(),
            Action::FlowDelete => self.flow_delete(),
            Action::FlowDuplicate => self.flow_duplicate(),
            Action::FlowArrange => self.flow_arrange(&schema),
            Action::FlowFit => self.flow.fit_next = true,
            Action::FlowRun => {
                self.flow.until = None;
                self.flow_run(ctx, schema, false);
            }
            Action::FlowCancel => {
                if let Some(r) = &self.flow.running {
                    r.cancel.store(true, Ordering::Relaxed);
                    self.flow.status = "Cancelando…".into();
                }
            }
            _ => {}
        }
    }

    fn flow_run(&mut self, ctx: &egui::Context, schema: Arc<Schema>, resume: bool) {
        if self.flow.running.is_some() {
            return;
        }
        // la ejecución parcial, y su reanudación, solo miran los nodos que hacen falta
        let model = match &self.flow.until {
            Some(id) => match self.flow.model.upto(id) {
                Ok(m) => m,
                Err(_) => {
                    self.flow.until = None;
                    self.flow.model.clone()
                }
            },
            None => self.flow.model.clone(),
        };
        let problems = model.problems(&schema);
        if let Some(p) = problems.first() {
            self.notice = Some(format!("El flujo no se puede ejecutar: {}", p.text));
            return;
        }
        let Some(exe) = hipso_exe() else {
            self.notice = Some("No se encuentra el ejecutable «hipso» junto al programa.".into());
            return;
        };
        // la carpeta de resultados es la del flujo entero
        let results = results_of(&self.flow.model);
        let jobs = self.flow.jobs;
        self.flow.states.clear();
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let inputs = HashMap::new();
            let run = model::Run { exe: &exe, schema: &schema, results: &results, inputs: &inputs, cancel: &flag, resume, jobs };
            let (tx_state, tx_log) = (tx.clone(), tx.clone());
            let (c1, c2) = (c.clone(), c.clone());
            let r = model.run(
                &run,
                &mut |id, st| {
                    let _ = tx_state.send(Msg::State(id.to_string(), st));
                    c1.request_repaint();
                },
                &mut |l| {
                    let _ = tx_log.send(Msg::Log(l.to_string()));
                    c2.request_repaint();
                },
            );
            let _ = tx.send(Msg::Done(r.map_err(|e| e.to_string())));
            c.request_repaint();
        });
        self.flow.running = Some(Running { rx, cancel });
        self.flow.status = "Ejecutando el flujo…".into();
        self.log("Flujo: en marcha".to_string());
    }

    /// Recoge lo que ha dicho el flujo en ejecución desde el último fotograma.
    fn flow_poll(&mut self, ctx: &egui::Context) {
        let Some(run) = &mut self.flow.running else { return };
        let mut msgs = Vec::new();
        while let Ok(m) = run.rx.try_recv() {
            msgs.push(m);
        }
        let mut done = None;
        for m in msgs {
            match m {
                Msg::Log(l) => {
                    // un flujo de miles de teselas no puede crecer sin fin en memoria: el
                    // registro entero queda en la carpeta de la ejecución
                    if self.log.len() >= MAX_LOG {
                        self.log.drain(..MAX_LOG / 2);
                    }
                    self.log.push(l);
                }
                Msg::State(id, st) => {
                    if st == State::Running {
                        let name = self
                            .flow
                            .model
                            .node(&id)
                            .map_or(id.clone(), |n| self.flow.model.name(n));
                        self.flow.status = format!("Ejecutando «{name}»…");
                    }
                    self.flow.states.insert(id, st);
                }
                Msg::Done(r) => done = Some(r),
            }
        }
        if self.flow.states.values().any(|s| *s == State::Running) {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let Some(r) = done else { return };
        self.flow.running = None;
        match r {
            Ok(report) => {
                let mut open = Vec::new();
                for (_, files) in &report.outputs {
                    self.log(format!("Flujo: {} archivos conservados", files.len()));
                    for f in files {
                        if [Kind::Cloud, Kind::Raster, Kind::Vector].iter().any(|k| k.accepts(f)) {
                            open.push(f.clone());
                        }
                    }
                }
                let folder = report.folder.display();
                if report.failed > 0 {
                    self.flow.status = format!(
                        "Terminado con {} fallos (errores.csv en {folder}). El primero: {}",
                        report.failed, report.first_error
                    );
                    self.show_log = true;
                } else {
                    self.flow.status = format!("Flujo terminado: {folder}");
                }
                self.log(format!("Flujo: {}", self.flow.status));
                // abrir miles de capas bloquearía el programa
                if self.flow.add_to_map && !open.is_empty() {
                    if open.len() <= MAX_TO_MAP {
                        self.open_files(ctx, open);
                    } else {
                        self.log(format!(
                            "Flujo: son {} archivos; no se abren en el mapa (más de {MAX_TO_MAP})",
                            open.len()
                        ));
                    }
                }
            }
            Err(e) => {
                // en una cadena hay varios nodos en marcha: ninguno se queda así
                for s in self.flow.states.values_mut() {
                    if *s == State::Running {
                        *s = State::Failed;
                    }
                }
                self.log(format!("Flujo: {e}"));
                self.flow.status = format!("El flujo se detuvo (se puede reanudar): {e}");
                self.show_log = true;
            }
        }
    }

    /// El esquema de las órdenes de `hipso`, cargado la primera vez que se pide.
    pub fn tool_schema(&mut self) -> Option<Arc<Schema>> {
        self.flow_start();
        match &self.flow.schema {
            Some(Ok(s)) => Some(s.clone()),
            _ => None,
        }
    }

    /// Abre en la vista de flujos un flujo guardado en un archivo (el que crea el asistente) y
    /// lleva a esa vista.
    pub fn flow_load_path(&mut self, path: &std::path::Path) {
        self.flow_start();
        if self.flow.running.is_some() {
            self.notice = Some("Hay un flujo en ejecución: espera a que termine.".into());
            return;
        }
        match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| Model::from_json(&t).map_err(|e| e.to_string()))
        {
            Ok(m) => {
                self.flow.push_undo();
                self.flow.model = m;
                self.flow.path = Some(path.to_path_buf());
                self.flow.selected.clear();
                self.flow.states.clear();
                self.flow.fit_next = true;
                self.flow_place();
                self.backstage = false;
                self.tab = FLOWS_TAB;
            }
            Err(e) => self.notice = Some(format!("No se pudo abrir el flujo: {e}")),
        }
    }

    /// Si hay un flujo en ejecución (la cinta muestra entonces Cancelar).
    pub fn flow_running(&self) -> bool {
        self.flow.running.is_some()
    }

    pub fn flow_can_undo(&self) -> (bool, bool) {
        (!self.flow.undo.is_empty(), !self.flow.redo.is_empty())
    }
}

/// El componente de una opción de una herramienta: una casilla, una lista, una ruta o un texto.
pub(crate) fn arg_widget(
    ui: &mut egui::Ui,
    t: &theme::Tokens,
    tool: &str,
    a: &model::ArgSpec,
    args: &mut std::collections::BTreeMap<String, Vec<String>>,
    wired: bool,
    inputs: &std::collections::HashMap<String, Option<String>>,
) {
    let name = label_es(tool, &a.id)
        .map(str::to_string)
        .unwrap_or_else(|| a.long.clone().map_or(a.id.clone(), |l| if a.positional { l } else { format!("--{l}") }));
    let title = if a.required { format!("{name} *") } else { name };
    let hint = a.default.clone().unwrap_or_default();
    ui.add_space(4.0);
    if a.flag {
        let mut on = args.get(&a.id).is_some_and(|v| v.first().is_some_and(|s| s == "true"));
        if ui.checkbox(&mut on, title).on_hover_text(&a.help).changed() {
            if on {
                args.insert(a.id.clone(), vec!["true".into()]);
            } else {
                args.remove(&a.id);
            }
        }
        return;
    }
    // una opción que nombra un campo: la lista de los campos del archivo de entrada de la herramienta,
    // escrito en ella o recibido de un nodo de entrada; si el archivo aún no existe (lo hace otro
    // nodo), el cuadro de texto
    if !wired
        && let Some(fs) = model::field_source(tool, &a.id)
    {
        let source = match inputs.get(fs.source) {
            Some(path) => path.clone(),
            None => args.get(fs.source).and_then(|v| v.first()).cloned(),
        };
        let upstream_pending = inputs.get(fs.source).is_some_and(Option::is_none);
        if !upstream_pending {
            let source_label = label_es(tool, fs.source).unwrap_or(fs.source).to_lowercase();
            if crate::fieldpick::row(ui, t, tool, a, &title, fs, &source_label, source.as_deref(), args) {
                return;
            }
        }
    }
    ui.label(title).on_hover_text(&a.help);
    if wired {
        ui.label(egui::RichText::new("Recibe lo que le llega por el hilo.").size(11.5).color(t.weak));
        return;
    }
    if !a.values.is_empty() {
        let now = args.get(&a.id).and_then(|v| v.first().cloned()).unwrap_or_default();
        let shown = if now.is_empty() { format!("(por defecto: {hint})") } else { now.clone() };
        egui::ComboBox::from_id_salt(("flow_arg", tool, &a.id))
            .selected_text(shown)
            .width(ui.available_width() - 8.0)
            .show_ui(ui, |ui| {
                if ui.selectable_label(now.is_empty(), "(por defecto)").clicked() {
                    args.remove(&a.id);
                }
                for v in &a.values {
                    if ui.selectable_label(*v == now, v).clicked() {
                        args.insert(a.id.clone(), vec![v.clone()]);
                    }
                }
            });
    } else {
        let mut text = args.get(&a.id).map(|v| v.join("\n")).unwrap_or_default();
        let r = if a.multiple && !a.path {
            ui.add(egui::TextEdit::multiline(&mut text).desired_rows(2).hint_text("uno por línea").desired_width(f32::INFINITY))
        } else {
            ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(if a.path { ui.available_width() - 40.0 } else { f32::INFINITY }))
        };
        let mut changed = r.changed();
        if a.path
            && ui.small_button("…").on_hover_text("Elegir un archivo").clicked()
            && let Some(f) = prefs::dialog().pick_file()
        {
            text = f.display().to_string();
            changed = true;
        }
        if changed {
            if text.trim().is_empty() {
                args.remove(&a.id);
            } else {
                args.insert(a.id.clone(), text.lines().map(str::to_string).collect());
            }
        }
    }
    if !a.help.is_empty() {
        ui.label(egui::RichText::new(a.help.lines().next().unwrap_or("")).size(10.5).color(t.weak));
    }
}

/// El nombre en español de una opción de las herramientas de la ventana genérica (las demás
/// muestran el nombre de la línea de comandos).
pub(crate) fn label_es(tool: &str, id: &str) -> Option<&'static str> {
    if !crate::cli_tool_ui::TOOLS.iter().any(|t| t.0 == tool) {
        return None;
    }
    Some(match id {
        "layer" | "input" => if tool == "rasterize" || tool == "extract" || tool == "join-table" { "Capa" } else { "Entrada" },
        "inputs" => "Capas",
        "raster" => "Ráster",
        "bands" => "Bandas (una por línea; vacío: todas)",
        "bilinear" => "Interpolación bilineal",
        "prefix" => "Prefijo de los campos nuevos",
        "output" => "Salida (vacío: junto a la entrada)",
        "table" => "Tabla",
        "layer_key" => "Campo clave de la capa",
        "table_key" => "Columna clave de la tabla",
        "origin" => "Campo con el archivo de origen",
        "field" => if tool == "split" { "Campo por el que dividir" } else { "Campo con el valor" },
        "dir" => "Carpeta de salida",
        "mask" => "Polígonos",
        "erase" => "Borrar lo que cubren en lugar de conservarlo",
        "to" => "Capa de la que buscar el elemento más cercano",
        "carry" => "Campo del elemento cercano que copiar",
        "max" => "Distancia máxima",
        "by" => "Agrupar por el campo",
        "fields" => "Campos que resumir (uno por línea; vacío: todos los numéricos)",
        "reference" => "Ráster de referencia (sus celdas)",
        "cell" => "Tamaño de celda",
        "stat" => "Estadístico (mean, median, min, max, sum, range, std, majority, gaussian)",
        "radius" => "Radio de la ventana (celdas)",
        "circle" => "Ventana circular",
        "sigma" => "Sigma del filtro gaussiano (celdas)",
        "min_valid" => "Mínimo de celdas con valor en la ventana",
        "bins" => "Intervalos del histograma",
        "zones" => "Ráster de zonas",
        "values" => "Ráster de clases",
        _ => return None,
    })
}

/// Los flujos de ejemplo que trae el programa: nombre y contenido (`docs/recetas/flujos`).
pub const EXAMPLES: [(&str, &str); 6] = [
    ("MDT, CHM y árboles", include_str!("../../../docs/recetas/flujos/mdt-chm-arboles.model.json")),
    ("MDT y derivados del terreno", include_str!("../../../docs/recetas/flujos/terreno-derivados.model.json")),
    ("Volumen de acopios", include_str!("../../../docs/recetas/flujos/acopios.model.json")),
    ("Inventario por área", include_str!("../../../docs/recetas/flujos/inventario-por-area.model.json")),
    ("Cambio entre dos fechas", include_str!("../../../docs/recetas/flujos/cambio-entre-fechas.model.json")),
    ("Estadística por rodales", include_str!("../../../docs/recetas/flujos/rodales-estadistica.model.json")),
];
