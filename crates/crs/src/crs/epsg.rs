//! La tabla EPSG que lleva terrio: nombres oficiales, WKT, conversiones de coordenadas como cadenas
//! PROJ, las transformaciones de datum que usa GDAL (sin rejillas y con ellas), sistemas verticales
//! y compuestos y modelos de geoide, generada a partir de GDAL y PROJ por
//! `scripts/gen_epsg_table.py`. La tabla ESRI (`scripts/gen_esri_table.py`) guarda de la misma
//! forma los sistemas geográficos y proyectados de la autoridad ESRI de PROJ.
//!
//! Derivada del conjunto de parámetros geodésicos de EPSG (<https://epsg.org>), cuyos términos de
//! uso permiten la redistribución con atribución, y de la base de datos de PROJ; la tabla ESRI y
//! los nombres de datum de ESRI proceden de los datos del motor de proyecciones de Esri
//! (Apache-2.0). Véase `NOTICE`.

use super::datum::{Op, parse_steps};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, OnceLock};

static DATA: &[u8] = include_bytes!("../../data/epsg.tsv.zst");
static ESRI_DATA: &[u8] = include_bytes!("../../data/esri.tsv.zst");
/// Transformaciones entre sistemas verticales EPSG (`scripts/gen_vertical_table.py`).
static VERTICAL_DATA: &[u8] = include_bytes!("../../data/epsg_vertical.tsv.zst");

/// Los nombres y alias de los datums EPSG, generados a partir de PROJ por
/// `scripts/gen_datum_names.py`: líneas `código<TAB>rango<TAB>nombre`.
static DATUM_NAMES: &[u8] = include_bytes!("../../data/datums.tsv.zst");

/// Un sistema EPSG (o ESRI) de la tabla.
#[derive(Debug)]
pub(crate) struct Entry {
    /// El nombre oficial.
    pub name: String,
    /// El código EPSG de su datum, 0 cuando se desconoce.
    pub datum: u32,
    /// El código EPSG de su sistema geográfico, 0 cuando se desconoce.
    pub geog: u32,
    /// Sus transformaciones de datum a WGS 84, en el orden en que las prueba GDAL cuando no hay
    /// rejillas instaladas.
    pub ops: Vec<Arc<Op>>,
    /// Las que prueba GDAL cuando hay rejillas disponibles (vacío cuando son las mismas).
    pub grid_ops: Vec<Arc<Op>>,
    /// Si es un sistema dinámico (su datum es un marco de referencia dinámico).
    pub dynamic: bool,
    /// Si es un sistema geográfico 3D.
    pub three_d: bool,
    /// Su área de uso: oeste, sur, este y norte en grados.
    pub area: Option<[f64; 4]>,
    /// La cadena PROJ de su conversión de coordenadas (sin ajuste de datum).
    pub proj: String,
    /// El WKT que escribe GDAL para él.
    pub wkt: String,
}

/// Un sistema vertical EPSG.
#[derive(Debug)]
pub(crate) struct Vertical {
    pub name: String,
    /// El código del datum vertical EPSG.
    pub datum: u32,
    /// Metros por unidad.
    pub to_meter: f64,
    /// Si las alturas son profundidades (positivas hacia abajo).
    pub down: bool,
    pub wkt: String,
}

/// Un sistema compuesto EPSG: un sistema horizontal y uno vertical.
#[derive(Debug)]
pub(crate) struct Compound {
    pub name: String,
    pub horizontal: u32,
    pub vertical: u32,
    pub wkt: String,
}

/// Transformaciones sin rejillas y con ellas.
type OpLists = (Vec<Arc<Op>>, Vec<Arc<Op>>);

/// Una transformación entre sistemas verticales, con el sistema geográfico en el que interpola.
#[derive(Debug, Clone)]
pub(crate) struct VerticalOp {
    pub op: Arc<Op>,
    /// El sistema geográfico EPSG al que se refieren sus rejillas o desfases: PROJ le encadena la
    /// transformación horizontal cuando los sistemas compuestos están sobre otro datum.
    pub interpolation: Option<u32>,
}

