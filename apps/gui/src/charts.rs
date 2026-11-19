//! EDA (análisis exploratorio de datos): gráficos enlazados a la selección: un dock inferior con un
//! histograma, una dispersión y unas cajas de la misma fuente de datos, y una pestaña de
//! estadísticas. Marcar en un gráfico (un clic, un arrastre o un rectángulo) selecciona lo marcado
//! en el mapa y en la tabla, y lo que se selecciona en cualquier otro sitio se pinta en naranja
//! sobre los gráficos.
//!
//! La fuente puede ser una capa vectorial (cada fila de la tabla es un dato), una nube de puntos
//! (los atributos de la muestra que ve el visor; la selección es una regla, como en los
//! histogramas del editor, así que también vale para el archivo entero) o un ráster (solo para
//! mirar: no hay selección de celdas). Los gráficos se dibujan con el painter de egui, sin
//! dependencias nuevas; los cálculos pesados se guardan hasta que cambian los datos o la
//! selección, y la dispersión de muchos datos se pinta como densidad en lugar de un punto por
//! dato.

use std::collections::HashMap;

use super::*;
use crate::aba_plots::{nice_step, tick_text};
use crate::editing::Cmd;
use crate::layers::VecLayer;
use hipso_core::chartstats::{self, BoxStats, ColumnStats, Fit, Hist2};
use hipso_core::edit::{self, Attr, AttrKind, Mode, RangeSel, Region, SelOp};
use hipso_core::grid::Grid;
use hipso_core::histogram::{self, Histogram};
use hipso_core::view::PointStore;

/// Cuántos datos se pintan uno a uno en una dispersión; con más, se pinta su densidad.
const MAX_DOTS: usize = 30_000;
/// Cuántos grupos como mucho tiene un diagrama de cajas.
const MAX_GROUPS: usize = 16;
/// El naranja de lo seleccionado, como en el resto del programa.
const SELECTED: Color32 = Color32::from_rgb(255, 150, 40);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Charts,
    Stats,
}

/// De dónde salen los datos.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Src {
    /// Una capa de `App.refs` (vectorial o ráster).
    Layer(usize),
    /// Los atributos de la nube de puntos abierta.
    Points,
}

/// Una columna de la fuente.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Col {
    Field(usize),
    Attr(AttrKind),
    Cells,
}

#[derive(Clone, Debug)]
struct ColDef {
    name: String,
    col: Col,
    /// Valores enteros: el histograma tiene una barra por valor.
    integer: bool,
}

/// Los datos de una fuente prestados al dock durante un fotograma.
enum Data<'a> {
    Vector(&'a VecLayer),
    Raster(&'a Grid),
    Points(&'a PointStore),
}

impl Data<'_> {
    fn len(&self) -> usize {
        match self {
            Data::Vector(v) => v.data.features.len(),
            Data::Raster(g) => g.data.len(),
            Data::Points(s) => s.len(),
        }
    }

    /// Las columnas numéricas.
    fn cols(&self) -> Vec<ColDef> {
        match self {
            Data::Vector(v) => (0..v.data.fields.len())
                .filter(|&c| v.data.field_type(c).is_numeric())
                .map(|c| ColDef {
                    name: v.data.fields[c].clone(),
                    col: Col::Field(c),
                    integer: v.data.field_type(c) == hipso_core::fieldtype::FieldType::Integer,
                })
                .collect(),
            Data::Raster(_) => vec![ColDef { name: "Valor de la celda".into(), col: Col::Cells, integer: false }],
            Data::Points(_) => AttrKind::ALL
                .iter()
                .map(|k| ColDef { name: k.name().to_string(), col: Col::Attr(*k), integer: k.is_integer() })
                .collect(),
        }
    }

    /// Las columnas por las que se pueden agrupar los datos.
    fn group_cols(&self) -> Vec<ColDef> {
        match self {
            Data::Vector(v) => (0..v.data.fields.len())
                .filter(|&c| {
                    use hipso_core::fieldtype::FieldType::{Bool, Integer, Text};
                    matches!(v.data.field_type(c), Text | Integer | Bool)
                })
                .map(|c| ColDef { name: v.data.fields[c].clone(), col: Col::Field(c), integer: false })
                .collect(),
            Data::Raster(_) => Vec::new(),
            Data::Points(_) => [AttrKind::Class, AttrKind::Source, AttrKind::ReturnNumber, AttrKind::NumReturns]
                .iter()
                .map(|k| ColDef { name: k.name().to_string(), col: Col::Attr(*k), integer: true })
                .collect(),
        }
    }

    /// El valor de un dato; `None` si falta (celda vacía, no numérico, punto borrado o fuera de la
    /// zona de edición).
    fn value(&self, col: Col, i: usize) -> Option<f64> {
        let v = match (self, col) {
            (Data::Vector(v), Col::Field(c)) => *v.data.num.get(i)?.get(c)?,
            (Data::Raster(g), Col::Cells) => f64::from(*g.data.get(i)?),
            (Data::Points(s), Col::Attr(k)) => {
                if s.deleted.get(i).copied().unwrap_or(false) || s.outside.get(i).copied().unwrap_or(false) {
                    return None;
                }
                edit::value_of(s, i, k)
            }
            _ => return None,
        };
        v.is_finite().then_some(v)
    }

    /// El grupo de un dato (el texto del campo, o el valor entero del atributo).
    fn label(&self, col: Col, i: usize) -> Option<String> {
        match (self, col) {
            (Data::Vector(v), Col::Field(c)) => {
                let t = v.data.text.get(i)?.get(c)?.trim();
                Some(if t.is_empty() { "(vacío)".to_string() } else { t.to_string() })
            }
            (Data::Points(_), Col::Attr(k)) => {
                let v = self.value(col, i)?;
                Some(if k == AttrKind::Class {
                    format!("{} · {}", v as i64, hipso_core::classes::name(v as u8))
                } else {
                    format!("{}", v as i64)
                })
            }
            _ => None,
        }
    }

    /// El número de grupo de cada dato por los valores de `col` (`u32::MAX` si no tiene) y la
    /// etiqueta de cada grupo. Se numeran de una pasada, sin crear un texto por dato.
    fn group_ids(&self, col: Col) -> (Vec<u32>, Vec<String>) {
        let n = self.len();
        let mut ids = vec![u32::MAX; n];
        let mut labels: Vec<String> = Vec::new();
        match (self, col) {
            (Data::Vector(v), Col::Field(c)) => {
                let mut map: HashMap<&str, u32> = HashMap::new();
                for (i, id) in ids.iter_mut().enumerate() {
                    let Some(t) = v.data.text.get(i).and_then(|r| r.get(c)) else { continue };
                    let t = t.trim();
                    let k = *map.entry(t).or_insert_with(|| {
                        labels.push(if t.is_empty() { "(vacío)".to_string() } else { t.to_string() });
                        (labels.len() - 1) as u32
                    });
                    *id = k;
                }
            }
            (Data::Points(_), Col::Attr(a)) => {
                let mut map: HashMap<i64, u32> = HashMap::new();
                for (i, id) in ids.iter_mut().enumerate() {
                    let Some(v) = self.value(col, i) else { continue };
                    let key = v as i64;
                    let k = *map.entry(key).or_insert_with(|| {
                        labels.push(if a == AttrKind::Class {
                            format!("{key} · {}", hipso_core::classes::name(key as u8))
                        } else {
                            format!("{key}")
                        });
                        (labels.len() - 1) as u32
                    });
                    *id = k;
                }
            }
            _ => {}
        }
        (ids, labels)
    }

    fn selected(&self, i: usize) -> bool {
        match self {
            Data::Vector(v) => v.selected.contains(&i),
            Data::Points(s) => s.selected.get(i).copied().unwrap_or(false),
            Data::Raster(_) => false,
        }
    }

    fn n_selected(&self) -> usize {
        match self {
            Data::Vector(v) => v.selected.len(),
            Data::Points(s) => s.selected.iter().filter(|b| **b).count(),
            Data::Raster(_) => 0,
        }
    }

    /// Una firma de la selección que cambia cuando cambia: para repetir los cálculos del naranja.
    fn selection_sig(&self, point_rev: u64) -> u64 {
        match self {
            Data::Vector(v) => v.selected.iter().fold(v.selected.len() as u64, |a, i| a ^ (*i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            Data::Points(_) => point_rev,
            Data::Raster(_) => 0,
        }
    }

    /// Una firma de los datos que cambia cuando cambian.
    fn data_sig(&self, point_rev: u64) -> u64 {
        match self {
            Data::Vector(v) => hash_of((v.rev, v.undo.len(), v.data.features.len(), v.data.fields.len())),
            Data::Raster(g) => hash_of((g.data.len(), g.data.first().map(|v| v.to_bits()), g.data.last().map(|v| v.to_bits()))),
            Data::Points(s) => hash_of((point_rev, s.len())),
        }
    }

    fn can_select(&self) -> bool {
        !matches!(self, Data::Raster(_))
    }
}

/// Lo que se calcula de los datos y solo cambia con ellos.
struct Base {
    hist: Option<Histogram>,
    /// Rango de x y de y de la dispersión.
    range: Option<[f64; 4]>,
    dots: Vec<(u32, f64, f64)>,
    h2: Option<Hist2>,
    fit: Option<Fit>,
    /// Los grupos de las cajas, con su caja (las del grupo vacío no se dibujan).
    groups: Vec<(String, Option<BoxStats>)>,
    /// Número de grupos que había antes de quedarse con los más poblados.
    all_groups: usize,
    /// El número de grupo de cada dato y, por número de grupo, su sitio entre las cajas.
    gids: Vec<u32>,
    gslot: Vec<Option<usize>>,
}

/// Lo que se calcula de la selección.
struct Over {
    n: usize,
    /// Recuento de lo seleccionado en cada barra del histograma.
    hist: Vec<u64>,
    dots: Vec<(f64, f64)>,
    boxes: Vec<Option<BoxStats>>,
}

/// Una fila de la tabla de estadísticas.
struct StatRow {
    group: String,
    column: String,
    s: ColumnStats,
}

pub struct Charts {
    pub open: bool,
    pub height: f32,
    tab: Tab,
    src: Option<Src>,
    x: usize,
    y: usize,
    /// 0: sin grupos; k + 1: la columna `k` de [`Data::group_cols`].
    group: usize,
    bins: f64,
    log: bool,
    base: Option<(u64, Base)>,
    over: Option<(u64, Over)>,
    /// Si ya se eligieron las columnas por defecto de la fuente actual.
    auto: bool,
    /// Dónde se dibujó cada gráfico en el último fotograma.
    rects: [Rect; 3],
    export: Option<Export>,
    /// Qué se calculó con los datos, para saber si hay que repetirlo.
    stats_group: usize,
    stats_only_selected: bool,
    stats: Option<(u64, Vec<StatRow>)>,
    msg: String,
}

impl Charts {
    /// Si la fuente elegida es la nube, para las pruebas.
    #[cfg(test)]
    pub fn src_is_points_for_test(&self) -> bool {
        self.src == Some(Src::Points)
    }
}

impl Default for Charts {
    fn default() -> Self {
        Self {
            open: false,
            height: 280.0,
            tab: Tab::Charts,
            src: None,
            x: 0,
            y: 1,
            group: 0,
            bins: 30.0,
            log: false,
            base: None,
            over: None,
            auto: false,
            rects: [Rect::NOTHING; 3],
            export: None,
            stats_group: 0,
            stats_only_selected: false,
            stats: None,
            msg: String::new(),
        }
    }
}

/// Qué gráfico se guarda como imagen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    /// El histograma (0), la dispersión (1) o las cajas (2).
    One(usize),
    /// Los tres, juntos.
    All,
}

impl Want {
    fn file_name(self) -> &'static str {
        match self {
            Want::One(0) => "eda_histograma.png",
            Want::One(1) => "eda_dispersion.png",
            Want::One(_) => "eda_cajas.png",
            Want::All => "eda_graficos.png",
        }
    }
}

/// Una imagen que se está guardando: egui entrega la captura de la ventana entera unos fotogramas
/// después de pedirla, y de ella se recorta el gráfico.
struct Export {
    path: std::path::PathBuf,
    want: Want,
    /// Fotogramas que faltan para pedir la captura (que se cierre el menú y se repinte).
    wait: u8,
    requested: bool,
}

/// Lo que hay que hacer con la selección tras dibujar, cuando ya no está prestado nada.
enum Pick {
    /// Filas de la capa vectorial; modo 0 nueva, 1 añadir, 2 quitar.
    Rows(Vec<usize>, u8),
    /// Intervalos de atributos de los puntos (todos a la vez) y cómo se combinan.
    Ranges(Vec<RangeSel>, Mode),
    Clear,
}

/// Lo que hizo el ratón sobre un gráfico.
enum Gesture {
    None,
    Dragging(Rect),
    Done { rect: Rect, mods: egui::Modifiers },
    Click { pos: Pos2, mods: egui::Modifiers },
}

fn gesture(ui: &egui::Ui, resp: &egui::Response) -> Gesture {
    let mods = ui.input(|i| i.modifiers);
    // el punto donde empezó el arrastre se guarda al empezar: al soltar, el puntero ya no lo da
    let key = resp.id.with("brush_start");
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        ui.data_mut(|d| d.insert_temp(key, o));
    }
    let start: Option<Pos2> = ui.data(|d| d.get_temp(key));
    if resp.dragged_by(egui::PointerButton::Primary)
        && let (Some(a), Some(b)) = (start, resp.interact_pointer_pos())
    {
        return Gesture::Dragging(Rect::from_two_pos(a, b));
    }
    if resp.drag_stopped_by(egui::PointerButton::Primary)
        && let (Some(a), Some(b)) = (start, resp.interact_pointer_pos().or(ui.input(|i| i.pointer.latest_pos())))
    {
        return Gesture::Done { rect: Rect::from_two_pos(a, b), mods };
    }
    if resp.clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = resp.interact_pointer_pos()
    {
        return Gesture::Click { pos, mods };
    }
    Gesture::None
}

