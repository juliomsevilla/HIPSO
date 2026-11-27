//! La ejecución de un flujo, pensada para entregas de cientos de miles de teselas.
//!
//! El trabajo de cada nodo se parte en **unidades** (una por tesela cuando la herramienta trata
//! cada archivo por su cuenta), cada una un proceso `hipso` que escribe en una carpeta provisional.
//! Solo lo que termina bien pasa a la carpeta del nodo y queda apuntado en un **diario**, de modo
//! que un corte de luz, una cancelación o un disco lleno no dejan archivos a medias que parezcan
//! buenos, y la ejecución se puede reanudar donde se quedó. Un archivo en mal estado hace fallar su
//! unidad, que se anota en `errores.csv`, y las demás siguen.
//!
//! Los nodos consecutivos que tratan cada tesela por su cuenta forman una **tanda**: cada tesela
//! recorre la cadena entera antes de empezar otra, y sus productos intermedios se borran en cuanto
//! nadie los necesita, de modo que el disco que hace falta no crece con el tamaño de la entrega. Se
//! tratan varias teselas a la vez, tantas como admitan los núcleos y la memoria.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use crate::json::{Json, quote};
use crate::model::{
    Kind, Model, Node, NodeKind, Report, Run, State, ToolSpec, arg_kind, command_line, entry,
    files_of, folder_name, free_name, is_hidden, spawn_with, wirable,
};
use crate::paths::NameIndex;
use crate::{Error, Result};

/// El diario de una ejecución, dentro de su carpeta.
pub const JOURNAL: &str = "flujo.estado";
/// La lista de lo que ha fallado.
pub const ERRORS: &str = "errores.csv";
/// El registro completo de lo que han dicho las órdenes.
pub const LOG: &str = "registro.log";
/// La carpeta provisional de la unidad en marcha, dentro de la del nodo.
const STAGING: &str = ".parcial";
/// Con tantos fallos seguidos el problema no es de un archivo (opciones, disco, red): se para.
const MAX_IN_A_ROW: usize = 25;
/// Hasta cuántas unidades se muestra todo lo que dice cada una; con más, el avance y los errores.
const CHATTY: usize = 20;

/// Las órdenes que tratan cada nube de `inputs` por su cuenta.
const PER_FILE: &[&str] = &[
    "info", "classify", "thin", "dedup", "crs", "hag", "colorize", "intensity", "copc",
    "propagate-class", "apply-edits", "dtm", "chm", "dsm", "metrics", "trees3d", "voxel-metrics",
    "stem-volume",
];

/// Las que además saben leer las hojas vecinas y unir un mosaico: entonces necesitan el bloque
/// entero en un solo proceso.
const BY_SHEET: &[&str] = &["classify", "dtm", "dsm", "chm", "metrics"];

/// Las que escriben archivos cuyo nombre no sale del de la tesela (los valores de un campo): dos a
/// la vez podrían repartirse los sufijos de otra manera en cada ejecución.
const SERIAL: &[&str] = &["split"];

/// La variable que fija cuántos hilos usa cada proceso.
const THREADS_ENV: &str = "RAYON_NUM_THREADS";

/// Los archivos de una entrada, compartidos entre las unidades que los reciben todos.
type Files = Arc<Vec<PathBuf>>;

/// Un proceso que hay que lanzar.
#[derive(Clone)]
struct Unit {
    ports: BTreeMap<String, Files>,
    /// Con qué se la nombra en los mensajes y en la lista de errores.
    label: String,
    /// El bloque entero con vecinas: la orden avisa de cada hoja que termina.
    block: bool,
}

/// Cómo se reparte el trabajo de un nodo.
struct Plan {
    units: Vec<Unit>,
    /// Las que no se pueden hacer porque falta una de sus entradas, con el motivo.
    missing: Vec<(String, String)>,
}

fn label_of(f: Option<&PathBuf>) -> String {
    f.map_or(String::new(), |f| f.display().to_string())
}

/// Si la herramienta necesita ver juntas todas las hojas.
fn needs_block(tool: &str, args: &BTreeMap<String, Vec<String>>) -> bool {
    BY_SHEET.contains(&tool)
        && (args
            .get("neighbors")
            .and_then(|v| v.first())
            .and_then(|v| v.trim().parse::<f64>().ok())
            .is_some_and(|v| v > 0.0)
            || args.get("mosaic").and_then(|v| v.first()).is_some_and(|v| v == "true"))
}

/// Reparte lo que ha recibido un nodo en unidades. Se parte por las nubes cuando la orden las trata
/// una a una, y por cualquier opción que admite un solo archivo y ha recibido varios; cuando son
/// varias, la primera guía y las demás se emparejan por el nombre.
fn plan(
    spec: &ToolSpec,
    args: &BTreeMap<String, Vec<String>>,
    sent: BTreeMap<String, Files>,
    wires: &[Files],
) -> Result<Plan> {
    let tool = spec.name.as_str();
    let mut out = Plan {
        units: Vec::new(),
        missing: Vec::new(),
    };
    if tool == "calc" && calc_by_sheet(args, wires) {
        let Some((first, others)) = wires.split_first() else {
            return Ok(out);
        };
        let indexes: Vec<NameIndex> = others.iter().map(|w| NameIndex::new(w)).collect();
        for f in first.iter() {
            let mut files = vec![f.clone()];
            files.extend(indexes.iter().filter_map(|i| i.pair(f).cloned()));
            if files.len() == wires.len() {
                out.units.push(Unit {
                    ports: BTreeMap::from([("inputs".to_string(), Arc::new(files))]),
                    label: label_of(Some(f)),
                    block: false,
                });
            } else {
                out.missing.push((
                    label_of(Some(f)),
                    "no hay en cada hilo de la calculadora un ráster con su nombre".into(),
                ));
            }
        }
        return Ok(out);
    }
    let block = needs_block(tool, args);
    // en el orden de la orden, para que la primera entrada guíe el emparejado
    let per_file = !block && PER_FILE.contains(&tool);
    let several = |id: &str| sent.get(id).is_some_and(|f| f.len() > 1);
    // una orden que trata cada nube por su cuenta y ha recibido varios MDT: cada nube es una unidad
    // y lleva el suyo, aunque la orden admita repetir la opción
    let paired_dtm = per_file && several("dtm") && sent.contains_key("inputs");
    let mut split: Vec<&str> = spec
        .args
        .iter()
        .filter(|a| {
            let each = a.id == "inputs" && per_file && (several(&a.id) || paired_dtm);
            let dtm = a.id == "dtm" && paired_dtm;
            each || dtm || several(&a.id) && !a.multiple && !block
        })
        .map(|a| a.id.as_str())
        .collect();
    if paired_dtm {
        // las nubes guían
        split.sort_by_key(|id| *id != "inputs");
    }
    if split.is_empty() {
        let label = sent
            .get("inputs")
            .or_else(|| sent.values().next())
            .and_then(|f| f.first())
            .cloned();
        out.units.push(Unit {
            ports: sent,
            label: label_of(label.as_ref()),
            block,
        });
        return Ok(out);
    }
    let lead = split.remove(0);
    let leads = sent.get(lead).cloned().unwrap_or_default();
    let lead_index = NameIndex::new(&leads);
    let others: Vec<(&str, NameIndex)> = split
        .iter()
        .map(|id| (*id, NameIndex::new(sent.get(*id).map_or(&[][..], |f| f.as_slice()))))
        .collect();
    // una opción que admite varios (las nubes) se reparte entre las vueltas cuando los nombres lo
    // permiten para todas; si no, cada una los recibe todos
    let mut shares: Vec<(&str, HashMap<&PathBuf, Vec<PathBuf>>)> = Vec::new();
    for (id, files) in &sent {
        if id == lead || split.contains(&id.as_str()) || files.len() < 2 {
            continue;
        }
        let mut by: HashMap<&PathBuf, Vec<PathBuf>> = HashMap::new();
        for f in files.iter() {
            if let Some(own) = lead_index.pair(f) {
                by.entry(own).or_default().push(f.clone());
            }
        }
        if leads.iter().all(|l| by.contains_key(l)) {
            shares.push((id.as_str(), by));
        }
    }
    'leads: for f in leads.iter() {
        let mut ports = sent.clone();
        ports.insert(lead.to_string(), Arc::new(vec![f.clone()]));
        for (id, index) in &others {
            let Some(own) = index.pair(f) else {
                out.missing.push((
                    label_of(Some(f)),
                    format!("no hay en «{id}» un archivo, y solo uno, con su nombre"),
                ));
                continue 'leads;
            };
            ports.insert((*id).to_string(), Arc::new(vec![own.clone()]));
        }
        for (id, by) in &shares {
            if let Some(own) = by.get(f) {
                ports.insert((*id).to_string(), Arc::new(own.clone()));
            }
        }
        out.units.push(Unit {
            ports,
            label: label_of(Some(f)),
            block: false,
        });
    }
    Ok(out)
}

/// La calculadora con un hilo por letra y varias hojas en cada uno se ejecuta una vez por hoja (A y
/// B son entonces la misma hoja); en los demás casos cada archivo es una banda.
fn calc_by_sheet(args: &BTreeMap<String, Vec<String>>, wires: &[Files]) -> bool {
    let Some(expr) = args.get("expr").and_then(|v| v.first()) else {
        return false;
    };
    // las bandas que nombra la expresión: letras mayúsculas sueltas
    let letters: HashSet<&str> = expr
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| w.len() == 1 && w.starts_with(|c: char| c.is_ascii_uppercase()))
        .collect();
    letters.len() == wires.len() && !wires.is_empty() && wires.iter().all(|w| w.len() > 1)
}

/// FNV-1a: una huella que no cambia entre versiones del programa.
#[derive(Clone, Copy)]
struct Hash(u64);