/// Transformaciones entre sistemas verticales sin rejillas y con ellas.
type VerticalLists = (Vec<VerticalOp>, Vec<VerticalOp>);

/// Las transformaciones que GDAL lista de un sistema geográfico a otro, sin rejillas y con ellas.
struct PairLists {
    lists: OpLists,
}

/// La tabla: su texto, descomprimido una vez, y un índice de sus líneas. Las entradas se analizan
/// la primera vez que se usan.
struct Table {
    text: String,
    crs: HashMap<u32, Lazy<Option<Entry>>>,
    ops: HashMap<u32, Lazy<Option<Arc<Op>>>>,
    /// Transformaciones directas entre sistemas geográficos, por código (origen, destino).
    pairs: HashMap<(u32, u32), Lazy<PairLists>>,
    verticals: HashMap<u32, Lazy<Option<Vertical>>>,
    compounds: HashMap<u32, Lazy<Option<Compound>>>,
    /// Modelos de geoide de sistemas geográficos 3D a sistemas verticales, por sistema vertical.
    geoids: HashMap<u32, Vec<(u32, Lazy<OpLists>)>>,
    /// Transformaciones entre sistemas verticales, por código (origen, destino).
    vertical_ops: HashMap<(u32, u32), Lazy<VerticalLists>>,
    /// Nombres antiguos de las rejillas.
    alternatives: HashMap<String, Vec<String>>,
    /// Las fechas de publicación de los datums (`2007-01-01`), por código de datum.
    dates: HashMap<u32, String>,
    /// Las líneas de los sistemas geodésicos (`E`) y de las operaciones registradas (`R`), según su
    /// tipo.
    geodetic: Vec<(usize, usize)>,
    registered: Vec<(usize, usize)>,
}

/// Una línea de la tabla (sus campos tras el código) y en qué se convierte al analizarla.
struct Lazy<T> {
    line: (u32, u32),
    value: OnceLock<T>,
}

impl<T> Lazy<T> {
    fn new(start: usize, end: usize) -> Self {
        Self {
            line: (start as u32, end as u32),
            value: OnceLock::new(),
        }
    }

    fn get(&self, text: &str, parse: impl FnOnce(&str) -> T) -> &T {
        self.value
            .get_or_init(|| parse(&text[self.line.0 as usize..self.line.1 as usize]))
    }
}

/// El texto descomprimido de una tabla.
fn decompress(data: &[u8]) -> String {
    let mut text = String::new();
    if let Ok(mut d) = ruzstd::decoding::StreamingDecoder::new(&mut &data[..]) {
        let _ = d.read_to_string(&mut text);
    }
    text
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| load(DATA))
}

fn vertical_table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| load(VERTICAL_DATA))
}

fn esri_table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| load(ESRI_DATA))
}

