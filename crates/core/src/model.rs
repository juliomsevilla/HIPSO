//! Modelos de flujo: un grafo con las herramientas de `hipso` que se guarda como archivo y se
//! ejecuta sin el lienzo. Un nodo es una entrada (un archivo o una carpeta) o una herramienta (una
//! orden de `hipso` con sus opciones); un hilo envía los productos de un nodo a una opción de
//! archivo de otro. El motor ejecuta el ejecutable `hipso` una vez por nodo, de modo que lo que
//! hace un modelo es exactamente lo que harían las mismas órdenes escritas una tras otra.
//!
//! Las opciones de cada orden salen de un esquema que escribe la propia línea de comandos (`hipso
//! schema`), así que una orden que se añada allí está disponible para los modelos sin más trabajo.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crate::json::{Json, quote};
use crate::{Error, Result};

/// Qué es un archivo, para conectar nodos: una herramienta toma de lo que recibe los archivos de su
/// tipo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Any,
    Cloud,
    Raster,
    Vector,
    Table,
}

impl Kind {
    /// Las extensiones de los archivos principales de este tipo (los acompañantes, como `.dbf`, no
    /// figuran).
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::Any => &[],
            Kind::Cloud => &["las", "laz"],
            Kind::Raster => &["tif", "tiff", "jp2", "img", "hgt", "asc"],
            Kind::Vector => &["shp", "geojson", "json", "gpkg", "kml", "kmz"],
            Kind::Table => &["csv", "txt", "xlsx", "xls"],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Any => "archivo",
            Kind::Cloud => "nube",
            Kind::Raster => "ráster",
            Kind::Vector => "vector",
            Kind::Table => "tabla",
        }
    }

    /// Si el archivo es de este tipo (todos lo son para `Any`, salvo los acompañantes).
    pub fn accepts(self, p: &Path) -> bool {
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match self {
            Kind::Any => is_main(&ext),
            k => k.extensions().contains(&ext.as_str()),
        }
    }

    /// Si lo que produce un nodo (`made`) puede alimentar una opción que pide `self`.
    pub fn takes(self, made: &[Kind]) -> bool {
        self == Kind::Any
            || made.is_empty()
            || made.iter().any(|m| *m == Kind::Any || *m == self)
    }
}

/// Un archivo que es un producto y no el acompañante de otro (`.dbf`, `.prj`, archivos de
/// georreferencia...).
fn is_main(ext: &str) -> bool {
    [Kind::Cloud, Kind::Raster, Kind::Vector, Kind::Table]
        .iter()
        .any(|k| k.extensions().contains(&ext))
        || matches!(ext, "png" | "jpg" | "jpeg" | "pdf" | "html")
}

// ---------------------------------------------------------------------------------------------
// El esquema de las órdenes

/// Una opción o argumento posicional de una orden.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArgSpec {
    pub id: String,
    pub long: Option<String>,
    pub short: Option<char>,
    pub help: String,
    pub positional: bool,
    /// Una opción que no lleva valor.
    pub flag: bool,
    pub multiple: bool,
    pub required: bool,
    /// El valor es un archivo o una carpeta.
    pub path: bool,
    /// Los valores que admite, cuando es una lista de nombres.
    pub values: Vec<String>,
    pub default: Option<String>,
}

/// Una orden de `hipso`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub about: String,
    pub args: Vec<ArgSpec>,
}

impl ToolSpec {
    pub fn arg(&self, id: &str) -> Option<&ArgSpec> {
        self.args.iter().find(|a| a.id == id)
    }

    /// La opción que da nombre al archivo de salida, cuando la orden la tiene.
    pub fn output_arg(&self) -> Option<&ArgSpec> {
        self.args
            .iter()
            .find(|a| a.id == "output" && a.path && !a.positional)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    pub tools: Vec<ToolSpec>,
}

fn json_str(v: &Json, key: &str) -> String {
    v.get(key).and_then(Json::as_str).unwrap_or_default().to_string()
}

fn json_bool(v: &Json, key: &str) -> bool {
    matches!(v.get(key), Some(Json::Bool(true)))
}

impl Schema {
    pub fn tool(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.iter().find(|t| t.name == name)
    }

    /// El esquema como el texto que imprime `hipso schema`.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"hipso_esquema\":1,\"tools\":[");
        for (i, t) in self.tools.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"name\":{},\"about\":{},\"args\":[",
                quote(&t.name),
                quote(&t.about)
            ));
            for (k, a) in t.args.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                out.push_str(&format!(
                    "{{\"id\":{},\"long\":{},\"short\":{},\"help\":{},\"positional\":{},\"flag\":{},\"multiple\":{},\"required\":{},\"path\":{},\"values\":[{}],\"default\":{}}}",
                    quote(&a.id),
                    a.long.as_deref().map_or("null".into(), quote),
                    a.short.map_or("null".into(), |c| quote(&c.to_string())),
                    quote(&a.help),
                    a.positional,
                    a.flag,
                    a.multiple,
                    a.required,
                    a.path,
                    a.values.iter().map(|v| quote(v)).collect::<Vec<_>>().join(","),
                    a.default.as_deref().map_or("null".into(), quote),
                ));
            }
            out.push_str("]}");
        }
        out.push_str("]}");
        out
    }

    pub fn from_json(text: &str) -> Result<Schema> {
        let j = Json::parse(text)?;
        let tools = j
            .get("tools")
            .and_then(Json::as_array)
            .ok_or_else(|| Error::Format("el esquema no tiene «tools»".into()))?;
        let mut s = Schema::default();
        for t in tools {
            let mut tool = ToolSpec {
                name: json_str(t, "name"),
                about: json_str(t, "about"),
                args: Vec::new(),
            };
            for a in t.get("args").and_then(Json::as_array).unwrap_or(&[]) {
                tool.args.push(ArgSpec {
                    id: json_str(a, "id"),
                    long: a.get("long").and_then(Json::as_str).map(str::to_string),
                    short: a
                        .get("short")
                        .and_then(Json::as_str)
                        .and_then(|c| c.chars().next()),
                    help: json_str(a, "help"),
                    positional: json_bool(a, "positional"),
                    flag: json_bool(a, "flag"),
                    multiple: json_bool(a, "multiple"),
                    required: json_bool(a, "required"),
                    path: json_bool(a, "path"),
                    values: a
                        .get("values")
                        .and_then(Json::as_array)
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                    default: a.get("default").and_then(Json::as_str).map(str::to_string),
                });
            }
            s.tools.push(tool);
        }
        Ok(s)
    }
}

