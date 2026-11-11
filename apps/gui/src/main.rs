//! HIPSO's desktop app: a point-cloud viewer (plan, 3D and profile windows, shaded TIN,
//! contours, reference layers, POI/AOI, query, measure, density) with the processing tools of the
//! command line one click away, with a ribbon interface.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod aba_plots;
mod aba_ui;
mod align_ui;
mod rasterops_ui;
mod reproject_ui;
mod vector_ui;
mod backstage;
mod basemap;
mod brand;
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
mod vecmore;
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
use hipso_core::classes::ClassSet;
use hipso_core::grid::Grid;
use hipso_core::inspect;
use hipso_core::jobs;
use hipso_core::markup::Aoi;
use hipso_core::view::{self, ColorBy, Image, Orbit, Plan, PointStore, ProfileBox, Style};

use copcdetail::contains;
use theme::Tokens;
use tools::{Launch, ToolId, Tools};

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
/// `HIPSO_RENDERER=glow` or `wgpu` forces one, and a `WGPU_BACKEND` is left to wgpu.
fn choose_renderer() -> eframe::Renderer {
    match std::env::var("HIPSO_RENDERER").as_deref() {
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
                .set_title(brand::NAME)
                .set_description(format!(
                    "{} se ha cerrado por un error.

{info}

Si es de gráficos, prueba                      a definir la variable de entorno HIPSO_RENDERER con glow o wgpu.",
                    brand::NAME
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
            .with_icon(brand::app_icon())
            .with_title(brand::NAME),
        renderer,
        ..Default::default()
    };
    eframe::run_native(
        brand::NAME,
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

/// The ribbon tabs before the ones that depend on the selected layer: Archivo, Vista, LiDAR,
/// Vectorial and Raster. Editing points lives in the side panel, so it stays within reach of the others.
const FIRST_EXTRA: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Layers,
    Info,
    Edit,
    Symbology,
}

/// The background read of a window: what it will send, and the window.
type DetailJob = (
    Receiver<Result<Option<(PointStore, f64)>, String>>,
    [f64; 4],
    f64,
);

/// A window of COPC clouds read at full detail, and the dot spacing that goes with it.
struct Detail {
    window: [f64; 4],
    /// Map units per pixel it was read for.
    px: f64,
    store: Arc<PointStore>,
    spacing: f64,
    /// Spacing of the octree level that was read: the dots of the 3D view follow it.
    level_spacing: f64,
    id: u64,
}

/// A cloud load that finished: the points, the files they came from (converted copies replace
/// the clouds that were in another coordinate system) and what the log should say.
struct CloudLoad {
    store: PointStore,
    files: Vec<PathBuf>,
    notes: Vec<String>,
}

/// A raster prepared for the 3D view.
struct Surf3 {
    key: u64,
    grid: Grid,
    color: Image,
    shade: bool,
    bounds: [f64; 6],
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
    ZoomNext,
    ZoomSel,
    ZoomLayer,
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
    Reproject,
    RasterOp(u8),
    VectorOp(u8),
    Change,
    Model,
    Corridor,
    ClearMarks,
    ToggleTheme,
    NorthUp,
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
    /// Where the dividers between the views are, as fractions of the space: the height of the
    /// plan over the profile, the width of the plan beside the other two and the height of the
    /// profile over the 3D view.
    split: [f32; 3],
    side: Side,
    show_log: bool,
    /// A field to colour the next vector layer by, once (for screenshots).
    auto_categorize: Option<String>,
    /// The log stays closed (for screenshots).
    hide_log: bool,
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
    next_plans: Vec<Plan>,
    orbit: Orbit,
    follow_plan: bool,
    profile: Option<ProfileBox>,
    profile_clicks: Vec<[f64; 2]>,
    profile_width: f64,
    tex_plan: Option<(egui::TextureHandle, u64)>,
    tex_prof: Option<(egui::TextureHandle, u64, f64, f64)>,
    /// The profile reads the points of its region from the files, at full density.
    prof_full: bool,
    prof_grid: bool,
    prof_data: Option<profile_data::ProfData>,
    prof_job: Option<profile_data::ProfJob>,
    prof_failed: Option<u64>,
    /// Where the profile was drawn in the last frame.
    profile_rect: Option<Rect>,
    tex_3d: Option<(egui::TextureHandle, u64)>,
    /// The raster shown as a surface in the 3D view: key, heights, colours.
    surf3: Option<Surf3>,
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
    /// The layer whose editing session just ended with changes, and the tool that was on.
    edit_confirm: Option<(usize, Nav)>,
    /// The ribbon was on the vector editing tab the last time it was drawn.
    new_layer: Option<layers::NewLayerDlg>,
    /// A GeoPackage with several tables: which of them to open.
    gpkg_pick: Option<(PathBuf, Vec<(String, bool)>)>,
    table_layer: Option<usize>,
    vedit: vecedit::VecEditState,
    /// Dialogs of the vector tools: field calculator, selection by location, GeoPackage rewrite.
    vm: vecmore::State,
    /// Command line `--vsel-all`: select every feature of the active layer on start.
    select_all_pending: bool,
    /// Command line `--view x,y,scale`: the view to take once everything has loaded.
    forced_view: Option<Plan>,
    rs: rs::RsState,
    table_new_field: String,
    line_weight: f32,
    measure: Vec<[f64; 3]>,
    query: Option<usize>,
    /// What the query tool found at the point it was last used: the point and the card of text
    /// it shows, which stays until it is closed or a click finds nothing.
    query_card: Option<([f64; 3], Vec<String>)>,
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
    rp: reproject_ui::ReprojectTool,
    ro: Vec<rasterops_ui::RasterOpsTool>,
    vt: Vec<vector_ui::VectorTool>,
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
    /// The "Archivo" view is open over the work area.
    backstage: bool,
    /// What the "Inicio" view shows on its right.
    backstage_page: backstage::Page,
    /// When the log last said how far the running job is.
    last_progress_log: Instant,
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
            split: [0.64, 0.56, 0.5],
            side: Side::Layers,
            auto_categorize: None,
            hide_log: false,
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
            next_plans: Vec::new(),
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
            prof_grid: true,
            prof_data: None,
            prof_job: None,
            prof_failed: None,
            profile_rect: None,
            tex_3d: None,
            surf3: None,
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
            edit_confirm: None,
            new_layer: None,
            gpkg_pick: None,
            table_layer: None,
            vedit: vecedit::VecEditState::default(),
            vm: vecmore::State::default(),
            select_all_pending: false,
            forced_view: None,
            rs: rs::RsState::default(),
            table_new_field: String::new(),
            line_weight: 2.0,
            measure: Vec::new(),
            query: None,
            query_card: None,
            density: None,
            drag_start: None,
            cursor_map: None,
            info_text: String::new(),
            info_run: None,
            isolate: false,
            tools: Tools::default(),
            running: None,
            log: vec![format!(
                "{} listo. Abre una nube LAS/LAZ (Archivo → Abrir).",
                brand::NAME
            )],
            palette: None,
            show_classes: false,
            ctable: classtable::ClassTable::load(),
            rt: rstools::RasterTools::default(),
            tt: terrain::TerrainTool::default(),
            ot: obia_ui::ObiaTool::default(),
            at: align_ui::AlignTool::default(),
            rp: reproject_ui::ReprojectTool::default(),
            ro: rasterops_ui::tools(),
            vt: vector_ui::tools(),
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
            backstage: false,
            backstage_page: backstage::Page::Home,
            last_progress_log: Instant::now(),
            manual: manual::Manual::default(),
        }
    }
}

/// Area and perimeter of the measured points taken as a closed polygon (three points or more).
/// A small × at the top right corner of a card drawn on the map; true when it is clicked.
fn close_button(ui: &mut egui::Ui, painter: &egui::Painter, card: Rect, id: &str) -> bool {
    let r = Rect::from_center_size(card.right_top() + Vec2::new(-11.0, 11.0), Vec2::splat(16.0));
    let resp = ui.interact(r, egui::Id::new(id), Sense::click());
    let col = if resp.hovered() { Color32::WHITE } else { Color32::from_gray(170) };
    if resp.hovered() {
        painter.rect_filled(r, 3.0, Color32::from_white_alpha(40));
    }
    let c = r.center();
    for (a, b) in [((-3.5, -3.5), (3.5, 3.5)), ((-3.5, 3.5), (3.5, -3.5))] {
        painter.line_segment(
            [c + Vec2::new(a.0, a.1), c + Vec2::new(b.0, b.1)],
            Stroke::new(1.5, col),
        );
    }
    resp.on_hover_text("Cerrar").clicked()
}

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
            .or_insert_with(|| hipso_core::crs::epsg_of(path))
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
        // the theme the system uses; when it is not known yet, the first frame sets it
        if let Some(theme) = ctx.system_theme() {
            self.dark = theme != egui::Theme::Light;
            self.theme_pending = false;
        }
        let mut it = args.into_iter();
        let mut files = Vec::new();
        let mut startup_actions = Vec::new();
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
                        Some("tls") => Some(ToolId::Tls),
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
                "--exag" => {
                    if let Some(v) = it.next().and_then(|v| v.parse().ok()) {
                        self.orbit.exaggeration = v;
                    }
                }
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
                        let op = hipso_core::edit::SelOp {
                            mode: hipso_core::edit::Mode::New,
                            region: hipso_core::edit::Region::Plan(hipso_core::edit::rectangle(
                                [v[0], v[1]],
                                [v[2], v[3]],
                            )),
                            attr: hipso_core::edit::Attr::default(),
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
                "--action" => startup_actions.push(match it.next().as_deref() {
                    Some("terrain") => Action::Terrain,
                    Some("obia") => Action::Obia,
                    Some("align") => Action::Align,
                    Some("reproject") => Action::Reproject,
                    Some("rasterops") => Action::RasterOp(0),
                    Some("vector") => Action::VectorOp(0),
                    Some("change") => Action::Change,
                    Some("model") => Action::Model,
                    Some("corridor") => Action::Corridor,
                    Some("classes") => Action::Classes,
                    Some("about") => Action::About,
                    Some("manual") => Action::Manual,
                    Some("shortcuts") => Action::Shortcuts,
                    _ => continue,
                }),
                "--obia" => {
                    // the CHM to segment: runs at once
                    self.ot.open = true;
                    self.ot.chm = it.next().map(PathBuf::from);
                    self.ot.autorun = true;
                    self.ot.classify = 3;
                    self.ot.k = 5;
                    self.ot.strata = true;
                    if let Some(p) = &self.ot.chm {
                        self.ot.out = p.with_file_name("objetos").display().to_string();
                    }
                }
                "--model" => {
                    // plots table, field table and variable, separated by commas
                    let v = it.next().unwrap_or_default();
                    let v: Vec<&str> = v.split(',').collect();
                    if let [plots, field, response] = v[..] {
                        self.mo.preset(plots, field, response);
                    }
                }
                "--hide-log" => self.hide_log = true,
                "--categorize" => self.auto_categorize = it.next(),
                "--dark" => {
                    self.dark = true;
                    self.theme_pending = false;
                }
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
        for a in startup_actions {
            self.act(ctx, a);
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
            project::EXT | project::LEGACY_EXT => self.open_project(ctx, &p),
            e if hipso_core::raster::is_raster_ext(e) => {
                if !self.add_raster(ctx, &p) {
                    self.log(format!("No se pudo abrir {}", p.display()));
                }
            }
            "shp" | "geojson" | "json" | "gpkg" | "kml" | "kmz" => self.add_vector(&p),
            "e57" => {
                if self.running.is_some() {
                    self.log("Hay otro proceso en marcha: espera a que termine para abrir el E57.");
                } else {
                    // an E57 is converted to LAZ (in the results folder) and that file is opened
                    let out =
                        hipso_core::paths::unique(&hipso_core::paths::product(&p, "e57", "laz"));
                    let (src, dst) = (p.clone(), out.clone());
                    let task: tools::Task = Box::new(move || {
                        let r = hipso_core::e57::convert(&src, &dst, None)?;
                        Ok(hipso_core::e57::describe(&r, &dst))
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
                    let out =
                        hipso_core::paths::unique(&hipso_core::paths::product(&p, "ply", "laz"));
                    let (src, dst) = (p.clone(), out.clone());
                    let task: tools::Task = Box::new(move || {
                        let r = hipso_core::ply::convert(&src, &dst, None)?;
                        Ok(hipso_core::ply::describe(&r, &dst))
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

    /// The groups of the LiDAR tab: what each button shows, whether its window is open and
    /// what it does. Large buttons first, then columns of up to three small ones.
    fn lidar_groups(&self) -> Vec<ribbon::Group<Action>> {
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
                        tool("@crs", "Asignar proyección", ToolId::Crs),
                        tool("@export", "Exportar puntos", ToolId::Export),
                        tool("@clip", "Recortar con polígonos", ToolId::ClipPoly),
                    ],
                    vec![
                        tool("@thin", "Adelgazar nube", ToolId::Thin),
                        tool("@hag", "Alturas sobre suelo", ToolId::Hag),
                        tool("@dups", "Quitar duplicados", ToolId::Dups),
                    ],
                    vec![
                        tool("@colorize", "Colorear con imagen", ToolId::Colorize),
                        tool("@intensity", "Normalizar intensidad", ToolId::Intensity),
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
                    tool("@icp", "Ajustar nubes", ToolId::Fit),
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
                    other(
                        "@change",
                        "Cambio de la cubierta vegetal",
                        self.ct.open,
                        Action::Change,
                    ),
                ]],
            },
            Group {
                caption: "ALS",
                icon: "@metrics",
                big: vec![
                    tool("@plots", "Extraer\nmuestras", ToolId::Clip),
                    tool("@metrics", "Métricas\npor celda", ToolId::Metrics),
                    other(
                        "@model",
                        "Modelos\nlineales",
                        self.mo.open,
                        Action::Model,
                    ),
                ],
                menus: vec![],
                columns: vec![vec![
                    tool("@segment", "Segmentar individuos", ToolId::Trees),
                    other("@obia", "Análisis por objetos", self.ot.open, Action::Obia),
                ]],
            },
            Group {
                caption: "TLS",
                icon: "@tree",
                big: vec![tool("@tree", "Medición\nde troncos", ToolId::Tls)],
                menus: vec![],
                columns: vec![],
            },
            Group {
                caption: "Infraestructuras",
                icon: "@corridor",
                big: vec![
                    other(
                        "@corridor",
                        "Corredores\neléctricos",
                        self.co.open,
                        Action::Corridor,
                    ),
                ],
                menus: vec![],
                columns: vec![],
            },
        ]
    }

    /// The groups of the Vectorial tab: the layer, the corrections of the geometry and the
    /// geoprocesses, each of which opens its own window.
    fn vector_groups(&self) -> Vec<ribbon::Group<Action>> {
        use ribbon::{Group, Item};
        let ops = |geometry: bool| -> Vec<Item<Action>> {
            vector_ui::Op::ALL
                .iter()
                .enumerate()
                .filter(|(_, op)| op.is_geometry() == geometry)
                .map(|(k, op)| Item {
                    icon: op.icon(),
                    label: op.label(),
                    on: self.vt.get(k).is_some_and(|t| t.open),
                    action: Action::VectorOp(k as u8),
                })
                .collect()
        };
        let columns = |items: Vec<Item<Action>>| -> Vec<Vec<Item<Action>>> {
            let mut cols: Vec<Vec<Item<Action>>> = Vec::new();
            for i in items {
                match cols.last_mut() {
                    Some(c) if c.len() < 3 => c.push(i),
                    _ => cols.push(vec![i]),
                }
            }
            cols
        };
        vec![
            Group {
                caption: "Capa",
                icon: "@newlayer",
                big: vec![
                    Item {
                        icon: "@newlayer",
                        label: "Nueva\ncapa",
                        on: false,
                        action: Action::NewLayer,
                    },
                    Item {
                        icon: "@table",
                        label: "Tabla de\natributos",
                        on: self.table_layer.is_some(),
                        action: Action::LayerTable,
                    },
                ],
                menus: vec![],
                columns: vec![],
            },
            Group {
                caption: "Geometría",
                icon: "@classify",
                big: vec![],
                menus: vec![],
                columns: columns(ops(true)),
            },
            Group {
                caption: "Geoprocesos",
                icon: "@zonal",
                big: vec![],
                menus: vec![],
                columns: columns(ops(false)),
            },
        ]
    }

    /// The groups of the Ráster tab: the operations on images and the geoprocesses on rasters.
    fn raster_groups(&self) -> Vec<ribbon::Group<Action>> {
        use ribbon::{Group, Item};
        vec![
            Group {
                caption: "Operaciones",
                icon: "@rs",
                big: vec![
                    Item {
                        icon: "@rs",
                        label: "Composición\nde bandas",
                        on: false,
                        action: Action::ImageTool(0),
                    },
                    Item {
                        icon: "@bands",
                        label: "Índices\nespectrales",
                        on: self.rs.open,
                        action: Action::ImageTool(1),
                    },
                    Item {
                        icon: "@stack",
                        label: "Crear\nmultibanda",
                        on: self.rt.open == Some(0),
                        action: Action::RasterTool(0),
                    },
                    Item {
                        icon: "@mosaic",
                        label: "Mosaico",
                        on: self.rt.open == Some(1),
                        action: Action::RasterTool(1),
                    },
                ],
                menus: vec![],
                columns: vec![],
            },
            Group {
                caption: "Geoprocesos",
                icon: "@reproject",
                big: vec![
                    Item {
                        icon: "@alignrs",
                        label: "Igualar\nrásteres",
                        on: self.at.open,
                        action: Action::Align,
                    },
                    Item {
                        icon: "@reproject",
                        label: "Reproyectar",
                        on: self.rp.open,
                        action: Action::Reproject,
                    },
                ],
                menus: vec![],
                columns: vec![
                    rasterops_ui::Op::ALL
                        .iter()
                        .enumerate()
                        .map(|(k, op)| Item {
                            icon: op.icon(),
                            label: op.label(),
                            on: self.ro.get(k).is_some_and(|t| t.open),
                            action: Action::RasterOp(k as u8),
                        })
                        .collect(),
                ],
            },
        ]
    }

    /// Opens what was picked or dropped: the clouds are added together (one load), the rest one by
    /// one as layers.
    fn open_files(&mut self, ctx: &egui::Context, files: Vec<PathBuf>) {
        prefs::opened(&files);
        let mut clouds = false;
        for f in files {
            if project::is_project(&f) {
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
        hipso_core::progress::reset();
        self.last_progress_log = Instant::now();
        let max = (self.max_points_m * 1e6) as usize;
        // clouds in another system than the first one are converted while they load
        let (target, other) = hipso_core::reproject::clouds_in_other_system(&files);
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                let moved = hipso_core::reproject::project_clouds(&files);
                if moved.files.is_empty() {
                    return Err(hipso_core::Error::Param(moved.notes.join("\n")));
                }
                // a cloud that has its COPC copy in the cache is read from it: the sample comes from
                // the first levels of the octree, without decompressing the whole file
                let read: Vec<PathBuf> = moved
                    .files
                    .iter()
                    .map(|p| hipso_core::copc_cache::find(p).unwrap_or_else(|| p.clone()))
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
                hipso_core::crs::name(t)
            ));
        }
        self.run_info(ctx);
    }

    /// The report of the Información tab: the clouds, then the other layers.
    fn info_full(&mut self) -> String {
        let layers = self.layers_report();
        match (self.info_text.is_empty(), layers.is_empty()) {
            (_, true) => self.info_text.clone(),
            (true, false) => layers,
            (false, false) => format!("{}\n\n{layers}", self.info_text),
        }
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
                        && hipso_core::crs::relation(c, w) == hipso_core::crs::Relation::Different
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
                    self.conform_to_undeclared_cloud(s.bounds);
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
                    self.ed.session = Some(hipso_core::edit::Session::new(&s));
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
                .is_some_and(|n| n.to_string_lossy().ends_with("_troncos"));
            let loadable = !on_demand
                && (hipso_core::raster::is_raster_ext(&e)
                    || matches!(e.as_str(), "shp" | "geojson" | "gpkg" | "kml" | "kmz" | "las" | "laz"));
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
        self.ed.session = Some(hipso_core::edit::Session::new(&s));
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
        hipso_core::reset_cancel();
        hipso_core::progress::reset();
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

    /// Back to the previous view of the plan; the one left can be reached again with
    /// [`Action::ZoomNext`].
    fn zoom_back(&mut self) {
        if let Some(p) = self.prev_plans.pop() {
            self.next_plans.push(self.plan);
            self.plan = p;
        }
    }

    fn zoom_to(&mut self, p: Plan) {
        self.prev_plans.push(self.plan);
        self.next_plans.clear();
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
                (s.thin, s.transparent, s.fill, s.edl.to_bits()),
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
                let mut every = vec![
                    "las",
                    "laz",
                    "ply",
                    "e57",
                    "xyz",
                    "txt",
                    project::EXT,
                    project::LEGACY_EXT,
                ];
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
        self.next_plans.clear();
                self.plan_fit = true;
            }
            Action::ZoomPrev => self.zoom_back(),
            Action::ZoomNext => {
                if let Some(p) = self.next_plans.pop() {
                    self.prev_plans.push(self.plan);
                    self.plan = p;
                }
            }
            Action::ZoomSel => self.zoom_selection(),
            Action::ZoomLayer => {
                if let Some(k) = self.sel_layer.filter(|k| *k < self.refs.len()) {
                    self.zoom_to_layer(k);
                }
            }
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
                            hipso_core::crs::name(c)
                        )),
                        None => self.log("Mapa base encendido, pero hace falta abrir una nube que declare su sistema de coordenadas para colocarlo."),
                    }
                }
            }
            Action::LoadRaster => {
                for p in crate::prefs::dialog()
                    .add_filter("Ráster", hipso_core::raster::EXTENSIONS)
                    .pick_files()
                    .unwrap_or_default()
                {
                    self.add_raster(ctx, &p);
                }
            }
            Action::LoadVector => {
                for p in crate::prefs::dialog()
                    .add_filter("Capa vectorial", &["shp", "geojson", "json", "gpkg", "kml", "kmz"])
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
                        self.side = Side::Symbology;
                    }
                    None => self.log("No hay imágenes cargadas: abre una con «Abrir»."),
                }
            }
            Action::ImageTool(_) => self.rs.open = true,
            Action::RasterTool(k) => self.rt.open = Some(k),
            Action::Terrain => self.tt.open = true,
            Action::Obia => self.ot.open = true,
            Action::Align => self.at.open = true,
            Action::Reproject => self.rp.open = true,
            Action::RasterOp(k) => {
                if let Some(t) = self.ro.get_mut(usize::from(k)) {
                    t.open = true;
                }
            }
            Action::VectorOp(k) => {
                if let Some(t) = self.vt.get_mut(usize::from(k)) {
                    t.open = true;
                }
            }
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
            Action::Layout(k) => {
                self.layout = [
                    Layout::Plan,
                    Layout::PlanProfile,
                    Layout::View3d,
                    Layout::All,
                ][usize::from(k.min(3))];
            }
            Action::Classes => self.show_classes = !self.show_classes,
            Action::About => self.show_backstage(backstage::Page::About),
            Action::Shortcuts => self.show_backstage(backstage::Page::Shortcuts),
            Action::Manual => self.show_backstage(backstage::Page::Manual),
        }
    }

    fn commands() -> Vec<(&'static str, Action)> {
        let mut v = vec![
            ("Abrir (nubes, ráster y vectores)", Action::Open),
            ("Cerrar todo", Action::CloseAll),
            ("Guardar vista como PNG", Action::SaveShot),
            ("Zoom a la extensión completa", Action::ZoomFull),
            ("Zoom anterior", Action::ZoomPrev),
            ("Zoom adelante", Action::ZoomNext),
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
            ("Crear capa de puntos, líneas o polígonos", Action::NewLayer),
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
                "Reproyectar nubes y rásteres (sistema de coordenadas y alturas)",
                Action::Reproject,
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
                "Cambio de la cubierta vegetal entre dos fechas (diferencia de CHM, perturbaciones, crecimiento)",
                Action::Change,
            ),
            (
                "Modelos lineales (regresión de una variable de campo sobre las métricas LiDAR de las unidades de muestreo)",
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
            ("Medición de troncos", Action::Tool(ToolId::Tls)),
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
                        ("Comparar nubes (cambios)", Action::Tool(ToolId::Compare)),
            ("Ajustar una nube a otra", Action::Tool(ToolId::Fit)),
            ("Separación entre pasadas", Action::Tool(ToolId::Swath)),
            ("Calcular MDT", Action::Tool(ToolId::Dtm)),
            ("Calcular MDS / CHM", Action::Tool(ToolId::Chm)),
            ("Métricas por celda", Action::Tool(ToolId::Metrics)),
            ("Segmentar individuos", Action::Tool(ToolId::Trees)),
            (
                "Extraer muestras con polígonos o puntos",
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
            ("Acerca del programa", Action::About),
            ("Manual de uso (F1)", Action::Manual),
        ];
        for (k, op) in vector_ui::Op::ALL.iter().enumerate() {
            v.push((op.name(), Action::VectorOp(k as u8)));
        }
        for (k, op) in rasterops_ui::Op::ALL.iter().enumerate() {
            v.push((op.label(), Action::RasterOp(k as u8)));
        }
        v
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
                    brand::paint(ui, brand::Art::Logo, t.dark, 24.0);
                    brand::paint(ui, brand::Art::Wordmark, t.dark, 14.0);
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
                if self.tab >= FIRST_EXTRA {
                    self.tab = 0;
                }
                let names = vec!["Archivo", "Vista", "LiDAR", "Vectorial", "Raster"];
                let mut tab = self.tab;
                if ribbon::tab_strip(ui, &t, "Inicio", &names, FIRST_EXTRA, &mut tab) {
                    self.backstage = !self.backstage;
                    self.backstage_page = backstage::Page::Home;
                }
                if tab != self.tab {
                    self.backstage = false;
                }
                self.tab = tab;
            });
        if self.backstage {
            return;
        }
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
                ribbon::fold_begin(ctx, ui.available_width() - 24.0);
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| match self.tab {
                            0 => {
                                ribbon::fold_group(ui, &t, "Archivo", "@open", 1, |ui| {
                                    if ribbon::big_button(ui, &t, "@save", "Guardar\nproyecto", false)
                                        .clicked()
                                    {
                                        self.act(ctx, Action::SaveProject);
                                    }
                                    if ribbon::big_button(ui, &t, "@open", "Abrir\nproyecto", false)
                                        .clicked()
                                    {
                                        self.act(ctx, Action::OpenProject);
                                    }
                                    if ribbon::big_button(ui, &t, "@adddata", "Añadir nubes\ny capas", false)
                                        .clicked()
                                    {
                                        self.act(ctx, Action::Open);
                                    }
                                    ui.vertical(|ui| {
                                        let recent = prefs::recent();
                                        let mut reopen = None;
                                        ribbon::small_menu(
                                            ui,
                                            &t,
                                            "@recent",
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
                                        if ribbon::small_button(ui, &t, "✖", "Cerrar todo", false)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::CloseAll);
                                        }
                                    });
                                });
                                ribbon::fold_group(ui, &t, "Interfaz", "@sun", 0, |ui| {
                                    if ribbon::big_button(
                                        ui,
                                        &t,
                                        "@log",
                                        "Registro",
                                        self.show_log,
                                    )
                                    .clicked()
                                    {
                                        self.act(ctx, Action::ToggleLog);
                                    }
                                    let (icon, label) = if self.dark {
                                        ("@sun", "Tema\nclaro")
                                    } else {
                                        ("@moon", "Tema\noscuro")
                                    };
                                    if ribbon::big_button(ui, &t, icon, label, false).clicked() {
                                        self.act(ctx, Action::ToggleTheme);
                                    }
                                });
                            }
                            2 => {
                                let groups = self.lidar_groups();
                                ribbon::adaptive_groups(ui, &t, &groups, |a| self.act(ctx, a));
                                if busy {
                                    ui.spinner();
                                }
                            }
                            3 => {
                                let groups = self.vector_groups();
                                ribbon::adaptive_groups(ui, &t, &groups, |a| self.act(ctx, a));
                                if busy {
                                    ui.spinner();
                                }
                            }
                            4 => {
                                let groups = self.raster_groups();
                                ribbon::adaptive_groups(ui, &t, &groups, |a| self.act(ctx, a));
                                if busy {
                                    ui.spinner();
                                }
                            }
                            1 => {
                                ribbon::fold_group(ui, &t, "Disposición", "@layall", 5, |ui| {
                                    for (k, (icon, name, l)) in [
                                        ("@lay2d", "2D", Layout::Plan),
                                        ("@lay2dprof", "2D +\nperfil", Layout::PlanProfile),
                                        ("@lay3d", "3D", Layout::View3d),
                                        ("@layall", "2D + perfil\n+ 3D", Layout::All),
                                    ]
                                    .into_iter()
                                    .enumerate()
                                    {
                                        if ribbon::big_button(ui, &t, icon, name, self.layout == l)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::Layout(k as u8));
                                        }
                                    }
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
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label("Máx. puntos");
                                            ui.add(
                                                crate::numfield::NumField::new(&mut self.max_points_m)
                                                    .range(0.5..=200.0)
                                                    .speed(0.5)
                                                    .suffix(" M"),
                                            )
                                            .on_hover_text(format!("Millones de puntos que se cargan en pantalla; una nube mayor se decima. Pulsa Aplicar para recargar. {status}"));
                                        });
                                        if ui
                                            .button("Aplicar")
                                            .on_hover_text(format!("Vuelve a cargar las nubes con ese máximo. {status}"))
                                            .clicked()
                                            && !self.clouds.is_empty()
                                        {
                                            self.reload(ctx);
                                        }
                                    });
                                });
                                ribbon::fold_group(ui, &t, "Navegación", "✋", 4, |ui| {
                                    if ribbon::big_button(ui, &t, "✋", "Desplazar", self.nav == Nav::Pan)
                                        .clicked()
                                    {
                                        self.act(ctx, Action::Nav(Nav::Pan));
                                    }
                                    ui.vertical(|ui| {
                                        if ribbon::small_button(ui, &t, "🔍", "Zoom", self.nav == Nav::Zoom)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::Nav(Nav::Zoom));
                                        }
                                        if ribbon::small_button(ui, &t, "@query", "Consultar", self.nav == Nav::Query)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::Nav(Nav::Query));
                                        }
                                        if ribbon::small_button(ui, &t, "@clear", "Limpiar marcas", false)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::ClearMarks);
                                        }
                                    });
                                    ui.vertical(|ui| {
                                        if ribbon::small_button(ui, &t, "@full", "Extensión completa", false)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::ZoomFull);
                                        }
                                        let on_layer = self.sel_layer.is_some_and(|k| k < self.refs.len());
                                        if ribbon::small_button_on(ui, &t, "@zoomlayer", "Zoom a la capa", false, on_layer)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::ZoomLayer);
                                        }
                                        if ribbon::small_button(ui, &t, "@zoomsel", "Zoom a la selección", false)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::ZoomSel);
                                        }
                                    });
                                    ui.vertical(|ui| {
                                        if ribbon::small_button_on(ui, &t, "@prev", "Zoom anterior", false, !self.prev_plans.is_empty())
                                            .clicked()
                                        {
                                            self.act(ctx, Action::ZoomPrev);
                                        }
                                        if ribbon::small_button_on(ui, &t, "@next", "Zoom adelante", false, !self.next_plans.is_empty())
                                            .clicked()
                                        {
                                            self.act(ctx, Action::ZoomNext);
                                        }
                                        if ribbon::small_button(ui, &t, "@relink", "Refrescar 3D", false)
                                            .clicked()
                                        {
                                            self.act(ctx, Action::Relink3d);
                                        }
                                    });
                                });
                                ribbon::fold_group(ui, &t, "Herramientas", "@measure", 3, |ui| {
                                    ui.vertical(|ui| {
                                        for (icon, name, n) in [
                                            ("@density", "Densidad", Nav::Density),
                                            ("@profile", "Perfil", Nav::Profile),
                                        ] {
                                            if ribbon::small_button(ui, &t, icon, name, self.nav == n)
                                                .clicked()
                                            {
                                                self.act(ctx, Action::Nav(n));
                                            }
                                        }
                                        if ribbon::small_button(ui, &t, "@measure", "Medir", self.nav == Nav::Measure)
                                            .clicked()
                                        {
                                            let to = if self.nav == Nav::Measure {
                                                Nav::Zoom
                                            } else {
                                                Nav::Measure
                                            };
                                            self.act(ctx, Action::Nav(to));
                                        }
                                    });
                                    ui.vertical(|ui| {
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
                                                "@contour",
                                                "Curvas de nivel",
                                                self.contours_on,
                                            )
                                            .clicked()
                                            {
                                                self.act(ctx, Action::ToggleContours);
                                            }


                                    });
                                });
                                ribbon::fold_group(ui, &t, "Vista 3D", "@lay3d", 2, |ui| {
                                    if ribbon::big_button(ui, &t, "@north", "Norte\narriba", false)
                                        .clicked()
                                    {
                                        self.act(ctx, Action::NorthUp);
                                    }
                                    ribbon::compact(ui);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label("Exageración");
                                            ui.add(
                                                egui::DragValue::new(&mut self.orbit.exaggeration)
                                                    .range(0.1..=100.0)
                                                    .speed(0.05)
                                                    .prefix("× ")
                                                    .max_decimals(1),
                                            )
                                            .on_hover_text("Exageración vertical de la vista 3D (nubes y superficies ráster). Arrastra o escribe el valor.");
                                        });
                                        ui.checkbox(&mut self.follow_plan, "Enlazar con la planta")
                                            .on_hover_text("En «2D + perfil + 3D», la vista 3D sigue lo que se ve en la planta.");
                                    });
                                });
                                ribbon::fold_group(ui, &t, "Visualización", "@intensity", 1, |ui| {
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
                                        ui.horizontal(|ui| {
                                        ui.menu_button("Rango Z  ▾", |ui| {
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
                                        ui.menu_button("Filtros  ▾", |ui| {
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
                                        ui.horizontal(|ui| {
                                            ui.label("Puntos");
                                            ribbon::stepper(
                                                ui,
                                                "Puntos más pequeños en la planta y el 3D (−)",
                                                "Puntos más grandes en la planta y el 3D (+)",
                                                |d| self.act(ctx, Action::PointSize(d)),
                                            );
                                        });
                                    });
                                });
                                ribbon::fold_group(ui, &t, "Perfil", "@profile", 0, |ui| {
                                    ribbon::compact(ui);
                                    egui::Grid::new("rb_profile")
                                        .num_columns(2)
                                        .spacing([8.0, 3.0])
                                        .show(ui, |ui| {
                                            ui.label("Ancho");
                                            ui.horizontal(|ui| {
                                                ui.add(
                                                    crate::numfield::NumField::new(&mut self.profile_width)
                                                        .range(0.5..=1000.0)
                                                        .speed(0.5)
                                                        .suffix(" m"),
                                                )
                                                .on_hover_text("Ancho de la caja del perfil");
                                                ui.checkbox(&mut self.prof_grid, "Cuadrícula")
                                                    .on_hover_text("Líneas horizontales con las cotas en el lado izquierdo del perfil");
                                            });
                                            ui.end_row();
                                            ui.label("Puntos");
                                            ui.horizontal(|ui| {
                                                ribbon::stepper(
                                                    ui,
                                                    "Puntos más pequeños en el perfil (− con el cursor sobre él)",
                                                    "Puntos más grandes en el perfil (+ con el cursor sobre él)",
                                                    |d| self.act(ctx, Action::ProfileSize(d)),
                                                );
                                                ui.checkbox(&mut self.prof_full, "Sin decimar")
                                                    .on_hover_text("El perfil lee de los archivos los puntos de su caja sin decimar (hasta 6 millones), en segundo plano. Sin esto usa los mismos puntos que la planta y el 3D.");
                                            });
                                            ui.end_row();
                                        });
                                });
                            }
                            _ => {}
                        });
                    });
            });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        egui::Panel::left("side")
            .default_size(350.0)
            .size_range(300.0..=620.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(t.pane)
                    .inner_margin(egui::Margin::same(8))
                    .stroke(Stroke::new(1.0, t.border)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.side, Side::Layers, "Contenido");
                    ui.selectable_value(&mut self.side, Side::Info, "Información");
                    ui.selectable_value(&mut self.side, Side::Edit, "Edición");
                    ui.selectable_value(&mut self.side, Side::Symbology, "Simbología");
                });
                ui.separator();
                match self.side {
                    Side::Edit => self.edit_panel(ui, ctx),
                    Side::Symbology => self.symbology_panel(ui, ctx),
                    Side::Info => {
                        ui.horizontal(|ui| {
                            if ui.button("↻ Analizar de nuevo").clicked() {
                                self.run_info(ctx);
                            }
                            if ui
                                .add_enabled(
                                    !(self.info_text.is_empty() && self.refs.is_empty()),
                                    egui::Button::new("⧉ Copiar todo"),
                                )
                                .on_hover_text("Copia todo el informe al portapapeles")
                                .clicked()
                            {
                                let all = self.info_full();
                                ctx.copy_text(all);
                                self.log("Información copiada al portapapeles.");
                            }
                        });
                        let full = self.info_full();
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&full).monospace().size(11.5),
                                    )
                                    .selectable(true)
                                    .wrap_mode(egui::TextWrapMode::Extend),
                                )
                                .context_menu(|ui| {
                                    if ui.button("Copiar todo").clicked() {
                                        ctx.copy_text(full.clone());
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
            Some(code) => hipso_core::crs::name(code),
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
                            hipso_core::request_cancel();
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

    /// What "Acerca de" says: in its window and in the "Archivo" view.
    fn about_body(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            brand::paint(ui, brand::Art::Logo, self.dark, 84.0);
            ui.vertical(|ui| {
                brand::paint(ui, brand::Art::Wordmark, self.dark, 26.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(brand::CLAIM)
                            .strong()
                            .color(theme::brand_text(&self.tokens())),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
                ui.label(format!("Versión {}", env!("CARGO_PKG_VERSION")));
            });
        });
        ui.add_space(6.0);
        ui.label(brand::DESCRIPTOR);
        ui.label("Lectura en flujo: la memoria depende del área, no del número de puntos.");
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
                        let h = full.height() * self.split[0];
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
                        let w = full.width() * self.split[1];
                        let h = full.height() * self.split[2];
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
                self.dividers(ui, full);
                self.sync_views();
            });
    }

    /// The bars between the views, to drag to resize them when two or more are showing.
    fn dividers(&mut self, ui: &mut egui::Ui, full: Rect) {
        let t = self.tokens();
        let bar = |ui: &mut egui::Ui, id: &str, r: Rect, vertical: bool| -> Option<Pos2> {
            let resp = ui.interact(r, ui.id().with(id), Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(if vertical {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::ResizeVertical
                });
                ui.painter().rect_filled(r, 0.0, t.accent.gamma_multiply(0.5));
            }
            resp.dragged().then(|| resp.interact_pointer_pos()).flatten()
        };
        let frac = |v: f32, lo: f32, hi: f32| v.clamp(lo, hi);
        match self.layout {
            Layout::PlanProfile => {
                let y = full.top() + full.height() * self.split[0];
                let r = Rect::from_min_max(Pos2::new(full.left(), y - 4.0), Pos2::new(full.right(), y + 1.0));
                if let Some(p) = bar(ui, "split_h", r, false) {
                    self.split[0] = frac((p.y - full.top()) / full.height().max(1.0), 0.15, 0.85);
                }
            }
            Layout::All => {
                let x = full.left() + full.width() * self.split[1];
                let y = full.top() + full.height() * self.split[2];
                let rv = Rect::from_min_max(Pos2::new(x - 4.0, full.top()), Pos2::new(x + 1.0, full.bottom()));
                let rh = Rect::from_min_max(Pos2::new(x + 1.0, y - 4.0), Pos2::new(full.right(), y + 1.0));
                if let Some(p) = bar(ui, "split_v", rv, true) {
                    self.split[1] = frac((p.x - full.left()) / full.width().max(1.0), 0.2, 0.8);
                }
                if let Some(p) = bar(ui, "split_r", rh, false) {
                    self.split[2] = frac((p.y - full.top()) / full.height().max(1.0), 0.15, 0.85);
                }
            }
            _ => {}
        }
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
        if resp.middle_clicked() {
            self.zoom_back();
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
        self.next_plans.clear();
                    self.plan.zoom_at(
                        2.0,
                        f64::from(p.x - rect.left()),
                        f64::from(p.y - rect.top()),
                        w,
                        h,
                    );
                } else if let Some(p) = right_click {
                    self.prev_plans.push(self.plan);
        self.next_plans.clear();
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
                    self.query_card = self.query.zip(store.as_ref()).and_then(|(i, store)| {
                        (i < store.len()).then(|| {
                            let p = store.xyz(i);
                            let class = store.class[i];
                            (
                                p,
                                vec![
                                    format!("X {:.2}   Y {:.2}   Z {:.2}", p[0], p[1], p[2]),
                                    format!("Clase {class} · {}", inspect::class_name(class)),
                                    format!(
                                        "Intensidad {}   Retorno {} de {}   Fuente {}",
                                        store.intensity[i],
                                        store.ret[i] & 15,
                                        store.ret[i] >> 4,
                                        store.src[i]
                                    ),
                                ],
                            )
                        })
                    });
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
                let pos = m2s(self, l[0], l[1]) + Vec2::new(10.0, -10.0);
                let q = self.measure[self.measure.len() - 2];
                let dz = if l[2].is_nan() || q[2].is_nan() {
                    f64::NAN
                } else {
                    l[2] - q[2]
                };
                let mut lines = vec![
                    format!(
                        "Último tramo {:.2} m   Total {:.2} m",
                        (l[0] - q[0]).hypot(l[1] - q[1]),
                        total
                    ),
                    format!(
                        "3D {:.2} m   Δz {}",
                        total3,
                        if dz.is_nan() {
                            "—".into()
                        } else {
                            format!("{dz:+.2} m")
                        }
                    ),
                ];
                let area = polygon_text(&self.measure);
                if !area.is_empty() {
                    lines.push(area.trim_start_matches(|c: char| c == ' ' || c == '|').trim().to_string());
                }
                let size = Vec2::new(
                    lines
                        .iter()
                        .map(|l| painter.layout_no_wrap(l.clone(), FontId::proportional(12.0), Color32::WHITE).size().x)
                        .fold(0.0, f32::max)
                        + 16.0,
                    lines.len() as f32 * 17.0 + 10.0,
                );
                // beside the last point, kept inside the view
                let mut at = pos - Vec2::new(0.0, size.y);
                at.x = at.x.min(rect.right() - size.x - 6.0).max(rect.left() + 6.0);
                at.y = at.y.max(rect.top() + 6.0).min(rect.bottom() - size.y - 6.0);
                let card = Rect::from_min_size(at, size);
                painter.rect_filled(card, 4.0, Color32::from_black_alpha(190));
                for (k, l) in lines.iter().enumerate() {
                    painter.text(
                        card.min + Vec2::new(8.0, 5.0 + k as f32 * 17.0),
                        Align2::LEFT_TOP,
                        l,
                        FontId::proportional(12.0),
                        Color32::from_rgb(120, 255, 120),
                    );
                }
            }
        }
        if let Some((p, lines)) = self.query_card.clone() {
            let c = m2s(self, p[0], p[1]);
            painter.circle_stroke(c, 7.0, Stroke::new(2.0, Color32::WHITE));
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
            if close_button(ui, &painter, box_rect, "query_card_x") {
                self.query_card = None;
                self.query = None;
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
        if let Some(b) = self.paint_ident(&painter, rect)
            && close_button(ui, &painter, b, "ident_card_x")
        {
            self.ident.clear();
        }
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

    /// Each visible raster layer sampled along the profile axis: `(name, [[along, z], ..])`,
    /// with a NaN z where the raster has no data.
    // ponytail: samples the displayed grid, which `add_grid` decimates above 4096 px a side;
    // sample the source file if that loses too much detail.
    fn terrain_profile(&self, bx: &view::ProfileBox) -> Vec<(String, Vec<[f64; 2]>)> {
        let len = bx.length();
        if len <= 0.0 {
            return Vec::new();
        }
        let (ux, uy) = ((bx.b[0] - bx.a[0]) / len, (bx.b[1] - bx.a[1]) / len);
        self.refs
            .iter()
            .filter(|r| r.visible)
            .filter_map(|r| match &r.kind {
                RefKind::Raster { grid, .. } => Some((&r.name, grid)),
                _ => None,
            })
            .filter_map(|(name, g)| {
                let n = ((len / (g.cell / 2.0)).ceil() as usize).clamp(2, 4000);
                let v: Vec<[f64; 2]> = (0..=n)
                    .map(|i| {
                        let u = len * i as f64 / n as f64;
                        let z = g
                            .sample(bx.a[0] + ux * u, bx.a[1] + uy * u)
                            .map_or(f64::NAN, f64::from);
                        [u, z]
                    })
                    .collect();
                v.iter().any(|p| p[1].is_finite()).then(|| (name.clone(), v))
            })
            .collect()
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
            let terrain = self.terrain_profile(&bx);
            let extra = terrain
                .iter()
                .flat_map(|(_, v)| v.iter().map(|p| p[1]).filter(|z| z.is_finite()))
                .fold(None, |a: Option<(f64, f64)>, z| {
                    Some(a.map_or((z, z), |a| (a.0.min(z), a.1.max(z))))
                });
            let key = hash_of((
                extra.map(|e| (e.0.to_bits(), e.1.to_bits())),
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
                let (img, lo, hi) = view::render_profile(&store, &bx, w, h, &self.style, extra);
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
            if let Some((_, _, lo, hi)) = &self.tex_prof {
                // same transform as `render_profile`, from image pixels to the screen rect
                let (lo, hi, len) = (*lo, *hi, bx.length().max(1e-9));
                let k = (w as f64 / len).min(h as f64 / (hi - lo));
                let (ox, oy) = (
                    (w as f64 - len * k) / 2.0,
                    (h as f64 - (hi - lo) * k) / 2.0,
                );
                let at = |p: &[f64; 2]| {
                    Pos2::new(
                        rect.min.x + ((ox + p[0] * k) * f64::from(rect.width()) / w as f64) as f32,
                        rect.min.y
                            + ((h as f64 - oy - (p[1] - lo) * k) * f64::from(rect.height())
                                / h as f64) as f32,
                    )
                };
                if self.prof_grid {
                    let step = aba_plots::nice_step(hi - lo, 6.0);
                    let line = Stroke::new(1.0, t.weak.gamma_multiply(0.35));
                    let mut z = (lo / step).ceil() * step;
                    while z <= hi {
                        let y = at(&[0.0, z]).y;
                        painter.line_segment(
                            [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
                            line,
                        );
                        painter.text(
                            Pos2::new(rect.min.x + 4.0, y - 1.0),
                            Align2::LEFT_BOTTOM,
                            aba_plots::tick_text(z, step),
                            FontId::proportional(11.0),
                            t.weak,
                        );
                        z += step;
                    }
                }
                const COLORS: [Color32; 4] = [
                    Color32::from_rgb(255, 160, 0),
                    Color32::from_rgb(0, 200, 255),
                    Color32::from_rgb(255, 80, 200),
                    Color32::from_rgb(120, 255, 80),
                ];
                for (i, (name, v)) in terrain.iter().enumerate() {
                    let c = COLORS[i % COLORS.len()];
                    for s in v.windows(2) {
                        if s[0][1].is_finite() && s[1][1].is_finite() {
                            painter.line_segment([at(&s[0]), at(&s[1])], Stroke::new(2.0, c));
                        }
                    }
                    painter.text(
                        rect.right_bottom() + Vec2::new(-8.0, -6.0 - 15.0 * i as f32),
                        Align2::RIGHT_BOTTOM,
                        format!("— {name}"),
                        FontId::proportional(12.0),
                        c,
                    );
                }
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
                    None => {
                        // a raster alone: closer, it is a flat sheet
                        let mut f = Orbit::fit(b);
                        f.dist *= 0.75;
                        f
                    }
                };
                o.exaggeration = keep;
                moved = true;
            }
        }
        if moved {
            self.touch();
        }
    }

    /// The raster the 3D view shows as a surface: the selected one, else the first visible
    /// (a one-band raster, or a one-band image whose reduced copy gives the heights).
    fn surface_layer(&self) -> Option<usize> {
        let ok = |r: &RefLayer| {
            r.visible
                && match &r.kind {
                    RefKind::Raster { .. } => true,
                    RefKind::Image(l) => l.work.bands.len() == 1,
                    RefKind::Vector(_) => false,
                }
        };
        self.sel_layer
            .filter(|k| self.refs.get(*k).is_some_and(ok))
            .or_else(|| self.refs.iter().position(ok))
    }

    /// Heights and colours of layer `k` for the 3D view, made again only when it changes.
    fn prepare_surface(&mut self, k: usize) {
        let r = &self.refs[k];
        let key = match &r.kind {
            RefKind::Raster { grid, style, .. } => hash_of((
                k,
                &r.path,
                grid.cols,
                grid.rows,
                (style.mode as u8, style.ramp, style.invert, style.classes),
                (style.lo.to_bits(), style.hi.to_bits()),
                (
                    style.azimuth.to_bits(),
                    style.altitude.to_bits(),
                    style.zfactor.to_bits(),
                    style.blend.to_bits(),
                ),
            )),
            _ => hash_of((k, &r.path)),
        };
        if self.surf3.as_ref().is_some_and(|s| s.key == key) {
            return;
        }
        let (grid, style) = match &r.kind {
            RefKind::Raster { grid, style, .. } => (grid.clone(), style.clone()),
            RefKind::Image(l) => {
                let g = Grid {
                    x0: l.work.x0,
                    y0: l.work.y0,
                    cell: l.work.cell,
                    cols: l.work.cols,
                    rows: l.work.rows,
                    data: l.work.bands[0].clone(),
                };
                let st = rs::GridStyle::new(&g);
                (g, st)
            }
            RefKind::Vector(_) => return,
        };
        let shade = style.mode != rs::GridMode::Hillshade;
        // empty cells stay transparent: the surface skips them
        let color = rs::render_grid(&grid, &style);
        let bounds = view::Surface {
            grid: &grid,
            color: &color,
            max_quads: 1,
            shade,
        }
        .bounds();
        let first = self.surf3.is_none() || self.store.is_none();
        self.surf3 = Some(Surf3 {
            key,
            grid,
            color,
            shade,
            bounds,
        });
        if first && self.store.is_none() {
            let keep = self.orbit.exaggeration;
            self.orbit = Orbit::fit(bounds);
            self.orbit.dist *= 0.75;
            self.orbit.exaggeration = keep;
        }
        self.tex_3d = None;
    }

    fn view_3d(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, rect: Rect) {
        let surf_layer = self.surface_layer();
        match surf_layer {
            Some(k) => self.prepare_surface(k),
            None => self.surf3 = None,
        }
        let t = self.tokens();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, color32(self.style.background));
        let resp = ui.interact(rect, ui.id().with("d3"), Sense::click_and_drag());
        let (w, h) = (
            rect.width().max(8.0) as usize,
            rect.height().max(8.0) as usize,
        );
        // with the cloud hidden and a raster shown, only the surface is drawn
        let has_cloud = self.store.is_some() && (self.show_points || self.surf3.is_none());
        let store = if has_cloud {
            self.store.clone()
        } else {
            self.surf3.as_ref().map(|_| Arc::new(PointStore::default()))
        };
        if let Some(store) = store {
            let nav_bounds = match (&self.surf3, has_cloud) {
                (Some(s), false) => s.bounds,
                _ => store.bounds,
            };
            self.nav_3d(ui, &resp, w, h, nav_bounds);
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
            if has_cloud {
                self.update_detail3(
                    ctx,
                    [tx - half, ty - half, tx + half, ty + half],
                    px3,
                    interactive,
                );
            }
            let over = self
                .detail3
                .as_ref()
                .filter(|_| has_cloud)
                .filter(|d| contains(d.window, [tx - half, ty - half, tx + half, ty + half]))
                .map(|d| {
                    (
                        d.store.clone(),
                        d.level_spacing.max(d.spacing),
                        d.id,
                        d.window,
                    )
                });
            if let Some(d) = &over {
                st.hole = Some(d.3);
            }
            let o = &self.orbit;
            let key = hash_of((
                self.style_key(),
                w,
                h,
                (st.thin, interactive, self.surf3.as_ref().map(|s| s.key), has_cloud),
                (o.yaw.to_bits(), o.pitch.to_bits(), o.dist.to_bits()),
                o.exaggeration.to_bits(),
                (
                    o.target[0].to_bits(),
                    o.target[1].to_bits(),
                    o.target[2].to_bits(),
                ),
                (
                    st.window.map(|w| w.map(f64::to_bits)),
                    over.as_ref().map(|d| d.2),
                ),
            ));
            if self.tex_3d.as_ref().is_none_or(|t| t.1 != key) {
                // a moving view draws fewer quads: the surface stays smooth to drag
                let quads = if interactive { 1_000_000 } else { 6_000_000 };
                let surface = self.surf3.as_ref().map(|s| view::Surface {
                    grid: &s.grid,
                    color: &s.color,
                    max_quads: quads,
                    shade: s.shade,
                });
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
                        view::render_3d_surface(
                            &store,
                            &st,
                            Some((ds, &dst)),
                            surface.as_ref(),
                            &self.orbit,
                            w,
                            h,
                        )
                    }
                    None => view::render_3d_surface(
                        &store,
                        &st,
                        None,
                        surface.as_ref(),
                        &self.orbit,
                        w,
                        h,
                    ),
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
            if (self.show_points || self.surf3.is_some())
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
            if has_cloud && editing::is_select(self.nav) {
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
                        self.style.class_colors = hipso_core::classes::default_colors();
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
                (Key::F1, _) => self.show_backstage(backstage::Page::Manual),
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
        // the drawing of the measure tool goes away with the tool
        if self.nav != Nav::Measure && !self.measure.is_empty() {
            self.measure.clear();
        }
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
        if self.hide_log {
            self.show_log = false;
        }
        if let Some(field) = self.auto_categorize.clone()
            && let Some(r) = self.refs.last_mut()
            && let RefKind::Vector(v) = &mut r.kind
            && let Some(c) = v.data.fields.iter().position(|f| *f == field)
        {
            self.auto_categorize = None;
            v.sym.kind = symbology::SymKind::Unique;
            v.sym.field = Some(c);
            v.sym.classify(&v.data);
            v.fill_alpha = 0.45;
        }
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
        if self.backstage {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.backstage = false;
            }
            self.backstage_view(ui, &ctx);
        } else {
            self.side_panel(ui, &ctx);
            self.views(ui, &ctx);
        }
        self.histogram_window(&ctx);
        let aois = self.polygon_aois();
        let busy = self.running.is_some();
        let t = self.tokens();
        let mut log = Vec::new();
        let first_cloud = self.clouds.clone();
        if std::mem::take(&mut self.tools.s2.stop) {
            hipso_core::request_cancel();
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
        self.tools.vectors = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Vector(_)))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        if let Some(mut l) = self.tools.window(&ctx, &t, &inputs, &aois, busy, &mut log) {
            if let (Some(s), true) = (&sel, !pairs.is_empty()) {
                let (ops, inner) = (s.3.clone(), l.task);
                let todo = pairs.clone();
                l.outputs.extend(pairs.iter().map(|p| p.1.clone()));
                l.task = Box::new(move || {
                    for (i, o) in &todo {
                        hipso_core::check_cancel()?;
                        hipso_core::edit::extract_selected(i, &ops, o)?;
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
        self.edit_confirm_window(&ctx);
        self.vecmore_windows(&ctx);
        self.gpkg_window(&ctx);
        self.table_window(&ctx);
        self.rs_window(&ctx);
        self.raster_tools_window(&ctx);
        self.terrain_window(&ctx);
        self.obia_window(&ctx);
        self.align_window(&ctx);
        self.reproject_window(&ctx);
        self.rasterops_windows(&ctx);
        self.vector_windows(&ctx);
        self.change_window(&ctx);
        self.model_window(&ctx);
        self.corridor_window(&ctx);
        if !self.ed.cmds.is_empty() || self.ed.want_save {
            ctx.request_repaint();
        }
        self.class_window(&ctx);
    }
}

/// "42 % · quedan ~ 3 min 20 s" for a job started at `started`, when it says how far it is.
fn progress_text(started: Instant) -> Option<String> {
    let f = hipso_core::progress::fraction()?;
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
    let mut v = hipso_core::raster::EXTENSIONS.to_vec();
    v.extend(["shp", "geojson", "json", "gpkg", "kml", "kmz"]);
    v
}