/// Descomprime una tabla e indexa sus líneas.
fn load(data: &[u8]) -> Table {
    let text = decompress(data);
    let (mut crs, mut ops, mut pairs) = (HashMap::new(), HashMap::new(), HashMap::new());
    let (mut verticals, mut compounds, mut geoids) = (
        HashMap::new(),
        HashMap::new(),
        HashMap::<u32, Vec<_>>::new(),
    );
    let mut vertical_ops = HashMap::new();
    let mut alternatives = HashMap::<String, Vec<String>>::new();
    let mut dates = HashMap::new();
    let (mut geodetic, mut registered) = (Vec::new(), Vec::new());
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let start = pos;
        pos += line.len();
        let line = line.trim_end_matches(['\n', '\r']);
        let end = start + line.len();
        let mut f = line.splitn(3, '\t');
        let (Some(kind), Some(key)) = (f.next(), f.next()) else {
            continue;
        };
        // El resto de la línea, tras los campos de tipo y clave.
        let rest = (start + kind.len() + key.len() + 2).min(end);
        // El resto tras un campo más.
        let after_second = || {
            (rest
                + line[rest - start..]
                    .find('\t')
                    .map_or(end - rest, |i| i + 1))
            .min(end)
        };
        let second = || {
            line[rest - start..]
                .split('\t')
                .next()
                .and_then(|v| v.parse::<u32>().ok())
        };
        match kind {
            "O" => {
                if let Ok(id) = key.parse::<u32>() {
                    ops.insert(id, Lazy::new(rest, end));
                }
            }
            "C" => {
                if let Ok(code) = key.parse::<u32>() {
                    crs.insert(code, Lazy::new(rest, end));
                }
            }
            "P" => {
                if let (Ok(a), Some(b)) = (key.parse::<u32>(), second()) {
                    pairs.insert((a, b), Lazy::new(after_second(), end));
                }
            }
            "V" => {
                if let Ok(code) = key.parse::<u32>() {
                    verticals.insert(code, Lazy::new(rest, end));
                }
            }
            "K" => {
                if let Ok(code) = key.parse::<u32>() {
                    compounds.insert(code, Lazy::new(rest, end));
                }
            }
            "G" => {
                if let (Ok(g), Some(v)) = (key.parse::<u32>(), second()) {
                    geoids
                        .entry(v)
                        .or_default()
                        .push((g, Lazy::new(after_second(), end)));
                }
            }
            "W" => {
                if let (Ok(a), Some(b)) = (key.parse::<u32>(), second()) {
                    vertical_ops.insert((a, b), Lazy::new(after_second(), end));
                }
            }
            "A" => {
                alternatives
                    .entry(key.to_string())
                    .or_default()
                    .push(line[rest - start..].to_string());
            }
            "D" => {
                if let Ok(code) = key.parse::<u32>() {
                    dates.insert(code, line[rest - start..].to_string());
                }
            }
            "E" => geodetic.push((start + 2, end)),
            "R" => registered.push((start + 2, end)),
            _ => {}
        }
    }
    Table {
        text,
        crs,
        ops,
        pairs,
        verticals,
        compounds,
        geoids,
        vertical_ops,
        alternatives,
        dates,
        geodetic,
        registered,
    }
}

impl Table {
    /// La transformación con un identificador, analizada a partir de su línea (`bounds`,
    /// `accuracy`, `flags`, `steps`, `name`, `chain accuracy`, `chain bounds`).
    fn op(&self, id: u32) -> Option<Arc<Op>> {
        self.ops
            .get(&id)?
            .get(&self.text, |line| {
                let mut f = line.split('\t');
                let (Some(bounds), Some(accuracy), Some(flags)) = (f.next(), f.next(), f.next())
                else {
                    return None;
                };
                let b: Vec<f64> = bounds.split(',').filter_map(|v| v.parse().ok()).collect();
                let (true, Some(steps)) = (b.len() == 4, parse_steps(f.next().unwrap_or("")))
                else {
                    return None;
                };
                let name = f.next().unwrap_or("").to_string();
                let accuracy = accuracy.parse().unwrap_or(-1.0);
                let bounds = [b[0], b[1], b[2], b[3]];
                Some(Arc::new(Op {
                    bounds,
                    accuracy,
                    chain_accuracy: f.next().and_then(|a| a.parse().ok()).unwrap_or(accuracy),
                    chain_bounds: f
                        .next()
                        .map(|v| {
                            v.split(',')
                                .filter_map(|v| v.parse().ok())
                                .collect::<Vec<f64>>()
                        })
                        .and_then(|v| <[f64; 4]>::try_from(v).ok())
                        .unwrap_or(bounds),
                    offshore: flags.contains('o'),
                    unknown_area: flags.contains('u'),
                    ballpark: flags.contains('b'),
                    concatenated: flags.contains('c'),
                    between_3d: flags.contains('h'),
                    geocentric: flags.contains('g'),
                    priority: flags.contains('p'),
                    keeps_height: flags.contains('z'),
                    steps,
                    name,
                }))
            })
            .clone()
    }

    /// El campo de la línea de una transformación tras los otros 8 (el sistema de interpolación de
    /// la tabla vertical).
    fn op_interpolation(&self, id: u32) -> Option<u32> {
        let lazy = self.ops.get(&id)?;
        self.text[lazy.line.0 as usize..lazy.line.1 as usize]
            .split('\t')
            .nth(7)?
            .parse()
            .ok()
    }

