//! Rásteres en el mapa: imágenes multibanda (fotos aéreas, escenas de satélite) leídas por ventana
//! de la pirámide que hay dentro del archivo, de modo que cualquier tamaño se ve nítido a cualquier
//! zoom, rejillas de una banda con su simbología (estirado, clases, relieve sombreado),
//! composiciones de color e índices espectrales.

use std::sync::mpsc::{Receiver, channel};

use super::*;
use crate::symbology::ramp_rgb;
use hipso_core::pyramid::Pyramid;
use hipso_core::raster::{self, BandStats, Expr, INDICES, Image as BandImage, RgbStyle};

/// Cómo se asignan los valores de una banda al negro y al blanco.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stretch {
    /// El rango del tipo de datos (0–255 para fotos).
    None,
    MinMax,
    /// Del percentil 2 al 98.
    Percent,
    /// La media más o menos dos desviaciones típicas.
    Sigma,
    /// Los límites escritos, los mismos para cada banda mostrada (reflectancia de Sentinel-2: 0 a
    /// 3000).
    Manual,
}

impl Stretch {
    pub const ALL: [Stretch; 5] = [
        Stretch::None,
        Stretch::MinMax,
        Stretch::Percent,
        Stretch::Sigma,
        Stretch::Manual,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Stretch::None => "Sin estirar",
            Stretch::MinMax => "Mínimo–máximo",
            Stretch::Percent => "Percentiles 2–98 %",
            Stretch::Sigma => "±2 desviaciones",
            Stretch::Manual => "Cortes manuales",
        }
    }
}

/// La imagen de una ventana de la imagen, al nivel que conviene al zoom.
pub struct ViewTex {
    tex: egui::TextureHandle,
    /// West, south, east, north.
    extent: [f64; 4],
    level: usize,
    rev: u64,
}

type Window = Result<(Vec<u8>, [usize; 2], [f64; 4]), String>;

struct ViewJob {
    rx: Receiver<Window>,
    level: usize,
    rev: u64,
}

/// Una capa de imagen multibanda.
pub struct ImageLayer {
    pub pyr: Arc<Pyramid>,
    /// La imagen entera a una resolución reducida: vista previa, estadísticos, valores e índices.
    pub work: Arc<BandImage>,
    /// Píxeles del archivo por píxel de `work`, a lo largo de cada eje.
    pub reduction: usize,
    pub stats: Vec<BandStats>,
    pub bands: [usize; 3],
    pub stretch: Stretch,
    /// El valor más bajo y más alto que se pinta con `Stretch::Manual`.
    pub manual: [f32; 2],
    pub brightness: f32,
    pub contrast: f32,
    pub gamma: f32,
    /// Pseudocolor monobanda: la rampa (índice en `RAMPS`) y si va invertida; sin ella, gris o
    /// compuesto de tres bandas.
    pub pseudo: Option<(usize, bool)>,
    rev: u64,
    preview: egui::TextureHandle,
    view: Option<ViewTex>,
    job: Option<ViewJob>,
    last_plan: (f64, f64, f64),
    steady_since: Instant,
    /// Lleva las coordenadas de la imagen al sistema del trabajo, cuando difieren.
    pub warp: Option<Arc<hipso_core::reproject::Transformer>>,
}

/// Una imagen abierta: su pirámide, la imagen entera reducida y la reducción.
type Opened = Result<(Arc<Pyramid>, BandImage, usize), String>;

/// Una imagen grande que se está abriendo en segundo plano.
pub struct ImageJob {
    pub path: PathBuf,
    pub rx: Receiver<Opened>,
    pub started: Instant,
}

/// La imagen con un píxel de cada `n` a lo largo de cada lado, de modo que tenga como mucho `max`
/// píxeles de ancho (una textura tiene un límite que depende de la tarjeta gráfica: 8192 es
/// habitual).
fn reduced(img: &BandImage, max: usize) -> std::borrow::Cow<'_, BandImage> {
    let step = (img.cols.max(img.rows) / max.max(1) + 1).max(1);
    if img.cols.max(img.rows) <= max {
        return std::borrow::Cow::Borrowed(img);
    }
    let (cols, rows) = (img.cols / step, img.rows / step);
    let bands = img
        .bands
        .iter()
        .map(|b| {
            (0..cols * rows)
                .map(|i| b[(i / cols) * step * img.cols + (i % cols) * step])
                .collect()
        })
        .collect();
    std::borrow::Cow::Owned(BandImage {
        cols,
        rows,
        cell: img.cell * step as f64,
        bands,
        ..img.clone()
    })
}

