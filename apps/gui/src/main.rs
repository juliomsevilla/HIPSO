//! EasyLiDAR's desktop app: a point-cloud viewer (plan, 3D and profile windows, shaded TIN,
//! contours, reference layers, POI/AOI, query, measure, density) with the processing tools of the
//! command line one click away, with a ribbon interface.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod aba_plots;
mod aba_ui;
mod align_ui;
mod basemap;
mod change_ui;
mod classtable;
mod contents;
mod copcdetail;
mod corridor_ui;
mod editing;
mod histo;
mod icons;
mod layer_tabs;
mod layers;
mod manual;
mod numfield;
mod obia_ui;
mod prefs;
mod profile_data;
mod project;
mod ribbon;
mod rs;
mod rstools;
mod symbology;
mod terrain;
mod theme;
mod tips;
mod tools;
mod vecedit;
mod vecraster;
mod warp;

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Vec2};
use rustlidar_core::classes::ClassSet;
use rustlidar_core::grid::Grid;
use rustlidar_core::inspect;
use rustlidar_core::jobs;
use rustlidar_core::markup::Aoi;
use rustlidar_core::view::{self, ColorBy, Image, Orbit, Plan, PointStore, ProfileBox, Style};

use copcdetail::contains;
use theme::Tokens;
use tools::{Launch, ToolId, Tools};

const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon-256.png");

/// The decoded RGBA pixels of the logo, with its size.
fn logo_pixels() -> Option<(Vec<u8>, [usize; 2])> {
    let mut r = png::Decoder::new(std::io::Cursor::new(ICON_PNG))
        .read_info()
        .ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    Some((buf, [info.width as usize, info.height as usize]))
}

fn app_icon() -> egui::IconData {
    logo_pixels().map_or_else(Default::default, |(rgba, [width, height])| egui::IconData {
        rgba,
        width: width as u32,
        height: height as u32,
    })
}

/// The graphics adapters wgpu sees. A "Cpu" one is the software adapter (Windows' Basic Render
/// Driver, llvmpipe): it draws, but with the processor, and slowly.
fn gpu_adapters() -> Vec<eframe::egui_wgpu::wgpu::AdapterInfo> {
    use eframe::egui_wgpu::wgpu;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
        .iter()
        .map(wgpu::Adapter::get_info)
        .collect()
}

/// Which renderer draws the interface: wgpu (the default) when a graphics card can be used
/// through DirectX 12, Vulkan or Metal; otherwise OpenGL (glow). Old drivers (an Intel HD of
/// 2015) have no DirectX 12 or Vulkan, so wgpu offers only the software adapter there, which is
/// slow, and its OpenGL backend does not start on them; the OpenGL of glow does.
/// `EASYLIDAR_RENDERER=glow` or `wgpu` forces one, and a `WGPU_BACKEND` is left to wgpu.
fn choose_renderer() -> eframe::Renderer {
    match std::env::var("EASYLIDAR_RENDERER").as_deref() {
        Ok("glow") => return eframe::Renderer::Glow,
        Ok("wgpu") => return eframe::Renderer::Wgpu,
        _ => {}
    }
    if std::env::var_os("WGPU_BACKEND").is_some() {
        return eframe::Renderer::Wgpu;
    }
    use eframe::egui_wgpu::wgpu::{Backend, DeviceType};
    let hardware = gpu_adapters()
        .iter()
        .any(|a| a.device_type != DeviceType::Cpu && a.backend != Backend::Gl);
    if hardware {
        eframe::Renderer::Wgpu
    } else {
        eframe::Renderer::Glow
    }
}

fn renderer_name(r: eframe::Renderer) -> &'static str {
    match r {
        eframe::Renderer::Glow => "OpenGL (glow)",
        eframe::Renderer::Wgpu => "wgpu",
    }
}

/// The renderer in use, for the diagnostics.
static RENDERER: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--gpu-info") {
        for i in gpu_adapters() {
            eprintln!(
                "{} · {:?} · {:?} · driver {} {}",
                i.name, i.backend, i.device_type, i.driver, i.driver_info
            );
        }
        eprintln!("se usaría: {}", renderer_name(choose_renderer()));
        return Ok(());
    }
    // Without a console (windows_subsystem) a panic at start-up would close the app in silence.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        if std::thread::current().name() == Some("main") {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("EasyLiDAR")
                .set_description(format!(
                    "EasyLiDAR se ha cerrado por un error.

{info}

Si es de gráficos, prueba                      a definir la variable de entorno EASYLIDAR_RENDERER con glow o wgpu."
                ))
                .show();
        }
    }));
    let first = choose_renderer();
    match run(first) {
        // the one chosen did not start: the other
        Err(_) => run(match first {
            eframe::Renderer::Glow => eframe::Renderer::Wgpu,
            eframe::Renderer::Wgpu => eframe::Renderer::Glow,
        }),
        r => r,
    }
}

