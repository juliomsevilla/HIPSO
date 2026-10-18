//! EasyLiDAR's desktop app: a point-cloud viewer (plan, 3D and profile windows, shaded TIN,
//! contours, reference layers, POI/AOI, query, measure, density) with the processing tools of the
//! command line one click away, with a ribbon interface.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod classtable;
mod editing;
mod layer_tabs;
mod layers;
mod manual;
mod ribbon;
mod rs;
mod rstools;
mod symbology;
mod theme;
mod tools;
mod vecedit;

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

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(app_icon())
            .with_title("EasyLiDAR"),
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Layers,
    Info,
    Edit,
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
    OpenClouds,
    AddClouds,
    CloseAll,
    SaveShot,
    ZoomFull,
    ZoomPrev,
    Nav(Nav),
    Tool(ToolId),
    Info,
    Color(ColorBy),
    PointSize(i32),
    ToggleTin,
    ToggleContours,
    ToggleLog,
    LoadRaster,
    LoadVector,
    LoadLayers,
    NewLayer,
    SaveLayer,
    LayerTable,
    ImageTool(u8),
    RasterTool(u8),
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
    tab: usize,
    nav: Nav,
    layout: Layout,
    side: Side,
    show_log: bool,
    clouds: Vec<PathBuf>,
    store: Option<Arc<PointStore>>,
    rev: u64,
    loading: Option<(Receiver<Result<PointStore, String>>, Instant)>,
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
    tex_3d: Option<(egui::TextureHandle, u64)>,
    moving_until: Instant,
    refs: Vec<RefLayer>,
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
    tools: Tools,
    running: Option<Running>,
    log: Vec<String>,
    palette: Option<String>,
    show_classes: bool,
    ctable: classtable::ClassTable,
    rt: rstools::RasterTools,
    show_about: bool,
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
            tab: 0,
            nav: Nav::Zoom,
            layout: Layout::Plan,
            side: Side::Layers,
            show_log: false,
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
            tex_3d: None,
            moving_until: Instant::now(),
            refs: Vec::new(),
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
            tools: Tools::default(),
            running: None,
            log: vec!["EasyLiDAR listo. Abre una nube LAS/LAZ (Inicio → Abrir).".into()],
            palette: None,
            show_classes: false,
            ctable: classtable::ClassTable::load(),
            rt: rstools::RasterTools::default(),
            show_about: false,
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
    fn tokens(&self) -> Tokens {
        if self.dark { theme::DARK } else { theme::LIGHT }
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
                "--table" => self.table_layer = Some(0),
                "--ndvi" => self.rs.compute_at_start = true,
                "--newlayer" => self.new_layer = Some(layers::NewLayerDlg::default()),
                "--tin" => {
                    self.tin_on = true;
                    self.show_points = false;
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
                "--light" => self.dark = false,
                "--side-info" => self.side = Side::Info,
                "--side-edit" => self.side = Side::Edit,
                _ => files.push(PathBuf::from(a)),
            }
        }
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
            "tif" | "tiff" | "asc" => {
                if !self.add_raster(ctx, &p) {
                    self.log(format!("No se pudo abrir {}", p.display()));
                }
            }
            "shp" => self.add_vector(&p),
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

    fn reload(&mut self, ctx: &egui::Context) {
        let files = self.clouds.clone();
        let max = (self.max_points_m * 1e6) as usize;
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| view::load(&files, max))
                .map_err(|_| "error interno al leer".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.loading = Some((rx, Instant::now()));
        self.log(format!("Cargando {} archivo(s)…", self.clouds.len()));
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
        self.refs.retain(|r| r.path != path);
        self.refs.push(RefLayer {
            name,
            path,
            kind: RefKind::Raster {
                grid: shown,
                tex,
                style,
            },
            visible: true,
            opacity: 1.0,
        });
        self.sel_layer = Some(self.refs.len() - 1);
        if self.store.is_none() {
            self.plan_fit = true;
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_images(ctx);
        if let Some((rx, started)) = &self.loading
            && let Ok(r) = rx.try_recv()
        {
            let secs = started.elapsed().as_secs_f64();
            self.loading = None;
            match r {
                Ok(s) => {
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
                    st.color_by = if s.rgb.is_empty() {
                        self.style.color_by
                    } else {
                        ColorBy::Rgb
                    };
                    st.point_size = self.style.point_size;
                    st.classes = self.style.classes;
                    st.returns = self.style.returns;
                    st.class_colors = self.style.class_colors;
                    self.style = st;
                    self.plan_fit = true;
                    self.orbit = Orbit {
                        yaw: self.orbit.yaw,
                        pitch: self.orbit.pitch,
                        exaggeration: self.orbit.exaggeration,
                        ..Orbit::fit(s.bounds)
                    };
                    self.tin = None;
                    self.query = None;
                    let old = std::mem::take(&mut self.ed);
                    self.ed = editing::Editing {
                        cmds: old.cmds,
                        want_save: old.want_save,
                        auto_save: old.auto_save,
                        mode: old.mode,
                        class: old.class,
                        ..editing::Editing::default()
                    };
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
            self.running = None;
            match result {
                Ok(summary) => {
                    self.log(format!("{label}: {summary}\nhecho en {secs:.2} s"));
                    for f in show {
                        if f.is_file() {
                            self.open_path(ctx, f, false);
                        }
                    }
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

    fn start(&mut self, ctx: &egui::Context, l: Launch) {
        rustlidar_core::reset_cancel();
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
            s.profile_scale.to_bits(),
            (
                s.color_by as u8,
                s.zmin.to_bits(),
                s.zmax.to_bits(),
                s.intensity_max,
                s.point_size,
                s.classes,
                s.returns,
                s.thin,
                s.transparent,
                self.colors_rev,
                self.rev,
                self.ed.sel_rev,
            ),
        ))
    }

    fn act(&mut self, ctx: &egui::Context, a: Action) {
        match a {
            Action::OpenClouds => {
                let files = rfd::FileDialog::new()
                    .add_filter("Nubes de puntos", &["las", "laz", "xyz", "txt"])
                    .pick_files()
                    .unwrap_or_default();
                if !files.is_empty() {
                    self.clouds = files;
                    self.reload(ctx);
                }
            }
            Action::AddClouds => {
                let files = rfd::FileDialog::new()
                    .add_filter("Nubes de puntos", &["las", "laz", "xyz", "txt"])
                    .pick_files()
                    .unwrap_or_default();
                if !files.is_empty() {
                    for f in files {
                        if !self.clouds.contains(&f) {
                            self.clouds.push(f);
                        }
                    }
                    self.reload(ctx);
                }
            }
            Action::CloseAll => {
                self.clouds.clear();
                self.store = None;
                self.refs.clear();
                self.tin = None;
                self.profile = None;
                self.info_text.clear();
                self.rev += 1;
            }
            Action::SaveShot => {
                if let (Some(s), Some(p)) = (
                    self.store.clone(),
                    rfd::FileDialog::new()
                        .add_filter("PNG", &["png"])
                        .set_file_name("vista.png")
                        .save_file(),
                ) {
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
            Action::Nav(n) => {
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
                self.style.profile_scale =
                    (self.style.profile_scale * 1.3f64.powi(d)).clamp(0.05, 4.0);
                self.style.point_size = (self.style.point_size as i32 + d).clamp(1, 12) as usize
            }
            Action::ToggleTin => {
                self.tin_on = !self.tin_on;
                if self.tin_on {
                    self.show_points = false;
                }
            }
            Action::ToggleContours => self.contours_on = !self.contours_on,
            Action::ToggleLog => self.show_log = !self.show_log,
            Action::LoadLayers => {
                let files = rfd::FileDialog::new()
                    .add_filter(
                        "Capas (ráster y vectoriales)",
                        &["tif", "tiff", "asc", "shp"],
                    )
                    .pick_files()
                    .unwrap_or_default();
                for f in files {
                    self.open_path(ctx, f, false);
                }
            }
            Action::LoadRaster => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Ráster", &["tif", "tiff", "asc"])
                    .pick_file()
                {
                    self.add_raster(ctx, &p);
                }
            }
            Action::LoadVector => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Shapefile", &["shp"])
                    .pick_file()
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
                        self.tab = 5;
                    }
                    None => self.log("No hay imágenes cargadas: abre una con «Cargar capas»."),
                }
            }
            Action::ImageTool(_) => self.rs.open = true,
            Action::RasterTool(k) => self.rt.open = Some(k),
            Action::ClearMarks => {
                self.measure.clear();
                self.density = None;
                self.query = None;
                self.profile = None;
                self.profile_clicks.clear();
                self.dig_draft.clear();
            }
            Action::ToggleTheme => {
                self.dark = !self.dark;
                theme::apply(ctx, &self.tokens());
                self.style.background = if self.dark {
                    [18, 21, 26]
                } else {
                    [221, 225, 231]
                };
                self.colors_rev += 1;
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
            Action::Manual => self.manual.open = true,
        }
    }

    fn commands() -> Vec<(&'static str, Action)> {
        vec![
            ("Abrir nubes de puntos", Action::OpenClouds),
            ("Añadir nubes", Action::AddClouds),
            ("Cerrar todo", Action::CloseAll),
            ("Guardar vista como PNG", Action::SaveShot),
            ("Zoom a la extensión completa", Action::ZoomFull),
            ("Zoom anterior", Action::ZoomPrev),
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
            ("Cubicar árboles (TLS)", Action::Tool(ToolId::Tls)),
            ("Quitar puntos duplicados", Action::Tool(ToolId::Dups)),
            ("Comparar nubes (cambios)", Action::Tool(ToolId::Compare)),
            ("Ajustar una nube a otra", Action::Tool(ToolId::Fit)),
            ("Separación entre pasadas", Action::Tool(ToolId::Swath)),
            ("Calcular MDT", Action::Tool(ToolId::Dtm)),
            ("Calcular CHM", Action::Tool(ToolId::Chm)),
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
            ("Cargar capas (ráster y shapefile)", Action::LoadLayers),
            ("Cargar ráster de fondo", Action::LoadRaster),
            ("Cargar shapefile de referencia", Action::LoadVector),
            ("Limpiar mediciones y perfil", Action::ClearMarks),
            ("Cambiar tema claro / oscuro", Action::ToggleTheme),
            ("Clases visibles y colores", Action::Classes),
            ("Mostrar u ocultar el registro", Action::ToggleLog),
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
                        ("@open", "Abrir nubes (Ctrl+O)", Action::OpenClouds),
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
                if self.tab >= 5 + extra.len() {
                    self.tab = 0;
                }
                let mut names = vec!["Inicio", "Vista", "Edición", "Procesar", "Ayuda"];
                names.extend_from_slice(extra);
                let mut tab = self.tab;
                if ribbon::tab_strip(ui, &t, "Archivo", &names, 5, &mut tab) {
                    self.act(ctx, Action::OpenClouds);
                }
                self.tab = tab;
            });
        egui::Panel::top("ribbon").exact_size(108.0).frame(egui::Frame::new().fill(t.ribbon).inner_margin(egui::Margin::symmetric(10, 6)).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
            let busy = self.running.is_some();
            egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
ui.horizontal(|ui| match self.tab {
                0 => {
                    ribbon::group(ui, &t, "Archivo", |ui| {
                        if ribbon::big_button(ui, &t, "@open", "Abrir", false).clicked() {
                            self.act(ctx, Action::OpenClouds);
                        }
                        if ribbon::big_button(ui, &t, "➕", "Añadir", false).clicked() {
                            self.act(ctx, Action::AddClouds);
                        }
                        ui.vertical(|ui| {
                            if ribbon::small_button(ui, &t, "ℹ", "Información", false).clicked() {
                                self.act(ctx, Action::Info);
                            }
                            if ribbon::small_button(ui, &t, "🖼", "Guardar vista", false).clicked() {
                                self.act(ctx, Action::SaveShot);
                            }
                            if ribbon::small_button(ui, &t, "✖", "Cerrar todo", false).clicked() {
                                self.act(ctx, Action::CloseAll);
                            }
                        });
                    });
                    ribbon::group(ui, &t, "Capas", |ui| {
                        if ribbon::big_button(ui, &t, "🗺", "Cargar\ncapas", false).clicked() {
                            self.act(ctx, Action::LoadLayers);
                        }
                        if ribbon::big_button(ui, &t, "@newlayer", "Nueva\ncapa", false).clicked() {
                            self.act(ctx, Action::NewLayer);
                        }
                        if ribbon::big_button(ui, &t, "@table", "Tabla de\natributos", self.table_layer.is_some()).clicked() {
                            self.act(ctx, Action::LayerTable);
                        }
                        ui.vertical(|ui| {
                            if ribbon::small_button(ui, &t, "@pencil", "Digitalizar", self.nav == Nav::Digitize).clicked() {
                                self.act(ctx, Action::Nav(Nav::Digitize));
                            }
                            if ribbon::small_button(ui, &t, "@save", "Guardar capa", false).clicked() {
                                self.act(ctx, Action::SaveLayer);
                            }
                        });
                    });
                    ribbon::group(ui, &t, "Teledetección", |ui| {
                        if ribbon::big_button(ui, &t, "@rs", "Composición\nde bandas", false).clicked() {
                            self.act(ctx, Action::ImageTool(0));
                        }
                        if ribbon::big_button(ui, &t, "@bands", "Índices\nespectrales", self.rs.open).clicked() {
                            self.act(ctx, Action::ImageTool(1));
                        }
                        ui.vertical(|ui| {
                            if ribbon::small_button(ui, &t, "@stack", "Crear multibanda", self.rt.open == Some(0)).clicked() {
                                self.act(ctx, Action::RasterTool(0));
                            }
                            if ribbon::small_button(ui, &t, "@mosaic", "Mosaico", self.rt.open == Some(1)).clicked() {
                                self.act(ctx, Action::RasterTool(1));
                            }
                        });
                    });
                    ribbon::group(ui, &t, "Aspecto", |ui| {
                        if ribbon::big_button(ui, &t, if self.dark { "☀" } else { "🌙" }, "Tema", false).clicked() {
                            self.act(ctx, Action::ToggleTheme);
                        }
                        if ribbon::big_button(ui, &t, "📜", "Registro", self.show_log).clicked() {
                            self.act(ctx, Action::ToggleLog);
                        }
                    });
                }
                2 => self.edit_ribbon(ui, &t),
                3 => {
                    ribbon::group(ui, &t, "Preparar", |ui| {
                        if ribbon::big_button(ui, &t, "ℹ", "Información\nde los datos", false).clicked() {
                            self.act(ctx, Action::Info);
                        }
                        if ribbon::big_button(ui, &t, "@crs", "Asignar
proyección", self.tools.open == Some(ToolId::Crs)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Crs));
                        }
                        if ribbon::big_button(ui, &t, "@thin", "Adelgazar
nube", self.tools.open == Some(ToolId::Thin)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Thin));
                        }
                        if ribbon::big_button(ui, &t, "@classify", "Clasificar\nruido / suelo", self.tools.open == Some(ToolId::Classify)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Classify));
                        }
                    });
                    ribbon::group(ui, &t, "Superficies", |ui| {
                        if ribbon::big_button(ui, &t, "@dtm", "MDT", self.tools.open == Some(ToolId::Dtm)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Dtm));
                        }
                        if ribbon::big_button(ui, &t, "@chm", "CHM", self.tools.open == Some(ToolId::Chm)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Chm));
                        }
                    });
                    ribbon::group(ui, &t, "Inventario", |ui| {
                        if ribbon::big_button(ui, &t, "📊", "Métricas\nde rodal", self.tools.open == Some(ToolId::Metrics)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Metrics));
                        }
                        if ribbon::big_button(ui, &t, "🌲", "Segmentar\nárboles", self.tools.open == Some(ToolId::Trees)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Trees));
                        }
                    });
                    ribbon::group(ui, &t, "Recorte y salida", |ui| {
                        if ribbon::big_button(ui, &t, "✂", "Recortar\nparcelas", self.tools.open == Some(ToolId::Clip)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Clip));
                        }
                        if ribbon::big_button(ui, &t, "📤", "Exportar\npuntos", self.tools.open == Some(ToolId::Export)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Export));
                        }
                    });
                    ribbon::group(ui, &t, "Calidad y comparación", |ui| {
                        for (icon, name, id) in [
                            ("@dups", "Quitar\nduplicados", ToolId::Dups),
                            ("@compare", "Comparar\nnubes", ToolId::Compare),
                            ("@fit", "Ajustar\nnubes", ToolId::Fit),
                            ("@swath", "Entre\npasadas", ToolId::Swath),
                        ] {
                            if ribbon::big_button(ui, &t, icon, name, self.tools.open == Some(id)).clicked() {
                                self.act(ctx, Action::Tool(id));
                            }
                        }
                    });
                    ribbon::group(ui, &t, "TLS", |ui| {
                        if ribbon::big_button(ui, &t, "@tree", "Cubicar\nárboles", self.tools.open == Some(ToolId::Tls)).clicked() {
                            self.act(ctx, Action::Tool(ToolId::Tls));
                        }
                    });
                    if busy {
                        ui.spinner();
                    }
                }
                1 => {
                    ribbon::group(ui, &t, "Navegación", |ui| {
                        for (icon, name, n) in [("🔍", "Zoom", Nav::Zoom), ("✋", "Desplazar", Nav::Pan)] {
                            if ribbon::big_button(ui, &t, icon, name, self.nav == n).clicked() {
                                self.act(ctx, Action::Nav(n));
                            }
                        }
                        ui.vertical(|ui| {
                            if ribbon::small_button(ui, &t, "@full", "Extensión completa", false).clicked() {
                                self.act(ctx, Action::ZoomFull);
                            }
                            if ribbon::small_button(ui, &t, "@prev", "Zoom anterior", false).clicked() {
                                self.act(ctx, Action::ZoomPrev);
                            }
                            if ribbon::small_button(ui, &t, "@clear", "Limpiar marcas", false).clicked() {
                                self.act(ctx, Action::ClearMarks);
                            }
                        });
                    });
                    ribbon::group(ui, &t, "Herramientas", |ui| {
                        for (icon, name, n) in [("❓", "Consultar\n(Q)", Nav::Query), ("📏", "Medir\n(W)", Nav::Measure), ("@density", "Densidad", Nav::Density), ("📈", "Perfil", Nav::Profile)] {
                            if ribbon::big_button(ui, &t, icon, name, self.nav == n).clicked() {
                                self.act(ctx, Action::Nav(n));
                            }
                        }
                    });
                    ribbon::group(ui, &t, "Ventanas", |ui| {
                        ui.vertical(|ui| {
                            let names = ["2D", "2D + perfil", "3D", "2D + perfil + 3D"];
                            let cur = [Layout::Plan, Layout::PlanProfile, Layout::View3d, Layout::All]
                                .iter()
                                .position(|l| *l == self.layout)
                                .unwrap_or(0);
                            egui::ComboBox::from_id_salt("layout").width(130.0).selected_text(names[cur]).show_ui(ui, |ui| {
                                for (k, n) in names.iter().enumerate() {
                                    if ui.selectable_label(k == cur, *n).clicked() {
                                        self.act(ctx, Action::Layout(k as u8));
                                    }
                                }
                            });
                            ui.menu_button("Vista 3D…", |ui| {
                                if ui.button("Norte arriba (N)").clicked() {
                                    self.act(ctx, Action::NorthUp);
                                }
                                if ui.button("Exageración vertical +").clicked() {
                                    self.act(ctx, Action::Exaggeration(1));
                                }
                                if ui.button("Exageración vertical −").clicked() {
                                    self.act(ctx, Action::Exaggeration(-1));
                                }
                                ui.checkbox(&mut self.follow_plan, "Enlazar con la planta (2D+perfil+3D)");
                            });
                        });
                    });
                    ribbon::group(ui, &t, "Puntos", |ui| {
                        ui.vertical(|ui| {
                            let names = [("Elevación (azul-rojo)", ColorBy::Elevation), ("Elevación (tierra)", ColorBy::ElevationEarth), ("Clasificación", ColorBy::Class), ("Intensidad", ColorBy::Intensity), ("Retorno", ColorBy::Return), ("ID de fuente", ColorBy::Source), ("Archivo", ColorBy::File), ("RGB", ColorBy::Rgb)];
                            let cur = names.iter().find(|n| n.1 == self.style.color_by).map_or("Colorear por…", |n| n.0);
                            egui::ComboBox::from_id_salt("colorby").width(160.0).selected_text(cur).show_ui(ui, |ui| {
                                for (name, c) in names {
                                    if ui.selectable_label(self.style.color_by == c, name).clicked() {
                                        self.act(ctx, Action::Color(c));
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Tamaño");
                                if ui.button("−").clicked() {
                                    self.act(ctx, Action::PointSize(-1));
                                }
                                if ui.button("+").clicked() {
                                    self.act(ctx, Action::PointSize(1));
                                }
                                ui.checkbox(&mut self.show_points, "Ver");
                            });
                        });
                        ui.vertical(|ui| {
                            ui.menu_button("Rango de elevación…", |ui| {
                                let (lo, hi) = self.store.as_ref().map_or((0.0, 1.0), |s| (s.bounds[2], s.bounds[5]));
                                ui.horizontal(|ui| {
                                    ui.label("Mín");
                                    if ui.add(egui::DragValue::new(&mut self.style.zmin).range(lo - 1000.0..=hi).speed(0.5)).changed() {
                                        self.touch();
                                    }
                                    ui.label("Máx");
                                    if ui.add(egui::DragValue::new(&mut self.style.zmax).range(lo..=hi + 1000.0).speed(0.5)).changed() {
                                        self.touch();
                                    }
                                });
                                if ui.button("Ajustar a los datos").clicked() {
                                    self.style.zmin = lo;
                                    self.style.zmax = hi;
                                }
                            });
                            ui.menu_button("Filtros…", |ui| {
                                for (k, n) in ["Primeros retornos", "Retornos intermedios", "Últimos retornos", "Retornos únicos"].iter().enumerate() {
                                    let mut on = self.style.returns & (1 << k) != 0;
                                    if ui.checkbox(&mut on, *n).changed() {
                                        self.style.returns ^= 1 << k;
                                    }
                                }
                                ui.separator();
                                if ui.button("Clases y colores…").clicked() {
                                    self.act(ctx, Action::Classes);
                                }
                            });
                        });
                    });
                    ribbon::group(ui, &t, "Superficie", |ui| {
                        if ribbon::big_button(ui, &t, "@tin", "TIN\nsombreado (T)", self.tin_on).clicked() {
                            self.act(ctx, Action::ToggleTin);
                        }
                        if ribbon::big_button(ui, &t, "〰", "Curvas\nde nivel", self.contours_on).clicked() {
                            self.act(ctx, Action::ToggleContours);
                        }
                        ui.vertical(|ui| {
                            ui.menu_button("Opciones…", |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Equidistancia");
                                    ui.add(egui::DragValue::new(&mut self.contour_iv).range(0.1..=500.0).speed(0.5));
                                });
                                ui.horizontal(|ui| {
                                    ui.label("TIN con");
                                    let ground = self.tin_classes == ClassSet::from_bits(1 << 2);
                                    if ui.selectable_label(ground, "suelo").clicked() {
                                        self.tin_classes = ClassSet::from_bits(1 << 2);
                                    }
                                    if ui.selectable_label(!ground, "todas las clases").clicked() {
                                        self.tin_classes = ClassSet::all();
                                    }
                                });
                            });
                        });
                    });
                }
                4 => {
                    ribbon::group(ui, &t, "Ayuda", |ui| {
                        if ribbon::big_button(ui, &t, "@manual", "Manual\n(F1)", self.manual.open).clicked() {
                            self.act(ctx, Action::Manual);
                        }
                        if ribbon::big_button(ui, &t, "ℹ", "Acerca de", false).clicked() {
                            self.act(ctx, Action::About);
                        }
                        if ribbon::big_button(ui, &t, "🔍", "Buscar\ncomando", false).clicked() {
                            self.palette = Some(String::new());
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Atajos").strong());
                        ui.label("Q consultar · W medir · Z zoom · X desplazar · T TIN · Ctrl+Z deshacer");
                        ui.label("+ / − tamaño del punto · flechas: mover el perfil · Ctrl+O abrir · Ctrl+K comandos");
                        ui.label("Zoom: clic acerca ×2, clic derecho aleja, arrastrar = ventana, rueda, botón central = anterior");
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
                    ui.selectable_value(&mut self.side, Side::Layers, "Capas");
                    ui.selectable_value(&mut self.side, Side::Info, "Información");
                    ui.selectable_value(&mut self.side, Side::Edit, "Edición");
                });
                ui.separator();
                match self.side {
                    Side::Edit => self.edit_side(ui),
                    Side::Info => {
                        if ui.button("↻ Analizar de nuevo").clicked() {
                            self.run_info(ctx);
                        }
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&self.info_text).monospace().size(11.5),
                                    )
                                    .selectable(true)
                                    .wrap_mode(egui::TextWrapMode::Extend),
                                );
                            });
                    }
                    Side::Layers => {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            egui::CollapsingHeader::new(format!(
                                "Nubes de puntos ({})",
                                self.clouds.len()
                            ))
                            .default_open(true)
                            .id_salt("clouds_header")
                            .show(ui, |ui| {
                                let mut remove = None;
                                for (k, p) in self.clouds.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let mut on = k >= 64 || self.style.files & (1u64 << k) != 0;
                                        let width = (ui.available_width() - 34.0).max(40.0);
                                        let changed = ui
                                            .scope(|ui| {
                                                ui.set_max_width(width);
                                                ui.style_mut().wrap_mode =
                                                    Some(egui::TextWrapMode::Truncate);
                                                ui.checkbox(
                                                    &mut on,
                                                    p.file_name().map_or(String::new(), |n| {
                                                        n.to_string_lossy().into_owned()
                                                    }),
                                                )
                                                .on_hover_text(p.display().to_string())
                                                .changed()
                                            })
                                            .inner;
                                        if changed && k < 64 {
                                            self.style.files ^= 1u64 << k;
                                        }
                                        if ui.small_button("✖").clicked() {
                                            remove = Some(k);
                                        }
                                    });
                                }
                                if let Some(k) = remove {
                                    self.clouds.remove(k);
                                    if self.clouds.is_empty() {
                                        self.act(ctx, Action::CloseAll);
                                    } else {
                                        self.reload(ctx);
                                    }
                                }
                                ui.checkbox(&mut self.show_points, "Mostrar las nubes de puntos");
                                ui.horizontal(|ui| {
                                    ui.label("Puntos en pantalla (M)");
                                    ui.add(
                                        egui::DragValue::new(&mut self.max_points_m)
                                            .range(0.5..=200.0)
                                            .speed(0.5),
                                    );
                                    if ui
                                        .button("Aplicar")
                                        .on_hover_text("Vuelve a cargar con esa decimación")
                                        .clicked()
                                        && !self.clouds.is_empty()
                                    {
                                        self.reload(ctx);
                                    }
                                });
                                if let Some(s) = &self.store {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} de {} puntos (1 de cada {})",
                                            s.len(),
                                            s.total,
                                            s.stride
                                        ))
                                        .color(t.weak)
                                        .size(11.0),
                                    );
                                }
                                if let Some((_, st)) = &self.loading {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(format!(
                                            "cargando… {:.0} s",
                                            st.elapsed().as_secs_f64()
                                        ));
                                    });
                                }
                            });
                            egui::CollapsingHeader::new(format!("Capas ({})", self.refs.len()))
                                .default_open(true)
                                .show(ui, |ui| self.layers_ui(ui, ctx));
                            if self.nav == Nav::Profile || self.profile.is_some() {
                                egui::CollapsingHeader::new("Perfil")
                                    .default_open(true)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label("Ancho de la caja (m)");
                                            ui.add(
                                                egui::DragValue::new(&mut self.profile_width)
                                                    .range(0.5..=1000.0)
                                                    .speed(0.5),
                                            );
                                        });
                                        if let Some(p) = &mut self.profile {
                                            p.width = self.profile_width;
                                            ui.label(format!(
                                                "Longitud {:.1} m · ↑/↓ o rueda mueven la caja",
                                                p.length()
                                            ));
                                        }
                                    });
                            }
                        });
                    }
                }
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
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
                            "{} · {:.0} s",
                            r.label,
                            r.started.elapsed().as_secs_f64()
                        )));
                    } else if let Some((_, st)) = &self.loading {
                        ui.separator();
                        ui.spinner();
                        ui.label(small(format!(
                            "cargando… {:.0} s",
                            st.elapsed().as_secs_f64()
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
        if let Some(store) = &store {
            let thin = if interactive && store.len() > 1_500_000 {
                store.len() / 1_500_000 + 1
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
            ));
            if self.tex_plan.as_ref().is_none_or(|t| t.1 != key) {
                let img = view::render_plan(store, &self.plan, w, h, &self.style);
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
        self.paint_refs(&painter, rect, w, h);
        if let Some(store) = &store {
            if self.tin_on || self.contours_on {
                self.paint_tin(ctx, &painter, store, rect, w, h);
            }
            if self.show_points
                && let Some((tex, _)) = &self.tex_plan
            {
                painter.image(tex.id(), rect, uv, Color32::WHITE);
            }
        }
        let m2s = |s: &Self, x: f64, y: f64| {
            let (px, py) = s.plan.to_screen(x, y, w, h);
            Pos2::new(px as f32, py as f32) + origin
        };
        self.paint_vectors(&painter, rect, w, h);
        self.paint_draft(&painter, pointer, rect, w, h);
        if let Some(bx) = &self.profile {
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
                rect.left_top() + Vec2::new(10.0, 28.0),
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
        let (w, h) = (
            rect.width().max(8.0) as usize,
            rect.height().max(8.0) as usize,
        );
        if let (Some(store), Some(bx)) = (self.store.clone(), self.profile) {
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
                        "Z {lo:.1} – {hi:.1} m · longitud {:.1} m · ancho {:.1} m",
                        bx.length(),
                        bx.width
                    ),
                    FontId::proportional(11.5),
                    t.weak,
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
        let mut moved = false;
        let (mods, hovered) = (ui.input(|i| i.modifiers), resp.hovered());
        let d = resp.drag_delta();
        let o = &mut self.orbit;
        let left = resp.dragged_by(egui::PointerButton::Primary);
        let pan = resp.dragged_by(egui::PointerButton::Secondary)
            || resp.dragged_by(egui::PointerButton::Middle)
            || (left && mods.shift);
        if left && !pan {
            if mods.ctrl {
                o.dist = (o.dist * 1.006f64.powf(f64::from(d.y))).max(0.5);
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
                o.dist = (o.dist * 0.9985f64.powf(f64::from(sc))).max(0.5);
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
                o.dist = (o.dist * (1.0 - f64::from(kz) * step)).max(0.5);
                moved = true;
                ui.ctx().request_repaint();
            }
            if home || resp.double_clicked() {
                let keep = o.exaggeration;
                *o = Orbit::fit(b);
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
                st.window.map(|w| w.map(f64::to_bits)),
            ));
            if self.tex_3d.as_ref().is_none_or(|t| t.1 != key) {
                let img = view::render_3d(&store, &self.orbit, w, h, &st);
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
                if rect.width() > 900.0 {
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
        for k in pressed {
            match (k, ctrl) {
                (Key::O, true) => self.act(ctx, Action::OpenClouds),
                (Key::K, true) => self.palette = Some(String::new()),
                (Key::Q, false) => self.act(ctx, Action::Nav(Nav::Query)),
                (Key::W, false) => self.act(ctx, Action::Nav(Nav::Measure)),
                (Key::Z, false) => self.act(ctx, Action::Nav(Nav::Zoom)),
                (Key::X, false) => self.act(ctx, Action::Nav(Nav::Pan)),
                (Key::T, false) => self.act(ctx, Action::ToggleTin),
                (Key::N, false) => self.act(ctx, Action::NorthUp),
                (Key::Plus | Key::Equals, false) => self.act(ctx, Action::PointSize(1)),
                (Key::Minus, false) => self.act(ctx, Action::PointSize(-1)),
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
        for p in dropped {
            self.open_path(&ctx, p, false);
        }
        self.keys(&ctx);
        self.header(ui, &ctx);
        self.ribbon(ui, &ctx);
        self.status_bar(ui);
        self.side_panel(ui, &ctx);
        self.views(ui, &ctx);
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
        self.palette_window(&ctx);
        self.new_layer_window(&ctx);
        self.table_window(&ctx);
        self.rs_window(&ctx);
        self.raster_tools_window(&ctx);
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
        self.manual.window(&ctx);
    }
}