/// La imagen de una imagen, de como mucho 4096 píxeles de ancho.
fn picture(ctx: &egui::Context, name: &str, img: &BandImage, st: &RgbStyle) -> egui::TextureHandle {
    let src = reduced(img, 4096);
    let rgba = raster::composite_style(&src, st);
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied([src.cols.max(1), src.rows.max(1)], &rgba),
        egui::TextureOptions::LINEAR,
    )
}

impl ImageLayer {
    fn new(
        ctx: &egui::Context,
        name: &str,
        pyr: Arc<Pyramid>,
        work: BandImage,
        reduction: usize,
    ) -> Self {
        let stats: Vec<BandStats> = work.bands.iter().map(|b| raster::band_stats(b)).collect();
        let nb = work.bands.len();
        let photo = pyr.bits == 8;
        let mut l = Self {
            bands: if nb >= 3 { [0, 1, 2] } else { [0, 0, 0] },
            stretch: if photo {
                Stretch::None
            } else {
                Stretch::Percent
            },
            manual: [0.0, 1.0],
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            pseudo: None,
            rev: 0,
            preview: ctx.load_texture(
                name,
                egui::ColorImage::filled([1, 1], Color32::TRANSPARENT),
                egui::TextureOptions::LINEAR,
            ),
            view: None,
            job: None,
            last_plan: (0.0, 0.0, 0.0),
            steady_since: Instant::now(),
            warp: None,
            pyr,
            work: Arc::new(work),
            reduction,
            stats,
        };
        l.restyle(ctx, name);
        l
    }

    /// La asignación de valores a colores, a partir del estirado y de la elección de bandas.
    pub fn style(&self) -> RgbStyle {
        let mut lo = [0.0; 3];
        let mut hi = [1.0; 3];
        for k in 0..3 {
            let s = self.stats.get(self.bands[k]).copied().unwrap_or_default();
            (lo[k], hi[k]) = match self.stretch {
                Stretch::None => match self.pyr.bits {
                    8 => (0.0, 255.0),
                    16 => (0.0, 65535.0),
                    _ => (s.min, s.max),
                },
                Stretch::MinMax => (s.min, s.max),
                Stretch::Percent => (s.p2, s.p98),
                Stretch::Sigma => (s.mean - 2.0 * s.std, s.mean + 2.0 * s.std),
                Stretch::Manual => (self.manual[0], self.manual[1].max(self.manual[0] + 1e-6)),
            };
        }
        RgbStyle {
            bands: self.bands,
            lo,
            hi,
            brightness: self.brightness,
            contrast: self.contrast,
            gamma: self.gamma,
            palette: self.pseudo.map(|(ramp, invert)| {
                (0..256)
                    .map(|k| crate::symbology::ramp_rgb(ramp, f64::from(k) / 255.0, invert))
                    .collect()
            }),
        }
    }

    /// El estilo cambió: la vista previa se redibuja ahora y la ventana nítida se vuelve a leer.
    pub fn restyle(&mut self, ctx: &egui::Context, name: &str) {
        self.rev += 1;
        self.view = None;
        self.job = None;
        self.preview = picture(ctx, &format!("img-{name}"), &self.work, &self.style());
    }

    /// La extensión en el sistema del trabajo (x mínima, y mínima, x máxima, y máxima).
    pub fn display_extent(&self) -> [f64; 4] {
        let e = self.pyr.extent();
        match &self.warp {
            Some(t) => t
                .transform_bounds((e[0], e[1], e[2], e[3]))
                .map_or(e, |b| [b.0, b.1, b.2, b.3]),
            None => e,
        }
    }