fn run(renderer: eframe::Renderer) -> eframe::Result {
    let _ = RENDERER.set(renderer_name(renderer));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(app_icon())
            .with_title("EasyLiDAR"),
        renderer,
        ..Default::default()
    };
    eframe::run_native(
        "EasyLiDAR",
        options,
        Box::new(|cc| {
            theme::install_fonts(&cc.egui_ctx);
            let mut app = App::default();
            app.startup(&cc.egui_ctx, std::env::args().skip(1).collect());
            theme::apply(&cc.egui_ctx, &app.tokens());
            Ok(Box::new(app))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Nav {
    Zoom,
    Pan,
    Query,
    Measure,
    Density,
    Profile,
    Digitize,
    VecSelect,
    VecVertex,
    VecMove,
    SelRect,
    SelPoly,
    SelLasso,
    SelCircle,
}

impl Nav {
    fn name(self) -> &'static str {
        match self {
            Nav::Zoom => "Zoom",
            Nav::Pan => "Desplazar",
            Nav::Query => "Consultar punto",
            Nav::Measure => "Medir",
            Nav::Density => "Densidad",
            Nav::Profile => "Perfil",
            Nav::Digitize => "Digitalizar",
            Nav::VecSelect => "Seleccionar elementos",
            Nav::VecVertex => "Editar vértices",
            Nav::VecMove => "Mover elementos",
            Nav::SelRect => "Seleccionar con rectángulo",
            Nav::SelPoly => "Seleccionar con polígono",
            Nav::SelLasso => "Seleccionar con lazo",
            Nav::SelCircle => "Seleccionar con círculo",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Plan,
    PlanProfile,
    View3d,
    All,
}

/// The ribbon tabs before the ones that depend on the selected layer: Inicio, Vista, Procesar
/// and Ayuda. Editing points lives in the side panel, so it stays within reach of the others.
const FIRST_EXTRA: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Layers,
    Info,
    Edit,
}

/// The background read of a window: what it will send, and the window.
type DetailJob = (Receiver<Result<Option<PointStore>, String>>, [f64; 4], f64);

/// A window of COPC clouds read at full detail, and the dot spacing that goes with it.
struct Detail {
    window: [f64; 4],
    /// Map units per pixel it was read for.
    px: f64,
    store: Arc<PointStore>,
    spacing: f64,
    id: u64,
}

/// A cloud load that finished: the points, the files they came from (converted copies replace
/// the clouds that were in another coordinate system) and what the log should say.
struct CloudLoad {
    store: PointStore,
    files: Vec<PathBuf>,
    notes: Vec<String>,
}

enum RefKind {
    Raster {
        grid: Grid,
        tex: egui::TextureHandle,
        style: rs::GridStyle,
    },
    Vector(Box<layers::VecLayer>),
    Image(Box<rs::ImageLayer>),
}

struct RefLayer {
    opacity: f32,
    name: String,
    path: PathBuf,
    kind: RefKind,
    visible: bool,
}

struct Tin {
    grid: Grid,
    tex: egui::TextureHandle,
    contours: Vec<[f64; 5]>,
    key: u64,
    contour_key: u64,
}

struct Running {
    outputs: Vec<PathBuf>,
    pre: std::collections::HashSet<PathBuf>,
    rx: Receiver<Result<String, String>>,
    started: Instant,
    label: String,
    show: Vec<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Open,
    CloseAll,
    SaveShot,
    ZoomFull,
    ZoomPrev,
    ZoomSel,
    SaveProject,
    OpenProject,
    Relink3d,
    Nav(Nav),
    Tool(ToolId),
    Shortcuts,
    Info,
    Color(ColorBy),
    PointSize(i32),
    ProfileSize(i32),
    ToggleTin,
    ToggleContours,
    ToggleLog,
    ToggleBasemap,
    LoadRaster,
    LoadVector,
    NewLayer,
    SaveLayer,
    LayerTable,
    ImageTool(u8),
    RasterTool(u8),
    Terrain,
    Obia,
    Align,
    Change,
    Model,
    Corridor,
    ClearMarks,
    ToggleTheme,
    NorthUp,
    Exaggeration(i32),
    Layout(u8),
    Classes,
    About,
    Manual,
}

struct App {
    ed: editing::Editing,
    ident: Vec<String>,
    plan_px: (usize, usize),
    snap_plan: Option<(f64, f64, f64)>,
    snap_orbit: Option<(f64, f64, f64)>,
    dark: bool,
    /// The system's theme was not known at start-up: take it from the first frame.
    theme_pending: bool,
    /// `--bench`: frames drawn, for measuring the renderer (see `ui`).
    bench: Option<(Instant, u64)>,
    tab: usize,
    nav: Nav,
    layout: Layout,
    side: Side,
    show_log: bool,
    /// A message that stays on screen until the user closes it.
    notice: Option<String>,
    clouds: Vec<PathBuf>,
    store: Option<Arc<PointStore>>,
    rev: u64,
    loading: Option<(Receiver<Result<CloudLoad, String>>, Instant)>,
    max_points_m: f64,
    class_counts: [u64; 256],
    style: Style,
    colors_rev: u64,
    plan: Plan,
    plan_fit: bool,
    prev_plans: Vec<Plan>,
    orbit: Orbit,
    follow_plan: bool,
    profile: Option<ProfileBox>,
    profile_clicks: Vec<[f64; 2]>,
    profile_width: f64,
    tex_plan: Option<(egui::TextureHandle, u64)>,
    tex_prof: Option<(egui::TextureHandle, u64, f64, f64)>,
    /// The profile reads the points of its region from the files, at full density.
    prof_full: bool,
    prof_data: Option<profile_data::ProfData>,
    prof_job: Option<profile_data::ProfJob>,
    prof_failed: Option<u64>,
    /// Where the profile was drawn in the last frame.
    profile_rect: Option<Rect>,
    tex_3d: Option<(egui::TextureHandle, u64)>,
    moving_until: Instant,
    refs: Vec<RefLayer>,
    /// How many of the layers are drawn under the point clouds (the first ones in `refs`).
    cloud_slot: usize,
    show_points: bool,
    tin_on: bool,
    tin_classes: ClassSet,
    tin: Option<Tin>,
    contours_on: bool,
    contour_iv: f64,
    /// The vector layer new features are drawn into.
    active: Option<usize>,
    sel_layer: Option<usize>,
    img_jobs: Vec<rs::ImageJob>,
    dig_draft: Vec<[f64; 2]>,
    new_layer: Option<layers::NewLayerDlg>,
    /// A GeoPackage with several tables: which of them to open.
    gpkg_pick: Option<(PathBuf, Vec<(String, bool)>)>,
    table_layer: Option<usize>,
    vedit: vecedit::VecEditState,
    /// Command line `--vsel-all`: select every feature of the active layer on start.
    select_all_pending: bool,
    /// Command line `--view x,y,scale`: the view to take once everything has loaded.
    forced_view: Option<Plan>,
    rs: rs::RsState,
    table_new_field: String,
    line_weight: f32,
    measure: Vec<[f64; 3]>,
    query: Option<usize>,
    density: Option<String>,
    drag_start: Option<Pos2>,
    cursor_map: Option<(f64, f64)>,
    info_text: String,
    info_run: Option<Receiver<Result<String, String>>>,
    /// The job running extracts the selection: its result replaces the loaded clouds.
    pub isolate: bool,
    tools: Tools,
    running: Option<Running>,
    log: Vec<String>,
    palette: Option<String>,
    show_classes: bool,
    ctable: classtable::ClassTable,
    rt: rstools::RasterTools,
    tt: terrain::TerrainTool,
    ot: obia_ui::ObiaTool,
    at: align_ui::AlignTool,
    ct: change_ui::ChangeTool,
    mo: aba_ui::ModelTool,
    co: corridor_ui::CorridorTool,
    /// The coordinate system each file declares, read once.
    crs_cache: std::collections::HashMap<PathBuf, Option<u16>>,
    /// The map of web tiles under the layers (made when it is first turned on).
    basemap: Option<basemap::Basemap>,
    /// Layers opened before any cloud: they are moved to the cloud's system once there is one.
    pending_conform: Vec<PathBuf>,
    /// The system of the work while there is no cloud: that of the first layer opened.
    work_crs: Option<u16>,
    /// The COPC file to read a window from for each cloud (the cloud itself, or its copy in the
    /// cache); empty until every cloud has one, and then a window can be read at full detail.
    detail_src: Vec<PathBuf>,
    /// The background making of the COPC copies.
    cache_job: Option<Receiver<Result<Vec<PathBuf>, String>>>,
    /// The points of the window being looked at, read from the COPC files.
    detail: Option<Detail>,
    detail_job: Option<DetailJob>,
    /// The same for the 3D view.
    detail3: Option<Detail>,
    detail3_job: Option<DetailJob>,
    detail_id: u64,
    show_about: bool,
    show_shortcuts: bool,
    /// When the log last said how far the running job is.
    last_progress_log: Instant,
    logo: Option<egui::TextureHandle>,
    manual: manual::Manual,
}

impl Default for App {
    fn default() -> Self {
        Self {
            ed: editing::Editing::default(),
            ident: Vec::new(),
            plan_px: (800, 600),
            snap_plan: None,
            snap_orbit: None,
            dark: true,
            theme_pending: true,
            bench: None,
            tab: 0,
            nav: Nav::Zoom,
            layout: Layout::Plan,
            side: Side::Layers,
            show_log: false,
            notice: None,
            clouds: Vec::new(),
            store: None,
            rev: 0,
            loading: None,
            max_points_m: 8.0,
            class_counts: [0; 256],
            style: Style::default(),
            colors_rev: 0,
            plan: Plan {
                cx: 0.0,
                cy: 0.0,
                scale: 1.0,
            },
            plan_fit: true,
            prev_plans: Vec::new(),
            orbit: Orbit {
                target: [0.0; 3],
                yaw: 0.0,
                pitch: 0.6,
                dist: 100.0,
                exaggeration: 1.0,
            },
            follow_plan: false,
            profile: None,
            profile_clicks: Vec::new(),
            profile_width: 10.0,
            tex_plan: None,
            tex_prof: None,
            prof_full: true,
            prof_data: None,
            prof_job: None,
            prof_failed: None,
            profile_rect: None,
            tex_3d: None,
            moving_until: Instant::now(),
            refs: Vec::new(),
            cloud_slot: 0,
            show_points: true,
            tin_on: false,
            tin_classes: ClassSet::from_bits(1 << 2),
            tin: None,
            contours_on: false,
            contour_iv: 5.0,
            active: None,
            sel_layer: None,
            img_jobs: Vec::new(),
            dig_draft: Vec::new(),
            new_layer: None,
            gpkg_pick: None,
            table_layer: None,
            vedit: vecedit::VecEditState::default(),
            select_all_pending: false,
            forced_view: None,
            rs: rs::RsState::default(),
            table_new_field: String::new(),
            line_weight: 2.0,
            measure: Vec::new(),
            query: None,
            density: None,
            drag_start: None,
            cursor_map: None,
            info_text: String::new(),
            info_run: None,
            isolate: false,
            tools: Tools::default(),
            running: None,
            log: vec!["EasyLiDAR listo. Abre una nube LAS/LAZ (Inicio → Abrir).".into()],
            palette: None,
            show_classes: false,
            ctable: classtable::ClassTable::load(),
            rt: rstools::RasterTools::default(),
            tt: terrain::TerrainTool::default(),
            ot: obia_ui::ObiaTool::default(),
            at: align_ui::AlignTool::default(),
            ct: change_ui::ChangeTool::default(),
            mo: aba_ui::ModelTool::default(),
            co: corridor_ui::CorridorTool::default(),
            crs_cache: std::collections::HashMap::new(),
            basemap: None,
            pending_conform: Vec::new(),
            work_crs: None,
            detail_src: Vec::new(),
            cache_job: None,
            detail: None,
            detail_job: None,
            detail3: None,
            detail3_job: None,
            detail_id: 0,
            show_about: false,
            show_shortcuts: false,
            last_progress_log: Instant::now(),
            logo: None,
            manual: manual::Manual::default(),
        }
    }
}

/// Area and perimeter of the measured points taken as a closed polygon (three points or more).
fn polygon_text(pts: &[[f64; 3]]) -> String {
    if pts.len() < 3 {
        return String::new();
    }
    let n = pts.len();
    let (mut a2, mut per) = (0.0, 0.0);
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        a2 += p[0] * q[1] - q[0] * p[1];
        per += (q[0] - p[0]).hypot(q[1] - p[1]);
    }
    let a = a2.abs() / 2.0;
    format!(
        "   |   área {a:.1} m² ({:.4} ha)   perímetro {per:.1} m",
        a / 10_000.0
    )
}

fn hash_of(v: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn color32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn to_texture(
    ctx: &egui::Context,
    name: &str,
    img: &Image,
    filter: egui::TextureOptions,
) -> egui::TextureHandle {
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied([img.w.max(1), img.h.max(1)], &img.rgba),
        filter,
    )
}

fn is_cloud(p: &Path) -> bool {
    p.extension().is_some_and(|e| {
        let e = e.to_string_lossy().to_ascii_lowercase();
        matches!(e.as_str(), "las" | "laz" | "xyz" | "txt")
    })
}

impl App {
    /// Dark or light: the interface and the background of the views.
    fn set_theme(&mut self, ctx: &egui::Context, dark: bool) {
        self.dark = dark;
        theme::apply(ctx, &self.tokens());
        self.style.background = if dark { [18, 21, 26] } else { [221, 225, 231] };
        self.colors_rev += 1;
    }

    fn tokens(&self) -> Tokens {
        if self.dark { theme::DARK } else { theme::LIGHT }
    }

    /// The EPSG code a file declares (read once; `None` when it declares none or is not a file).
    fn crs_of(&mut self, path: &Path) -> Option<u16> {
        if path.as_os_str().is_empty() {
            return None;
        }
        *self
            .crs_cache
            .entry(path.to_path_buf())
            .or_insert_with(|| rustlidar_core::crs::epsg_of(path))
    }

    /// The system of the work: that of the first cloud.
    fn project_crs(&mut self) -> Option<u16> {
        match self.clouds.first().cloned() {
            Some(first) => self.crs_of(&first),
            None => self.work_crs,
        }
    }

    fn log(&mut self, m: impl Into<String>) {
        self.log.push(m.into());
    }

    /// Command-line arguments: files to open, and a few switches for screenshots.
    fn startup(&mut self, ctx: &egui::Context, args: Vec<String>) {
        self.ctable.paint(&mut self.style.class_colors);
        if let Some((rgba, size)) = logo_pixels() {
            self.logo = Some(ctx.load_texture(
                "logo",
                egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                egui::TextureOptions::LINEAR,
            ));
        }
        // the theme the system uses; when it is not known yet, the first frame sets it
        if let Some(theme) = ctx.system_theme() {
            self.dark = theme != egui::Theme::Light;
            self.theme_pending = false;
        }
        let mut it = args.into_iter();
        let mut files = Vec::new();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--tab" => self.tab = it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                "--layout" => {
                    self.layout = match it.next().as_deref() {
                        Some("plan-profile") => Layout::PlanProfile,
                        Some("3d" | "plan-3d") => Layout::View3d,
                        Some("all") => Layout::All,
                        _ => Layout::Plan,
                    }
                }
                "--color" => {
                    self.style.color_by = match it.next().as_deref() {
                        Some("class") => ColorBy::Class,
                        Some("intensity") => ColorBy::Intensity,
                        Some("earth") => ColorBy::ElevationEarth,
                        Some("return") => ColorBy::Return,
                        _ => ColorBy::Elevation,
                    }
                }
                "--tool" => {
                    self.tools.open = match it.next().as_deref() {
                        Some("classify") => Some(ToolId::Classify),
                        Some("dtm") => Some(ToolId::Dtm),
                        Some("chm") => Some(ToolId::Chm),
                        Some("metrics") => Some(ToolId::Metrics),
                        Some("trees") => Some(ToolId::Trees),
                        Some("clip") => Some(ToolId::Clip),
                        Some("export") => Some(ToolId::Export),
                        _ => None,
                    }
                }
                "--orbit" => {
                    // yaw and pitch of the 3D view, in degrees
                    let v: Vec<f64> = it
                        .next()
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    if let [yaw, pitch] = v[..] {
                        self.orbit.yaw = yaw.to_radians();
                        self.orbit.pitch = pitch.to_radians();
                    }
                }
                "--hide-points" => self.show_points = false,
                "--nav" => {
                    self.nav = match it.next().as_deref() {
                        Some("digitize") => Nav::Digitize,
                        Some("vsel") => Nav::VecSelect,
                        Some("vvertex") => Nav::VecVertex,
                        Some("vmove") => Nav::VecMove,
                        Some("measure") => Nav::Measure,
                        Some("pan") => Nav::Pan,
                        _ => Nav::Zoom,
                    }
                }
                "--vsel-all" => self.select_all_pending = true,
                "--view" => {
                    let v: Vec<f64> = it
                        .next()
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    if v.len() == 3 {
                        self.forced_view = Some(Plan {
                            cx: v[0],
                            cy: v[1],
                            scale: v[2],
                        });
                    }
                }
                "--rs" => self.rs.open = true,
                "--histogram" => self.ed.hist.open = true,
                "--bench" => self.bench = Some((Instant::now(), 0)),
                "--table" => self.table_layer = Some(0),
                "--ndvi" => self.rs.compute_at_start = true,
                "--newlayer" => self.new_layer = Some(layers::NewLayerDlg::default()),
                "--tin" => self.tin_on = true,
                "--basemap" => {
                    let k = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    let bm = self
                        .basemap
                        .get_or_insert_with(|| basemap::Basemap::new(ctx));
                    bm.on = true;
                    bm.provider = k;
                }
                "--run" => self.tools.autorun = true,
                "--select" => {
                    let v: Vec<f64> = it
                        .next()
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    if v.len() == 4 {
                        let op = rustlidar_core::edit::SelOp {
                            mode: rustlidar_core::edit::Mode::New,
                            region: rustlidar_core::edit::Region::Plan(
                                rustlidar_core::edit::rectangle([v[0], v[1]], [v[2], v[3]]),
                            ),
                            attr: rustlidar_core::edit::Attr::default(),
                        };
                        self.queue(editing::Cmd::AddOp(Box::new(op)));
                    }
                }
                "--delete" => self.queue(editing::Cmd::Delete),
                "--reclass" => {
                    let c = it.next().and_then(|v| v.parse().ok()).unwrap_or(7);
                    self.queue(editing::Cmd::Reclass(c));
                }
                "--save-edited" => {
                    self.ed.auto_save = true;
                    self.ed.want_save = true;
                }
                "--profile" => {
                    let v: Vec<f64> = it
                        .next()
                        .unwrap_or_default()
                        .split(',')
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    if v.len() == 5 {
                        self.profile = Some(ProfileBox {
                            a: [v[0], v[1]],
                            b: [v[2], v[3]],
                            width: v[4],
                        });
                        self.profile_width = v[4];
                    }
                }
                "--contours" => self.contours_on = true,
                "--light" => {
                    self.dark = false;
                    self.theme_pending = false;
                }
                "--side-info" => self.side = Side::Info,
                "--side-edit" => self.side = Side::Edit,
                _ => files.push(PathBuf::from(a)),
            }
        }
        self.style.background = if self.dark {
            [18, 21, 26]
        } else {
            [221, 225, 231]
        };
        prefs::opened(&files);
        for f in files {
            self.open_path(ctx, f, true);
        }
        if std::mem::take(&mut self.select_all_pending)
            && let Some(v) = self.active_vec_mut()
        {
            v.selected = (0..v.data.features.len()).collect();
        }
        if self.rs.compute_at_start {
            self.rs.open = true;
            self.rs.layer = self
                .refs
                .iter()
                .position(|r| matches!(r.kind, RefKind::Image(_)))
                .unwrap_or(0);
            self.compute_index(ctx);
        }
    }

    fn open_path(&mut self, ctx: &egui::Context, p: PathBuf, replace: bool) {
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            project::EXT => self.open_project(ctx, &p),
            e if rustlidar_core::raster::is_raster_ext(e) => {
                if !self.add_raster(ctx, &p) {
                    self.log(format!("No se pudo abrir {}", p.display()));
                }
            }
            "shp" | "geojson" | "json" | "gpkg" => self.add_vector(&p),
            "e57" => {
                if self.running.is_some() {
                    self.log("Hay otro proceso en marcha: espera a que termine para abrir el E57.");
                } else {
                    // an E57 is converted to LAZ (in the results folder) and that file is opened
                    let out = rustlidar_core::paths::unique(&rustlidar_core::paths::product(
                        &p, "e57", "laz",
                    ));
                    let (src, dst) = (p.clone(), out.clone());
                    let task: tools::Task = Box::new(move || {
                        let r = rustlidar_core::e57::convert(&src, &dst, None)?;
                        Ok(rustlidar_core::e57::describe(&r, &dst))
                    });
                    self.log("El E57 no suele decir su sistema de coordenadas: asígnalo después con «Asignar proyección» si lo necesitas.");
                    self.start(
                        ctx,
                        Launch {
                            label: "Convertir E57 a LAZ".to_string(),
                            task,
                            show: vec![out.clone()],
                            outputs: vec![out],
                        },
                    );
                }
            }
            "ply" => {
                if self.running.is_some() {
                    self.log("Hay otro proceso en marcha: espera a que termine para abrir el PLY.");
                } else {
                    // a PLY is converted to LAZ (in the results folder) and that file is opened
                    let out = rustlidar_core::paths::unique(&rustlidar_core::paths::product(
                        &p, "ply", "laz",
                    ));
                    let (src, dst) = (p.clone(), out.clone());
                    let task: tools::Task = Box::new(move || {
                        let r = rustlidar_core::ply::convert(&src, &dst, None)?;
                        Ok(rustlidar_core::ply::describe(&r, &dst))
                    });
                    self.log("El PLY no dice su sistema de coordenadas: asígnalo después con «Asignar proyección» si lo necesitas.");
                    self.start(
                        ctx,
                        Launch {
                            label: "Convertir PLY a LAZ".to_string(),
                            task,
                            show: vec![out.clone()],
                            outputs: vec![out],
                        },
                    );
                }
            }
            _ if is_cloud(&p) => {
                if replace && self.loading.is_none() && !self.clouds.contains(&p) {
                    // the first file of a batch replaces what was open
                }
                if !self.clouds.contains(&p) {
                    self.clouds.push(p);
                }
                self.reload(ctx);
            }
            _ => self.log(format!("Formato no admitido: {}", p.display())),
        }
    }

    /// The groups of the Procesar tab: what each button shows, whether its window is open and
    /// what it does. Large buttons first, then columns of up to three small ones.
    fn process_groups(&self) -> Vec<ribbon::Group<Action>> {
        use ribbon::{Group, Item};
        let tool = |icon: &'static str, label: &'static str, id: ToolId| Item {
            icon,
            label,
            on: self.tools.open == Some(id),
            action: Action::Tool(id),
        };
        let other = |icon: &'static str, label: &'static str, on: bool, action: Action| Item {
            icon,
            label,
            on,
            action,
        };
        vec![
            Group {
                caption: "Preparar",
                icon: "@classify",
                big: vec![tool("@classify", "Clasificar", ToolId::Classify)],
                menus: vec![],
                columns: vec![
                    vec![
                        other("ℹ", "Información", false, Action::Info),
                        tool("@crs", "Asignar proyección", ToolId::Crs),
                        tool("@export", "Exportar puntos", ToolId::Export),
                    ],
                    vec![
                        tool("@thin", "Adelgazar nube", ToolId::Thin),
                        tool("@hag", "Alturas sobre suelo", ToolId::Hag),
                        tool("@dups", "Quitar duplicados", ToolId::Dups),
                    ],
                    vec![
                        tool("@colorize", "Colorear con imagen", ToolId::Colorize),
                        tool("@intensity", "Normalizar intensidad", ToolId::Intensity),
                        tool("@copc", "Guardar como COPC", ToolId::Copc),
                    ],
                ],
            },
            Group {
                caption: "Calidad",
                icon: "@compare",
                big: vec![],
                menus: vec![],
                columns: vec![vec![
                    tool("@compare", "Comparar nubes", ToolId::Compare),
                    tool("@fit", "Ajustar nubes", ToolId::Fit),
                    tool("@swath", "Entre pasadas", ToolId::Swath),
                ]],
            },
            Group {
                caption: "Superficies",
                icon: "@dtm",
                big: vec![
                    tool("@dtm", "MDT", ToolId::Dtm),
                    tool("@chm", "MDS / CHM", ToolId::Chm),
                ],
                menus: vec![],
                columns: vec![vec![
                    other(
                        "@terrain",
                        "Análisis de terreno",
                        self.tt.open,
                        Action::Terrain,
                    ),
                    tool("@volume", "Volumen sobre MDT", ToolId::Volume),
                ]],
            },
            Group {
                caption: "ALS",
                icon: "@metrics",
                big: vec![],
                menus: vec![ribbon::Menu {
                    icon: "@metrics",
                    label: "Métodos\nde masa",
                    items: vec![
                        tool("@metrics", "Métricas de rodal", ToolId::Metrics),
                        tool("@clip", "Recortar parcelas", ToolId::Clip),
                        other(
                            "@model",
                            "Modelos de transición",
                            self.mo.open,
                            Action::Model,
                        ),
                    ],
                }],
                columns: vec![vec![
                    tool("@segment", "Segmentar árboles", ToolId::Trees),
                    other("@change", "Cambio forestal", self.ct.open, Action::Change),
                    other("@obia", "Análisis por objetos", self.ot.open, Action::Obia),
                ]],
            },
            Group {
                caption: "TLS",
                icon: "@tree",
                big: vec![tool("@tree", "Dendrometría", ToolId::Tls)],
                menus: vec![],
                columns: vec![],
            },
            Group {
                caption: "Infraestructuras",
                icon: "@corridor",
                big: vec![other(
                    "@corridor",
                    "Corredores\neléctricos",
                    self.co.open,
                    Action::Corridor,
                )],
                menus: vec![],
                columns: vec![],
            },
        ]
    }

    /// Opens what was picked or dropped: the clouds are added together (one load), the rest one by
    /// one as layers.
    fn open_files(&mut self, ctx: &egui::Context, files: Vec<PathBuf>) {
        prefs::opened(&files);
        let mut clouds = false;
        for f in files {
            if f.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(project::EXT))
            {
                self.open_project(ctx, &f);
            } else if is_cloud(&f) {
                if !self.clouds.contains(&f) {
                    self.clouds.push(f);
                }
                clouds = true;
            } else {
                self.open_path(ctx, f, false);
            }
        }
        if clouds {
            self.reload(ctx);
        }
    }

    fn reload(&mut self, ctx: &egui::Context) {
        let files = self.clouds.clone();
        rustlidar_core::progress::reset();
        self.last_progress_log = Instant::now();
        let max = (self.max_points_m * 1e6) as usize;
        // clouds in another system than the first one are converted while they load
        let (target, other) = rustlidar_core::reproject::clouds_in_other_system(&files);
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                let moved = rustlidar_core::reproject::project_clouds(&files);
                if moved.files.is_empty() {
                    return Err(rustlidar_core::Error::Param(moved.notes.join("\n")));
                }
                // a cloud that has its COPC copy in the cache is read from it: the sample comes from
                // the first levels of the octree, without decompressing the whole file
                let read: Vec<PathBuf> = moved
                    .files
                    .iter()
                    .map(|p| rustlidar_core::copc_cache::find(p).unwrap_or_else(|| p.clone()))
                    .collect();
                view::load_from(&moved.files, &read, max).map(|store| CloudLoad {
                    store,
                    files: moved.files,
                    notes: moved.notes,
                })
            })
            .map_err(|_| "error interno al leer".to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.loading = Some((rx, Instant::now()));
        self.log(format!("Cargando {} archivo(s)…", self.clouds.len()));
        if let (Some(t), true) = (target, other > 0) {
            self.log(format!(
                "{other} nube(s) están en un sistema distinto del de la primera ({}): se pasan a él al cargar, y eso tarda más. La copia convertida queda guardada junto a la original y la próxima vez se carga sin convertir.",
                rustlidar_core::crs::name(t)
            ));
        }
        self.run_info(ctx);
    }

    fn run_info(&mut self, ctx: &egui::Context) {
        let files: Vec<PathBuf> = self
            .clouds
            .iter()
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("las") || e.eq_ignore_ascii_case("laz"))
            })
            .cloned()
            .collect();
        if files.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| jobs::info(&files))
                .map_err(|_| "error interno".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.info_run = Some(rx);
        self.info_text = "Analizando los datos…".into();
    }

    /// A grid as a raster layer, coloured between its 2nd and 98th percentile.
    fn add_grid(&mut self, ctx: &egui::Context, name: String, path: PathBuf, g: Grid) {
        let step = (g.cols.max(g.rows) / 4096).max(1);
        let shown = if step > 1 {
            let (w, h) = (g.cols / step, g.rows / step);
            let mut s = g.clone();
            s.cols = w;
            s.rows = h;
            s.cell = g.cell * step as f64;
            s.y0 = g.y0 + (g.rows - h * step) as f64 * g.cell;
            s.data = (0..w * h)
                .map(|i| g.data[(i / w) * step * g.cols + (i % w) * step])
                .collect();
            s
        } else {
            g
        };
        let style = rs::GridStyle::new(&shown);
        let tex = to_texture(
            ctx,
            &format!("ref-{}", path.display()),
            &rs::render_grid(&shown, &style),
            egui::TextureOptions::LINEAR,
        );
        self.remove_paths(&path);
        let k = self.add_ref(
            RefLayer {
                name,
                path,
                kind: RefKind::Raster {
                    grid: shown,
                    tex,
                    style,
                },
                visible: true,
                opacity: 1.0,
            },
            true,
        );
        self.sel_layer = Some(k);
        if self.store.is_none() {
            self.plan_fit = true;
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_images(ctx);
        // every half minute the log says how far the job is and how long it has left
        if self.last_progress_log.elapsed() >= Duration::from_secs(30) {
            let started = self
                .running
                .as_ref()
                .map(|r| (r.label.clone(), r.started))
                .or_else(|| {
                    self.loading
                        .as_ref()
                        .map(|(_, st)| ("Cargando".to_string(), *st))
                });
            if let Some((label, st)) = started {
                self.last_progress_log = Instant::now();
                if let Some(p) = progress_text(st) {
                    self.log(format!("{label}: {p}"));
                }
            }
        }
        if let Some((rx, started)) = &self.loading
            && let Ok(r) = rx.try_recv()
        {
            let secs = started.elapsed().as_secs_f64();
            self.loading = None;
            match r {
                Ok(CloudLoad {
                    store: s,
                    files,
                    notes,
                }) => {
                    for n in notes {
                        self.log(n);
                    }
                    self.clouds = files;
                    self.detail_src.clear();
                    self.cache_job = None;
                    self.start_cache(ctx, s.stride > 1);
                    self.detail = None;
                    self.detail_job = None;
                    self.detail3 = None;
                    self.detail3_job = None;
                    // layers opened before it follow the system of the first layer: if the cloud is in
                    // another, they are loaded again in the cloud's
                    if let Some(w) = self.work_crs.take()
                        && let Some(c) = self.project_crs()
                        && rustlidar_core::crs::relation(c, w)
                            == rustlidar_core::crs::Relation::Different
                    {
                        for q in self.refs.iter().map(|r| r.path.clone()).collect::<Vec<_>>() {
                            let gpkg = q
                                .extension()
                                .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"));
                            if !gpkg && !self.pending_conform.contains(&q) {
                                self.pending_conform.push(q);
                            }
                        }
                    }
                    // layers that were opened before the cloud are moved to its system now
                    for q in std::mem::take(&mut self.pending_conform) {
                        if self.crs_move(&q).is_some() {
                            self.log(format!(
                                "«{}» se vuelve a cargar en el sistema de la nube.",
                                q.display()
                            ));
                            self.open_path(ctx, q, false);
                        }
                    }
                    let n = s.len();
                    self.log(format!(
                        "{} puntos en pantalla de {} (1 de cada {}) en {secs:.1} s",
                        n, s.total, s.stride
                    ));
                    self.class_counts = [0; 256];
                    for c in &s.class {
                        self.class_counts[usize::from(*c)] += 1;
                    }
                    let mut st = Style::fit(&s);
                    st.color_by = if !s.rgb.iter().any(|c| *c != [0, 0, 0]) {
                        self.style.color_by
                    } else {
                        ColorBy::Rgb
                    };
                    st.point_size = self.style.point_size;
                    st.classes = self.style.classes;
                    st.returns = self.style.returns;
                    st.class_colors = self.style.class_colors;
                    st.background = self.style.background;
                    self.style = st;
                    self.plan_fit = true;
                    self.orbit = Orbit {
                        yaw: self.orbit.yaw,
                        pitch: self.orbit.pitch,
                        exaggeration: self.orbit.exaggeration,
                        ..Orbit::fit_scene(&s)
                    };
                    self.tin = None;
                    self.query = None;
                    let old = std::mem::take(&mut self.ed);
                    let hist_open = old.hist.open;
                    self.ed = editing::Editing {
                        cmds: old.cmds,
                        want_save: old.want_save,
                        auto_save: old.auto_save,
                        mode: old.mode,
                        class: old.class,
                        ..editing::Editing::default()
                    };
                    // the marks belong to the cloud they were made on; the window stays open
                    self.ed.hist.open = hist_open;
                    self.ed.session = Some(rustlidar_core::edit::Session::new(&s));
                    self.store = Some(Arc::new(s));
                    self.rev += 1;
                }
                Err(e) => self.log(format!("Error al cargar: {e}")),
            }
        }
        if let Some(rx) = &self.info_run
            && let Ok(r) = rx.try_recv()
        {
            self.info_run = None;
            self.info_text = r.unwrap_or_else(|e| format!("Error: {e}"));
        }
        if let Some(run) = &self.running
            && let Ok(result) = run.rx.try_recv()
        {
            let secs = run.started.elapsed().as_secs_f64();
            let (label, show) = (run.label.clone(), run.show.clone());
            let (outputs, pre) = (run.outputs.clone(), run.pre.clone());
            let isolate = std::mem::take(&mut self.isolate);
            self.running = None;
            match result {
                Ok(summary) => {
                    self.log(format!("{label}: {summary}\nhecho en {secs:.2} s"));
                    if isolate {
                        while !self.clouds.is_empty() {
                            self.drop_cloud(0);
                        }
                        self.style.files = u64::MAX;
                    }
                    self.load_results(ctx, show, &outputs, &pre);
                }
                Err(e) => {
                    if e.contains("cancelado") {
                        let mut removed = 0;
                        for o in &outputs {
                            for f in tools::related(o) {
                                if f.exists()
                                    && !pre.contains(&f)
                                    && std::fs::remove_file(&f).is_ok()
                                {
                                    removed += 1;
                                }
                            }
                        }
                        self.log(format!(
                            "{label}: detenido. Se borraron {removed} archivos parciales."
                        ));
                    } else {
                        self.log(format!("{label}: error: {e}"));
                    }
                }
            }
        }
    }

    /// Loads what a job made, so a result is never left on disk unseen: the files the tool asked
    /// to show and every raster, vector layer or cloud among its outputs. The first result ends
    /// up first in the layer list (drawn on top and selected); a result cloud goes first in the
    /// cloud list.
    fn load_results(
        &mut self,
        ctx: &egui::Context,
        show: Vec<PathBuf>,
        outputs: &[PathBuf],
        pre: &std::collections::HashSet<PathBuf>,
    ) {
        const MAX_CLOUDS: usize = 8;
        let ext = |p: &Path| {
            p.extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        };
        let mut list: Vec<PathBuf> = Vec::new();
        for f in show
            .iter()
            .chain(outputs.iter().filter(|o| !pre.contains(*o)))
        {
            let e = ext(f);
            // the stems and branches cloud of the TLS tool is big and made to be opened on demand
            let on_demand = f
                .file_stem()
                .is_some_and(|n| n.to_string_lossy().ends_with("_fustes"));
            let loadable = !on_demand
                && (rustlidar_core::raster::is_raster_ext(&e)
                    || matches!(e.as_str(), "shp" | "geojson" | "gpkg" | "las" | "laz"));
            if f.is_file() && loadable && !list.contains(f) {
                list.push(f.clone());
            }
        }
        let (clouds, layers): (Vec<PathBuf>, Vec<PathBuf>) =
            list.into_iter().partition(|f| is_cloud(f));
        // last loaded is on top: the first result goes last
        for f in layers.into_iter().rev() {
            self.open_path(ctx, f, false);
        }
        if clouds.is_empty() {
            return;
        }
        if clouds.len() > MAX_CLOUDS {
            let msg = format!(
                "El proceso ha dado {} nubes. Se han cargado las {MAX_CLOUDS} primeras en el visor; el resto está en disco, junto a las originales, y puedes abrirlas con Abrir o Añadir.",
                clouds.len()
            );
            self.log(msg.clone());
            self.notice = Some(msg);
        }
        for f in clouds.into_iter().take(MAX_CLOUDS).rev() {
            if self.clouds.contains(&f) {
                continue;
            }
            self.clouds.insert(0, f);
            // the visibility bits follow the list: the new first cloud is on
            self.style.files = self.style.files << 1 | 1;
        }
        self.reload(ctx);
    }

    /// Takes cloud `k` out of the loaded points without reading the others again. False when the
    /// points do not map one to one to the clouds (then the caller reloads).
    fn drop_cloud_in_memory(&mut self, ctx: &egui::Context, k: usize) -> bool {
        if self.loading.is_some() {
            return false;
        }
        let Some(s) = self
            .store
            .as_ref()
            .filter(|s| s.files.len() == self.clouds.len())
        else {
            return false;
        };
        let Some(s) = s.without_file(k) else {
            return false;
        };
        self.drop_cloud(k);
        self.class_counts = [0; 256];
        for c in &s.class {
            self.class_counts[usize::from(*c)] += 1;
        }
        self.ed.draft.clear();
        self.ed.pdraft.clear();
        self.ed.session = Some(rustlidar_core::edit::Session::new(&s));
        self.tin = None;
        self.query = None;
        self.profile = None;
        self.store = Some(Arc::new(s));
        self.rev += 1;
        self.run_info(ctx);
        true
    }

    /// Removes the cloud at `k` from the list, keeping the visibility bits of the others.
    fn drop_cloud(&mut self, k: usize) {
        self.clouds.remove(k);
        if k < 64 {
            let low = self.style.files & ((1u64 << k) - 1);
            self.style.files = low | (self.style.files >> (k + 1) << k);
        }
    }

    fn start(&mut self, ctx: &egui::Context, l: Launch) {
        rustlidar_core::reset_cancel();
        rustlidar_core::progress::reset();
        self.last_progress_log = Instant::now();
        let (tx, rx) = channel();
        let c = ctx.clone();
        let task = l.task;
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                .map_err(|_| "error interno".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.log(format!("{} …", l.label));
        let pre: std::collections::HashSet<PathBuf> = l
            .outputs
            .iter()
            .flat_map(|o| tools::related(o))
            .filter(|p| p.exists())
            .collect();
        self.running = Some(Running {
            outputs: l.outputs,
            pre,
            rx,
            started: Instant::now(),
            label: l.label,
            show: l.show,
        });
        self.show_log = true;
    }

    fn touch(&mut self) {
        self.moving_until = Instant::now() + Duration::from_millis(160);
    }

    fn zoom_to(&mut self, p: Plan) {
        self.prev_plans.push(self.plan);
        if self.prev_plans.len() > 50 {
            self.prev_plans.remove(0);
        }
        self.plan = p;
    }

    fn style_key(&self) -> u64 {
        let s = &self.style;
        hash_of((
            s.files,
            s.profile_size,
            (
                s.color_by as u8,
                s.zmin.to_bits(),
                s.zmax.to_bits(),
                s.intensity_max,
                s.point_size,
                s.classes,
                s.returns,
                (s.thin, s.transparent, s.fill),
                self.colors_rev,
                self.rev,
                self.ed.sel_rev,
            ),
        ))
    }

    fn act(&mut self, ctx: &egui::Context, a: Action) {
        match a {
            Action::Open => {
                // one door for everything: point clouds, rasters, images and vector layers; what
                // is opened is added to what is there (Cerrar todo starts again)
                let mut every = vec!["las", "laz", "ply", "e57", "xyz", "txt", project::EXT];
                every.extend(layer_extensions());
                let files = crate::prefs::dialog()
                    .add_filter("Todo lo que se puede abrir", &every)
                    .add_filter(
                        "Nubes de puntos",
                        &["las", "laz", "ply", "e57", "xyz", "txt"],
                    )
                    .add_filter("Capas (ráster y vectoriales)", &layer_extensions())
                    .pick_files()
                    .unwrap_or_default();
                self.open_files(ctx, files);
            }
            Action::CloseAll => {
                self.clouds.clear();
                self.work_crs = None;
                self.crs_cache.clear();
                // a load still running would bring the cloud back when it finishes
                self.loading = None;
                self.info_run = None;
                self.store = None;
                self.ed.session = None;
                self.ed.draft.clear();
                self.ed.pdraft.clear();
                self.query = None;
                self.class_counts = [0; 256];
                self.refs.clear();
                self.cloud_slot = 0;
                self.tin = None;
                self.profile = None;
                self.info_text.clear();
                self.rev += 1;
            }
            Action::SaveShot => {
                let Some(s) = self.store.clone() else {
                    self.log("Guardar vista: abre primero una nube de puntos; la imagen se hace con sus puntos.");
                    self.show_log = true;
                    return;
                };
                if let Some(p) = crate::prefs::dialog()
                    .add_filter("PNG", &["png"])
                    .set_file_name("vista.png")
                    .save_file()
                {
                    let img = view::render_plan(
                        &s,
                        &Plan::fit(s.bounds, 1920, 1200),
                        1920,
                        1200,
                        &self.style,
                    );
                    match img.write_png(&p) {
                        Ok(()) => self.log(format!("Imagen guardada: {}", p.display())),
                        Err(e) => self.log(format!("No se pudo guardar: {e}")),
                    }
                }
            }
            Action::ZoomFull => {
                self.prev_plans.push(self.plan);
                self.plan_fit = true;
            }
            Action::ZoomPrev => {
                if let Some(p) = self.prev_plans.pop() {
                    self.plan = p;
                }
            }
            Action::ZoomSel => self.zoom_selection(),
            Action::SaveProject => self.save_project(),
            Action::OpenProject => self.open_project_dialog(ctx),
            Action::Relink3d => self.relink_3d(),
            Action::Nav(n) => {
                if editing::is_select(n) {
                    self.side = Side::Edit;
                }
                self.nav = n;
                self.profile_clicks.clear();
                self.dig_draft.clear();
                if n == Nav::Profile && self.layout == Layout::Plan {
                    self.layout = Layout::PlanProfile;
                }
            }
            Action::Tool(t) => self.tools.open = Some(t),
            Action::Info => {
                self.side = Side::Info;
                self.run_info(ctx);
            }
            Action::Color(c) => {
                self.style.color_by = c;
                self.touch();
            }
            Action::PointSize(d) => {
                self.style.point_size = (self.style.point_size as i32 + d).clamp(1, 12) as usize
            }
            Action::ProfileSize(d) => {
                self.style.profile_size = (self.style.profile_size as i32 + d).clamp(1, 12) as usize
            }
            Action::ToggleTin => self.tin_on = !self.tin_on,
            Action::ToggleContours => self.contours_on = !self.contours_on,
            Action::ToggleLog => self.show_log = !self.show_log,
            Action::ToggleBasemap => {
                let bm = self
                    .basemap
                    .get_or_insert_with(|| basemap::Basemap::new(ctx));
                bm.on = !bm.on;
                let on = bm.on;
                if on {
                    match self.project_crs() {
                        Some(c) => self.log(format!(
                            "Mapa base encendido (se coloca en {}). Las teselas se descargan de internet y se guardan una semana en la caché del sistema.",
                            rustlidar_core::crs::name(c)
                        )),
                        None => self.log("Mapa base encendido, pero hace falta abrir una nube que declare su sistema de coordenadas para colocarlo."),
                    }
                }
            }
            Action::LoadRaster => {
                for p in crate::prefs::dialog()
                    .add_filter("Ráster", rustlidar_core::raster::EXTENSIONS)
                    .pick_files()
                    .unwrap_or_default()
                {
                    self.add_raster(ctx, &p);
                }
            }
            Action::LoadVector => {
                for p in crate::prefs::dialog()
                    .add_filter("Capa vectorial", &["shp", "geojson", "json", "gpkg"])
                    .pick_files()
                    .unwrap_or_default()
                {
                    self.add_vector(&p);
                }
            }
            Action::NewLayer => {
                self.new_layer = Some(layers::NewLayerDlg::default());
            }
            Action::SaveLayer => {
                if let Some(k) = self.active {
                    self.save_layer(k, false);
                } else {
                    self.log("No hay una capa vectorial activa que guardar.");
                }
            }
            Action::LayerTable => {
                self.table_layer = self.active.or_else(|| {
                    self.refs
                        .iter()
                        .position(|r| matches!(r.kind, RefKind::Vector(_)))
                });
                if self.table_layer.is_none() {
                    self.log("No hay capas vectoriales: carga una o crea una nueva.");
                }
            }
            Action::ImageTool(0) => {
                // the colour composition is in the contextual tab of the image
                let k = self.sel_layer.filter(|k| {
                    matches!(self.refs.get(*k).map(|r| &r.kind), Some(RefKind::Image(_)))
                });
                match k.or_else(|| {
                    self.refs
                        .iter()
                        .position(|r| matches!(r.kind, RefKind::Image(_)))
                }) {
                    Some(k) => {
                        self.sel_layer = Some(k);
                        self.tab = FIRST_EXTRA;
                    }
                    None => self.log("No hay imágenes cargadas: abre una con «Abrir»."),
                }
            }
            Action::ImageTool(_) => self.rs.open = true,
            Action::RasterTool(k) => self.rt.open = Some(k),
            Action::Terrain => self.tt.open = true,
            Action::Obia => self.ot.open = true,
            Action::Align => self.at.open = true,
            Action::Change => self.ct.open = true,
            Action::Model => self.mo.open = true,
            Action::Corridor => self.co.open = true,
            Action::ClearMarks => {
                self.measure.clear();
                self.density = None;
                self.query = None;
                self.profile = None;
                self.profile_clicks.clear();
                self.dig_draft.clear();
            }
            Action::ToggleTheme => {
                self.theme_pending = false;
                self.set_theme(ctx, !self.dark);
            }
            Action::NorthUp => self.orbit.yaw = 0.0,
            Action::Exaggeration(d) => {
                self.orbit.exaggeration =
                    (self.orbit.exaggeration + 0.5 * f64::from(d)).clamp(0.5, 10.0)
            }
            Action::Layout(k) => {
                self.layout = [
                    Layout::Plan,
                    Layout::PlanProfile,
                    Layout::View3d,
                    Layout::All,
                ][usize::from(k.min(3))];
            }
            Action::Classes => self.show_classes = !self.show_classes,
            Action::About => self.show_about = true,
            Action::Shortcuts => self.show_shortcuts = !self.show_shortcuts,
            Action::Manual => self.manual.open = true,
        }
    }

    fn commands() -> Vec<(&'static str, Action)> {
        vec![
            ("Abrir (nubes, ráster y vectores)", Action::Open),
            ("Cerrar todo", Action::CloseAll),
            ("Guardar vista como PNG", Action::SaveShot),
            ("Zoom a la extensión completa", Action::ZoomFull),
            ("Zoom anterior", Action::ZoomPrev),
            ("Zoom a la selección", Action::ZoomSel),
            ("Guardar proyecto", Action::SaveProject),
            ("Abrir proyecto", Action::OpenProject),
            (
                "Refrescar la vista 3D (vincular con la planta)",
                Action::Relink3d,
            ),
            ("Herramienta: Zoom", Action::Nav(Nav::Zoom)),
            ("Herramienta: Desplazar", Action::Nav(Nav::Pan)),
            ("Herramienta: Consultar punto", Action::Nav(Nav::Query)),
            ("Herramienta: Medir", Action::Nav(Nav::Measure)),
            ("Herramienta: Densidad de puntos", Action::Nav(Nav::Density)),
            ("Herramienta: Perfil", Action::Nav(Nav::Profile)),
            (
                "Herramienta: Digitalizar en la capa activa",
                Action::Nav(Nav::Digitize),
            ),
            ("Crear capa de puntos, líneas o áreas", Action::NewLayer),
            ("Tabla de atributos", Action::LayerTable),
            ("Guardar la capa activa", Action::SaveLayer),
            ("Composición de bandas", Action::ImageTool(0)),
            ("Índices espectrales (NDVI…)", Action::ImageTool(1)),
            ("Crear imagen multibanda", Action::RasterTool(0)),
            ("Mosaico de rásteres", Action::RasterTool(1)),
            (
                "Igualar rásteres (mismas celdas que una referencia)",
                Action::Align,
            ),
            (
                "Análisis de terreno (pendiente, orientación, drenaje, visibilidad…)",
                Action::Terrain,
            ),
            (
                "OBIA: análisis por objetos (segmentar, describir y clasificar)",
                Action::Obia,
            ),
            (
                "Cambio forestal entre dos fechas (diferencia de CHM, perturbaciones, crecimiento)",
                Action::Change,
            ),
            (
                "Modelos de transición (regresión de una variable de campo sobre las métricas LiDAR de las parcelas)",
                Action::Model,
            ),
            (
                "Corredores eléctricos (conductores, postes y vegetación en riesgo)",
                Action::Corridor,
            ),
            (
                "Editar: seleccionar con rectángulo",
                Action::Nav(Nav::SelRect),
            ),
            (
                "Editar: seleccionar con polígono",
                Action::Nav(Nav::SelPoly),
            ),
            ("Editar: seleccionar con lazo", Action::Nav(Nav::SelLasso)),
            (
                "Editar: seleccionar con círculo",
                Action::Nav(Nav::SelCircle),
            ),
            ("Información de los datos", Action::Info),
            ("Clasificar puntos", Action::Tool(ToolId::Classify)),
            ("Asignar sistema de coordenadas", Action::Tool(ToolId::Crs)),
            ("Adelgazar nube (decimar)", Action::Tool(ToolId::Thin)),
            ("Dendrometría de fustes (TLS)", Action::Tool(ToolId::Tls)),
            ("Volúmenes respecto al MDT", Action::Tool(ToolId::Volume)),
            ("Quitar puntos duplicados", Action::Tool(ToolId::Dups)),
            ("Alturas sobre el suelo", Action::Tool(ToolId::Hag)),
            (
                "Colorear la nube con una ortofoto (RGB)",
                Action::Tool(ToolId::Colorize),
            ),
            (
                "Normalizar la intensidad por distancia",
                Action::Tool(ToolId::Intensity),
            ),
            ("Guardar como COPC", Action::Tool(ToolId::Copc)),
            ("Comparar nubes (cambios)", Action::Tool(ToolId::Compare)),
            ("Ajustar una nube a otra", Action::Tool(ToolId::Fit)),
            ("Separación entre pasadas", Action::Tool(ToolId::Swath)),
            ("Calcular MDT", Action::Tool(ToolId::Dtm)),
            ("Calcular MDS / CHM", Action::Tool(ToolId::Chm)),
            ("Métricas de rodal", Action::Tool(ToolId::Metrics)),
            ("Segmentar árboles", Action::Tool(ToolId::Trees)),
            (
                "Recortar con polígonos o parcelas",
                Action::Tool(ToolId::Clip),
            ),
            ("Exportar puntos", Action::Tool(ToolId::Export)),
            ("Colorear por elevación", Action::Color(ColorBy::Elevation)),
            (
                "Colorear por elevación (tierra)",
                Action::Color(ColorBy::ElevationEarth),
            ),
            ("Colorear por clase", Action::Color(ColorBy::Class)),
            ("Colorear por intensidad", Action::Color(ColorBy::Intensity)),
            ("Colorear por retorno", Action::Color(ColorBy::Return)),
            ("Colorear por fuente", Action::Color(ColorBy::Source)),
            ("Colorear por archivo", Action::Color(ColorBy::File)),
            ("Colorear por RGB", Action::Color(ColorBy::Rgb)),
            ("TIN sombreado", Action::ToggleTin),
            ("Curvas de nivel", Action::ToggleContours),
            ("Cargar ráster de fondo", Action::LoadRaster),
            ("Cargar shapefile de referencia", Action::LoadVector),
            ("Limpiar mediciones y perfil", Action::ClearMarks),
            ("Cambiar tema claro / oscuro", Action::ToggleTheme),
            ("Clases visibles y colores", Action::Classes),
            ("Mostrar u ocultar el registro", Action::ToggleLog),
            ("Atajos de teclado", Action::Shortcuts),
            ("Acerca de EasyLiDAR", Action::About),
            ("Manual de uso (F1)", Action::Manual),
        ]
    }

    // ----------------------------------------------------------------------------- chrome

    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        egui::Panel::top("header")
            .exact_size(34.0)
            .frame(egui::Frame::new().fill(t.header))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    if let Some(l) = &self.logo {
                        ui.image((l.id(), Vec2::splat(22.0)));
                    }
                    ui.label(
                        egui::RichText::new("EasyLiDAR")
                            .strong()
                            .size(14.0)
                            .color(theme::BRAND),
                    );
                    ui.add_space(8.0);
                    for (icon, tip, a) in [
                        ("@open", "Abrir (Ctrl+O)", Action::Open),
                        ("🖼", "Guardar vista PNG", Action::SaveShot),
                        ("@full", "Zoom completo", Action::ZoomFull),
                        ("@prev", "Zoom anterior", Action::ZoomPrev),
                    ] {
                        if ribbon::icon_button(ui, &t, icon, tip).clicked() {
                            self.act(ctx, a);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new("🔍 Buscar comando  Ctrl+K")
                                    .size(11.5)
                                    .color(t.weak),
                            ))
                            .clicked()
                        {
                            self.palette = Some(String::new());
                        }
                        if ui
                            .add(egui::Button::new(if self.dark { "☀" } else { "🌙" }).frame(false))
                            .on_hover_text("Tema claro / oscuro")
                            .clicked()
                        {
                            self.act(ctx, Action::ToggleTheme);
                        }
                    });
                });
            });
    }

    fn ribbon(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        egui::Panel::top("tabs")
            .exact_size(34.0)
            .frame(
                egui::Frame::new()
                    .fill(t.header)
                    .inner_margin(egui::Margin::symmetric(8, 2)),
            )
            .show(ui, |ui| {
                let extra = layer_tabs::tabs_for(
                    self.sel_layer
                        .and_then(|k| self.refs.get(k))
                        .map(|r| &r.kind),
                );
                if self.tab >= FIRST_EXTRA + extra.len() {
                    self.tab = 0;
                }
                let mut names = vec!["Inicio", "Vista", "Procesar", "Ayuda"];
                names.extend_from_slice(extra);
                let mut tab = self.tab;
                if ribbon::tab_strip(ui, &t, "Archivo", &names, FIRST_EXTRA, &mut tab) {
                    self.act(ctx, Action::Open);
                }
                self.tab = tab;
            });
        egui::Panel::top("ribbon")
            .exact_size(108.0)
            .frame(
                egui::Frame::new()
                    .fill(t.ribbon)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .stroke(Stroke::new(1.0, t.border)),
            )
            .show(ui, |ui| {
                let busy = self.running.is_some();
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| match self.tab {
                            0 => {
                                ribbon::group(ui, &t, "Proyecto", |ui| {
                                    if ribbon::big_button(ui, &t, "@save", "Guardar\nproyecto", false)
                                        .on_hover_text("Guarda qué nubes y capas están abiertas y cómo se ve (archivo .elp).")
                                        .clicked()
                                    {
                                        self.act(ctx, Action::SaveProject);
                                    }
                                    if ribbon::big_button(ui, &t, "@open", "Abrir\nproyecto", false)
                                        .clicked()
                                    {
                                        self.act(ctx, Action::OpenProject);
                                    }
                                });
                                ribbon::group(ui, &t, "Archivo", |ui| {
                                    if ribbon::big_button(ui, &t, "@adddata", "Abrir", false).clicked()
                                    {
                                        self.act(ctx, Action::Open);
                                    }
                                    ui.vertical(|ui| {
                                        let recent = prefs::recent();
                                        let mut reopen = None;
                                        ribbon::small_menu(
                                            ui,
                                            &t,
                                            "@open",
                                            "Recientes",
                                            !recent.is_empty(),
                                            |ui| {
                                                for f in &recent {
                                                    if ui
                                                        .button(prefs::label(f))
                                                        .on_hover_text(f.display().to_string())
                                                        .clicked()
                                                    {
                                                        reopen = Some(f.clone());
                                                    }
                                                }
                                                ui.separator();
                                                if ui.button("Vaciar la lista").clicked() {
                                                    prefs::clear_recent();
                                                }
                                            },
                                        )
                                        .on_hover_text("Los últimos archivos abiertos")
                                        .on_disabled_hover_text("Aún no se ha abierto ningún archivo");
                                        if let Some(f) = reopen {
                                            self.open_files(ctx, vec![f]);
                                        }
                                        if ribbon::small_button(ui, &t, "ℹ", "Información", false)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::Info);
                                        }
                                        if ribbon::small_button(ui, &t, "✖", "Cerrar todo", false)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::CloseAll);
                                        }
                                    });
                                });
                                ribbon::group(ui, &t, "Vectorial", |ui| {
                                    if ribbon::big_button(ui, &t, "@newlayer", "Nueva\ncapa", false)
                                        .clicked()
                                    {
                                        self.act(ctx, Action::NewLayer);
                                    }
                                    if ribbon::big_button(
                                        ui,
                                        &t,
                                        "@table",
                                        "Tabla de\natributos",
                                        self.table_layer.is_some(),
                                    )
                                    .clicked()
                                    {
                                        self.act(ctx, Action::LayerTable);
                                    }
                                    ui.vertical(|ui| {
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@pencil",
                                            "Digitalizar",
                                            self.nav == Nav::Digitize,
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::Nav(Nav::Digitize));
                                        }
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@save",
                                            "Guardar capa",
                                            false,
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::SaveLayer);
                                        }
                                    });
                                });
                                ribbon::group(ui, &t, "Ráster", |ui| {
                                    if ribbon::big_button(
                                        ui,
                                        &t,
                                        "@rs",
                                        "Composición\nde bandas",
                                        false,
                                    )
                                    .clicked()
                                    {
                                        self.act(ctx, Action::ImageTool(0));
                                    }
                                    if ribbon::big_button(
                                        ui,
                                        &t,
                                        "@bands",
                                        "Índices\nespectrales",
                                        self.rs.open,
                                    )
                                    .clicked()
                                    {
                                        self.act(ctx, Action::ImageTool(1));
                                    }
                                    ui.vertical(|ui| {
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@stack",
                                            "Crear multibanda",
                                            self.rt.open == Some(0),
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::RasterTool(0));
                                        }
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@mosaic",
                                            "Mosaico",
                                            self.rt.open == Some(1),
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::RasterTool(1));
                                        }
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@stack",
                                            "Igualar rásteres",
                                            self.at.open,
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::Align);
                                        }
                                    });
                                });
                                ribbon::group(ui, &t, "Aspecto", |ui| {
                                    if ribbon::big_button(
                                        ui,
                                        &t,
                                        if self.dark { "☀" } else { "🌙" },
                                        "Tema",
                                        false,
                                    )
                                    .clicked()
                                    {
                                        self.act(ctx, Action::ToggleTheme);
                                    }
                                    if ribbon::big_button(ui, &t, "📜", "Registro", self.show_log)
                                        .clicked()
                                    {
                                        self.act(ctx, Action::ToggleLog);
                                    }
                                });
                            }
                            2 => {
                                let groups = self.process_groups();
                                ribbon::adaptive_groups(ui, &t, &groups, |a| self.act(ctx, a));
                                if busy {
                                    ui.spinner();
                                }
                            }
                            1 => {
                                ribbon::group(ui, &t, "Navegación", |ui| {
                                    for (icon, name, n) in
                                        [("🔍", "Zoom", Nav::Zoom), ("✋", "Desplazar", Nav::Pan)]
                                    {
                                        if ribbon::big_button(ui, &t, icon, name, self.nav == n)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::Nav(n));
                                        }
                                    }
                                    ui.vertical(|ui| {
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@full",
                                            "Extensión completa",
                                            false,
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::ZoomFull);
                                        }
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@prev",
                                            "Zoom anterior",
                                            false,
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::ZoomPrev);
                                        }
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@fit",
                                            "Zoom a la selección",
                                            false,
                                        )
                                        .on_hover_text("Encuadra los puntos seleccionados o los elementos seleccionados de la capa vectorial activa.")
                                        .clicked()
                                        {
                                            self.act(ctx, Action::ZoomSel);
                                        }
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@full",
                                            "Refrescar 3D",
                                            false,
                                        )
                                        .on_hover_text("Vuelve a encuadrar la vista 3D en lo que se ve en la planta (útil con capas muy separadas).")
                                        .clicked()
                                        {
                                            self.act(ctx, Action::Relink3d);
                                        }
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@clear",
                                            "Limpiar marcas",
                                            false,
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::ClearMarks);
                                        }
                                    });
                                });
                                ribbon::group(ui, &t, "Ventanas", |ui| {
                                    ribbon::compact(ui);
                                    egui::Grid::new("rb_windows")
                                        .num_columns(2)
                                        .spacing([8.0, 3.0])
                                        .show(ui, |ui| {
                                            ui.label("Disposición");
                                            let names = ["2D", "2D + perfil", "3D", "2D + perfil + 3D"];
                                            let cur = [
                                                Layout::Plan,
                                                Layout::PlanProfile,
                                                Layout::View3d,
                                                Layout::All,
                                            ]
                                            .iter()
                                            .position(|l| *l == self.layout)
                                            .unwrap_or(0);
                                            egui::ComboBox::from_id_salt("layout")
                                                .width(128.0)
                                                .selected_text(names[cur])
                                                .show_ui(ui, |ui| {
                                                    for (k, n) in names.iter().enumerate() {
                                                        if ui.selectable_label(k == cur, *n).clicked() {
                                                            self.act(ctx, Action::Layout(k as u8));
                                                        }
                                                    }
                                                })
                                                .response
                                                .on_hover_text("Qué vistas se muestran a la vez: planta, perfil y 3D.");
                                            ui.end_row();
                                            ui.label("Puntos");
                                            ribbon::stepper(
                                                ui,
                                                "Puntos más pequeños en la planta y el 3D (−)",
                                                "Puntos más grandes en la planta y el 3D (+)",
                                                |d| self.act(ctx, Action::PointSize(d)),
                                            );
                                            ui.end_row();
                                            ui.label("3D");
                                            ui.menu_button("Opciones 3D  ▾", |ui| {
                                                if ui.button("Norte arriba").clicked() {
                                                    self.act(ctx, Action::NorthUp);
                                                }
                                                if ui.button("Exageración vertical +").clicked() {
                                                    self.act(ctx, Action::Exaggeration(1));
                                                }
                                                if ui.button("Exageración vertical −").clicked() {
                                                    self.act(ctx, Action::Exaggeration(-1));
                                                }
                                                ui.checkbox(
                                                    &mut self.follow_plan,
                                                    "Enlazar con la planta (2D+perfil+3D)",
                                                );
                                            })
                                            .response
                                            .on_hover_text("Norte arriba, exageración vertical y enlace de la vista 3D con la planta.");
                                            ui.end_row();
                                        });
                                });
                                ribbon::group(ui, &t, "Perfil", |ui| {
                                    ribbon::compact(ui);
                                    egui::Grid::new("rb_profile")
                                        .num_columns(2)
                                        .spacing([8.0, 3.0])
                                        .show(ui, |ui| {
                                            ui.label("Ancho");
                                            ui.add(
                                                crate::numfield::NumField::new(&mut self.profile_width)
                                                    .range(0.5..=1000.0)
                                                    .speed(0.5)
                                                    .suffix(" m"),
                                            )
                                            .on_hover_text("Ancho de la caja del perfil");
                                            ui.end_row();
                                            ui.label("Puntos");
                                            ribbon::stepper(
                                                ui,
                                                "Puntos más pequeños en el perfil (− con el cursor sobre él)",
                                                "Puntos más grandes en el perfil (+ con el cursor sobre él)",
                                                |d| self.act(ctx, Action::ProfileSize(d)),
                                            );
                                            ui.end_row();
                                            ui.label("");
                                            ui.checkbox(&mut self.prof_full, "Sin decimar")
                                                .on_hover_text("El perfil lee de los archivos los puntos de su caja sin decimar (hasta 6 millones), en segundo plano. Sin esto usa los mismos puntos que la planta y el 3D.");
                                            ui.end_row();
                                        });
                                });
                                ribbon::group(ui, &t, "Carga de puntos", |ui| {
                                    ribbon::compact(ui);
                                    let status = self.store.as_ref().map_or_else(
                                        || "Aún no hay ninguna nube cargada.".to_string(),
                                        |st| {
                                            format!(
                                                "Ahora: 1 de cada {} ({} de {} puntos).",
                                                st.stride,
                                                st.len(),
                                                st.total
                                            )
                                        },
                                    );
                                    egui::Grid::new("rb_load")
                                        .num_columns(2)
                                        .spacing([8.0, 3.0])
                                        .show(ui, |ui| {
                                            ui.label("Máximo");
                                            ui.add(
                                                crate::numfield::NumField::new(&mut self.max_points_m)
                                                    .range(0.5..=200.0)
                                                    .speed(0.5)
                                                    .suffix(" M"),
                                            )
                                            .on_hover_text(format!("Millones de puntos que se cargan en pantalla; una nube mayor se decima. Pulsa Aplicar para recargar. {status}"));
                                            ui.end_row();
                                            ui.label("");
                                            if ui
                                                .button("Aplicar")
                                                .on_hover_text(format!("Vuelve a cargar las nubes con ese máximo. {status}"))
                                                .clicked()
                                                && !self.clouds.is_empty()
                                            {
                                                self.reload(ctx);
                                            }
                                            ui.end_row();
                                        });
                                });
                                ribbon::group(ui, &t, "Visualización", |ui| {
                                    ribbon::compact(ui);
                                    ui.vertical(|ui| {
                                        let names = [
                                            ("Elevación (azul-rojo)", ColorBy::Elevation),
                                            ("Elevación (tierra)", ColorBy::ElevationEarth),
                                            ("Clasificación", ColorBy::Class),
                                            ("Intensidad", ColorBy::Intensity),
                                            ("Retorno", ColorBy::Return),
                                            ("ID de fuente", ColorBy::Source),
                                            ("Archivo", ColorBy::File),
                                            ("RGB", ColorBy::Rgb),
                                        ];
                                        let cur = names
                                            .iter()
                                            .find(|n| n.1 == self.style.color_by)
                                            .map_or("Colorear por…", |n| n.0);
                                        ui.horizontal(|ui| {
                                        ui.label("Color");
                                        egui::ComboBox::from_id_salt("colorby")
                                            .width(150.0)
                                            .selected_text(cur)
                                            .show_ui(ui, |ui| {
                                                for (name, c) in names {
                                                    if ui
                                                        .selectable_label(
                                                            self.style.color_by == c,
                                                            name,
                                                        )
                                                        .clicked()
                                                    {
                                                        self.act(ctx, Action::Color(c));
                                                    }
                                                }
                                            })
.response
.on_hover_text("Atributo con el que se colorean los puntos.");
                                        });
                                        ui.menu_button("Rango de elevación  ▾", |ui| {
                                            let (lo, hi) = self
                                                .store
                                                .as_ref()
                                                .map_or((0.0, 1.0), |s| (s.bounds[2], s.bounds[5]));
                                            ui.horizontal(|ui| {
                                                ui.label("Mín");
                                                if ui
                                                    .add(
                                                        crate::numfield::NumField::new(
                                                            &mut self.style.zmin,
                                                        )
                                                        .range(lo - 1000.0..=hi)
                                                        .speed(0.5),
                                                    )
                                                    .changed()
                                                {
                                                    self.touch();
                                                }
                                                ui.label("Máx");
                                                if ui
                                                    .add(
                                                        crate::numfield::NumField::new(
                                                            &mut self.style.zmax,
                                                        )
                                                        .range(lo..=hi + 1000.0)
                                                        .speed(0.5),
                                                    )
                                                    .changed()
                                                {
                                                    self.touch();
                                                }
                                            });
                                            if ui.button("Ajustar a los datos").clicked() {
                                                self.style.zmin = lo;
                                                self.style.zmax = hi;
                                            }
                                        })
.response
.on_hover_text("Rango de elevaciones al que se reparte la rampa de color.");
                                        ui.menu_button("Filtros de retornos y clases  ▾", |ui| {
                                            for (k, n) in [
                                                "Primeros retornos",
                                                "Retornos intermedios",
                                                "Últimos retornos",
                                                "Retornos únicos",
                                            ]
                                            .iter()
                                            .enumerate()
                                            {
                                                let mut on = self.style.returns & (1 << k) != 0;
                                                if ui.checkbox(&mut on, *n).changed() {
                                                    self.style.returns ^= 1 << k;
                                                }
                                            }
                                            ui.separator();
                                            if ui.button("Clases y colores…").clicked() {
                                                self.act(ctx, Action::Classes);
                                            }
                                        })
.response
.on_hover_text("Qué retornos y qué clases se dibujan.");
                                    });
                                });
                                ribbon::group(ui, &t, "Herramientas", |ui| {
                                    for (icon, name, n) in [
                                        ("❓", "Consultar", Nav::Query),
                                        ("@density", "Densidad", Nav::Density),
                                        ("📈", "Perfil", Nav::Profile),
                                    ] {
                                        if ribbon::big_button(ui, &t, icon, name, self.nav == n)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::Nav(n));
                                        }
                                    }
                                    ui.vertical(|ui| {
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "📏",
                                            "Medir",
                                            self.nav == Nav::Measure,
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::Nav(Nav::Measure));
                                        }
                                        ui.horizontal(|ui| {
                                        if ribbon::small_button(
                                            ui,
                                            &t,
                                            "@tin",
                                            "TIN",
                                            self.tin_on,
                                        )
                                        .clicked()
                                        {
                                            self.act(ctx, Action::ToggleTin);
                                        }
ui.menu_button("⚙", |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label("Equidistancia");
                                                    ui.add(
                                                        crate::numfield::NumField::new(
                                                            &mut self.contour_iv,
                                                        )
                                                        .range(0.1..=500.0),
                                                    );
                                                    ui.label("m");
                                                });
                                                ui.horizontal(|ui| {
                                                    ui.label("TIN con");
                                                    let ground = self.tin_classes
                                                        == ClassSet::from_bits(1 << 2);
                                                    if ui
                                                        .selectable_label(ground, "suelo")
                                                        .clicked()
                                                    {
                                                        self.tin_classes =
                                                            ClassSet::from_bits(1 << 2);
                                                    }
                                                    if ui
                                                        .selectable_label(
                                                            !ground,
                                                            "todas las clases",
                                                        )
                                                        .clicked()
                                                    {
                                                        self.tin_classes = ClassSet::all();
                                                    }
                                                });
                                            })
                                            .response
                                            .on_hover_text("Opciones del TIN y las curvas");
                                        });
                                                                                    if ribbon::small_button(
                                                ui,
                                                &t,
                                                "〰",
                                                "Curvas de nivel",
                                                self.contours_on,
                                            )
                                            .clicked()
                                            {
                                                self.act(ctx, Action::ToggleContours);
                                            }


                                    });
                                });
                            }
                            3 => {
                                ribbon::group(ui, &t, "Ayuda", |ui| {
                                    if ribbon::big_button(
                                        ui,
                                        &t,
                                        "@manual",
                                        "Manual",
                                        self.manual.open,
                                    )
                                    .clicked()
                                    {
                                        self.act(ctx, Action::Manual);
                                    }
                                    if ribbon::big_button(
                                        ui,
                                        &t,
                                        "@keys",
                                        "Atajos de\nteclado",
                                        self.show_shortcuts,
                                    )
                                    .clicked()
                                    {
                                        self.act(ctx, Action::Shortcuts);
                                    }
                                    if ribbon::big_button(ui, &t, "ℹ", "Acerca de", false).clicked()
                                    {
                                        self.act(ctx, Action::About);
                                    }
                                    if ribbon::big_button(ui, &t, "🔍", "Buscar\ncomando", false)
                                        .clicked()
                                    {
                                        self.palette = Some(String::new());
                                    }
                                });
                            }
                            _ => self.layer_tab(ui, &t, ctx, self.tab),
                        });
                    });
            });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        egui::Panel::left("side")
            .default_size(280.0)
            .size_range(200.0..=520.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(t.pane)
                    .inner_margin(egui::Margin::same(8))
                    .stroke(Stroke::new(1.0, t.border)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.side, Side::Layers, "Contenido");
                    ui.selectable_value(&mut self.side, Side::Info, "Información");
                    ui.selectable_value(&mut self.side, Side::Edit, "Edición");
                });
                ui.separator();
                match self.side {
                    Side::Edit => self.edit_side(ui, ctx),
                    Side::Info => {
                        ui.horizontal(|ui| {
                            if ui.button("↻ Analizar de nuevo").clicked() {
                                self.run_info(ctx);
                            }
                            if ui
                                .add_enabled(
                                    !self.info_text.is_empty(),
                                    egui::Button::new("⧉ Copiar todo"),
                                )
                                .on_hover_text("Copia todo el informe al portapapeles")
                                .clicked()
                            {
                                ctx.copy_text(self.info_text.clone());
                                self.log("Información copiada al portapapeles.");
                            }
                        });
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&self.info_text).monospace().size(11.5),
                                    )
                                    .selectable(true)
                                    .wrap_mode(egui::TextWrapMode::Extend),
                                )
                                .context_menu(|ui| {
                                    if ui.button("Copiar todo").clicked() {
                                        ctx.copy_text(self.info_text.clone());
                                        ui.close();
                                    }
                                });
                            });
                    }
                    Side::Layers => self.contents_ui(ui, ctx),
                }
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        // the system of the view: that of the first cloud (the others are brought to it)
        let system = (!self.clouds.is_empty()).then(|| match self.project_crs() {
            Some(code) => rustlidar_core::crs::name(code),
            None => "sin declarar".to_string(),
        });
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(
                egui::Frame::new()
                    .fill(t.header)
                    .inner_margin(egui::Margin::symmetric(10, 3))
                    .stroke(Stroke::new(1.0, t.border)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let small = |s: String| egui::RichText::new(s).size(11.5).color(t.weak);
                    ui.label(small(match self.cursor_map {
                        Some((x, y)) => format!("X {x:.2}   Y {y:.2}"),
                        None => "X —   Y —".into(),
                    }));
                    ui.separator();
                    if let Some(sys) = &system {
                        ui.label(small(format!("Sistema: {sys}"))).on_hover_text(
                            "Sistema de coordenadas de la vista: el de la primera nube. Las capas en otro sistema se pasan a este al cargarlas.",
                        );
                        ui.separator();
                    }
                    if let Some(s) = &self.store {
                        ui.label(small(format!(
                            "{} puntos de {} (1 de cada {})",
                            s.len(),
                            s.total,
                            s.stride
                        )));
                        ui.separator();
                        ui.label(small(format!(
                            "Z {:.1} – {:.1} m",
                            s.bounds[2], s.bounds[5]
                        )));
                        ui.separator();
                    }
                    if let Some(e) = self.edit_status() {
                        ui.label(
                            egui::RichText::new(e)
                                .size(11.5)
                                .color(Color32::from_rgb(0, 220, 220)),
                        );
                        ui.separator();
                    }
                    ui.label(small(format!("Herramienta: {}", self.nav.name())));
                    ui.separator();
                    ui.label(small(format!(
                        "Escala 1 px = {:.3} m",
                        1.0 / self.plan.scale.max(1e-9)
                    )));
                    if self.running.is_some() {
                        ui.separator();
                        if ui
                            .small_button("■ Detener")
                            .on_hover_text("Para el trabajo y borra lo que haya creado")
                            .clicked()
                        {
                            rustlidar_core::request_cancel();
                        }
                    }
                    if let Some(r) = &self.running {
                        ui.separator();
                        ui.spinner();
                        ui.label(small(format!(
                            "{} · {:.0} s{}",
                            r.label,
                            r.started.elapsed().as_secs_f64(),
                            progress_text(r.started).map_or(String::new(), |p| format!(" · {p}"))
                        )));
                    } else if let Some((_, st)) = &self.loading {
                        ui.separator();
                        ui.spinner();
                        ui.label(small(format!(
                            "cargando… {:.0} s{}",
                            st.elapsed().as_secs_f64(),
                            progress_text(*st).map_or(String::new(), |p| format!(" · {p}"))
                        )));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(q) = &self.density {
                            ui.label(small(q.clone()));
                        }
                    });
                });
            });
        if self.show_log {
            egui::Panel::bottom("log")
                .default_size(150.0)
                .resizable(true)
                .frame(
                    egui::Frame::new()
                        .fill(t.pane)
                        .inner_margin(egui::Margin::same(6))
                        .stroke(Stroke::new(1.0, t.border)),
                )
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for l in &self.log {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(l).monospace().size(11.5))
                                        .selectable(true),
                                );
                            }
                        });
                });
        }
    }

    // ----------------------------------------------------------------------------- views

    fn views(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.canvas))
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let gap = 3.0;
                let (plan, prof, d3) = match self.layout {
                    Layout::Plan => (Some(full), None, None),
                    Layout::View3d => (None, None, Some(full)),
                    Layout::PlanProfile => {
                        let h = full.height() * 0.64;
                        (
                            Some(Rect::from_min_size(
                                full.min,
                                Vec2::new(full.width(), h - gap),
                            )),
                            Some(Rect::from_min_max(
                                Pos2::new(full.left(), full.top() + h),
                                full.max,
                            )),
                            None,
                        )
                    }
                    Layout::All => {
                        let w = full.width() * 0.56;
                        let h = full.height() * 0.5;
                        (
                            Some(Rect::from_min_size(
                                full.min,
                                Vec2::new(w - gap, full.height()),
                            )),
                            Some(Rect::from_min_size(
                                Pos2::new(full.left() + w, full.top()),
                                Vec2::new(full.width() - w, h - gap),
                            )),
                            Some(Rect::from_min_max(
                                Pos2::new(full.left() + w, full.top() + h),
                                full.max,
                            )),
                        )
                    }
                };
                if let Some(r) = plan {
                    self.plan_view(ui, ctx, r);
                }
                if let Some(r) = prof {
                    self.profile_view(ui, ctx, r);
                }
                if let Some(r) = d3 {
                    self.view_3d(ui, ctx, r);
                }
                self.sync_views();
            });
    }

    fn frame_title(&self, ui: &egui::Ui, rect: Rect, title: &str) {
        let t = self.tokens();
        let p = ui.painter_at(rect);
        p.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        p.text(
            rect.left_top() + Vec2::new(8.0, 6.0),
            Align2::LEFT_TOP,
            title,
            FontId::proportional(11.5),
            t.weak,
        );
    }

    fn plan_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rect: Rect) {
        let t = self.tokens();
        let (w, h) = (
            rect.width().max(8.0) as usize,
            rect.height().max(8.0) as usize,
        );
        let resp = ui.interact(rect, ui.id().with("plan"), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, color32(self.style.background));
        let store = self.store.clone();
        if store.is_none() && self.refs.is_empty() {
            let msg = if self.loading.is_some() {
                "Cargando la nube…"
            } else {
                "Abre una nube LAS/LAZ con «Abrir», o arrástrala a esta ventana"
            };
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                msg,
                FontId::proportional(14.0),
                t.weak,
            );
        }
        self.plan_px = (w, h);
        if self.plan_fit {
            if let Some(s) = &store {
                self.plan = Plan::fit(s.bounds, w, h);
                self.plan_fit = false;
            } else if let Some(b) = self.refs_bounds() {
                // a lone point has no extent: it gets a window of 40 m
                let pad = if (b[2] - b[0]).max(b[3] - b[1]) < 1e-6 {
                    20.0
                } else {
                    0.0
                };
                self.plan = Plan::fit(
                    [b[0] - pad, b[1] - pad, 0.0, b[2] + pad, b[3] + pad, 0.0],
                    w,
                    h,
                );
                self.plan_fit = false;
            }
        }
        if !self.plan_fit
            && self.img_jobs.is_empty()
            && self.loading.is_none()
            && let Some(p) = self.forced_view.take()
        {
            self.plan = p;
            self.relink_3d();
        }
        let origin = rect.min.to_vec2();
        let pointer = resp.hover_pos().or_else(|| resp.interact_pointer_pos());
        if let Some(p) = pointer {
            self.cursor_map = Some(self.plan.to_map(
                f64::from(p.x - rect.left()),
                f64::from(p.y - rect.top()),
                w,
                h,
            ));
        }
        // wheel zoom
        if resp.hovered() {
            let sc = ui.input(|i| i.smooth_scroll_delta.y);
            if sc.abs() > 0.5
                && let Some(p) = pointer
            {
                self.plan.zoom_at(
                    1.0015f64.powf(f64::from(sc)),
                    f64::from(p.x - rect.left()),
                    f64::from(p.y - rect.top()),
                    w,
                    h,
                );
                self.touch();
            }
        }
        // pan: middle or right drag, or the Pan tool
        let pan_drag = resp.dragged_by(egui::PointerButton::Middle)
            || resp.dragged_by(egui::PointerButton::Secondary)
            || (self.nav == Nav::Pan && resp.dragged_by(egui::PointerButton::Primary));
        if pan_drag {
            let d = resp.drag_delta();
            self.plan.cx -= f64::from(d.x) / self.plan.scale;
            self.plan.cy += f64::from(d.y) / self.plan.scale;
            self.touch();
        }
        if resp.middle_clicked()
            && let Some(p) = self.prev_plans.pop()
        {
            self.plan = p;
        }
        // tools
        let to_map = |s: &Self, p: Pos2| {
            s.plan.to_map(
                f64::from(p.x - rect.left()),
                f64::from(p.y - rect.top()),
                w,
                h,
            )
        };
        if resp.drag_started_by(egui::PointerButton::Primary) {
            self.drag_start = resp.interact_pointer_pos();
        }
        let click = if resp.clicked() {
            resp.interact_pointer_pos()
        } else {
            None
        };
        let right_click = if resp.secondary_clicked() {
            resp.interact_pointer_pos()
        } else {
            None
        };
        match self.nav {
            Nav::Zoom => {
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some(a), Some(b)) = (self.drag_start, resp.interact_pointer_pos())
                        && (a - b).length() > 6.0
                    {
                        let (m1, m2) = (to_map(self, a), to_map(self, b));
                        let bounds = [
                            m1.0.min(m2.0),
                            m1.1.min(m2.1),
                            0.0,
                            m1.0.max(m2.0),
                            m1.1.max(m2.1),
                            0.0,
                        ];
                        let p = Plan::fit(bounds, w, h);
                        self.zoom_to(Plan {
                            scale: p.scale / 0.96,
                            ..p
                        });
                    }
                    self.drag_start = None;
                } else if let Some(p) = click {
                    self.prev_plans.push(self.plan);
                    self.plan.zoom_at(
                        2.0,
                        f64::from(p.x - rect.left()),
                        f64::from(p.y - rect.top()),
                        w,
                        h,
                    );
                } else if let Some(p) = right_click {
                    self.prev_plans.push(self.plan);
                    self.plan.zoom_at(
                        0.5,
                        f64::from(p.x - rect.left()),
                        f64::from(p.y - rect.top()),
                        w,
                        h,
                    );
                }
            }
            Nav::Pan => {}
            Nav::SelRect | Nav::SelPoly | Nav::SelLasso | Nav::SelCircle => {
                if store.is_some() {
                    self.plan_select(ctx, &resp, rect, w, h)
                }
            }
            Nav::Query => {
                if let Some(p) = click {
                    let m = to_map(self, p);
                    let tol = 6.0 / self.plan.scale;
                    self.query = store
                        .as_ref()
                        .and_then(|s| view::nearest_point(s, &self.style, m.0, m.1, tol));
                    self.ident = self.identify(m.0, m.1, tol);
                }
            }
            Nav::Measure => {
                if let Some(p) = click {
                    let m = to_map(self, p);
                    let tol = 6.0 / self.plan.scale;
                    let z = store
                        .as_ref()
                        .and_then(|s| {
                            view::nearest_point(s, &self.style, m.0, m.1, tol).map(|i| s.xyz(i)[2])
                        })
                        .unwrap_or(f64::NAN);
                    self.measure.push([m.0, m.1, z]);
                }
                if right_click.is_some() {
                    self.measure.clear();
                }
            }
            Nav::Density => {
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some(a), Some(b), Some(s)) =
                        (self.drag_start, resp.interact_pointer_pos(), &store)
                    {
                        let (m1, m2) = (to_map(self, a), to_map(self, b));
                        let r = [
                            m1.0.min(m2.0),
                            m1.1.min(m2.1),
                            m1.0.max(m2.0),
                            m1.1.max(m2.1),
                        ];
                        let (n, area, d) = view::density(s, &self.style, r);
                        self.density = Some(format!(
                            "Densidad: {n} puntos en {area:.1} m² = {d:.2} pts/m²"
                        ));
                    }
                    self.drag_start = None;
                }
            }
            Nav::Profile => {
                if let Some(p) = click {
                    let m = to_map(self, p);
                    self.profile_clicks.push([m.0, m.1]);
                    if self.profile_clicks.len() == 3 {
                        let (a, b, c) = (
                            self.profile_clicks[0],
                            self.profile_clicks[1],
                            self.profile_clicks[2],
                        );
                        let bx = ProfileBox { a, b, width: 1.0 };
                        let (_, across) = bx.local(c[0], c[1]);
                        self.profile_width = (across.abs() * 2.0).max(1.0);
                        self.profile = Some(ProfileBox {
                            a,
                            b,
                            width: self.profile_width,
                        });
                        self.profile_clicks.clear();
                        if self.layout == Layout::Plan {
                            self.layout = Layout::PlanProfile;
                        }
                    }
                }
                if right_click.is_some() {
                    self.profile_clicks.clear();
                }
            }
            Nav::Digitize | Nav::VecSelect | Nav::VecVertex | Nav::VecMove => {
                self.vector_tool(&resp, rect, w, h)
            }
        }
        // render points (thinned while the view moves)
        let interactive = Instant::now() < self.moving_until;
        let (lo, hi) = (
            self.plan.to_map(0.0, h as f64, w, h),
            self.plan.to_map(w as f64, 0.0, w, h),
        );
        self.update_detail(ctx, [lo.0, lo.1, hi.0, hi.1], interactive);
        if let Some(store) = &store {
            // zoomed into COPC clouds the window is drawn from its own, denser points
            let detail = self
                .detail
                .as_ref()
                .filter(|d| contains(d.window, [lo.0, lo.1, hi.0, hi.1]))
                .map(|d| (d.store.clone(), d.spacing, d.id));
            let shown: &Arc<PointStore> = detail.as_ref().map_or(store, |d| &d.0);
            let thin = if interactive && shown.len() > 1_500_000 {
                shown.len() / 1_500_000 + 1
            } else {
                1
            };
            self.style.thin = thin;
            self.style.transparent = true;
            let key = hash_of((
                self.style_key(),
                w,
                h,
                self.plan.cx.to_bits(),
                self.plan.cy.to_bits(),
                self.plan.scale.to_bits(),
                detail.as_ref().map(|d| d.2),
            ));
            if self.tex_plan.as_ref().is_none_or(|t| t.1 != key) {
                let mut st = self.style.clone();
                if let Some(d) = &detail {
                    st.spacing = d.1;
                }
                let img = view::render_plan(shown, &self.plan, w, h, &st);
                match &mut self.tex_plan {
                    Some((tex, k)) => {
                        tex.set(
                            egui::ColorImage::from_rgba_unmultiplied([img.w, img.h], &img.rgba),
                            egui::TextureOptions::NEAREST,
                        );
                        *k = key;
                    }
                    None => {
                        self.tex_plan = Some((
                            to_texture(ctx, "plan", &img, egui::TextureOptions::NEAREST),
                            key,
                        ))
                    }
                }
            }
            self.style.thin = 1;
        }
        if interactive {
            ctx.request_repaint_after(Duration::from_millis(170));
        }
        // paint: images and grids, TIN, points, vectors, marks
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        self.update_images(ctx, w, h);
        // the layers under the point clouds, then the clouds, then the layers over them
        let slot = self.cloud_slot.min(self.refs.len());
        // the base map goes under everything
        let project = if self.basemap.as_ref().is_some_and(|b| b.on) {
            self.project_crs()
        } else {
            None
        };
        let plan_now = self.plan;
        if let Some(bm) = &mut self.basemap
            && let Some(msg) = bm.paint(ctx, &painter, &plan_now, rect, w, h, project)
        {
            painter.text(
                rect.left_top() + Vec2::new(10.0, 26.0),
                egui::Align2::LEFT_TOP,
                msg,
                egui::FontId::proportional(12.0),
                Color32::from_rgb(230, 170, 60),
            );
        }
        self.paint_layers(&painter, rect, w, h, 0..slot);
        if let Some(store) = &store {
            if self.tin_on || self.contours_on {
                self.paint_tin(ctx, &painter, store, rect, w, h);
            }
            // the TIN and the contours stand where the points would
            if self.show_points
                && !(self.tin_on || self.contours_on)
                && let Some((tex, _)) = &self.tex_plan
            {
                painter.image(tex.id(), rect, uv, Color32::WHITE);
            }
        }
        let m2s = |s: &Self, x: f64, y: f64| {
            let (px, py) = s.plan.to_screen(x, y, w, h);
            Pos2::new(px as f32, py as f32) + origin
        };
        self.paint_layers(&painter, rect, w, h, slot..self.refs.len());
        self.paint_draft(&painter, pointer, rect, w, h);
        // the box marks what the profile window shows: with no such window it is not drawn
        let profile_shown = matches!(self.layout, Layout::PlanProfile | Layout::All);
        if let Some(bx) = self.profile.as_ref().filter(|_| profile_shown) {
            let l = bx.length().max(1e-9);
            let (nx, ny) = (
                -(bx.b[1] - bx.a[1]) / l * bx.width / 2.0,
                (bx.b[0] - bx.a[0]) / l * bx.width / 2.0,
            );
            let c = [
                [bx.a[0] + nx, bx.a[1] + ny],
                [bx.b[0] + nx, bx.b[1] + ny],
                [bx.b[0] - nx, bx.b[1] - ny],
                [bx.a[0] - nx, bx.a[1] - ny],
                [bx.a[0] + nx, bx.a[1] + ny],
            ];
            painter.add(egui::Shape::line(
                c.iter().map(|p| m2s(self, p[0], p[1])).collect(),
                Stroke::new(self.line_weight, Color32::from_rgb(255, 80, 80)),
            ));
            painter.line_segment(
                [m2s(self, bx.a[0], bx.a[1]), m2s(self, bx.b[0], bx.b[1])],
                Stroke::new(1.0, Color32::from_rgb(255, 80, 80)),
            );
        }
        for (k, c) in self.profile_clicks.iter().enumerate() {
            painter.circle_filled(m2s(self, c[0], c[1]), 4.0, Color32::from_rgb(255, 80, 80));
            if k > 0 {
                painter.line_segment(
                    [
                        m2s(
                            self,
                            self.profile_clicks[k - 1][0],
                            self.profile_clicks[k - 1][1],
                        ),
                        m2s(self, c[0], c[1]),
                    ],
                    Stroke::new(1.0, Color32::from_rgb(255, 80, 80)),
                );
            }
        }
        if !self.measure.is_empty() {
            let (mut total, mut total3) = (0.0, 0.0);
            for k in 0..self.measure.len() {
                let p = self.measure[k];
                painter.circle_filled(m2s(self, p[0], p[1]), 3.5, Color32::from_rgb(120, 255, 120));
                if k > 0 {
                    let q = self.measure[k - 1];
                    painter.line_segment(
                        [m2s(self, q[0], q[1]), m2s(self, p[0], p[1])],
                        Stroke::new(self.line_weight, Color32::from_rgb(120, 255, 120)),
                    );
                    let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                    total += d;
                    let dz = if p[2].is_nan() || q[2].is_nan() {
                        0.0
                    } else {
                        p[2] - q[2]
                    };
                    total3 += (d * d + dz * dz).sqrt();
                }
            }
            if self.measure.len() >= 2 {
                let l = self.measure[self.measure.len() - 1];
                let pos = m2s(self, l[0], l[1]) + Vec2::new(8.0, -8.0);
                let q = self.measure[self.measure.len() - 2];
                let dz = if l[2].is_nan() || q[2].is_nan() {
                    f64::NAN
                } else {
                    l[2] - q[2]
                };
                painter.text(
                    pos,
                    Align2::LEFT_BOTTOM,
                    format!(
                        "{:.2} m (último tramo)   total {:.2} m   3D {:.2} m   Δz {}{}",
                        (l[0] - q[0]).hypot(l[1] - q[1]),
                        total,
                        total3,
                        if dz.is_nan() {
                            "—".into()
                        } else {
                            format!("{dz:+.2} m")
                        },
                        polygon_text(&self.measure)
                    ),
                    FontId::proportional(12.0),
                    Color32::from_rgb(120, 255, 120),
                );
            }
        }
        if let (Some(i), Some(store)) = (self.query, &store)
            && i < store.len()
        {
            let p = store.xyz(i);
            let c = m2s(self, p[0], p[1]);
            painter.circle_stroke(c, 7.0, Stroke::new(2.0, Color32::WHITE));
            let class = store.class[i];
            let lines = [
                format!("X {:.2}   Y {:.2}   Z {:.2}", p[0], p[1], p[2]),
                format!("Clase {class} · {}", inspect::class_name(class)),
                format!(
                    "Intensidad {}   Retorno {} de {}   Fuente {}",
                    store.intensity[i],
                    store.ret[i] & 15,
                    store.ret[i] >> 4,
                    store.src[i]
                ),
            ];
            let box_rect = Rect::from_min_size(
                rect.left_top() + Vec2::new(10.0, 46.0),
                Vec2::new(330.0, 62.0),
            );
            painter.rect_filled(box_rect, 4.0, Color32::from_black_alpha(190));
            for (k, l) in lines.iter().enumerate() {
                painter.text(
                    box_rect.min + Vec2::new(8.0, 6.0 + k as f32 * 17.0),
                    Align2::LEFT_TOP,
                    l,
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
        }
        if let (Some(a), Some(p), true) = (
            self.drag_start,
            resp.interact_pointer_pos(),
            matches!(self.nav, Nav::Zoom | Nav::Density),
        ) && resp.dragged_by(egui::PointerButton::Primary)
        {
            let r = Rect::from_two_pos(a, p);
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            painter.rect_filled(r, 0.0, t.accent.gamma_multiply(0.15));
        }
        self.paint_ident(&painter, rect);
        let hint = self.vec_tool_hint();
        if let Some(hint) = hint {
            painter.text(
                rect.center_top() + Vec2::new(0.0, 8.0),
                Align2::CENTER_TOP,
                hint,
                FontId::proportional(12.0),
                Color32::from_rgb(255, 220, 120),
            );
        }
        if editing::is_select(self.nav) {
            self.paint_plan_select(&painter, rect, w, h, &resp);
        }
        // scale bar
        let bar_m = {
            let target = 120.0 / self.plan.scale;
            let p10 = 10f64.powf(target.log10().floor());
            [1.0, 2.0, 5.0, 10.0]
                .iter()
                .map(|k| k * p10)
                .filter(|v| *v <= target * 1.2)
                .fold(p10, f64::max)
        };
        let bar_px = (bar_m * self.plan.scale) as f32;
        let y = rect.bottom() - 18.0;
        let x0 = rect.left() + 14.0;
        painter.line_segment(
            [Pos2::new(x0, y), Pos2::new(x0 + bar_px, y)],
            Stroke::new(2.0, Color32::WHITE),
        );
        painter.text(
            Pos2::new(x0, y - 4.0),
            Align2::LEFT_BOTTOM,
            format!("{} m", bar_m),
            FontId::proportional(11.0),
            Color32::WHITE,
        );
        self.frame_title(
            ui,
            rect,
            &format!(
                "Planta · {} · {}",
                self.nav.name(),
                match self.style.color_by {
                    ColorBy::Elevation | ColorBy::ElevationEarth => "elevación",
                    ColorBy::Class => "clasificación",
                    ColorBy::Intensity => "intensidad",
                    ColorBy::Return => "retorno",
                    ColorBy::Source => "fuente",
                    ColorBy::File => "archivo",
                    ColorBy::Rgb => "RGB",
                }
            ),
        );
        // legend for classes
        if self.style.color_by == ColorBy::Class {
            let mut y = rect.top() + 26.0;
            for c in 0..256usize {
                if self.class_counts[c] > 0 {
                    let x = rect.right() - 170.0;
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y), Vec2::new(12.0, 12.0)),
                        2.0,
                        color32(self.style.class_colors[c]),
                    );
                    painter.text(
                        Pos2::new(x + 18.0, y + 6.0),
                        Align2::LEFT_CENTER,
                        format!("{c} {}", self.ctable.name(c as u8)),
                        FontId::proportional(11.0),
                        t.text,
                    );
                    y += 16.0;
                }
            }
        }
    }

    fn paint_tin(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        store: &PointStore,
        rect: Rect,
        w: usize,
        h: usize,
    ) {
        let key = hash_of((
            self.rev,
            self.tin_classes,
            self.style.color_by as u8,
            self.style.zmin.to_bits(),
            self.style.zmax.to_bits(),
        ));
        let ckey = hash_of((self.contour_iv.to_bits(), key));
        if self.tin.as_ref().is_none_or(|t| t.key != key) {
            match view::surface(store, self.tin_classes, 1400) {
                Ok(grid) => {
                    let img = view::shade(&grid, &self.style, 315.0, 45.0, true);
                    let tex = to_texture(ctx, "tin", &img, egui::TextureOptions::LINEAR);
                    self.tin = Some(Tin {
                        grid,
                        tex,
                        contours: Vec::new(),
                        key,
                        contour_key: 0,
                    });
                }
                Err(e) => {
                    self.log(format!("TIN: {e}"));
                    self.tin_on = false;
                    return;
                }
            }
        }
        let (on, contours_on, iv) = (self.tin_on, self.contours_on, self.contour_iv);
        let (lw, origin) = (self.line_weight, rect.min.to_vec2());
        let Some(tin) = &mut self.tin else { return };
        if contours_on && tin.contour_key != ckey {
            tin.contours = view::contours(&tin.grid, iv);
            tin.contour_key = ckey;
        }
        let g = &tin.grid;
        let clip = painter.with_clip_rect(rect);
        if on {
            let (a, b) = (
                self.plan
                    .to_screen(g.x0, g.y0 + g.rows as f64 * g.cell, w, h),
                self.plan
                    .to_screen(g.x0 + g.cols as f64 * g.cell, g.y0, w, h),
            );
            let rr = Rect::from_min_max(
                Pos2::new(a.0 as f32, a.1 as f32) + origin,
                Pos2::new(b.0 as f32, b.1 as f32) + origin,
            );
            clip.image(
                tin.tex.id(),
                rr,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if contours_on {
            let line = Color32::from_rgba_unmultiplied(250, 250, 250, 200);
            for s in &tin.contours {
                let (a, b) = (
                    self.plan.to_screen(s[0], s[1], w, h),
                    self.plan.to_screen(s[2], s[3], w, h),
                );
                let thick = (s[4] / (iv * 5.0)).fract().abs() < 1e-6;
                clip.line_segment(
                    [
                        Pos2::new(a.0 as f32, a.1 as f32) + origin,
                        Pos2::new(b.0 as f32, b.1 as f32) + origin,
                    ],
                    Stroke::new(if thick { lw * 0.75 } else { lw * 0.4 }, line),
                );
            }
        }
    }

    fn profile_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rect: Rect) {
        let t = self.tokens();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, color32(self.style.background));
        let resp = ui.interact(rect, ui.id().with("prof"), Sense::click_and_drag());
        self.profile_rect = Some(rect);
        let (w, h) = (
            rect.width().max(8.0) as usize,
            rect.height().max(8.0) as usize,
        );
        if let Some(p) = &mut self.profile {
            p.width = self.profile_width;
        }
        let pstore = self.profile.and_then(|b| self.profile_store(ctx, &b));
        if let (Some(store), Some(bx)) = (pstore, self.profile) {
            if resp.hovered() {
                let step = ui.input(|i| i.smooth_scroll_delta.y);
                let arrows = ui.input(|i| i.key_pressed(Key::ArrowUp)) as i32
                    - ui.input(|i| i.key_pressed(Key::ArrowDown)) as i32;
                let k = if step.abs() > 0.5 {
                    step.signum() as i32
                } else {
                    arrows
                };
                if k != 0 {
                    self.profile = Some(bx.shifted(f64::from(k) * bx.width));
                }
            }
            let bx = self.profile.unwrap_or(bx);
            self.style.thin = 1;
            self.style.transparent = false;
            let key = hash_of((
                self.style_key(),
                w,
                h,
                bx.a[0].to_bits(),
                bx.a[1].to_bits(),
                bx.b[0].to_bits(),
                bx.b[1].to_bits(),
                bx.width.to_bits(),
                Arc::as_ptr(&store) as usize,
            ));
            if self.tex_prof.as_ref().is_none_or(|t| t.1 != key) {
                let (img, lo, hi) = view::render_profile(&store, &bx, w, h, &self.style);
                match &mut self.tex_prof {
                    Some((tex, k, l, hh)) => {
                        tex.set(
                            egui::ColorImage::from_rgba_unmultiplied([img.w, img.h], &img.rgba),
                            egui::TextureOptions::NEAREST,
                        );
                        (*k, *l, *hh) = (key, lo, hi);
                    }
                    None => {
                        self.tex_prof = Some((
                            to_texture(ctx, "prof", &img, egui::TextureOptions::NEAREST),
                            key,
                            lo,
                            hi,
                        ))
                    }
                }
            }
            if self.show_points
                && let Some((tex, _, lo, hi)) = &self.tex_prof
            {
                painter.image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                painter.text(
                    rect.right_top() + Vec2::new(-8.0, 6.0),
                    Align2::RIGHT_TOP,
                    format!(
                        "Z {lo:.1} – {hi:.1} m · longitud {:.1} m · ancho {:.1} m · {} puntos{}{}",
                        bx.length(),
                        bx.width,
                        store.len(),
                        if store.stride > 1 {
                            format!(" (1 de cada {})", store.stride)
                        } else {
                            String::new()
                        },
                        if self.profile_loading() {
                            " · leyendo todos…"
                        } else {
                            ""
                        }
                    ),
                    FontId::proportional(11.5),
                    t.weak,
                );
            }
            if self.profile_loading() {
                painter.text(
                    rect.left_bottom() + Vec2::new(10.0, -8.0),
                    Align2::LEFT_BOTTOM,
                    "⏳ Leyendo todos los puntos de la zona; ahora se ven solo los de pantalla…",
                    FontId::proportional(12.0),
                    t.accent,
                );
            }
            if let Some((_, _, lo, hi)) = &self.tex_prof {
                let (lo, hi) = (*lo, *hi);
                self.profile_select(ctx, &painter, &resp, rect, bx, (lo, hi));
            }
        } else {
            let g = painter.layout("Herramienta Perfil: clic en el extremo izquierdo, en el derecho y en el borde de la caja".to_string(), FontId::proportional(13.0), t.weak, rect.width() - 40.0);
            painter.galley(rect.center() - g.size() / 2.0, g, t.weak);
        }
        self.frame_title(ui, rect, "Perfil");
    }

    /// Navegación 3D: izquierdo gira (sin tope), derecho y central desplazan en el plano de la
    /// pantalla (el punto sigue al cursor), rueda acerca, doble clic centra, teclas como LDV.
    fn nav_3d(&mut self, ui: &egui::Ui, resp: &egui::Response, w: usize, h: usize, b: [f64; 6]) {
        let store = self.store.clone();
        let mut moved = false;
        let (mods, hovered) = (ui.input(|i| i.modifiers), resp.hovered());
        let d = resp.drag_delta();
        let o = &mut self.orbit;
        // with a selection tool the left button draws; the middle one turns the scene
        let selecting = editing::is_select(self.nav);
        let left = resp.dragged_by(egui::PointerButton::Primary) && !selecting;
        let turn = selecting && resp.dragged_by(egui::PointerButton::Middle);
        let pan = resp.dragged_by(egui::PointerButton::Secondary)
            || (resp.dragged_by(egui::PointerButton::Middle) && !selecting)
            || (left && mods.shift);
        if (left && !pan) || turn {
            if mods.ctrl && !turn {
                o.dist = (o.dist * 1.006f64.powf(f64::from(d.y))).max(0.02);
            } else {
                o.yaw -= f64::from(d.x) * 0.008;
                o.pitch = (o.pitch + f64::from(d.y) * 0.008).clamp(-1.5533, 1.5533);
            }
            moved = true;
        }
        if pan {
            o.pan_px(f64::from(d.x), f64::from(d.y), w, h);
            moved = true;
        }
        if hovered {
            let sc = ui.input(|i| i.smooth_scroll_delta.y);
            if sc.abs() > 0.5 {
                o.dist = (o.dist * 0.9985f64.powf(f64::from(sc))).max(0.02);
                moved = true;
            }
            let step = if mods.shift { 0.15 } else { 0.03 };
            let (kx, ky, kz, home) = ui.input(|i| {
                (
                    i.key_down(Key::ArrowRight) as i8 - i.key_down(Key::ArrowLeft) as i8,
                    i.key_down(Key::ArrowUp) as i8 - i.key_down(Key::ArrowDown) as i8,
                    i.key_down(Key::PageUp) as i8 - i.key_down(Key::PageDown) as i8,
                    i.key_pressed(Key::Home),
                )
            });
            if kx != 0 || ky != 0 || kz != 0 {
                o.yaw -= f64::from(kx) * step;
                o.pitch = (o.pitch - f64::from(ky) * step).clamp(-1.5533, 1.5533);
                o.dist = (o.dist * (1.0 - f64::from(kz) * step)).max(0.02);
                moved = true;
                ui.ctx().request_repaint();
            }
            if home || (resp.double_clicked() && !selecting) {
                let keep = o.exaggeration;
                *o = match &store {
                    Some(s) => Orbit::fit_scene(s),
                    None => Orbit::fit(b),
                };
                o.exaggeration = keep;
                moved = true;
            }
        }
        if moved {
            self.touch();
        }
    }

    fn view_3d(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rect: Rect) {
        let t = self.tokens();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, color32(self.style.background));
        let resp = ui.interact(rect, ui.id().with("d3"), Sense::click_and_drag());
        let (w, h) = (
            rect.width().max(8.0) as usize,
            rect.height().max(8.0) as usize,
        );
        if let Some(store) = self.store.clone() {
            self.nav_3d(ui, &resp, w, h, store.bounds);
            let interactive = Instant::now() < self.moving_until;
            let mut st = self.style.clone();
            st.thin = if interactive && store.len() > 1_000_000 {
                store.len() / 1_000_000 + 1
            } else {
                1
            };
            st.transparent = false;
            if !self.follow_plan {
                st.window = None;
            }
            // close to the cloud the ground the camera takes in is read again at full detail
            let f = Orbit::focal(w, h);
            let px3 = self.orbit.dist / f;
            let half = w.max(h) as f64 * px3 * 0.6;
            let (tx, ty) = (self.orbit.target[0], self.orbit.target[1]);
            self.update_detail3(
                ctx,
                [tx - half, ty - half, tx + half, ty + half],
                px3,
                interactive,
            );
            let over = self
                .detail3
                .as_ref()
                .filter(|d| contains(d.window, [tx - half, ty - half, tx + half, ty + half]))
                .map(|d| (d.store.clone(), d.spacing, d.id, d.window));
            if let Some(d) = &over {
                st.hole = Some(d.3);
            }
            let o = &self.orbit;
            let key = hash_of((
                self.style_key(),
                w,
                h,
                st.thin,
                o.yaw.to_bits(),
                o.pitch.to_bits(),
                o.dist.to_bits(),
                o.exaggeration.to_bits(),
                o.target[0].to_bits(),
                o.target[1].to_bits(),
                o.target[2].to_bits(),
                (
                    st.window.map(|w| w.map(f64::to_bits)),
                    over.as_ref().map(|d| d.2),
                ),
            ));
            if self.tex_3d.as_ref().is_none_or(|t| t.1 != key) {
                let img = match &over {
                    Some((ds, spacing, _, _)) => {
                        let mut dst = st.clone();
                        dst.hole = None;
                        dst.spacing = *spacing;
                        dst.thin = if interactive && ds.len() > 1_000_000 {
                            ds.len() / 1_000_000 + 1
                        } else {
                            1
                        };
                        view::render_3d_with(&store, &st, Some((ds, &dst)), &self.orbit, w, h)
                    }
                    None => view::render_3d(&store, &self.orbit, w, h, &st),
                };
                match &mut self.tex_3d {
                    Some((tex, k)) => {
                        tex.set(
                            egui::ColorImage::from_rgba_unmultiplied([img.w, img.h], &img.rgba),
                            egui::TextureOptions::NEAREST,
                        );
                        *k = key;
                    }
                    None => {
                        self.tex_3d = Some((
                            to_texture(ctx, "d3", &img, egui::TextureOptions::NEAREST),
                            key,
                        ))
                    }
                }
            }
            if interactive {
                ctx.request_repaint_after(Duration::from_millis(170));
            }
            if self.show_points
                && let Some((tex, _)) = &self.tex_3d
            {
                painter.image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            painter.text(
                rect.right_top() + Vec2::new(-8.0, 6.0),
                Align2::RIGHT_TOP,
                if editing::is_select(self.nav) {
                    "izq.: seleccionar · central: girar · der.: mover · rueda: zoom · flechas: girar".to_string()
                } else if rect.width() > 900.0 {
                    format!(
                        "izq.: girar · der./central: mover · rueda: zoom · Ctrl+izq.: zoom · flechas: girar · doble clic/Inicio: encuadrar · exageración ×{:.1}",
                        self.orbit.exaggeration
                    )
                } else {
                    format!("izq.: girar · der.: mover · rueda: zoom · ×{:.1}", self.orbit.exaggeration)
                },
                FontId::proportional(11.0),
                t.weak,
            );
            if editing::is_select(self.nav) {
                self.view3d_select(ctx, &painter, &resp, rect, w, h);
            }
            // north arrow
            let c = rect.right_bottom() + Vec2::new(-34.0, -34.0);
            let (s, co) = self.orbit.yaw.sin_cos();
            let tip = c + Vec2::new(-(s as f32), -(co as f32)) * 18.0;
            painter.circle_stroke(c, 20.0, Stroke::new(1.0, t.weak));
            painter.line_segment([c, tip], Stroke::new(2.0, Color32::from_rgb(230, 70, 60)));
            painter.text(
                tip,
                Align2::CENTER_CENTER,
                "N",
                FontId::proportional(11.0),
                Color32::from_rgb(230, 70, 60),
            );
        } else {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Vista 3D",
                FontId::proportional(13.0),
                t.weak,
            );
        }
        self.frame_title(ui, rect, "3D");
    }

    // ----------------------------------------------------------------------------- dialogs

    fn palette_window(&mut self, ctx: &egui::Context) {
        let Some(q) = self.palette.clone() else {
            return;
        };
        let mut text = q;
        let mut chosen = None;
        let mut close = false;
        egui::Window::new("Comandos")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, [0.0, 80.0])
            .fixed_size([460.0, 0.0])
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text("Escribe un comando…")
                        .desired_width(f32::INFINITY),
                );
                r.request_focus();
                let needle = text.to_lowercase();
                let list: Vec<_> = Self::commands()
                    .into_iter()
                    .filter(|(n, _)| n.to_lowercase().contains(&needle))
                    .take(12)
                    .collect();
                for (n, a) in &list {
                    if ui.selectable_label(false, *n).clicked() {
                        chosen = Some(*a);
                    }
                }
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    chosen = list.first().map(|l| l.1);
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    close = true;
                }
            });
        if let Some(a) = chosen {
            self.palette = None;
            self.act(ctx, a);
        } else if close {
            self.palette = None;
        } else {
            self.palette = Some(text);
        }
    }

    fn class_window(&mut self, ctx: &egui::Context) {
        if !self.show_classes {
            return;
        }
        let mut open = true;
        egui::Window::new("Clases visibles y colores")
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Todas").clicked() {
                        self.style.classes = ClassSet::all();
                    }
                    if ui.button("Ninguna").clicked() {
                        self.style.classes = ClassSet::none();
                    }
                    if ui.button("Colores por defecto").clicked() {
                        self.style.class_colors = rustlidar_core::classes::default_colors();
                        self.ctable.paint(&mut self.style.class_colors);
                        self.colors_rev += 1;
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        let present = if self.store.is_some() {
                            self.class_counts
                        } else {
                            [1; 256]
                        };
                        let mut remove = None;
                        for c in self.ctable.known(&present) {
                            if self.class_counts[usize::from(c)] == 0
                                && self.store.is_some()
                                && !self.ctable.custom.contains_key(&c)
                            {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                let mut on = self.style.classes.contains(c);
                                if ui.checkbox(&mut on, "").changed() {
                                    self.style.classes.toggle(c);
                                }
                                let mut col = color32(self.style.class_colors[usize::from(c)]);
                                if ui.color_edit_button_srgba(&mut col).changed() {
                                    let k = [col.r(), col.g(), col.b()];
                                    self.style.class_colors[usize::from(c)] = k;
                                    if let Some(e) = self.ctable.custom.get_mut(&c) {
                                        e.1 = k;
                                        self.ctable.save();
                                    }
                                    self.colors_rev += 1;
                                }
                                ui.label(format!("{c} · {}", self.ctable.name(c)));
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}",
                                        self.class_counts[usize::from(c)]
                                    ))
                                    .weak(),
                                );
                                if self.ctable.custom.contains_key(&c)
                                    && ui
                                        .small_button("🗑")
                                        .on_hover_text("Quitar esta clase de la lista")
                                        .clicked()
                                {
                                    remove = Some(c);
                                }
                            });
                        }
                        if let Some(c) = remove {
                            self.ctable.remove(c, &mut self.style.class_colors);
                            self.colors_rev += 1;
                        }
                    });
                ui.separator();
                ui.label(
                    egui::RichText::new("Añadir una clase (ASPRS deja libres 64–255)").strong(),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ctable.draft.0)
                            .hint_text("código")
                            .desired_width(52.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ctable.draft.1)
                            .hint_text("nombre")
                            .desired_width(190.0),
                    );
                    if ui.button("Añadir").clicked() {
                        let (code, name) = self.ctable.draft.clone();
                        match self.ctable.add(&code, &name, &mut self.style.class_colors) {
                            Ok(c) => {
                                self.style.classes.set(c, true);
                                self.ctable.draft = Default::default();
                                self.colors_rev += 1;
                            }
                            Err(e) => self.log(e),
                        }
                    }
                });
            });
        self.show_classes = open;
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        self.edit_keys(ctx);
        self.vector_keys(ctx);
        let (ctrl, pressed): (bool, Vec<Key>) = ctx.input(|i| {
            (
                i.modifiers.command,
                [
                    Key::Q,
                    Key::W,
                    Key::Z,
                    Key::X,
                    Key::T,
                    Key::E,
                    Key::F,
                    Key::N,
                    Key::O,
                    Key::K,
                    Key::Plus,
                    Key::Equals,
                    Key::Minus,
                    Key::Escape,
                    Key::F1,
                ]
                .into_iter()
                .filter(|k| i.key_pressed(*k))
                .collect(),
            )
        });
        // + and − size the dots of the profile when the cursor is over it, else those of 2D and 3D
        let over_profile = self.profile_rect.take().is_some_and(|r| {
            ctx.input(|i| i.pointer.hover_pos())
                .is_some_and(|p| r.contains(p))
        });
        for k in pressed {
            match (k, ctrl) {
                (Key::O, true) => self.act(ctx, Action::Open),
                (Key::K, true) => self.palette = Some(String::new()),
                (Key::Q, false) => self.act(ctx, Action::Nav(Nav::Query)),
                (Key::W, false) => self.act(ctx, Action::Nav(Nav::Measure)),
                (Key::Z, false) => self.act(ctx, Action::Nav(Nav::Zoom)),
                (Key::X, false) => self.act(ctx, Action::Nav(Nav::Pan)),
                (Key::T, false) => self.act(ctx, Action::ToggleTin),
                (Key::N, false) => self.act(ctx, Action::NorthUp),
                (Key::Plus | Key::Equals, false) => self.act(
                    ctx,
                    if over_profile {
                        Action::ProfileSize(1)
                    } else {
                        Action::PointSize(1)
                    },
                ),
                (Key::Minus, false) => self.act(
                    ctx,
                    if over_profile {
                        Action::ProfileSize(-1)
                    } else {
                        Action::PointSize(-1)
                    },
                ),
                (Key::F1, _) => self.manual.open = true,
                (Key::Escape, _) => {
                    self.dig_draft.clear();
                    self.profile_clicks.clear();
                    self.measure.clear();
                }
                _ => {}
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // `--bench`: draws as fast as it can; after 6 s (the cloud is loaded) it counts the frames of
        // 8 s, prints them with the renderer and exits
        if let Some((start, frames)) = &mut self.bench {
            ctx.request_repaint();
            let t = start.elapsed().as_secs_f64();
            if t >= 6.0 {
                *frames += 1;
            }
            if t >= 14.0 {
                eprintln!(
                    "{} ({}): {} fotogramas en 8 s = {:.1} por segundo",
                    RENDERER.get().copied().unwrap_or("?"),
                    self.clouds.len(),
                    frames,
                    *frames as f64 / 8.0
                );
                std::process::exit(0);
            }
        }
        if self.theme_pending
            && let Some(theme) = ctx.system_theme()
        {
            self.theme_pending = false;
            self.set_theme(&ctx, theme != egui::Theme::Light);
        }
        self.poll(&ctx);
        self.process_edit_cmds();
        if self.ed.want_save && self.ed.session.is_some() && self.ed.cmds.is_empty() {
            self.ed.want_save = false;
            self.save_edited(&ctx);
        }
        if self.loading.is_some()
            || self.running.is_some()
            || self.info_run.is_some()
            || !self.img_jobs.is_empty()
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        // files dropped on the window
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        self.open_files(&ctx, dropped);
        self.keys(&ctx);
        self.header(ui, &ctx);
        self.ribbon(ui, &ctx);
        self.status_bar(ui);
        self.side_panel(ui, &ctx);
        self.views(ui, &ctx);
        self.histogram_window(&ctx);
        let aois = self.polygon_aois();
        let busy = self.running.is_some();
        let t = self.tokens();
        let mut log = Vec::new();
        let first_cloud = self.clouds.clone();
        if std::mem::take(&mut self.tools.s2.stop) {
            rustlidar_core::request_cancel();
            self.log("Deteniendo…");
        }
        let sel = self.selection_inputs();
        let processing = matches!(
            self.tools.open,
            Some(
                ToolId::Classify
                    | ToolId::Dtm
                    | ToolId::Chm
                    | ToolId::Metrics
                    | ToolId::Clip
                    | ToolId::Export
                    | ToolId::Thin
                    | ToolId::Dups
                    | ToolId::Hag
                    | ToolId::Compare
                    | ToolId::Fit
            )
        );
        let (inputs, pairs) = match (&sel, processing) {
            (Some(s), true) => (s.0.clone(), s.1.clone()),
            _ => (first_cloud, Vec::new()),
        };
        self.tools.set_selection_note(sel.as_ref().filter(|_| processing).map(|s| format!("Hay {} puntos seleccionados: el proceso se aplica solo a ellos (se crea {} archivo(s) «…_sel»).", s.2, s.1.len())));
        if let Some(mut l) = self.tools.window(&ctx, &t, &inputs, &aois, busy, &mut log) {
            if let (Some(s), true) = (&sel, !pairs.is_empty()) {
                let (ops, inner) = (s.3.clone(), l.task);
                let todo = pairs.clone();
                l.outputs.extend(pairs.iter().map(|p| p.1.clone()));
                l.task = Box::new(move || {
                    for (i, o) in &todo {
                        rustlidar_core::check_cancel()?;
                        rustlidar_core::edit::extract_selected(i, &ops, o)?;
                    }
                    inner()
                });
            }
            self.start(&ctx, l);
        }
        for m in log {
            self.log(m);
            self.show_log = true;
        }
        if let Some(msg) = self.notice.clone() {
            let mut close = false;
            egui::Window::new("Aviso")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size([380.0, 0.0])
                .show(&ctx, |ui| {
                    ui.label(&msg);
                    ui.add_space(6.0);
                    if ui.button("Entendido").clicked() {
                        close = true;
                    }
                });
            if close {
                self.notice = None;
            }
        }
        self.palette_window(&ctx);
        self.new_layer_window(&ctx);
        self.gpkg_window(&ctx);
        self.table_window(&ctx);
        self.rs_window(&ctx);
        self.raster_tools_window(&ctx);
        self.terrain_window(&ctx);
        self.obia_window(&ctx);
        self.align_window(&ctx);
        self.change_window(&ctx);
        self.model_window(&ctx);
        self.corridor_window(&ctx);
        if !self.ed.cmds.is_empty() || self.ed.want_save {
            ctx.request_repaint();
        }
        self.class_window(&ctx);
        if self.show_about {
            let mut open = true;
            egui::Window::new("Acerca de EasyLiDAR")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(l) = &self.logo {
                            ui.image((l.id(), Vec2::splat(72.0)));
                        }
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("EasyLiDAR")
                                    .size(26.0)
                                    .strong()
                                    .color(Color32::from_rgb(0xFF, 0x8A, 0x1F)),
                            );
                            ui.label(format!("Versión {}", env!("CARGO_PKG_VERSION")));
                        });
                    });
                    ui.add_space(6.0);
                    ui.label("EasyLiDAR · Procesamiento y edición de nubes LiDAR para forestales");
                    ui.label(
                        "Lectura en flujo: la memoria depende del área, no del número de puntos.",
                    );
                    ui.add_space(6.0);
                    ui.label("Autoría: Julio M. Sevilla");
                    ui.horizontal(|ui| {
                        ui.label("Contacto:");
                        ui.hyperlink_to("julio@jsevilla.es", "mailto:julio@jsevilla.es");
                        ui.label("·");
                        ui.hyperlink_to("LinkedIn", "https://es.linkedin.com/in/juliomsevilla");
                    });
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "Uso gratuito. Consulta LICENSE, NOTICE y THIRD_PARTY_LICENSES.md.",
                        )
                        .size(11.0)
                        .weak(),
                    );
                });
            self.show_about = open;
        }
        if self.show_shortcuts {
            let mut open = true;
            egui::Window::new("Atajos de teclado")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    egui::Grid::new("shortcuts")
                        .num_columns(2)
                        .spacing([24.0, 4.0])
                        .show(ui, |ui| {
                            for (k, d) in [
                                ("Ctrl+O", "Abrir nubes"),
                                ("Ctrl+K", "Buscar comando"),
                                ("F1", "Manual"),
                                ("Z / X", "Zoom / desplazar"),
                                ("Q / W", "Consultar / medir"),
                                ("T", "TIN sombreado"),
                                ("N", "Norte arriba (vista 3D)"),
                                ("+ / −", "Tamaño del punto"),
                                ("Flechas", "Mover el perfil"),
                                ("Clic / clic derecho", "Zoom: acerca ×2 / aleja"),
                                ("Arrastrar · rueda", "Zoom: ventana / continuo"),
                                ("Botón central", "Zoom anterior"),
                                ("Ctrl+Z / Ctrl+Y", "Deshacer / rehacer (edición)"),
                                ("Ctrl+A / Ctrl+I", "Seleccionar todo / invertir"),
                                ("Esc", "Quitar la selección"),
                                ("Supr", "Eliminar los puntos seleccionados"),
                                ("Mayús", "Selección: añadir"),
                                ("Alt", "Selección: restar"),
                                ("Ctrl", "Selección: intersecar"),
                            ] {
                                ui.label(egui::RichText::new(k).strong());
                                ui.label(d);
                                ui.end_row();
                            }
                        });
                });
            self.show_shortcuts = open;
        }
        self.manual.window(&ctx);
    }
}

/// "42 % · quedan ~ 3 min 20 s" for a job started at `started`, when it says how far it is.
fn progress_text(started: Instant) -> Option<String> {
    let f = rustlidar_core::progress::fraction()?;
    let elapsed = started.elapsed().as_secs_f64();
    // too early for a steady rate: show the share only
    if f < 0.02 || elapsed < 5.0 {
        return Some(format!("{:.0} %", f * 100.0));
    }
    let left = (elapsed * (1.0 - f) / f).round() as u64;
    let (m, s) = (left / 60, left % 60);
    let when = match (left / 3600, m % 60) {
        (0, 0) => format!("{s} s"),
        (0, m) => format!("{m} min {s:02} s"),
        (h, m) => format!("{h} h {m:02} min"),
    };
    Some(format!("{:.0} % · quedan ~ {when}", f * 100.0))
}

/// Extensions of everything the Layers dialog opens.
fn layer_extensions() -> Vec<&'static str> {
    let mut v = rustlidar_core::raster::EXTENSIONS.to_vec();
    v.extend(["shp", "geojson", "json", "gpkg"]);
    v
}
