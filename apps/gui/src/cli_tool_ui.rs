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
pub const TOOLS: [(&str, &str, &str, &str); 20] = [
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
    ("fix-topology", "@fixtopo", "Reparar\ntopología", "Reparar topología"),
    ("simplify", "@simplify", "Simplificar", "Simplificar"),
    ("parallel", "@parallel", "Paralela", "Paralela"),
    ("fields", "@fields", "Quitar y renombrar\ncampos", "Quitar y renombrar campos"),
    ("dtm-validate", "@dtmcheck", "Validar\nMDT", "Validar MDT"),
    ("canopy-gaps", "@gaps", "Huecos\ndel dosel", "Huecos del dosel"),
    ("voxel-metrics", "@voxels", "Métricas de\nvóxeles", "Métricas de vóxeles"),
    ("trees3d", "@trees3d", "Segmentar\ncon puntos", "Segmentar con puntos"),
    ("tree-metrics", "@treemetrics", "Métricas\npor árbol", "Métricas por árbol"),
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
        "fix-topology" => "Repara la topología de una capa de polígonos o de líneas. En polígonos ajusta a una distancia los vértices cercanos de vecinos (y apoya en el borde de otro el que queda cerca de él), de modo que los bordes que debían coincidir coincidan; quita los solapes, dejando la superficie común al polígono elegido; y rellena con el vecino de más borde común los huecos que no pasen de una superficie. En líneas lleva los extremos que se quedan cortos a la línea más cercana, a una distancia como mucho. Con los ajustes en cero solo se reparan los polígonos no válidos y se quitan los solapes. Escribe una capa nueva con los atributos de la original.",
        "simplify" => "Quita vértices de las líneas y de los contornos de una capa: se conserva el que se aleja de la recta entre sus vecinos más que la tolerancia (Douglas y Peucker). En polígonos que comparten bordes, el borde común se simplifica una sola vez para los dos, de modo que no se abren huecos ni solapes; las esquinas donde se juntan tres vecinos y los extremos de las líneas no se mueven. Escribe una capa nueva con los atributos de la original.",
        "parallel" => "Traza la paralela de cada línea a una distancia: positiva a su izquierda según se recorre, negativa a su derecha. Las esquinas exteriores se resuelven con una inglete, que se corta con un bisel cuando se alarga más que el límite, y los lazos que se forman en las esquinas interiores se recortan. Sirve para cunetas, zonas de servidumbre a un lado de una vía o márgenes de un cauce. Para ampliar un polígono, usa el búfer. Escribe una capa nueva con los atributos de la original.",
        "fields" => "Quita campos de la tabla de una capa y renombra otros; los demás conservan sus valores y su tipo. Los nombres se escriben como «viejo=nuevo». Un shapefile corta los nombres a 10 caracteres al guardarse. Escribe una capa nueva; la original no se toca.",
        "dtm-validate" => "Mide la exactitud vertical de un MDT con puntos de control del suelo: los de suelo de una nube (clase 2), una capa de puntos con un campo de cota o una tabla con X, Y y Z. El error de cada punto es MDT − referencia; se dan el sesgo, el error absoluto medio (MAE), el error cuadrático medio (RMSE) y la desviación, y los estadísticos robustos NMAD y LE95 (el percentil 95 del error absoluto), que no se dejan arrastrar por unos pocos puntos malos. Escribe la tabla de errores punto a punto, un resumen y, si se pide, los errores como capa de puntos.",
        "canopy-gaps" => "Encuentra los huecos del dosel: las manchas conectadas de un modelo de altura de copas (CHM) por debajo de una altura. De cada hueco da la superficie, el perímetro, un índice de forma (1 para un círculo), las alturas del CHM dentro y el centro, y resume qué fracción del dosel es hueco. Un hueco cortado por el borde del ráster o por celdas sin dato no se conoce entero y se deja fuera, salvo que se marque. Escribe los polígonos, una tabla y, si se pide, el ráster de huecos.",
        "voxel-metrics" => "Describe la estructura vertical del dosel: la nube se reparte en columnas de celdas y en capas de altura sobre el suelo, y de los puntos de cada vóxel salen un ráster por métrica (puntos, vóxeles ocupados, volumen, relleno, altura máxima y media, entropía de las alturas, fracción de huecos y LAI de MacArthur y Horn). Con un MDT se normalizan las alturas; sin él, la nube ya debe traerlas sobre el suelo.",
        "trees3d" => "Segmenta los árboles directamente sobre los puntos, sin ráster, de arriba abajo en la línea de Li et al. (2012): los puntos más altos son cimas y cada punto más bajo se une al árbol que tiene cerca. Sirve donde el CHM esconde árboles bajo otros. Escribe la nube con el atributo treeID, las cimas y una tabla. Con una nube densa, adelgázala antes.",
        "tree-metrics" => "Mide cada árbol de una nube con el atributo treeID (la que escriben «Segmentar con CHM» o «Segmentar con puntos»): altura, base de la copa, longitud y proporción de copa, superficie y radio de la copa, y el volumen de la copa como envolvente convexa y como forma alfa tridimensional, que sigue el contorno real de la copa. Escribe una tabla y la capa de cimas con las métricas.",
        _ => "",
    }
}