// ---------------------------------------------------------------------------------------------
// Qué son las herramientas para las personas: grupo, título y tipos de archivo

/// Lo que el esquema no puede decir de una orden.
pub struct Entry {
    pub tool: &'static str,
    pub group: &'static str,
    pub title: &'static str,
    /// Lo que admiten sus opciones de archivo.
    pub takes: Kind,
    /// Lo que escribe.
    pub makes: &'static [Kind],
    /// La extensión de la salida que se le pide cuando necesita una.
    pub ext: &'static str,
}

const C: Kind = Kind::Cloud;
const R: Kind = Kind::Raster;
const V: Kind = Kind::Vector;
const T: Kind = Kind::Table;
const A: Kind = Kind::Any;

const fn e(
    tool: &'static str,
    group: &'static str,
    title: &'static str,
    takes: Kind,
    makes: &'static [Kind],
    ext: &'static str,
) -> Entry {
    Entry {
        tool,
        group,
        title,
        takes,
        makes,
        ext,
    }
}

/// Las órdenes que tienen sentido como nodos, agrupadas como las agrupa la cinta.
pub const CATALOG: &[Entry] = &[
    e("info", "Nubes", "Información de nubes", C, &[], "txt"),
    e("catalog", "Nubes", "Revisar entrega de hojas", C, &[], "txt"),
    e("classify", "Preparar", "Clasificar", C, &[C], "laz"),
    e("filter", "Preparar", "Filtrar y unir nubes", C, &[C], "laz"),
    e("thin", "Preparar", "Adelgazar nube", C, &[C], "laz"),
    e("tile", "Preparar", "Dividir en teselas", C, &[C], "laz"),
    e("clip", "Preparar", "Recortar con polígonos", C, &[C, V, T], "laz"),
    e("edit", "Preparar", "Editar puntos con polígonos", C, &[C], "laz"),
    e("apply-edits", "Preparar", "Aplicar ediciones guardadas", C, &[C], "laz"),
    e("propagate-class", "Preparar", "Propagar clase a nube completa", C, &[C], "laz"),
    e("crs", "Preparar", "Asignar proyección", C, &[C], "laz"),
    e("dedup", "Preparar", "Quitar duplicados", C, &[C], "laz"),
    e("merge-clouds", "Preparar", "Unir nubes", C, &[C], "laz"),
    e("hag", "Preparar", "Altura sobre el suelo", C, &[C], "laz"),
    e("colorize", "Preparar", "Colorear con una ortofoto", C, &[C], "laz"),
    e("intensity", "Preparar", "Normalizar intensidad", C, &[C], "laz"),
    e("import", "Preparar", "Importar PLY o E57", A, &[C], "laz"),
    e("copc", "Preparar", "Escribir COPC", C, &[C], "laz"),
    e("fit", "Preparar", "Ajustar una nube a otra", C, &[C], "laz"),
    e("export", "Preparar", "Exportar puntos", C, &[C, T], "laz"),
    e("reproject", "Preparar", "Reproyectar", A, &[C, R], "tif"),
    e("dtm", "Superficies", "Modelo digital del terreno", C, &[R], "tif"),
    e("chm", "Superficies", "Modelo de altura de copas", C, &[R], "tif"),
    e("dsm", "Superficies", "Modelo digital de superficie", C, &[R], "tif"),
    e("raster", "Superficies", "Ráster de una estadística", C, &[R], "tif"),
    e("hillshade", "Superficies", "Sombreado", R, &[A], "png"),
    e("contour", "Superficies", "Curvas de nivel", R, &[V], "shp"),
    e("render", "Nubes", "Dibujar una nube en PNG", C, &[A], "png"),
    e("trees", "Inventario", "Árboles individuales", R, &[R, V, T], "shp"),
    e("obia", "Inventario", "Análisis por objetos", R, &[V, T], "shp"),
    e("metrics", "Inventario", "Métricas por celda", C, &[V, T], "shp"),
    e("model", "Inventario", "Modelo lineal por área", A, &[T, R], "csv"),
    e("change", "Inventario", "Cambio forestal", R, &[R, V, T], "tif"),
    e("corridor", "Inventario", "Corredor eléctrico", C, &[V, T], "shp"),
    e("stem-volume", "Inventario", "Medición de troncos", C, &[V, T], "shp"),
    e("flightlines", "Inventario", "Líneas de vuelo", C, &[T, C], "csv"),
    e("swath", "Inventario", "Diferencia entre pasadas", C, &[R], "tif"),
    e("compare", "Inventario", "Comparar nubes", C, &[C], "laz"),
    e("voronoi-density", "Inventario", "Densidad por Voronoi", C, &[R, T], "tif"),
    e("terrain", "Terreno", "Análisis de terreno", R, &[R, V], "tif"),
    e("flood", "Terreno", "Inundación", R, &[R, T], "tif"),
    e("ridges", "Terreno", "Crestas y vaguadas", R, &[V], "shp"),
    e("leastcost", "Terreno", "Ruta de menor coste", R, &[V], "shp"),
    e("piles", "Terreno", "Volumen de acopios", A, &[V, T], "shp"),
    e("cutfill", "Terreno", "Desmonte y terraplén", R, &[R, T], "tif"),
    e("ponds", "Terreno", "Balsas", R, &[R, V], "tif"),
    e("align", "Ráster", "Igualar rásteres", R, &[R], "tif"),
    e("mosaic", "Ráster", "Mosaico", R, &[R], "tif"),
    e("stack", "Ráster", "Crear multibanda", R, &[R], "tif"),
    e("calc", "Ráster", "Calculadora ráster", R, &[R], "tif"),
    e("reclass", "Ráster", "Reclasificar", R, &[R], "tif"),
    e("clip-raster", "Ráster", "Cortar por máscara", A, &[R], "tif"),
    e("sieve", "Ráster", "Tamizar", R, &[R], "tif"),
    e("proximity", "Ráster", "Proximidad", R, &[R], "tif"),
    e("fill-nodata", "Ráster", "Rellenar huecos", R, &[R], "tif"),
    e("cog", "Ráster", "Escribir COG", R, &[R], "tif"),
    e("vectorize", "Ráster", "Vectorizar", R, &[V], "shp"),
    e("zonal", "Vectorial", "Estadística zonal", A, &[V], "shp"),
    e("is-valid", "Vectorial", "Comprobar validez", V, &[], "txt"),
    e("make-valid", "Vectorial", "Reparar geometrías", V, &[V], "shp"),
    e("buffer", "Vectorial", "Búfer", V, &[V], "shp"),
    e("dissolve", "Vectorial", "Disolver", V, &[V], "shp"),
    e("union", "Vectorial", "Unión", V, &[V], "shp"),
    e("intersect", "Vectorial", "Intersección", V, &[V], "shp"),
    e("voronoi", "Vectorial", "Voronoi", V, &[V], "shp"),
    e("extract", "Vectorial", "Extraer valores de un ráster", V, &[V], "shp"),
    e("join-table", "Vectorial", "Unir una tabla a una capa", V, &[V], "shp"),
    e("merge", "Vectorial", "Combinar capas", V, &[V], "shp"),
    e("split", "Vectorial", "Dividir por un campo", V, &[V], "shp"),
    e("clip-vector", "Vectorial", "Recortar o borrar con polígonos", V, &[V], "shp"),
    e("near", "Vectorial", "Cercanía", V, &[V], "shp"),
    e("summary", "Vectorial", "Estadísticas de resumen", A, &[T], "csv"),
    e("topology", "Vectorial", "Comprobar topología", A, &[V], "csv"),
    e("rasterize", "Ráster", "Rasterizar una capa", V, &[R], "tif"),
    e("focal", "Ráster", "Filtro focal", R, &[R], "tif"),
    e("rstats", "Ráster", "Estadísticas de un ráster", R, &[T], "csv"),
    e("tabulate", "Ráster", "Tabular áreas por clases", R, &[T], "csv"),
    e("geojson", "Vectorial", "Convertir shapefile y GeoJSON", V, &[V], "geojson"),
];

