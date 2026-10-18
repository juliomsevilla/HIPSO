//! The processing tools of the app: one window per tool with its parameters, output paths that
//! are worked out from the input and the settings (never carried over from an earlier run),
//! and the job each one starts.

use std::path::{Path, PathBuf};

use eframe::egui;
use rustlidar_core::buildings::BuildingParams;
use rustlidar_core::csf::CsfParams;
use rustlidar_core::export::{self, AsciiJob, Item};
use rustlidar_core::extras::{Extras, PoleParams, VegParams, WireParams};
use rustlidar_core::filter::{ClassSel, Filter, ReturnSel};
use rustlidar_core::ground::GroundParams;
use rustlidar_core::jobs::{self, ChmMethod};
use rustlidar_core::markup::Aoi;
use rustlidar_core::mcc::MccParams;
use rustlidar_core::metrics::MetricParams;
use rustlidar_core::noise::NoiseParams;
use rustlidar_core::paths;
use rustlidar_core::pipeline::{Ground, Noise, PitFree, Steps, Surface, Tiling};
use rustlidar_core::pmf::PmfParams;
use rustlidar_core::trees::{CrownParams, Segmenter, Shape, Smoothing, TopParams};
use rustlidar_core::vector::{self, Polygon, Region, Shape2};

use crate::theme::Tokens;

mod lidar_ui;
mod sheets_ui;
pub use sheets_ui::related;

pub type Task = Box<dyn FnOnce() -> rustlidar_core::Result<String> + Send>;

/// A job ready to run, and what to show when it ends.
pub struct Launch {
    pub label: String,
    pub task: Task,
    /// Files the job writes that the app should add as layers when it ends.
    pub show: Vec<PathBuf>,
    /// Everything the job creates: removed if the job is stopped.
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
}

impl ToolId {
    pub fn title(self) -> &'static str {
        match self {
            ToolId::Classify => "Clasificar puntos (ruido, suelo, edificios)",
            ToolId::Dtm => "Modelo digital del terreno (MDT)",
            ToolId::Chm => "Modelo de altura de copas (CHM)",
            ToolId::Metrics => "Métricas de rodal",
            ToolId::Trees => "Segmentación de árboles",
            ToolId::Clip => "Parcelas: recorte y métricas",
            ToolId::Export => "Exportar puntos (ASCII / shapefile)",
            ToolId::Crs => "Asignar sistema de coordenadas",
            ToolId::Thin => "Adelgazar nube (decimar)",
            ToolId::Tls => "Cubicar árboles (TLS)",
            ToolId::Dups => "Quitar puntos duplicados",
            ToolId::Compare => "Comparar nubes (cambios)",
            ToolId::Fit => "Ajustar una nube a otra",
            ToolId::Swath => "Separación entre pasadas",
        }
    }
}

/// An output path: worked out from the input and settings until the user edits it.
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

/// An input that is found for the cloud (a DTM or CHM of that same cloud) until picked by hand.
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
    Classes,
}

#[derive(Clone, Copy, PartialEq)]
enum NoiseChoice {
    None,
    Isolated,
    Sor,
}