impl Hash {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn add(&mut self, text: &str) {
        for b in text.bytes().chain([0]) {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    fn hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// La huella de un archivo de entrada: cambia si se sustituye.
fn stamp_of(path: &Path) -> String {
    let m = std::fs::metadata(path);
    let time = m
        .as_ref()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("{}:{time}", m.map_or(0, |m| m.len()))
}

/// Qué flujo es: sus órdenes, opciones, hilos y entradas (ni los nombres, ni las estrellas, ni la
/// posición de los nodos).
fn signature(m: &Model) -> String {
    let mut h = Hash::new();
    for n in &m.nodes {
        h.add(&n.id);
        match &n.kind {
            NodeKind::Input { path } => h.add(path),
            NodeKind::Tool { tool, args } => h.add(&format!("{tool} {args:?}")),
        }
    }
    for e in &m.edges {
        h.add(&format!("{}>{}.{}", e.from, e.to, e.port));
    }
    h.hex()
}

/// Por qué un archivo de una entrada no se puede usar, mirando solo su principio: lo que se puede
/// saber sin leerlo entero.
fn broken(path: &Path) -> Option<String> {
    let len = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => return Some(e.to_string()),
    };
    if len == 0 {
        return Some("el archivo está vacío".into());
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let head = |n: usize| -> std::io::Result<Vec<u8>> {
        use std::io::Read;
        let mut b = vec![0; n];
        std::fs::File::open(path)?.read_exact(&mut b)?;
        Ok(b)
    };
    match ext.as_str() {
        "las" | "laz" => crate::las::Reader::open(path).err().map(|e| e.to_string()),
        "tif" | "tiff" => match head(4) {
            Ok(b) if b.starts_with(b"II") || b.starts_with(b"MM") => None,
            _ => Some("no es un GeoTIFF".into()),
        },
        "shp" => match head(28) {
            // la cabecera dice cuánto mide el archivo, en palabras de dos bytes
            Ok(b) => {
                let words = u32::from_be_bytes([b[24], b[25], b[26], b[27]]);
                if b[..4] != [0, 0, 0x27, 0x0a] {
                    Some("no es un shapefile".into())
                } else if u64::from(words) * 2 > len {
                    Some("el shapefile está cortado".into())
                } else if !path.with_extension("shx").exists() && !path.with_extension("SHX").exists() {
                    Some("falta su archivo .shx".into())
                } else {
                    None
                }
            }
            Err(e) => Some(e.to_string()),
        },
        _ => None,
    }
}

/// Un error que no depende del archivo: seguir solo serviría para repetirlo.
fn is_fatal(message: &str) -> bool {
    [
        "cancelado",
        "no se pudo lanzar",
        "os error 28",
        "os error 112",
        "No space left",
        "espacio en disco",
    ]
    .iter()
    .any(|m| message.contains(m))
}

/// Lo que se sabe de una unidad ya hecha (o fallida).
#[derive(Clone)]
struct Record {
    node: String,
    /// Los archivos que dejó, relativos a la carpeta de la ejecución, con su tamaño.
    files: Vec<(String, u64)>,
}

/// El diario: una línea por cada cosa terminada. Una línea cortada por un apagón se ignora.
struct Journal {
    file: std::fs::File,
    synced: Instant,
    done: HashMap<String, Record>,
    /// Los nodos intermedios cuyos productos ya se borraron porque nadie los necesitaba.
    purged: HashSet<String>,
    /// Lo mismo, unidad a unidad: los intermedios de las teselas que ya han terminado su cadena.
    purged_units: HashSet<String>,
}

impl Journal {
    fn open(work: &Path, sig: &str) -> Result<Self> {
        let path = work.join(JOURNAL);
        let (mut done, mut purged) = (HashMap::new(), HashSet::new());
        let mut purged_units = HashSet::new();
        let old = std::fs::read(&path).unwrap_or_default();
        for line in String::from_utf8_lossy(&old).lines() {
            let Ok(j) = Json::parse(line) else { continue };
            let text = |k: &str| j.get(k).and_then(Json::as_str).map(str::to_string);
            if let Some(n) = text("purged") {
                purged.insert(n);
            } else if let Some(u) = text("pu") {
                purged_units.insert(u);
            } else if let (Some(u), Some(node)) = (text("u"), text("n")) {
                let files = j
                    .get("f")
                    .and_then(Json::as_array)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|f| {
                        let f = f.as_array()?;
                        Some((f.first()?.as_str()?.to_string(), f.get(1)?.as_f64()? as u64))
                    })
                    .collect();
                // rehecha después de borrarla: vuelve a depender de sus archivos
                purged_units.remove(&u);
                done.insert(u, Record { node, files });
            }
        }
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        if old.is_empty() {
            writeln!(file, "{{\"flujo\":{}}}", quote(sig))?;
        } else if !old.ends_with(b"\n") {
            writeln!(file)?;
        }
        Ok(Self {
            file,
            synced: Instant::now(),
            done,
            purged,
            purged_units,
        })
    }

    fn line(&mut self, text: &str) -> Result<()> {
        writeln!(self.file, "{text}")?;
        // al disco cada poco: tras un apagón solo se repite lo de los últimos segundos
        if self.synced.elapsed().as_secs() >= 2 {
            self.file.sync_data()?;
            self.synced = Instant::now();
        }
        Ok(())
    }

    fn record(&mut self, unit: &str, record: Record) -> Result<()> {
        let files: Vec<String> = record
            .files
            .iter()
            .map(|(f, s)| format!("[{},{s}]", quote(f)))
            .collect();
        self.line(&format!(
            "{{\"u\":{},\"n\":{},\"f\":[{}]}}",
            quote(unit),
            quote(&record.node),
            files.join(",")
        ))?;
        self.purged_units.remove(unit);
        self.done.insert(unit.to_string(), record);
        Ok(())
    }

    /// Lo que dejó una unidad, si sigue valiendo: sus archivos están y miden lo que medían (o se
    /// borraron a propósito).
    fn reuse(&self, work: &Path, unit: &str) -> Option<&Record> {
        let r = self.done.get(unit)?;
        let whole = self.purged.contains(&r.node)
            || self.purged_units.contains(unit)
            || r.files
                .iter()
                .all(|(f, size)| std::fs::metadata(work.join(f)).is_ok_and(|m| m.len() == *size));
        whole.then_some(r)
    }
}

/// La última ejecución sin terminar de este mismo flujo en `results`.
pub fn resumable(model: &Model, results: &Path) -> Option<PathBuf> {
    let head = format!("{{\"flujo\":{}}}", quote(&signature(model)));
    let mut found: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in std::fs::read_dir(results).ok()?.flatten() {
        let journal = e.path().join(JOURNAL);
        let Ok(text) = std::fs::read(&journal) else { continue };
        let text = String::from_utf8_lossy(&text);
        if text.lines().next() != Some(head.as_str())
            || text.lines().last().is_some_and(|l| l == "{\"fin\":true}")
        {
            continue;
        }
        let Ok(time) = std::fs::metadata(&journal).and_then(|m| m.modified()) else {
            continue;
        };
        if found.as_ref().is_none_or(|f| time > f.0) {
            found = Some((time, e.path()));
        }
    }
    found.map(|f| f.1)
}

/// Un producto de un nodo y su huella, que arrastra la de lo que se usó para hacerlo.
#[derive(Clone)]
struct Product {
    path: PathBuf,
    stamp: String,
}

/// Los archivos de una carpeta y de las que cuelgan de ella (las partes de `split`).
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for p in std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()) {
            if is_hidden(&p) {
            } else if p.is_dir() {
                pending.push(p);
            } else {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

/// Cambia de nombre, insistiendo si un antivirus o un indexador retienen el archivo.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    for wait in [20, 100, 500, 2000] {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                std::thread::sleep(std::time::Duration::from_millis(wait));
            }
            other => return other,
        }
    }
    std::fs::rename(from, to)
}

/// Pasa a `dir` lo que una unidad dejó en `staging`, sin pisar nada: si un nombre ya está, todos
/// los de la unidad llevan el mismo sufijo, para que un shapefile no se separe de sus acompañantes.
fn publish(staging: &Path, dir: &Path) -> Result<Vec<PathBuf>> {
    let made = walk(staging);
    let rels: Vec<&Path> = made.iter().filter_map(|f| f.strip_prefix(staging).ok()).collect();
    let target = |rel: &Path, k: usize| -> PathBuf {
        if k < 2 {
            return dir.join(rel);
        }
        let stem = crate::paths::stem(rel);
        let name = match rel.extension() {
            Some(e) => format!("{stem}_{k}.{}", e.to_string_lossy()),
            None => format!("{stem}_{k}"),
        };
        dir.join(rel.with_file_name(name))
    };
    let mut k = 1;
    while rels.iter().any(|r| target(r, k).exists()) {
        k += 1;
    }
    let mut out = Vec::new();
    for (from, rel) in made.iter().zip(&rels) {
        let to = target(rel, k);
        crate::paths::ensure_parent(&to)?;
        rename(from, &to)?;
        out.push(to);
    }
    Ok(out)
}

/// Lo que va saliendo de un nodo.
#[derive(Default)]
struct Made {
    /// Lo que puede pasar a los nodos siguientes.
    products: Vec<Product>,
    /// Todos sus archivos, acompañantes incluidos.
    valid: HashSet<PathBuf>,
}

