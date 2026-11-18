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

/// El párrafo de ayuda de la ventana de cada herramienta: qué hace, qué espera y qué escribe.
fn help_for(name: &str) -> &'static str {
    match name {
        "extract" => "Añade a una capa un campo por cada banda de uno o varios rásteres, con el valor que hay bajo cada elemento: el de la celda bajo un punto, o el estadístico de las celdas que cubre una línea o un polígono. Sirve, por ejemplo, para saber el valor del modelo de alturas (CHM) en cada parcela o en cada árbol. Los campos nuevos llevan el prefijo que se escriba y el número de banda. Escribe una capa nueva; la original no se toca.",
        "join-table" => "Une a una capa las columnas de una tabla (CSV, texto o Excel) comparando un campo clave de la capa con una columna de la tabla. Sirve, por ejemplo, para pasar a las parcelas el inventario de campo. Los elementos sin pareja quedan con los campos nuevos vacíos. Escribe una capa nueva; la original no se toca.",
        "merge" => "Junta varias capas del mismo tipo de geometría (todas de puntos, de líneas o de polígonos) en una sola. Los campos de la capa resultante son la unión de los de todas; lo que una capa no tiene queda vacío. Si se pide, se añade un campo con el nombre del archivo de origen de cada elemento.",
        "split" => "Escribe una capa por cada valor distinto de un campo: un archivo por rodal, por especie, por municipio… Los archivos se guardan en la carpeta indicada, con el valor en el nombre, y se abren en el mapa al terminar.",
        "clip-vector" => "Recorta una capa de puntos, líneas o polígonos con los polígonos de otra: se queda con lo que cae dentro o, si se marca la casilla, borra lo que cubren y deja el resto. Las líneas y los polígonos se cortan por el borde. Escribe una capa nueva; la original no se toca.",
        "near" => "Añade a cada elemento de una capa la distancia al elemento más cercano de otra capa, el número de orden de ese elemento y, si se pide, uno de sus campos. Con una distancia máxima, los elementos sin nada a su alcance quedan vacíos. Las distancias van en las unidades del mapa.",
        "summary" => "Calcula el recuento, la suma, la media, el mínimo, el máximo y la desviación típica de los campos numéricos de una capa o tabla, con un grupo por cada valor de un campo si se pide agrupar. Escribe una tabla CSV.",
        "rasterize" => "Convierte una capa vectorial en un ráster: cada celda recibe el valor de un campo del último elemento que la cubre, o 1 si no se elige campo. El tamaño de celda se da a mano o se toma de un ráster de referencia, cuyas celdas y sistema de coordenadas pasan a ser los del resultado. Escribe un GeoTIFF.",
        "focal" => "Calcula para cada celda un estadístico de las celdas de su alrededor: media, mediana, mínimo, máximo, suma, rango, desviación, mayoría o suavizado gaussiano. La ventana es un cuadrado o un círculo del radio indicado, en celdas, y se puede exigir un mínimo de celdas con valor. Sirve para suavizar un modelo, quitar ruido o resumir una clasificación. Escribe un GeoTIFF con las celdas del original.",
        "rstats" => "Calcula el recuento, el mínimo, el máximo, la media, la desviación, la mediana, los percentiles y un histograma de las celdas con valor de un ráster, banda por banda. El resultado sale en el registro y como tabla CSV. No modifica el ráster.",
        "tabulate" => "Calcula el área de cada clase de un ráster dentro de cada zona de otro: por ejemplo, la superficie de cada tipo de cobertura en cada municipio. Los dos rásteres deben tener las mismas celdas. Escribe una tabla CSV con una fila por zona y una columna por clase.",
        _ => "",
    }
}

/// Los nombres cortos de las opciones en la ventana: la etiqueta va junto al control, como en las
/// demás ventanas, así que no caben frases; lo que explica una opción está en su ayuda de debajo.
fn short_label(tool: &str, id: &str) -> Option<String> {
    let t = match id {
        "layer" | "input" => {
            if matches!(tool, "rasterize" | "extract" | "join-table") { "Capa" } else { "Entrada" }
        }
        "inputs" => "Capas",
        "raster" => "Ráster",
        "bands" => "Bandas",
        "bilinear" => "Interpolación bilineal",
        "prefix" => "Prefijo de los campos",
        "table" => "Tabla",
        "layer_key" => "Campo clave",
        "table_key" => "Columna clave",
        "origin" => "Campo de origen",
        "field" => if tool == "split" { "Campo de división" } else { "Campo del valor" },
        "mask" => "Polígonos",
        "erase" => "Borrar lo que cubren",
        "to" => "Capa destino",
        "carry" => "Campo a copiar",
        "max" => "Distancia máxima",
        "by" => "Agrupar por",
        "fields" => "Campos",
        "reference" => "Ráster de referencia",
        "cell" => "Tamaño de celda",
        "stat" => "Estadístico",
        "radius" => "Radio (celdas)",
        "circle" => "Ventana circular",
        "sigma" => "Sigma (celdas)",
        "min_valid" => "Mínimo de celdas",
        "bins" => "Intervalos",
        "zones" => "Ráster de zonas",
        "values" => "Ráster de clases",
        _ => return None,
    };
    Some(t.to_string())
}

