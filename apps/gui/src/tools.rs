//! Las herramientas de proceso de la aplicación: una ventana por herramienta con sus parámetros,
//! rutas de salida que se calculan a partir de la entrada y los ajustes (nunca heredadas de una
//! ejecución anterior) y el trabajo que lanza cada una.

use std::path::{Path, PathBuf};

use eframe::egui;
use hipso_core::axelsson::AxelssonParams;
use hipso_core::buildings::BuildingParams;
use hipso_core::csf::CsfParams;
use hipso_core::export::{self, AsciiJob, Item};
use hipso_core::extras::{Extras, PoleParams, RoadParams, VegParams, WaterParams, WireParams};
use hipso_core::filter::{ClassSel, Filter, ReturnSel};
use hipso_core::ground::GroundParams;
use hipso_core::jobs::{self, ChmMethod};
use hipso_core::markup::Aoi;
use hipso_core::mcc::MccParams;
use hipso_core::metrics::MetricParams;
use hipso_core::noise::NoiseParams;
use hipso_core::paths;
use hipso_core::pipeline::{Ground, Noise, PitFree, Steps, Surface, Tiling};
use hipso_core::pmf::PmfParams;
use hipso_core::classes::ClassSet;
use hipso_core::recipe::{self, Loaded, Recipe};
use hipso_core::rules::ClassRule;
use hipso_core::smrf::SmrfParams;
use hipso_core::trees::{CrownParams, Segmenter, Shape, Smoothing, TopParams};
use hipso_core::vector::{self, Polygon, Region, Shape2};

use crate::theme::Tokens;

mod lidar_ui;
mod sheets_ui;
pub use sheets_ui::related;

pub type Task = Box<dyn FnOnce() -> hipso_core::Result<String> + Send>;

/// Un trabajo listo para ejecutar y qué mostrar cuando termina.
pub struct Launch {
    pub label: String,
    pub task: Task,
    /// Archivos que escribe el trabajo y que la aplicación debe añadir como capas cuando termina.
    pub show: Vec<PathBuf>,
    /// Todo lo que crea el trabajo: se borra si el trabajo se detiene.
    pub outputs: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolId {
    Classify,
    Dtm,
    Chm,
    Metrics,
    Trees,
    Clip,
    Export,
    Crs,
    Thin,
    Tls,
    Dups,
    Compare,
    Fit,
    Swath,
    /// Pasadas a partir de los saltos del tiempo GPS.
    FlightLines,
    /// Densidad de puntos por celdas de Voronoi.
    VDensity,
    /// Puntos por metro cuadrado como GeoTIFF, uno por nube.
    IntensityImage,
    Volume,
    Hag,
    Colorize,
    Intensity,
    /// Recorta las nubes con una capa de polígonos.
    ClipPoly,
}

impl ToolId {
    pub fn title(self) -> &'static str {
        match self {
            ToolId::Classify => "Clasificar puntos (ruido, suelo, edificios)",
            ToolId::Dtm => "Modelo digital del terreno (MDT)",
            ToolId::Chm => "Modelo de superficie (MDS) y modelo de alturas de la vegetación (CHM)",
            ToolId::Metrics => "Métricas por celda",
            ToolId::Trees => "Segmentación de individuos",
            ToolId::Clip => "Muestras: extracción y métricas",
            ToolId::Export => "Exportar puntos (LAZ / COPC / ASCII / shapefile)",
            ToolId::Crs => "Asignar sistema de coordenadas",
            ToolId::Thin => "Adelgazar nube (decimar)",
            ToolId::Tls => "Medición de troncos",
            ToolId::Dups => "Quitar puntos duplicados",
            ToolId::Compare => "Comparar nubes (cambios)",
            ToolId::Fit => "Ajustar una nube a otra",
            ToolId::Swath => "Separación entre pasadas",
            ToolId::FlightLines => "Líneas de vuelo (por el tiempo GPS)",
            ToolId::VDensity => "Densidad de Voronoi",
            ToolId::IntensityImage => "Imagen de intensidad",
            ToolId::Volume => "Volúmenes respecto al MDT",
            ToolId::Hag => "Alturas sobre el suelo",
            ToolId::Colorize => "Colorear la nube con una imagen (RGB)",
            ToolId::Intensity => "Normalizar la intensidad por distancia",
            ToolId::ClipPoly => "Recortar nubes con una capa de polígonos",
        }
    }
}

/// Una ruta de salida: calculada a partir de la entrada y los ajustes hasta que el usuario la
/// edita.
#[derive(Default, Clone)]
pub struct OutPath {
    pub text: String,
    pub manual: bool,
}

impl OutPath {
    fn auto(&mut self, p: PathBuf) {
        if !self.manual {
            self.text = p.display().to_string();
        }
    }

    fn row(&mut self, ui: &mut egui::Ui, label: &str, pick: impl FnOnce() -> Option<PathBuf>) {
        ui.horizontal(|ui| {
            ui.label(label);
            if ui
                .add(egui::TextEdit::singleline(&mut self.text).desired_width(330.0))
                .changed()
            {
                self.manual = true;
            }
            if ui.button("…").on_hover_text("Elegir a mano").clicked()
                && let Some(p) = pick()
            {
                self.text = p.display().to_string();
                self.manual = true;
            }
            if self.manual
                && ui
                    .button("auto")
                    .on_hover_text("Volver a la ruta automática")
                    .clicked()
            {
                self.manual = false;
            }
        });
    }

    fn path(&self) -> PathBuf {
        PathBuf::from(self.text.trim())
    }
}

/// Una entrada que se encuentra para la nube (un MDT o un CHM de esa misma nube) hasta que se elige
/// a mano.
#[derive(Default, Clone)]
pub struct FoundPath {
    pub text: String,
    pub manual: bool,
}

impl FoundPath {
    fn auto(&mut self, p: Option<PathBuf>) {
        if !self.manual {
            self.text = p.map_or(String::new(), |p| p.display().to_string());
        }
    }