/// El tipo de archivo que admite una opción cuando no es la entrada principal de la orden.
const ARG_KINDS: &[(&str, &str, Kind)] = &[
    ("trees", "around", C),
    ("trees", "cloud", C),
    ("trees", "tops_file", V),
    ("trees", "field", V),
    ("obia", "cloud", C),
    ("obia", "samples", V),
    ("obia", "rules", A),
    ("change", "stands", V),
    ("colorize", "image", R),
    ("intensity", "trajectory", T),
    ("clip", "regions", V),
    ("clip", "dtm", R),
    ("dtm", "breaklines", V),
    ("chm", "dtm", R),
    ("corridor", "dtm", R),
    ("filter", "dtm", R),
    ("export", "dtm", R),
    ("metrics", "dtm", R),
    ("edit", "regions", V),
    ("apply-edits", "edits", A),
    ("propagate-class", "reference", C),
    ("model", "plots", V),
    ("model", "field", T),
    ("model", "cells", V),
    ("piles", "piles", V),
    ("cutfill", "regions", V),
    ("leastcost", "barriers", V),
    ("leastcost", "prefer", V),
    ("zonal", "regions", V),
    ("clip-raster", "mask", A),
    ("reclass", "rules_file", T),
    ("reproject", "reference", A),
    ("compare", "reference", C),
    ("fit", "reference", C),
    ("align", "reference", R),
    ("extract", "raster", R),
    ("join-table", "table", T),
    ("clip-vector", "mask", V),
    ("near", "to", V),
    ("rasterize", "reference", R),
    ("tabulate", "values", R),
    ("classify", "recipe", A),
    ("dtm", "recipe", A),
];

/// Las opciones que nombran las tablas, para comprobar que existen.
pub fn named_args() -> impl Iterator<Item = (&'static str, &'static str)> {
    ARG_KINDS
        .iter()
        .map(|(t, a, _)| (*t, *a))
        .chain(OUTPUT_ARGS.iter().copied())
}

/// Las opciones que llevan una ruta pero la escriben.
const OUTPUT_ARGS: &[(&str, &str)] = &[("clip", "layout"), ("clip", "metrics")];

/// El tipo de archivo que admite la opción `arg` de `tool`.
pub fn arg_kind(tool: &str, arg: &str) -> Kind {
    ARG_KINDS
        .iter()
        .find(|(t, a, _)| *t == tool && *a == arg)
        .map(|(_, _, k)| *k)
        .or_else(|| entry(tool).map(|e| e.takes))
        .unwrap_or(Kind::Any)
}

/// Si la opción admite un archivo que otro nodo puede enviarle.
pub fn wirable(tool: &str, a: &ArgSpec) -> bool {
    a.path
        && a.id != "output"
        && a.id != "results_dir"
        && !OUTPUT_ARGS.iter().any(|(t, i)| *t == tool && *i == a.id)
}

/// La entrada de una orden, si la tiene.
pub fn entry(tool: &str) -> Option<&'static Entry> {
    CATALOG.iter().find(|e| e.tool == tool)
}

// ---------------------------------------------------------------------------------------------
// El modelo

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// Un archivo o una carpeta de archivos.
    Input { path: String },
    /// Una orden de `hipso`; `args` asigna a cada identificador de argumento sus valores.
    Tool {
        tool: String,
        args: BTreeMap<String, Vec<String>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    /// Los productos se guardan en la carpeta de resultados (si no, se borran cuando nada los
    /// necesita).
    pub keep: bool,
    pub x: f32,
    pub y: f32,
}