    /// Transformaciones verticales por listas `ops<TAB>grid ops` de identificadores.
    fn vertical_lists(&self, ids: &str) -> VerticalLists {
        let list = |s: &str| -> Vec<VerticalOp> {
            s.split(',')
                .filter_map(|i| i.parse().ok())
                .filter_map(|i| {
                    Some(VerticalOp {
                        op: self.op(i)?,
                        interpolation: self.op_interpolation(i),
                    })
                })
                .collect()
        };
        let (a, b) = ids.split_once('\t').unwrap_or((ids, ""));
        (list(a), list(b))
    }

    /// Transformaciones por una lista de identificadores separados por comas.
    fn resolve(&self, ids: &str) -> Vec<Arc<Op>> {
        ids.split(',')
            .filter_map(|i| i.parse().ok())
            .filter_map(|i| self.op(i))
            .collect()
    }

    fn entry(&self, code: u32) -> Option<&Entry> {
        self.crs
            .get(&code)?
            .get(&self.text, |line| {
                let fields: Vec<&str> = line.split('\t').collect();
                let [name, datum, geog, op_ids, grid_ids, flags, area, proj, wkt] = fields[..]
                else {
                    return None;
                };
                let area: Vec<f64> = area.split(',').filter_map(|v| v.parse().ok()).collect();
                Some(Entry {
                    name: name.to_string(),
                    datum: datum.parse().unwrap_or(0),
                    geog: geog.parse().unwrap_or(0),
                    ops: self.resolve(op_ids),
                    grid_ops: self.resolve(grid_ids),
                    dynamic: flags.contains('d'),
                    three_d: flags.contains('3'),
                    area: (area.len() == 4).then(|| [area[0], area[1], area[2], area[3]]),
                    proj: proj.to_string(),
                    wkt: wkt.to_string(),
                })
            })
            .as_ref()
    }

    fn lists(&self, ids: &str) -> OpLists {
        let (a, b) = ids.split_once('\t').unwrap_or((ids, ""));
        (self.resolve(a), self.resolve(b))
    }

    fn pair(&self, a: u32, b: u32) -> Option<&PairLists> {
        Some(self.pairs.get(&(a, b))?.get(&self.text, |ids| {
            let mut f = ids.split('\t');
            let mut next = || self.resolve(f.next().unwrap_or(""));
            PairLists {
                lists: (next(), next()),
            }
        }))
    }
}

/// Las operaciones que usa GDAL dadas las rejillas que se pueden encontrar: las listadas con
/// rejillas, sin aquellas cuyas rejillas faltan, cuando queda al menos una operación que necesita
/// rejillas; si no, las listadas sin rejillas.
pub(crate) fn usable(ops: &[Arc<Op>], grid_ops: &[Arc<Op>]) -> Vec<Arc<Op>> {
    if grid_ops.is_empty() {
        return ops.to_vec();
    }
    let kept: Vec<Arc<Op>> = grid_ops.iter().filter(|o| o.available()).cloned().collect();
    if kept.iter().any(|o| o.needs_grids() && !o.ballpark) {
        kept
    } else {
        ops.to_vec()
    }
}

/// Empieza a cargar la tabla en segundo plano, para trabajos que la necesitarán tras leer su
/// entrada (compilaciones nativas; no hace nada en WebAssembly).
pub(crate) fn prefetch() {
    #[cfg(not(target_family = "wasm"))]
    rayon::spawn(|| {
        table();
    });
}

/// La entrada de la tabla para un código EPSG.
pub(crate) fn get(code: u32) -> Option<&'static Entry> {
    table().entry(code)
}

/// La entrada de la tabla para un código ESRI.
pub(crate) fn esri(code: u32) -> Option<&'static Entry> {
    esri_table().entry(code)
}

/// Un índice de los sistemas de una tabla por nombre, tal como lo da `key`.
fn name_index(t: &Table, key: impl Fn(&str) -> String) -> HashMap<String, Vec<u32>> {
    let mut names = HashMap::<String, Vec<u32>>::new();
    for (code, l) in &t.crs {
        if let Some(n) = t.text[l.line.0 as usize..l.line.1 as usize]
            .split('\t')
            .next()
        {
            names.entry(key(n)).or_default().push(*code);
        }
    }
    names.values_mut().for_each(|c| c.sort_unstable());
    names
}