    fn row(
        &mut self,
        ui: &mut egui::Ui,
        t: &Tokens,
        label: &str,
        what: &str,
        pick: impl FnOnce() -> Option<PathBuf>,
    ) {
        ui.horizontal(|ui| {
            ui.label(label);
            if ui
                .add(egui::TextEdit::singleline(&mut self.text).desired_width(330.0))
                .changed()
            {
                self.manual = true;
            }
            if ui.button("…").clicked()
                && let Some(p) = pick()
            {
                self.text = p.display().to_string();
                self.manual = true;
            }
            if self.manual && ui.button("auto").clicked() {
                self.manual = false;
            }
        });
        if self.text.trim().is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(230, 160, 60),
                format!("No hay {what} de esta nube: genéralo antes o elige uno a mano."),
            );
        } else if !Path::new(self.text.trim()).exists() {
            ui.colored_label(
                egui::Color32::from_rgb(230, 90, 70),
                "El archivo no existe.",
            );
        } else if !self.manual {
            ui.label(
                egui::RichText::new(format!("{what} de esta nube (automático)"))
                    .color(t.weak)
                    .size(11.0),
            );
        }
    }

    fn path(&self) -> PathBuf {
        PathBuf::from(self.text.trim())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum GroundChoice {
    Fusion,
    Mcc,
    Csf,
    Pmf,
    Smrf,
    Axelsson,
    Classes,
}

/// Una condición sobre un valor (altura sobre el suelo, intensidad, una característica de forma)
/// tal como la muestra la ventana: activada o no, desde y hasta, cada extremo abierto si se pide
/// (sin límite inferior o sin límite superior).
#[derive(Clone, Copy)]
struct Cond {
    on: bool,
    lo: f64,
    hi: f64,
    lo_open: bool,
    hi_open: bool,
}

impl Cond {
    fn new(lo: f64, hi: f64) -> Self {
        Self {
            on: false,
            lo,
            hi,
            lo_open: false,
            hi_open: false,
        }
    }

    fn range(&self) -> Option<(f64, f64)> {
        let (lo, hi) = if self.lo_open || self.hi_open || self.lo <= self.hi {
            (self.lo, self.hi)
        } else {
            (self.hi, self.lo)
        };
        self.on.then_some((
            if self.lo_open { f64::NEG_INFINITY } else { lo },
            if self.hi_open { f64::INFINITY } else { hi },
        ))
    }
}

/// Las condiciones de forma de una regla: pendiente (°), planaridad, linealidad, esfericidad,
/// rugosidad (m) y número de vecinos, con el rango que puede tomar cada una y la unidad en que se
/// muestra.
const SHAPE: [(&str, f64, &str); 6] = [
    ("Pendiente", 90.0, "°"),
    ("Planaridad", 1.0, ""),
    ("Linealidad", 1.0, ""),
    ("Esfericidad", 1.0, ""),
    ("Rugosidad", 5.0, " m"),
    ("Vecinos", 10_000.0, ""),
];

/// Una regla tal como la muestra la ventana: clase, eco, ángulo de barrido, altura sobre el suelo,
/// intensidad y forma del entorno.
#[derive(Clone)]
struct RuleRow {
    on: bool,
    /// Clases que mira; vacío: todas menos el ruido.
    from: String,
    /// Único, primero de varios, intermedio, último de varios.
    echo: [bool; 4],
    angle_on: bool,
    /// Ángulo de barrido absoluto desde y hasta, grados.
    angle: (f64, f64),
    /// Altura sobre el suelo (m) e intensidad.
    height: Cond,
    intensity: Cond,
    /// Pendiente, planaridad, linealidad, esfericidad, rugosidad y vecinos (véase `SHAPE`).
    shape: [Cond; 6],
    /// Radio del entorno que miran las condiciones de forma (m).
    radius: f64,
    to: f64,
}

impl RuleRow {
    fn new(echo: [bool; 4], angle_on: bool, from: &str, to: f64) -> Self {
        Self {
            on: false,
            from: from.into(),
            echo,
            angle_on,
            angle: (25.0, 90.0),
            height: Cond::new(0.0, 100.0),
            intensity: Cond::new(0.0, 65535.0),
            shape: SHAPE.map(|(_, max, _)| Cond::new(0.0, max)),
            radius: 1.0,
            to,
        }
    }

    /// La fila como la lee una regla: la inversa de `rule`. Los números de retorno (1 a 9) no
    /// tienen sitio en una fila, solo los tipos de eco.
    fn from_rule(r: &ClassRule) -> Self {
        let mut default_from = ClassSet::all();
        default_from.set(7, false);
        default_from.set(18, false);
        let from = if r.from == default_from {
            String::new()
        } else {
            (0..=255u8)
                .filter(|c| r.from.contains(*c))
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let cond = |range: Option<(f64, f64)>, default: Cond| match range {
            Some((lo, hi)) => Cond {
                on: true,
                lo: if lo.is_finite() { lo } else { default.lo },
                hi: if hi.is_finite() { hi } else { default.hi },
                lo_open: !lo.is_finite(),
                hi_open: !hi.is_finite(),
            },
            None => default,
        };
        let mut row = Self::new(r.returns.kinds, r.angle.is_some(), &from, f64::from(r.to));
        row.on = true;
        row.angle = r.angle.unwrap_or(row.angle);
        row.height = cond(r.height, row.height);
        row.intensity = cond(r.intensity, row.intensity);
        let ranges = [r.slope, r.planarity, r.linearity, r.sphericity, r.roughness, r.neighbours];
        for (c, range) in row.shape.iter_mut().zip(ranges) {
            *c = cond(range, *c);
        }
        row.radius = r.radius;
        row
    }

    /// Las reglas con las que arranca la ventana, todas desactivadas.
    fn defaults() -> Vec<Self> {
        vec![
            // ángulos de barrido pronunciados: menos precisos, se dejan fuera del suelo y de las
            // superficies
            Self::new([true; 4], true, "", 12.0),
            // los primeros ecos de pulsos que dieron varios, aún sin clasificar: copa
            Self::new([false, true, false, false], false, "1", 5.0),
            Self::new([true; 4], false, "", 1.0),
        ]
    }

    fn rule(&self) -> Option<ClassRule> {
        if !self.on {
            return None;
        }
        let mut rule = ClassRule::new(self.to as u8);
        if !self.from.trim().is_empty() {
            rule.from = ClassSel::parse(&self.from).ok()?.set;
        }
        if self.echo.iter().any(|e| *e) {
            rule.returns = ReturnSel {
                kinds: self.echo,
                numbers: 0,
            };
        }
        rule.angle = self.angle_on.then_some(self.angle);
        rule.height = self.height.range();
        rule.intensity = self.intensity.range();
        let [slope, planarity, linearity, sphericity, roughness, neighbours] = self.shape;
        rule.slope = slope.range();
        rule.planarity = planarity.range();
        rule.linearity = linearity.range();
        rule.sphericity = sphericity.range();
        rule.roughness = roughness.range();
        rule.neighbours = neighbours.range();
        rule.radius = self.radius.max(0.05);
        Some(rule)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NoiseChoice {
    None,
    Isolated,
    Sor,
    Elm,
}

#[allow(dead_code)]
pub struct Tools {
    pub s2: sheets_ui::State,
    pub lx: lidar_ui::State,
    pub open: Option<ToolId>,
    /// Ejecuta la herramienta abierta en cuanto se carga una nube (línea de comandos `--run`).
    pub autorun: bool,
    confirm: Option<ToolId>,
    // classification
    ground: GroundChoice,
    gp: GroundParams,
    mcc: MccParams,
    csf: CsfParams,
    pmf: PmfParams,
    smrf: SmrfParams,
    ax: AxelssonParams,
    rule_rows: Vec<RuleRow>,
    /// Aparta el solape entre pasadas (clase 12), con esta celda (m).
    overlap: bool,
    overlap_cell: f64,
    noise: NoiseChoice,
    np: NoiseParams,
    sor_k: usize,
    sor_m: f64,
    elm_cell: f64,
    elm_drop: f64,
    buildings: bool,
    ex_veg: bool,
    ex_wire: bool,
    ex_pole: bool,
    ex_road: bool,
    ex_water: bool,
    vp: VegParams,
    wp: WireParams,
    pp: PoleParams,
    rdp: RoadParams,
    wtp: WaterParams,
    bp: BuildingParams,
    tiling: Tiling,
    /// Lo que hizo el último guardado o carga de una receta.
    recipe_note: String,
    /// La pestaña de la ventana de clasificación: pasos, reglas, teselas.
    class_tab: usize,
    // rasters
    dtm_cell: f64,
    dtm_surface: Surface,
    chm_cell: f64,
    max_edge: f64,
    fill: bool,
    // metrics
    mp: MetricParams,
    intensity: bool,
    strata: bool,
    /// Formatos de resultado de las tablas (métricas, árboles, métricas de parcelas).
    fmt_vector: bool,
    fmt_csv: bool,
    // árboles
    tops: TopParams,
    crown: CrownParams,
    smooth: f64,
    smoothing: Smoothing,
    manual_tops: String,
    crowns_tif: bool,
    crowns_shp: bool,
    /// Cimas y copas en un solo GeoPackage, y las métricas de nube sobre las copas.
    trees_gpkg: bool,
    crown_metrics: bool,
    tops_shp: bool,
    cloud_ids: bool,
    across: bool,
    across_margin: f64,
    field: String,
    field_radius: f64,
    match_dist: f64,
    // clip
    regions_file: String,
    name_field: String,
    radius: f64,
    radius_field: String,
    where_: String,
    rect: [f64; 4],
    circle: [f64; 3],
    clip_mode: usize,
    clip_class: String,
    clip_returns: String,
    clip_zmin: String,
    clip_zmax: String,
    clip_normalise: bool,
    exclusive: bool,
    keep_empty: bool,
    clip_metrics: bool,
    // export
    items: Vec<Item>,
    delimiter: usize,
    header: bool,
    decimals: usize,
    decimate: u64,
    ex_class: String,
    /// Export format: 0 LAZ, 1 LAS, 2 text, 3 3D shapefile.
    ex_fmt: usize,
    ex_norm: bool,
    ex_dtm: String,
    // paths
    /// Carpeta que eligió el usuario para todo resultado (vacío: junto a cada entrada).
    pub out_dir: String,
    out_classify: OutPath,
    out_dtm: OutPath,
    out_chm: OutPath,
    out_metrics: OutPath,
    out_trees: OutPath,
    out_clip: OutPath,
    out_export: OutPath,
    /// Las capas vectoriales que están cargadas: nombre y archivo, para las herramientas que toman
    /// polígonos.
    pub vectors: Vec<(String, PathBuf)>,
    in_dtm_chm: FoundPath,
    in_dtm_metrics: FoundPath,
    in_dtm_clip: FoundPath,
    in_chm: FoundPath,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            s2: sheets_ui::State::default(),
            lx: lidar_ui::State::default(),
            open: None,
            autorun: false,
            confirm: None,
            ground: GroundChoice::Classes,
            gp: GroundParams::default(),
            mcc: MccParams::default(),
            csf: CsfParams::default(),
            pmf: PmfParams::default(),
            smrf: SmrfParams::default(),
            ax: AxelssonParams::default(),
            rule_rows: RuleRow::defaults(),
            overlap: false,
            overlap_cell: 5.0,
            noise: NoiseChoice::None,
            np: NoiseParams::default(),
            sor_k: 10,
            sor_m: 3.0,
            elm_cell: 10.0,
            elm_drop: 3.0,
            buildings: false,
            ex_veg: false,
            ex_wire: false,
            ex_pole: false,
            ex_road: false,
            ex_water: false,
            vp: VegParams::default(),
            wp: WireParams::default(),
            pp: PoleParams::default(),
            rdp: RoadParams::default(),
            wtp: WaterParams::default(),
            bp: BuildingParams::default(),
            tiling: Tiling::default(),
            recipe_note: String::new(),
            class_tab: 0,
            dtm_cell: 1.0,
            dtm_surface: Surface::Tin,
            chm_cell: 0.5,
            max_edge: 1.0,
            fill: true,
            mp: MetricParams::default(),
            intensity: false,
            strata: false,
            fmt_vector: true,
            fmt_csv: false,
            tops: TopParams::default(),
            crown: CrownParams::default(),
            smooth: 0.0,
            smoothing: Smoothing::Mean,
            manual_tops: String::new(),
            crowns_tif: true,
            crowns_shp: true,
            trees_gpkg: false,
            crown_metrics: false,
            tops_shp: true,
            cloud_ids: false,
            across: true,
            across_margin: 30.0,
            field: String::new(),
            field_radius: 10.0,
            match_dist: 3.0,
            regions_file: String::new(),
            name_field: String::new(),
            radius: 10.0,
            radius_field: String::new(),
            where_: String::new(),
            rect: [0.0; 4],
            circle: [0.0, 0.0, 10.0],
            clip_mode: 2,
            clip_class: String::new(),
            clip_returns: String::new(),
            clip_zmin: String::new(),
            clip_zmax: String::new(),
            clip_normalise: true,
            exclusive: false,
            keep_empty: false,
            clip_metrics: false,
            items: vec![Item::X, Item::Y, Item::Z],
            delimiter: 0,
            header: true,
            decimals: 2,
            decimate: 1,
            ex_class: String::new(),
            ex_fmt: 0,
            ex_norm: false,
            ex_dtm: String::new(),
            out_dir: String::new(),
            out_classify: OutPath::default(),
            out_dtm: OutPath::default(),
            out_chm: OutPath::default(),
            out_metrics: OutPath::default(),
            out_trees: OutPath::default(),
            out_clip: OutPath::default(),
            out_export: OutPath::default(),
            vectors: Vec::new(),
            in_dtm_chm: FoundPath::default(),
            in_dtm_metrics: FoundPath::default(),
            in_dtm_clip: FoundPath::default(),
            in_chm: FoundPath::default(),
        }
    }
}

/// Los ajustes del filtro morfológico simple. Una fila de una regla: la casilla y, cuando está
/// activada, desde y hasta.
fn cond_row(
    ui: &mut egui::Ui,
    label: &str,
    c: &mut Cond,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
    suffix: &str,
) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut c.on, label);
        ui.add_enabled(
            c.on && !c.lo_open,
            crate::numfield::NumField::new(&mut c.lo)
                .range(range.clone())
                .speed(speed)
                .suffix(suffix),
        );
        ui.label("a");
        ui.add_enabled(
            c.on && !c.hi_open,
            crate::numfield::NumField::new(&mut c.hi)
                .range(range)
                .speed(speed)
                .suffix(suffix),
        );
    });
    if c.on {
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.checkbox(&mut c.lo_open, "sin mínimo");
            ui.checkbox(&mut c.hi_open, "sin máximo");
        });
    }
}