/// Un nodo de herramienta preparado para ejecutarse.
struct Member<'a> {
    node: &'a Node,
    tool: &'a str,
    args: &'a BTreeMap<String, Vec<String>>,
    spec: &'a ToolSpec,
    name: String,
    dir: PathBuf,
    /// Lo que reciben sus opciones de nodos ya terminados.
    fixed: BTreeMap<String, Files>,
    /// Los de cada hilo de la calculadora, que da nombre a una banda.
    wires: Vec<Files>,
    /// Las opciones que reciben archivos tesela a tesela, y de dónde: `None` es la entrega que
    /// reparte la tanda; si no, la posición de otro nodo de la tanda.
    live: Vec<(String, Vec<Option<usize>>)>,
    /// No se puede hacer porque falló un paso anterior.
    dead: bool,
}

/// El reparto de una tanda: qué nodos la forman y qué entrega recorren.
struct Shape {
    /// Posiciones en el modelo, en orden de ejecución.
    nodes: Vec<usize>,
    /// El nodo cuyos productos se reparten de uno en uno, y de qué tipo son; `None` cuando la tanda
    /// es un solo nodo que reparte su propio trabajo.
    source: Option<(usize, Kind)>,
}

/// Cómo puede entrar `tool` en una tanda que reparte los productos de `source`: sus opciones que
/// reciben tesela a tesela, o `None` si necesita verlas todas juntas.
fn live_ports(
    model: &Model,
    n: &Node,
    spec: &ToolSpec,
    source: usize,
    kind: Kind,
    members: &[usize],
    products: &HashMap<String, Vec<Product>>,
) -> Option<Vec<(String, Vec<Option<usize>>)>> {
    let NodeKind::Tool { tool, args } = &n.kind else { return None };
    let tool = tool.as_str();
    if tool == "calc" || SERIAL.contains(&tool) || needs_block(tool, args) {
        return None;
    }
    let source_id = model.nodes[source].id.as_str();
    let per_file = PER_FILE.contains(&tool);
    let mut live = Vec::new();
    for a in spec.args.iter().filter(|a| wirable(tool, a)) {
        let want = arg_kind(tool, &a.id);
        // si la opción se puede repartir archivo a archivo
        let splits = per_file && (a.id == "inputs" || a.id == "dtm") || !a.multiple;
        let edges: Vec<_> = model.edges.iter().filter(|e| e.to == n.id && e.port == a.id).collect();
        let from: Vec<Option<Option<usize>>> = edges
            .iter()
            .map(|e| {
                if e.from == source_id {
                    Some(None)
                } else {
                    members.iter().position(|&m| model.nodes[m].id == e.from).map(Some)
                }
            })
            .collect();
        if from.iter().all(Option::is_none) {
            let files: usize = edges
                .iter()
                .flat_map(|e| products.get(&e.from).into_iter().flatten())
                .filter(|p| want.accepts(&p.path))
                .count();
            if files > 1 && !splits {
                return None;
            }
            // las nubes mandan: si llegan por un hilo, han de ser las que se reparten
            if per_file && a.id == "inputs" {
                return None;
            }
            continue;
        }
        let from: Vec<Option<usize>> = from.into_iter().collect::<Option<_>>()?;
        if !splits || from.contains(&None) && want != kind {
            return None;
        }
        live.push((a.id.clone(), from));
    }
    // sin las nubes por un hilo, cada tesela repetiría las escritas a mano
    if per_file && spec.arg("inputs").is_some() && !live.iter().any(|(p, _)| p == "inputs") {
        return None;
    }
    (!live.is_empty()).then_some(live)
}

/// La tanda que empieza en el nodo `first`: todos los que se pueden encadenar con él tesela a
/// tesela, o él solo.
fn shape(
    model: &Model,
    run: &Run,
    order: &[usize],
    first: usize,
    done: &[bool],
    products: &HashMap<String, Vec<Product>>,
) -> Shape {
    let alone = Shape {
        nodes: vec![first],
        source: None,
    };
    let spec_of = |i: usize| match &model.nodes[i].kind {
        NodeKind::Tool { tool, .. } => run.schema.tool(tool),
        NodeKind::Input { .. } => None,
    };
    let (Some(spec), NodeKind::Tool { tool, .. }) = (spec_of(first), &model.nodes[first].kind) else {
        return alone;
    };
    for a in spec.args.iter().filter(|a| wirable(tool, a)) {
        let want = arg_kind(tool, &a.id);
        for e in model.edges.iter().filter(|e| e.to == model.nodes[first].id && e.port == a.id) {
            let Some(source) = model.nodes.iter().position(|n| n.id == e.from) else { continue };
            let count = products
                .get(&e.from)
                .map_or(0, |p| p.iter().filter(|p| want.accepts(&p.path)).take(2).count());
            if count < 2 || live_ports(model, &model.nodes[first], spec, source, want, &[], products).is_none() {
                continue;
            }
            let mut members = vec![first];
            for &j in order {
                let n = &model.nodes[j];
                if done[j] || members.contains(&j) {
                    continue;
                }
                let Some(spec) = spec_of(j) else { continue };
                let ready = model.edges.iter().filter(|e| e.to == n.id).all(|e| {
                    model
                        .nodes
                        .iter()
                        .position(|n| n.id == e.from)
                        .is_some_and(|i| done[i] || members.contains(&i))
                });
                if ready && live_ports(model, n, spec, source, want, &members, products).is_some() {
                    members.push(j);
                }
            }
            if members.len() > 1 {
                return Shape {
                    nodes: members,
                    source: Some((source, want)),
                };
            }
        }
    }
    alone
}

/// El recorrido de una tesela (o de una unidad suelta) por los nodos de una tanda.
struct Track {
    /// Su número, para el avance.
    number: usize,
    /// La unidad de una tanda de un solo nodo, o la tesela que recorre la cadena.
    start: Start,
    /// Con qué se la nombra, y con qué nombre no puede haber dos a la vez.
    label: String,
    step: usize,
    /// Las unidades del paso actual que faltan.
    queue: VecDeque<Unit>,
    planned: bool,
    /// Lo que ha salido de cada nodo de la tanda en este recorrido.
    steps: Vec<Step>,
    /// Qué unidad hizo cada archivo.
    from: HashMap<PathBuf, String>,
    /// Las que hay que rehacer aunque consten, porque sus productos se borraron y hacen falta.
    forced: HashSet<String>,
    /// La que ya se repitió sola por falta de memoria.
    retried: Option<String>,
}

#[derive(Clone)]
enum Start {
    Unit(Unit),
    Sheet(PathBuf),
}

/// Lo de un nodo en un recorrido.
#[derive(Default)]
struct Step {
    /// Cada unidad hecha: su huella y sus archivos.
    done: Vec<(Hash, Vec<(String, u64)>)>,
    reused: usize,
    /// Falló aquí o antes.
    bad: bool,
    /// Sus intermedios ya se borraron.
    purged: bool,
}

impl Track {
    fn new(number: usize, start: Start, label: String, members: usize) -> Self {
        Self {
            number,
            start,
            label,
            step: 0,
            queue: VecDeque::new(),
            planned: false,
            steps: (0..members).map(|_| Step::default()).collect(),
            from: HashMap::new(),
            forced: HashSet::new(),
            retried: None,
        }
    }

    /// Vuelve al principio para rehacer lo que hace falta; lo que consta no se repite.
    fn restart(&mut self) {
        self.step = 0;
        self.queue.clear();
        self.planned = false;
        self.from.clear();
        self.steps = (0..self.steps.len()).map(|_| Step::default()).collect();
    }
}

/// Qué hay que hacer en un paso de un recorrido.
enum Planned {
    Units(Plan),
    /// Un paso anterior falló y no ha dejado nada que tratar.
    Blocked,
    /// No hay nada de su tipo en esta tesela.
    Nothing,
}

/// Una orden en marcha.
struct Running {
    track: Track,
    unit: Unit,
    key: Hash,
    staging: PathBuf,
    /// Las hojas de un bloque, con su huella.
    sheets: Vec<(PathBuf, Hash)>,
    skip_file: PathBuf,
    /// Lo que se ha anunciado desde la última hoja terminada.
    announced: Vec<PathBuf>,
    /// Los archivos del bloque ya apuntados.
    written: HashSet<PathBuf>,
    /// Las hojas que la orden dice que han fallado, con su motivo: más útil que su resumen final.
    causes: Vec<(String, String)>,
    /// Lo que ha dicho, guardado para no mezclar las líneas de varias órdenes a la vez.
    lines: Vec<String>,
}

/// Lo que cuenta una orden en marcha.
enum Message {
    Line(usize, String),
    Announced(usize, PathBuf),
    Done(usize, Result<()>),
}

/// Lo que hay que lanzar.
struct Job {
    argv: Vec<String>,
    env: Vec<(&'static str, PathBuf)>,
}

/// La memoria del equipo; 8 GB cuando no se puede saber.
fn memory() -> u64 {
    crate::copc_cache::total_memory()
        .or_else(|| {
            if !cfg!(windows) {
                return None;
            }
            let out = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
                ])
                .output()
                .ok()?;
            String::from_utf8_lossy(&out.stdout).trim().parse().ok()
        })
        .unwrap_or(8 << 30)
}

/// Cuántas teselas tratar a la vez cuando no se dice: cada orden ya usa varios núcleos, así que con
/// archivos grandes rinden pocas (medido con hojas de 100 MB: dos van mejor que una o que seis), y
/// la memoria que pide cada una (hasta unas 25 veces el tamaño de la nube comprimida) pone el tope.
fn automatic_jobs(bytes: u64, cores: usize, memory: u64) -> usize {
    let by_size = if bytes < 8 << 20 {
        cores
    } else if bytes < 32 << 20 {
        (cores / 2).max(2)
    } else {
        (cores / 5).max(2)
    };
    let each = bytes.saturating_mul(25).saturating_add(150 << 20);
    let by_memory = usize::try_from(memory / 2 / each).unwrap_or(usize::MAX);
    by_size.min(by_memory).max(1)
}

