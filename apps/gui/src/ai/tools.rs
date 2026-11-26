//! Las herramientas del asistente. Son pocas y de solo lectura (más la construcción de flujos), y
//! cada una devuelve poco: la documentación se consulta por apartados, los archivos se describen en
//! unas líneas y todo resultado se recorta. El asistente no ejecuta herramientas de HIPSO: arma el
//! flujo y la persona lo ejecuta.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hipso_core::json::Json;
use hipso_core::model::{self, Edge, Model, Node, NodeKind, Schema, ToolSpec};

use super::docs::{self, compact_options, cut, docs, excerpt};
use super::jsonw::{arr, obj, s, text};

/// Lo que el asistente puede ver del programa en el momento de preguntar.
#[derive(Clone, Default)]
pub struct Env {
    pub layers: Vec<LayerInfo>,
    pub schema: Option<Arc<Schema>>,
    /// Dónde se guardan los flujos que crea.
    pub flows_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerInfo {
    pub name: String,
    /// «nube de puntos», «ráster», «vector» o «tabla».
    pub kind: String,
    pub path: String,
    pub note: String,
}

#[derive(Default)]
pub struct Output {
    pub text: String,
    /// El flujo que se acaba de crear.
    pub flow: Option<PathBuf>,
}

impl From<String> for Output {
    fn from(text: String) -> Self {
        Output { text, flow: None }
    }
}

/// Lo máximo que devuelve una herramienta, en caracteres.
const CAP: usize = 4500;
/// El tamaño de cada parte de un apartado largo.
const PART: usize = 3500;

pub const OVERVIEW: &str = "HIPSO es un programa libre y gratuito de escritorio (Windows, macOS y Linux) para procesar nubes de puntos LiDAR forestales (aéreas y terrestres), rásteres y capas vectoriales. Está escrito en Rust. Tiene tres premisas, por este orden: (1) rapidez; (2) eficiencia con nubes enormes: los procesos leen la nube en flujo y la memoria depende del área y de la celda, no del número de puntos, y los pasos locales trabajan por teselas en paralelo; (3) edición cómoda y asequible: seleccionar, reclasificar y borrar con pocos clics, con deshacer y rehacer, y el original nunca se modifica.
Cada herramienta es una orden `hipso <herramienta>`; la interfaz gráfica y el constructor de flujos ejecutan esas mismas órdenes (hay 87).
Interfaz: ventanas en planta, perfil y 3D (también rásteres como superficie 3D), cinta con las pestañas Archivo, Vista, LiDAR, Vectorial, Raster y Flujos, y panel lateral con Contenido, Información, Edición y Simbología. Edición por selecciones (rectángulo, círculo, polígono, lazo, en planta, perfil o 3D), varita mágica, histogramas de atributos, EDA enlazado a la selección, tabla de atributos, digitalización y topología.
LiDAR: Preparar (clasificar, asignar proyección, exportar, recortar, adelgazar, alturas sobre el suelo, quitar duplicados, colorear, intensidad, propagar clase, editar con capas), Calidad (comparar nubes, ICP, entre pasadas, líneas de vuelo, densidad de Voronoi, unir nubes, validar MDT), Superficies (MDT, MDS/CHM), ALS (extraer muestras, métricas por celda, modelos lineales, segmentar con CHM, segmentar con puntos, métricas por árbol, huecos del dosel, métricas de vóxeles, análisis por objetos), TLS (medición de troncos) e Infraestructuras (corredores eléctricos). Clasificar es una cadena de pasos (solape, ruido, suelo con seis métodos, edificios, vegetación, cables, postes, carreteras, agua y reglas) que se guarda como receta.
Ráster: reproyectar/igualar/remuestrear (con altura vertical y geoide), mosaico, multibanda, índices espectrales, calculadora, reclasificar, cortar por máscara, tamizar, proximidad, vectorizar y análisis del terreno e hidrología. Vectorial: geometría, tabla y geoprocesos con comprobación y reparación de topología.
Formatos: LAS/LAZ/COPC/PLY/E57; GeoTIFF/COG/JPEG 2000/ENVI; shapefile, GeoJSON, KML, GeoPackage, CSV y XLSX.
Flujos: un grafo de nodos (cada herramienta es un nodo y cada archivo un hilo) que se guarda como .model.json y se ejecuta sin interfaz con `hipso run`. También hay un servidor MCP (`hipso mcp`) y este asistente.
Para profundizar en algo concreto, usa buscar_manual, leer_seccion y ayuda_herramienta.";

/// La definición de las herramientas, en el formato de la API de OpenAI.
pub fn definitions() -> Json {
    fn f(name: &str, description: &str, properties: Vec<(&str, Json)>, required: &[&str]) -> Json {
        obj(vec![
            ("type", s("function")),
            (
                "function",
                obj(vec![
                    ("name", s(name)),
                    ("description", s(description)),
                    (
                        "parameters",
                        obj(vec![
                            ("type", s("object")),
                            ("properties", Json::Obj(properties.into_iter().map(|(k, v)| (k.to_string(), v)).collect())),
                            ("required", arr(required.iter().map(|r| s(*r)).collect())),
                        ]),
                    ),
                ]),
            ),
        ])
    }
    fn p(kind: &str, desc: &str) -> Json {
        obj(vec![("type", s(kind)), ("description", s(desc))])
    }
    arr(vec![
        f("que_es_hipso", "Resumen de qué es HIPSO y qué hace. Para presentar el programa; para profundizar, buscar_manual.", vec![], &[]),
        f(
            "listar_herramientas",
            "Herramientas por grupo (orden y título). Con `grupo` (Nubes, Preparar, Superficies, Inventario, Terreno, Ráster, Vectorial) añade qué hace cada una.",
            vec![("grupo", p("string", "opcional"))],
            &[],
        ),
        f(
            "ayuda_herramienta",
            "Referencia exacta de una orden: qué hace, opciones con su valor por defecto, puertos de archivo y apartados del manual. Úsala antes de explicarla o de ponerla en un flujo.",
            vec![("nombre", p("string", "la orden, p. ej. chm, classify, tree-metrics")), ("parte", p("integer", "parte (1 por defecto)"))],
            &["nombre"],
        ),
        f(
            "buscar_manual",
            "Busca en el manual y devuelve los apartados más relevantes (id, ruta, extracto). Palabras clave concretas; luego leer_seccion.",
            vec![("consulta", p("string", "palabras clave")), ("n", p("integer", "3 por defecto, máx. 6"))],
            &["consulta"],
        ),
        f(
            "leer_seccion",
            "Lee un apartado por su id (de buscar_manual o ayuda_herramienta), por partes si es largo.",
            vec![("id", p("string", "p. ej. m812")), ("parte", p("integer", "1 por defecto"))],
            &["id"],
        ),
        f("capas_abiertas", "Capas abiertas en HIPSO con su ruta. Para preguntas sobre «mis datos».", vec![], &[]),
        f(
            "describir_archivo",
            "Describe una nube LAS/LAZ, un ráster o una capa/tabla en pocas líneas.",
            vec![("ruta", p("string", "nombre o ruta de una capa abierta (solo se leen las abiertas)"))],
            &["ruta"],
        ),
        f(
            "analisis_exploratorio",
            "Análisis exploratorio de una capa o tabla (estadísticos por campo, frecuentes, correlaciones); en ráster, estadísticos e histograma; en nube, su resumen.",
            vec![("ruta", p("string", "nombre o ruta de una capa abierta (solo se leen las abiertas)")), ("campos", arr_of_strings("campos (todos por defecto)"))],
            &["ruta"],
        ),
        f(
            "crear_flujo",
            "Construye un flujo y lo valida contra las herramientas reales (si devuelve errores, corrige y repite). Cada paso: `herramienta`, `opciones` (opción → valor) y `entradas` (puerto de archivo → id de una entrada o de otro paso). Opciones y puertos salen de ayuda_herramienta.",
            vec![
                ("nombre", p("string", "nombre del flujo")),
                (
                    "entradas",
                    obj(vec![
                        ("type", s("array")),
                        ("description", s("[{id, ruta}]; ruta vacía si la pone la persona")),
                        ("items", obj(vec![("type", s("object"))])),
                    ]),
                ),
                (
                    "pasos",
                    obj(vec![
                        ("type", s("array")),
                        ("description", s("[{id, herramienta, opciones:{}, entradas:{puerto:id|[ids]}, conservar:bool, titulo}]")),
                        ("items", obj(vec![("type", s("object"))])),
                    ]),
                ),
            ],
            &["nombre", "pasos"],
        ),
    ])
}

fn arr_of_strings(desc: &str) -> Json {
    obj(vec![("type", s("array")), ("description", s(desc)), ("items", obj(vec![("type", s("string"))]))])
}

/// Ejecuta una herramienta. Nunca falla: un error es el texto que el asistente lee y arregla.
pub fn run(name: &str, args: &Json, env: &Env) -> Output {
    let out: Output = match name {
        "que_es_hipso" => OVERVIEW.to_string().into(),
        "listar_herramientas" => list_tools(&text(args, "grupo")).into(),
        "ayuda_herramienta" => tool_help(&text(args, "nombre"), page(args)).into(),
        "buscar_manual" => search(&text(args, "consulta"), args.get("n").and_then(Json::as_f64).map_or(3, |v| v as usize)).into(),
        "leer_seccion" => read_section(&text(args, "id"), page(args)).into(),
        "capas_abiertas" => layers(env).into(),
        "describir_archivo" => {
            let given = text(args, "ruta");
            resolve(&given, env).map_or_else(|| not_open(&given), |p| describe(&p)).into()
        }
        "analisis_exploratorio" => {
            let fields: Vec<String> = args
                .get("campos")
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            let given = text(args, "ruta");
            resolve(&given, env).map_or_else(|| not_open(&given), |p| explore(&p, &fields)).into()
        }
        "crear_flujo" => return create_flow(args, env),
        other => format!("No existe la herramienta «{other}». Las que hay: que_es_hipso, listar_herramientas, ayuda_herramienta, buscar_manual, leer_seccion, capas_abiertas, describir_archivo, analisis_exploratorio y crear_flujo.").into(),
    };
    Output { text: cut(&out.text, CAP + 400), flow: out.flow }
}

fn page(args: &Json) -> usize {
    args.get("parte").and_then(Json::as_f64).map_or(1, |v| (v as usize).max(1))
}

// ---------- documentación ----------

fn list_tools(group: &str) -> String {
    let want = docs::fold(group);
    let mut by_group: Vec<(&str, Vec<&model::Entry>)> = Vec::new();
    for e in model::CATALOG {
        if !want.is_empty() && docs::fold(e.group) != want {
            continue;
        }
        match by_group.iter_mut().find(|(g, _)| *g == e.group) {
            Some((_, v)) => v.push(e),
            None => by_group.push((e.group, vec![e])),
        }
    }
    if by_group.is_empty() {
        return format!("No hay un grupo «{group}». Grupos: Nubes, Preparar, Superficies, Inventario, Terreno, Ráster, Vectorial.");
    }
    let d = docs();
    let mut out = String::new();
    for (g, list) in by_group {
        out.push_str(&format!("{g}:\n"));
        for e in list {
            if want.is_empty() {
                out.push_str(&format!("  {} ({})\n", e.tool, e.title));
            } else {
                let about = d
                    .tool(e.tool)
                    .and_then(|t| t.body.lines().find(|l| !l.trim().is_empty()))
                    .map_or(String::new(), |l| cut(l.trim(), 150));
                out.push_str(&format!("  {} ({}): {about}\n", e.tool, e.title));
            }
        }
    }
    out
}

fn tool_help(name: &str, part: usize) -> String {
    let d = docs();
    let Some(sec) = d.tool(name) else {
        let near: Vec<&str> = model::CATALOG
            .iter()
            .filter(|e| docs::fold(e.tool).contains(&docs::fold(name)) || docs::fold(e.title).contains(&docs::fold(name)))
            .map(|e| e.tool)
            .take(6)
            .collect();
        return format!("No hay una herramienta «{name}».{}", if near.is_empty() { " Usa listar_herramientas.".into() } else { format!(" ¿Quizá: {}?", near.join(", ")) });
    };
    let body = compact_options(&sec.body);
    let mut chunks = pages(&body, PART);
    let total = chunks.len();
    let mut out = if part <= total { chunks.swap_remove(part - 1) } else { String::from("No hay más partes.") };
    if part == 1 {
        let entry = model::entry(name.trim_start_matches("hipso ").trim());
        if let Some(e) = entry {
            out = format!("{} — grupo {}.\n{out}", e.title, e.group);
        }
    }
    if total > 1 {
        out.push_str(&format!("\n[parte {part}/{total}; pide parte {} para continuar]", part + 1));
    }
    if part == 1 {
        let q = format!("{} {}", entry_title(name), name);
        let related: Vec<String> = d.search(&q, 3).iter().map(|s| format!("{} «{}»", s.id, cut(&s.trail, 90))).collect();
        if !related.is_empty() {
            out.push_str(&format!("\nMás en el manual: {}", related.join("; ")));
        }
    }
    out
}

fn entry_title(name: &str) -> &'static str {
    model::entry(name.trim_start_matches("hipso ").trim()).map_or("", |e| e.title)
}