/// Los productos de `from` van a la opción de archivo `port` de `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub port: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Adónde van los productos conservados (vacío: una carpeta junto al modelo).
    pub results: String,
}

/// Un problema que impide ejecutar un modelo.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub node: Option<String>,
    pub text: String,
}

impl Model {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Un identificador que no usa ningún nodo.
    pub fn new_id(&self) -> String {
        let mut k = self.nodes.len() + 1;
        loop {
            let id = format!("n{k}");
            if self.node(&id).is_none() {
                return id;
            }
            k += 1;
        }
    }

    pub fn to_json(&self) -> String {
        let mut out = format!(
            "{{\"hipso_modelo\":1,\"results\":{},\"nodes\":[",
            quote(&self.results)
        );
        for (i, n) in self.nodes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"id\":{},\"label\":{},\"keep\":{},\"x\":{},\"y\":{},",
                quote(&n.id),
                quote(&n.label),
                n.keep,
                n.x,
                n.y
            ));
            match &n.kind {
                NodeKind::Input { path } => {
                    out.push_str(&format!("\"type\":\"input\",\"path\":{}}}", quote(path)));
                }
                NodeKind::Tool { tool, args } => {
                    out.push_str(&format!("\"type\":\"tool\",\"tool\":{},\"args\":{{", quote(tool)));
                    for (k, (id, vals)) in args.iter().enumerate() {
                        if k > 0 {
                            out.push(',');
                        }
                        out.push_str(&format!(
                            "{}:[{}]",
                            quote(id),
                            vals.iter().map(|v| quote(v)).collect::<Vec<_>>().join(",")
                        ));
                    }
                    out.push_str("}}");
                }
            }
        }
        out.push_str("],\"edges\":[");
        for (i, e) in self.edges.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"from\":{},\"to\":{},\"port\":{}}}",
                quote(&e.from),
                quote(&e.to),
                quote(&e.port)
            ));
        }
        out.push_str("]}");
        out
    }

    pub fn from_json(text: &str) -> Result<Model> {
        let j = Json::parse(text)?;
        if j.get("hipso_modelo").is_none() {
            return Err(Error::Format("no es un modelo de flujo de HIPSO".into()));
        }
        let mut m = Model {
            results: json_str(&j, "results"),
            ..Model::default()
        };
        for n in j.get("nodes").and_then(Json::as_array).unwrap_or(&[]) {
            let kind = match json_str(n, "type").as_str() {
                "input" => NodeKind::Input {
                    path: json_str(n, "path"),
                },
                "tool" => {
                    let mut args = BTreeMap::new();
                    if let Some(Json::Obj(members)) = n.get("args") {
                        for (k, v) in members {
                            let vals = v
                                .as_array()
                                .unwrap_or(&[])
                                .iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect();
                            args.insert(k.clone(), vals);
                        }
                    }
                    NodeKind::Tool {
                        tool: json_str(n, "tool"),
                        args,
                    }
                }
                other => return Err(Error::Format(format!("nodo de tipo desconocido: «{other}»"))),
            };
            m.nodes.push(Node {
                id: json_str(n, "id"),
                label: json_str(n, "label"),
                kind,
                keep: json_bool(n, "keep"),
                x: n.get("x").and_then(Json::as_f64).unwrap_or(0.0) as f32,
                y: n.get("y").and_then(Json::as_f64).unwrap_or(0.0) as f32,
            });
        }
        for e in j.get("edges").and_then(Json::as_array).unwrap_or(&[]) {
            m.edges.push(Edge {
                from: json_str(e, "from"),
                to: json_str(e, "to"),
                port: json_str(e, "port"),
            });
        }
        Ok(m)
    }

    /// Los nodos en un orden en el que cada uno va después de los que lo alimentan; `None` cuando
    /// hay un bucle.
    pub fn order(&self) -> Option<Vec<usize>> {
        let index: HashMap<&str, usize> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let mut depth = vec![0usize; self.nodes.len()];
        let mut indeg = vec![0usize; self.nodes.len()];
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];
        for e in &self.edges {
            if let (Some(&a), Some(&b)) = (index.get(e.from.as_str()), index.get(e.to.as_str())) {
                indeg[b] += 1;
                out[a].push(b);
            }
        }
        let mut ready: Vec<usize> = (0..self.nodes.len()).filter(|i| indeg[*i] == 0).collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(i) = ready.pop() {
            order.push(i);
            for &b in &out[i] {
                depth[b] = depth[b].max(depth[i] + 1);
                indeg[b] -= 1;
                if indeg[b] == 0 {
                    ready.push(b);
                }
            }
        }
        (order.len() == self.nodes.len()).then_some(order)
    }

    /// A cuántos pasos de las entradas está cada nodo (0 para una entrada), para colocar el lienzo
    /// en columnas. Un nodo dentro de un bucle recibe 0.
    pub fn depths(&self) -> Vec<usize> {
        let mut depth = vec![0usize; self.nodes.len()];
        if let Some(order) = self.order() {
            let index: HashMap<&str, usize> = self
                .nodes
                .iter()
                .enumerate()
                .map(|(i, n)| (n.id.as_str(), i))
                .collect();
            for i in order {
                for e in self.edges.iter().filter(|e| e.to == self.nodes[i].id) {
                    if let Some(&a) = index.get(e.from.as_str()) {
                        depth[i] = depth[i].max(depth[a] + 1);
                    }
                }
            }
        }
        depth
    }

    /// Todo lo que impide ejecutar el modelo, con las palabras que muestra el lienzo.
    pub fn problems(&self, schema: &Schema) -> Vec<Problem> {
        let mut out = Vec::new();
        let mut add = |node: Option<&str>, text: String| {
            out.push(Problem {
                node: node.map(str::to_string),
                text,
            });
        };
        if self.nodes.is_empty() {
            add(None, "El flujo está vacío.".into());
            return out;
        }
        if self.order().is_none() {
            add(None, "Hay un bucle: un nodo depende de su propio resultado.".into());
        }
        let mut ids = HashSet::new();
        for n in &self.nodes {
            if !ids.insert(n.id.as_str()) {
                add(Some(&n.id), format!("El identificador «{}» está repetido.", n.id));
            }
        }
        if !self.nodes.iter().any(|n| n.keep) {
            add(None, "Ningún nodo conserva su resultado: marca al menos uno.".into());
        }
        for e in &self.edges {
            if self.node(&e.from).is_none() || self.node(&e.to).is_none() {
                add(None, format!("Un hilo une nodos que no existen ({} → {}).", e.from, e.to));
            }
        }
        for n in &self.nodes {
            match &n.kind {
                NodeKind::Input { path } => {
                    if path.trim().is_empty() {
                        add(Some(&n.id), format!("«{}» no tiene archivo ni carpeta.", self.name(n)));
                    } else if !Path::new(path).exists() {
                        add(Some(&n.id), format!("«{}»: no existe {path}.", self.name(n)));
                    }
                }
                NodeKind::Tool { tool, args } => {
                    let Some(spec) = schema.tool(tool) else {
                        add(Some(&n.id), format!("«{tool}» no es una herramienta de HIPSO."));
                        continue;
                    };
                    for a in &spec.args {
                        if a.id == "results_dir" || a.id == "output" {
                            continue;
                        }
                        let takes = arg_kind(tool, &a.id);
                        let wired: Vec<&Edge> = self
                            .edges
                            .iter()
                            .filter(|e| e.to == n.id && e.port == a.id)
                            .collect();
                        let typed = args.get(&a.id).is_some_and(|v| v.iter().any(|s| !s.trim().is_empty()));
                        for e in &wired {
                            if let Some(src) = self.node(&e.from)
                                && !takes_from(src, takes)
                            {
                                add(
                                    Some(&n.id),
                                    format!(
                                        "«{}» no puede recibir lo que produce «{}».",
                                        self.name(n),
                                        self.name(src)
                                    ),
                                );
                            }
                        }
                        if a.required && !a.flag && a.default.is_none() && wired.is_empty() && !typed {
                            let what = a.long.as_deref().map_or(a.id.clone(), |l| format!("--{l}"));
                            add(
                                Some(&n.id),
                                format!("«{}»: falta {} ({}).", self.name(n), what, first_line(&a.help)),
                            );
                        }
                    }
                    for e in self.edges.iter().filter(|e| e.to == n.id) {
                        if spec.arg(&e.port).is_none_or(|a| !a.path) {
                            add(
                                Some(&n.id),
                                format!("«{}» no tiene la entrada «{}».", self.name(n), e.port),
                            );
                        }
                    }
                }
            }
        }
        out
    }

    /// El nombre con el que se muestra un nodo.
    pub fn name(&self, n: &Node) -> String {
        if !n.label.trim().is_empty() {
            return n.label.clone();
        }
        match &n.kind {
            NodeKind::Input { path } => Path::new(path)
                .file_name()
                .map_or("Entrada".into(), |f| f.to_string_lossy().into_owned()),
            NodeKind::Tool { tool, .. } => entry(tool).map_or(tool.clone(), |e| e.title.to_string()),
        }
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

/// Si lo que produce el nodo `src` puede ir a la opción `arg` de `tool`.
pub fn can_feed(src: &Node, tool: &str, arg: &str) -> bool {
    takes_from(src, arg_kind(tool, arg))
}

/// Si el nodo `src` puede alimentar una opción que admite archivos del tipo `want`.
fn takes_from(src: &Node, want: Kind) -> bool {
    match &src.kind {
        NodeKind::Input { .. } => true,
        NodeKind::Tool { tool, .. } => want.takes(entry(tool).map_or(&[], |e| e.makes)),
    }
}

// ---------------------------------------------------------------------------------------------
// Ejecución

/// Lo que produjo una ejecución.
#[derive(Debug, Default)]
pub struct Report {
    /// Los productos conservados: el nodo y los archivos que ha producido ahora en la carpeta de
    /// resultados.
    pub outputs: Vec<(String, Vec<PathBuf>)>,
}

/// En qué estado está un nodo mientras se ejecuta el modelo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Waiting,
    Running,
    Done,
    Failed,
}