    /// Los valores de banda en un punto del mapa, a partir de la imagen reducida.
    pub fn values_at(&self, x: f64, y: f64) -> Option<String> {
        if self.warp.is_some() {
            return None;
        }
        let w = &self.work;
        let c = ((x - w.x0) / w.cell).floor();
        let r = ((w.y0 + w.rows as f64 * w.cell - y) / w.cell).floor();
        if c < 0.0 || r < 0.0 || c as usize >= w.cols || r as usize >= w.rows {
            return None;
        }
        let i = r as usize * w.cols + c as usize;
        let v: Vec<String> = w
            .bands
            .iter()
            .enumerate()
            .map(|(b, v)| format!("B{}={:.0}", b + 1, v[i]))
            .collect();
        Some(v.join(" "))
    }

    /// Recoge la ventana que terminó y empieza a leer la que necesita la vista, una vez que la
    /// vista ha estado quieta un momento. Devuelve verdadero mientras algo está pendiente.
    pub fn update(&mut self, ctx: &egui::Context, plan: &Plan, w: usize, h: usize) -> bool {
        if self.warp.is_some() {
            return false;
        }
        let finished = self
            .job
            .as_ref()
            .and_then(|j| j.rx.try_recv().ok().map(|r| (j.level, j.rev, r)));
        if let Some((level, rev, r)) = finished {
            self.job = None;
            if let Ok((rgba, [cols, rows], extent)) = r
                && rev == self.rev
            {
                self.view = Some(ViewTex {
                    tex: ctx.load_texture(
                        "img-window",
                        egui::ColorImage::from_rgba_unmultiplied([cols, rows], &rgba),
                        egui::TextureOptions::LINEAR,
                    ),
                    extent,
                    level,
                    rev,
                });
            }
        }
        let cur = (plan.cx, plan.cy, plan.scale);
        if cur != self.last_plan {
            self.last_plan = cur;
            self.steady_since = Instant::now();
        }
        let (lo, hi) = (
            plan.to_map(0.0, h as f64, w, h),
            plan.to_map(w as f64, 0.0, w, h),
        );
        let ext = self.pyr.extent();
        let vis = [
            lo.0.max(ext[0]),
            lo.1.max(ext[1]),
            hi.0.min(ext[2]),
            hi.1.min(ext[3]),
        ];
        if vis[2] <= vis[0] || vis[3] <= vis[1] {
            return false;
        }
        let n = self.pyr.levels.len();
        let mut level = self.pyr.level_for(1.0 / plan.scale);
        let px = |level: usize, a: f64, b: f64| (b - a) / self.pyr.cell_at(level);
        while level + 1 < n
            && (px(level, vis[0], vis[2]) > 3500.0 || px(level, vis[1], vis[3]) > 3500.0)
        {
            level += 1;
        }
        let inside = |a: &[f64; 4], b: &[f64; 4]| {
            let e = 1e-6;
            a[0] <= b[0] + e && a[1] <= b[1] + e && a[2] >= b[2] - e && a[3] >= b[3] - e
        };
        if self
            .view
            .as_ref()
            .is_some_and(|v| v.level == level && v.rev == self.rev && inside(&v.extent, &vis))
        {
            return false;
        }
        if self
            .job
            .as_ref()
            .is_some_and(|j| j.level == level && j.rev == self.rev)
        {
            return true;
        }
        let wait = Duration::from_millis(120);
        if self.steady_since.elapsed() < wait {
            ctx.request_repaint_after(wait);
            return true;
        }
        // la ventana: lo que se ve más un margen, dentro de lo que cabe en una textura
        let mut grow = 0.3;
        let window = loop {
            let (dx, dy) = ((vis[2] - vis[0]) * grow, (vis[3] - vis[1]) * grow);
            let win = [
                (vis[0] - dx).max(ext[0]),
                (vis[1] - dy).max(ext[1]),
                (vis[2] + dx).min(ext[2]),
                (vis[3] + dy).min(ext[3]),
            ];
            if grow <= 0.0
                || (px(level, win[0], win[2]) <= 4096.0 && px(level, win[1], win[3]) <= 4096.0)
            {
                break win;
            }
            grow -= 0.1;
        };
        let (pyr, st, rev) = (self.pyr.clone(), self.style(), self.rev);
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = pyr
                .read(level, window)
                .map(|full| {
                    // la ventana de una imagen demasiado grande para una textura se lee adelgazada
                    let img = reduced(&full, 4096);
                    let extent = [
                        img.x0,
                        img.y0,
                        img.x0 + img.cols as f64 * img.cell,
                        img.y0 + img.rows as f64 * img.cell,
                    ];
                    (
                        raster::composite_style(&img, &st),
                        [img.cols, img.rows],
                        extent,
                    )
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.job = Some(ViewJob { rx, level, rev });
        true
    }

    /// Dibuja la vista previa sobre toda la imagen y la ventana nítida sobre ella.
    pub fn paint(
        &self,
        painter: &egui::Painter,
        plan: &Plan,
        rect: Rect,
        w: usize,
        h: usize,
        opacity: f32,
    ) {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        let tint = Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8);
        let clip = painter.with_clip_rect(rect);
        let at = |e: &[f64; 4]| {
            let (a, b) = (
                plan.to_screen(e[0], e[3], w, h),
                plan.to_screen(e[2], e[1], w, h),
            );
            Rect::from_min_max(
                Pos2::new(a.0 as f32, a.1 as f32) + rect.min.to_vec2(),
                Pos2::new(b.0 as f32, b.1 as f32) + rect.min.to_vec2(),
            )
        };
        if let Some(tf) = &self.warp {
            crate::warp::paint_warped(
                painter,
                self.preview.id(),
                self.pyr.extent(),
                tf,
                plan,
                rect,
                w,
                h,
                tint,
                24,
            );
            return;
        }
        clip.image(self.preview.id(), at(&self.pyr.extent()), uv, tint);
        if let Some(v) = self.view.as_ref().filter(|v| v.rev == self.rev) {
            clip.image(v.tex.id(), at(&v.extent), uv, tint);
        }
    }
}

// ------------------------------------------------------------------------------ rejillas

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GridMode {
    Stretch,
    Classified,
    Hillshade,
}

impl GridMode {
    pub const ALL: [GridMode; 3] = [GridMode::Stretch, GridMode::Classified, GridMode::Hillshade];