/// Un fallo que parece de falta de memoria (o una caída sin explicación): solo, quizá salga.
fn looks_starved(message: &str) -> bool {
    message.starts_with("la orden terminó con") || message.contains("memory allocation") || message.contains("memoria")
}

/// El estado de una ejecución mientras avanza.
struct Exec<'a> {
    r: &'a Run<'a>,
    work: PathBuf,
    journal: Journal,
    log_file: std::io::BufWriter<std::fs::File>,
    failed: usize,
    first_error: String,
    in_a_row: usize,
    /// La huella de cada archivo que puede alimentar una unidad.
    stamps: HashMap<PathBuf, String>,
}

impl Exec<'_> {
    fn note(&mut self, line: &str) {
        let _ = writeln!(self.log_file, "{line}");
    }

    fn fail(&mut self, node: &str, unit: &str, message: &str) -> Result<()> {
        let path = self.work.join(ERRORS);
        let new = !path.exists();
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        if new {
            // la marca de orden de bytes hace que Excel lea las tildes
            writeln!(f, "\u{feff}nodo;archivo;error")?;
        }
        let cell = |s: &str| format!("\"{}\"", s.replace('"', "\"\"").replace('\n', " "));
        writeln!(f, "{};{};{}", cell(node), cell(unit), cell(message))?;
        self.failed += 1;
        if self.first_error.is_empty() {
            self.first_error = message.to_string();
        }
        Ok(())
    }

    fn relative(&self, p: &Path) -> String {
        p.strip_prefix(&self.work)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn sized(&self, files: &[PathBuf]) -> Vec<(String, u64)> {
        files
            .iter()
            .map(|f| (self.relative(f), std::fs::metadata(f).map_or(0, |m| m.len())))
            .collect()
    }

    fn record(&mut self, unit: &str, node: &str, files: Vec<(String, u64)>) -> Result<()> {
        self.journal.record(
            unit,
            Record {
                node: node.to_string(),
                files,
            },
        )
    }

    /// Los productos que hay entre unos archivos apuntados, con su huella anotada.
    fn products(&mut self, key: Hash, files: &[(String, u64)]) -> Vec<Product> {
        let mut out = Vec::new();
        for (f, _) in files {
            let path = self.work.join(f);
            if Kind::Any.accepts(&path) {
                self.stamps.insert(path.clone(), key.hex());
                out.push(Product {
                    path,
                    stamp: key.hex(),
                });
            }
        }
        out
    }

    fn take(&mut self, made: &mut Made, key: Hash, files: &[(String, u64)]) {
        made.valid.extend(files.iter().map(|f| self.work.join(&f.0)));
        made.products.extend(self.products(key, files));
    }

    /// La huella de una unidad: la orden, sus opciones y cada archivo que recibe.
    fn unit_key(&self, m: &Member, unit: &Unit) -> Hash {
        let mut h = Hash::new();
        h.add(&m.node.id);
        h.add(&format!("{} {:?}", m.tool, m.args));
        for (port, files) in &unit.ports {
            h.add(port);
            for f in files.iter() {
                h.add(&self.relative(f));
                h.add(self.stamps.get(f).map_or("", String::as_str));
            }
        }
        h
    }

    /// La huella de una hoja dentro de un bloque.
    fn sheet_key(&self, mut block: Hash, sheet: &Path) -> Hash {
        block.add(&self.relative(sheet));
        block
    }

    /// Los argumentos con los que se lanza una unidad.
    fn argv(&self, m: &Member, unit: &Unit, dir: &Path) -> Vec<String> {
        // una herramienta a la que hay que decir dónde escribir recibe un nombre en la carpeta
        let output = m.spec.output_arg().filter(|a| a.required).map(|_| {
            // la entrada que da nombre: la primera, en el orden de la orden, con un solo archivo
            let source = m
                .spec
                .args
                .iter()
                .filter_map(|a| unit.ports.get(&a.id))
                .find(|f| f.len() == 1)
                .and_then(|f| f.first());
            // una reproyección conserva el tipo de archivo que recibió
            let ext = source
                .and_then(|f| f.extension())
                .filter(|_| m.tool == "reproject")
                .map_or(entry(m.tool).map_or("tif", |e| e.ext).to_string(), |e| {
                    e.to_string_lossy().into_owned()
                });
            let name = match source {
                Some(f) => format!("{}_{}", crate::paths::stem(f), folder_name(m.tool)),
                None => folder_name(m.tool),
            };
            dir.join(format!("{name}.{ext}"))
        });
        let ports: HashMap<String, Vec<PathBuf>> =
            unit.ports.iter().map(|(k, v)| (k.clone(), v.to_vec())).collect();
        let mut argv = vec![
            "--results-dir".to_string(),
            dir.display().to_string(),
            m.tool.to_string(),
        ];
        argv.extend(command_line(m.spec, m.node, m.args, &ports, output.as_deref()));
        argv
    }
}

/// Los archivos de cada opción fija de un nodo, ordenados por nombre para emparejarlos.
type Indexes<'a> = BTreeMap<&'a str, NameIndex<'a>>;

/// Los nodos de una tanda y cómo se emparejan con cada tesela lo que reciben de antes.
struct Stage<'a> {
    members: Vec<Member<'a>>,
    kind: Option<Kind>,
}

impl Stage<'_> {
    /// Qué tiene que hacer el paso actual de un recorrido.
    fn plan_step(&self, track: &Track, indexes: &[Indexes], work: &Path) -> Result<Planned> {
        let m = &self.members[track.step];
        let sheet = match &track.start {
            Start::Unit(u) => {
                return Ok(Planned::Units(Plan {
                    units: vec![u.clone()],
                    missing: Vec::new(),
                }));
            }
            Start::Sheet(f) => f,
        };
        if m.dead {
            return Ok(Planned::Blocked);
        }
        let mut sent = BTreeMap::new();
        for (port, files) in &m.fixed {
            let own = if files.len() > 1 {
                match indexes[track.step].get(port.as_str()).and_then(|i| i.pair(sheet)) {
                    Some(f) => Arc::new(vec![f.clone()]),
                    None => {
                        return Ok(Planned::Units(Plan {
                            units: Vec::new(),
                            missing: vec![(
                                label_of(Some(sheet)),
                                format!("no hay en «{port}» un archivo, y solo uno, con su nombre"),
                            )],
                        }));
                    }
                }
            } else {
                files.clone()
            };
            sent.insert(port.clone(), own);
        }
        for (port, from) in &m.live {
            let want = arg_kind(m.tool, port);
            let mut files = Vec::new();
            let mut upstream = false;
            for source in from {
                match source {
                    None => files.extend(self.kind.filter(|_| want.accepts(sheet)).map(|_| sheet.clone())),
                    Some(j) => {
                        upstream |= track.steps[*j].bad;
                        files.extend(
                            track.steps[*j]
                                .done
                                .iter()
                                .flat_map(|(_, f)| f)
                                .map(|(f, _)| work.join(f))
                                .filter(|f| Kind::Any.accepts(f) && want.accepts(f)),
                        );
                    }
                }
            }
            if files.is_empty() {
                return Ok(if upstream { Planned::Blocked } else { Planned::Nothing });
            }
            sent.insert(port.clone(), Arc::new(files));
        }
        Ok(Planned::Units(plan(m.spec, m.args, sent, &[])?))
    }
}

/// Lo que se va sabiendo de una tanda mientras se ejecuta.
struct Tally {
    made: Vec<Made>,
    failures: Vec<usize>,
    reused: Vec<usize>,
    /// Si cada nodo ha tenido algo que hacer.
    received: Vec<bool>,
    told: Instant,
}

impl Tally {
    /// Suma lo de un recorrido terminado.
    fn commit(&mut self, x: &mut Exec, track: &Track) {
        for (s, step) in track.steps.iter().enumerate() {
            self.reused[s] += step.reused;
            for (key, files) in &step.done {
                x.take(&mut self.made[s], *key, files);
            }
        }
    }
}

/// Una unidad lista para lanzarse.
struct Next {
    unit: Unit,
    key: Hash,
    /// Las hojas del bloque que faltan y las que ya están.
    sheets: Vec<(PathBuf, Hash)>,
    skip: Vec<String>,
}

struct Plant<'a> {
    stage: Stage<'a>,
    indexes: Vec<Indexes<'a>>,
    /// Para cada nodo cuyos productos son intermedios que solo usa la tanda, quién los usa.
    uses: Vec<Option<Vec<usize>>>,
    total: usize,
}