/// Lo que necesita `run` además del modelo.
pub struct Run<'a> {
    pub exe: &'a Path,
    pub schema: &'a Schema,
    /// Adónde van los resultados; la carpeta del propio modelo si está vacío.
    pub results: &'a Path,
    /// Valores para los nodos de entrada, por nombre o identificador, que sustituyen a los
    /// guardados en el modelo.
    pub inputs: &'a HashMap<String, String>,
    pub cancel: &'a AtomicBool,
}

/// Un archivo oculto: los que crean los sistemas de archivos (`._nombre` en discos externos)
/// no son productos.
fn is_hidden(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
}

fn files_of(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(path)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && !is_hidden(p))
            .collect();
        v.sort();
        Ok(v)
    } else {
        Ok(vec![path.to_path_buf()])
    }
}

/// Los archivos que el nodo acaba de producir: los de su carpeta que son productos.
fn collect(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !is_hidden(p) && Kind::Any.accepts(p))
        .collect();
    v.sort();
    v
}

/// Un nombre que sirve como nombre de carpeta.
fn folder_name(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let t = t.trim_matches('_').to_string();
    if t.is_empty() { "resultado".into() } else { t }
}

/// Un nombre de carpeta que todavía no existe en `base`.
fn free_name(base: &Path, name: &str) -> PathBuf {
    let first = base.join(name);
    if !first.exists() {
        return first;
    }
    let mut k = 2;
    loop {
        let p = base.join(format!("{name}_{k}"));
        if !p.exists() {
            return p;
        }
        k += 1;
    }
}