/// El modo con que una marca se combina con la selección: reemplaza, con Mayús añade, con Alt quita.
fn mode_of(m: egui::Modifiers) -> u8 {
    if m.alt {
        2
    } else if m.shift {
        1
    } else {
        0
    }
}

/// Dónde se dibujó cada gráfico y qué imagen se pidió guardar con el menú del botón derecho.
struct Out<'a> {
    rects: &'a mut [Rect; 3],
    want: &'a mut Option<Want>,
    /// Si los gráficos seleccionan (los rásteres solo se miran).
    select: bool,
}

impl Out<'_> {
    /// Anota el rectángulo del gráfico `slot` y le pone el menú del botón derecho.
    fn register(&mut self, slot: usize, resp: &egui::Response) {
        self.rects[slot] = resp.rect;
        let want = &mut *self.want;
        let tip = if self.select {
            "Arrastra para seleccionar · Mayús añade · Alt quita · botón derecho: guardar la imagen"
        } else {
            "Botón derecho: guardar la imagen"
        };
        resp.clone().on_hover_text(tip).context_menu(|ui| {
            if ui.button("Guardar esta imagen…").clicked() {
                *want = Some(Want::One(slot));
                ui.close();
            }
            if ui.button("Guardar los tres gráficos…").clicked() {
                *want = Some(Want::All);
                ui.close();
            }
        });
    }
}

/// El rectángulo de pantalla de la imagen que se guarda, en puntos.
fn export_rect(st: &Charts, want: Want) -> Option<Rect> {
    let r = match want {
        Want::One(i) => *st.rects.get(i)?,
        Want::All => st.rects.iter().fold(Rect::NOTHING, |a, r| if r.is_positive() { a.union(*r) } else { a }),
    };
    r.is_positive().then_some(r)
}

/// Guarda en `path` (PNG) el trozo `rect` (en puntos) de una captura de la ventana. Devuelve el
/// tamaño de la imagen en píxeles.
fn save_crop(image: &egui::ColorImage, rect: Rect, pixels_per_point: f32, path: &std::path::Path) -> Result<[usize; 2], String> {
    let [w, h] = image.size;
    let px = |v: f32, max: usize| ((v * pixels_per_point).round().max(0.0) as usize).min(max);
    let (x0, y0, x1, y1) = (px(rect.left(), w), px(rect.top(), h), px(rect.right(), w), px(rect.bottom(), h));
    if x1 <= x0 || y1 <= y0 {
        return Err("el gráfico no está a la vista".into());
    }
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut rgba = Vec::with_capacity(cw * ch * 4);
    for y in y0..y1 {
        for x in x0..x1 {
            let c = image.pixels[y * w + x];
            rgba.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
        }
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), cw as u32, ch as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok([cw, ch])
}

/// `--window-png ruta`: la ventana entera guardada como PNG pasados unos fotogramas, para capturas
/// repetibles de la interfaz sin tener que abrir la pantalla del sistema (y sin que salga nada más
/// que HIPSO).
pub struct WindowShot {
    path: std::path::PathBuf,
    started: std::time::Instant,
    /// Segundos que se esperan antes de la captura (`--shot-wait`; por defecto, 4).
    pub wait_secs: f64,
    requested: bool,
    done: bool,
}

impl WindowShot {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path, started: std::time::Instant::now(), wait_secs: 4.0, requested: false, done: false }
    }

    pub fn advance(&mut self, ctx: &egui::Context) {
        if self.done {
            return;
        }
        let left = self.wait_secs - self.started.elapsed().as_secs_f64();
        if left > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        if !self.requested {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|ev| match ev {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        match shot {
            Some(image) => {
                let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(image.size[0] as f32, image.size[1] as f32));
                let r = save_crop(&image, rect, 1.0, &self.path);
                eprintln!("{}", r.map_or_else(|e| format!("no se guardó la ventana: {e}"), |s| format!("ventana guardada ({} × {}): {}", s[0], s[1], self.path.display())));
                self.done = true;
            }
            None => ctx.request_repaint(),
        }
    }
}