    pub fn name(self) -> &'static str {
        match self {
            GridMode::Stretch => "Estirar",
            GridMode::Classified => "Clasificar",
            GridMode::Hillshade => "Relieve sombreado",
        }
    }
}

/// Cómo se colorea un ráster de una banda.
#[derive(Clone, Debug)]
pub struct GridStyle {
    pub mode: GridMode,
    pub ramp: usize,
    pub invert: bool,
    pub lo: f32,
    pub hi: f32,
    pub classes: usize,
    /// Acimut y altitud del sol en grados, exageración vertical y cuánto del color de la rampa se
    /// ve a través del relieve (0 = relieve gris).
    pub azimuth: f32,
    pub altitude: f32,
    pub zfactor: f32,
    pub blend: f32,
}

impl GridStyle {
    pub fn new(g: &Grid) -> Self {
        let (lo, hi) = raster::stretch(&g.data);
        Self {
            mode: GridMode::Stretch,
            ramp: 5,
            invert: false,
            lo,
            hi,
            classes: 6,
            azimuth: 315.0,
            altitude: 45.0,
            zfactor: 1.0,
            blend: 0.6,
        }
    }
}

/// La imagen de una rejilla.
pub fn render_grid(g: &Grid, st: &GridStyle) -> Image {
    use rayon::prelude::{IndexedParallelIterator, ParallelIterator, ParallelSliceMut};
    let mut img = Image {
        w: g.cols,
        h: g.rows,
        rgba: vec![0; g.cols * g.rows * 4],
    };
    let span = f64::from(st.hi - st.lo).max(1e-9);
    let t_of = |v: f32| (f64::from(v - st.lo) / span).clamp(0.0, 1.0);
    let (zen, az) = (
        (90.0 - f64::from(st.altitude)).to_radians(),
        (360.0 - f64::from(st.azimuth) + 90.0)
            .rem_euclid(360.0)
            .to_radians(),
    );
    let (cols, rows) = (g.cols, g.rows);
    let at = |x: usize, y: usize, fallback: f32| {
        let v = g.data[y * cols + x];
        if v.is_nan() { fallback } else { v }
    };
    img.rgba
        .par_chunks_mut(4 * cols.max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let v = g.data[y * cols + x];
                if v.is_nan() {
                    continue;
                }
                let c = match st.mode {
                    GridMode::Stretch => ramp_rgb(st.ramp, t_of(v), st.invert),
                    GridMode::Classified => {
                        let n = st.classes.clamp(2, 20);
                        let k = ((t_of(v) * n as f64).floor() as usize).min(n - 1);
                        ramp_rgb(st.ramp, (k as f64 + 0.5) / n as f64, st.invert)
                    }
                    GridMode::Hillshade => {
                        let (x0, x1) = (x.saturating_sub(1), (x + 1).min(cols - 1));
                        let (y0, y1) = (y.saturating_sub(1), (y + 1).min(rows - 1));
                        let z = |xx, yy| f64::from(at(xx, yy, v));
                        let dzdx = ((z(x1, y0) + 2.0 * z(x1, y) + z(x1, y1))
                            - (z(x0, y0) + 2.0 * z(x0, y) + z(x0, y1)))
                            / (8.0 * g.cell)
                            * f64::from(st.zfactor);
                        let dzdy = ((z(x0, y1) + 2.0 * z(x, y1) + z(x1, y1))
                            - (z(x0, y0) + 2.0 * z(x, y0) + z(x1, y0)))
                            / (8.0 * g.cell)
                            * f64::from(st.zfactor);
                        let slope = (dzdx.hypot(dzdy)).atan();
                        let aspect = dzdy.atan2(-dzdx);
                        let shade = (zen.cos() * slope.cos()
                            + zen.sin() * slope.sin() * (az - aspect).cos())
                        .clamp(0.0, 1.0);
                        let base = ramp_rgb(st.ramp, t_of(v), st.invert);
                        let k = 0.25 + 0.75 * shade;
                        let b = f64::from(st.blend);
                        let mix = |c: u8| ((f64::from(c) * b + 255.0 * (1.0 - b)) * k) as u8;
                        [mix(base[0]), mix(base[1]), mix(base[2])]
                    }
                };
                px[..3].copy_from_slice(&c);
                px[3] = 255;
            }
        });
    img
}