fn smrf_fields(ui: &mut egui::Ui, p: &mut SmrfParams) {
    num(ui, "Celda", &mut p.cell, 0.1..=10.0, 0.1);
    num(
        ui,
        "Ventana máxima (radio)",
        &mut p.max_window,
        2.0..=200.0,
        1.0,
    );
    num(
        ui,
        "Pendiente máxima del terreno",
        &mut p.slope,
        0.0..=2.0,
        0.01,
    );
    num(ui, "Umbral de altura", &mut p.threshold, 0.05..=5.0, 0.05);
    num(ui, "Factor por pendiente", &mut p.scalar, 0.0..=5.0, 0.05);
}

/// Los parámetros del filtro de suelo por TIN progresivo (Axelsson).
fn axelsson_fields(ui: &mut egui::Ui, p: &mut AxelssonParams) {
    num(
        ui,
        "Celda de las semillas (m): mayor que el edificio más grande",
        &mut p.seed_cell,
        1.0..=500.0,
        1.0,
    );
    num(
        ui,
        "Ángulo de iteración (°)",
        &mut p.iter_angle,
        0.5..=45.0,
        0.5,
    );
    num(
        ui,
        "Distancia de iteración (m)",
        &mut p.iter_dist,
        0.05..=10.0,
        0.05,
    );
    num(
        ui,
        "Tolerancia llana (m): cerca del plano es suelo",
        &mut p.flat_tolerance,
        0.0..=1.0,
        0.01,
    );
}

pub fn num(
    ui: &mut egui::Ui,
    label: &str,
    v: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(crate::numfield::NumField::new(v).range(range).speed(speed));
    });
}

fn pick_file(name: &str, exts: &[&str]) -> Option<PathBuf> {
    crate::prefs::dialog().add_filter(name, exts).pick_file()
}

fn save_as(ext: &str, name: &str) -> Option<PathBuf> {
    crate::prefs::dialog()
        .add_filter(ext, &[ext])
        .set_file_name(name)
        .save_file()
}

fn class_mask(text: &str) -> Result<ClassSel, String> {
    if text.trim().is_empty() {
        Ok(ClassSel::all())
    } else {
        ClassSel::parse(text).map_err(|e| e.to_string())
    }
}

fn parse_opt(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse().ok()
}

impl Tools {
    /// La línea que dice que el próximo trabajo funciona solo sobre los puntos seleccionados.
    pub fn set_selection_note(&mut self, n: Option<String>) {
        self.s2.sel_note = n;
    }

    fn formats(&self) -> paths::Formats {
        paths::Formats {
            vector: self.fmt_vector,
            csv: self.fmt_csv,
        }
        .fixed()
    }