/// Los códigos de los sistemas con un nombre oficial, sea cual sea su capitalización, en orden: a
/// partir de un índice de nombres construido la primera vez que se usa.
pub(crate) fn named(name: &str) -> Vec<u32> {
    static NAMES: OnceLock<HashMap<String, Vec<u32>>> = OnceLock::new();
    let names = NAMES.get_or_init(|| name_index(table(), str::to_ascii_lowercase));
    names
        .get(&name.to_ascii_lowercase())
        .cloned()
        .unwrap_or_default()
}

/// Un nombre de datum tal como se comparan los datums por el nombre: sus letras ASCII, dígitos y
/// `+` (CH1903+ no es CH1903), sea cual sea su capitalización, sin el prefijo `D_` de ESRI.
pub(crate) fn datum_key(name: &str) -> String {
    let name = name.strip_prefix("D_").unwrap_or(name);
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '+')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// El rango de los nombres que ESRI escribe para sistemas sobre otro datum (`D_ETRS_1989` para
/// ETRS89-NOR).
const ESRI_WRITTEN: u8 = 5;

/// Los nombres de datum de la tabla por rango: los de los rangos 0 a 4 y los que escribe ESRI.
type DatumNames = (HashMap<String, (u8, Vec<u32>)>, HashMap<String, Vec<u32>>);

fn datum_names() -> &'static DatumNames {
    static NAMES: OnceLock<DatumNames> = OnceLock::new();
    NAMES.get_or_init(|| {
        let (mut names, mut written) = (
            HashMap::<String, (u8, Vec<u32>)>::new(),
            HashMap::<String, Vec<u32>>::new(),
        );
        for line in decompress(DATUM_NAMES)
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let mut f = line.splitn(3, '\t');
            let (Some(Ok(code)), Some(Ok(rank)), Some(name)) = (
                f.next().map(str::parse::<u32>),
                f.next().map(str::parse::<u8>),
                f.next(),
            ) else {
                continue;
            };
            if rank == ESRI_WRITTEN {
                written.entry(datum_key(name)).or_default().push(code);
                continue;
            }
            let e = names.entry(datum_key(name)).or_insert((rank, Vec::new()));
            if rank < e.0 {
                *e = (rank, Vec::new());
            }
            if rank == e.0 && !e.1.contains(&code) {
                e.1.push(code);
            }
        }
        names.values_mut().for_each(|(_, c)| c.sort_unstable());
        written.values_mut().for_each(|c| c.sort_unstable());
        // Sin ello, todo WKT sin código de datum se quedaría sin código EPSG.
        debug_assert!(!names.is_empty(), "the datum name table did not decompress");
        (names, written)
    })
}

/// Los datums EPSG que representa un nombre, como los lista la base de datos de PROJ: aquellos de
/// los que es el nombre oficial (o el nombre WKT 1 de GDAL), si no un alias EPSG, si no un nombre
/// de PROJ, de ESRI (`D_ETRS_1989`) o antiguo, por orden de código.
pub(crate) fn datums_named(name: &str) -> Vec<u32> {
    datum_names()
        .0
        .get(&datum_key(name))
        .map(|(_, c)| c.clone())
        .unwrap_or_default()
}

/// Los otros datums EPSG cuyos sistemas escribe ESRI con un nombre de datum: ETRS89-NOR para
/// `D_ETRS_1989` (ETRS89-NOR [EUREF89] / NTM zone 18, EPSG:5118), S-JTSK (Ferro) para `D_S_JTSK`.
pub(crate) fn datums_written_as(name: &str) -> &'static [u32] {
    datum_names()
        .1
        .get(&datum_key(name))
        .map_or(&[], Vec::as_slice)
}