/// Hace avanzar el guardado de una imagen: espera unos fotogramas, pide la captura de la ventana y
/// cuando llega recorta el gráfico y lo escribe.
fn advance_export(ctx: &egui::Context, st: &mut Charts) {
    let Some(e) = &mut st.export else { return };
    if e.wait > 0 {
        e.wait -= 1;
        ctx.request_repaint();
        return;
    }
    if !e.requested {
        e.requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        ctx.request_repaint();
        return;
    }
    let shot = ctx.input(|i| {
        i.events.iter().find_map(|ev| match ev {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    let Some(image) = shot else {
        ctx.request_repaint();
        return;
    };
    let Some(e) = st.export.take() else { return };
    st.msg = match export_rect(st, e.want) {
        Some(rect) => match save_crop(&image, rect, ctx.pixels_per_point(), &e.path) {
            Ok([w, h]) => format!("Imagen guardada ({w} × {h}): {}", e.path.display()),
            Err(m) => format!("No se pudo guardar la imagen: {m}"),
        },
        None => "No se pudo guardar la imagen: el gráfico no se ha dibujado".to_string(),
    };
}

/// Un gráfico con ejes: el rectángulo del trazado y el paso de coordenadas de datos a pantalla.
struct Frame {
    resp: egui::Response,
    painter: egui::Painter,
    plot: Rect,
    lo: [f64; 2],
    hi: [f64; 2],
    ylog: bool,
}

impl Frame {
    fn y_of(&self, y: f64) -> f64 {
        if self.ylog { (1.0 + y.max(0.0)).ln() } else { y }
    }

    fn to(&self, x: f64, y: f64) -> Pos2 {
        let y = self.y_of(y);
        Pos2::new(
            self.plot.left() + ((x - self.lo[0]) / (self.hi[0] - self.lo[0])) as f32 * self.plot.width(),
            self.plot.bottom() - ((y - self.y_of(self.lo[1])) / (self.y_of(self.hi[1]) - self.y_of(self.lo[1]))) as f32 * self.plot.height(),
        )
    }

    fn from(&self, p: Pos2) -> (f64, f64) {
        let fx = f64::from((p.x - self.plot.left()) / self.plot.width());
        let fy = f64::from((self.plot.bottom() - p.y) / self.plot.height());
        let x = self.lo[0] + fx * (self.hi[0] - self.lo[0]);
        let (ya, yb) = (self.y_of(self.lo[1]), self.y_of(self.hi[1]));
        let y = ya + fy * (yb - ya);
        (x, if self.ylog { y.exp() - 1.0 } else { y })
    }
}

/// El texto de una marca de un eje: los millones se abrevian («200M») para que no se pisen.
fn axis_text(v: f64, step: f64) -> String {
    if v.abs() >= 1e6 && step >= 1e5 {
        let m = v / 1e6;
        if (m - m.round()).abs() < 1e-9 { format!("{}M", m.round() as i64) } else { format!("{m:.1}M") }
    } else {
        tick_text(v, step)
    }
}

/// Cuántas marcas caben en `len` puntos de eje si cada texto mide como `sample` (con un hueco).
fn ticks_that_fit(len: f32, sample: &str) -> f64 {
    let each = sample.chars().count() as f32 * 6.6 + 18.0;
    f64::from((len / each).floor().clamp(2.0, 6.0))
}

/// El marco de un gráfico: fondo, título, cuadrícula, marcas y rótulos de los ejes. `lo` y `hi` son
/// los límites de los datos; con `ylog` el eje vertical es logarítmico (de 1 + y).
fn frame(ui: &mut egui::Ui, size: Vec2, title: &str, xl: &str, yl: &str, lo: [f64; 2], hi: [f64; 2], ylog: bool, yint: bool) -> Frame {
    let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let v = ui.visuals();
    let (fg, weak) = (v.text_color(), v.weak_text_color());
    let grid = v.widgets.noninteractive.bg_stroke.color;
    let r = resp.rect;
    painter.rect_filled(r, 3.0, v.extreme_bg_color);
    painter.text(Pos2::new(r.left() + 6.0, r.top() + 3.0), Align2::LEFT_TOP, title, FontId::proportional(12.0), fg);
    let plot = Rect::from_min_max(Pos2::new(r.left() + 58.0, r.top() + 22.0), Pos2::new(r.right() - 10.0, r.bottom() - 32.0));
    let (mut lo, mut hi) = (lo, hi);
    for k in 0..2 {
        if !(hi[k] > lo[k]) {
            lo[k] -= 1.0;
            hi[k] += 1.0;
        }
    }
    let f = Frame { resp, painter, plot, lo, hi, ylog };
    // x: tantas marcas como quepan sin pisarse
    let widest = [lo[0], hi[0]].iter().map(|v| axis_text(*v, nice_step(hi[0] - lo[0], 4.0))).max_by_key(|t| t.chars().count()).unwrap_or_default();
    let step = nice_step(hi[0] - lo[0], ticks_that_fit(plot.width(), &widest));
    let mut t = (lo[0] / step).ceil() * step;
    while t <= hi[0] {
        let x = f.to(t, lo[1]).x;
        f.painter.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(0.5, grid));
        f.painter.text(Pos2::new(x, plot.bottom() + 3.0), Align2::CENTER_TOP, axis_text(t, step), FontId::proportional(10.0), weak);
        t += step;
    }
    // y
    if ylog {
        let mut c = 1.0f64;
        while c <= hi[1] {
            let y = f.to(lo[0], c).y;
            f.painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(0.5, grid));
            f.painter.text(Pos2::new(plot.left() - 4.0, y), Align2::RIGHT_CENTER, format!("{c:.0}"), FontId::proportional(10.0), weak);
            c *= 10.0;
        }
    } else {
        let step = nice_step(hi[1] - lo[1], 4.0);
        // un recuento solo tiene marcas en números enteros
        let step = if yint { step.max(1.0) } else { step };
        let mut t = (lo[1] / step).ceil() * step;
        while t <= hi[1] {
            let y = f.to(lo[0], t).y;
            f.painter.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(0.5, grid));
            f.painter.text(Pos2::new(plot.left() - 4.0, y), Align2::RIGHT_CENTER, axis_text(t, step), FontId::proportional(10.0), weak);
            t += step;
        }
    }
    f.painter.rect_stroke(plot, 0.0, Stroke::new(1.0, grid), egui::StrokeKind::Inside);
    f.painter.text(Pos2::new(plot.center().x, r.bottom() - 2.0), Align2::CENTER_BOTTOM, xl, FontId::proportional(11.0), weak);
    let galley = f.painter.layout_no_wrap(yl.to_string(), FontId::proportional(11.0), weak);
    let gw = galley.size().x.min(plot.height());
    f.painter.add(egui::epaint::TextShape::new(Pos2::new(r.left() + 4.0, plot.center().y + gw / 2.0), galley, weak).with_angle(-std::f32::consts::FRAC_PI_2));
    f
}

/// Texto pequeño en la esquina superior derecha del trazado.
fn corner_text(f: &Frame, ui: &egui::Ui, s: String) {
    // dentro del trazado, en dos líneas si hace falta, sobre un fondo suave: así no pisa las marcas
    // de los ejes ni lo que se dibuja debajo
    let galley = f.painter.layout(s, FontId::proportional(11.0), ui.visuals().text_color(), (f.plot.width() - 12.0).max(40.0));
    let at = Pos2::new(f.plot.right() - 4.0 - galley.size().x, f.plot.top() + 3.0);
    let bg = Rect::from_min_size(at - Vec2::new(3.0, 1.0), galley.size() + Vec2::new(6.0, 2.0));
    f.painter.rect_filled(bg, 3.0, ui.visuals().extreme_bg_color.gamma_multiply(0.85));
    f.painter.galley(at, galley, ui.visuals().text_color());
}

/// Un número para un rótulo: con tres decimales, o abreviado en millones cuando es muy grande.
fn num_short(v: f64) -> String {
    if v.abs() >= 1e6 {
        format!("{:.2}M", v / 1e6)
    } else {
        format!("{v:.3}")
    }
}

fn draw_brush(f: &Frame, rect: Rect, full_height: bool) {
    let r = if full_height { Rect::from_x_y_ranges(rect.x_range(), f.plot.y_range()) } else { rect };
    let r = r.intersect(f.plot);
    f.painter.rect_filled(r, 0.0, SELECTED.gamma_multiply(0.18));
    f.painter.rect_stroke(r, 0.0, Stroke::new(1.0, SELECTED), egui::StrokeKind::Inside);
}

impl App {
    /// El alto que ocupa el dock de gráficos en una zona de `total` puntos de alto.
    pub fn charts_height(&self, total: f32) -> f32 {
        if !self.ch.open {
            return 0.0;
        }
        self.ch.height.clamp(170.0, (total * 0.7).max(240.0)).min(total)
    }

    /// Las fuentes que ofrece el dock, con su nombre.
    fn chart_sources(&self) -> Vec<(Src, String)> {
        let mut v: Vec<(Src, String)> = self
            .refs
            .iter()
            .enumerate()
            .filter_map(|(k, r)| match &r.kind {
                RefKind::Vector(_) => Some((Src::Layer(k), format!("{} (capa)", r.name))),
                RefKind::Raster { .. } => Some((Src::Layer(k), format!("{} (ráster)", r.name))),
                RefKind::Image(_) => None,
            })
            .collect();
        if self.store.is_some() {
            v.push((Src::Points, "Nube de puntos".to_string()));
        }
        v
    }

    /// Elige una fuente por defecto: la capa de la tabla abierta o activa, si no la nube y si no la
    /// primera que haya.
    fn default_chart_source(&self) -> Option<Src> {
        let sources = self.chart_sources();
        let wanted = self.table_layer.or(self.active).map(Src::Layer);
        wanted
            .filter(|w| sources.iter().any(|s| s.0 == *w))
            .or_else(|| sources.iter().map(|s| s.0).find(|s| matches!(s, Src::Layer(k) if matches!(self.refs[*k].kind, RefKind::Vector(_)))))
            .or_else(|| sources.iter().map(|s| s.0).find(|s| *s == Src::Points))
            .or_else(|| sources.first().map(|s| s.0))
    }

    fn chart_data(&self, src: Src) -> Option<Data<'_>> {
        match src {
            Src::Layer(k) => match &self.refs.get(k)?.kind {
                RefKind::Vector(v) => Some(Data::Vector(v)),
                RefKind::Raster { grid, .. } => Some(Data::Raster(grid)),
                RefKind::Image(_) => None,
            },
            Src::Points => self.store.as_deref().map(Data::Points),
        }
    }

    /// El dock de gráficos, en `area`, la franja inferior de la zona de las vistas.
    pub fn charts_dock(&mut self, ui: &mut egui::Ui, area: Rect, max_h: f32) {
        if !self.ch.open {
            return;
        }
        let t = self.tokens();
        let mut st = std::mem::take(&mut self.ch);
        if st.src.is_none_or(|s| self.chart_data(s).is_none()) {
            st.src = self.default_chart_source();
            st.auto = false;
            st.base = None;
            st.over = None;
            st.stats = None;
        }
        let sources = self.chart_sources();
        let point_rev = self.rev;
        let sel_rev = self.ed.sel_rev;
        let mut pick: Option<Pick> = None;
        let mut want: Option<Want> = None;
        let mut close = false;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(area.intersect(ui.clip_rect()));
        egui::Frame::new()
            .fill(t.pane)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .stroke(Stroke::new(1.0, t.border))
            .show(&mut child, |ui| {
                ui.set_min_size(Vec2::new(area.width() - 18.0, area.height() - 10.0));
                ui.set_max_size(Vec2::new(area.width() - 18.0, area.height() - 10.0));
                let grip = Rect::from_min_size(Pos2::new(area.left(), area.top() - 3.0), Vec2::new(area.width(), 8.0));
                let g = ui.interact(grip, ui.id().with("charts_grip"), Sense::drag());
                if g.hovered() || g.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
                if g.dragged() {
                    st.height = (st.height - g.drag_delta().y).clamp(170.0, max_h);
                }
                // ---- barra: fuente, pestañas, selección y cierre
                ui.horizontal(|ui| {
                    crate::ribbon::draw_icon(ui.painter(), ui.cursor().left_center() + Vec2::new(9.0, 0.0), 15.0, "@charts", t.text);
                    ui.add_space(20.0);
                    ui.label(egui::RichText::new("EDA").strong());
                    let current = sources.iter().find(|s| Some(s.0) == st.src).map_or("(sin datos)".to_string(), |s| s.1.clone());
                    egui::ComboBox::from_id_salt("charts_src")
                        .width(200.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for (s, name) in &sources {
                                if ui.selectable_label(Some(*s) == st.src, name).clicked() {
                                    st.src = Some(*s);
                                    st.auto = false;
                                    st.x = 0;
                                    st.y = 1;
                                    st.group = 0;
                                    st.base = None;
                                    st.over = None;
                                    st.stats = None;
                                }
                            }
                        });
                    ui.selectable_value(&mut st.tab, Tab::Charts, "Gráficos");
                    ui.selectable_value(&mut st.tab, Tab::Stats, "Estadísticas");
                    if let Some(d) = st.src.and_then(|s| self.chart_data(s)) {
                        let n = d.n_selected();
                        ui.label(egui::RichText::new(if d.can_select() { format!("{n} seleccionados de {}", d.len()) } else { format!("{} celdas", d.len()) }).size(11.5).color(t.weak));
                        if d.can_select() && n > 0 && ui.small_button("Limpiar selección").clicked() {
                            pick = Some(Pick::Clear);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::ribbon::icon_button(ui, &t, "@close", "Cerrar el EDA").clicked() {
                            close = true;
                        }
                        if st.tab == Tab::Charts {
                            ui.menu_button("Guardar imagen", |ui| {
                                for (label, w) in [
                                    ("Histograma…", Want::One(0)),
                                    ("Dispersión…", Want::One(1)),
                                    ("Cajas…", Want::One(2)),
                                    ("Los tres juntos…", Want::All),
                                ] {
                                    if ui.button(label).clicked() {
                                        want = Some(w);
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Guarda un gráfico como imagen PNG, tal como se ve (con la selección en naranja). También con el botón derecho sobre el gráfico.");
                        }
                        if !st.msg.is_empty() {
                            ui.label(egui::RichText::new(&st.msg).size(11.0).color(t.weak));
                        }
                    });
                });
                ui.add_space(2.0);
                let Some(data) = st.src.and_then(|s| self.chart_data(s)) else {
                    ui.label("Abre una capa vectorial, una nube de puntos o un ráster para ver sus gráficos.");
                    return;
                };
                match st.tab {
                    Tab::Charts => charts_body(ui, &mut st, &data, point_rev, sel_rev, &t, &mut pick, &mut want, area.width() - 18.0),
                    Tab::Stats => stats_body(ui, &mut st, &data, point_rev, sel_rev, &t),
                }
            });
        let ctx = ui.ctx().clone();
        if let Some(w) = want
            && let Some(path) = crate::prefs::dialog().add_filter("Imagen PNG", &["png"]).set_file_name(w.file_name()).save_file()
        {
            st.export = Some(Export { path, want: w, wait: 3, requested: false });
            ctx.request_repaint();
        }
        advance_export(&ctx, &mut st);
        self.ch = st;
        if close {
            self.ch.open = false;
        }
        if let Some(p) = pick {
            self.apply_pick(p);
        }
    }

    /// Interruptor de la línea de comandos `--eda-png ruta`: abre el EDA y guarda en esa ruta, en
    /// cuanto estén dibujados, los tres gráficos (para comprobar la exportación y para las capturas
    /// del manual).
    pub fn eda_png_at_start(&mut self, path: std::path::PathBuf) {
        self.ch.open = true;
        self.ch.export = Some(Export { path, want: Want::All, wait: 40, requested: false });
    }

    /// Dónde se dibujó cada gráfico, para las pruebas.
    #[cfg(test)]
    pub fn ch_rects_for_test(&self) -> [Rect; 3] {
        self.ch.rects
    }

    /// La fuente elegida, para las pruebas.
    #[cfg(test)]
    pub fn ch_src_for_test(&self) -> Option<usize> {
        match self.ch.src {
            Some(Src::Layer(k)) => Some(k),
            _ => None,
        }
    }

    /// Aplica a la capa o a la nube lo que se marcó en un gráfico.
    fn apply_pick(&mut self, pick: Pick) {
        let Some(src) = self.ch.src else { return };
        match (pick, src) {
            (Pick::Rows(rows, mode), Src::Layer(k)) => {
                if let Some(RefKind::Vector(v)) = self.refs.get_mut(k).map(|r| &mut r.kind) {
                    match mode {
                        0 => v.selected = rows.into_iter().collect(),
                        1 => v.selected.extend(rows),
                        _ => {
                            for r in rows {
                                v.selected.remove(&r);
                            }
                        }
                    }
                }
            }
            (Pick::Ranges(ranges, mode), Src::Points) => {
                let op = SelOp { mode, region: Region::All, attr: Attr { ranges, ..self.ed.attr.clone() } };
                self.queue(Cmd::AddOp(Box::new(op)));
            }
            (Pick::Clear, Src::Layer(_)) => self.clear_selection(),
            (Pick::Clear, Src::Points) => self.clear_selection(),
            _ => {}
        }
    }
}

/// El modo de selección de puntos que corresponde al de las teclas.
fn point_mode(mode: u8) -> Mode {
    match mode {
        0 => Mode::New,
        1 => Mode::Add,
        _ => Mode::Subtract,
    }
}

/// Los tres gráficos, uno junto a otro.
fn charts_body(ui: &mut egui::Ui, st: &mut Charts, data: &Data<'_>, point_rev: u64, sel_rev: u64, t: &Tokens, pick: &mut Option<Pick>, want: &mut Option<Want>, width: f32) {
    let cols = data.cols();
    if cols.is_empty() {
        ui.label("Esta fuente no tiene columnas numéricas que graficar.");
        return;
    }
    let groups = data.group_cols();
    if !st.auto {
        (st.x, st.y) = default_columns(data, &cols);
        st.auto = true;
    }
    st.x = st.x.min(cols.len() - 1);
    st.y = st.y.min(cols.len() - 1);
    if st.group > groups.len() {
        st.group = 0;
    }
    // ---- elección de columnas (la fila salta de línea si no cabe, sin ensanchar el dock)
    ui.horizontal_wrapped(|ui| {
        let combo = |ui: &mut egui::Ui, id: &str, label: &str, sel: &mut usize, list: &[ColDef]| {
            ui.label(label);
            egui::ComboBox::from_id_salt(id).width(130.0).selected_text(list[*sel].name.clone()).show_ui(ui, |ui| {
                for (k, c) in list.iter().enumerate() {
                    ui.selectable_value(sel, k, &c.name);
                }
            });
        };
        combo(ui, "charts_x", "X", &mut st.x, &cols);
        combo(ui, "charts_y", "Y", &mut st.y, &cols);
        if !groups.is_empty() {
            ui.label("Agrupar cajas por");
            let name = if st.group == 0 { "(sin grupos)".to_string() } else { groups[st.group - 1].name.clone() };
            egui::ComboBox::from_id_salt("charts_group").width(130.0).selected_text(name).show_ui(ui, |ui| {
                ui.selectable_value(&mut st.group, 0, "(sin grupos)");
                for (k, c) in groups.iter().enumerate() {
                    ui.selectable_value(&mut st.group, k + 1, &c.name);
                }
            });
        }
        if !cols[st.x].integer {
            ui.label("Barras");
            ui.add(crate::numfield::NumField::new(&mut st.bins).range(5.0..=200.0).speed(1.0));
        }
        ui.checkbox(&mut st.log, "Recuento logarítmico");
        if !data.can_select() {
            ui.label(egui::RichText::new("Los rásteres se miran, no se seleccionan.").size(11.0).color(t.weak));
        }
    });
    let (xc, yc) = (cols[st.x].clone(), cols[st.y].clone());
    let gcol = (st.group > 0).then(|| groups[st.group - 1].clone());
    // ---- cálculos de los datos
    let base_key = hash_of((data.data_sig(point_rev), xc.col, yc.col, gcol.as_ref().map(|g| g.col), st.bins as usize));
    if st.base.as_ref().is_none_or(|b| b.0 != base_key) {
        st.base = Some((base_key, compute_base(data, &xc, &yc, gcol.as_ref(), st.bins as usize)));
        st.over = None;
    }
    let over_key = hash_of((base_key, data.selection_sig(sel_rev)));
    if st.over.as_ref().is_none_or(|o| o.0 != over_key)
        && let Some((_, base)) = &st.base
    {
        st.over = Some((over_key, compute_over(data, base, &xc, &yc)));
    }
    let (Some((_, base)), Some((_, over))) = (&st.base, &st.over) else { return };
    // el ancho del dock, no el que haya dejado la fila de controles si es más ancha que él
    let w = ((width - 16.0) / 3.0).floor();
    let h = ui.available_height().max(120.0) - 2.0;
    let size = Vec2::new(w, h);
    let mut out = Out { rects: &mut st.rects, want, select: data.can_select() };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        histogram_chart(ui, size, st.log, base, over, &xc, data, pick, &mut out);
        scatter_chart(ui, size, base, over, &xc, &yc, data, pick, &mut out);
        box_chart(ui, size, base, over, &xc, gcol.as_ref(), data, pick, &mut out);
    });
}

/// Las columnas con que se abre una fuente: las dos primeras que no sean un identificador (una
/// columna de enteros distintos en cada fila, como el número de parcela). En una nube, las
/// elevaciones y la intensidad.
fn default_columns(data: &Data<'_>, cols: &[ColDef]) -> (usize, usize) {
    if matches!(data, Data::Points(_)) || cols.len() < 2 {
        return (0, 1.min(cols.len().saturating_sub(1)));
    }
    let n = data.len();
    let id_like = |c: &ColDef| -> bool {
        if !c.integer || n < 6 {
            return false;
        }
        let mut seen = std::collections::HashSet::new();
        let mut valid = 0usize;
        for i in 0..n {
            if let Some(v) = data.value(c.col, i) {
                valid += 1;
                seen.insert(v.to_bits());
            }
        }
        valid == n && seen.len() == n
    };
    // una columna con el mismo valor en todas las filas no da nada que mirar
    let constant = |c: &ColDef| -> bool {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..n {
            if let Some(v) = data.value(c.col, i) {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        !(hi > lo)
    };
    let keep: Vec<usize> = (0..cols.len()).filter(|k| !id_like(&cols[*k]) && !constant(&cols[*k])).collect();
    match keep.as_slice() {
        [a, b, ..] => (*a, *b),
        [a] => (*a, *a),
        [] => (0, 1.min(cols.len() - 1)),
    }
}

// ---------------------------------------------------------------------------------------------
// cálculos

fn compute_base(data: &Data<'_>, xc: &ColDef, yc: &ColDef, group: Option<&ColDef>, bins: usize) -> Base {
    let n = data.len();
    let hist = histogram::histogram(n, |i| data.value(xc.col, i), bins, xc.integer);
    // dispersión: rango, puntos o densidad, ajuste lineal
    let pair = |i: usize| Some((data.value(xc.col, i)?, data.value(yc.col, i)?));
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    let mut count = 0usize;
    for i in 0..n {
        if let Some((x, y)) = pair(i) {
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
            count += 1;
        }
    }
    let range = (count > 0).then_some([lo[0], lo[1], hi[0], hi[1]]);
    let mut dots = Vec::new();
    let mut h2 = None;
    if let Some(r) = range {
        if count <= MAX_DOTS {
            dots = (0..n).filter_map(|i| pair(i).map(|(x, y)| (i as u32, x, y))).collect();
        } else {
            let r = [r[0], r[1], r[2].max(r[0] + f64::MIN_POSITIVE), r[3].max(r[1] + f64::MIN_POSITIVE)];
            h2 = Some(chartstats::hist2d(n, pair, r, 160, 100));
        }
    }
    let fit = chartstats::linear_fit(n, pair);
    // cajas por grupo (sin grupo: una sola)
    let (gids, glabels) = match group {
        Some(g) => data.group_ids(g.col),
        None => (vec![0; n], vec![xc.name.clone()]),
    };
    let mut per: Vec<Vec<f64>> = vec![Vec::new(); glabels.len()];
    for (i, g) in gids.iter().enumerate() {
        if *g == u32::MAX {
            continue;
        }
        if let Some(v) = data.value(xc.col, i) {
            per[*g as usize].push(v);
        }
    }
    let all_groups = per.iter().filter(|v| !v.is_empty()).count();
    // los más poblados y, entre ellos, en orden alfabético (numérico cuando son números)
    let mut order: Vec<usize> = (0..glabels.len()).filter(|k| !per[*k].is_empty()).collect();
    order.sort_by(|a, b| per[*b].len().cmp(&per[*a].len()).then_with(|| glabels[*a].cmp(&glabels[*b])));
    order.truncate(MAX_GROUPS);
    order.sort_by(|a, b| natural(&glabels[*a]).total_cmp(&natural(&glabels[*b])).then_with(|| glabels[*a].cmp(&glabels[*b])));
    let mut gslot = vec![None; glabels.len()];
    let mut groups = Vec::new();
    for (slot, k) in order.into_iter().enumerate() {
        gslot[k] = Some(slot);
        groups.push((glabels[k].clone(), chartstats::box_stats(&per[k])));
    }
    Base { hist, range, dots, h2, fit, groups, all_groups, gids, gslot }
}

/// Un número para ordenar una etiqueta: el que lleva delante, si lo lleva.
fn natural(s: &str) -> f64 {
    let t: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
    t.parse().unwrap_or(f64::INFINITY)
}

fn compute_over(data: &Data<'_>, base: &Base, xc: &ColDef, yc: &ColDef) -> Over {
    let n = data.len();
    let sel: Vec<usize> = match data {
        Data::Vector(v) => {
            let mut s: Vec<usize> = v.selected.iter().copied().filter(|i| *i < n).collect();
            s.sort_unstable();
            s
        }
        Data::Points(s) => s.selected.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect(),
        Data::Raster(_) => Vec::new(),
    };
    let mut hist = vec![0u64; base.hist.as_ref().map_or(0, |h| h.counts.len())];
    if let Some(h) = &base.hist {
        for &i in &sel {
            if let Some(v) = data.value(xc.col, i) {
                hist[h.bar_of(v)] += 1;
            }
        }
    }
    let stride = sel.len().div_ceil(MAX_DOTS).max(1);
    let dots: Vec<(f64, f64)> = sel
        .iter()
        .step_by(stride)
        .filter_map(|&i| Some((data.value(xc.col, i)?, data.value(yc.col, i)?)))
        .collect();
    let mut per: Vec<Vec<f64>> = vec![Vec::new(); base.groups.len()];
    for &i in &sel {
        let Some(v) = data.value(xc.col, i) else { continue };
        let slot = base.gids.get(i).and_then(|g| base.gslot.get(*g as usize)).copied().flatten();
        if let Some(k) = slot {
            per[k].push(v);
        }
    }
    Over { n: sel.len(), hist, dots, boxes: per.iter().map(|v| chartstats::box_stats(v)).collect() }
}

// ---------------------------------------------------------------------------------------------
// los tres gráficos

#[allow(clippy::too_many_arguments)]
fn histogram_chart(ui: &mut egui::Ui, size: Vec2, log: bool, base: &Base, over: &Over, xc: &ColDef, data: &Data<'_>, pick: &mut Option<Pick>, out: &mut Out<'_>) {
    let Some(h) = &base.hist else {
        ui.allocate_ui(size, |ui| ui.label("No hay valores que contar."));
        return;
    };
    let nb = h.counts.len();
    let top = h.counts.iter().copied().max().unwrap_or(1).max(1) as f64;
    let f = frame(ui, size, &format!("Histograma de {}", xc.name), &xc.name, "recuento", [h.lo, 0.0], [h.lo + h.width * nb as f64, top * 1.05], log, true);
    out.register(0, &f.resp);
    let base_color = ui.visuals().hyperlink_color.gamma_multiply(0.7);
    let hover = f.resp.hover_pos().filter(|p| f.plot.contains(*p));
    let hover_bar = hover.map(|p| h.bar_of(f.from(p).0));
    let clipped = f.painter.with_clip_rect(f.plot);
    for i in 0..nb {
        let (a, b) = h.bar(i);
        let c = h.counts[i] as f64;
        let (p0, p1) = (f.to(a, 0.0), f.to(b, c));
        let r = Rect::from_min_max(Pos2::new(p0.x + 0.5, p1.y), Pos2::new(p1.x - 0.5, p0.y));
        clipped.rect_filled(r, 0.0, if hover_bar == Some(i) { base_color.gamma_multiply(1.4) } else { base_color });
        if let Some(&s) = over.hist.get(i)
            && s > 0
        {
            let ps = f.to(b, s as f64);
            clipped.rect_filled(Rect::from_min_max(Pos2::new(p0.x + 0.5, ps.y), Pos2::new(p1.x - 0.5, p0.y)), 0.0, SELECTED);
        }
    }
    if let Some(i) = hover_bar {
        let (a, b) = h.bar(i);
        corner_text(&f, ui, format!("[{a:.3}, {b:.3}): {} ({} sel.)", h.counts[i], over.hist.get(i).copied().unwrap_or(0)));
    } else {
        corner_text(&f, ui, format!("n = {} · media {} · mediana {}", h.n, num_short(h.mean), num_short(h.median)));
    }
    if !data.can_select() {
        return;
    }
    let bar_range = |x0: f64, x1: f64| -> (f64, f64) {
        let (i0, i1) = (h.bar_of(x0.min(x1)), h.bar_of(x0.max(x1)));
        let (lo, _) = h.bar(i0);
        let (_, mut hi) = h.bar(i1);
        if i1 + 1 == nb {
            hi = hi.next_up();
        }
        (lo, hi)
    };
    match gesture(ui, &f.resp) {
        Gesture::Dragging(r) => draw_brush(&f, r, true),
        Gesture::Done { rect, mods } => {
            let (a, b) = (f.from(rect.left_top()).0, f.from(rect.right_bottom()).0);
            *pick = select_range(data, xc, bar_range(a, b), mods);
        }
        Gesture::Click { pos, mods } if f.plot.contains(pos) => {
            let x = f.from(pos).0;
            *pick = select_range(data, xc, bar_range(x, x), mods);
        }
        _ => {}
    }
}

/// Selecciona los datos cuyo valor de `xc` cae en `[lo, hi)`.
fn select_range(data: &Data<'_>, xc: &ColDef, (lo, hi): (f64, f64), mods: egui::Modifiers) -> Option<Pick> {
    let mode = mode_of(mods);
    match (data, xc.col) {
        (Data::Points(_), Col::Attr(k)) => Some(Pick::Ranges(vec![RangeSel { kind: k, bins: vec![(lo, hi)] }], point_mode(mode))),
        (Data::Vector(_), col) => {
            let rows = (0..data.len()).filter(|&i| data.value(col, i).is_some_and(|v| v >= lo && v < hi)).collect();
            Some(Pick::Rows(rows, mode))
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn scatter_chart(ui: &mut egui::Ui, size: Vec2, base: &Base, over: &Over, xc: &ColDef, yc: &ColDef, data: &Data<'_>, pick: &mut Option<Pick>, out: &mut Out<'_>) {
    let Some(r) = base.range else {
        ui.allocate_ui(size, |ui| ui.label("No hay pares de valores que dibujar."));
        return;
    };
    let padx = (r[2] - r[0]).max(f64::MIN_POSITIVE) * 0.04;
    let pady = (r[3] - r[1]).max(f64::MIN_POSITIVE) * 0.04;
    let f = frame(ui, size, &format!("{} frente a {}", yc.name, xc.name), &xc.name, &yc.name, [r[0] - padx, r[1] - pady], [r[2] + padx, r[3] + pady], false, false);
    out.register(1, &f.resp);
    let clipped = f.painter.with_clip_rect(f.plot);
    let dot = ui.visuals().hyperlink_color;
    if let Some(h2) = &base.h2 {
        let cell_w = (f.to(h2.x0 + h2.dx, 0.0).x - f.to(h2.x0, 0.0).x).abs().max(1.0);
        let cell_h = (f.to(0.0, h2.y0).y - f.to(0.0, h2.y0 + h2.dy).y).abs().max(1.0);
        let top = f64::from(h2.max.max(1)).ln_1p();
        for iy in 0..h2.ny {
            for ix in 0..h2.nx {
                let c = h2.counts[iy * h2.nx + ix];
                if c == 0 {
                    continue;
                }
                let a = (f64::from(c).ln_1p() / top) as f32;
                let p = f.to(h2.x0 + ix as f64 * h2.dx, h2.y0 + (iy + 1) as f64 * h2.dy);
                clipped.rect_filled(Rect::from_min_size(p, Vec2::new(cell_w, cell_h)), 0.0, dot.gamma_multiply(0.25 + 0.75 * a));
            }
        }
    } else {
        for &(_, x, y) in &base.dots {
            clipped.circle_filled(f.to(x, y), 2.2, dot.gamma_multiply(0.75));
        }
    }
    if let Some(fit) = base.fit {
        let (a, b) = (r[0], r[2]);
        let line = [f.to(a, fit.intercept + fit.slope * a), f.to(b, fit.intercept + fit.slope * b)];
        for s in egui::Shape::dashed_line(&line, Stroke::new(1.2, ui.visuals().text_color()), 6.0, 4.0) {
            clipped.add(s);
        }
    }
    for &(x, y) in &over.dots {
        clipped.circle_filled(f.to(x, y), 2.8, SELECTED);
    }
    if let Some(fit) = base.fit {
        corner_text(&f, ui, format!("r = {:.3} · R² = {:.3} · y = {:.4}·x {:+.4}", fit.r, fit.r2(), fit.slope, fit.intercept));
    }
    if !data.can_select() {
        return;
    }
    match gesture(ui, &f.resp) {
        Gesture::Dragging(rect) => draw_brush(&f, rect, false),
        Gesture::Done { rect, mods } => {
            let (a, b) = (f.from(rect.left_bottom()), f.from(rect.right_top()));
            *pick = select_rect(data, xc, yc, [a.0, a.1, b.0, b.1], mods);
        }
        Gesture::Click { mods, .. } if !mods.shift && !mods.alt => *pick = Some(Pick::Clear),
        _ => {}
    }
}

/// Selecciona los datos dentro del rectángulo `[x0, y0, x1, y1]` de la dispersión.
fn select_rect(data: &Data<'_>, xc: &ColDef, yc: &ColDef, r: [f64; 4], mods: egui::Modifiers) -> Option<Pick> {
    let mode = mode_of(mods);
    match (data, xc.col, yc.col) {
        (Data::Points(_), Col::Attr(kx), Col::Attr(ky)) => Some(Pick::Ranges(
            vec![
                RangeSel { kind: kx, bins: vec![(r[0], r[2].next_up())] },
                RangeSel { kind: ky, bins: vec![(r[1], r[3].next_up())] },
            ],
            point_mode(mode),
        )),
        (Data::Vector(_), cx, cy) => {
            let rows = (0..data.len())
                .filter(|&i| match (data.value(cx, i), data.value(cy, i)) {
                    (Some(x), Some(y)) => x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3],
                    _ => false,
                })
                .collect();
            Some(Pick::Rows(rows, mode))
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn box_chart(ui: &mut egui::Ui, size: Vec2, base: &Base, over: &Over, xc: &ColDef, group: Option<&ColDef>, data: &Data<'_>, pick: &mut Option<Pick>, out: &mut Out<'_>) {
    let groups: Vec<(usize, &String, &BoxStats)> = base.groups.iter().enumerate().filter_map(|(k, (l, b))| b.as_ref().map(|b| (k, l, b))).collect();
    if groups.is_empty() {
        ui.allocate_ui(size, |ui| ui.label("No hay valores para las cajas."));
        return;
    }
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (_, _, b) in &groups {
        lo = lo.min(b.min);
        hi = hi.max(b.max);
    }
    let pad = (hi - lo).max(f64::MIN_POSITIVE) * 0.05;
    let title = match group {
        Some(g) => format!("{} por {}", xc.name, g.name),
        None => format!("Caja de {}", xc.name),
    };
    let n = groups.len();
    let f = frame(ui, size, &title, "", &xc.name, [0.0, lo - pad], [n as f64, hi + pad], false, false);
    out.register(2, &f.resp);
    // las marcas de x de `frame` no valen aquí: se tapan y se ponen los nombres de los grupos
    let strip = Rect::from_min_max(Pos2::new(f.plot.left() - 30.0, f.plot.bottom() + 1.0), Pos2::new(f.plot.right() + 10.0, f.plot.bottom() + 24.0));
    f.painter.rect_filled(strip, 0.0, ui.visuals().extreme_bg_color);
    let (fg, weak) = (ui.visuals().text_color(), ui.visuals().weak_text_color());
    let clipped = f.painter.with_clip_rect(f.plot);
    let body = ui.visuals().selection.bg_fill.gamma_multiply(1.1);
    let hover = f.resp.hover_pos().filter(|p| f.plot.contains(*p));
    let hovered = hover.map(|p| (f.from(p).0.floor().max(0.0) as usize).min(n - 1));
    let draw_box = |slot: usize, off: f64, half: f64, b: &BoxStats, color: Color32, stroke: Stroke| {
        let cx = slot as f64 + 0.5 + off;
        let (l, r) = (f.to(cx - half, 0.0).x, f.to(cx + half, 0.0).x);
        let (yq1, yq3, ym) = (f.to(0.0, b.q1).y, f.to(0.0, b.q3).y, f.to(0.0, b.median).y);
        clipped.rect_filled(Rect::from_min_max(Pos2::new(l, yq3), Pos2::new(r, yq1)), 0.0, color);
        clipped.rect_stroke(Rect::from_min_max(Pos2::new(l, yq3), Pos2::new(r, yq1)), 0.0, stroke, egui::StrokeKind::Inside);
        clipped.line_segment([Pos2::new(l, ym), Pos2::new(r, ym)], Stroke::new(2.0, stroke.color));
        let xm = f.to(cx, 0.0).x;
        clipped.line_segment([Pos2::new(xm, f.to(0.0, b.whisker_hi).y), Pos2::new(xm, yq3)], stroke);
        clipped.line_segment([Pos2::new(xm, yq1), Pos2::new(xm, f.to(0.0, b.whisker_lo).y)], stroke);
        for w in [b.whisker_hi, b.whisker_lo] {
            let y = f.to(0.0, w).y;
            clipped.line_segment([Pos2::new(xm - (r - l) * 0.25, y), Pos2::new(xm + (r - l) * 0.25, y)], stroke);
        }
        if b.min < b.whisker_lo {
            clipped.circle_stroke(Pos2::new(xm, f.to(0.0, b.min).y), 2.5, stroke);
        }
        if b.max > b.whisker_hi {
            clipped.circle_stroke(Pos2::new(xm, f.to(0.0, b.max).y), 2.5, stroke);
        }
    };
    let any_sel = over.n > 0;
    for (slot, (k, label, b)) in groups.iter().enumerate() {
        let hl = hovered == Some(slot);
        let col = if hl { body.gamma_multiply(1.4) } else { body };
        let (off, half) = if any_sel { (-0.17, 0.15) } else { (0.0, 0.3) };
        draw_box(slot, off, half, b, col.gamma_multiply(0.6), Stroke::new(1.3, fg));
        if let Some(Some(sb)) = over.boxes.get(*k) {
            draw_box(slot, 0.17, 0.15, sb, SELECTED.gamma_multiply(0.45), Stroke::new(1.3, SELECTED));
        }
        let x = f.to(slot as f64 + 0.5, 0.0).x;
        let short: String = label.chars().take(((f.plot.width() / n as f32) / 6.5) as usize + 1).collect();
        f.painter.text(Pos2::new(x, f.plot.bottom() + 4.0), Align2::CENTER_TOP, short, FontId::proportional(10.0), weak);
    }
    if let Some(slot) = hovered
        && let Some((_, label, b)) = groups.get(slot)
    {
        corner_text(&f, ui, format!("{label}: n {} · mediana {:.3} · Q1 {:.3} · Q3 {:.3} · fuera {}", b.n, b.median, b.q1, b.q3, b.outliers));
    } else if base.all_groups > MAX_GROUPS {
        corner_text(&f, ui, format!("{MAX_GROUPS} grupos de {}", base.all_groups));
    }
    if !data.can_select() {
        return;
    }
    // un clic en una caja selecciona su grupo; un rectángulo, los grupos que cruza con ese rango de valores
    let slots_of = |rect: Rect| -> Vec<usize> {
        let (a, b) = (f.from(rect.left_top()).0, f.from(rect.right_top()).0);
        (0..n).filter(|s| (*s as f64) < a.max(b) && (*s as f64 + 1.0) > a.min(b)).collect()
    };
    match gesture(ui, &f.resp) {
        Gesture::Dragging(r) => draw_brush(&f, r, false),
        Gesture::Done { rect, mods } => {
            let (ya, yb) = (f.from(rect.left_bottom()).1, f.from(rect.left_top()).1);
            let labels: Vec<&String> = slots_of(rect).into_iter().filter_map(|s| groups.get(s).map(|g| g.1)).collect();
            *pick = select_groups(data, xc, group, &labels, Some((ya, yb)), mods);
        }
        Gesture::Click { pos, mods } if f.plot.contains(pos) => {
            let s = (f.from(pos).0.floor().max(0.0) as usize).min(n - 1);
            if let Some(g) = groups.get(s) {
                *pick = select_groups(data, xc, group, &[g.1], None, mods);
            }
        }
        _ => {}
    }
}

/// Selecciona los datos de unos grupos, con un rango de valores de `xc` si se da.
fn select_groups(data: &Data<'_>, xc: &ColDef, group: Option<&ColDef>, labels: &[&String], range: Option<(f64, f64)>, mods: egui::Modifiers) -> Option<Pick> {
    let mode = mode_of(mods);
    if labels.is_empty() {
        return None;
    }
    match data {
        Data::Vector(_) => {
            let rows = (0..data.len())
                .filter(|&i| {
                    let in_group = match group {
                        Some(g) => data.label(g.col, i).is_some_and(|l| labels.iter().any(|x| **x == l)),
                        None => true,
                    };
                    in_group && range.is_none_or(|(a, b)| data.value(xc.col, i).is_some_and(|v| v >= a && v <= b))
                })
                .collect();
            Some(Pick::Rows(rows, mode))
        }
        Data::Points(_) => {
            let mut ranges = Vec::new();
            if let Some(g) = group
                && let Col::Attr(k) = g.col
            {
                let bins: Vec<(f64, f64)> = labels
                    .iter()
                    .filter_map(|l| {
                        let v = natural(l);
                        v.is_finite().then_some((v, v + 1.0))
                    })
                    .collect();
                if bins.is_empty() {
                    return None;
                }
                ranges.push(RangeSel { kind: k, bins });
            }
            if let (Some((a, b)), Col::Attr(k)) = (range, xc.col) {
                ranges.push(RangeSel { kind: k, bins: vec![(a, b.next_up())] });
            }
            (!ranges.is_empty()).then_some(Pick::Ranges(ranges, point_mode(mode)))
        }
        Data::Raster(_) => None,
    }
}

// ---------------------------------------------------------------------------------------------
// estadísticas

fn compute_stats(data: &Data<'_>, group: Option<&ColDef>, only_selected: bool) -> Vec<StatRow> {
    let n = data.len();
    let rows: Vec<usize> = (0..n).filter(|&i| !only_selected || data.selected(i)).collect();
    let (gids, labels) = match group {
        Some(g) => data.group_ids(g.col),
        None => (Vec::new(), Vec::new()),
    };
    // los grupos con datos, en orden numérico o alfabético
    let mut order: Vec<usize> = (0..labels.len()).collect();
    order.sort_by(|a, b| natural(&labels[*a]).total_cmp(&natural(&labels[*b])).then_with(|| labels[*a].cmp(&labels[*b])));
    let mut out = Vec::new();
    for c in data.cols() {
        if group.is_none() {
            let values: Vec<f64> = rows.iter().map(|&i| data.value(c.col, i).unwrap_or(f64::NAN)).collect();
            if let Some(s) = chartstats::column_stats(&values) {
                out.push(StatRow { group: String::new(), column: c.name.clone(), s });
            }
            continue;
        }
        let mut by: Vec<Vec<f64>> = vec![Vec::new(); labels.len()];
        for &i in &rows {
            let g = gids[i];
            if g != u32::MAX {
                by[g as usize].push(data.value(c.col, i).unwrap_or(f64::NAN));
            }
        }
        for &k in &order {
            if let Some(s) = chartstats::column_stats(&by[k]) {
                out.push(StatRow { group: labels[k].clone(), column: c.name.clone(), s });
            }
        }
    }
    out
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Las estadísticas como CSV (punto decimal), con una columna de grupo si se agrupó.
fn stats_csv(rows: &[StatRow], grouped: bool) -> String {
    let mut s = String::new();
    if grouped {
        s.push_str("grupo,");
    }
    s.push_str("columna,n,faltan,suma,media,desviacion,minimo,q1,mediana,q3,maximo,mad\n");
    for r in rows {
        if grouped {
            s.push_str(&csv_field(&r.group));
            s.push(',');
        }
        let c = &r.s;
        s.push_str(&format!("{},{},{},{},{},{},{},{},{},{},{},{}\n", csv_field(&r.column), c.n, c.missing, c.sum, c.mean, c.std, c.min, c.q1, c.median, c.q3, c.max, c.mad));
    }
    s
}

fn stats_body(ui: &mut egui::Ui, st: &mut Charts, data: &Data<'_>, point_rev: u64, sel_rev: u64, t: &Tokens) {
    let groups = data.group_cols();
    if st.stats_group > groups.len() {
        st.stats_group = 0;
    }
    ui.horizontal(|ui| {
        if !groups.is_empty() {
            ui.label("Agrupar por");
            let name = if st.stats_group == 0 { "(sin grupos)".to_string() } else { groups[st.stats_group - 1].name.clone() };
            egui::ComboBox::from_id_salt("stats_group").width(140.0).selected_text(name).show_ui(ui, |ui| {
                ui.selectable_value(&mut st.stats_group, 0, "(sin grupos)");
                for (k, c) in groups.iter().enumerate() {
                    ui.selectable_value(&mut st.stats_group, k + 1, &c.name);
                }
            });
        }
        if data.can_select() {
            ui.checkbox(&mut st.stats_only_selected, "Solo los seleccionados");
        } else {
            st.stats_only_selected = false;
        }
    });
    let group = (st.stats_group > 0).then(|| groups[st.stats_group - 1].clone());
    let key = hash_of((
        data.data_sig(point_rev),
        group.as_ref().map(|g| g.col),
        st.stats_only_selected,
        if st.stats_only_selected { data.selection_sig(sel_rev) } else { 0 },
    ));
    if st.stats.as_ref().is_none_or(|s| s.0 != key) {
        st.stats = Some((key, compute_stats(data, group.as_ref(), st.stats_only_selected)));
    }
    let Some((_, rows)) = &st.stats else { return };
    let grouped = group.is_some();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("{} filas", rows.len())).size(11.5).color(t.weak));
        if ui.button("Copiar").on_hover_text("Copia la tabla, con tabuladores, para pegarla en una hoja de cálculo.").clicked() {
            let text = stats_csv(rows, grouped).replace(',', "\t");
            ui.ctx().copy_text(text);
        }
        if ui.button("Exportar CSV…").clicked()
            && let Some(p) = crate::prefs::dialog().add_filter("CSV", &["csv"]).set_file_name("estadisticas.csv").save_file()
        {
            st.msg = match std::fs::write(&p, stats_csv(rows, grouped)) {
                Ok(()) => format!("Guardado en {}", p.display()),
                Err(e) => format!("No se pudo guardar: {e}"),
            };
        }
        if !st.msg.is_empty() {
            ui.label(egui::RichText::new(&st.msg).size(11.0).color(t.weak));
        }
    });
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("stats_grid").striped(true).num_columns(12).spacing([14.0, 3.0]).show(ui, |ui| {
            let head = |ui: &mut egui::Ui, s: &str| {
                ui.label(egui::RichText::new(s).strong());
            };
            if grouped {
                head(ui, "Grupo");
            }
            for h in ["Columna", "n", "Faltan", "Media", "Desv. típica", "Mínimo", "Q1", "Mediana", "Q3", "Máximo", "MAD"] {
                head(ui, h);
            }
            ui.end_row();
            let num = |ui: &mut egui::Ui, v: f64| {
                ui.label(egui::RichText::new(format!("{v:.4}")).monospace());
            };
            for r in rows {
                if grouped {
                    ui.label(&r.group);
                }
                ui.label(&r.column);
                ui.label(egui::RichText::new(format!("{}", r.s.n)).monospace());
                ui.label(egui::RichText::new(format!("{}", r.s.missing)).monospace());
                for v in [r.s.mean, r.s.std, r.s.min, r.s.q1, r.s.median, r.s.q3, r.s.max, r.s.mad] {
                    num(ui, v);
                }
                ui.end_row();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hipso_core::fieldtype::FieldType;
    use hipso_core::view::{Feature, GeomKind, VectorData};

    fn layer() -> VecLayer {
        let mut d = VectorData { points: true, ..VectorData::default() };
        d.features = (0..40).map(|i| Feature { parts: vec![vec![[f64::from(i), 0.0]]] }).collect();
        d.text = vec![Vec::new(); 40];
        d.num = vec![Vec::new(); 40];
        let h = d.add_field("altura", FieldType::Real);
        let a = d.add_field("edad", FieldType::Real);
        let e = d.add_field("especie", FieldType::Text);
        for i in 0..40 {
            let _ = d.set_value(i, h, &format!("{}", 5 + i / 2));
            let _ = d.set_value(i, a, &format!("{}", 10 + i));
            let _ = d.set_value(i, e, if i % 2 == 0 { "pino" } else { "encina" });
        }
        VecLayer::new(d, GeomKind::Point)
    }

    #[test]
    fn a_vector_source_offers_its_numeric_columns_and_groups_by_text() {
        let v = layer();
        let d = Data::Vector(&v);
        let cols: Vec<String> = d.cols().into_iter().map(|c| c.name).collect();
        assert_eq!(cols, vec!["altura", "edad"]);
        let groups: Vec<String> = d.group_cols().into_iter().map(|c| c.name).collect();
        assert_eq!(groups, vec!["especie"]);
        assert_eq!(d.label(Col::Field(2), 0).as_deref(), Some("pino"));
        assert_eq!(d.value(Col::Field(0), 3), Some(6.0));
        assert_eq!(d.value(Col::Field(2), 3), None);
    }

    #[test]
    fn base_and_selection_overlays_follow_the_selected_rows() {
        let mut v = layer();
        let d = Data::Vector(&v);
        let cols = d.cols();
        let group = d.group_cols().into_iter().next();
        let base = compute_base(&d, &cols[0], &cols[1], group.as_ref(), 10);
        assert!(base.hist.is_some());
        assert_eq!(base.dots.len(), 40);
        assert!(base.h2.is_none());
        assert_eq!(base.groups.len(), 2);
        assert!(base.fit.is_some_and(|f| f.r > 0.99));
        v.selected = (0..10).collect();
        let d = Data::Vector(&v);
        let over = compute_over(&d, &base, &cols[0], &cols[1]);
        assert_eq!(over.n, 10);
        assert_eq!(over.hist.iter().sum::<u64>(), 10);
        assert_eq!(over.dots.len(), 10);
        assert_eq!(over.boxes.iter().flatten().map(|b| b.n).sum::<usize>(), 10);
    }

    #[test]
    fn marking_a_range_or_a_rectangle_gives_the_rows_inside() {
        let v = layer();
        let d = Data::Vector(&v);
        let cols = d.cols();
        let none = egui::Modifiers::NONE;
        // altura 5 + i/2: valores 5..=24; [6, 8) son i = 2, 3, 4, 5
        let Some(Pick::Rows(rows, 0)) = select_range(&d, &cols[0], (6.0, 8.0), none) else { panic!("sin filas") };
        assert_eq!(rows, vec![2, 3, 4, 5]);
        let shift = egui::Modifiers { shift: true, ..egui::Modifiers::NONE };
        assert!(matches!(select_range(&d, &cols[0], (6.0, 8.0), shift), Some(Pick::Rows(_, 1))));
        let alt = egui::Modifiers { alt: true, ..egui::Modifiers::NONE };
        assert!(matches!(select_range(&d, &cols[0], (6.0, 8.0), alt), Some(Pick::Rows(_, 2))));
        // edad 10 + i: el rectángulo [5, 6] x [10, 12] deja i = 0, 1, 2 (alturas 5, 5, 6)
        let Some(Pick::Rows(rows, _)) = select_rect(&d, &cols[0], &cols[1], [5.0, 10.0, 6.0, 12.0], none) else { panic!("sin filas") };
        assert_eq!(rows, vec![0, 1, 2]);
        // un grupo de cajas
        let g = d.group_cols().into_iter().next();
        let pino = "pino".to_string();
        let Some(Pick::Rows(rows, _)) = select_groups(&d, &cols[0], g.as_ref(), &[&pino], None, none) else { panic!("sin filas") };
        assert_eq!(rows.len(), 20);
        assert!(rows.iter().all(|i| i % 2 == 0));
    }

    #[test]
    fn marking_points_builds_rules_that_replay_on_the_whole_file() {
        let store = PointStore::default();
        let d = Data::Points(&store);
        let cols = d.cols();
        let z = cols.iter().find(|c| c.col == Col::Attr(AttrKind::Z)).cloned().unwrap();
        let int = cols.iter().find(|c| c.col == Col::Attr(AttrKind::Intensity)).cloned().unwrap();
        let Some(Pick::Ranges(r, Mode::New)) = select_rect(&d, &z, &int, [1.0, 10.0, 2.0, 20.0], egui::Modifiers::NONE) else { panic!("sin reglas") };
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].kind, AttrKind::Z);
        assert!(r[0].bins[0].1 > 2.0);
        let class = "2 · Suelo".to_string();
        let g = d.group_cols().into_iter().find(|c| c.col == Col::Attr(AttrKind::Class));
        let Some(Pick::Ranges(r, _)) = select_groups(&d, &z, g.as_ref(), &[&class], None, egui::Modifiers::NONE) else { panic!("sin reglas") };
        assert_eq!(r[0].kind, AttrKind::Class);
        assert_eq!(r[0].bins, vec![(2.0, 3.0)]);
    }

    #[test]
    fn statistics_group_and_export_as_csv() {
        let mut v = layer();
        let d = Data::Vector(&v);
        let g = d.group_cols().into_iter().next();
        let all = compute_stats(&d, None, false);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].s.n, 40);
        let by = compute_stats(&d, g.as_ref(), false);
        assert_eq!(by.len(), 4);
        v.selected = (0..4).collect();
        let d = Data::Vector(&v);
        let sel = compute_stats(&d, None, true);
        assert_eq!(sel[0].s.n, 4);
        let csv = stats_csv(&by, true);
        assert!(csv.starts_with("grupo,columna,n,faltan"));
        assert_eq!(csv.lines().count(), 5);
        assert_eq!(csv_field("a,b"), "\"a,b\"");
    }

    #[test]
    fn the_default_columns_skip_an_identifier() {
        let mut v = layer();
        // un campo de identificadores distintos, antes de los demás
        let id = v.data.add_field("parcela", hipso_core::fieldtype::FieldType::Integer);
        for i in 0..40 {
            let _ = v.data.set_value(i, id, &format!("{}", i + 1));
        }
        let d = Data::Vector(&v);
        let cols = d.cols();
        let (x, y) = default_columns(&d, &cols);
        assert_eq!((cols[x].name.as_str(), cols[y].name.as_str()), ("altura", "edad"));
    }

    #[test]
    fn a_chart_is_cropped_from_the_window_capture_and_written_as_png() {
        // una captura de 40 x 20 píxeles al doble de resolución: 20 x 10 puntos; un rectángulo de
        // 10 x 4 puntos desde (5, 3) son 20 x 8 píxeles desde (10, 6)
        let mut img = egui::ColorImage::filled([40, 20], Color32::from_rgb(10, 20, 30));
        for y in 6..14 {
            for x in 10..30 {
                img.pixels[y * 40 + x] = Color32::from_rgb(200, 100, 50);
            }
        }
        let dir = std::env::temp_dir().join(format!("hipso_chartpng_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("g.png");
        let rect = Rect::from_min_size(Pos2::new(5.0, 3.0), Vec2::new(10.0, 4.0));
        assert_eq!(save_crop(&img, rect, 2.0, &path), Ok([20, 8]));
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (20, 8));
        assert_eq!(&buf[0..4], &[200, 100, 50, 255]);
        // fuera de la captura: error, sin archivo nuevo
        let out = Rect::from_min_size(Pos2::new(500.0, 500.0), Vec2::new(10.0, 10.0));
        assert!(save_crop(&img, out, 2.0, &dir.join("x.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_image_of_all_three_charts_is_the_union_of_their_rectangles() {
        let mut st = Charts::default();
        assert!(export_rect(&st, Want::All).is_none());
        st.rects = [
            Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
            Rect::from_min_size(Pos2::new(12.0, 0.0), Vec2::new(10.0, 10.0)),
            Rect::from_min_size(Pos2::new(24.0, 0.0), Vec2::new(10.0, 10.0)),
        ];
        assert_eq!(export_rect(&st, Want::All), Some(Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(34.0, 10.0))));
        assert_eq!(export_rect(&st, Want::One(1)), Some(st.rects[1]));
        assert_eq!(Want::One(2).file_name(), "eda_cajas.png");
    }

    #[test]
    fn the_default_columns_skip_a_constant_column() {
        let mut v = layer();
        let k = v.data.add_field("area", hipso_core::fieldtype::FieldType::Real);
        for i in 0..40 {
            let _ = v.data.set_value(i, k, "399.73");
        }
        let d = Data::Vector(&v);
        let cols = d.cols();
        let (x, y) = default_columns(&d, &cols);
        assert_ne!(cols[x].name, "area");
        assert_ne!(cols[y].name, "area");
    }

    #[test]
    fn large_axis_values_are_abbreviated_and_fewer_ticks_fit_when_labels_are_long() {
        assert_eq!(axis_text(200_000_000.0, 100_000_000.0), "200M");
        assert_eq!(axis_text(1_500_000.0, 500_000.0), "1.5M");
        assert_eq!(axis_text(2_500.0, 500.0), tick_text(2_500.0, 500.0));
        assert!(ticks_that_fit(300.0, "200M") > ticks_that_fit(300.0, "200000000"));
        assert_eq!(ticks_that_fit(10.0, "x"), 2.0);
        assert_eq!(ticks_that_fit(2000.0, "1"), 6.0);
    }

    #[test]
    fn long_numbers_in_labels_are_abbreviated() {
        assert_eq!(num_short(608_977_123.737), "608.98M");
        assert_eq!(num_short(12.34567), "12.346");
        assert_eq!(num_short(-2_500_000.0), "-2.50M");
    }

    #[test]
    fn natural_order_reads_the_number_in_front() {
        assert_eq!(natural("2 · Suelo"), 2.0);
        assert_eq!(natural("10"), 10.0);
        assert!(natural("pino").is_infinite());
    }

    /// Mide los cálculos de los gráficos con 8 millones de puntos (`cargo test --release -- --ignored
    /// --nocapture timing`).
    #[test]
    #[ignore]
    fn timing_with_eight_million_points() {
        let n = 8_000_000usize;
        let mut s = PointStore { stride: 1, ..PointStore::default() };
        for i in 0..n {
            let f = (i % 10_000) as f32;
            s.x.push(f / 10.0);
            s.y.push((i / 10_000) as f32);
            s.z.push(f.sin() * 5.0 + 10.0 + ((i * 7919) % 100) as f32 / 50.0);
            s.class.push(if i % 3 == 0 { 2 } else { 5 });
            s.intensity.push(((i * 31) % 4000) as u16);
            s.ret.push(0x11);
            s.src.push((i % 4) as u16);
            s.scan.push(0);
            s.file.push(0);
        }
        s.total = n as u64;
        s.selected = (0..n).map(|i| i % 50 == 0).collect();
        let d = Data::Points(&s);
        let cols = d.cols();
        let z = cols[0].clone();
        let int = cols[1].clone();
        let g = d.group_cols().into_iter().next();
        let t = std::time::Instant::now();
        let base = compute_base(&d, &z, &int, g.as_ref(), 30);
        eprintln!("base (histograma, dispersión, ajuste y cajas): {:.2} s", t.elapsed().as_secs_f64());
        assert!(base.h2.is_some());
        let t = std::time::Instant::now();
        let over = compute_over(&d, &base, &z, &int);
        eprintln!("selección de {} puntos: {:.2} s", over.n, t.elapsed().as_secs_f64());
        let t = std::time::Instant::now();
        let rows = compute_stats(&d, g.as_ref(), false);
        eprintln!("estadísticas agrupadas ({} filas): {:.2} s", rows.len(), t.elapsed().as_secs_f64());
    }
}