/// Los nombres cortos de las opciones en la ventana: la etiqueta va junto al control, como en las
/// demás ventanas, así que no caben frases; lo que explica una opción está en su ayuda de debajo.
fn short_label(tool: &str, id: &str) -> Option<String> {
    // las opciones de las herramientas nuevas
    let own = match (tool, id) {
        ("dtm-validate", "dtm") => Some("MDT"),
        ("dtm-validate", "reference") => Some("Puntos de control"),
        ("dtm-validate", "sampling") => Some("Lectura del MDT"),
        ("dtm-validate", "class") => Some("Clase de suelo"),
        ("dtm-validate", "z_field") => Some("Campo de la cota"),
        ("dtm-validate", "x_field") => Some("Columna X"),
        ("dtm-validate", "y_field") => Some("Columna Y"),
        ("dtm-validate", "max_error") => Some("Error máximo (m)"),
        ("dtm-validate", "no_csv") => Some("Sin tabla de errores"),
        ("dtm-validate", "shp") => Some("Errores como capa de puntos"),
        ("canopy-gaps", "input") => Some("CHM"),
        ("canopy-gaps", "height") => Some("Altura del hueco (m)"),
        ("canopy-gaps", "min_area") => Some("Área mínima (m²)"),
        ("canopy-gaps", "max_area") => Some("Área máxima (m²)"),
        ("canopy-gaps", "include_edge") => Some("Conservar los cortados por el borde"),
        ("canopy-gaps", "raster") => Some("Escribir el ráster de huecos"),
        ("canopy-gaps", "gpkg") => Some("Escribir también GeoPackage"),
        ("voxel-metrics", "inputs") | ("trees3d", "inputs") => Some("Nubes"),
        ("voxel-metrics", "dtm") | ("trees3d", "dtm") | ("tree-metrics", "dtm") => Some("MDT"),
        ("voxel-metrics", "cell") => Some("Lado de la columna (m)"),
        ("voxel-metrics", "dz") => Some("Grosor de capa (m)"),
        ("voxel-metrics", "max_height") => Some("Altura máxima (m)"),
        ("voxel-metrics", "min_points") => Some("Puntos por vóxel ocupado"),
        ("voxel-metrics", "ground_height") => Some("Altura de suelo (m)"),
        ("voxel-metrics", "k") => Some("Coeficiente de extinción"),
        ("voxel-metrics", "metrics") => Some("Métricas"),
        ("voxel-metrics", "returns") => Some("Retornos"),
        ("voxel-metrics", "class") | ("trees3d", "class") | ("tree-metrics", "class") => Some("Clases"),
        ("voxel-metrics", "tile") | ("trees3d", "tile") => Some("Lado de las teselas (m)"),
        ("trees3d", "hmin") => Some("Altura mínima (m)"),
        ("trees3d", "dt1") => Some("Separación bajo la altura Zu (m)"),
        ("trees3d", "dt2") => Some("Separación sobre la altura Zu (m)"),
        ("trees3d", "zu") => Some("Altura Zu (m)"),
        ("trees3d", "radius") => Some("Radio de unión (m)"),
        ("trees3d", "min_points") | ("tree-metrics", "min_points") => Some("Puntos mínimos por árbol"),
        ("trees3d", "buffer") => Some("Margen de las teselas (m)"),
        ("trees3d", "no_cloud") => Some("Sin nube con treeID"),
        ("trees3d", "no_csv") => Some("Sin tabla de cimas"),
        ("trees3d", "no_shp") | ("tree-metrics", "no_shp") => Some("Sin capa de cimas"),
        ("tree-metrics", "input") => Some("Nube con treeID"),
        ("tree-metrics", "normalized") => Some("Las alturas ya son sobre el suelo"),
        ("tree-metrics", "gap") => Some("Salto que cierra la copa (m)"),
        ("tree-metrics", "crown_min") => Some("Altura mínima de copa (m)"),
        ("tree-metrics", "alpha") => Some("Radio alfa (m)"),
        ("tree-metrics", "dim") => Some("Atributo que numera los individuos"),
        _ => None,
    };
    if let Some(o) = own {
        return Some(o.to_string());
    }
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
        "snap" => "Distancia de ajuste",
        "max_gap" => "Hueco máximo (superficie)",
        "overlaps" => "Solapes",
        "tolerance" => "Tolerancia",
        "independent" => "Cada contorno por separado",
        "distance" => "Distancia (izquierda +, derecha −)",
        "miter" => "Límite de la inglete",
        "drop" => "Campos que quitar",
        "rename" => "Renombrar (viejo=nuevo)",
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

/// Una herramienta en marcha. Cada una es un proceso aparte (`hipso`), de modo que varias pueden
/// correr a la vez, cada una con su ventana.
struct Running {
    /// El índice de la herramienta en [`TOOLS`].
    tool: usize,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    lines: Vec<String>,
    started: std::time::Instant,
}

/// Lo que guarda la ventana entre fotogramas.
#[derive(Default)]
pub struct CliTool {
    /// Las herramientas cuya ventana está abierta (índices de [`TOOLS`]), por orden de apertura:
    /// abrir una nueva no cierra las demás.
    pub open: Vec<usize>,
    args: Vec<BTreeMap<String, Vec<String>>>,
    /// Las herramientas que se están ejecutando; una misma herramienta no corre dos veces a la vez.
    running: Vec<Running>,
    /// Lo último que dijo cada herramienta (índice de [`TOOLS`]).
    messages: Vec<String>,
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
        if self.cli.messages.len() < TOOLS.len() {
            self.cli.messages.resize(TOOLS.len(), String::new());
        }
        if !self.cli.open.contains(&k) {
            self.cli.open.push(k);
            self.cli.messages[k].clear();
            self.prefill_with_active_layer(k);
        }
    }

    /// Si la primera opción de archivo que exige la herramienta está vacía y la capa activa es del
    /// tipo que admite, la ventana se abre ya con ella: lo habitual es trabajar sobre la capa que se
    /// tiene delante, y así se ven enseguida sus campos en las listas.
    fn prefill_with_active_layer(&mut self, k: usize) {
        let name = TOOLS[k].0;
        let Some(schema) = self.tool_schema() else { return };
        let Some(spec) = schema.tool(name) else { return };
        let Some(a) = spec.args.iter().find(|a| a.required && model::wirable(name, a) && !a.multiple) else { return };
        if self.cli.args[k].get(&a.id).is_some_and(|v| v.iter().any(|s| !s.trim().is_empty())) {
            return;
        }
        let kind = model::arg_kind(name, &a.id);
        let path = self.active.and_then(|i| self.refs.get(i)).map(|r| r.path.clone()).filter(|p| kind.accepts(p));
        if let Some(p) = path {
            self.cli.args[k].insert(a.id.clone(), vec![p.display().to_string()]);
        }
    }

    /// Si la herramienta `k` tiene su ventana abierta (para iluminar su botón).
    pub fn cli_tool_open(&self, k: usize) -> bool {
        self.cli.open.contains(&k)
    }

    /// Las herramientas de la ventana genérica que están ejecutándose: su índice, su título y los
    /// segundos que llevan.
    pub fn cli_running_list(&self) -> Vec<(usize, &'static str, f64)> {
        self.cli
            .running
            .iter()
            .map(|r| (r.tool, model::entry(TOOLS[r.tool].0).map_or(TOOLS[r.tool].0, |e| e.title), r.started.elapsed().as_secs_f64()))
            .collect()
    }

    /// Pide detener la herramienta `k` si está en marcha.
    pub fn cli_stop(&mut self, k: usize) {
        for r in self.cli.running.iter().filter(|r| r.tool == k) {
            r.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Lo que han dicho las órdenes en marcha: líneas al registro y, al terminar, los archivos
    /// nombrados tras `->` abiertos en el mapa.
    fn cli_tool_poll(&mut self, ctx: &egui::Context) {
        // las líneas nuevas van al registro y se ve cuáles herramientas han terminado
        let mut finished: Vec<(usize, Result<(), String>)> = Vec::new();
        for run in &mut self.cli.running {
            while let Ok(m) = run.rx.try_recv() {
                match m {
                    Msg::Line(l) => {
                        run.lines.push(l.clone());
                        self.log.push(l);
                    }
                    Msg::Done(r) => finished.push((run.tool, r)),
                }
            }
        }
        for (tool, r) in finished {
            let Some(pos) = self.cli.running.iter().position(|x| x.tool == tool) else { continue };
            let lines = self.cli.running.remove(pos).lines;
            let title = model::entry(TOOLS[tool].0).map_or(TOOLS[tool].0, |e| e.title);
            if self.cli.messages.len() <= tool {
                self.cli.messages.resize(TOOLS.len(), String::new());
            }
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
                    self.cli.messages[tool] = lines.last().cloned().unwrap_or_else(|| "Terminado.".into());
                    self.log(format!("{title}: terminado."));
                    if !open.is_empty() {
                        self.open_files(ctx, open);
                    }
                }
                Err(e) => {
                    self.cli.messages[tool] = format!("Falló: {e}");
                    self.notice = Some(format!("«{title}» falló: {e}"));
                }
            }
            self.show_log = true;
        }
        if !self.cli.running.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
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
        if self.cli.open.is_empty() {
            return;
        }
        if self.tool_schema().is_none() {
            self.cli.open.clear();
            self.notice = Some("No se encuentra el ejecutable «hipso» junto al programa.".into());
            return;
        }
        // una ventana por herramienta abierta: abrir otra no cierra las demás ni detiene lo que corre
        for k in self.cli.open.clone() {
            if !self.cli_tool_one(ctx, k) {
                self.cli.open.retain(|x| *x != k);
            }
        }
    }

    /// La ventana de la herramienta `k`; devuelve `false` cuando se cierra.
    fn cli_tool_one(&mut self, ctx: &egui::Context, k: usize) -> bool {
        let Some(schema) = self.tool_schema() else { return false };
        let name = TOOLS[k].0;
        let Some(spec) = schema.tool(name) else { return true };
        let title = model::entry(name).map_or(name, |e| e.title);
        let t = self.tokens();
        let mut open = true;
        let running = self.cli.running.iter().any(|r| r.tool == k);
        // lo que lleva la herramienta en marcha: segundos y la última línea que dijo
        let progress = self.cli.running.iter().find(|r| r.tool == k).map(|r| {
            let last = r.lines.last().map(|l| format!(" · {l}")).unwrap_or_default();
            format!("Ejecutando… {:.0} s{last}", r.started.elapsed().as_secs_f64())
        });
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
        // las opciones de un método o de una casilla solo salen cuando se usa
        let visible: Vec<&model::ArgSpec> = spec
            .args
            .iter()
            .filter(|a| !hidden(a) && spec.shown(a, &args))
            .collect();
        let accent = t.accent;
        let mut busy_stop = false;
        let mut out_dir = std::mem::take(&mut self.tools.out_dir);
        crate::procwin::window(ctx, title)
            .id(egui::Id::new(("cli_tool", name)))
            .open(&mut open)
            .show(ctx, |ui| {
                crate::procwin::begin(ui, t.weak, help_for(name));
                if !missing.is_empty() {
                    crate::procwin::note(format!("Falta: {}", missing.join(", ")));
                }
                for a in visible.iter().copied() {
                    // las opciones que nombran un campo: una lista con los campos de la tabla del archivo
                    // elegido (si no se lee, el cuadro de texto de siempre)
                    if let Some(fs) = model::field_source(name, &a.id) {
                        let title = {
                            let n = short_label(name, &a.id).unwrap_or_else(|| a.long.clone().map_or(a.id.clone(), |l| format!("--{l}")));
                            if a.required { format!("{n} *") } else { n }
                        };
                        let source_label = short_label(name, fs.source).unwrap_or_else(|| fs.source.to_string()).to_lowercase();
                        let source = args.get(fs.source).and_then(|v| v.first()).cloned();
                        ui.add_space(4.0);
                        if crate::fieldpick::row(ui, &t, name, a, &title, fs, &source_label, source.as_deref(), &mut args) {
                            continue;
                        }
                    }
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
                if let Some(p) = &progress {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new(p).size(11.5).color(theme::brand_text(&t)));
                    });
                } else if let Some(m) = self.cli.messages.get(k).filter(|m| !m.is_empty()) {
                    ui.label(egui::RichText::new(m).size(11.5).color(theme::brand_text(&t)));
                }
                let why = format!("Falta: {}", missing.join(", "));
                if crate::procwin::footer(ui, &mut out_dir, accent, "Ejecutar", missing.is_empty(), running, &why, &mut busy_stop) {
                    run = true;
                }
            });
        self.tools.out_dir = out_dir;
        stop |= busy_stop;
        self.cli.args[k] = args;
        if stop {
            self.cli_stop(k);
        }
        if run {
            self.cli_tool_run(ctx, k);
        }
        open
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
        self.cli.running.push(Running { tool: k, rx, cancel, lines: Vec::new(), started: std::time::Instant::now() });
        if self.cli.messages.len() <= k {
            self.cli.messages.resize(TOOLS.len(), String::new());
        }
        self.cli.messages[k] = "Ejecutando…".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(tool: usize) -> (Running, std::sync::mpsc::Sender<Msg>, Arc<AtomicBool>) {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        (Running { tool, rx, cancel: cancel.clone(), lines: Vec::new(), started: std::time::Instant::now() }, tx, cancel)
    }

    #[test]
    fn several_tool_windows_stay_open_and_several_tools_run_at_once() {
        let mut app = App::default();
        let ctx = egui::Context::default();
        app.open_cli_tool(0);
        app.open_cli_tool(5);
        app.open_cli_tool(0);
        assert_eq!(app.cli.open, vec![0, 5], "abrir otra ventana no cierra las demás ni se repite");
        assert!(app.cli_tool_open(0) && app.cli_tool_open(5) && !app.cli_tool_open(1));
        // dos herramientas en marcha a la vez
        let (r0, tx0, c0) = running(0);
        let (r5, tx5, c5) = running(5);
        app.cli.running.push(r0);
        app.cli.running.push(r5);
        let list = app.cli_running_list();
        assert_eq!(list.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 5]);
        // detener una no toca la otra
        app.cli_stop(0);
        assert!(c0.load(Ordering::Relaxed) && !c5.load(Ordering::Relaxed));
        // una línea de una va al registro y no acaba a la otra; al terminar la 5 queda la 0
        tx0.send(Msg::Line("avanza".into())).unwrap();
        tx5.send(Msg::Line("hecho".into())).unwrap();
        tx5.send(Msg::Done(Ok(()))).unwrap();
        app.cli_tool_poll(&ctx);
        assert_eq!(app.cli.running.len(), 1);
        assert_eq!(app.cli.running[0].tool, 0);
        assert_eq!(app.cli.running[0].lines, vec!["avanza".to_string()]);
        assert_eq!(app.cli.messages[5], "hecho");
        assert!(app.log.iter().any(|l| l == "avanza") && app.log.iter().any(|l| l.contains("terminado")));
        // la que falla deja su mensaje y avisa, sin tocar la otra
        let (r1, tx1, _) = running(1);
        app.cli.running.push(r1);
        tx1.send(Msg::Done(Err("sin memoria".into()))).unwrap();
        app.cli_tool_poll(&ctx);
        assert!(app.cli.messages[1].starts_with("Falló"));
        assert!(app.notice.as_deref().is_some_and(|n| n.contains("sin memoria")));
        assert_eq!(app.cli.running.len(), 1);
    }
}