#[allow(dead_code)]
pub struct Tools {
    pub s2: sheets_ui::State,
    pub lx: lidar_ui::State,
    pub open: Option<ToolId>,
    /// Run the open tool as soon as a cloud is loaded (command-line `--run`).
    pub autorun: bool,
    confirm: Option<ToolId>,
    // classification
    ground: GroundChoice,
    gp: GroundParams,
    mcc: MccParams,
    csf: CsfParams,
    pmf: PmfParams,
    noise: NoiseChoice,
    np: NoiseParams,
    sor_k: usize,
    sor_m: f64,
    buildings: bool,
    ex_veg: bool,
    ex_wire: bool,
    ex_pole: bool,
    vp: VegParams,
    wp: WireParams,
    pp: PoleParams,
    bp: BuildingParams,
    tiling: Tiling,
    // rasters
    dtm_cell: f64,
    tin: bool,
    chm_cell: f64,
    pitfree: bool,
    max_edge: f64,
    fill: bool,
    // metrics
    mp: MetricParams,
    intensity: bool,
    strata: bool,
    /// Result formats of the tables (metrics, trees, plot metrics).
    fmt_vector: bool,
    fmt_csv: bool,
    // trees
    tops: TopParams,
    crown: CrownParams,
    smooth: f64,
    smoothing: Smoothing,
    manual_tops: String,
    crowns_tif: bool,
    crowns_shp: bool,
    tops_shp: bool,
    cloud_ids: bool,
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
    ex_shp: bool,
    // paths
    out_classify: OutPath,
    out_dtm: OutPath,
    out_chm: OutPath,
    out_metrics: OutPath,
    out_trees: OutPath,
    out_clip: OutPath,
    out_export: OutPath,
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
            noise: NoiseChoice::None,
            np: NoiseParams::default(),
            sor_k: 10,
            sor_m: 3.0,
            buildings: false,
            ex_veg: false,
            ex_wire: false,
            ex_pole: false,
            vp: VegParams::default(),
            wp: WireParams::default(),
            pp: PoleParams::default(),
            bp: BuildingParams::default(),
            tiling: Tiling::default(),
            dtm_cell: 1.0,
            tin: false,
            chm_cell: 0.5,
            pitfree: false,
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
            tops_shp: true,
            cloud_ids: false,
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
            clip_normalise: false,
            exclusive: false,
            keep_empty: false,
            clip_metrics: false,
            items: vec![Item::X, Item::Y, Item::Z],
            delimiter: 0,
            header: true,
            decimals: 2,
            decimate: 1,
            ex_class: String::new(),
            ex_shp: false,
            out_classify: OutPath::default(),
            out_dtm: OutPath::default(),
            out_chm: OutPath::default(),
            out_metrics: OutPath::default(),
            out_trees: OutPath::default(),
            out_clip: OutPath::default(),
            out_export: OutPath::default(),
            in_dtm_chm: FoundPath::default(),
            in_dtm_metrics: FoundPath::default(),
            in_dtm_clip: FoundPath::default(),
            in_chm: FoundPath::default(),
        }
    }
}

fn num(
    ui: &mut egui::Ui,
    label: &str,
    v: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(v).range(range).speed(speed));
    });
}

fn pick_file(name: &str, exts: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new().add_filter(name, exts).pick_file()
}