impl Plant<'_> {
    /// Avanza un recorrido hasta la siguiente unidad que hay que ejecutar; `None` cuando termina.
    fn advance(
        &self,
        x: &mut Exec,
        track: &mut Track,
        tally: &mut Tally,
        log: &mut dyn FnMut(&str),
    ) -> Result<Option<Next>> {
        loop {
            let s = track.step;
            let Some(m) = self.stage.members.get(s) else { return Ok(None) };
            if !track.planned {
                match self.stage.plan_step(track, &self.indexes, &x.work)? {
                    Planned::Units(p) => {
                        for (label, why) in &p.missing {
                            log(&format!("error: {label}: {why}"));
                            x.fail(&m.name, label, why)?;
                            tally.failures[s] += 1;
                            track.steps[s].bad = true;
                        }
                        track.queue = p.units.into();
                    }
                    Planned::Blocked => {
                        if !m.dead {
                            x.fail(&m.name, &track.label, "no se hace: falló un paso anterior")?;
                            tally.failures[s] += 1;
                        }
                        track.steps[s].bad = true;
                    }
                    Planned::Nothing => {}
                }
                track.planned = true;
            }
            let Some(unit) = track.queue.pop_front() else {
                self.purge(x, track)?;
                track.step += 1;
                track.planned = false;
                continue;
            };
            tally.received[s] = true;
            let key = x.unit_key(m, &unit);
            // en un bloque, cada hoja tiene su apunte y las hechas solo sirven de contexto
            let mut sheets = Vec::new();
            let mut skip = Vec::new();
            if unit.block {
                let joined = m.args.get("mosaic").and_then(|v| v.first()).is_some_and(|v| v == "true");
                for f in unit.ports.get("inputs").into_iter().flat_map(|f| f.iter()) {
                    let h = x.sheet_key(key, f);
                    match x.journal.reuse(&x.work, &h.hex()).filter(|_| !joined) {
                        Some(rec) => {
                            let files = rec.files.clone();
                            track.steps[s].reused += 1;
                            track.steps[s].done.push((h, files));
                            skip.push(f.display().to_string());
                        }
                        None => sheets.push((f.clone(), h)),
                    }
                }
            }
            let hex = key.hex();
            if !track.forced.contains(&hex)
                && let Some(rec) = x.journal.reuse(&x.work, &hex)
            {
                let files = rec.files.clone();
                // las huellas de estos productos hacen falta para las unidades que los usan
                for p in x.products(key, &files) {
                    track.from.insert(p.path, hex.clone());
                }
                track.steps[s].reused += 1;
                track.steps[s].done.push((key, files));
                continue;
            }
            if unit.block && sheets.is_empty() && !skip.is_empty() {
                continue;
            }
            // un intermedio que se borró y vuelve a hacer falta: se rehace quien lo hizo
            let gone = unit
                .ports
                .values()
                .flat_map(|f| f.iter())
                .filter(|f| track.from.contains_key(*f) && !f.exists())
                .find_map(|f| track.from.get(f))
                .cloned();
            if let Some(maker) = gone
                && track.forced.insert(maker)
            {
                track.restart();
                continue;
            }
            let chatty = self.total <= CHATTY;
            if self.total > 1 && (chatty || tally.told.elapsed().as_secs() >= 2 || track.number == self.total) {
                log(&format!("[{}/{}] {}", track.number, self.total, m.name));
                tally.told = Instant::now();
            }
            return Ok(Some(Next {
                unit,
                key,
                sheets,
                skip,
            }));
        }
    }

    /// Borra de un recorrido los intermedios que ya han usado todos los que los necesitaban.
    fn purge(&self, x: &mut Exec, track: &mut Track) -> Result<()> {
        for p in 0..=track.step {
            let Some(Some(users)) = self.uses.get(p) else { continue };
            let ready = !track.steps[p].purged
                && !track.steps[p].bad
                && users.iter().all(|&u| u <= track.step && !track.steps[u].bad);
            if !ready {
                continue;
            }
            for (key, files) in &track.steps[p].done {
                x.journal.line(&format!("{{\"pu\":{}}}", quote(&key.hex())))?;
                x.journal.purged_units.insert(key.hex());
                for (f, _) in files {
                    let _ = std::fs::remove_file(x.work.join(f));
                }
            }
            track.steps[p].purged = true;
        }
        Ok(())
    }

    /// Al reanudar, en las carpetas de la tanda solo se queda lo que sigue valiendo: lo demás (de
    /// una unidad que no terminó, de un resultado dañado o de una entrada que ha cambiado) se borra
    /// antes de rehacerlo, para que lo nuevo no acabe con otro nombre al lado de lo viejo.
    fn sweep(&self, x: &mut Exec, starts: &[(Start, String)]) -> Result<()> {
        let mut keep: HashSet<PathBuf> = HashSet::new();
        for (start, label) in starts {
            let mut track = Track::new(0, start.clone(), label.clone(), self.stage.members.len());
            for (s, m) in self.stage.members.iter().enumerate() {
                track.step = s;
                let Planned::Units(p) = self.stage.plan_step(&track, &self.indexes, &x.work)? else {
                    continue;
                };
                for unit in &p.units {
                    let key = x.unit_key(m, unit);
                    if unit.block {
                        for f in unit.ports.get("inputs").into_iter().flat_map(|f| f.iter()) {
                            if let Some(rec) = x.journal.reuse(&x.work, &x.sheet_key(key, f).hex()) {
                                keep.extend(rec.files.iter().map(|f| x.work.join(&f.0)));
                            }
                        }
                    }
                    let hex = key.hex();
                    if let Some(rec) = x.journal.reuse(&x.work, &hex) {
                        keep.extend(rec.files.iter().map(|f| x.work.join(&f.0)));
                    }
                    if let Some(files) = x.journal.done.get(&hex).map(|r| r.files.clone()) {
                        x.products(key, &files);
                        track.steps[s].done.push((key, files));
                    }
                }
            }
        }
        for m in &self.stage.members {
            for f in walk(&m.dir) {
                if !keep.contains(&f) {
                    let _ = std::fs::remove_file(f);
                }
            }
        }
        Ok(())
    }
}

/// Lo que sale de un nodo de una tanda.
struct Outcome {
    node: usize,
    products: Vec<Product>,
    failed: bool,
}