/// Los sistemas geográficos 2D de EPSG sobre un datum, sin los obsoletos, por orden de código.
pub(crate) fn geographic_on(datum: u32) -> &'static [u32] {
    static INDEX: OnceLock<HashMap<u32, Vec<u32>>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let t = table();
        let mut index = HashMap::<u32, Vec<u32>>::new();
        for (code, l) in &t.crs {
            let f: Vec<&str> = t.text[l.line.0 as usize..l.line.1 as usize]
                .splitn(9, '\t')
                .collect();
            // Un código obsoleto tiene el sistema geográfico de su sustituto.
            let [_, datum_field, geog, _, _, flags, _, proj, ..] = f[..] else {
                continue;
            };
            if geog.parse() == Ok(*code)
                && !flags.contains('3')
                && proj.starts_with("+proj=longlat")
                && let Ok(d) = datum_field.parse::<u32>()
            {
                index.entry(d).or_default().push(*code);
            }
        }
        index.values_mut().for_each(|c| c.sort_unstable());
        index
    });
    index.get(&datum).map_or(&[], Vec::as_slice)
}

/// Los sistemas proyectados de EPSG sobre un sistema geográfico, sin los obsoletos, por orden de
/// código, con su método de proyección (`+proj=tmerc`).
pub(crate) fn projected_on(geog: u32) -> &'static [(u32, &'static str)] {
    static INDEX: OnceLock<HashMap<u32, Vec<(u32, &'static str)>>> = OnceLock::new();
    let index = INDEX.get_or_init(|| {
        let t = table();
        let mut index = HashMap::<u32, Vec<(u32, &'static str)>>::new();
        for (code, l) in &t.crs {
            let f: Vec<&'static str> = t.text[l.line.0 as usize..l.line.1 as usize]
                .splitn(9, '\t')
                .collect();
            let [_, _, geog, _, _, flags, _, proj, wkt] = f[..] else {
                continue;
            };
            // Un código obsoleto tiene el WKT de su sustituto.
            let current = super::declared_epsg(wkt) == Some(*code);
            let method = proj.split_whitespace().next().unwrap_or_default();
            if current
                && !flags.contains('3')
                && method != "+proj=longlat"
                && let Ok(g) = geog.parse::<u32>()
            {
                index.entry(g).or_default().push((*code, method));
            }
        }
        index.values_mut().for_each(|c| c.sort_unstable());
        index
    });
    index.get(&geog).map_or(&[], Vec::as_slice)
}

/// Los códigos ESRI de los sistemas con un nombre, sea cual sea su capitalización y con espacios
/// por guiones bajos (`World Mollweide` por `World_Mollweide`), en orden.
pub(crate) fn esri_named(name: &str) -> Vec<u32> {
    static NAMES: OnceLock<HashMap<String, Vec<u32>>> = OnceLock::new();
    let key = |n: &str| n.to_ascii_lowercase().replace(' ', "_");
    let names = NAMES.get_or_init(|| name_index(esri_table(), key));
    names.get(&key(name)).cloned().unwrap_or_default()
}

/// El tipo de un sistema geodésico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GeodeticKind {
    Geographic2D,
    Geographic3D,
    Geocentric,
}

/// Un sistema geodésico de EPSG (geográfico 2D o 3D, o geocéntrico), como PROJ encadena
/// transformaciones a través de ellos.
#[derive(Debug)]
pub(crate) struct Geodetic {
    pub code: u32,
    pub datum: u32,
    pub kind: GeodeticKind,
    pub name: String,
}

/// Una transformación que la base de datos de PROJ registra entre dos sistemas geodésicos de datums
/// distintos.
#[derive(Debug)]
pub(crate) struct Registered {
    /// Su autoridad y código (`EPSG:1321`).
    pub key: String,
    /// Los sistemas de los que parte y a los que llega según la base de datos de PROJ.
    pub source: u32,
    pub target: u32,
    /// La transformación, ejecutada entre las coordenadas geográficas de los dos datums.
    pub op: Arc<Op>,
    /// Las claves de las transformaciones que la sustituyen entre los mismos sistemas.
    pub replaced_by: Vec<String>,
    /// Si necesita las épocas de las coordenadas (una transformación dependiente del tiempo).
    pub time_dependent: bool,
    /// Si es la transformación nula de PROJ entre un conjunto de datums y uno de sus miembros (WGS
    /// 84 a WGS 84 (G1762)).
    pub ensemble: bool,
    /// Los sistemas por los que pasa una concatenación.
    pub via: Vec<u32>,
}

/// Los sistemas geodésicos y las transformaciones registradas con las que PROJ encadena
/// transformaciones.
#[derive(Debug, Default)]
pub(crate) struct Registry {
    pub geodetic: HashMap<u32, Geodetic>,
    /// Los códigos de los sistemas geodésicos de cada datum, en orden.
    pub by_datum: HashMap<u32, Vec<u32>>,
    pub ops: Vec<Registered>,
    /// Los índices en `ops` de las transformaciones registradas desde o hacia cada sistema.
    pub at: HashMap<u32, Vec<usize>>,
}

/// Los sistemas geodésicos y las transformaciones que la base de datos de PROJ registra entre
/// ellos, que terrio encadena a través de datums intermedios como lo hace PROJ.
pub(crate) fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let t = table();
        let mut r = Registry::default();
        for &(start, end) in &t.geodetic {
            let f: Vec<&str> = t.text[start..end].splitn(4, '\t').collect();
            let [code, datum, kind, name] = f[..] else {
                continue;
            };
            let kind = match kind {
                "2" => GeodeticKind::Geographic2D,
                "3" => GeodeticKind::Geographic3D,
                "g" => GeodeticKind::Geocentric,
                _ => continue,
            };
            let (Ok(code), Ok(datum)) = (code.parse(), datum.parse()) else {
                continue;
            };
            r.by_datum.entry(datum).or_default().push(code);
            r.geodetic.insert(
                code,
                Geodetic {
                    code,
                    datum,
                    kind,
                    name: name.to_string(),
                },
            );
        }
        r.by_datum.values_mut().for_each(|c| c.sort_unstable());
        let codes = |s: &str| {
            s.split(',')
                .filter_map(|v| v.parse().ok())
                .collect::<Vec<u32>>()
        };
        for &(start, end) in &t.registered {
            let f: Vec<&str> = t.text[start..end].split('\t').collect();
            let [key, source, target, id, replaced_by, flags, via] = f[..] else {
                continue;
            };
            let (Ok(source), Ok(target), Some(op)) = (
                source.parse(),
                target.parse(),
                id.parse().ok().and_then(|i| t.op(i)),
            ) else {
                continue;
            };
            let i = r.ops.len();
            r.at.entry(source).or_default().push(i);
            r.at.entry(target).or_default().push(i);
            r.ops.push(Registered {
                key: key.to_string(),
                source,
                target,
                op,
                replaced_by: replaced_by
                    .split(',')
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .collect(),
                time_dependent: flags.contains('t'),
                ensemble: flags.contains('e'),
                via: codes(via),
            });
        }
        r
    })
}