/// Las opciones cuyo valor es uno de una lista corta aunque la orden no lo declare como tal.
fn known_values(tool: &str, id: &str) -> Vec<String> {
    match (tool, id) {
        ("focal", "stat") => ["mean", "median", "min", "max", "sum", "range", "std", "majority", "gaussian"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        _ => Vec::new(),
    }
}

/// Las opciones que la ventana no enseña: la salida la decide «Guardar en» (la carpeta de resultados)
/// y los nombres de archivo los pone el programa.
fn hidden(a: &model::ArgSpec) -> bool {
    matches!(a.id.as_str(), "results_dir" | "output" | "dir")
}

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

/// Una opción que no es un archivo, con el aspecto de los ajustes de las demás ventanas: la etiqueta y
/// el control en la misma fila, el control con un ancho a su medida (nunca el de toda la ventana) y la
/// ayuda entera debajo, en gris.
fn cli_option_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    tool: &str,
    a: &model::ArgSpec,
    args: &mut BTreeMap<String, Vec<String>>,
) {
    let name = short_label(tool, &a.id).unwrap_or_else(|| {
        a.long.clone().map_or(a.id.clone(), |l| if a.positional { l } else { format!("--{l}") })
    });
    let title = if a.required { format!("{name} *") } else { name };
    ui.add_space(4.0);
    let help = |ui: &mut egui::Ui| {
        if !a.help.is_empty() {
            ui.label(egui::RichText::new(&a.help).size(11.0).color(t.weak));
        }
    };
    if a.flag {
        let mut on = args.get(&a.id).is_some_and(|v| v.first().is_some_and(|s| s == "true"));
        if ui.checkbox(&mut on, title).on_hover_text(&a.help).changed() {
            if on {
                args.insert(a.id.clone(), vec!["true".into()]);
            } else {
                args.remove(&a.id);
            }
        }
        help(ui);
        return;
    }
    let hint = a.default.clone().unwrap_or_default();
    let mut values = a.values.clone();
    if values.is_empty() {
        values = known_values(tool, &a.id);
    }
    let mut text = args.get(&a.id).map(|v| v.join("\n")).unwrap_or_default();
    if !values.is_empty() {
        let now = args.get(&a.id).and_then(|v| v.first().cloned()).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(&title).on_hover_text(&a.help);
            let shown = if now.is_empty() { format!("(por defecto: {hint})") } else { now.clone() };
            egui::ComboBox::from_id_salt(("cli_arg", tool, &a.id))
                .selected_text(shown)
                .width(190.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(now.is_empty(), "(por defecto)").clicked() {
                        args.remove(&a.id);
                    }
                    for v in &values {
                        if ui.selectable_label(*v == now, v).clicked() {
                            args.insert(a.id.clone(), vec![v.clone()]);
                        }
                    }
                });
        });
        help(ui);
        return;
    }
    let numeric = hint.trim().parse::<f64>().is_ok()
        || matches!(a.id.as_str(), "cell" | "radius" | "sigma" | "min_valid" | "bins" | "max");
    let mut changed = false;
    if a.multiple {
        ui.label(&title).on_hover_text(&a.help);
        changed = ui
            .add(egui::TextEdit::multiline(&mut text).desired_rows(3).hint_text("uno por línea").desired_width(330.0))
            .changed();
    } else {
        ui.horizontal(|ui| {
            ui.label(&title).on_hover_text(&a.help);
            changed = ui
                .add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text(hint)
                        .desired_width(if numeric { 90.0 } else { 260.0 }),
                )
                .changed();
        });
    }
    if changed {
        if text.trim().is_empty() {
            args.remove(&a.id);
        } else {
            args.insert(a.id.clone(), text.lines().map(str::to_string).collect());
        }
    }
    help(ui);
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

    /// Una opción de archivo con el aspecto del resto de las ventanas: su nombre y su ayuda, el
    /// archivo (o la lista de archivos, si admite varios), «Archivo…» y las capas cargadas.
    #[allow(clippy::too_many_arguments)]
    pub fn cli_file_row(
        &self,
        ui: &mut egui::Ui,
        t: &Tokens,
        tool: &str,
        a: &model::ArgSpec,
        kind: Kind,
        files: &[(String, PathBuf)],
        args: &mut BTreeMap<String, Vec<String>>,
    ) {
        let name = short_label(tool, &a.id).unwrap_or_else(|| {
            a.long.clone().map_or(a.id.clone(), |l| if a.positional { l } else { format!("--{l}") })
        });
        ui.add_space(4.0);
        let none = match kind {
            Kind::Raster => "No hay rásteres cargados",
            Kind::Vector => "No hay capas vectoriales cargadas",
            Kind::Cloud => "No hay nubes cargadas",
            _ => "No hay capas cargadas",
        };
        let (filter_name, exts): (&str, Vec<&str>) = match kind {
            Kind::Raster => ("Ráster", hipso_core::raster::EXTENSIONS.to_vec()),
            Kind::Vector => ("Capa vectorial", vec!["shp", "geojson", "json", "gpkg", "kml", "kmz"]),
            Kind::Cloud => ("Nube de puntos", vec!["las", "laz"]),
            Kind::Table => ("Tabla", vec!["csv", "txt", "tsv", "xlsx", "xls"]),
            _ => ("Archivo", Vec::new()),
        };
        let pick = |multiple: bool| -> Vec<PathBuf> {
            let mut d = crate::prefs::dialog();
            if !exts.is_empty() {
                d = d.add_filter(filter_name, &exts);
            }
            if multiple { d.pick_files().unwrap_or_default() } else { d.pick_file().into_iter().collect() }
        };
        let mut chosen: Vec<PathBuf> = Vec::new();
        let mut clear = false;
        let mut remove: Option<usize> = None;
        let current = args.get(&a.id).cloned().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(if a.required { format!("{name} *") } else { name.clone() }).on_hover_text(&a.help);
            if a.multiple {
                if ui.button("Añadir…").clicked() {
                    chosen = pick(true);
                }
            } else {
                let shown = current.first().map_or("(sin elegir)".to_string(), |p| hipso_core::paths::stem(Path::new(p)));
                ui.label(shown).on_hover_text(current.first().cloned().unwrap_or_default());
                if ui.button("Archivo…").clicked() {
                    chosen = pick(false);
                }
            }
            ui.menu_button("De las capas cargadas", |ui| {
                for (n, p) in files {
                    if ui.button(n).clicked() {
                        chosen = vec![p.clone()];
                        ui.close();
                    }
                }
                if files.is_empty() {
                    ui.label(none);
                }
            });
            if a.multiple && !current.is_empty() && ui.button("Vaciar lista").clicked() {
                clear = true;
            }
            if !a.multiple && !current.is_empty() && ui.small_button("🗑").on_hover_text("Quitar").clicked() {
                clear = true;
            }
        });
        if a.multiple {
            for (k, p) in current.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("🗑").on_hover_text("Quitar de la lista").clicked() {
                        remove = Some(k);
                    }
                    ui.label(hipso_core::paths::stem(Path::new(p))).on_hover_text(p);
                });
            }
        }
        if !a.help.is_empty() {
            ui.label(egui::RichText::new(&a.help).size(11.0).color(t.weak));
        }
        if clear {
            args.remove(&a.id);
        }
        if let Some(k) = remove {
            let v = args.entry(a.id.clone()).or_default();
            if k < v.len() {
                v.remove(k);
            }
            if v.is_empty() {
                args.remove(&a.id);
            }
        }
        for p in chosen {
            let v = args.entry(a.id.clone()).or_default();
            let text = p.display().to_string();
            if a.multiple {
                if !v.contains(&text) {
                    v.push(text);
                }
            } else {
                *v = vec![text];
            }
        }
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
            .filter(|a| a.required && !a.flag && a.default.is_none() && !hidden(a))
            .filter(|a| args.get(&a.id).is_none_or(|v| v.iter().all(|s| s.trim().is_empty())))
            .map(|a| short_label(name, &a.id).unwrap_or_else(|| a.long.clone().map_or(a.id.clone(), |l| format!("--{l}"))))
            .collect();
        let accent = t.accent;
        let mut busy_stop = false;
        let mut out_dir = std::mem::take(&mut self.tools.out_dir);
        crate::procwin::window(ctx, title)
            .id(egui::Id::new(("cli_tool", name)))
            .open(&mut open)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut out_dir);
                ui.separator();
                crate::procwin::help(ui, t.weak, help_for(name));
                for a in spec.args.iter().filter(|a| !hidden(a)) {
                    // las opciones de archivo: el archivo, «Archivo…» y las capas cargadas
                    if model::wirable(name, a) || (a.path && a.id != "output") {
                        let kind = model::arg_kind(name, &a.id);
                        let files = self.map_files(kind);
                        self.cli_file_row(ui, &t, name, a, kind, &files, &mut args);
                    } else {
                        cli_option_row(ui, &t, name, a, &mut args);
                    }
                }
                ui.add_space(6.0);
                if !missing.is_empty() {
                    ui.label(egui::RichText::new(format!("Falta: {}", missing.join(", "))).size(11.0).color(t.weak));
                }
                if !self.cli.message.is_empty() {
                    ui.label(egui::RichText::new(&self.cli.message).size(11.5).color(theme::brand_text(&t)));
                }
                let why = format!("Falta: {}", missing.join(", "));
                if crate::procwin::run_bar(ui, accent, "Ejecutar", missing.is_empty(), running, &why, &mut busy_stop) {
                    run = true;
                }
            });
        self.tools.out_dir = out_dir;
        stop |= busy_stop;
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
        // «Guardar en» de la ventana: la carpeta de resultados de la orden
        let dir = self.tools.out_dir.trim().to_string();
        if dir.is_empty() {
            self.cli.args[k].remove("results_dir");
        } else {
            self.cli.args[k].insert("results_dir".into(), vec![dir]);
        }
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