    /// La elección entre una capa vectorial y una tabla de texto (ejecuciones muy grandes: solo
    /// tabla).
    fn formats_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Formato de salida");
            ui.checkbox(&mut self.fmt_vector, "Vectorial (shapefile)");
            ui.checkbox(&mut self.fmt_csv, "CSV")
                .on_hover_text("Más rápido y ligero en procesos muy grandes");
            if !self.fmt_vector && !self.fmt_csv {
                self.fmt_vector = true;
            }
        });
    }

    /// Pone en la ventana los pasos y las teselas de una receta.
    fn apply_recipe(&mut self, r: &Recipe) {
        let st = &r.steps;
        self.overlap = st.overlap.is_some();
        if let Some(c) = st.overlap {
            self.overlap_cell = c;
        }
        match &st.ground {
            Ground::Fusion(p) => (self.ground, self.gp) = (GroundChoice::Fusion, p.clone()),
            Ground::Mcc(p) => (self.ground, self.mcc) = (GroundChoice::Mcc, p.clone()),
            Ground::Csf(p) => (self.ground, self.csf) = (GroundChoice::Csf, p.clone()),
            Ground::Pmf(p) => (self.ground, self.pmf) = (GroundChoice::Pmf, p.clone()),
            Ground::Smrf(p) => (self.ground, self.smrf) = (GroundChoice::Smrf, p.clone()),
            Ground::Axelsson(p) => (self.ground, self.ax) = (GroundChoice::Axelsson, p.clone()),
            Ground::Classes => self.ground = GroundChoice::Classes,
        }
        match &st.noise {
            None => self.noise = NoiseChoice::None,
            Some(Noise::Isolated(p)) => (self.noise, self.np) = (NoiseChoice::Isolated, p.clone()),
            Some(Noise::Sor { k, m }) => {
                (self.noise, self.sor_k, self.sor_m) = (NoiseChoice::Sor, *k, *m);
            }
            Some(Noise::Elm { cell, threshold }) => {
                (self.noise, self.elm_cell, self.elm_drop) = (NoiseChoice::Elm, *cell, *threshold);
            }
        }
        self.buildings = st.buildings.is_some();
        if let Some(b) = &st.buildings {
            self.bp = b.clone();
        }
        self.ex_veg = st.extras.vegetation.is_some();
        self.ex_wire = st.extras.wires.is_some();
        self.ex_pole = st.extras.poles.is_some();
        self.ex_road = st.extras.roads.is_some();
        self.ex_water = st.extras.water.is_some();
        if let Some(r) = &st.extras.roads {
            self.rdp = r.clone();
        }
        if let Some(w) = &st.extras.water {
            self.wtp = w.clone();
        }
        if let Some(v) = &st.extras.vegetation {
            self.vp = v.clone();
        }
        if let Some(w) = &st.extras.wires {
            self.wp = w.clone();
        }
        if let Some(p) = &st.extras.poles {
            self.pp = p.clone();
        }
        self.rule_rows = if st.rules.is_empty() {
            RuleRow::defaults()
        } else {
            st.rules.iter().map(RuleRow::from_rule).collect()
        };
        self.tiling = r.tiling.clone();
    }

    /// Los dos botones que guardan como receta la clasificación de la ventana y cargan una.
    fn recipe_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .button("Guardar receta…")
                .on_hover_text("Guarda esta clasificación en un archivo de texto, para aplicarla otra vez aquí o con `hipso classify --recipe`")
                .clicked()
                && let Some(path) = crate::prefs::dialog()
                    .add_filter("Receta de clasificación", &["txt"])
                    .set_file_name("receta_clasificacion.txt")
                    .save_file()
            {
                let r = Recipe {
                    steps: self.steps(),
                    tiling: self.tiling.clone(),
                };
                self.recipe_note = match std::fs::write(&path, recipe::write(&r)) {
                    Ok(()) if recipe::is_exact(&r) => format!("Receta guardada en {}.", path.display()),
                    Ok(()) => format!(
                        "Receta guardada en {}. Algún parámetro avanzado de la ventana no tiene opción en la línea de comandos y no se guarda: al cargarla tendrá su valor por defecto.",
                        path.display()
                    ),
                    Err(e) => format!("No se pudo guardar la receta: {e}"),
                };
            }
            if ui
                .button("Cargar receta…")
                .on_hover_text("Pone en esta ventana la clasificación de un archivo de receta")
                .clicked()
                && let Some(path) = crate::prefs::dialog()
                    .add_filter("Receta de clasificación", &["txt"])
                    .pick_file()
            {
                self.recipe_note = match std::fs::read_to_string(&path).map_err(hipso_core::Error::Io).and_then(|t| recipe::parse(&t)) {
                    Ok(Loaded { recipe, ignored }) => {
                        self.apply_recipe(&recipe);
                        if ignored.is_empty() {
                            format!("Receta cargada de {}.", path.display())
                        } else {
                            format!(
                                "Receta cargada de {}. No se usan en la clasificación: {}.",
                                path.display(),
                                ignored.join(" ")
                            )
                        }
                    }
                    Err(e) => format!("No se pudo cargar la receta: {e}"),
                };
            }
        });
        if !self.recipe_note.is_empty() {
            ui.label(egui::RichText::new(self.recipe_note.clone()).size(11.0));
        }
    }

    fn steps(&self) -> Steps {
        Steps {
            overlap: self.overlap.then_some(self.overlap_cell),
            noise: match self.noise {
                NoiseChoice::None => None,
                NoiseChoice::Isolated => Some(Noise::Isolated(self.np.clone())),
                NoiseChoice::Sor => Some(Noise::Sor {
                    k: self.sor_k,
                    m: self.sor_m,
                }),
                NoiseChoice::Elm => Some(Noise::Elm {
                    cell: self.elm_cell,
                    threshold: self.elm_drop,
                }),
            },
            ground: match self.ground {
                GroundChoice::Fusion => Ground::Fusion(self.gp.clone()),
                GroundChoice::Mcc => Ground::Mcc(self.mcc.clone()),
                GroundChoice::Csf => Ground::Csf(self.csf.clone()),
                GroundChoice::Pmf => Ground::Pmf(self.pmf.clone()),
                GroundChoice::Smrf => Ground::Smrf(self.smrf.clone()),
                GroundChoice::Axelsson => Ground::Axelsson(self.ax.clone()),
                GroundChoice::Classes => Ground::Classes,
            },
            buildings: self.buildings.then(|| self.bp.clone()),
            extras: self.extras(),
            rules: self.rule_rows.iter().filter_map(RuleRow::rule).collect(),
        }
    }

    pub fn extras(&self) -> Extras {
        Extras {
            vegetation: self.ex_veg.then(|| self.vp.clone()),
            wires: self.ex_wire.then(|| self.wp.clone()),
            poles: self.ex_pole.then(|| self.pp.clone()),
            roads: self.ex_road.then(|| self.rdp.clone()),
            water: self.ex_water.then(|| self.wtp.clone()),
        }
    }

    /// El método en el nombre de archivo del MDT.
    fn surface_tag(&self) -> &'static str {
        match self.dtm_surface {
            Surface::Mean => "_media",
            Surface::Tin => "_tin",
            Surface::Idw => "_idw",
        }
    }

    fn tiling(&self) -> Tiling {
        let mut t = self.tiling.clone();
        if self.buildings {
            t.buffer = t.buffer.max(self.bp.window / 2.0);
        }
        t
    }

    fn ground_tag(&self) -> &'static str {
        match self.ground {
            GroundChoice::Fusion => "",
            GroundChoice::Mcc => "_mcc",
            GroundChoice::Csf => "_csf",
            GroundChoice::Pmf => "_pmf",
            GroundChoice::Smrf => "_smrf",
            GroundChoice::Axelsson => "_axelsson",
            GroundChoice::Classes => "_clase2",
        }
    }

    /// Rutas de salida y entradas halladas, calculadas a partir de la primera nube y los ajustes de
    /// ahora.
    fn sync(&mut self, input: &Path) {
        let ext = input
            .extension()
            .map_or("las".into(), |e| e.to_string_lossy().into_owned());
        let g = self.ground_tag();
        let noise = match self.noise {
            NoiseChoice::None => "",
            NoiseChoice::Isolated => "_aislados",
            NoiseChoice::Sor => "_sor",
            NoiseChoice::Elm => "_elm",
        };
        let bld = format!(
            "{}{}",
            if self.buildings { "_edif" } else { "" },
            self.extras().tag()
        );
        self.out_classify.auto(paths::product(
            input,
            &format!("clasif{g}{noise}{bld}"),
            &ext,
        ));
        let tin = self.surface_tag();
        self.out_dtm.auto(paths::product(
            input,
            &format!("mdt_{}m{tin}{g}", paths::num(self.dtm_cell)),
            "tif",
        ));
        self.out_chm.auto(paths::product(
            input,
            &format!("chm_{}m", paths::num(self.chm_cell)),
            "tif",
        ));
        self.out_metrics.auto(paths::product(
            input,
            &format!("metricas_{}m", paths::num(self.mp.cell)),
            "shp",
        ));
        let dtm = paths::find_dtm(input);
        self.in_dtm_chm.auto(dtm.clone());
        self.in_dtm_metrics.auto(dtm.clone());
        self.in_dtm_clip.auto(dtm);
        self.in_chm.auto(paths::find_chm(input));
        let chm = self.in_chm.path();
        // junto al CHM del que viene; sin uno, donde van los productos de esta nube
        let dir = if self.in_chm.text.trim().is_empty() || paths::output_dir().is_some() {
            paths::results_dir(input)
        } else {
            chm.parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        };
        let base = if self.in_chm.text.trim().is_empty() {
            input.to_path_buf()
        } else {
            chm
        };
        let seg = self.seg_name();
        self.out_trees
            .auto(dir.join(format!("{}_arboles_{seg}.shp", paths::stem(&base))));
        let tag = ["recorte", "fuera", "muestra"][match self.clip_mode {
            1 => 1,
            2 => 2,
            _ => 0,
        }];
        self.out_clip.auto(paths::product(input, tag, &ext));
        let tag = if self.ex_norm {
            "normalizada"
        } else {
            "export"
        };
        self.out_export
            .auto(paths::product(
                input,
                if self.ex_fmt == 4 { "optimizada" } else { tag },
                Self::EX_EXT[self.ex_fmt.min(4)],
            ));
    }

    const EX_EXT: [&'static str; 5] = ["laz", "las", "csv", "shp", "copc.laz"];

    fn seg_name(&self) -> &'static str {
        match self.crown.method {
            Segmenter::Dalponte => "dalponte",
            Segmenter::Silva => "silva",
            Segmenter::Watershed => "watershed",
        }
    }

    /// Dibuja la ventana de la herramienta abierta; devuelve un trabajo cuando el usuario la
    /// ejecuta.
    pub fn window(
        &mut self,
        ctx: &egui::Context,
        t: &Tokens,
        inputs: &[PathBuf],
        aois: &[Aoi],
        busy: bool,
        log: &mut Vec<String>,
    ) -> Option<Launch> {
        let tool = self.open?;
        let first = inputs.first().cloned();
        let dir = self.out_dir.trim();
        paths::set_output_dir((!dir.is_empty()).then(|| PathBuf::from(dir)));
        if let Some(f) = &first {
            self.sync(f);
        }
        let mut open = true;
        let mut launch = None;
        let mut run = false;
        // como un panel de geoprocesos: a la derecha, bajo la cinta, fuera del camino del mapa
        let right = ctx.content_rect().right();
        egui::Window::new(tool.title())
            .open(&mut open)
            .default_width(520.0)
            .default_pos([(right - 560.0).max(320.0), 190.0])
            .show(ctx, |ui| {
                if self.autorun && !busy && first.is_some() {
                    run = true;
                    self.autorun = false;
                }
                // la entrada, por su nombre: la ruta completa está en la ayuda emergente
                match &first {
                    Some(f) => {
                        let name = f
                            .file_name()
                            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                        let text = if inputs.len() > 1 {
                            format!("Entrada: {name} y {} más", inputs.len() - 1)
                        } else {
                            format!("Entrada: {name}")
                        };
                        ui.label(egui::RichText::new(text).strong())
                            .on_hover_text(f.display().to_string());
                    }
                    None => {
                        ui.label(
                            egui::RichText::new("Abre primero una nube LAS/LAZ.").color(t.weak),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Guardar en");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.out_dir)
                            .hint_text("carpeta de resultados (vacío: junto a cada nube)")
                            .desired_width(300.0),
                    );
                    if ui.button("…").clicked()
                        && let Some(d) = crate::prefs::dialog()
                            .set_title("Carpeta donde guardar los resultados")
                            .pick_folder()
                    {
                        self.out_dir = d.display().to_string();
                    }
                    if !self.out_dir.is_empty() && ui.button("auto").clicked() {
                        self.out_dir.clear();
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() - 360.0)
                    .show(ui, |ui| {
                        if let Some(n) = &self.s2.sel_note {
                            ui.colored_label(egui::Color32::from_rgb(0, 220, 220), n.clone());
                        }
                        ui.add_space(4.0);
                        match tool {
                            ToolId::Classify => self.classify_form(ui, t, inputs),
                            ToolId::Dtm => self.dtm_form(ui, t, inputs),
                            ToolId::Chm => self.chm_form(ui, t, inputs),
                            ToolId::Metrics => self.metrics_form(ui, t, inputs),
                            ToolId::Crs => self.crs_form(ui, t, inputs),
                            ToolId::Thin => self.thin_form(ui, t, inputs),
                            ToolId::Tls => self.tls_form(ui, t),
                            ToolId::Dups => self.dups_form(ui, t),
                            ToolId::Hag => self.hag_form(ui, t),
                            ToolId::Colorize => self.colorize_form(ui, t),
                            ToolId::Intensity => self.intensity_form(ui, t),
                            ToolId::ClipPoly => self.clip_poly_form(ui, t),
                            ToolId::Compare => self.compare_form(ui, t),
                            ToolId::Fit => self.fit_form(ui, t),
                            ToolId::Swath => self.swath_form(ui, t),
                            ToolId::FlightLines => self.flightlines_form(ui, t),
                            ToolId::VDensity => self.vdensity_form(ui, t),
                            ToolId::IntensityImage => self.intensity_image_form(ui, t, inputs),
                            ToolId::Volume => self.volume_form(ui, t, inputs, aois),
                            ToolId::Trees => self.trees_form(ui, t, inputs),
                            ToolId::Clip => self.plots_form(ui, t, aois),
                            ToolId::Export => self.export_form(ui),
                        }
                    });
                // el botón de ejecutar, solo al pie, como acción principal de la ventana
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let go = egui::Button::new(
                        egui::RichText::new("▶  Ejecutar")
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(t.accent)
                    .min_size(egui::vec2(120.0, 30.0));
                    if ui
                        .add_enabled(!busy && first.is_some(), go)
                        .on_disabled_hover_text(if busy {
                            "Hay un proceso en marcha"
                        } else {
                            "Abre primero una nube"
                        })
                        .clicked()
                    {
                        run = true;
                    }
                    if busy {
                        if ui.button("■ Detener y borrar lo parcial").clicked() {
                            self.s2.stop = true;
                        }
                        ui.spinner();
                    }
                });
            });
        if self.confirm == Some(tool) {
            egui::Window::new("El archivo ya existe")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("La ruta elegida a mano ya existe. ¿Sobrescribirla?");
                    ui.horizontal(|ui| {
                        if ui.button("Sobrescribir").clicked() {
                            self.confirm = None;
                            launch = self.build(tool, inputs, aois, log, true);
                        }
                        if ui.button("Cancelar").clicked() {
                            self.confirm = None;
                        }
                    });
                });
        }
        if run {
            launch = self.build(tool, inputs, aois, log, false);
        }
        if !open {
            self.open = None;
        } else if launch.is_some() {
            // la ventana se queda abierta para que el usuario pueda ajustar y ejecutar de nuevo con
            // otros ajustes
        }
        launch
    }

    fn steps_form(&mut self, ui: &mut egui::Ui) {
        self.recipe_buttons(ui);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (i, name) in ["Clasificación", "Reglas", "Teselas y rendimiento"]
                .iter()
                .enumerate()
            {
                ui.selectable_value(&mut self.class_tab, i, *name);
            }
        });
        ui.separator();
        if self.class_tab == 0 {
        crate::collapsing("Solape entre pasadas").show(ui, |ui| {
            ui.checkbox(&mut self.overlap, "Apartar el solape (clase 12)");
            ui.label(
                egui::RichText::new(
                    "Donde varias pasadas cubren una celda, la de menor ángulo de barrido medio se queda con ella y los puntos de las otras pasan a la clase 12, que los pasos siguientes no usan.",
                )
                .small()
                .weak(),
            );
            if self.overlap {
                num(ui, "Celda (m)", &mut self.overlap_cell, 1.0..=100.0, 1.0);
            }
        });
        crate::collapsing("Ruido").show(ui, |ui| {
            ui.radio_value(&mut self.noise, NoiseChoice::None, "Sin filtro");
            ui.radio_value(
                &mut self.noise,
                NoiseChoice::Isolated,
                "Puntos aislados (celdas 3×3×3)",
            );
            ui.radio_value(&mut self.noise, NoiseChoice::Sor, "Estadístico (SOR)");
            ui.radio_value(
                &mut self.noise,
                NoiseChoice::Elm,
                "Puntos bajos (ELM)",
            );
            match self.noise {
                NoiseChoice::Isolated => {
                    num(ui, "Celda xy", &mut self.np.step_xy, 0.1..=50.0, 0.1);
                    num(ui, "Celda z", &mut self.np.step_z, 0.1..=50.0, 0.1);
                }
                NoiseChoice::Sor => {
                    let mut k = self.sor_k as f64;
                    num(ui, "Vecinos (k)", &mut k, 2.0..=100.0, 1.0);
                    self.sor_k = k as usize;
                    num(ui, "Desviaciones (m)", &mut self.sor_m, 0.5..=10.0, 0.1);
                }
                NoiseChoice::Elm => {
                    num(ui, "Celda", &mut self.elm_cell, 1.0..=100.0, 0.5);
                    num(ui, "Bajada mínima (m)", &mut self.elm_drop, 0.5..=50.0, 0.1);
                    ui.label(
                        egui::RichText::new(
                            "Marca como ruido el punto más bajo de una celda que queda esa distancia por debajo del resto de su celda y de las vecinas.",
                        )
                        .small()
                        .weak(),
                    );
                }
                NoiseChoice::None => {}
            }
        });
        crate::collapsing("Suelo")
            .default_open(true)
            .show(ui, |ui| {
                let names = [
                    (GroundChoice::Fusion, "Kraus y Pfeifer (iterativo)"),
                    (GroundChoice::Mcc, "MCC (curvatura multiescala)"),
                    (GroundChoice::Csf, "CSF (simulación de tela)"),
                    (GroundChoice::Pmf, "PMF (morfológico progresivo)"),
                    (GroundChoice::Smrf, "SMRF (morfológico simple, Pingel)"),
                    (
                        GroundChoice::Axelsson,
                        "Axelsson (TIN progresivo, ángulo y distancia)",
                    ),
                    (GroundChoice::Classes, "Clase 2 existente"),
                ];
                for (c, n) in names {
                    ui.radio_value(&mut self.ground, c, n);
                }
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
                        num(ui, "g", &mut self.gp.g, -10.0..=10.0, 0.1);
                        num(ui, "w", &mut self.gp.w, 0.1..=10.0, 0.1);
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
                        let mut r = f64::from(self.csf.rigidness);
                        num(ui, "Rigidez (1 abrupto – 3 llano)", &mut r, 1.0..=3.0, 1.0);
                        self.csf.rigidness = r as u8;
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
                    GroundChoice::Smrf => smrf_fields(ui, &mut self.smrf),
                    GroundChoice::Axelsson => {
                        axelsson_fields(ui, &mut self.ax);
                    }
                    GroundChoice::Classes => {}
                }
            });
        crate::collapsing("Edificios").show(ui, |ui| {
            ui.checkbox(
                &mut self.buildings,
                "Clasificar edificios (6) y vegetación alta (5)",
            );
            if self.buildings {
                num(ui, "Altura mínima", &mut self.bp.offset, 0.5..=20.0, 0.1);
                num(
                    ui,
                    "Edificio más ancho",
                    &mut self.bp.window,
                    10.0..=500.0,
                    1.0,
                );
                num(ui, "Área mínima", &mut self.bp.min_area, 1.0..=10000.0, 1.0);
            }
        });
        crate::collapsing("Carreteras y agua").show(ui, |ui| {
            ui.checkbox(
                &mut self.ex_road,
                "Carreteras (11): suelo liso, llano y de intensidad baja",
            );
            if self.ex_road {
                num(
                    ui,
                    "Intensidad máxima (0: relativa al suelo)",
                    &mut self.rdp.max_intensity,
                    0.0..=65535.0,
                    100.0,
                );
                num(
                    ui,
                    "Rugosidad máxima (m)",
                    &mut self.rdp.max_roughness,
                    0.005..=0.2,
                    0.005,
                );
                num(
                    ui,
                    "Pendiente máxima (°)",
                    &mut self.rdp.max_slope,
                    0.5..=30.0,
                    0.5,
                );
            }
            ui.checkbox(
                &mut self.ex_water,
                "Agua (9): superficies llanas extensas de intensidad muy baja",
            );
            if self.ex_water {
                num(
                    ui,
                    "Intensidad máxima (0: relativa al suelo)",
                    &mut self.wtp.max_intensity,
                    0.0..=65535.0,
                    100.0,
                );
                num(
                    ui,
                    "Superficie mínima (m²)",
                    &mut self.wtp.min_area,
                    10.0..=100000.0,
                    10.0,
                );
                num(
                    ui,
                    "Desviación máxima de la cota (m)",
                    &mut self.wtp.max_deviation,
                    0.02..=2.0,
                    0.02,
                );
            }
        });
        crate::collapsing("Vegetación, cables y postes").show(ui, |ui| {
            ui.checkbox(
                &mut self.ex_veg,
                "Vegetación por alturas: baja (3), media (4) y alta (5)",
            );
            if self.ex_veg {
                num(ui, "Baja hasta (m)", &mut self.vp.low, 0.1..=5.0, 0.1);
                num(ui, "Media hasta (m)", &mut self.vp.medium, 0.5..=20.0, 0.1);
            }
            ui.checkbox(
                &mut self.ex_wire,
                "Cables (14): líneas casi horizontales altas",
            );
            if self.ex_wire {
                num(
                    ui,
                    "Altura mínima (m)",
                    &mut self.wp.min_height,
                    1.0..=100.0,
                    0.5,
                );
                num(
                    ui,
                    "Radio del entorno (m, 0 = según la densidad)",
                    &mut self.wp.radius,
                    0.0..=10.0,
                    0.1,
                );
                num(
                    ui,
                    "Linealidad mínima (0–1)",
                    &mut self.wp.linearity,
                    0.5..=0.99,
                    0.01,
                );
            }
            ui.checkbox(
                &mut self.ex_pole,
                "Postes y torres (15): estructuras verticales estrechas y aisladas",
            );
            if self.ex_pole {
                num(
                    ui,
                    "Altura mínima (m)",
                    &mut self.pp.min_height,
                    0.5..=100.0,
                    0.5,
                );
                num(
                    ui,
                    "Distancia de unión (m)",
                    &mut self.pp.radius,
                    0.3..=10.0,
                    0.1,
                );
                num(
                    ui,
                    "Anchura máxima (m)",
                    &mut self.pp.max_width,
                    0.1..=30.0,
                    0.5,
                );
                num(
                    ui,
                    "Altura mínima de la estructura (m)",
                    &mut self.pp.min_extent,
                    1.0..=100.0,
                    0.5,
                );
                num(
                    ui,
                    "Cabeza de apoyo: longitud máxima (m, 0 = no)",
                    &mut self.pp.head_length,
                    0.0..=50.0,
                    0.5,
                );
            }
        });
        }
        if self.class_tab == 1 {
            ui.label(egui::RichText::new("Cambian la clase de los puntos que cumplen todas las condiciones de la regla (eco, ángulo, altura sobre el suelo, intensidad y forma del entorno), al final y en orden: una regla ve lo que hizo la anterior. Para hacer estratos de vegetación, una regla por altura (por ejemplo, de la clase 1, altura de 0,5 a 2 m, a la clase 4). El ruido (7 y 18) no se toca salvo que lo pidas en «De clases».").size(11.0));
            for (k, row) in self.rule_rows.iter_mut().enumerate() {
                ui.separator();
                ui.checkbox(&mut row.on, format!("Regla {}", k + 1));
                if !row.on {
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.label("De clases");
                    ui.add(
                        egui::TextEdit::singleline(&mut row.from)
                            .hint_text("todas menos ruido; p. ej. 1,2 o ~7")
                            .desired_width(190.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Eco");
                    for (e, name) in ["Único", "Primero", "Intermedio", "Último"]
                        .iter()
                        .enumerate()
                    {
                        ui.checkbox(&mut row.echo[e], *name);
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut row.angle_on, "Ángulo |a| de");
                    ui.add_enabled(
                        row.angle_on,
                        crate::numfield::NumField::new(&mut row.angle.0)
                            .range(0.0..=90.0)
                            .speed(0.5)
                            .suffix("°"),
                    );
                    ui.label("a");
                    ui.add_enabled(
                        row.angle_on,
                        crate::numfield::NumField::new(&mut row.angle.1)
                            .range(0.0..=180.0)
                            .speed(0.5)
                            .suffix("°"),
                    );
                });
                cond_row(ui, "Altura sobre el suelo", &mut row.height, 0.0..=1000.0, 0.1, " m");
                cond_row(ui, "Intensidad", &mut row.intensity, 0.0..=65535.0, 10.0, "");
                ui.label(
                    egui::RichText::new("Forma del entorno (de los vecinos dentro del radio)")
                        .size(11.0),
                );
                num(ui, "Radio", &mut row.radius, 0.1..=20.0, 0.1);
                for ((name, max, unit), cond) in SHAPE.iter().zip(row.shape.iter_mut()) {
                    cond_row(ui, name, cond, 0.0..=*max, max / 200.0, unit);
                }
                num(ui, "A la clase", &mut row.to, 0.0..=255.0, 1.0);
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Añadir regla").clicked() {
                    let mut row = RuleRow::new([true; 4], false, "1", 4.0);
                    row.on = true;
                    self.rule_rows.push(row);
                }
                if self.rule_rows.len() > 1 && ui.button("Quitar la última").clicked() {
                    self.rule_rows.pop();
                }
            });
        }
        if self.class_tab == 2 {
            num(
                ui,
                "Tamaño de tesela",
                &mut self.tiling.size,
                50.0..=10000.0,
                10.0,
            );
            num(ui, "Búfer", &mut self.tiling.buffer, 0.0..=500.0, 1.0);
            let mut th = self.tiling.threads as f64;
            num(ui, "Hilos (0 = todos)", &mut th, 0.0..=256.0, 1.0);
            self.tiling.threads = th as usize;
        }
    }

    fn trees_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        self.in_chm.row(ui, t, "CHM", "CHM", || {
            pick_file("Ráster", hipso_core::raster::EXTENSIONS)
        });
        if inputs.len() > 1 && !self.in_chm.manual {
            ui.label(
                egui::RichText::new(format!(
                    "Hay {} nubes cargadas: se segmenta el CHM de cada una y cada hoja da su resultado. Elige un CHM a mano para segmentar solo ese.",
                    inputs.len()
                ))
                .color(t.weak)
                .size(11.0),
            );
        }
        self.out_trees.row(ui, "Individuos (base del nombre)", || {
            save_as("shp", "arboles.shp")
        });
        self.formats_row(ui);
        crate::collapsing("1 · Suavizado del CHM")
            .default_open(true)
            .show(ui, |ui| {
                num(
                    ui,
                    "Ventana (m, 0 = sin suavizar)",
                    &mut self.smooth,
                    0.0..=20.0,
                    0.1,
                );
                if self.smooth > 0.0 {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.smoothing, Smoothing::Mean, "Media");
                        ui.radio_value(&mut self.smoothing, Smoothing::Median, "Mediana");
                        ui.radio_value(&mut self.smoothing, Smoothing::Gaussian, "Gauss");
                    });
                }
            });
        crate::collapsing("2 · Ápices de los individuos (máximo local)")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Origen");
                    let manual = !self.manual_tops.is_empty();
                    if ui.selectable_label(!manual, "Buscar en el CHM").clicked() {
                        self.manual_tops.clear();
                    }
                    if ui
                        .selectable_label(manual, "Archivo de ápices (x,y)…")
                        .clicked()
                        && let Some(p) = pick_file("Ápices", &["csv", "txt"])
                    {
                        self.manual_tops = p.display().to_string();
                    }
                });
                if self.manual_tops.is_empty() {
                    num(ui, "Altura mínima", &mut self.tops.hmin, 0.0..=100.0, 0.1);
                    ui.label("Ventana (diámetro) = a + b·h + c·h²  (b = c = 0: fija)");
                    num(ui, "a", &mut self.tops.a, 0.0..=50.0, 0.1);
                    num(ui, "b", &mut self.tops.b, 0.0..=5.0, 0.01);
                    num(ui, "c", &mut self.tops.c, 0.0..=1.0, 0.001);
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.tops.shape, Shape::Circle, "Circular");
                        ui.radio_value(&mut self.tops.shape, Shape::Square, "Cuadrada");
                    });
                } else {
                    ui.label(&self.manual_tops);
                }
            });
        crate::collapsing("3 · Copas")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(
                        &mut self.crown.method,
                        Segmenter::Dalponte,
                        "Dalponte y Coomes 2016",
                    );
                    ui.radio_value(
                        &mut self.crown.method,
                        Segmenter::Silva,
                        "Silva et al. 2016",
                    );
                    ui.radio_value(
                        &mut self.crown.method,
                        Segmenter::Watershed,
                        "Cuencas (watershed)",
                    );
                });
                match self.crown.method {
                    Segmenter::Dalponte => {
                        num(
                            ui,
                            "th_seed (parte de la altura del ápice)",
                            &mut self.crown.th_seed,
                            0.0..=1.0,
                            0.01,
                        );
                        num(
                            ui,
                            "th_cr (parte de la altura media de la copa)",
                            &mut self.crown.th_cr,
                            0.0..=1.0,
                            0.01,
                        );
                        num(
                            ui,
                            "max_cr (alcance en píxeles)",
                            &mut self.crown.max_cr,
                            1.0..=100.0,
                            0.5,
                        );
                    }
                    Segmenter::Silva => {
                        num(
                            ui,
                            "max_cr_factor (radio / altura)",
                            &mut self.crown.max_cr_factor,
                            0.05..=3.0,
                            0.01,
                        );
                        num(
                            ui,
                            "exclusión (parte de la altura)",
                            &mut self.crown.exclusion,
                            0.0..=1.0,
                            0.01,
                        );
                    }
                    Segmenter::Watershed => {
                        num(
                            ui,
                            "tolerancia (m)",
                            &mut self.crown.tolerance,
                            0.0..=20.0,
                            0.1,
                        );
                        let mut e = self.crown.ext as f64;
                        num(ui, "ext (píxeles)", &mut e, 1.0..=20.0, 1.0);
                        self.crown.ext = e as usize;
                    }
                }
            });
        crate::collapsing("4 · Resultados")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(
                    &mut self.crowns_tif,
                    "Ráster de copas (identificador de individuo)",
                );
                ui.checkbox(&mut self.crowns_shp, "Polígonos de copa (shapefile)");
                ui.checkbox(&mut self.tops_shp, "Ápices (shapefile PointZ)");
                ui.checkbox(
                    &mut self.trees_gpkg,
                    "GeoPackage: ápices y copas en un solo archivo (.gpkg)",
                );
                ui.checkbox(
                    &mut self.crown_metrics,
                    "Métricas de la nube en cada copa (las de Extraer muestras; más lento, usa alturas normalizadas)",
                );
                ui.checkbox(
                    &mut self.cloud_ids,
                    "Nube LAS/LAZ con atributo treeID por punto",
                );
                ui.checkbox(
                    &mut self.across,
                    "Con varias hojas, cada árbol una sola vez: lo busca con lo que tienen las vecinas y lo deja a la hoja de su cima",
                );
                if self.across {
                    num(
                        ui,
                        "Margen hacia las vecinas (m)",
                        &mut self.across_margin,
                        1.0..=500.0,
                        1.0,
                    );
                }
            });
        crate::collapsing("5 · Comprobar con inventario de campo").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Inventario");
                ui.add(egui::TextEdit::singleline(&mut self.field).desired_width(300.0));
                if ui.button("…").clicked()
                    && let Some(p) = pick_file("Inventario", &["txt", "csv", "tsv"])
                {
                    self.field = p.display().to_string();
                }
            });
            ui.label(
                egui::RichText::new(
                    "Columnas: X_arbol, Y_arbol, Ht, Muestra, X_Muestra, Y_Muestra",
                )
                .color(t.weak)
                .size(11.0),
            );
            num(
                ui,
                "Radio de la unidad de muestreo (m)",
                &mut self.field_radius,
                1.0..=100.0,
                0.5,
            );
            num(
                ui,
                "Distancia de emparejado (m)",
                &mut self.match_dist,
                0.5..=20.0,
                0.1,
            );
        });
    }

    fn export_form(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Formato");
            for (k, n) in ["LAZ", "LAS", "Texto (ASCII)", "Shapefile 3D", "COPC"]
                .iter()
                .enumerate()
            {
                if ui.radio_value(&mut self.ex_fmt, k, *n).clicked() {
                    // un formato nuevo necesita una extensión nueva en el nombre automático
                    self.out_export.manual = false;
                }
            }
        });
        if self.ex_fmt == 4 {
            ui.label(
                egui::RichText::new("COPC (Cloud Optimized Point Cloud): un LAZ normal, que lee cualquier programa, cuyos bloques son los nodos de un octree. Así un visor o un servidor (QGIS, visores web) lee solo la zona y el detalle que necesita, y se publica en internet sin servidor especial. Se conservan todos los atributos; los formatos 0 a 3 pasan a 6 o 7, como pide el formato. La nube se guarda entera en memoria mientras se ordena (unos 40 bytes por punto): para nubes de cientos de millones de puntos, divídelas antes en teselas.")
                    .size(11.0)
                    .weak(),
            );
            self.out_export.row(ui, "Salida", || save_as("laz", "puntos"));
            return;
        }
        ui.checkbox(
            &mut self.ex_norm,
            "Normalizar: alturas sobre el suelo (cada nube menos su MDT)",
        );
        if self.ex_norm {
            Self::raster_row(ui, "MDT", &mut self.ex_dtm, hipso_core::raster::EXTENSIONS);
            ui.label(
                egui::RichText::new(
                    "Vacío: se usa el último MDT calculado de cada nube. Los puntos fuera del MDT no se exportan.",
                )
                .size(11.0)
                .weak(),
            );
        }
        self.out_export.row(ui, "Salida", || {
            save_as(Self::EX_EXT[self.ex_fmt.min(4)], "puntos")
        });
        if self.ex_fmt == 2 {
            ui.label("Columnas (en este orden):");
            ui.horizontal_wrapped(|ui| {
                for it in Item::ALL {
                    let mut on = self.items.contains(&it);
                    if ui.checkbox(&mut on, it.label()).changed() {
                        if on {
                            self.items.push(it);
                        } else {
                            self.items.retain(|i| *i != it);
                        }
                    }
                }
            });
            ui.label(format!(
                "Orden: {}",
                self.items
                    .iter()
                    .map(|i| i.label())
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
            ui.horizontal(|ui| {
                ui.label("Separador");
                for (k, n) in ["coma", "tabulador", "espacio", "punto y coma"]
                    .iter()
                    .enumerate()
                {
                    ui.radio_value(&mut self.delimiter, k, *n);
                }
            });
            ui.checkbox(&mut self.header, "Línea de cabecera");
            let mut d = self.decimals as f64;
            num(ui, "Decimales", &mut d, 0.0..=8.0, 1.0);
            self.decimals = d as usize;
        }
        let mut dec = self.decimate as f64;
        num(ui, "Un punto de cada", &mut dec, 1.0..=100000.0, 1.0);
        self.decimate = dec as u64;
        ui.horizontal(|ui| {
            ui.label("Clases");
            ui.add(
                egui::TextEdit::singleline(&mut self.ex_class)
                    .hint_text("2,5  o  ~7,18")
                    .desired_width(90.0),
            );
        });
    }

    /// Construye el trabajo de `tool`; comprueba antes las salidas (una ruta automática que existe
    /// recibe un nombre libre, una hecha a mano pregunta antes de sobrescribirse).
    fn build(
        &mut self,
        tool: ToolId,
        inputs: &[PathBuf],
        aois: &[Aoi],
        log: &mut Vec<String>,
        confirmed: bool,
    ) -> Option<Launch> {
        let input = inputs.first()?.clone();
        // las herramientas que trabajan hoja a hoja, y las parcelas, construyen sus propios
        // trabajos
        match tool {
            ToolId::Classify | ToolId::Dtm | ToolId::Chm | ToolId::Metrics | ToolId::Crs => {
                return self.build_sheets(tool, inputs, log);
            }
            ToolId::Clip => return self.build_clip(inputs, aois, log),
            ToolId::Volume => return self.build_volume(inputs, aois, log),
            ToolId::Thin
            | ToolId::Tls
            | ToolId::Dups
            | ToolId::Hag
            | ToolId::Compare
            | ToolId::Fit
            | ToolId::Swath
            | ToolId::FlightLines
            | ToolId::VDensity
            | ToolId::IntensityImage
            | ToolId::Colorize
            | ToolId::Intensity => {
                return self.build_lidar(tool, inputs, log);
            }
            ToolId::ClipPoly => return self.build_clip_poly(inputs, log),
            _ => {}
        }
        let out = match tool {
            ToolId::Classify => &mut self.out_classify,
            ToolId::Dtm => &mut self.out_dtm,
            ToolId::Chm => &mut self.out_chm,
            ToolId::Metrics => &mut self.out_metrics,
            ToolId::Trees => &mut self.out_trees,
            ToolId::Clip => &mut self.out_clip,
            ToolId::Export => &mut self.out_export,
            _ => return None,
        };
        let mut output = out.path();
        if output.as_os_str().is_empty() {
            log.push("Falta la ruta de salida.".into());
            return None;
        }
        if output.exists() {
            if !out.manual {
                output = paths::unique(&output);
                log.push(format!(
                    "Ya existía un resultado con esos ajustes: se guarda como {}",
                    output.display()
                ));
            } else if !confirmed {
                self.confirm = Some(tool);
                return None;
            }
        }
        let label = tool.title().to_string();
        let (task, show): (Task, Vec<PathBuf>) = match tool {
            ToolId::Classify
            | ToolId::Dtm
            | ToolId::Chm
            | ToolId::Metrics
            | ToolId::Crs
            | ToolId::Thin
            | ToolId::Tls
            | ToolId::Dups
            | ToolId::Hag
            | ToolId::Compare
            | ToolId::Fit
            | ToolId::Swath
            | ToolId::FlightLines
            | ToolId::VDensity
            | ToolId::IntensityImage
            | ToolId::Colorize
            | ToolId::Intensity
            | ToolId::ClipPoly
            | ToolId::Volume
            | ToolId::Clip => return None,
            ToolId::Trees => {
                // cada nube cargada es una hoja con su propio CHM; un CHM elegido a mano se usa
                // solo
                let sheets: Vec<(PathBuf, PathBuf, PathBuf)> =
                    if self.in_chm.manual || inputs.len() == 1 {
                        vec![(self.in_chm.path(), output.clone(), input.clone())]
                    } else {
                        let seg = self.seg_name();
                        let mut v = Vec::new();
                        for p in inputs {
                            let Some(chm) = paths::find_chm(p) else {
                                log.push(format!(
                                "Falta el CHM de {}: genéralo antes con la herramienta MDS / CHM.",
                                paths::stem(p)
                            ));
                                return None;
                            };
                            let dir = paths::output_dir().unwrap_or_else(|| {
                                chm.parent()
                                    .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
                            });
                            let out = paths::unique(
                                &dir.join(format!("{}_arboles_{seg}.shp", paths::stem(&chm))),
                            );
                            v.push((chm, out, p.clone()));
                        }
                        v
                    };
                let mut show: Vec<PathBuf> = Vec::new();
                let mut trees_jobs = Vec::new();
                for (chm, output, input) in sheets {
                    if !chm.is_file() {
                        log.push(
                            "Falta el CHM: genéralo antes con la herramienta MDS / CHM o elige uno."
                                .into(),
                        );
                        return None;
                    }
                    let sib = |suffix: &str, ext: &str| {
                        paths::unique(
                            &output
                                .with_file_name(format!("{}_{suffix}.{ext}", paths::stem(&output))),
                        )
                    };
                    let (crowns, crowns_shp, tops_shp) = (
                        self.crowns_tif.then(|| sib("individuos", "tif")),
                        self.crowns_shp.then(|| sib("individuos", "shp")),
                        self.tops_shp.then(|| sib("apices", "shp")),
                    );
                    let cloud = self.cloud_ids.then(|| {
                        let ext = input
                            .extension()
                            .map_or("las".into(), |e| e.to_string_lossy().into_owned());
                        (
                            input.clone(),
                            paths::unique(&paths::product(
                                &input,
                                &format!("treeid_{}", paths::stem(&output)),
                                &ext,
                            )),
                        )
                    });
                    show.extend(
                        [&crowns, &crowns_shp, &tops_shp]
                            .iter()
                            .filter_map(|p| (*p).clone()),
                    );
                    show.push(output.clone());
                    trees_jobs.push(jobs::TreesJob {
                        formats: self.formats(),
                        chm,
                        output,
                        crowns,
                        crowns_shp,
                        tops_shp,
                        cloud,
                        tops_file: (!self.manual_tops.is_empty())
                            .then(|| PathBuf::from(&self.manual_tops)),
                        tops: self.tops.clone(),
                        crown: CrownParams {
                            th_tree: self.tops.hmin,
                            ..self.crown.clone()
                        },
                        smooth: self.smooth,
                        smoothing: self.smoothing,
                        field: (!self.field.trim().is_empty()).then(|| jobs::FieldJob {
                            file: PathBuf::from(self.field.trim()),
                            columns: hipso_core::forest::FieldColumns::default(),
                            plot_x: "X_Muestra".into(),
                            plot_y: "Y_Muestra".into(),
                            plot_radius: self.field_radius,
                            max_dist: self.match_dist,
                        }),
                        around: Vec::new(),
                        margin: if self.across { self.across_margin } else { 0.0 },
                        gpkg: self.trees_gpkg,
                        crown_metrics: if self.crown_metrics { vec![input.clone()] } else { Vec::new() },
                    });
                }
                // cada hoja mira los modelos de las demás, que dicen cuáles son sus vecinas
                if self.across && trees_jobs.len() > 1 {
                    let all: Vec<PathBuf> = trees_jobs.iter().map(|j| j.chm.clone()).collect();
                    for j in &mut trees_jobs {
                        j.around = all.iter().filter(|c| **c != j.chm).cloned().collect();
                    }
                }
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for j in &trees_jobs {
                            hipso_core::check_cancel()?;
                            lines.push(jobs::trees(j)?);
                        }
                        Ok(lines.join("\n"))
                    }),
                    show,
                )
            }
            ToolId::Export => {
                let classes = match class_mask(&self.ex_class) {
                    Ok(c) => c,
                    Err(e) => {
                        log.push(format!("Clases: {e}"));
                        return None;
                    }
                };
                let filter = Filter {
                    classes,
                    returns: ReturnSel::all(),
                    zmin: None,
                    zmax: None,
                    decimate: self.decimate.max(1),
                    skip_noise: false,
                };
                let inputs = inputs.to_vec();
                let mut dtms: Vec<PathBuf> = Vec::new();
                if self.ex_norm {
                    let manual = self.ex_dtm.trim();
                    for i in &inputs {
                        let found = if manual.is_empty() {
                            paths::find_dtm(i)
                        } else {
                            Some(PathBuf::from(manual))
                        };
                        match found {
                            Some(d) => dtms.push(d),
                            None => {
                                log.push(format!(
                                    "Exportar: falta el MDT de {}; calcúlalo antes o elígelo a mano.",
                                    paths::stem(i)
                                ));
                                return None;
                            }
                        }
                    }
                }
                if self.ex_fmt == 4 {
                    let inputs = inputs.clone();
                    (
                        Box::new(move || {
                            let mut lines = Vec::new();
                            for (k, i) in inputs.iter().enumerate() {
                                hipso_core::check_cancel()?;
                                let o = if inputs.len() > 1 {
                                    output.with_file_name(format!(
                                        "{}_{}.copc.laz",
                                        paths::stem(&output)
                                            .strip_suffix(".copc")
                                            .unwrap_or(&paths::stem(&output)),
                                        k + 1
                                    ))
                                } else {
                                    output.clone()
                                };
                                let format = hipso_core::las::Reader::open(i)?.header.format;
                                let r = hipso_core::copc_write::write(i, &o)?;
                                lines.push(hipso_core::copc_write::describe(&r, format, &o));
                            }
                            Ok(lines.join("\n"))
                        }),
                        vec![],
                    )
                } else if self.ex_fmt != 2 {
                    let ext = Self::EX_EXT[self.ex_fmt.min(3)];
                    let to_cloud = self.ex_fmt < 2;
                    (
                        Box::new(move || {
                            let mut total = export::Exported::default();
                            export::begin_for(&inputs);
                            for (k, i) in inputs.iter().enumerate() {
                                let o = if inputs.len() > 1 {
                                    output.with_file_name(format!(
                                        "{}_{}.{ext}",
                                        paths::stem(&output),
                                        k + 1
                                    ))
                                } else {
                                    output.clone()
                                };
                                let d = dtms.get(k).map(PathBuf::as_path);
                                total.add(&if to_cloud {
                                    export::cloud(i, &o, &filter, d)?
                                } else {
                                    export::shapefile_points(i, &o, &filter, d)?
                                });
                            }
                            Ok(total.text(&output.display().to_string()))
                        }),
                        vec![],
                    )
                } else {
                    let delimiter = ['\u{2c}', '\t', ' ', ';'][self.delimiter.min(3)];
                    let job = AsciiJob {
                        inputs,
                        output,
                        items: self.items.clone(),
                        delimiter,
                        header: self.header,
                        filter,
                        decimals: self.decimals,
                        dtms,
                    };
                    (
                        Box::new(move || {
                            let (files, n) = export::ascii(&job)?;
                            Ok(n.text(
                                &files
                                    .first()
                                    .map_or(String::new(), |f| f.display().to_string()),
                            ))
                        }),
                        vec![],
                    )
                }
            }
        };
        Some(Launch {
            label,
            outputs: show.clone(),
            task,
            show,
        })
    }
}