/// Las transformaciones directas de un sistema geográfico a otro (las que usa GDAL dadas las
/// rejillas disponibles) y si se ejecutan a la inversa (la tabla guarda cada par una vez).
pub(crate) fn pair(from: u32, to: u32) -> Option<(Vec<Arc<Op>>, bool)> {
    let t = table();
    let (p, reverse) = t
        .pair(from, to)
        .map(|o| (o, false))
        .or_else(|| t.pair(to, from).map(|o| (o, true)))?;
    Some((usable(&p.lists.0, &p.lists.1), reverse))
}

/// Un sistema vertical EPSG.
pub(crate) fn vertical(code: u32) -> Option<&'static Vertical> {
    let t = table();
    t.verticals
        .get(&code)?
        .get(&t.text, |line| {
            let f: Vec<&str> = line.split('\t').collect();
            let [name, datum, unit, direction, wkt] = f[..] else {
                return None;
            };
            Some(Vertical {
                name: name.to_string(),
                datum: datum.parse().unwrap_or(0),
                to_meter: unit.parse().unwrap_or(1.0),
                down: direction == "down",
                wkt: wkt.to_string(),
            })
        })
        .as_ref()
}

/// Un sistema compuesto EPSG.
pub(crate) fn compound(code: u32) -> Option<&'static Compound> {
    let t = table();
    t.compounds
        .get(&code)?
        .get(&t.text, |line| {
            let f: Vec<&str> = line.split('\t').collect();
            let [name, h, v, wkt] = f[..] else {
                return None;
            };
            Some(Compound {
                name: name.to_string(),
                horizontal: h.parse().ok()?,
                vertical: v.parse().ok()?,
                wkt: wkt.to_string(),
            })
        })
        .as_ref()
}