/// El texto en partes de como mucho `size` caracteres, cortadas en un salto de línea.
fn pages(text: &str, size: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if cur.chars().count() + line.chars().count() + 1 > size && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push_str(line);
        cur.push('\n');
    }
    if !cur.trim().is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

fn search(query: &str, n: usize) -> String {
    let d = docs();
    let hits = d.search(query, n.clamp(1, 6));
    if hits.is_empty() {
        return "Sin resultados. Prueba con otras palabras (el nombre de la herramienta o del concepto).".into();
    }
    hits.iter()
        .map(|s| format!("[{}] {} ({} car.)\n{}", s.id, cut(&s.trail, 140), s.body.chars().count(), excerpt(&s.body, query, 420)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn read_section(id: &str, part: usize) -> String {
    let d = docs();
    let Some(sec) = d.section(id.trim()) else {
        return format!("No existe el apartado «{id}». Usa un id de buscar_manual o de ayuda_herramienta (m… o c…).");
    };
    let body = if sec.id.starts_with('c') { compact_options(&sec.body) } else { sec.body.clone() };
    let chunks = pages(&body, PART);
    let total = chunks.len();
    let mut out = format!("[{}] {}\n", sec.id, sec.trail);
    out.push_str(chunks.get(part - 1).map_or("No hay más partes.", String::as_str));
    if total > 1 {
        out.push_str(&format!("\n[parte {part}/{total}]"));
    }
    if part == 1 {
        let kids = d.children(sec);
        if !kids.is_empty() {
            out.push_str(&format!(
                "\nSubapartados: {}",
                kids.iter().take(14).map(|k| format!("{} «{}»", k.id, cut(&k.title, 50))).collect::<Vec<_>>().join("; ")
            ));
        }
    }
    out
}

// ---------- datos ----------

fn layers(env: &Env) -> String {
    if env.layers.is_empty() {
        return "No hay ninguna capa abierta.".into();
    }
    env.layers
        .iter()
        .map(|l| format!("- {} [{}] {}{}", l.name, l.kind, l.path, if l.note.is_empty() { String::new() } else { format!(" — {}", l.note) }))
        .collect::<Vec<_>>()
        .join("\n")
}

/// El archivo de la capa abierta que se llama así o que tiene esa ruta. Solo se leen las capas
/// abiertas: lo que se lee se envía al servicio, así que el modelo no puede pedir cualquier archivo
/// del equipo.
fn resolve(given: &str, env: &Env) -> Option<PathBuf> {
    let g = given.trim();
    env.layers
        .iter()
        .find(|l| l.name == g || l.path == g)
        .map(|l| PathBuf::from(&l.path))
}

/// Lo que se contesta cuando se pide un archivo que no está abierto.
fn not_open(given: &str) -> String {
    format!("«{}» no es una capa abierta, y solo se leen las abiertas. Pide a la persona que la abra en HIPSO (capas_abiertas dice cuáles hay).", given.trim())
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn num(v: f64) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let a = v.abs();
    if a == 0.0 {
        "0".into()
    } else if a >= 1e6 || a < 1e-3 {
        format!("{v:.3e}")
    } else if a >= 100.0 {
        format!("{v:.1}")
    } else if a >= 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.4}")
    }
}

fn describe(path: &Path) -> String {
    if !path.exists() {
        return format!("No existe {}.", path.display());
    }
    match ext(path).as_str() {
        "las" | "laz" => cloud_summary(path),
        "tif" | "tiff" | "asc" | "jp2" | "hgt" => raster_summary(path),
        "csv" | "txt" | "tsv" | "xlsx" => match hipso_core::view::read_table(path) {
            Ok(d) => table_summary(&d, "tabla"),
            Err(e) => format!("No se pudo leer la tabla: {e}"),
        },
        _ => match hipso_core::view::read_vector(path) {
            Ok(d) => table_summary(&d, "capa vectorial"),
            Err(e) => format!("No se pudo leer: {e}"),
        },
    }
}

fn cloud_summary(path: &Path) -> String {
    match hipso_core::inspect::inspect(path) {
        Ok(r) => cut(&hipso_core::inspect::render(&r), 3000),
        Err(e) => format!("No se pudo leer la nube: {e}"),
    }
}

fn raster_summary(path: &Path) -> String {
    let g = match hipso_core::grid::Grid::read(path) {
        Ok(g) => g,
        Err(e) => return format!("No se pudo leer el ráster: {e}"),
    };
    let epsg = hipso_core::grid::Grid::read_epsg(path).map_or("no declarado".to_string(), |e| format!("EPSG:{e}"));
    let mut out = format!(
        "Ráster {} x {} celdas de {} m, esquina ({:.2}, {:.2}), sistema {epsg}.\n",
        g.cols, g.rows, num(g.cell), g.x0, g.y0
    );
    match hipso_core::rasterproc::stats(&g, 10) {
        Ok(st) => out.push_str(&hipso_core::rasterproc::stats_report(&st)),
        Err(e) => out.push_str(&e.to_string()),
    }
    out
}

fn table_summary(d: &hipso_core::view::VectorData, what: &str) -> String {
    let kind = if d.polygons {
        "polígonos"
    } else if d.points {
        "puntos"
    } else {
        "líneas"
    };
    let mut out = format!("{what}: {} elementos ({kind}), {} campos.\n", d.features.len(), d.fields.len());
    if d.bounds != [0.0; 4] && d.bounds[0].is_finite() {
        out.push_str(&format!("Extensión: x {:.2}..{:.2}, y {:.2}..{:.2}.\n", d.bounds[0], d.bounds[2], d.bounds[1], d.bounds[3]));
    }
    for (c, name) in d.fields.iter().enumerate().take(40) {
        out.push_str(&format!("- {name} ({:?})\n", d.field_type(c)));
    }
    if d.fields.len() > 40 {
        out.push_str(&format!("… y {} campos más\n", d.fields.len() - 40));
    }
    out
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let r = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (r.floor() as usize, r.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (r - lo as f64)
}

fn pearson(x: &[f64], y: &[f64]) -> Option<(f64, usize)> {
    let pairs: Vec<(f64, f64)> = x.iter().zip(y).filter(|(a, b)| a.is_finite() && b.is_finite()).map(|(a, b)| (*a, *b)).collect();
    if pairs.len() < 10 {
        return None;
    }
    let n = pairs.len() as f64;
    let (mx, my) = (pairs.iter().map(|p| p.0).sum::<f64>() / n, pairs.iter().map(|p| p.1).sum::<f64>() / n);
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for (a, b) in &pairs {
        sxx += (a - mx).powi(2);
        syy += (b - my).powi(2);
        sxy += (a - mx) * (b - my);
    }
    (sxx > 0.0 && syy > 0.0).then(|| (sxy / (sxx * syy).sqrt(), pairs.len()))
}

fn explore(path: &Path, only: &[String]) -> String {
    use hipso_core::fieldtype::FieldType;
    if !path.exists() {
        return format!("No existe {}.", path.display());
    }
    match ext(path).as_str() {
        "las" | "laz" => return cloud_summary(path),
        "tif" | "tiff" | "asc" | "jp2" | "hgt" => {
            return match hipso_core::grid::Grid::read(path).map_err(|e| e.to_string()).and_then(|g| hipso_core::rasterproc::stats(&g, 10).map_err(|e| e.to_string())) {
                Ok(st) => format!("{}\nHistograma (10 intervalos): {:?}", hipso_core::rasterproc::stats_report(&st), st.histogram),
                Err(e) => format!("No se pudo analizar el ráster: {e}"),
            };
        }
        _ => {}
    }
    let d = match if matches!(ext(path).as_str(), "csv" | "txt" | "tsv" | "xlsx") { hipso_core::view::read_table(path) } else { hipso_core::view::read_vector(path) } {
        Ok(d) => d,
        Err(e) => return format!("No se pudo leer: {e}"),
    };
    let rows = d.features.len();
    let wanted: Vec<usize> = (0..d.fields.len())
        .filter(|&c| only.is_empty() || only.iter().any(|o| o.eq_ignore_ascii_case(&d.fields[c])))
        .take(25)
        .collect();
    let mut out = format!("{rows} elementos, {} campos.\n", d.fields.len());
    let mut numeric: Vec<(usize, Vec<f64>)> = Vec::new();
    for &c in &wanted {
        let name = &d.fields[c];
        let ty = d.field_type(c);
        if matches!(ty, FieldType::Integer | FieldType::Real) {
            let col: Vec<f64> = (0..rows).map(|r| d.num.get(r).and_then(|v| v.get(c)).copied().unwrap_or(f64::NAN)).collect();
            let mut v: Vec<f64> = col.iter().copied().filter(|x| x.is_finite()).collect();
            if v.is_empty() {
                out.push_str(&format!("- {name}: sin valores numéricos ({rows} vacíos)\n"));
                continue;
            }
            v.sort_by(f64::total_cmp);
            let n = v.len() as f64;
            let mean = v.iter().sum::<f64>() / n;
            let sd = if v.len() > 1 { (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt() } else { 0.0 };
            let skew = if sd > 0.0 && v.len() > 2 { v.iter().map(|x| ((x - mean) / sd).powi(3)).sum::<f64>() * n / ((n - 1.0) * (n - 2.0)) } else { 0.0 };
            out.push_str(&format!(
                "- {name}: n {} (vacíos {}), media {}, sd {}, mín {}, Q1 {}, mediana {}, Q3 {}, máx {}, asimetría {}\n",
                v.len(),
                rows - v.len(),
                num(mean),
                num(sd),
                num(v[0]),
                num(quantile(&v, 0.25)),
                num(quantile(&v, 0.5)),
                num(quantile(&v, 0.75)),
                num(v[v.len() - 1]),
                num(skew)
            ));
            numeric.push((c, col));
        } else {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            let mut empty = 0;
            for r in 0..rows {
                let t = d.text.get(r).and_then(|v| v.get(c)).map_or("", String::as_str).trim();
                if t.is_empty() {
                    empty += 1;
                } else {
                    *counts.entry(t).or_insert(0) += 1;
                }
            }
            let mut top: Vec<(&str, usize)> = counts.iter().map(|(k, v)| (*k, *v)).collect();
            top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            out.push_str(&format!(
                "- {name} ({ty:?}): {} valores distintos, vacíos {empty}; más frecuentes: {}\n",
                counts.len(),
                top.iter().take(5).map(|(k, v)| format!("{} ({v})", cut(k, 24))).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    let mut corr: Vec<(f64, String)> = Vec::new();
    for (i, (ci, a)) in numeric.iter().enumerate().take(12) {
        for (cj, b) in numeric.iter().take(12).skip(i + 1) {
            if let Some((r, n)) = pearson(a, b)
                && r.abs() >= 0.6
            {
                corr.push((r.abs(), format!("{} ~ {}: r = {:.2} (n {n})", d.fields[*ci], d.fields[*cj], r)));
            }
        }
    }
    corr.sort_by(|a, b| b.0.total_cmp(&a.0));
    if !corr.is_empty() {
        out.push_str(&format!("Correlaciones fuertes (|r| ≥ 0,6): {}\n", corr.iter().take(8).map(|c| c.1.as_str()).collect::<Vec<_>>().join("; ")));
    }
    if d.fields.len() > wanted.len() && only.is_empty() {
        out.push_str(&format!("({} campos más sin analizar; pide `campos` para elegirlos)\n", d.fields.len() - wanted.len()));
    }
    out
}

// ---------- flujos ----------

/// El id de una opción tal como lo escribe la persona o el modelo: `--ws-a`, `ws_a`…
fn norm_id(s: &str) -> String {
    s.trim().trim_start_matches('-').replace('-', "_").to_lowercase()
}

fn find_arg<'a>(spec: &'a ToolSpec, key: &str) -> Option<&'a model::ArgSpec> {
    let k = norm_id(key);
    spec.args.iter().find(|a| a.id.to_lowercase() == k || a.long.as_deref().is_some_and(|l| norm_id(l) == k))
}

fn values_of(v: &Json) -> Vec<String> {
    match v {
        Json::Str(x) => vec![x.clone()],
        Json::Num(n) => vec![if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", *n as i64) } else { n.to_string() }],
        Json::Bool(b) => vec![b.to_string()],
        Json::Arr(a) => a.iter().flat_map(values_of).collect(),
        _ => Vec::new(),
    }
}

fn slug(name: &str) -> String {
    let t: String = docs::fold(name).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let t = t.trim_matches('_').to_string();
    if t.is_empty() { "flujo".into() } else { t.chars().take(40).collect() }
}

/// Convierte la descripción del asistente en un modelo; los errores son frases que él puede corregir.
pub fn build_model(args: &Json, schema: Option<&Schema>) -> Result<Model, Vec<String>> {
    let mut errors = Vec::new();
    let mut m = Model::default();
    let mut ids: Vec<String> = Vec::new();
    for e in args.get("entradas").and_then(Json::as_array).unwrap_or(&[]) {
        let id = text(e, "id");
        if !model::valid_id(&id) || ids.contains(&id) {
            errors.push(format!("Una entrada necesita un id único de letras sin tilde, cifras, «-» o «_» (no vale: «{id}»)."));
            continue;
        }
        ids.push(id.clone());
        m.nodes.push(Node { id, label: text(e, "titulo"), kind: NodeKind::Input { path: text(e, "ruta") }, keep: false, x: 0.0, y: 0.0 });
    }
    let steps = args.get("pasos").and_then(Json::as_array).unwrap_or(&[]);
    if steps.is_empty() {
        errors.push("Hace falta al menos un paso en `pasos`.".into());
    }
    // los ids de los pasos primero, para que un paso pueda recibir de otro anterior o posterior
    for st in steps {
        let id = text(st, "id");
        if !model::valid_id(&id) || ids.contains(&id) {
            errors.push(format!("Un paso necesita un id único de letras sin tilde, cifras, «-» o «_» (no vale: «{id}»)."));
        } else {
            ids.push(id);
        }
    }
    let mut consumed: Vec<String> = Vec::new();
    for st in steps {
        let id = text(st, "id");
        let tool = text(st, "herramienta");
        let spec = schema.and_then(|s| s.tool(&tool));
        if schema.is_some() && spec.is_none() {
            let near: Vec<&str> = model::CATALOG.iter().filter(|e| e.tool.contains(tool.as_str()) || tool.contains(e.tool)).map(|e| e.tool).take(4).collect();
            errors.push(format!("«{tool}» no es una herramienta de HIPSO{}.", if near.is_empty() { String::new() } else { format!(" (¿{}?)", near.join(", ")) }));
            continue;
        }
        let mut options: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if let Some(Json::Obj(members)) = st.get("opciones") {
            for (k, v) in members {
                match spec.map(|sp| find_arg(sp, k)) {
                    Some(Some(a)) if !a.path || a.id == "output" => {
                        options.insert(a.id.clone(), values_of(v));
                    }
                    Some(Some(a)) => errors.push(format!("«{k}» de {tool} es un archivo: va en `entradas` (puerto «{}»), no en `opciones`.", a.id)),
                    Some(None) => {
                        let valid: Vec<String> = spec.map_or(Vec::new(), |sp| sp.args.iter().filter(|a| !a.path).take(30).map(|a| a.id.clone()).collect());
                        errors.push(format!("{tool} no tiene la opción «{k}». Opciones: {}.", valid.join(", ")));
                    }
                    None => {
                        options.insert(norm_id(k), values_of(v));
                    }
                }
            }
        }
        if let Some(Json::Obj(members)) = st.get("entradas") {
            for (port, v) in members {
                let port_id = match spec.map(|sp| find_arg(sp, port)) {
                    Some(Some(a)) if a.path => a.id.clone(),
                    Some(Some(_)) | Some(None) => {
                        let valid: Vec<String> = spec.map_or(Vec::new(), |sp| sp.args.iter().filter(|a| a.path && a.id != "output" && a.id != "results_dir").map(|a| a.id.clone()).collect());
                        errors.push(format!("{tool} no tiene el puerto de archivo «{port}». Puertos: {}.", valid.join(", ")));
                        continue;
                    }
                    None => norm_id(port),
                };
                for from in values_of(v) {
                    if !ids.contains(&from) || from == id {
                        errors.push(format!("El paso «{id}» recibe de «{from}», que no existe (o es él mismo). Ids: {}.", ids.join(", ")));
                        continue;
                    }
                    consumed.push(from.clone());
                    m.edges.push(Edge { from, to: id.clone(), port: port_id.clone() });
                }
            }
        }
        m.nodes.push(Node { id, label: text(st, "titulo"), kind: NodeKind::Tool { tool, args: options }, keep: false, x: 0.0, y: 0.0 });
    }
    // el resultado de un paso se conserva si se pide, o si nadie más lo usa; lo que otro paso consume
    // y no se pidió conservar se borra al acabar
    for st in steps {
        let id = text(st, "id");
        let asked = st.get("conservar").map(|v| matches!(v, Json::Bool(true)));
        if let Some(n) = m.nodes.iter_mut().find(|n| n.id == id && matches!(n.kind, NodeKind::Tool { .. })) {
            n.keep = asked.unwrap_or(!consumed.contains(&id));
        }
    }
    if errors.is_empty() { Ok(m) } else { Err(errors) }
}

fn create_flow(args: &Json, env: &Env) -> Output {
    let name = text(args, "nombre");
    let schema = env.schema.as_deref();
    let m = match build_model(args, schema) {
        Ok(m) => m,
        Err(errs) => {
            return Output { text: format!("El flujo tiene errores; corrígelos y llama de nuevo:\n- {}", errs.join("\n- ")), flow: None };
        }
    };
    let problems: Vec<String> = schema.map(|sc| m.problems(sc).into_iter().map(|p| p.text).collect()).unwrap_or_default();
    let Some(dir) = &env.flows_dir else {
        return "No hay una carpeta donde guardar el flujo.".to_string().into();
    };
    if std::fs::create_dir_all(dir).is_err() {
        return format!("No se pudo crear la carpeta {}.", dir.display()).into();
    }
    let mut path = dir.join(format!("{}.model.json", slug(&name)));
    let mut k = 2;
    while path.exists() {
        path = dir.join(format!("{}_{k}.model.json", slug(&name)));
        k += 1;
    }
    if let Err(e) = std::fs::write(&path, m.to_json()) {
        return format!("No se pudo guardar el flujo: {e}").into();
    }
    let mut out = format!("Flujo «{name}» creado: {} nodos y {} hilos, guardado en {}.", m.nodes.len(), m.edges.len(), path.display());
    if problems.is_empty() {
        out.push_str(" Sin problemas: está listo para ejecutar.");
    } else {
        out.push_str(&format!(
            " Antes de ejecutarlo hay que resolver (lo normal es que falten las rutas de los archivos, que la persona pone en el nodo de entrada):\n- {}",
            problems.join("\n- ")
        ));
    }
    out.push_str("\nLa persona lo abre con el botón «Abrir en Flujos» bajo tu respuesta.");
    Output { text: out, flow: Some(path) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Option<Schema> {
        // el esquema de las órdenes sale del ejecutable `hipso` si está compilado junto a las pruebas
        let exe = std::env::current_exe().ok()?;
        let hipso = exe.parent()?.parent()?.join(if cfg!(windows) { "hipso.exe" } else { "hipso" });
        let out = std::process::Command::new(hipso).arg("schema").output().ok()?;
        Schema::from_json(&String::from_utf8(out.stdout).ok()?).ok()
    }

    #[test]
    fn the_tool_definitions_are_valid_json_with_every_tool() -> hipso_core::Result<()> {
        let j = Json::parse(&super::super::jsonw::dump(&definitions()))?;
        let names: Vec<String> = j.as_array().unwrap_or(&[]).iter().filter_map(|t| t.get("function").map(|f| text(f, "name"))).collect();
        assert_eq!(names.len(), 9);
        assert!(names.contains(&"crear_flujo".to_string()) && names.contains(&"buscar_manual".to_string()));
        // el prompt de las definiciones es pequeño: unos pocos miles de caracteres
        assert!(super::super::jsonw::dump(&definitions()).chars().count() < 3800);
        Ok(())
    }

    #[test]
    fn documentation_tools_answer_from_the_embedded_docs() {
        let env = Env::default();
        let h = run("ayuda_herramienta", &Json::Obj(vec![("nombre".into(), Json::Str("canopy-gaps".into()))]), &env).text;
        assert!(h.contains("--height") && h.contains("Huecos del dosel"), "{h}");
        let l = run("listar_herramientas", &Json::Obj(vec![]), &env).text;
        assert!(l.contains("trees3d") && l.contains("dtm-validate"));
        let g = run("listar_herramientas", &Json::Obj(vec![("grupo".into(), Json::Str("inventario".into()))]), &env).text;
        assert!(g.contains("canopy-gaps:") || g.contains("canopy-gaps ("), "{g}");
        let b = run("buscar_manual", &Json::Obj(vec![("consulta".into(), Json::Str("servidor MCP".into()))]), &env).text;
        assert!(b.contains("MCP"), "{b}");
        let o = run("que_es_hipso", &Json::Obj(vec![]), &env).text;
        assert!(o.contains("tres premisas") || o.contains("Tiene tres premisas"));
        assert!(run("nada", &Json::Obj(vec![]), &env).text.contains("No existe la herramienta"));
        // los resultados nunca pasan del tope
        let big = run("leer_seccion", &Json::Obj(vec![("id".into(), Json::Str("m10".into()))]), &env).text;
        assert!(big.chars().count() <= CAP + 400);
    }

    #[test]
    fn a_flow_is_built_validated_and_saved() -> hipso_core::Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_ai_flows_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let schema = schema().map(Arc::new);
        let env = Env { layers: Vec::new(), schema: schema.clone(), flows_dir: Some(dir.clone()) };
        let args = Json::parse(
            r#"{"nombre":"MDT y CHM","entradas":[{"id":"nube","ruta":""}],
            "pasos":[{"id":"mdt","herramienta":"dtm","opciones":{"cell":1},"entradas":{"inputs":"nube"}},
                     {"id":"chm","herramienta":"chm","opciones":{"--cell":"0.5"},"entradas":{"inputs":"nube","dtm":"mdt"}}]}"#,
        )?;
        let out = create_flow(&args, &env);
        assert!(out.flow.is_some(), "{}", out.text);
        let saved = std::fs::read_to_string(out.flow.as_deref().unwrap_or(Path::new("")))?;
        let m = Model::from_json(&saved)?;
        assert_eq!(m.nodes.len(), 3);
        assert_eq!(m.edges.len(), 3);
        // el MDT lo usa el CHM y no se conserva; el CHM sí
        assert!(!m.node("mdt").is_some_and(|n| n.keep));
        assert!(m.node("chm").is_some_and(|n| n.keep));
        if schema.is_some() {
            // la ruta vacía de la nube es lo que falta, y se dice
            assert!(out.text.contains("no tiene archivo"), "{}", out.text);
        }
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn a_wrong_flow_gets_errors_the_model_can_fix() -> hipso_core::Result<()> {
        let Some(schema) = schema() else { return Ok(()) };
        let env = Env { layers: Vec::new(), schema: Some(Arc::new(schema)), flows_dir: Some(std::env::temp_dir()) };
        for (json, expect) in [
            (r#"{"nombre":"x","pasos":[{"id":"a","herramienta":"nada"}]}"#, "no es una herramienta"),
            (r#"{"nombre":"x","pasos":[{"id":"a","herramienta":"chm","opciones":{"celda":1}}]}"#, "no tiene la opción"),
            (r#"{"nombre":"x","pasos":[{"id":"a","herramienta":"chm","entradas":{"inputs":"fantasma"}}]}"#, "no existe"),
            (r#"{"nombre":"x","pasos":[{"id":"a","herramienta":"chm","opciones":{"inputs":"x.laz"}}]}"#, "va en `entradas`"),
            (r#"{"nombre":"x"}"#, "al menos un paso"),
        ] {
            let out = create_flow(&Json::parse(json)?, &env);
            assert!(out.flow.is_none() && out.text.contains(expect), "{json}: {}", out.text);
        }
        Ok(())
    }

    #[test]
    fn exploratory_analysis_of_a_table() -> std::io::Result<()> {
        let f = std::env::temp_dir().join(format!("hipso_ai_eda_{}.csv", std::process::id()));
        let mut t = String::from("especie,altura,dap,vol\n");
        for k in 0..30 {
            t.push_str(&format!("{},{},{},{}\n", if k % 3 == 0 { "pino" } else { "encina" }, 10 + k, 5 + k / 2, 2 * (10 + k) + 1));
        }
        std::fs::write(&f, t)?;
        let out = explore(&f, &[]);
        assert!(out.contains("30 elementos") && out.contains("altura: n 30"), "{out}");
        assert!(out.contains("especie") && out.contains("encina (20)"), "{out}");
        assert!(out.contains("Correlaciones fuertes") && out.contains("altura ~ vol"), "{out}");
        let one = explore(&f, &["altura".to_string()]);
        assert!(!one.contains("dap:"));
        let _ = std::fs::remove_file(f);
        assert!(describe(Path::new("/no/existe.tif")).contains("No existe"));
        Ok(())
    }

    #[test]
    fn layer_names_resolve_to_paths() {
        let env = Env { layers: vec![LayerInfo { name: "chm.tif".into(), kind: "ráster".into(), path: "/d/chm.tif".into(), note: String::new() }], ..Env::default() };
        assert_eq!(resolve("chm.tif", &env), Some(PathBuf::from("/d/chm.tif")));
        assert_eq!(resolve("/otra/ruta.laz", &env), None);
        assert!(layers(&env).contains("[ráster]"));
        assert_eq!(layers(&Env::default()), "No hay ninguna capa abierta.");
    }
}