/// La fila «Guardar en» de cada ventana de herramienta: la carpeta adonde van los resultados
/// (vacío: junto a cada entrada). Es la misma elección para todas las herramientas, aplicada de una
/// vez.
pub fn output_row(ui: &mut egui::Ui, out_dir: &mut String) {
    ui.horizontal(|ui| {
        ui.label("Guardar en");
        ui.add(
            egui::TextEdit::singleline(out_dir)
                .hint_text("carpeta de resultados (vacío: junto al original)")
                .desired_width(300.0),
        );
        if ui.button("…").clicked()
            && let Some(d) = crate::prefs::dialog()
                .set_title("Carpeta donde guardar los resultados")
                .pick_folder()
        {
            *out_dir = d.display().to_string();
        }
        if !out_dir.is_empty() && ui.button("auto").clicked() {
            out_dir.clear();
        }
    });
    let dir = out_dir.trim();
    hipso_core::paths::set_output_dir((!dir.is_empty()).then(|| std::path::PathBuf::from(dir)));
}

#[cfg(test)]
mod recipe_tests {
    use super::*;

    /// Una regla cargada de una receta en una fila de la ventana devuelve la misma regla, y una
    /// receta aplicada a la ventana y escrita de nuevo es la misma receta.
    #[test]
    fn a_recipe_goes_through_the_window_unchanged() -> hipso_core::Result<()> {
        for text in [
            "to=5",
            "from=1,2;returns=F;angle=25..90;to=12",
            "from=1;height=..0.5;to=3",
            "from=1;height=0.5..2;intensity=10..200;to=4",
            "height=2..;slope=60..;planarity=0.7..;roughness=..0.2;neighbours=5..;radius=2.5;to=6",
        ] {
            let rule = ClassRule::parse(text)?;
            assert_eq!(RuleRow::from_rule(&rule).rule(), Some(rule), "{text}");
        }
        let text = "--noise elm --elm-cell 12 --ground axelsson --axelsson-angle 10 --buildings --building-window 40 --vegetation --wires \
                    --rule from=1;height=..0.5;to=3 --rule angle=25;to=12 --tile 200 --buffer 40 --threads 4";
        let loaded = recipe::parse(text)?;
        let mut tools = Tools::default();
        tools.apply_recipe(&loaded.recipe);
        let back = Recipe {
            steps: tools.steps(),
            tiling: tools.tiling.clone(),
        };
        assert_eq!(back, loaded.recipe);
        Ok(())
    }