/// La línea de órdenes de un nodo de herramienta: las opciones que ha fijado, los archivos que
/// recibió y `-o` cuando la herramienta no puede elegir el nombre de su salida.
fn command_line(
    spec: &ToolSpec,
    node: &Node,
    args: &BTreeMap<String, Vec<String>>,
    sent: &HashMap<String, Vec<PathBuf>>,
    output: Option<&Path>,
) -> Vec<String> {
    let mut opts = Vec::new();
    let mut positional = Vec::new();
    let _ = node;
    for a in &spec.args {
        if a.id == "results_dir" {
            continue;
        }
        if a.id == "output" && !a.positional {
            if let (Some(out), Some(l)) = (output, &a.long) {
                opts.push(format!("--{l}"));
                opts.push(out.display().to_string());
            }
            continue;
        }
        let named = spec.name == "calc" && a.id == "inputs";
        let values: Vec<String> = match sent.get(&a.id) {
            // los rásteres de la calculadora se llaman A, B, C... en el orden en que se enviaron
            Some(files) if named && files.len() > 1 => files
                .iter()
                .zip('A'..='Z')
                .map(|(f, c)| format!("{c}={}", f.display()))
                .collect(),
            Some(files) if !files.is_empty() => files.iter().map(|f| f.display().to_string()).collect(),
            _ => args
                .get(&a.id)
                .map(|v| v.iter().filter(|s| !s.trim().is_empty()).cloned().collect())
                .unwrap_or_default(),
        };
        if values.is_empty() {
            continue;
        }
        if a.positional {
            positional.extend(values);
        } else if let Some(l) = &a.long {
            if a.flag {
                if values[0] == "true" {
                    opts.push(format!("--{l}"));
                }
            } else if a.multiple {
                for v in values {
                    opts.push(format!("--{l}"));
                    opts.push(v);
                }
            } else {
                opts.push(format!("--{l}"));
                opts.push(values[0].clone());
            }
        }
    }
    let mut line = opts;
    line.extend(positional);
    line
}

/// Ejecuta `exe` con `argv`, pasa sus líneas de salida a `log` y lo detiene cuando `cancel` está
/// activado.
/// La línea de órdenes de una herramienta con las opciones dadas (sin archivos enviados por
/// hilos), la misma que escribe el constructor de flujos.
pub fn tool_argv(spec: &ToolSpec, args: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let node = Node {
        id: String::new(),
        label: String::new(),
        kind: NodeKind::Input { path: String::new() },
        keep: false,
        x: 0.0,
        y: 0.0,
    };
    let mut v = vec![spec.name.clone()];
    v.extend(command_line(spec, &node, args, &HashMap::new(), None));
    v
}

pub fn spawn(exe: &Path, argv: &[String], cancel: &AtomicBool, log: &mut dyn FnMut(&str)) -> Result<()> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let mut child = Command::new(exe)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Param(format!("no se pudo lanzar {}: {e}", exe.display())))?;
    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(o) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for l in BufReader::new(o).lines().map_while(std::result::Result::ok) {
                let _ = tx.send(l);
            }
        }));
    }
    if let Some(o) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for l in BufReader::new(o).lines().map_while(std::result::Result::ok) {
                let _ = tx.send(l);
            }
        }));
    }
    drop(tx);
    let mut last_error = String::new();
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(l) => {
                if l.starts_with("error") {
                    last_error = l.clone();
                }
                log(&l);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            for r in readers {
                let _ = r.join();
            }
            return Err(Error::Param("cancelado".into()));
        }
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else if last_error.is_empty() {
        Err(Error::Param(format!("la orden terminó con {status}")))
    } else {
        Err(Error::Param(last_error))
    }
}