/// Los modelos de geoide (transformaciones desde sistemas geográficos 3D) de un sistema vertical,
/// con el código de su sistema geográfico 3D, como los usa GDAL dadas las rejillas disponibles.
pub(crate) fn geoid_models(vertical: u32) -> Vec<(u32, Vec<Arc<Op>>)> {
    let t = table();
    let Some(list) = t.geoids.get(&vertical) else {
        return Vec::new();
    };
    list.iter()
        .map(|(g, lazy)| {
            let (ops, grid_ops) = lazy.get(&t.text, |ids| t.lists(ids));
            (*g, usable(ops, grid_ops))
        })
        .filter(|(_, ops)| !ops.is_empty())
        .collect()
}

/// Las transformaciones de un sistema vertical a otro que lista PROJ (desfases verticales, desfases
/// y pendientes, VERTCON y otras rejillas, encadenadas a través de un sistema vertical intermedio),
/// dadas las rejillas que se pueden encontrar. Vacío cuando el conjunto de datos EPSG los conecta
/// solo por la aproximación de PROJ.
pub(crate) fn vertical_transformations(source: u32, target: u32) -> Vec<VerticalOp> {
    let t = vertical_table();
    let Some(lazy) = t.vertical_ops.get(&(source, target)) else {
        return Vec::new();
    };
    let (ops, grid_ops) = lazy.get(&t.text, |ids| t.vertical_lists(ids));
    let arcs = |l: &[VerticalOp]| l.iter().map(|v| v.op.clone()).collect::<Vec<_>>();
    let chosen = usable(&arcs(ops), &arcs(grid_ops));
    let all: Vec<&VerticalOp> = grid_ops.iter().chain(ops.iter()).collect();
    chosen
        .iter()
        .filter_map(|c| {
            all.iter()
                .find(|v| Arc::ptr_eq(&v.op, c))
                .map(|v| (*v).clone())
        })
        .collect()
}

/// La fecha de publicación de un datum EPSG (`2007-01-01`), cuando la tiene.
pub(crate) fn datum_date(datum: u32) -> Option<&'static str> {
    table().dates.get(&datum).map(String::as_str)
}

/// Los nombres que tuvo una rejilla de PROJ-data antes de PROJ 7.
pub(crate) fn grid_alternatives(name: &str) -> Vec<String> {
    table().alternatives.get(name).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_loads() {
        let e = get(2263).unwrap();
        assert_eq!(e.name, "NAD83 / New York Long Island (ftUS)");
        assert!(e.proj.contains("+units=us-ft"));
        assert!(
            e.wkt
                .starts_with("PROJCS[\"NAD83 / New York Long Island (ftUS)\"")
        );
        assert_eq!(e.datum, 6269);
        assert!(get(23030).unwrap().ops.len() > 3);
        assert_eq!(get(31467).unwrap().geog, 4314);
        assert!(pair(4314, 4258).is_some());
        assert_eq!(vertical(5773).unwrap().name, "EGM96 height");
        let c = compound(5498).unwrap();
        assert_eq!((c.horizontal, c.vertical), (4269, 5703));
        assert!(get(4979).unwrap().three_d && get(4979).unwrap().geog == 4326);
        assert!(get(9000).unwrap().dynamic);
        assert_eq!(
            grid_alternatives("us_noaa_conus.tif"),
            vec!["conus".to_string()]
        );
        assert_eq!(datum_date(6759), Some("2007-01-01"));
        assert_eq!(datum_date(6326), None);
    }

    #[test]
    fn esri_table_loads() {
        let e = esri(102003).unwrap();
        assert_eq!(e.name, "USA_Contiguous_Albers_Equal_Area_Conic");
        assert_eq!((e.datum, e.geog), (6269, 4269));
        assert!(e.proj.starts_with("+proj=aea") && !e.ops.is_empty());
        assert!(esri(4326).is_none() && get(54009).is_none());
        assert_eq!(esri_named("World Mollweide"), vec![54009]);
    }
}