// ------------------------------------------------------------------------------ la aplicación

/// Si un archivo tiene más de una banda (o es demasiado grande para leerlo entero), de modo que es
/// una imagen.
fn is_image(p: &Path) -> bool {
    Pyramid::open(p).is_ok_and(|py| {
        py.bands >= 2 || u64::from(py.levels[0].cols) * u64::from(py.levels[0].rows) > 60_000_000
    })
}

impl App {
    /// Un ráster como capa: una rejilla de una banda o, si no, una imagen abierta en segundo plano.
    pub fn add_raster(&mut self, ctx: &egui::Context, p: &Path) -> bool {
        let ascii = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("asc"));
        if ascii || !is_image(p) {
            return match Grid::read(p) {
                Ok(mut g) => {
                    if let Some((from, to)) = self.crs_move(p) {
                        let started = Instant::now();
                        self.log(format!(
                            "«{}» está en {}, distinto del de la nube ({}): se pasa a él al cargar, y eso tarda más.",
                            p.display(),
                            hipso_core::crs::name(from),
                            hipso_core::crs::name(to)
                        ));
                        match hipso_core::reproject::grid(&g, from, to) {
                            Ok(m) => {
                                g = m;
                                self.log(format!(
                                    "  convertido en {:.1} s",
                                    started.elapsed().as_secs_f64()
                                ));
                            }
                            Err(e) => {
                                self.log(format!(
                                    "No se pudo pasar {} al sistema de la nube: {e}",
                                    p.display()
                                ));
                                return false;
                            }
                        }
                    }
                    let name = p
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    self.add_grid(ctx, name, p.to_path_buf(), g);
                    true
                }
                Err(e) => {
                    self.log(format!("No se pudo leer {}: {e}", p.display()));
                    false
                }
            };
        }
        self.add_image(ctx, p)
    }

    /// Añade una imagen como capa: abierta y reducida en segundo plano.
    pub fn add_image(&mut self, ctx: &egui::Context, p: &Path) -> bool {
        let (tx, rx) = channel();
        let (path, c) = (p.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let r = (|| {
                let pyr = Arc::new(Pyramid::open(&path).map_err(|e| e.to_string())?);
                let (work, reduction) = pyr.overview(12_000_000).map_err(|e| e.to_string())?;
                Ok((pyr, work, reduction))
            })();
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.img_jobs.push(ImageJob {
            path: p.to_path_buf(),
            rx,
            started: Instant::now(),
        });
        self.log(format!("Abriendo la imagen {}…", p.display()));
        true
    }

    /// Recoge las imágenes que han terminado los hilos de segundo plano.
    pub fn poll_images(&mut self, ctx: &egui::Context) {
        let mut done = Vec::new();
        for (k, j) in self.img_jobs.iter().enumerate() {
            if let Ok(r) = j.rx.try_recv() {
                done.push((k, r));
            }
        }
        for (k, r) in done.into_iter().rev() {
            let job = self.img_jobs.remove(k);
            match r {
                Ok((pyr, work, reduction)) => {
                    let (nb, levels) = (pyr.bands, pyr.levels.len());
                    let name = job
                        .path
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    let from = pyr.epsg;
                    if from.is_some() && self.project_crs().is_none() {
                        self.pending_conform.push(job.path.clone());
                    }
                    let mut layer = ImageLayer::new(ctx, &name, pyr, work, reduction);
                    if let (Some(from), Some(to)) = (from, self.project_crs())
                        && hipso_core::crs::relation(to, from)
                            == hipso_core::crs::Relation::Different
                    {
                        match hipso_core::reproject::transformer(from, to) {
                            Ok(t) => {
                                layer.warp = Some(Arc::new(t));
                                self.log(format!(
                                    "«{}» está en {}, distinto del de la nube ({}): se coloca en el mapa convertida (a la resolución de la vista previa, sin ventanas nítidas al acercar).",
                                    job.path.display(),
                                    hipso_core::crs::name(from),
                                    hipso_core::crs::name(to)
                                ));
                            }
                            Err(e) => self.log(format!(
                                "No se pudo pasar la imagen al sistema de la nube: {e}"
                            )),
                        }
                    }
                    self.remove_paths(&job.path);
                    let k = self.add_ref(
                        RefLayer {
                            name,
                            path: job.path.clone(),
                            kind: RefKind::Image(Box::new(layer)),
                            visible: true,
                            opacity: 1.0,
                        },
                        true,
                    );
                    self.sel_layer = Some(k);
                    self.log(format!(
                        "Imagen abierta en {:.1} s: {} ({nb} bandas, {levels} niveles de pirámide)",
                        job.started.elapsed().as_secs_f64(),
                        job.path.display()
                    ));
                    if self.store.is_none() && self.refs.iter().filter(|r| r.visible).count() == 1 {
                        self.plan_fit = true;
                    }
                }
                Err(e) => self.log(format!(
                    "No se pudo leer la imagen {}: {e}",
                    job.path.display()
                )),
            }
        }
    }

    /// Lee las ventanas que necesitan las capas de imagen para la vista actual.
    pub fn update_images(&mut self, ctx: &egui::Context, w: usize, h: usize) {
        let plan = self.plan;
        let mut busy = false;
        for r in self.refs.iter_mut().filter(|r| r.visible) {
            if let RefKind::Image(il) = &mut r.kind {
                busy |= il.update(ctx, &plan, w, h);
            }
        }
        if busy {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    /// Dibuja las imágenes y las rejillas (los vectores van sobre los puntos, estas bajo ellos).
    /// Dibuja las capas cuyo índice está en `range`, de abajo arriba: imágenes y rásteres, y las
    /// capas vectoriales, cada una en su lugar del orden de dibujo.
    pub fn paint_layers(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        w: usize,
        h: usize,
        range: std::ops::Range<usize>,
    ) {
        for k in range.clone() {
            self.paint_refs(painter, rect, w, h, k..k + 1);
            self.paint_vectors(painter, rect, w, h, k..k + 1);
        }
        // las etiquetas van sobre todo, así que se dibujan una vez, con el último rango
        if range.end >= self.refs.len() {
            self.paint_vector_labels(painter, rect, w, h, 0..self.refs.len());
        }
    }

    /// Dibuja las imágenes y los rásteres entre las capas de `range`.
    pub fn paint_refs(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        w: usize,
        h: usize,
        range: std::ops::Range<usize>,
    ) {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        for (_, r) in self
            .refs
            .iter()
            .enumerate()
            .filter(|(k, r)| r.visible && range.contains(k))
        {
            match &r.kind {
                RefKind::Image(il) => il.paint(painter, &self.plan, rect, w, h, r.opacity),
                RefKind::Raster { grid, tex, .. } => {
                    let (a, b) = (
                        self.plan
                            .to_screen(grid.x0, grid.y0 + grid.rows as f64 * grid.cell, w, h),
                        self.plan
                            .to_screen(grid.x0 + grid.cols as f64 * grid.cell, grid.y0, w, h),
                    );
                    let rr = Rect::from_min_max(
                        Pos2::new(a.0 as f32, a.1 as f32) + rect.min.to_vec2(),
                        Pos2::new(b.0 as f32, b.1 as f32) + rect.min.to_vec2(),
                    );
                    painter.with_clip_rect(rect).image(
                        tex.id(),
                        rr,
                        uv,
                        Color32::from_white_alpha((r.opacity.clamp(0.0, 1.0) * 255.0) as u8),
                    );
                }
                RefKind::Vector(_) => {}
            }
        }
    }

    /// Índices de las capas de imagen en `refs`.
    fn image_layers(&self) -> Vec<usize> {
        self.refs
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.kind, RefKind::Image(_)))
            .map(|(k, _)| k)
            .collect()
    }

    pub fn rs_window(&mut self, ctx: &egui::Context) {
        if !self.rs.open {
            return;
        }
        let mut open = true;
        let accent = self.tokens().accent;
        let images = self.image_layers();
        crate::procwin::window(ctx, "Índices espectrales")
            .open(&mut open)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                if images.is_empty() {
                    ui.label("Carga una imagen multibanda (GeoTIFF) con «Abrir».");
                    return;
                }
                if !images.contains(&self.rs.layer) {
                    self.rs.layer = images[0];
                }
                let (nb, reduction) = match &self.refs[self.rs.layer].kind {
                    RefKind::Image(i) => (i.work.bands.len(), i.reduction),
                    _ => (0, 1),
                };
                ui.horizontal(|ui| {
                    ui.label("Imagen");
                    egui::ComboBox::from_id_salt("Imagen")
                        .selected_text(self.refs[self.rs.layer].name.clone())
                        .show_ui(ui, |ui| {
                            for &k in &images {
                                let name = self.refs[k].name.clone();
                                ui.selectable_value(&mut self.rs.layer, k, name);
                            }
                        });
                });
                ui.label(format!(
                    "{nb} banda(s){}",
                    if reduction > 1 {
                        format!(" · el índice se calcula con 1 de cada {reduction} píxeles")
                    } else {
                        String::new()
                    }
                ));
                let band = |ui: &mut egui::Ui, label: &str, v: &mut usize| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(label)
                            .selected_text(format!("Banda {}", *v + 1))
                            .show_ui(ui, |ui| {
                                for b in 0..nb {
                                    ui.selectable_value(v, b, format!("Banda {}", b + 1));
                                }
                            });
                    });
                };
                ui.horizontal(|ui| {
                    ui.label("Índice");
                    egui::ComboBox::from_id_salt("Índice")
                        .selected_text(INDICES[self.rs.preset].0)
                        .show_ui(ui, |ui| {
                            for (k, (name, expr)) in INDICES.iter().enumerate() {
                                if ui.selectable_value(&mut self.rs.preset, k, *name).clicked() {
                                    self.rs.expr = (*expr).to_string();
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Expresión");
                    ui.add(egui::TextEdit::singleline(&mut self.rs.expr).desired_width(300.0));
                });
                ui.label(
                    egui::RichText::new(
                        "Usa RED, GREEN, BLUE, NIR, SWIR1, SWIR2 o B1…Bn con + − × ÷ y paréntesis.",
                    )
                    .size(11.0),
                );
                ui.label("¿Qué banda es cada una?");
                egui::Grid::new("roles").show(ui, |ui| {
                    for (k, name) in ROLES.iter().enumerate() {
                        let mut v = self.rs.roles[k].min(nb.saturating_sub(1));
                        band(ui, name, &mut v);
                        self.rs.roles[k] = v;
                        if k % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                if ui.add(crate::procwin::primary("Calcular índice", accent)).clicked() {
                    self.compute_index(ctx);
                }
                if let Some((name, _)) = &self.rs.last {
                    ui.label(format!("Último: {name}"));
                    if ui.button("Guardar como GeoTIFF…").clicked() {
                        self.save_index();
                    }
                }
            });
        self.rs.open = open;
    }

    pub fn compute_index(&mut self, ctx: &egui::Context) {
        let Some(RefKind::Image(il)) = self.refs.get(self.rs.layer).map(|r| &r.kind) else {
            return;
        };
        let roles: Vec<(&str, usize)> = ROLES
            .iter()
            .copied()
            .zip(self.rs.roles.iter().copied())
            .collect();
        let result = Expr::parse(&self.rs.expr, &roles, il.work.bands.len())
            .and_then(|e| raster::index(&il.work, &e));
        let label = INDICES
            .get(self.rs.preset)
            .map_or("indice", |i| i.0.split(' ').next().unwrap_or("indice"))
            .to_string();
        let base = self.refs[self.rs.layer].path.clone();
        match result {
            Ok(g) => {
                let name = format!("{}_{label}", hipso_core::paths::stem(&base));
                let path = base.with_file_name(format!("{name}.tif"));
                self.add_grid(ctx, name.clone(), path, g.clone());
                self.rs.last = Some((name, g));
            }
            Err(e) => self.log(format!("Índice: {e}")),
        }
    }

    fn save_index(&mut self) {
        let Some((name, g)) = &self.rs.last else {
            return;
        };
        if let Some(p) = crate::prefs::dialog()
            .add_filter("GeoTIFF", &["tif"])
            .set_file_name(format!("{name}.tif"))
            .save_file()
        {
            let r = g.write_tiff(&p, None);
            let msg = r.map_or_else(
                |e| format!("No se pudo guardar: {e}"),
                |()| format!("Índice guardado: {}", p.display()),
            );
            self.log(msg);
        }
    }
}

const ROLES: [&str; 6] = ["RED", "GREEN", "BLUE", "NIR", "SWIR1", "SWIR2"];

/// Estado de la ventana de índices.
pub struct RsState {
    pub open: bool,
    pub layer: usize,
    pub preset: usize,
    pub expr: String,
    /// Banda (desde 0) de ROJO, VERDE, AZUL, NIR, SWIR1, SWIR2.
    pub roles: [usize; 6],
    pub last: Option<(String, Grid)>,
    /// Línea de comandos `--ndvi`: calcula el primer índice al arrancar.
    pub compute_at_start: bool,
}

impl Default for RsState {
    fn default() -> Self {
        Self {
            open: false,
            layer: 0,
            preset: 0,
            expr: INDICES[0].1.to_string(),
            roles: [0, 1, 2, 3, 4, 5],
            last: None,
            compute_at_start: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un lado de 10980 píxeles (una tesela de Sentinel-2) es más de lo que muchas tarjetas
    /// gráficas admiten como textura.
    #[test]
    fn a_picture_too_big_for_a_texture_is_thinned_and_keeps_its_extent() {
        let (cols, rows) = (10980usize, 40usize);
        let img = BandImage {
            x0: 500_000.0,
            y0: 4_600_000.0,
            cell: 10.0,
            cols,
            rows,
            bands: vec![vec![1.0; cols * rows]; 2],
        };
        let r = reduced(&img, 4096);
        assert!(r.cols <= 4096 && r.rows >= 1);
        let wide = |i: &BandImage| i.cols as f64 * i.cell;
        assert!((wide(&r) - wide(&img)).abs() < 3.0 * r.cell, "{}", wide(&r));
        assert_eq!(r.bands.len(), 2);
        assert_eq!(r.bands[0].len(), r.cols * r.rows);
        // una pequeña no se copia
        let small = BandImage {
            cols: 10,
            rows: 10,
            bands: vec![vec![0.0; 100]],
            ..img
        };
        assert!(matches!(
            reduced(&small, 4096),
            std::borrow::Cow::Borrowed(_)
        ));
    }
}