    /// La receta de ejemplo del manual se carga en la ventana sin perder nada y sale igual.
    #[test]
    fn the_example_recipe_loads_in_the_window() -> hipso_core::Result<()> {
        let text = include_str!("../../../docs/recetas/clasificacion_pnoa.txt");
        let loaded = recipe::parse(text)?;
        assert!(loaded.ignored.is_empty(), "{:?}", loaded.ignored);
        assert!(recipe::is_exact(&loaded.recipe));
        let mut tools = Tools::default();
        tools.apply_recipe(&loaded.recipe);
        let back = Recipe {
            steps: tools.steps(),
            tiling: tools.tiling.clone(),
        };
        assert_eq!(back, loaded.recipe);
        assert_eq!(back.steps.rules.len(), 4);
        assert!(back.steps.extras.roads.is_some());
        assert!(back.steps.overlap.is_some() && back.steps.extras.wires.is_some());
        Ok(())
    }

    /// La receta de línea eléctrica del manual se carga en la ventana sin perder nada.
    #[test]
    fn the_power_line_recipe_loads_in_the_window() -> hipso_core::Result<()> {
        let text = include_str!("../../../docs/recetas/clasificacion_lineas_electricas.txt");
        let loaded = recipe::parse(text)?;
        assert!(loaded.ignored.is_empty(), "{:?}", loaded.ignored);
        assert!(recipe::is_exact(&loaded.recipe));
        let mut tools = Tools::default();
        tools.apply_recipe(&loaded.recipe);
        let back = Recipe {
            steps: tools.steps(),
            tiling: tools.tiling.clone(),
        };
        assert_eq!(back, loaded.recipe);
        assert!(back.steps.extras.wires.is_some() && back.steps.extras.poles.is_some());
        assert!(back.steps.overlap.is_none() && back.steps.extras.roads.is_none());
        Ok(())
    }
}