fn save_as(ext: &str, name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
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
    /// The line that says the next job works on the selected points only.
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

    /// The choice between a vector layer and a text table (very large runs: table only).
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

    fn steps(&self) -> Steps {
        Steps {
            noise: match self.noise {
                NoiseChoice::None => None,
                NoiseChoice::Isolated => Some(Noise::Isolated(self.np.clone())),
                NoiseChoice::Sor => Some(Noise::Sor {
                    k: self.sor_k,
                    m: self.sor_m,
                }),
            },
            ground: match self.ground {
                GroundChoice::Fusion => Ground::Fusion(self.gp.clone()),
                GroundChoice::Mcc => Ground::Mcc(self.mcc.clone()),
                GroundChoice::Csf => Ground::Csf(self.csf.clone()),
                GroundChoice::Pmf => Ground::Pmf(self.pmf.clone()),
                GroundChoice::Classes => Ground::Classes,
            },
            buildings: self.buildings.then(|| self.bp.clone()),
            extras: self.extras(),
        }
    }

    pub fn extras(&self) -> Extras {
        Extras {
            vegetation: self.ex_veg.then(|| self.vp.clone()),
            wires: self.ex_wire.then(|| self.wp.clone()),
            poles: self.ex_pole.then(|| self.pp.clone()),
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
            GroundChoice::Classes => "_clase2",
        }
    }

    /// Output paths and found inputs, worked out from the first cloud and the settings now.
    fn sync(&mut self, input: &Path) {
        let ext = input
            .extension()
            .map_or("las".into(), |e| e.to_string_lossy().into_owned());
        let g = self.ground_tag();
        let noise = match self.noise {
            NoiseChoice::None => "",
            NoiseChoice::Isolated => "_aislados",
            NoiseChoice::Sor => "_sor",
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
        let tin = if self.tin { "_tin" } else { "" };
        self.out_dtm.auto(paths::product(
            input,
            &format!("mdt_{}m{tin}{g}", paths::num(self.dtm_cell)),
            "tif",
        ));
        let pf = if self.pitfree { "_pitfree" } else { "" };
        self.out_chm.auto(paths::product(
            input,
            &format!("chm_{}m{pf}", paths::num(self.chm_cell)),
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
        // beside the CHM it comes from; without one, where this cloud's products go
        let dir = if self.in_chm.text.trim().is_empty() {
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
        let seg = match self.crown.method {
            Segmenter::Dalponte => "dalponte",
            Segmenter::Silva => "silva",
            Segmenter::Watershed => "watershed",
        };
        self.out_trees
            .auto(dir.join(format!("{}_arboles_{seg}.shp", paths::stem(&base))));
        let tag = ["recorte", "fuera", "parcela"][match self.clip_mode {
            1 => 1,
            2 => 2,
            _ => 0,
        }];
        self.out_clip.auto(paths::product(input, tag, &ext));
        self.out_export.auto(paths::product(
            input,
            "export",
            if self.ex_shp { "shp" } else { "csv" },
        ));
    }

    /// Draws the open tool's window; returns a job when the user runs it.
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
        if let Some(f) = &first {
            self.sync(f);
        }
        let mut open = true;
        let mut launch = None;
        let mut run = false;
        egui::Window::new(tool.title())
            .open(&mut open)
            .default_width(520.0)
            .default_pos([320.0, 120.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy && first.is_some(), egui::Button::new("▶ Ejecutar"))
                        .clicked()
                        || (self.autorun && !busy && first.is_some())
                    {
                        run = true;
                        self.autorun = false;
                    }
                    if busy {
                        ui.spinner();
                        if ui.button("■ Detener y borrar lo parcial").clicked() {
                            self.s2.stop = true;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() - 300.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(match &first {
                                Some(f) if inputs.len() > 1 => {
                                    format!("Entrada: {} (+{} más)", f.display(), inputs.len() - 1)
                                }
                                Some(f) => format!("Entrada: {}", f.display()),
                                None => "Abre primero una nube LAS/LAZ.".to_string(),
                            })
                            .color(t.weak),
                        );
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
                            ToolId::Compare => self.compare_form(ui, t),
                            ToolId::Fit => self.fit_form(ui, t),
                            ToolId::Swath => self.swath_form(ui, t),
                            ToolId::Trees => self.trees_form(ui, t),
                            ToolId::Clip => self.plots_form(ui, t, aois),
                            ToolId::Export => self.export_form(ui),
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
            // the window stays open so the user can adjust and run again with other settings
        }
        launch
    }

    fn steps_form(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Suelo")
            .default_open(true)
            .show(ui, |ui| {
                let names = [
                    (GroundChoice::Fusion, "Kraus y Pfeifer (iterativo)"),
                    (GroundChoice::Mcc, "MCC (curvatura multiescala)"),
                    (GroundChoice::Csf, "CSF (simulación de tela)"),
                    (GroundChoice::Pmf, "PMF (morfológico progresivo)"),
                    (GroundChoice::Classes, "Clase 2 existente"),
                ];
                egui::ComboBox::from_label("Método")
                    .selected_text(
                        names
                            .iter()
                            .find(|n| n.0 == self.ground)
                            .map_or("", |n| n.1),
                    )
                    .show_ui(ui, |ui| {
                        for (c, n) in names {
                            ui.selectable_value(&mut self.ground, c, n);
                        }
                    });
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
                    GroundChoice::Classes => {}
                }
            });
        egui::CollapsingHeader::new("Ruido").show(ui, |ui| {
            ui.radio_value(&mut self.noise, NoiseChoice::None, "Sin filtro");
            ui.radio_value(
                &mut self.noise,
                NoiseChoice::Isolated,
                "Puntos aislados (celdas 3×3×3)",
            );
            ui.radio_value(&mut self.noise, NoiseChoice::Sor, "Estadístico (SOR)");
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
                NoiseChoice::None => {}
            }
        });
        egui::CollapsingHeader::new("Edificios").show(ui, |ui| {
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
        egui::CollapsingHeader::new("Vegetación, cables y postes").show(ui, |ui| {
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
                    "Radio del entorno (m)",
                    &mut self.wp.radius,
                    0.3..=10.0,
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
                "Postes y torres (15): líneas finas casi verticales",
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
                    "Radio del entorno (m)",
                    &mut self.pp.radius,
                    0.3..=10.0,
                    0.1,
                );
                num(
                    ui,
                    "Anchura máxima (m)",
                    &mut self.pp.max_width,
                    0.05..=3.0,
                    0.05,
                );
            }
        });
        egui::CollapsingHeader::new("Teselas y rendimiento").show(ui, |ui| {
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
        });
    }

    fn trees_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        self.in_chm.row(ui, t, "CHM", "CHM", || {
            pick_file("Ráster", &["tif", "tiff", "asc"])
        });
        self.out_trees.row(ui, "Árboles (base del nombre)", || {
            save_as("shp", "arboles.shp")
        });
        self.formats_row(ui);
        egui::CollapsingHeader::new("1 · Suavizado del CHM")
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
        egui::CollapsingHeader::new("2 · Ápices de los árboles (máximo local)")
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
        egui::CollapsingHeader::new("3 · Copas")
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
        egui::CollapsingHeader::new("4 · Resultados")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(
                    &mut self.crowns_tif,
                    "Ráster de copas (identificador de árbol)",
                );
                ui.checkbox(&mut self.crowns_shp, "Polígonos de copa (shapefile)");
                ui.checkbox(&mut self.tops_shp, "Ápices (shapefile PointZ)");
                ui.checkbox(
                    &mut self.cloud_ids,
                    "Nube LAS/LAZ con atributo treeID por punto",
                );
            });
        egui::CollapsingHeader::new("5 · Comprobar con inventario de campo").show(ui, |ui| {
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
                    "Columnas: X_arbol, Y_arbol, Ht, Parcela, X_Parcela, Y_Parcela",
                )
                .color(t.weak)
                .size(11.0),
            );
            num(
                ui,
                "Radio de parcela (m)",
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
        ui.checkbox(&mut self.ex_shp, "Shapefile 3D (PointZ) en lugar de texto");
        self.out_export.row(ui, "Salida", || {
            save_as(if self.ex_shp { "shp" } else { "csv" }, "puntos")
        });
        if !self.ex_shp {
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

    /// Builds the job of `tool`; checks the outputs first (an automatic path that exists gets a
    /// free name, a hand-made one asks before it is overwritten).
    fn build(
        &mut self,
        tool: ToolId,
        inputs: &[PathBuf],
        aois: &[Aoi],
        log: &mut Vec<String>,
        confirmed: bool,
    ) -> Option<Launch> {
        let input = inputs.first()?.clone();
        // the tools that work sheet by sheet, and the plots, build their own jobs
        match tool {
            ToolId::Classify | ToolId::Dtm | ToolId::Chm | ToolId::Metrics | ToolId::Crs => {
                return self.build_sheets(tool, inputs, log);
            }
            ToolId::Clip => return self.build_clip(inputs, aois, log),
            ToolId::Thin
            | ToolId::Tls
            | ToolId::Dups
            | ToolId::Compare
            | ToolId::Fit
            | ToolId::Swath => {
                return self.build_lidar(tool, inputs, log);
            }
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
            | ToolId::Compare
            | ToolId::Fit
            | ToolId::Swath
            | ToolId::Clip => return None,
            ToolId::Trees => {
                let chm = self.in_chm.path();
                if !chm.is_file() {
                    log.push(
                        "Falta el CHM: genéralo antes con la herramienta CHM o elige uno.".into(),
                    );
                    return None;
                }
                let sib = |suffix: &str, ext: &str| {
                    paths::unique(
                        &output.with_file_name(format!("{}_{suffix}.{ext}", paths::stem(&output))),
                    )
                };
                let (crowns, crowns_shp, tops_shp) = (
                    self.crowns_tif.then(|| sib("copas", "tif")),
                    self.crowns_shp.then(|| sib("copas", "shp")),
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
                let mut show: Vec<PathBuf> = [&crowns, &crowns_shp, &tops_shp]
                    .iter()
                    .filter_map(|p| (*p).clone())
                    .collect();
                show.push(output.clone());
                let job = jobs::TreesJob {
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
                        columns: rustlidar_core::forest::FieldColumns::default(),
                        plot_x: "X_Parcela".into(),
                        plot_y: "Y_Parcela".into(),
                        plot_radius: self.field_radius,
                        max_dist: self.match_dist,
                    }),
                };
                (Box::new(move || jobs::trees(&job)), show)
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
                if self.ex_shp {
                    (
                        Box::new(move || {
                            let mut n = 0;
                            for (k, i) in inputs.iter().enumerate() {
                                let o = if inputs.len() > 1 {
                                    output.with_file_name(format!(
                                        "{}_{}.shp",
                                        paths::stem(&output),
                                        k + 1
                                    ))
                                } else {
                                    output.clone()
                                };
                                n += export::shapefile_points(i, &o, &filter)?;
                            }
                            Ok(format!("{n} puntos exportados a {}", output.display()))
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
                    };
                    (
                        Box::new(move || {
                            let (files, n) = export::ascii(&job)?;
                            Ok(format!(
                                "{n} puntos exportados a {}",
                                files
                                    .first()
                                    .map_or(String::new(), |f| f.display().to_string())
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