impl Model {
    /// Ejecuta el modelo: cada nodo en orden, cada uno como una orden de `hipso`. Los productos van
    /// a una carpeta de esta ejecución; cuando todo ha funcionado, los conservados pasan a la
    /// carpeta de resultados (nunca sobre algo que ya esté allí) y el resto se borra, y cuando algo
    /// falla o se cancela la ejecución se borra todo lo que esta escribió.
    pub fn run(
        &self,
        r: &Run,
        state: &mut dyn FnMut(&str, State),
        log: &mut dyn FnMut(&str),
    ) -> Result<Report> {
        // los nodos de entrada sustituidos al ejecutar cuentan ya para comprobar el modelo
        let mut replaced = self.clone();
        for n in &mut replaced.nodes {
            if let NodeKind::Input { path } = &mut n.kind
                && let Some(p) = r.inputs.get(&n.label).or_else(|| r.inputs.get(&n.id))
            {
                *path = p.clone();
            }
        }
        let this = &replaced;
        let problems = this.problems(r.schema);
        if let Some(p) = problems.first() {
            return Err(Error::Param(p.text.clone()));
        }
        let order = self
            .order()
            .ok_or_else(|| Error::Param("el flujo tiene un bucle".into()))?;
        std::fs::create_dir_all(r.results)?;
        let work = free_name(r.results, &format!(".flujo_{}", std::process::id()));
        std::fs::create_dir_all(&work)?;
        for n in &this.nodes {
            state(&n.id, State::Waiting);
        }
        let outcome = this.run_in(r, &work, &order, state, log);
        match outcome {
            Ok(products) => {
                let mut report = Report::default();
                for n in this.nodes.iter().filter(|n| n.keep) {
                    let Some(files) = products.get(&n.id) else { continue };
                    if matches!(n.kind, NodeKind::Input { .. }) || files.is_empty() {
                        continue;
                    }
                    let name = folder_name(&this.name(n));
                    let dest = free_name(r.results, &name);
                    let from = work.join(&n.id);
                    std::fs::rename(&from, &dest)?;
                    let moved = files
                        .iter()
                        .filter_map(|f| f.file_name().map(|n| dest.join(n)))
                        .collect();
                    report.outputs.push((n.id.clone(), moved));
                }
                let _ = std::fs::remove_dir_all(&work);
                Ok(report)
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&work);
                Err(e)
            }
        }
    }

    fn run_in(
        &self,
        r: &Run,
        work: &Path,
        order: &[usize],
        state: &mut dyn FnMut(&str, State),
        log: &mut dyn FnMut(&str),
    ) -> Result<HashMap<String, Vec<PathBuf>>> {
        let mut products: HashMap<String, Vec<PathBuf>> = HashMap::new();
        let mut pending: HashMap<&str, usize> = HashMap::new();
        for e in &self.edges {
            *pending.entry(e.from.as_str()).or_insert(0) += 1;
        }
        for &i in order {
            let n = &self.nodes[i];
            state(&n.id, State::Running);
            let made = match &n.kind {
                NodeKind::Input { path } => {
                    let path = r
                        .inputs
                        .get(&n.label)
                        .or_else(|| r.inputs.get(&n.id))
                        .unwrap_or(path);
                    files_of(Path::new(path))?
                }
                NodeKind::Tool { tool, args } => {
                    let spec = r
                        .schema
                        .tool(tool)
                        .ok_or_else(|| Error::Param(format!("«{tool}» no es una herramienta")))?;
                    let ent = entry(tool);
                    // los archivos enviados a cada opción, tomados por tipo
                    let mut sent: HashMap<String, Vec<PathBuf>> = HashMap::new();
                    for a in spec.args.iter().filter(|a| wirable(tool, a)) {
                        let want = arg_kind(tool, &a.id);
                        let mut files = Vec::new();
                        let mut wired = false;
                        for e in self.edges.iter().filter(|e| e.to == n.id && e.port == a.id) {
                            wired = true;
                            for f in products.get(&e.from).into_iter().flatten() {
                                if want.accepts(f) {
                                    files.push(f.clone());
                                }
                            }
                        }
                        if wired && files.is_empty() {
                            return Err(Error::Param(format!(
                                "«{}» no ha recibido archivos de tipo {}",
                                self.name(n),
                                want.name()
                            )));
                        }
                        if !files.is_empty() {
                            sent.insert(a.id.clone(), files);
                        }
                    }
                    // una opción que admite un archivo y recibió varios ejecuta la herramienta una
                    // vez por archivo
                    let many: Vec<String> = sent
                        .iter()
                        .filter(|(id, f)| f.len() > 1 && spec.arg(id).is_some_and(|a| !a.multiple))
                        .map(|(id, _)| id.clone())
                        .collect();
                    if many.len() > 1 {
                        return Err(Error::Param(format!(
                            "«{}» recibe varios archivos en más de una entrada que solo admite uno",
                            self.name(n)
                        )));
                    }
                    let dir = work.join(&n.id);
                    std::fs::create_dir_all(&dir)?;
                    let rounds: Vec<HashMap<String, Vec<PathBuf>>> = match many.first() {
                        Some(id) => sent[id]
                            .iter()
                            .map(|f| {
                                let mut s = sent.clone();
                                s.insert(id.clone(), vec![f.clone()]);
                                s
                            })
                            .collect(),
                        None => vec![sent],
                    };
                    for (k, round) in rounds.iter().enumerate() {
                        // una herramienta a la que hay que decir dónde escribir recibe un nombre en
                        // la carpeta del nodo
                        let output = spec
                            .output_arg()
                            .filter(|a| a.required)
                            .map(|_| {
                                // una reproyección conserva el tipo de archivo que recibió
                                let ext = round
                                    .values()
                                    .next()
                                    .and_then(|f| f.first())
                                    .and_then(|f| f.extension())
                                    .filter(|_| tool == "reproject")
                                    .map_or(ent.map_or("tif", |e| e.ext).to_string(), |e| e.to_string_lossy().into_owned());
                                dir.join(format!("{}_{}.{}", folder_name(tool), k + 1, ext))
                            });
                        let mut argv = vec!["--results-dir".to_string(), dir.display().to_string(), tool.clone()];
                        argv.extend(command_line(spec, n, args, round, output.as_deref()));
                        log(&format!("» {} {}", "hipso", argv.join(" ")));
                        spawn(r.exe, &argv, r.cancel, log)?;
                    }
                    collect(&dir)
                }
            };
            products.insert(n.id.clone(), made);
            state(&n.id, State::Done);
            // lo que ya no necesita nadie se borra para dejar libre el disco
            for e in self.edges.iter().filter(|e| e.to == n.id) {
                if let Some(c) = pending.get_mut(e.from.as_str()) {
                    *c -= 1;
                    if *c == 0
                        && let Some(src) = self.node(&e.from)
                        && !src.keep
                        && matches!(src.kind, NodeKind::Tool { .. })
                    {
                        let _ = std::fs::remove_dir_all(work.join(&src.id));
                    }
                }
            }
        }
        Ok(products)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        let p = |id: &str, positional: bool, required: bool, path: bool| ArgSpec {
            id: id.into(),
            long: (!positional).then(|| id.replace('_', "-")),
            positional,
            required,
            path,
            ..ArgSpec::default()
        };
        Schema {
            tools: vec![
                ToolSpec {
                    name: "chm".into(),
                    about: "x".into(),
                    args: vec![p("inputs", true, true, true), p("output", false, false, true)],
                },
                ToolSpec {
                    name: "zonal".into(),
                    about: "x".into(),
                    args: vec![p("raster", true, true, true), p("regions", false, true, true)],
                },
            ],
        }
    }

    fn tool(id: &str, name: &str, keep: bool) -> Node {
        Node {
            id: id.into(),
            label: String::new(),
            kind: NodeKind::Tool {
                tool: name.into(),
                args: BTreeMap::new(),
            },
            keep,
            x: 0.0,
            y: 0.0,
        }
    }

    fn input(id: &str, path: &str) -> Node {
        Node {
            id: id.into(),
            label: String::new(),
            kind: NodeKind::Input { path: path.into() },
            keep: false,
            x: 0.0,
            y: 0.0,
        }
    }

    #[test]
    fn the_schema_and_the_model_survive_their_text() -> Result<()> {
        let s = schema();
        assert_eq!(Schema::from_json(&s.to_json())?, s);
        let mut m = Model::default();
        m.nodes.push(input("n1", "/tmp/a \"b\".las"));
        let mut t = tool("n2", "chm", true);
        if let NodeKind::Tool { args, .. } = &mut t.kind {
            args.insert("cell".into(), vec!["0.5".into()]);
        }
        m.nodes.push(t);
        m.edges.push(Edge {
            from: "n1".into(),
            to: "n2".into(),
            port: "inputs".into(),
        });
        m.results = "res".into();
        assert_eq!(Model::from_json(&m.to_json())?, m);
        Ok(())
    }

    #[test]
    fn order_puts_each_node_after_its_sources_and_finds_loops() {
        let mut m = Model::default();
        m.nodes = vec![tool("b", "zonal", true), input("a", "x"), tool("c", "chm", false)];
        m.edges = vec![
            Edge { from: "a".into(), to: "c".into(), port: "inputs".into() },
            Edge { from: "c".into(), to: "b".into(), port: "raster".into() },
        ];
        let order = m.order().unwrap_or_default();
        let pos = |id: &str| order.iter().position(|&i| m.nodes[i].id == id);
        assert!(pos("a") < pos("c") && pos("c") < pos("b"));
        assert_eq!(m.depths(), vec![2, 0, 1]);
        m.edges.push(Edge { from: "b".into(), to: "c".into(), port: "inputs".into() });
        assert!(m.order().is_none());
        assert!(m.problems(&schema()).iter().any(|p| p.text.contains("bucle")));
    }

    #[test]
    fn problems_say_what_is_missing_and_what_cannot_be_connected() {
        let s = schema();
        let mut m = Model::default();
        m.nodes = vec![input("a", ""), tool("b", "zonal", false), tool("c", "nada", true)];
        let texts: Vec<String> = m.problems(&s).into_iter().map(|p| p.text).collect();
        assert!(texts.iter().any(|t| t.contains("no tiene archivo")));
        assert!(texts.iter().any(|t| t.contains("falta")));
        assert!(texts.iter().any(|t| t.contains("no es una herramienta")));
        // una herramienta ráster no puede recibir lo que produce una herramienta vectorial
        let mut m = Model::default();
        m.nodes = vec![tool("v", "buffer", false), tool("z", "chm", true)];
        m.edges = vec![Edge { from: "v".into(), to: "z".into(), port: "inputs".into() }];
        assert!(m.problems(&s).iter().any(|p| p.text.contains("no puede recibir")));
    }

    #[test]
    fn kinds_pick_main_files_and_skip_sidecars() {
        assert!(Kind::Vector.accepts(Path::new("a.SHP")));
        assert!(!Kind::Vector.accepts(Path::new("a.dbf")));
        assert!(Kind::Any.accepts(Path::new("a.laz")) && !Kind::Any.accepts(Path::new("a.prj")));
        assert!(Kind::Raster.takes(&[Kind::Cloud, Kind::Raster]) && !Kind::Raster.takes(&[Kind::Vector]));
    }

    /// Un modelo de dos pasos ejecutado con un sustituto de `hipso` (un guion de shell que escribe
    /// un archivo con el nombre de su orden) y después un fallo que no deja nada atrás.
    #[cfg(unix)]
    #[test]
    fn runs_in_order_keeps_the_marked_and_cleans_up_on_failure() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("hipso_model_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base)?;
        let exe = base.join("fake.sh");
        // argumentos: --results-dir DIR herramienta ...; la herramienta «chm» escribe DIR/chm.tif y
        // «zonal» falla
        std::fs::write(
            &exe,
            "#!/bin/sh\ndir=$2\ntool=$3\nif [ \"$tool\" = zonal ]; then echo 'error: no va' >&2; exit 1; fi\necho \"$@\" > \"$dir/$tool.tif\"\n",
        )?;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
        let las = base.join("a.laz");
        std::fs::write(&las, b"x")?;
        let s = schema();
        let mut m = Model::default();
        m.nodes = vec![input("n1", &las.display().to_string()), tool("n2", "chm", true)];
        m.edges = vec![Edge { from: "n1".into(), to: "n2".into(), port: "inputs".into() }];
        let results = base.join("res");
        let cancel = AtomicBool::new(false);
        let inputs = HashMap::new();
        let run = Run { exe: &exe, schema: &s, results: &results, inputs: &inputs, cancel: &cancel };
        let mut states = Vec::new();
        let report = m.run(&run, &mut |id, st| states.push((id.to_string(), st)), &mut |_| {})?;
        assert_eq!(report.outputs.len(), 1);
        let made = &report.outputs[0].1;
        assert_eq!(made.len(), 1);
        assert!(made[0].is_file());
        assert!(std::fs::read_to_string(&made[0])?.contains(&las.display().to_string()));
        assert!(states.contains(&("n2".to_string(), State::Done)));
        // en los resultados solo queda la carpeta conservada
        let left: Vec<_> = std::fs::read_dir(&results)?.flatten().collect();
        assert_eq!(left.len(), 1);
        // una segunda ejecución no sobrescribe la primera
        m.run(&run, &mut |_, _| {}, &mut |_| {})?;
        assert_eq!(std::fs::read_dir(&results)?.flatten().count(), 2);
        // un paso que falla borra lo que escribió la ejecución
        m.nodes.push(tool("n3", "zonal", true));
        m.edges.push(Edge { from: "n2".into(), to: "n3".into(), port: "raster".into() });
        if let NodeKind::Tool { args, .. } = &mut m.nodes[2].kind {
            args.insert("regions".into(), vec!["r.shp".into()]);
        }
        let r = m.run(&run, &mut |_, _| {}, &mut |_| {});
        assert!(matches!(r, Err(Error::Param(t)) if t.contains("no va")));
        assert_eq!(std::fs::read_dir(&results)?.flatten().count(), 2);
        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }
}