#[allow(clippy::too_many_arguments)]
fn run_stage(
    x: &mut Exec,
    model: &Model,
    shape: &Shape,
    dirs: &HashMap<&str, PathBuf>,
    products: &HashMap<String, Vec<Product>>,
    flawed: &HashSet<&str>,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<Outcome>> {
    let r = x.r;
    let mut tally = Tally {
        made: Vec::new(),
        failures: Vec::new(),
        reused: Vec::new(),
        received: Vec::new(),
        told: Instant::now(),
    };
    let mut members = Vec::new();
    for (pos, &i) in shape.nodes.iter().enumerate() {
        let n = &model.nodes[i];
        let NodeKind::Tool { tool, args } = &n.kind else {
            return Err(Error::Param("una tanda solo lleva herramientas".into()));
        };
        let name = model.name(n);
        let spec = r
            .schema
            .tool(tool)
            .ok_or_else(|| Error::Param(format!("«{tool}» no es una herramienta")))?;
        let live = match shape.source {
            Some((source, kind)) => live_ports(model, n, spec, source, kind, &shape.nodes[..pos], products)
                .ok_or_else(|| Error::Param(format!("«{name}» no se puede encadenar")))?,
            None => Vec::new(),
        };
        let mut member = Member {
            node: n,
            tool,
            args,
            spec,
            name,
            dir: dirs.get(n.id.as_str()).cloned().unwrap_or_default(),
            fixed: BTreeMap::new(),
            wires: Vec::new(),
            live,
            dead: false,
        };
        let mut failures = 0;
        // los archivos enviados a cada opción por nodos ya terminados, tomados por tipo
        for a in spec.args.iter().filter(|a| wirable(tool, a)) {
            if member.live.iter().any(|(p, _)| *p == a.id) {
                continue;
            }
            let want = arg_kind(tool, &a.id);
            let mut files = Vec::new();
            let (mut wired, mut upstream) = (false, false);
            for e in model.edges.iter().filter(|e| e.to == n.id && e.port == a.id) {
                wired = true;
                upstream |= flawed.contains(e.from.as_str());
                let mut own = Vec::new();
                for p in products.get(&e.from).into_iter().flatten() {
                    if want.accepts(&p.path) {
                        x.stamps.insert(p.path.clone(), p.stamp.clone());
                        own.push(p.path.clone());
                    }
                }
                files.extend(own.iter().cloned());
                if tool == "calc" {
                    member.wires.push(Arc::new(own));
                }
            }
            if wired && files.is_empty() {
                let why = format!("no ha recibido archivos de tipo {}", want.name());
                // si un paso anterior falló, este no se puede hacer, pero los demás sí
                if !upstream {
                    return Err(Error::Param(format!("«{}» {why}", member.name)));
                }
                log(&format!("error: «{}» {why}", member.name));
                x.fail(&member.name, "", &format!("{why}: falló un paso anterior"))?;
                failures += 1;
                member.dead = true;
            }
            if !files.is_empty() {
                member.fixed.insert(a.id.clone(), Arc::new(files));
            }
        }
        std::fs::create_dir_all(&member.dir)?;
        let _ = std::fs::remove_dir_all(member.dir.join(STAGING));
        tally.made.push(Made::default());
        tally.failures.push(failures);
        tally.reused.push(0);
        tally.received.push(false);
        members.push(member);
    }
    // qué recorre la tanda: las teselas de su fuente o las unidades de su único nodo
    let mut starts: Vec<(Start, String)> = Vec::new();
    match shape.source {
        Some((source, kind)) => {
            let id = model.nodes[source].id.as_str();
            for p in products.get(id).into_iter().flatten().filter(|p| kind.accepts(&p.path)) {
                x.stamps.insert(p.path.clone(), p.stamp.clone());
                starts.push((Start::Sheet(p.path.clone()), label_of(Some(&p.path))));
            }
        }
        None => {
            if let Some(m) = members.first().filter(|m| !m.dead) {
                let plan = plan(m.spec, m.args, m.fixed.clone(), &m.wires)?;
                for (label, why) in &plan.missing {
                    log(&format!("error: {label}: {why}"));
                    x.fail(&m.name, label, why)?;
                    tally.failures[0] += 1;
                }
                for u in plan.units {
                    let label = u.label.clone();
                    starts.push((Start::Unit(u), label));
                }
            }
        }
    }
    // los índices apuntan a las listas de cada nodo, que ya no cambian
    let fixed: Vec<BTreeMap<String, Files>> = members.iter().map(|m| m.fixed.clone()).collect();
    let indexes: Vec<Indexes> = fixed
        .iter()
        .map(|ports| {
            ports
                .iter()
                .filter(|(_, f)| shape.source.is_some() && f.len() > 1)
                .map(|(p, f)| (p.as_str(), NameIndex::new(f)))
                .collect()
        })
        .collect();
    let uses = members
        .iter()
        .map(|m| {
            if shape.source.is_none() || model.kept(m.node) {
                return None;
            }
            model
                .edges
                .iter()
                .filter(|e| e.from == m.node.id)
                .map(|e| members.iter().position(|o| o.node.id == e.to))
                .collect::<Option<Vec<usize>>>()
        })
        .collect();
    let total = starts.len();
    let serial = members.iter().any(|m| SERIAL.contains(&m.tool))
        || starts.iter().any(|s| matches!(&s.0, Start::Unit(u) if u.block));
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let jobs = if serial || total < 2 {
        1
    } else if r.jobs > 0 {
        r.jobs.min(total)
    } else {
        // lo que pesa la mayor de las primeras
        let bytes = starts
            .iter()
            .take(64)
            .filter_map(|s| std::fs::metadata(&s.1).ok())
            .map(|m| m.len())
            .max()
            .unwrap_or(0);
        automatic_jobs(bytes, cores, memory()).min(total)
    };
    if members.len() > 1 {
        let chain: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
        log(&format!(
            "cada tesela de principio a fin: {} ({total} teselas)",
            chain.join(", ")
        ));
    }
    if jobs > 1 {
        log(&format!("{jobs} a la vez"));
    }
    let plant = Plant {
        stage: Stage {
            members,
            kind: shape.source.map(|s| s.1),
        },
        indexes,
        uses,
        total,
    };
    if r.resume {
        plant.sweep(x, &starts)?;
    }
    let threads = (jobs > 1 && std::env::var_os(THREADS_ENV).is_none())
        .then(|| PathBuf::from(cores.div_ceil(jobs).to_string()));
    drive(x, &plant, &starts, jobs, threads, &mut tally, log)?;
    let mut out = Vec::new();
    for (s, (m, mut made)) in plant.stage.members.iter().zip(tally.made).enumerate() {
        let upstream = model
            .edges
            .iter()
            .any(|e| e.to == m.node.id && flawed.contains(e.from.as_str()))
            || m.live.iter().flat_map(|(_, f)| f).flatten().any(|&j| tally.failures[j] > 0);
        if shape.source.is_some() && !tally.received[s] && tally.failures[s] == 0 && !upstream {
            let want = m.live.first().map_or(Kind::Any, |(p, _)| arg_kind(m.tool, p));
            return Err(Error::Param(format!(
                "«{}» no ha recibido archivos de tipo {}",
                m.name,
                want.name()
            )));
        }
        if tally.reused[s] > 0 {
            log(&format!("«{}»: {} ya hechos, no se repiten", m.name, tally.reused[s]));
        }
        made.products.sort_by(|a, b| a.path.cmp(&b.path));
        out.push(Outcome {
            node: shape.nodes[s],
            products: made.products,
            failed: tally.failures[s] > 0,
        });
    }
    Ok(out)
}

/// Lanza las órdenes de una tanda, `jobs` a la vez, y apunta lo que van dejando.
fn drive(
    x: &mut Exec,
    plant: &Plant,
    starts: &[(Start, String)],
    jobs: usize,
    threads: Option<PathBuf>,
    tally: &mut Tally,
    log: &mut dyn FnMut(&str),
) -> Result<()> {
    let (exe, cancel) = (x.r.exe, x.r.cancel);
    // para las órdenes en marcha: por una cancelación o por un fallo que no tiene arreglo
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<Message>();
        let mut running: BTreeMap<usize, Running> = BTreeMap::new();
        // los recorridos a medias que esperan hueco: tienen preferencia sobre empezar otros
        let mut held: VecDeque<Track> = VecDeque::new();
        // dos teselas del mismo nombre no van a la vez: sus productos se llaman igual
        let mut active: HashSet<String> = HashSet::new();
        let stem = |label: &str| crate::paths::stem(Path::new(label)).to_lowercase();
        let (mut next, mut seq, mut limit) = (0, 0usize, jobs);
        let mut fatal: Option<Error> = None;
        loop {
            if cancel.load(Ordering::Relaxed) && fatal.is_none() {
                fatal = Some(Error::Param("cancelado".into()));
            }
            if fatal.is_some() {
                stop.store(true, Ordering::Relaxed);
            }
            // al reanudar, miles de teselas ya hechas pasan por aquí seguidas: también se cancelan
            while fatal.is_none() && running.len() < limit && !cancel.load(Ordering::Relaxed) {
                let mut track = if let Some(t) = held.pop_front() {
                    t
                } else if let Some((start, label)) = starts.get(next).filter(|s| !active.contains(&stem(&s.1))) {
                    next += 1;
                    active.insert(stem(label));
                    Track::new(next, start.clone(), label.clone(), plant.stage.members.len())
                } else {
                    break;
                };
                seq += 1;
                let begun = plant.advance(x, &mut track, tally, log).and_then(|n| match n {
                    Some(n) => begin(x, plant, track, n, seq, log).map(Some),
                    None => {
                        tally.commit(x, &track);
                        active.remove(&stem(&track.label));
                        Ok(None)
                    }
                });
                match begun {
                    Ok(Some((run, mut job))) => {
                        if limit > 1
                            && let Some(t) = &threads
                        {
                            job.env.push((THREADS_ENV, t.clone()));
                        }
                        running.insert(seq, run);
                        let (tx, stop, slot) = (tx.clone(), &stop, seq);
                        scope.spawn(move || {
                            let result = spawn_with(
                                exe,
                                &job.argv,
                                &job.env,
                                stop,
                                &mut |l| {
                                    let _ = tx.send(Message::Line(slot, l.to_string()));
                                },
                                &mut |p| {
                                    let _ = tx.send(Message::Announced(slot, p.to_path_buf()));
                                },
                            );
                            let _ = tx.send(Message::Done(slot, result));
                        });
                    }
                    Ok(None) => {}
                    Err(e) => fatal = Some(e),
                }
            }
            if running.is_empty() {
                if fatal.is_some() || next >= starts.len() && held.is_empty() {
                    break;
                }
                continue;
            }
            let Ok(message) = rx.recv_timeout(std::time::Duration::from_millis(100)) else {
                continue;
            };
            let handled = match message {
                Message::Line(slot, line) => match running.get_mut(&slot) {
                    Some(run) => heard(x, plant, run, &line, jobs == 1, log),
                    None => Ok(()),
                },
                Message::Announced(slot, path) => {
                    if let Some(run) = running.get_mut(&slot) {
                        run.announced.push(path);
                    }
                    Ok(())
                }
                Message::Done(slot, result) => match running.remove(&slot) {
                    Some(mut run) => {
                        // la que ha caído por falta de memoria se repite sin compañía
                        let again = limit > 1
                            && fatal.is_none()
                            && result.as_ref().is_err_and(|e| looks_starved(&e.to_string()))
                            && run.track.retried.as_ref() != Some(&run.key.hex());
                        if again {
                            limit = 1;
                            log(&format!(
                                "{}: se repite sola y se sigue de una en una (¿falta memoria?)",
                                run.track.label
                            ));
                            let _ = std::fs::remove_dir_all(&run.staging);
                            run.track.retried = Some(run.key.hex());
                            run.track.queue.push_front(run.unit);
                            held.push_back(run.track);
                            Ok(())
                        } else {
                            let stopping = fatal.is_some();
                            finish(x, plant, run, result, stopping, tally, log).map(|track| held.push_front(track))
                        }
                    }
                    None => Ok(()),
                },
            };
            if let Err(e) = handled
                && fatal.is_none()
            {
                fatal = Some(e);
            }
        }
        fatal.map_or(Ok(()), Err)
    })
}

/// Prepara la carpeta y los argumentos de una unidad.
fn begin(
    x: &mut Exec,
    plant: &Plant,
    track: Track,
    next: Next,
    seq: usize,
    log: &mut dyn FnMut(&str),
) -> Result<(Running, Job)> {
    let m = &plant.stage.members[track.step];
    let staging = m.dir.join(STAGING).join(seq.to_string());
    // un bloque escribe en la carpeta del nodo, para que cada hoja terminada quede
    let target = if next.unit.block { m.dir.clone() } else { staging.clone() };
    std::fs::create_dir_all(&target)?;
    let argv = x.argv(m, &next.unit, &target);
    let mut env = Vec::new();
    let skip_file = m.dir.join(".hechas");
    if !next.skip.is_empty() {
        std::fs::write(&skip_file, next.skip.join("\n"))?;
        env.push((crate::sheets::SKIP_ENV, skip_file.clone()));
    }
    let line = format!("» hipso {}", argv.join(" "));
    let line = if line.len() > 2000 {
        let files = next.unit.ports.values().map(|f| f.len()).sum::<usize>();
        format!("» hipso {} ({files} archivos)", m.tool)
    } else {
        line
    };
    x.note(&line);
    if plant.total <= CHATTY {
        log(&line);
    }
    let written = track.steps[track.step]
        .done
        .iter()
        .flat_map(|(_, f)| f)
        .map(|(f, _)| x.work.join(f))
        .collect();
    Ok((
        Running {
            track,
            unit: next.unit,
            key: next.key,
            staging,
            sheets: next.sheets,
            skip_file,
            announced: Vec::new(),
            written,
            causes: Vec::new(),
            lines: Vec::new(),
        },
        Job { argv, env },
    ))
}

/// Una línea de una orden en marcha.
fn heard(
    x: &mut Exec,
    plant: &Plant,
    run: &mut Running,
    line: &str,
    live: bool,
    log: &mut dyn FnMut(&str),
) -> Result<()> {
    let s = run.track.step;
    let m = &plant.stage.members[s];
    // lo que la orden dice de su carpeta provisional se enseña con la definitiva
    let l = line.replace(&run.staging.display().to_string(), &m.dir.display().to_string());
    if let Some((sheet, why)) = l.split_once(": ERROR: ")
        && sheet.starts_with('[')
    {
        // `[k/n] ruta`; la orden lo repite en su resumen
        let sheet = sheet.split_once("] ").map_or(sheet, |s| s.1).to_string();
        if !run.causes.iter().any(|c| c.0 == sheet) {
            run.causes.push((sheet, why.to_string()));
        }
    }
    let done = line.strip_prefix(crate::sheets::DONE).map(PathBuf::from);
    if live {
        x.note(&l);
        if done.is_none() && (plant.total <= CHATTY || l.starts_with("error")) {
            log(&l);
        }
    } else {
        run.lines.push(l);
    }
    // una hoja de un bloque que ya está entera: se apunta en cuanto la orden lo dice
    if let Some(sheet) = done {
        let outputs = std::mem::take(&mut run.announced);
        let Some(&(_, h)) = run.sheets.iter().find(|s| s.0 == sheet) else {
            return Ok(());
        };
        // cada salida anunciada, con sus acompañantes
        let stems: HashSet<PathBuf> = outputs.iter().map(|o| o.with_extension("")).collect();
        let files: Vec<PathBuf> = walk(&m.dir)
            .into_iter()
            .filter(|f| stems.contains(&f.with_extension("")) && !run.written.contains(f))
            .collect();
        let listed = x.sized(&files);
        x.record(&h.hex(), &m.node.id, listed.clone())?;
        run.written.extend(files);
        run.track.steps[s].done.push((h, listed));
    }
    Ok(())
}

/// Recoge lo que ha dejado una orden que ha terminado y devuelve su recorrido.
fn finish(
    x: &mut Exec,
    plant: &Plant,
    mut run: Running,
    result: Result<()>,
    stopping: bool,
    tally: &mut Tally,
    log: &mut dyn FnMut(&str),
) -> Result<Track> {
    let s = run.track.step;
    let m = &plant.stage.members[s];
    let chatty = plant.total <= CHATTY;
    for l in std::mem::take(&mut run.lines) {
        x.note(&l);
        if !l.starts_with(crate::sheets::DONE) && (chatty || l.starts_with("error")) {
            log(&l);
        }
    }
    let _ = std::fs::remove_file(&run.skip_file);
    if result.is_ok() {
        let files = if run.unit.block {
            // lo que no es de ninguna hoja (el mosaico), y las órdenes que no avisan por hoja
            walk(&m.dir).into_iter().filter(|f| !run.written.contains(f)).collect()
        } else {
            publish(&run.staging, &m.dir)?
        };
        let listed = x.sized(&files);
        let hex = run.key.hex();
        x.record(&hex, &m.node.id, listed.clone())?;
        for p in x.products(run.key, &listed) {
            run.track.from.insert(p.path, hex.clone());
        }
        run.track.steps[s].done.push((run.key, listed));
    }
    let _ = std::fs::remove_dir_all(&run.staging);
    match result {
        Ok(()) => x.in_a_row = 0,
        // las que se detienen porque se para todo no son fallos suyos
        Err(_) if stopping => {}
        Err(e) => {
            let message = e.to_string();
            if let Some((_, why)) = run.causes.iter().find(|c| is_fatal(&c.1)) {
                return Err(Error::Param(why.clone()));
            }
            if is_fatal(&message) {
                return Err(e);
            }
            if run.causes.is_empty() {
                let message = message.trim_start_matches("error: ").to_string();
                run.causes.push((run.unit.label.clone(), message));
            }
            for (sheet, why) in &run.causes {
                if !chatty {
                    log(&format!("error: {sheet}: {why}"));
                }
                x.fail(&m.name, sheet, why)?;
                tally.failures[s] += 1;
            }
            run.track.steps[s].bad = true;
            x.in_a_row += 1;
            if x.in_a_row >= MAX_IN_A_ROW {
                return Err(Error::Param(format!(
                    "{MAX_IN_A_ROW} fallos seguidos en «{}», se para: {message}",
                    m.name
                )));
            }
        }
    }
    Ok(run.track)
}

pub(crate) fn execute(
    model: &Model,
    r: &Run,
    state: &mut dyn FnMut(&str, State),
    log: &mut dyn FnMut(&str),
) -> Result<Report> {
    let order = model
        .order()
        .ok_or_else(|| Error::Param("el flujo tiene un bucle".into()))?;
    std::fs::create_dir_all(r.results)?;
    let work = if r.resume {
        resumable(model, r.results).ok_or_else(|| {
            Error::Param(format!(
                "no hay en {} ninguna ejecución de este flujo sin terminar (si se cambió una opción o un hilo, hay que ejecutarlo de nuevo)",
                r.results.display()
            ))
        })?
    } else {
        let w = free_name(r.results, "ejecucion");
        std::fs::create_dir(&w)?;
        w
    };
    crate::paths::write_atomic(&work.join("flujo.model.json"), model.to_json())?;
    // los errores de un intento anterior se vuelven a comprobar
    let _ = std::fs::remove_file(work.join(ERRORS));
    let journal = Journal::open(&work, &signature(model))?;
    let log_file = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(work.join(LOG))?,
    );
    log(&format!("carpeta de esta ejecución: {}", work.display()));
    let mut x = Exec {
        r,
        work,
        journal,
        log_file,
        failed: 0,
        first_error: String::new(),
        in_a_row: 0,
        stamps: HashMap::new(),
    };
    for n in &model.nodes {
        state(&n.id, State::Waiting);
    }
    // una carpeta por nodo con su nombre, la misma al reanudar
    let mut dirs: HashMap<&str, PathBuf> = HashMap::new();
    let mut taken = HashSet::new();
    for n in model.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Tool { .. })) {
        let base = folder_name(&model.name(n));
        let mut name = base.clone();
        let mut k = 2;
        while !taken.insert(name.to_lowercase()) {
            name = format!("{base}_{k}");
            k += 1;
        }
        dirs.insert(n.id.as_str(), x.work.join(name));
    }
    let mut products: HashMap<String, Vec<Product>> = HashMap::new();
    let mut flawed: HashSet<&str> = HashSet::new();
    let mut pending: HashMap<&str, usize> = HashMap::new();
    for e in &model.edges {
        *pending.entry(e.from.as_str()).or_insert(0) += 1;
    }
    let mut done = vec![false; model.nodes.len()];
    let outcome = (|| -> Result<()> {
        // primero las entradas: no dependen de nada y así las tandas las ven ya leídas
        for &i in &order {
            let n = &model.nodes[i];
            let NodeKind::Input { path } = &n.kind else { continue };
            state(&n.id, State::Running);
            let before = x.failed;
            let path = r
                .inputs
                .get(&n.label)
                .or_else(|| r.inputs.get(&n.id))
                .unwrap_or(path);
            let mut made = Vec::new();
            for f in files_of(Path::new(path))? {
                if r.cancel.load(Ordering::Relaxed) {
                    return Err(Error::Param("cancelado".into()));
                }
                match broken(&f) {
                    Some(why) => {
                        let why = format!("archivo en mal estado: {why}");
                        log(&format!("error: {}: {why}", f.display()));
                        x.fail(&model.name(n), &f.display().to_string(), &why)?;
                    }
                    None => made.push(Product {
                        stamp: stamp_of(&f),
                        path: f,
                    }),
                }
            }
            if x.failed > before {
                flawed.insert(n.id.as_str());
            }
            products.insert(n.id.clone(), made);
            done[i] = true;
            state(&n.id, if x.failed > before { State::Failed } else { State::Done });
        }
        for &i in &order {
            if done[i] {
                continue;
            }
            let shape = shape(model, r, &order, i, &done, &products);
            for &m in &shape.nodes {
                state(&model.nodes[m].id, State::Running);
            }
            let outcomes = run_stage(&mut x, model, &shape, &dirs, &products, &flawed, log)?;
            for o in outcomes {
                let n = &model.nodes[o.node];
                let upstream = model
                    .edges
                    .iter()
                    .any(|e| e.to == n.id && flawed.contains(e.from.as_str()));
                if o.failed || upstream {
                    flawed.insert(n.id.as_str());
                }
                products.insert(n.id.clone(), o.products);
                done[o.node] = true;
                state(&n.id, if o.failed { State::Failed } else { State::Done });
            }
            // los intermedios que ya no necesita nadie se borran para dejar libre el disco, salvo
            // que algo haya fallado por el camino y haga falta al reanudar
            for &m in &shape.nodes {
                let n = &model.nodes[m];
                for e in model.edges.iter().filter(|e| e.to == n.id) {
                    let Some(c) = pending.get_mut(e.from.as_str()) else { continue };
                    *c -= 1;
                    let used = *c == 0
                        && model
                            .edges
                            .iter()
                            .filter(|o| o.from == e.from)
                            .all(|o| !flawed.contains(o.to.as_str()));
                    if used
                        && !flawed.contains(e.from.as_str())
                        && model.node(&e.from).is_some_and(|s| !model.kept(s))
                        && let Some(d) = dirs.get(e.from.as_str())
                    {
                        x.journal.line(&format!("{{\"purged\":{}}}", quote(&e.from)))?;
                        x.journal.purged.insert(e.from.clone());
                        let _ = std::fs::remove_dir_all(d);
                    }
                }
            }
        }
        Ok(())
    })();
    for d in dirs.values() {
        let _ = std::fs::remove_dir_all(d.join(STAGING));
        // solo se quita si ha quedado vacía
        let _ = std::fs::remove_dir(d);
    }
    let _ = x.log_file.flush();
    let mut report = Report {
        folder: x.work.clone(),
        outputs: Vec::new(),
        failed: x.failed,
        first_error: x.first_error.clone(),
    };
    for n in model.nodes.iter().filter(|n| model.kept(n)) {
        if matches!(n.kind, NodeKind::Input { .. }) {
            continue;
        }
        let files: Vec<PathBuf> = products
            .get(&n.id)
            .into_iter()
            .flatten()
            .map(|p| p.path.clone())
            .filter(|p| p.is_file())
            .collect();
        if !files.is_empty() {
            report.outputs.push((n.id.clone(), files));
        }
    }
    if let Err(e) = outcome {
        let _ = x.journal.file.sync_data();
        log(&format!(
            "se puede reanudar: lo terminado está en {}",
            report.folder.display()
        ));
        return Err(e);
    }
    if x.failed == 0 {
        x.journal.line("{\"fin\":true}")?;
    }
    x.journal.file.sync_data()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ArgSpec;

    fn spec(name: &str, args: &[(&str, bool)]) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            about: String::new(),
            args: args
                .iter()
                .map(|(id, multiple)| ArgSpec {
                    id: (*id).into(),
                    multiple: *multiple,
                    path: true,
                    ..ArgSpec::default()
                })
                .collect(),
        }
    }

    fn files(v: &[&str]) -> Files {
        Arc::new(v.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn two_folders_of_sheets_are_paired_by_name() -> Result<()> {
        let s = spec("change", &[("before", false), ("after", false)]);
        let sent = BTreeMap::from([
            ("before".to_string(), files(&["x/a_chm.tif", "x/b_chm.tif"])),
            ("after".to_string(), files(&["y/b_chm.tif", "y/a_chm.tif"])),
        ]);
        let p = plan(&s, &BTreeMap::new(), sent.clone(), &[])?;
        assert_eq!(p.units.len(), 2);
        assert_eq!(p.units[0].ports["after"], files(&["y/a_chm.tif"]));
        let mut odd = sent;
        odd.insert("after".to_string(), files(&["y/c_chm.tif", "y/a_chm.tif"]));
        let p = plan(&s, &BTreeMap::new(), odd, &[])?;
        assert_eq!((p.units.len(), p.missing.len()), (1, 1));
        Ok(())
    }

    #[test]
    fn each_cloud_is_a_unit_and_takes_its_own_model() -> Result<()> {
        let s = spec("chm", &[("inputs", true), ("dtm", true)]);
        let sent = BTreeMap::from([
            ("inputs".to_string(), files(&["a.laz", "b.laz"])),
            ("dtm".to_string(), files(&["m/b_mdt_1m.tif", "m/a_mdt_1m.tif"])),
        ]);
        let p = plan(&s, &BTreeMap::new(), sent.clone(), &[])?;
        assert_eq!(p.units.len(), 2);
        assert_eq!(p.units[1].ports["dtm"], files(&["m/b_mdt_1m.tif"]));
        // la nube sin MDT no se hace, y las otras llevan solo el suyo
        let mut short = sent.clone();
        short.insert("inputs".to_string(), files(&["a.laz", "b.laz", "c.laz"]));
        let p = plan(&s, &BTreeMap::new(), short, &[])?;
        assert_eq!((p.units.len(), p.missing.len()), (2, 1));
        assert_eq!(p.units[0].ports["dtm"], files(&["m/a_mdt_1m.tif"]));
        // con vecinas, el bloque entero en un proceso
        let args = BTreeMap::from([("neighbors".to_string(), vec!["30".to_string()])]);
        let p = plan(&s, &args, sent, &[])?;
        assert!(p.units.len() == 1 && p.units[0].block);
        Ok(())
    }

    #[test]
    fn a_tool_run_once_per_model_takes_the_cloud_of_the_same_name() -> Result<()> {
        let s = spec("x", &[("dtm", false), ("inputs", true)]);
        let sent = BTreeMap::from([
            ("inputs".to_string(), files(&["a.laz", "b.laz"])),
            ("dtm".to_string(), files(&["a_mdt_1m.tif", "b_mdt_1m.tif"])),
        ]);
        let p = plan(&s, &BTreeMap::new(), sent, &[])?;
        assert_eq!(p.units[0].ports["inputs"], files(&["a.laz"]));
        assert_eq!(p.units[1].ports["inputs"], files(&["b.laz"]));
        Ok(())
    }

    #[test]
    fn the_calculator_runs_once_per_sheet_with_one_wire_per_band() -> Result<()> {
        let s = spec("calc", &[("inputs", true)]);
        let args = BTreeMap::from([("expr".to_string(), vec!["A - B".to_string()])]);
        let wires = [
            files(&["a_mds.tif", "b_mds.tif"]),
            files(&["a_mdt.tif", "b_mdt.tif"]),
        ];
        let p = plan(&s, &args, BTreeMap::new(), &wires)?;
        assert_eq!(p.units.len(), 2);
        assert_eq!(p.units[1].ports["inputs"], files(&["b_mds.tif", "b_mdt.tif"]));
        // un solo hilo con dos archivos y dos letras: son las dos bandas
        assert!(!calc_by_sheet(&args, &wires[..1]));
        Ok(())
    }

    #[test]
    fn big_clouds_run_a_few_at_a_time_and_small_files_many() {
        // hojas de 100 MB en un equipo de 10 núcleos: dos con 16 GB, una con 4 GB
        assert_eq!(automatic_jobs(100 << 20, 10, 16 << 30), 2);
        assert_eq!(automatic_jobs(100 << 20, 10, 4 << 30), 1);
        assert_eq!(automatic_jobs(100 << 10, 10, 16 << 30), 10);
        assert_eq!(automatic_jobs(1 << 30, 32, 64 << 30), 1);
    }

    /// Una cadena con un `hipso` de pega que cae la primera vez con una de las teselas: cada una
    /// recorre la cadena, la caída se repite sola y los intermedios no quedan.
    #[cfg(unix)]
    #[test]
    fn a_chain_runs_sheet_by_sheet_and_retries_a_crash_alone() -> Result<()> {
        use crate::model::{Edge, Schema};
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("hipso_cadena_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let clouds = dir.join("nubes");
        std::fs::create_dir_all(&clouds)?;
        for name in ["a", "b", "c"] {
            crate::hag::tests::write_las(&clouds.join(format!("{name}.las")), &[(1.0, 2.0, 3.0, 2)])?;
        }
        let exe = dir.join("pega.sh");
        std::fs::write(
            &exe,
            "#!/bin/sh\ndir=$2; tool=$3; for a; do last=$a; done\n\
             name=$(basename \"$last\"); name=${name%.*}\n\
             if [ $name = b ] && mkdir \"$(dirname \"$0\")/caida\" 2>/dev/null; then kill -9 $$; fi\n\
             printf x > \"$dir/${name}_$tool.tif\"\n",
        )?;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
        let arg = |id: &str, positional: bool| ArgSpec {
            id: id.into(),
            long: (!positional).then(|| id.into()),
            positional,
            multiple: true,
            path: true,
            ..ArgSpec::default()
        };
        let tool = |name: &str, args: Vec<ArgSpec>| ToolSpec {
            name: name.into(),
            about: String::new(),
            args,
        };
        let schema = Schema {
            tools: vec![
                tool("dtm", vec![arg("inputs", true)]),
                tool("chm", vec![arg("inputs", true), arg("dtm", false)]),
            ],
        };
        let node = |id: &str, kind| Node {
            id: id.into(),
            label: id.into(),
            kind,
            keep: false,
            x: 0.0,
            y: 0.0,
        };
        let of = |t: &str| NodeKind::Tool {
            tool: t.into(),
            args: BTreeMap::new(),
        };
        let edge = |from: &str, to: &str, port: &str| Edge {
            from: from.into(),
            to: to.into(),
            port: port.into(),
        };
        let model = Model {
            nodes: vec![
                node("A", NodeKind::Input { path: clouds.display().to_string() }),
                node("M", of("dtm")),
                node("C", of("chm")),
            ],
            edges: vec![edge("A", "M", "inputs"), edge("A", "C", "inputs"), edge("M", "C", "dtm")],
            ..Model::default()
        };
        let (cancel, inputs, results) = (AtomicBool::new(false), HashMap::new(), dir.join("r"));
        let run = Run {
            exe: &exe,
            schema: &schema,
            results: &results,
            inputs: &inputs,
            cancel: &cancel,
            resume: false,
            jobs: 2,
        };
        let mut said = Vec::new();
        let report = execute(&model, &run, &mut |_, _| {}, &mut |l| said.push(l.to_string()))?;
        assert_eq!(report.failed, 0, "{said:?}");
        assert!(said.iter().any(|l| l.contains("de principio a fin")), "{said:?}");
        assert!(said.iter().any(|l| l.contains("se repite sola")), "{said:?}");
        assert_eq!(walk(&report.folder.join("C")).len(), 3);
        assert!(!report.folder.join("M").exists(), "los intermedios se borran");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn planning_a_huge_delivery_is_quick() -> Result<()> {
        let n = 200_000;
        let list = |f: &dyn Fn(usize) -> String| Arc::new((0..n).map(|i| PathBuf::from(f(i))).collect::<Vec<_>>());
        let s = spec("chm", &[("inputs", true), ("dtm", true)]);
        let sent = BTreeMap::from([
            ("inputs".to_string(), list(&|i| format!("n/h_{i}.laz"))),
            ("dtm".to_string(), list(&|i| format!("m/h_{i}_mdt_1m.tif"))),
        ]);
        let t = Instant::now();
        let p = plan(&s, &BTreeMap::new(), sent, &[])?;
        assert_eq!((p.units.len(), p.missing.len()), (n, 0));
        assert!(t.elapsed().as_secs() < 30, "{:?}", t.elapsed());
        Ok(())
    }
}
