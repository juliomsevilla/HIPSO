//! Edición de puntos en el visor, con las herramientas de un editor vectorial: se seleccionan
//! puntos con un rectángulo, polígono, lazo o círculo (en la planta o en un perfil), se combinan
//! las selecciones (nueva, añadir con Mayús, restar con Alt, intersecar con Ctrl, invertir), se
//! restringen por atributos, se eliminan o se les cambia la clase, deshacer y rehacer, y se guarda
//! un archivo nuevo. Las ediciones son reglas que el motor aplica a cada punto del archivo al
//! guardar.

use super::*;
use hipso_core::edit::{self, Attr, Edit, Mode, Region, SelOp, Session, ViewCut};
use hipso_core::vector::Polygon;

pub enum Cmd {
    AddOp(Box<SelOp>),
    Clear,
    All,
    Invert,
    Delete,
    Reclass(u8),
    Undo,
    Redo,
    /// Activa o desactiva la edición del historial en esta posición.
    Toggle(usize),
    Discard,
    /// Fija el foco en lo que elige la selección actual.
    Focus,
    /// Quita el foco: la vista vuelve a toda la nube.
    Unfocus,
    /// Aplica estas ediciones, leídas de un archivo, a la sesión.
    Apply(Vec<Edit>),
}

/// El foco de la edición: se ve y se edita solo lo que cae en él, y todo ocurre en la sesión de la
/// nube original. No crea ningún archivo.
pub struct Focus {
    /// La forma de la selección con que se fijó, sin sus filtros de atributos: un foco que dependiera
    /// de la clase perdería los puntos que se reclasifican dentro de él.
    pub inner: Vec<SelOp>,
    /// La ventana (x mínima, y mínima, x máxima, y máxima) que se lee a toda densidad.
    pub window: [f64; 4],
    /// Puntos del foco en pantalla.
    pub shown: usize,
}

/// La selección `ops` limitada al foco, si lo hay. Todo lo que se hace con una selección (contarla,
/// marcarla, editarla, exportarla) pasa por aquí, de modo que con un foco no se alcanza nada de lo que
/// queda fuera de él, esté a la vista o no.
pub fn with_focus(ops: &[SelOp], focus: &Option<Focus>) -> Vec<SelOp> {
    let mut v = ops.to_vec();
    if let (false, Some(f)) = (v.is_empty(), focus) {
        v.push(edit::focus_op(&f.inner));
    }
    v
}

/// Qué lado del plano local se selecciona.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlaneSide {
    /// Los puntos por encima de la superficie (vegetación, ruido alto).
    Above,
    /// Los puntos por debajo (ruido bajo suelo).
    Below,
    /// Los puntos cerca de la superficie, a menos de la tolerancia (añadir al suelo).
    Near,
}

pub struct Editing {
    pub session: Option<Session>,
    pub ops: Vec<SelOp>,
    pub mode: Mode,
    /// Vértices del polígono o trazo del lazo en coordenadas de planta, y en las de perfil
    /// (distancia, z).
    pub draft: Vec<[f64; 2]>,
    pub pdraft: Vec<[f64; 2]>,
    pub pstart: Option<Pos2>,
    /// Trazo del lazo e inicio del arrastre en la vista 3D, en píxeles de la vista.
    pub d3draft: Vec<[f64; 2]>,
    pub d3start: Option<Pos2>,
    /// Radio de los pinceles en la pantalla, en píxeles.
    pub brush_px: f64,
    /// Alto de la franja de «por encima/por debajo de una línea» en el perfil (0: sin límite).
    pub side_height: f64,
    /// «Cerca de una línea»: hasta cuánto por encima y por debajo de ella se considera cerca (m).
    pub near_up: f64,
    pub near_down: f64,
    /// «Clasificar a lo largo de una línea»: para sobre, cerca y bajo, si se clasifica y a qué clase.
    pub line_classes: [(bool, u8); 3],
    /// Ajuste de «plano local»: radio del círculo (m), clase de los puntos con que se ajusta, si el
    /// ajuste es una curva de segundo grado en vez de un plano, qué lado se selecciona, la
    /// tolerancia (m, o desviaciones típicas si `plane_sigma`) y cuántas desviaciones.
    pub plane_r: f64,
    pub plane_class: u8,
    pub plane_quad: bool,
    pub plane_side: PlaneSide,
    pub plane_tol: f64,
    pub plane_sigma: bool,
    pub plane_k: f64,
    /// Las selecciones en 3D alcanzan solo tantos metros a cada lado del plano del objetivo.
    pub slab: bool,
    pub slab_half: f64,
    pub sel_rev: u64,
    /// Sube con cada cambio de clases o eliminaciones: las ventanas de detalle lo siguen.
    pub edit_rev: u64,
    pub sel_count: usize,
    pub class: u8,
    pub cmds: Vec<Cmd>,
    pub attr: Attr,
    pub hist: crate::histo::State,
    pub z_lo: String,
    pub z_hi: String,
    pub i_lo: String,
    pub i_hi: String,
    /// Ediciones ya escritas en un archivo.
    pub saved: usize,
    /// Se activó o desactivó una edición desde el último guardado.
    pub toggled: bool,
    pub want_save: bool,
    /// Guarda sin preguntar dónde (línea de comandos `--save-edited`).
    pub auto_save: bool,
    pub focus: Option<Focus>,
    /// Dónde se guarda esta sesión mientras no se guarda la nube editada.
    pub session_file: Option<PathBuf>,
    /// Ediciones de una sesión anterior que no se llegaron a guardar y que se ofrecen recuperar.
    pub recover: Option<Vec<Edit>>,
    /// Las nubes originales y las editadas que escribe el guardado en curso.
    pub pending_swap: Option<(Vec<PathBuf>, Vec<PathBuf>)>,
    /// El guardado terminó: se ofrece cambiar las originales por las editadas.
    pub swap_offer: Option<(Vec<PathBuf>, Vec<PathBuf>)>,
}

impl Default for Editing {
    fn default() -> Self {
        Self {
            session: None,
            ops: Vec::new(),
            mode: Mode::New,
            draft: Vec::new(),
            pdraft: Vec::new(),
            pstart: None,
            d3draft: Vec::new(),
            d3start: None,
            brush_px: 18.0,
            side_height: 0.0,
            near_up: 0.3,
            near_down: 0.3,
            line_classes: [(true, 5), (true, 14), (false, 2)],
            plane_r: 5.0,
            plane_class: 2,
            plane_quad: false,
            plane_side: PlaneSide::Above,
            plane_tol: 0.3,
            plane_sigma: false,
            plane_k: 2.5,
            slab: false,
            slab_half: 1.0,
            sel_rev: 0,
            edit_rev: 0,
            sel_count: 0,
            class: 7,
            cmds: Vec::new(),
            attr: Attr::default(),
            hist: crate::histo::State::default(),
            z_lo: String::new(),
            z_hi: String::new(),
            i_lo: String::new(),
            i_hi: String::new(),
            saved: 0,
            toggled: false,
            want_save: false,
            auto_save: false,
            focus: None,
            session_file: None,
            recover: None,
            pending_swap: None,
            swap_offer: None,
        }
    }
}

fn parse(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse().ok()
}

pub fn is_select(n: Nav) -> bool {
    is_select_3d(n) || is_brush(n) || is_line(n) || n == Nav::SelPlane
}

/// Las herramientas que funcionan también en la vista 3D: formas dibujadas en la pantalla. Los
/// pinceles y las líneas pertenecen a la planta y al perfil.
pub fn is_select_3d(n: Nav) -> bool {
    matches!(
        n,
        Nav::SelRect | Nav::SelPoly | Nav::SelLasso | Nav::SelCircle
    )
}

pub fn is_brush(n: Nav) -> bool {
    matches!(n, Nav::SelBrush | Nav::SelBrushSq)
}

/// Por encima o por debajo de una línea dibujada en el perfil.
pub fn is_line(n: Nav) -> bool {
    matches!(
        n,
        Nav::SelAbove | Nav::SelBelow | Nav::SelNear | Nav::ClassLine
    )
}

/// Un trazo de pincel en curso: añade la posición del puntero a `draft` (coordenadas de la vista)
/// mientras el botón está pulsado, y da el trazo cuando termina. `px` es cuántos píxeles hacen una
/// unidad de la vista y `size_px` el radio del pincel en la pantalla; `[` y `]` lo cambian.
fn brush_stroke(
    nav: Nav,
    resp: &egui::Response,
    size_px: &mut f64,
    draft: &mut Vec<[f64; 2]>,
    to: &dyn Fn(Pos2) -> [f64; 2],
    px: f64,
) -> Option<edit::Brush> {
    if resp.hovered() {
        let (less, more) = resp.ctx.input(|i| {
            (
                i.key_pressed(egui::Key::OpenBracket),
                i.key_pressed(egui::Key::CloseBracket),
            )
        });
        if less {
            *size_px = (*size_px / 1.15).max(2.0);
        }
        if more {
            *size_px = (*size_px * 1.15).min(300.0);
        }
    }
    let end = resp.interact_pointer_pos();
    if resp.dragged_by(egui::PointerButton::Primary)
        && let Some(p) = end
    {
        let c = to(p);
        if draft
            .last()
            .is_none_or(|l| (l[0] - c[0]).hypot(l[1] - c[1]) * px > 2.0)
        {
            draft.push(c);
        }
    }
    let stopped = resp.drag_stopped_by(egui::PointerButton::Primary);
    if !(stopped || resp.clicked_by(egui::PointerButton::Primary)) {
        return None;
    }
    let mut path = std::mem::take(draft);
    if path.is_empty() {
        // un clic pinta una pincelada
        path.push(to(end?));
    }
    Some(edit::Brush::new(
        &path,
        *size_px / px,
        nav == Nav::SelBrushSq,
    ))
}

/// El pincel en la pantalla: el trazo hasta ahora y la forma del pincel bajo el puntero.
fn paint_brush(
    painter: &egui::Painter,
    nav: Nav,
    resp: &egui::Response,
    trail: &[Pos2],
    size_px: f32,
    color: Color32,
) {
    let square = nav == Nav::SelBrushSq;
    let fill = color.gamma_multiply(0.28);
    if trail.len() >= 2 {
        painter.add(egui::Shape::line(
            trail.to_vec(),
            Stroke::new(2.0 * size_px, fill),
        ));
    }
    for p in trail
        .iter()
        .step_by(trail.len().div_ceil(40).max(1))
        .chain(trail.last())
    {
        if square {
            painter.rect_filled(
                Rect::from_center_size(*p, Vec2::splat(2.0 * size_px)),
                0.0,
                fill,
            );
        } else {
            painter.circle_filled(*p, size_px, fill);
        }
    }
    if let Some(h) = resp.hover_pos() {
        let stroke = Stroke::new(1.5, color);
        if square {
            painter.rect_stroke(
                Rect::from_center_size(h, Vec2::splat(2.0 * size_px)),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        } else {
            painter.circle_stroke(h, size_px, stroke);
        }
    }
}

/// La forma que acaba de terminar una herramienta de selección, a partir de los eventos del
/// puntero. `to` convierte una posición de pantalla en las coordenadas de la vista; `px` es cuántos
/// píxeles hacen una unidad de ellas.
fn finished_shape(
    nav: Nav,
    resp: &egui::Response,
    start: Option<Pos2>,
    draft: &mut Vec<[f64; 2]>,
    to: &dyn Fn(Pos2) -> [f64; 2],
    px: f64,
) -> Option<Polygon> {
    let stopped = resp.drag_stopped_by(egui::PointerButton::Primary);
    let end = resp.interact_pointer_pos();
    match nav {
        Nav::SelRect if stopped => {
            let (a, b) = (start?, end?);
            let (p, q) = if (a - b).length() < 4.0 {
                // un clic elige lo que está a unos pocos píxeles
                let r = 4.0 / px;
                let c = to(a);
                ([c[0] - r, c[1] - r], [c[0] + r, c[1] + r])
            } else {
                (to(a), to(b))
            };
            Some(edit::rectangle(p, q))
        }
        Nav::SelRect if resp.clicked() => {
            let c = to(resp.interact_pointer_pos()?);
            let r = 4.0 / px;
            Some(edit::rectangle([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]))
        }
        Nav::SelCircle if stopped => {
            let (a, b) = (to(start?), to(end?));
            let r = (a[0] - b[0]).hypot(a[1] - b[1]);
            (r > 0.0).then(|| edit::circle(a[0], a[1], r))
        }
        Nav::SelLasso => {
            if resp.dragged_by(egui::PointerButton::Primary)
                && let Some(p) = end
            {
                let c = to(p);
                if draft
                    .last()
                    .is_none_or(|l| (l[0] - c[0]).hypot(l[1] - c[1]) * px > 3.0)
                {
                    draft.push(c);
                }
            }
            if stopped {
                let d = std::mem::take(draft);
                (d.len() >= 3).then(|| Polygon::new(vec![d]))
            } else {
                None
            }
        }
        Nav::SelPoly => {
            if resp.clicked()
                && let Some(p) = end
            {
                draft.push(to(p));
            }
            if (resp.secondary_clicked() || resp.double_clicked()) && draft.len() >= 3 {
                Some(Polygon::new(vec![std::mem::take(draft)]))
            } else {
                if resp.secondary_clicked() {
                    draft.clear();
                }
                None
            }
        }
        _ => None,
    }
}

fn paint_shape(
    painter: &egui::Painter,
    nav: Nav,
    resp: &egui::Response,
    start: Option<Pos2>,
    trail: &[Pos2],
    color: Color32,
) {
    let stroke = Stroke::new(1.5, color);
    let dragging = resp.dragged_by(egui::PointerButton::Primary);
    match nav {
        Nav::SelRect if dragging => {
            if let (Some(a), Some(b)) = (start, resp.interact_pointer_pos()) {
                let r = Rect::from_two_pos(a, b);
                painter.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
                painter.rect_filled(r, 0.0, color.gamma_multiply(0.12));
            }
        }
        Nav::SelCircle if dragging => {
            if let (Some(a), Some(b)) = (start, resp.interact_pointer_pos()) {
                painter.circle(a, (a - b).length(), color.gamma_multiply(0.12), stroke);
            }
        }
        Nav::SelLasso | Nav::SelPoly if trail.len() >= 2 => {
            let mut v = trail.to_vec();
            if nav == Nav::SelPoly
                && let Some(p) = resp.hover_pos()
            {
                v.push(p);
            }
            painter.add(egui::Shape::line(v, stroke));
        }
        _ => {}
    }
}

impl App {
    pub fn mode_now(&self, ctx: &egui::Context) -> Mode {
        let m = ctx.input(|i| i.modifiers);
        if m.shift {
            Mode::Add
        } else if m.alt {
            Mode::Subtract
        } else if m.command {
            Mode::Intersect
        } else {
            self.ed.mode
        }
    }

    pub fn queue(&mut self, c: Cmd) {
        self.ed.cmds.push(c);
    }

    /// Herramientas de selección en la ventana de planta.
    pub fn plan_select(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        rect: Rect,
        w: usize,
        h: usize,
    ) {
        let plan = self.plan;
        let to = move |p: Pos2| {
            let m = plan.to_map(
                f64::from(p.x - rect.left()),
                f64::from(p.y - rect.top()),
                w,
                h,
            );
            [m.0, m.1]
        };
        if is_brush(self.nav) {
            if let Some(b) = brush_stroke(
                self.nav,
                resp,
                &mut self.ed.brush_px,
                &mut self.ed.draft,
                &to,
                plan.scale,
            ) {
                let op = SelOp {
                    mode: self.mode_now(ctx),
                    region: Region::PlanBrush(b),
                    attr: self.ed.attr.clone(),
                };
                self.queue(Cmd::AddOp(Box::new(op)));
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) {
                self.drag_start = None;
            }
            return;
        }
        if self.nav == Nav::SelPlane {
            if resp.clicked_by(egui::PointerButton::Primary)
                && let Some(p) = resp.interact_pointer_pos()
            {
                let c = to(p);
                self.select_by_plane(ctx, c[0], c[1]);
            }
            return;
        }
        let start = self.drag_start;
        if let Some(poly) =
            finished_shape(self.nav, resp, start, &mut self.ed.draft, &to, plan.scale)
        {
            let op = SelOp {
                mode: self.mode_now(ctx),
                region: Region::Plan(poly),
                attr: self.ed.attr.clone(),
            };
            self.queue(Cmd::AddOp(Box::new(op)));
            self.drag_start = None;
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            self.drag_start = None;
        }
    }

    /// «Plano local»: ajusta un plano (o una curva) a los puntos de la clase elegida que caen en un
    /// círculo alrededor de `(x, y)` y añade a la selección los que quedan por encima, por debajo o
    /// cerca de él. Toma los puntos de la ventana de detalle si la tiene y el clic cae dentro, y del
    /// almacén de la vista si no. La selección es una región (los coeficientes y la banda), no una
    /// lista de puntos: el archivo guardado da lo mismo que se ve.
    fn select_by_plane(&mut self, ctx: &egui::Context, x: f64, y: f64) {
        let r = self.ed.plane_r.max(0.01);
        let class = self.ed.plane_class;
        let detail = self
            .detail
            .as_ref()
            .filter(|d| x >= d.window[0] && x <= d.window[2] && y >= d.window[1] && y <= d.window[3])
            .map(|d| d.store.clone());
        let Some(store) = detail.or_else(|| self.store.clone()) else {
            return;
        };
        let live = |i: usize| {
            !store.deleted.get(i).copied().unwrap_or(false)
                && !store.outside.get(i).copied().unwrap_or(false)
        };
        let pts = (0..store.len())
            .filter(|&i| store.class[i] == class && live(i))
            .map(|i| store.xyz(i));
        let Some((coef, sigma, n)) = edit::fit_surface(pts, x, y, r, self.ed.plane_quad) else {
            self.log(format!(
                "Plano local: en ese círculo (radio {r:.1} m) no hay puntos suficientes de la clase {class} para ajustar {}. Agranda el radio o elige otra clase.",
                if self.ed.plane_quad { "una curva (hacen falta 6, no alineados)" } else { "un plano (hacen falta 3, no alineados)" }
            ));
            return;
        };
        let tol = if self.ed.plane_sigma {
            (self.ed.plane_k * sigma).max(0.02)
        } else {
            self.ed.plane_tol.max(0.0)
        };
        let (lo, hi) = match self.ed.plane_side {
            PlaneSide::Above => (tol, f64::INFINITY),
            PlaneSide::Below => (f64::NEG_INFINITY, -tol),
            PlaneSide::Near => (-tol, tol),
        };
        self.log(format!(
            "Plano local: {n} puntos de la clase {class}, desviación {sigma:.2} m, tolerancia {tol:.2} m."
        ));
        let op = SelOp {
            mode: self.mode_now(ctx),
            region: Region::PlaneBand {
                cx: x,
                cy: y,
                r,
                coef,
                lo,
                hi,
            },
            attr: self.ed.attr.clone(),
        };
        self.queue(Cmd::AddOp(Box::new(op)));
    }

    /// Dibuja la selección en curso sobre la planta.
    pub fn paint_plan_select(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        w: usize,
        h: usize,
        resp: &egui::Response,
    ) {
        let trail: Vec<Pos2> = self
            .ed
            .draft
            .iter()
            .map(|p| {
                let s = self.plan.to_screen(p[0], p[1], w, h);
                Pos2::new(s.0 as f32, s.1 as f32) + rect.min.to_vec2()
            })
            .collect();
        if is_brush(self.nav) {
            paint_brush(
                painter,
                self.nav,
                resp,
                &trail,
                self.ed.brush_px as f32,
                Color32::from_rgb(0, 255, 255),
            );
            return;
        }
        if is_line(self.nav) {
            painter.text(
                rect.left_top() + Vec2::new(8.0, 6.0),
                Align2::LEFT_TOP,
                "Esta herramienta se usa en la ventana del perfil, no en la planta",
                FontId::proportional(12.0),
                Color32::from_rgb(0, 255, 255),
            );
            return;
        }
        if self.nav == Nav::SelPlane {
            // el círculo con el que se ajustará el plano, a escala
            if let Some(h) = resp.hover_pos() {
                let c = Color32::from_rgb(0, 255, 255);
                painter.circle(
                    h,
                    (self.ed.plane_r * self.plan.scale) as f32,
                    c.gamma_multiply(0.10),
                    Stroke::new(1.5, c),
                );
            }
            return;
        }
        paint_shape(
            painter,
            self.nav,
            resp,
            self.drag_start,
            &trail,
            Color32::from_rgb(0, 255, 255),
        );
    }

    /// Herramientas de selección en la vista 3D: las formas se dibujan en la pantalla y eligen cada
    /// punto que se proyecta dentro, delante o detrás (limitadas a una rebanada cuando está
    /// activada).
    pub fn view3d_select(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        resp: &egui::Response,
        rect: Rect,
        w: usize,
        h: usize,
    ) {
        if resp.drag_started_by(egui::PointerButton::Primary) {
            self.ed.d3start = resp.interact_pointer_pos();
        }
        let start = self.ed.d3start;
        let to = move |p: Pos2| [f64::from(p.x - rect.left()), f64::from(p.y - rect.top())];
        if let Some(poly) = finished_shape(self.nav, resp, start, &mut self.ed.d3draft, &to, 1.0) {
            let depth = self.ed.slab.then_some((
                self.orbit.dist - self.ed.slab_half,
                self.orbit.dist + self.ed.slab_half,
            ));
            let op = SelOp {
                mode: self.mode_now(ctx),
                region: Region::View(ViewCut::new(&self.orbit, w, h, poly, depth)),
                attr: self.ed.attr.clone(),
            };
            self.queue(Cmd::AddOp(Box::new(op)));
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            self.ed.d3start = None;
        }
        let trail: Vec<Pos2> = self
            .ed
            .d3draft
            .iter()
            .map(|p| Pos2::new(p[0] as f32, p[1] as f32) + rect.min.to_vec2())
            .collect();
        paint_shape(
            painter,
            self.nav,
            resp,
            self.ed.d3start,
            &trail,
            Color32::from_rgb(0, 255, 255),
        );
    }

    /// Herramientas de selección en la ventana de perfil: las formas viven en (distancia,
    /// elevación).
    pub fn profile_select(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        resp: &egui::Response,
        rect: Rect,
        bx: ProfileBox,
        (lo, hi): (f64, f64),
    ) {
        let (w, h) = (
            rect.width().max(8.0).floor() as f64,
            rect.height().max(8.0).floor() as f64,
        );
        let len = bx.length().max(1e-9);
        let k = (w / len).min(h / (hi - lo).max(1e-9));
        let (ox, oy) = ((w - len * k) / 2.0, (h - (hi - lo) * k) / 2.0);
        let to = move |p: Pos2| {
            [
                (f64::from(p.x - rect.left()) - ox) / k,
                lo + (h - oy - f64::from(p.y - rect.top())) / k,
            ]
        };
        let back = move |u: f64, z: f64| {
            Pos2::new(
                rect.left() + (ox + u * k) as f32,
                rect.top() + (h - oy - (z - lo) * k) as f32,
            )
        };
        if !is_select(self.nav) {
            return;
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            self.ed.pstart = resp.interact_pointer_pos();
        }
        let color = Color32::from_rgb(0, 255, 255);
        if is_brush(self.nav) {
            if let Some(b) = brush_stroke(
                self.nav,
                resp,
                &mut self.ed.brush_px,
                &mut self.ed.pdraft,
                &to,
                k,
            ) {
                let op = SelOp {
                    mode: self.mode_now(ctx),
                    region: Region::ProfileBrush { bx, brush: b },
                    attr: self.ed.attr.clone(),
                };
                self.queue(Cmd::AddOp(Box::new(op)));
            }
            let trail: Vec<Pos2> = self.ed.pdraft.iter().map(|p| back(p[0], p[1])).collect();
            paint_brush(
                painter,
                self.nav,
                resp,
                &trail,
                self.ed.brush_px as f32,
                color,
            );
            return;
        }
        if is_line(self.nav) {
            // una recta se arrastra; una línea quebrada se hace con un clic por vértice, y se acaba
            // con doble clic o con el botón derecho
            let mut done: Option<Vec<[f64; 2]>> = None;
            if resp.drag_stopped_by(egui::PointerButton::Primary)
                && let (Some(a), Some(b)) = (self.ed.pstart, resp.interact_pointer_pos())
                && (a.x - b.x).abs() >= 4.0
            {
                self.ed.pdraft.clear();
                done = Some(vec![to(a), to(b)]);
            } else if resp.double_clicked_by(egui::PointerButton::Primary)
                || resp.triple_clicked_by(egui::PointerButton::Primary)
            {
                // egui cuenta como triple un tercer clic cercano en el tiempo aunque los dos primeros
                // estén lejos entre sí: para acabar la línea vale cualquiera de los dos
                done = Some(std::mem::take(&mut self.ed.pdraft));
            } else if resp.clicked_by(egui::PointerButton::Primary)
                && let Some(p) = resp.interact_pointer_pos()
            {
                self.ed.pdraft.push(to(p));
            } else if resp.secondary_clicked()
                || (resp.hovered() && ctx.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                if self.ed.pdraft.len() >= 2 {
                    done = Some(std::mem::take(&mut self.ed.pdraft));
                } else {
                    self.ed.pdraft.clear();
                }
            }
            if let Some(mut path) = done {
                path.dedup_by(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9);
                if path.len() >= 2 {
                    self.finish_line(ctx, bx, edit::sorted_path(path));
                }
            }
            // qué hacer, a la vista mientras no se está trazando nada
            painter.text(
                rect.left_top() + Vec2::new(8.0, 6.0),
                Align2::LEFT_TOP,
                if self.ed.pdraft.is_empty() {
                    "Traza la línea: arrastra para una recta, o haz clic en cada vértice"
                } else {
                    "Doble clic, Intro o botón derecho para acabar la línea"
                },
                FontId::proportional(12.0),
                color,
            );
            // lo que se está trazando: la recta que se arrastra, o los vértices de la línea quebrada
            let mut trace: Vec<Pos2> = self.ed.pdraft.iter().map(|p| back(p[0], p[1])).collect();
            if let (true, Some(a), Some(b)) = (
                resp.dragged_by(egui::PointerButton::Primary),
                self.ed.pstart,
                resp.interact_pointer_pos(),
            ) {
                trace = vec![a, b];
            } else if let (false, Some(h)) = (trace.is_empty(), resp.hover_pos()) {
                trace.push(h);
            }
            for p in &self.ed.pdraft {
                painter.circle_filled(back(p[0], p[1]), 3.5, color);
            }
            if trace.len() >= 2 {
                painter.add(egui::Shape::line(trace.clone(), Stroke::new(2.0, color)));
                let pts: Vec<[f64; 2]> = trace.iter().map(|p| to(*p)).collect();
                let (up, down) = self.line_band();
                // la franja que se toma, a escala; sin límite, un trazo grueso a ese lado
                for w in pts.windows(2).zip(trace.windows(2)) {
                    let (v, sc) = w;
                    match (up, down) {
                        (Some(u), Some(d)) => painter.add(egui::Shape::convex_polygon(
                            vec![
                                back(v[0][0], v[0][1] + u),
                                back(v[1][0], v[1][1] + u),
                                back(v[1][0], v[1][1] - d),
                                back(v[0][0], v[0][1] - d),
                            ],
                            color.gamma_multiply(0.18),
                            Stroke::NONE,
                        )),
                        (Some(u), None) if u > 0.0 => painter.add(egui::Shape::convex_polygon(
                            vec![sc[0], sc[1], back(v[1][0], v[1][1] + u), back(v[0][0], v[0][1] + u)],
                            color.gamma_multiply(0.18),
                            Stroke::NONE,
                        )),
                        (None, Some(d)) if d > 0.0 => painter.add(egui::Shape::convex_polygon(
                            vec![sc[0], sc[1], back(v[1][0], v[1][1] - d), back(v[0][0], v[0][1] - d)],
                            color.gamma_multiply(0.18),
                            Stroke::NONE,
                        )),
                        _ => {
                            let off = match (up, down) {
                                (Some(_), None) => -14.0,
                                _ => 14.0,
                            };
                            painter.line_segment(
                                [sc[0] + Vec2::new(0.0, off), sc[1] + Vec2::new(0.0, off)],
                                Stroke::new(6.0, color.gamma_multiply(0.25)),
                            )
                        }
                    };
                }
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) {
                self.ed.pstart = None;
            }
            return;
        }
        let start = self.ed.pstart;
        if let Some(poly) = finished_shape(self.nav, resp, start, &mut self.ed.pdraft, &to, k) {
            let op = SelOp {
                mode: self.mode_now(ctx),
                region: Region::Profile { bx, poly },
                attr: self.ed.attr.clone(),
            };
            self.queue(Cmd::AddOp(Box::new(op)));
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            self.ed.pstart = None;
        }
        let trail: Vec<Pos2> = self.ed.pdraft.iter().map(|p| back(p[0], p[1])).collect();
        paint_shape(
            painter,
            self.nav,
            resp,
            self.ed.pstart,
            &trail,
            Color32::from_rgb(0, 255, 255),
        );
    }

    /// La franja que toma la herramienta de línea activa, en cota por encima y por debajo de la
    /// línea: `None` en un lado es «sin límite» (todo lo que queda de ese lado).
    fn line_band(&self) -> (Option<f64>, Option<f64>) {
        match self.nav {
            Nav::SelAbove => (Some(self.ed.side_height.max(0.0)), None),
            Nav::SelBelow => (None, Some(self.ed.side_height.max(0.0))),
            _ => (Some(self.ed.near_up.max(0.0)), Some(self.ed.near_down.max(0.0))),
        }
    }

    /// Acaba una línea trazada en el perfil (`path` ordenada por distancia): según la herramienta,
    /// añade a la selección lo que queda sobre, bajo o cerca de ella, o clasifica de una vez los tres
    /// tramos (sobre, cerca y bajo) con una sola edición agrupada.
    fn finish_line(&mut self, ctx: &egui::Context, bx: ProfileBox, path: Vec<[f64; 2]>) {
        let (up, down) = (self.ed.near_up.max(0.0), self.ed.near_down.max(0.0));
        match self.nav {
            Nav::SelAbove | Nav::SelBelow => {
                let op = SelOp {
                    mode: self.mode_now(ctx),
                    region: Region::Side {
                        bx,
                        path,
                        above: self.nav == Nav::SelAbove,
                        height: self.ed.side_height.max(0.0),
                    },
                    attr: self.ed.attr.clone(),
                };
                self.queue(Cmd::AddOp(Box::new(op)));
            }
            Nav::SelNear => {
                let op = SelOp {
                    mode: self.mode_now(ctx),
                    region: Region::Near {
                        bx,
                        path,
                        above: up,
                        below: down,
                    },
                    attr: self.ed.attr.clone(),
                };
                self.queue(Cmd::AddOp(Box::new(op)));
            }
            Nav::ClassLine => {
                let Some(ses) = &self.ed.session else { return };
                let group = ses.new_group();
                let attr = visible_attr(&self.style, self.ed.attr.clone());
                // tres franjas que no se solapan: sobre la tolerancia, dentro de ella y bajo ella
                let bands = [
                    ("sobre", f64::INFINITY, -up),
                    ("cerca", up, down),
                    ("bajo", -down, f64::INFINITY),
                ];
                let edits: Vec<Edit> = bands
                    .iter()
                    .zip(self.ed.line_classes)
                    .filter(|(_, (on, _))| *on)
                    .map(|((name, above, below), (_, class))| Edit {
                        // con un foco puesto, la edición no sale de él
                        sel: with_focus(
                            &[SelOp {
                                mode: Mode::New,
                                region: Region::Near {
                                    bx,
                                    path: path.clone(),
                                    above: *above,
                                    below: *below,
                                },
                                attr: attr.clone(),
                            }],
                            &self.ed.focus,
                        ),
                        action: edit::Action::Reclassify(class),
                        label: format!("Cerca de la línea: {name} → clase {class}"),
                        enabled: true,
                        group,
                    })
                    .collect();
                if edits.is_empty() {
                    self.log
                        .push("Clasificar a lo largo de una línea: marca al menos un tramo.".into());
                } else {
                    self.queue(Cmd::Apply(edits));
                }
            }
            _ => {}
        }
    }

    /// Cuando hay puntos seleccionados: los archivos que los contienen como rutas `…_sel`, los
    /// pares (nube, archivo `_sel`) que escribir, el número de puntos y la propia selección.
    #[allow(clippy::type_complexity)]
    pub fn selection_inputs(
        &self,
    ) -> Option<(Vec<PathBuf>, Vec<(PathBuf, PathBuf)>, usize, Vec<SelOp>)> {
        let s = self.store.as_ref()?;
        if self.ed.ops.is_empty() || self.ed.sel_count == 0 || s.selected.is_empty() {
            return None;
        }
        // el archivo k del almacén es la k-ésima nube LAS o LAZ
        let las: Vec<&PathBuf> = self
            .clouds
            .iter()
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("las") || e.eq_ignore_ascii_case("laz"))
            })
            .collect();
        let mut count = vec![0usize; las.len()];
        for (i, on) in s.selected.iter().enumerate() {
            if *on && let Some(c) = count.get_mut(usize::from(s.file[i])) {
                *c += 1;
            }
        }
        let mut paths = Vec::new();
        let mut pairs = Vec::new();
        for (k, c) in las.iter().enumerate() {
            if count[k] == 0 {
                continue;
            }
            let ext = c
                .extension()
                .map_or("las".to_string(), |e| e.to_string_lossy().into_owned());
            let out = hipso_core::paths::results_dir(c)
                .join(format!("{}_sel.{ext}", hipso_core::paths::stem(c)));
            paths.push(out.clone());
            pairs.push(((*c).clone(), out));
        }
        (!pairs.is_empty()).then(|| {
            (
                paths,
                pairs,
                self.ed.sel_count,
                with_focus(&self.ed.ops, &self.ed.focus),
            )
        })
    }

    pub fn edit_status(&self) -> Option<String> {
        let s = self.store.as_ref()?;
        let ses = self.ed.session.as_ref()?;
        let mut parts = Vec::new();
        if let Some(f) = &self.ed.focus {
            parts.push(format!(
                "foco: {} puntos (≈ {} en el archivo)",
                f.shown,
                f.shown as u64 * s.stride
            ));
        }
        if self.ed.sel_count > 0 {
            parts.push(format!(
                "{} seleccionados (≈ {} en el archivo)",
                self.ed.sel_count,
                self.ed.sel_count as u64 * s.stride
            ));
        }
        if ses.edits.len() > self.ed.saved {
            parts.push(format!(
                "{} cambio(s) sin guardar",
                ses.edits.len() - self.ed.saved
            ));
        } else if self.ed.toggled {
            parts.push("cambios sin guardar".into());
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Aplica las órdenes de edición en cola. Se ejecuta entre fotogramas, cuando nada más retiene
    /// el almacén.
    pub fn process_edit_cmds(&mut self) {
        if self.ed.cmds.is_empty() || self.store.is_none() || self.ed.session.is_none() {
            return;
        }
        let cmds = std::mem::take(&mut self.ed.cmds);
        let Some(arc) = self.store.as_mut() else {
            return;
        };
        let store = Arc::make_mut(arc);
        let mut edited = false;
        let mut focus_changed = false;
        for c in cmds {
            match c {
                Cmd::AddOp(op) => {
                    if op.mode == Mode::New {
                        self.ed.ops.clear();
                    }
                    let mut op = *op;
                    op.attr = visible_attr(&self.style, op.attr);
                    self.ed.ops.push(op);
                }
                Cmd::Clear => self.ed.ops.clear(),
                Cmd::All => {
                    self.ed.ops = vec![SelOp {
                        mode: Mode::New,
                        region: Region::All,
                        attr: visible_attr(&self.style, self.ed.attr.clone()),
                    }];
                }
                Cmd::Invert => {
                    self.ed.ops.push(SelOp {
                        mode: Mode::Invert,
                        region: Region::All,
                        attr: Attr::default(),
                    });
                    // lo oculto por los filtros de la vista no entra en la inversión
                    self.ed.ops.push(SelOp {
                        mode: Mode::Intersect,
                        region: Region::All,
                        attr: visible_attr(&self.style, Attr::default()),
                    });
                }
                Cmd::Delete | Cmd::Reclass(_) => {
                    if self.ed.ops.is_empty() {
                        self.log
                            .push("Edición: no hay puntos seleccionados.".into());
                        continue;
                    }
                    let action = match c {
                        Cmd::Reclass(k) => edit::Action::Reclassify(k),
                        _ => edit::Action::Delete,
                    };
                    let sel_ops = with_focus(&self.ed.ops, &self.ed.focus);
                    let n = edit::selection(store, &sel_ops)
                        .iter()
                        .filter(|b| **b)
                        .count();
                    if n == 0 {
                        self.log
                            .push("Edición: la selección no contiene puntos.".into());
                        continue;
                    }
                    let label = match action {
                        edit::Action::Delete => format!("Eliminar {n} puntos"),
                        edit::Action::Reclassify(k) => {
                            format!("Clase {k} ({}) a {n} puntos", self.ctable.name(k))
                        }
                    };
                    self.ed.ops.clear();
                    let e = Edit::new(sel_ops, action, label.clone());
                    if let Some(ses) = &mut self.ed.session {
                        ses.apply(store, e);
                    }
                    self.log.push(format!("Edición: {label} (en pantalla)"));
                    edited = true;
                }
                Cmd::Focus => {
                    let sel_ops = with_focus(&self.ed.ops, &self.ed.focus);
                    let picked = edit::selection(store, &sel_ops);
                    let Some((window, shown)) = focus_window(store, &picked) else {
                        self.log
                            .push("Foco: no hay puntos seleccionados que aislar.".into());
                        continue;
                    };
                    let inner = edit::geometry_of(&sel_ops);
                    store.outside = edit::outside_focus(store, &inner);
                    self.ed.focus = Some(Focus {
                        inner,
                        window,
                        shown,
                    });
                    self.ed.ops.clear();
                    self.log.push(format!(
                        "Foco: {shown} puntos en pantalla. Todo lo demás queda oculto y fuera de las ediciones hasta pulsar «Volver a la nube»."
                    ));
                    focus_changed = true;
                }
                Cmd::Unfocus => {
                    if self.ed.focus.take().is_some() {
                        store.outside.clear();
                        self.ed.ops.clear();
                        self.log.push("Foco quitado: se ve toda la nube.".into());
                        focus_changed = true;
                    }
                }
                Cmd::Apply(list) => {
                    self.ed.ops.clear();
                    let n = list.len();
                    if let Some(ses) = &mut self.ed.session {
                        for e in list {
                            ses.apply(store, e);
                        }
                        self.log
                            .push(format!("Ediciones: {n} aplicadas en pantalla."));
                        edited = true;
                    }
                }
                Cmd::Undo | Cmd::Redo | Cmd::Discard | Cmd::Toggle(_) => {
                    self.ed.ops.clear();
                    if let Some(ses) = &mut self.ed.session {
                        match c {
                            Cmd::Undo => edited |= ses.undo(store),
                            Cmd::Redo => edited |= ses.redo(store),
                            Cmd::Toggle(k) => {
                                if ses.toggle(store, k) {
                                    self.ed.toggled = true;
                                    edited = true;
                                }
                            }
                            _ => {
                                ses.discard(store);
                                self.ed.saved = 0;
                                self.ed.toggled = false;
                                edited = true;
                            }
                        }
                    }
                }
            }
        }
        store.selected = if self.ed.ops.is_empty() {
            Vec::new()
        } else {
            edit::selection(store, &with_focus(&self.ed.ops, &self.ed.focus))
        };
        self.ed.sel_count = store.selected.iter().filter(|b| **b).count();
        let session_changed = edited;
        if focus_changed {
            // las ventanas de detalle se leyeron para otro foco: se vuelven a leer
            self.detail = None;
            self.detail_job = None;
            self.detail3 = None;
            self.detail3_job = None;
            self.tex_prof = None;
            edited = true;
        }
        if edited {
            self.class_counts = [0; 256];
            for (i, c) in store.class.iter().enumerate() {
                if !store.deleted.get(i).copied().unwrap_or(false) {
                    self.class_counts[usize::from(*c)] += 1;
                }
            }
            self.rev += 1;
            self.ed.edit_rev += 1;
            self.tin = None;
        }
        self.ed.sel_rev += 1;
        if session_changed {
            self.autosave_edits();
        }
    }

    /// Guarda la sesión de edición en la carpeta de la caché tras cada cambio: si el programa se
    /// cierra sin guardar la nube, al abrirla de nuevo se ofrece recuperar las ediciones. Sin
    /// ediciones, no deja nada.
    fn autosave_edits(&mut self) {
        let Some(path) = self.ed.session_file.clone() else {
            return;
        };
        let Some(ses) = &self.ed.session else { return };
        if ses.edits.is_empty() {
            let _ = std::fs::remove_file(&path);
            return;
        }
        if let Err(e) = edit::file::write(&path, &ses.edits) {
            self.log
                .push(format!("No se pudo guardar la sesión de edición: {e}"));
        }
    }

    /// Fija dónde se guarda la sesión de las nubes abiertas y, si quedó una sin guardar de otra vez,
    /// la ofrece. Se llama cuando termina de cargarse una nube.
    pub fn open_edit_session(&mut self) {
        let las: Vec<PathBuf> = self
            .clouds
            .iter()
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("las") || e.eq_ignore_ascii_case("laz"))
            })
            .cloned()
            .collect();
        self.ed.session_file = edit::file::session_path(&las);
        self.ed.recover = None;
        let Some(path) = self.ed.session_file.clone().filter(|p| p.is_file()) else {
            return;
        };
        match edit::file::read(&path) {
            Ok(list) if !list.is_empty() => {
                self.log.push(format!(
                    "Hay {} ediciones de una sesión anterior sin guardar: se pueden recuperar desde el panel Edición.",
                    list.len()
                ));
                self.ed.recover = Some(list);
            }
            Ok(_) => {}
            Err(e) => self.log.push(format!(
                "La sesión de edición anterior no se puede leer ({e}); se deja el archivo: {}",
                path.display()
            )),
        }
    }

    /// Escribe las ediciones de la sesión (todas, también las desactivadas) en un archivo que se
    /// puede aplicar a otra nube o con `hipso apply-edits`.
    pub fn save_edits_file(&mut self) {
        let Some(ses) = &self.ed.session else { return };
        if ses.edits.is_empty() {
            self.log.push("No hay ediciones que guardar.".into());
            return;
        }
        let Some(path) = crate::prefs::dialog()
            .add_filter("Ediciones de HIPSO", &["json"])
            .set_file_name("ediciones.json")
            .save_file()
        else {
            return;
        };
        match edit::file::write(&path, &ses.edits) {
            Ok(()) => self.log.push(format!(
                "{} ediciones guardadas en {}",
                ses.edits.len(),
                path.display()
            )),
            Err(e) => self.log.push(format!("No se pudieron guardar las ediciones: {e}")),
        }
    }

    /// Lee un archivo de ediciones y las aplica a la nube abierta.
    pub fn load_edits_file(&mut self) {
        let Some(path) = crate::prefs::dialog()
            .add_filter("Ediciones de HIPSO", &["json"])
            .pick_file()
        else {
            return;
        };
        match edit::file::read(&path) {
            Ok(list) if list.is_empty() => self.log.push("El archivo no tiene ediciones.".into()),
            Ok(list) => self.queue(Cmd::Apply(list)),
            Err(e) => self.log.push(format!("No se pudieron leer las ediciones: {e}")),
        }
    }

    /// Escribe las nubes editadas: cada punto de cada archivo pasa por las reglas de edición.
    pub fn save_edited(&mut self, ctx: &egui::Context) {
        let Some(ses) = &self.ed.session else { return };
        if ses.edits.is_empty() {
            self.log.push("No hay cambios que guardar.".into());
            return;
        }
        let edits = ses.active();
        if edits.is_empty() {
            self.log
                .push("Todas las ediciones están desactivadas: no hay nada que guardar.".into());
            return;
        }
        let saved_len = ses.edits.len();
        let inputs: Vec<PathBuf> = self
            .clouds
            .iter()
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("las") || e.eq_ignore_ascii_case("laz"))
            })
            .cloned()
            .collect();
        if inputs.is_empty() {
            self.log
                .push("Solo se pueden guardar nubes LAS/LAZ.".into());
            return;
        }
        let mut pairs = Vec::new();
        for i in &inputs {
            let ext = i
                .extension()
                .map_or("las".into(), |e| e.to_string_lossy().into_owned());
            let auto = hipso_core::paths::unique(&hipso_core::paths::product(i, "editada", &ext));
            let out = if inputs.len() == 1 && !self.ed.auto_save {
                match crate::prefs::dialog()
                    .add_filter("LAS/LAZ", &["las", "laz"])
                    .set_directory(auto.parent().unwrap_or(Path::new(".")))
                    .set_file_name(
                        auto.file_name()
                            .map_or(String::new(), |n| n.to_string_lossy().into_owned()),
                    )
                    .save_file()
                {
                    Some(p) => p,
                    None => return,
                }
            } else {
                auto
            };
            pairs.push((i.clone(), out));
        }
        self.ed.saved = saved_len;
        self.ed.toggled = false;
        let outputs: Vec<PathBuf> = pairs.iter().map(|p| p.1.clone()).collect();
        self.ed.pending_swap = Some((pairs.iter().map(|p| p.0.clone()).collect(), outputs.clone()));
        let session_file = self.ed.session_file.clone();
        let launch = Launch {
            outputs,
            label: "Guardar nube editada".into(),
            task: Box::new(move || {
                let mut lines = Vec::new();
                for (i, o) in &pairs {
                    let r = edit::save(i, o, &edits)?;
                    lines.push(edit::report_text(&r, o));
                }
                // las ediciones ya están en la nube guardada: no se ofrecerán como pendientes
                if let Some(f) = &session_file {
                    let _ = std::fs::remove_file(f);
                }
                Ok(lines.join("\n"))
            }),
            show: vec![],
        };
        self.start(ctx, launch);
    }

    /// Tras guardar la nube editada, el resultado se abre encima de la original y las dos se
    /// superponen (el perfil y la planta mezclan los puntos de ambas). Esta ventana ofrece cerrar la
    /// original y quedarse con el resultado, sin perder la vista.
    pub fn swap_window(&mut self, ctx: &egui::Context) {
        let Some((originals, outputs)) = self.ed.swap_offer.clone() else {
            return;
        };
        let names: Vec<String> = outputs
            .iter()
            .filter_map(|o| o.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let ready = self.loading.is_none() && outputs.iter().all(|o| self.clouds.contains(o));
        let (mut swap, mut keep) = (false, false);
        egui::Window::new("Nube editada guardada")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -40.0])
            .show(ctx, |ui| {
                ui.label(format!("Guardada: {}.", names.join(", ")));
                ui.label("Está abierta encima de la original y las dos se superponen: en la planta, el perfil y el 3D se mezclan los puntos de ambas.");
                ui.horizontal(|ui| {
                    swap = ui
                        .add_enabled(ready, egui::Button::new("Cerrar la original y quedarme con el resultado"))
                        .on_disabled_hover_text("Espera a que termine de cargarse el resultado.")
                        .clicked();
                    keep = ui.button("Dejar las dos").clicked();
                });
            });
        if swap {
            self.swap_originals(ctx, &originals);
            self.ed.swap_offer = None;
        } else if keep {
            self.ed.swap_offer = None;
        }
    }

    /// Cierra las nubes `originals` conservando la vista y la caja del perfil.
    pub fn swap_originals(&mut self, ctx: &egui::Context, originals: &[PathBuf]) {
        let profile = self.profile;
        for o in originals {
            let Some(k) = self.clouds.iter().position(|c| c == o) else {
                continue;
            };
            if !self.drop_cloud_in_memory(ctx, k) {
                // sin el almacén en memoria (o con una carga en curso): se quita y se recarga
                self.clouds.remove(k);
                self.reload(ctx);
                return;
            }
        }
        self.profile = profile;
        self.tex_prof = None;
        self.prof_data = None;
    }

    pub fn edit_keys(&mut self, ctx: &egui::Context) {
        // las herramientas vectoriales tienen sus propias teclas
        if self.edit_layer_on() {
            return;
        }
        let (ctrl, del, z, y, a, i, esc) = ctx.input(|s| {
            (
                s.modifiers.command,
                s.key_pressed(Key::Delete) || s.key_pressed(Key::Backspace),
                s.key_pressed(Key::Z),
                s.key_pressed(Key::Y),
                s.key_pressed(Key::A),
                s.key_pressed(Key::I),
                s.key_pressed(Key::Escape),
            )
        });
        if del && !ctrl {
            self.queue(Cmd::Delete);
        }
        if ctrl && z {
            self.queue(Cmd::Undo);
        }
        if ctrl && y {
            self.queue(Cmd::Redo);
        }
        if ctrl && a {
            self.queue(Cmd::All);
        }
        if ctrl && i {
            self.queue(Cmd::Invert);
        }
        if esc {
            self.ed.draft.clear();
            self.ed.pdraft.clear();
            if !self.ed.ops.is_empty() {
                self.queue(Cmd::Clear);
            }
        }
    }

    pub fn class_combo(&mut self, ui: &mut egui::Ui) {
        let present = self.class_counts;
        egui::ComboBox::from_id_salt("target_class")
            .width(190.0)
            .selected_text(format!(
                "{} · {}",
                self.ed.class,
                self.ctable.name(self.ed.class)
            ))
            .show_ui(ui, |ui| {
                for c in self.ctable.known(&present) {
                    let label = format!("{c} · {}", self.ctable.name(c));
                    ui.selectable_value(&mut self.ed.class, c, label);
                }
            })
            .response
            .on_hover_text("Clase que se asigna con «Cambiar clase».");
        let mut code = i32::from(self.ed.class);
        if ui
            .add(
                crate::numfield::NumField::new(&mut code)
                    .range(0..=255)
                    .prefix("código "),
            )
            .on_hover_text("Cualquier código 0–255; los nuevos se añaden en Vista > Clases")
            .changed()
        {
            self.ed.class = code as u8;
        }
    }

    // ---------------------------------------------------------------------------- pestaña lateral

    /// El panel de edición: todas las herramientas de edición de puntos en un solo sitio, junto a
    /// la cinta, de modo que las herramientas de Vista y estas pueden estar a mano juntas.
    pub fn edit_side(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        if self.store.is_none() {
            ui.label("Abre una nube para editar sus puntos.");
            return;
        }
        let dirty = self
            .ed
            .session
            .as_ref()
            .is_some_and(|s| s.edits.len() > self.ed.saved || self.ed.toggled);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Las ediciones se aplican a todos los puntos del archivo al guardar, no solo a los que se ven.").size(11.0).color(t.weak));
                crate::collapsing(egui::RichText::new("Selección").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                            for (icon, name, n) in [
                                ("@selrect", "Seleccionar con rectángulo", Nav::SelRect),
                                ("@aoi", "Seleccionar con polígono", Nav::SelPoly),
                                ("@lasso", "Seleccionar con lazo", Nav::SelLasso),
                                ("@circle", "Seleccionar con círculo", Nav::SelCircle),
                                ("@brush", "Pincel redondo (plano y perfil)", Nav::SelBrush),
                                ("@brushsq", "Pincel cuadrado (plano y perfil)", Nav::SelBrushSq),
                                ("@above", "Seleccionar todo lo que queda sobre una línea del perfil", Nav::SelAbove),
                                ("@below", "Seleccionar todo lo que queda bajo una línea del perfil", Nav::SelBelow),
                                ("@linenear", "Seleccionar lo que queda cerca de una línea del perfil", Nav::SelNear),
                                ("@lineclass", "Clasificar de una vez sobre, cerca y bajo una línea del perfil", Nav::ClassLine),
                                ("@planefit", "Seleccionar con plano local", Nav::SelPlane),
                            ] {
                                if ribbon::tool_button(ui, &t, icon, name, self.nav == n).clicked()
                                {
                                    self.act(ctx, Action::Nav(n));
                                }
                            }
                        });
                        if self.nav == Nav::SelPlane {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Radio");
                                ui.add(
                                    egui::DragValue::new(&mut self.ed.plane_r)
                                        .range(0.1..=10_000.0)
                                        .speed(0.1)
                                        .suffix(" m"),
                                );
                                ui.label("Ajustar a la clase");
                                ui.add(egui::DragValue::new(&mut self.ed.plane_class).range(0..=255));
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.radio_value(&mut self.ed.plane_side, PlaneSide::Above, "Sobre")
                                    .on_hover_text("Los puntos por encima de la superficie ajustada: vegetación, ruido alto.");
                                ui.radio_value(&mut self.ed.plane_side, PlaneSide::Below, "Bajo")
                                    .on_hover_text("Los puntos por debajo de la superficie ajustada: ruido bajo el suelo.");
                                ui.radio_value(&mut self.ed.plane_side, PlaneSide::Near, "Cerca")
                                    .on_hover_text("Los puntos a menos de la tolerancia de la superficie: sirve para añadir al suelo.");
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.radio_value(&mut self.ed.plane_quad, false, "Plano");
                                ui.radio_value(&mut self.ed.plane_quad, true, "Curva")
                                    .on_hover_text("Superficie de segundo grado: sigue una vaguada o un cerro dentro del círculo.");
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Tolerancia");
                                if self.ed.plane_sigma {
                                    ui.add(
                                        egui::DragValue::new(&mut self.ed.plane_k)
                                            .range(0.5..=20.0)
                                            .speed(0.05)
                                            .suffix(" σ"),
                                    );
                                } else {
                                    ui.add(
                                        egui::DragValue::new(&mut self.ed.plane_tol)
                                            .range(0.0..=1000.0)
                                            .speed(0.01)
                                            .suffix(" m"),
                                    );
                                }
                                ui.checkbox(&mut self.ed.plane_sigma, "en σ")
                                    .on_hover_text("Mide la tolerancia en desviaciones típicas de los puntos del ajuste, en vez de en metros.");
                            });
                            ui.label(
                                egui::RichText::new(
                                    "Clic en la planta: se ajusta una superficie a los puntos de esa clase dentro del círculo y se selecciona lo que queda sobre, bajo o cerca de ella. Después, Cambiar clase o Eliminar.",
                                )
                                .size(11.0)
                                .color(t.weak),
                            );
                        } else if is_brush(self.nav) {
                            ui.horizontal(|ui| {
                                ui.label("Tamaño del pincel");
                                ui.add(
                                    egui::Slider::new(&mut self.ed.brush_px, 2.0..=300.0)
                                        .suffix(" px")
                                        .logarithmic(true),
                                );
                            });
                            ui.label(
                                egui::RichText::new(
                                    "Arrastra para pintar; los puntos tocados se seleccionan. [ y ] cambian el tamaño. Vale en planta y en perfil.",
                                )
                                .size(11.0)
                                .color(t.weak),
                            );
                        } else if is_line(self.nav) {
                            if matches!(self.nav, Nav::SelAbove | Nav::SelBelow) {
                                ui.horizontal(|ui| {
                                    ui.label(if self.nav == Nav::SelAbove {
                                        "Altura sobre la línea"
                                    } else {
                                        "Altura bajo la línea"
                                    });
                                    ui.add(
                                        egui::DragValue::new(&mut self.ed.side_height)
                                            .range(0.0..=100_000.0)
                                            .speed(0.1)
                                            .suffix(" m")
                                            .custom_formatter(|v, _| {
                                                if v <= 0.0 { "sin límite".to_string() } else { format!("{v:.2}") }
                                            }),
                                    );
                                });
                            } else {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label("Cerca = hasta");
                                    ui.add(
                                        egui::DragValue::new(&mut self.ed.near_up)
                                            .range(0.0..=1000.0)
                                            .speed(0.01)
                                            .prefix("↑ ")
                                            .suffix(" m"),
                                    )
                                    .on_hover_text("Hasta cuánto por encima de la línea se considera cerca.");
                                    ui.add(
                                        egui::DragValue::new(&mut self.ed.near_down)
                                            .range(0.0..=1000.0)
                                            .speed(0.01)
                                            .prefix("↓ ")
                                            .suffix(" m"),
                                    )
                                    .on_hover_text("Hasta cuánto por debajo de la línea se considera cerca.");
                                });
                            }
                            if self.nav == Nav::ClassLine {
                                for (k, name) in ["Sobre", "Cerca", "Bajo"].iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.checkbox(&mut self.ed.line_classes[k].0, *name);
                                        ui.label("→ clase");
                                        ui.add(
                                            egui::DragValue::new(&mut self.ed.line_classes[k].1)
                                                .range(0..=255),
                                        );
                                    });
                                }
                            }
                            ui.label(
                                egui::RichText::new(match self.nav {
                                    Nav::SelNear => "En el perfil, traza una línea: se selecciona lo que queda cerca de ella. Arrastra para una recta, o haz un clic por vértice y doble clic (o botón derecho) para acabar una línea quebrada.",
                                    Nav::ClassLine => "En el perfil, traza una línea (arrastra para una recta, o un clic por vértice y doble clic para acabar): de una vez se clasifica lo que queda sobre, cerca y bajo ella, cada tramo con su clase. Se deshace de una vez. El filtro de atributos limita qué puntos se tocan.",
                                    _ => "En el perfil, traza una línea: se selecciona todo lo que queda por encima (o por debajo) a lo largo de ella, dentro de la caja del perfil. Arrastra para una recta, o haz un clic por vértice y doble clic (o botón derecho) para acabar una línea quebrada.",
                                })
                                .size(11.0)
                                .color(t.weak),
                            );
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                            if ribbon::small_button(ui, &t, "@selall", "Todo", false).clicked() {
                                self.queue(Cmd::All);
                            }
                            if ribbon::small_button(ui, &t, "@invert", "Invertir", false).clicked()
                            {
                                self.queue(Cmd::Invert);
                            }
                            if ribbon::small_button(ui, &t, "@deselect", "Quitar", false).clicked() {
                                self.queue(Cmd::Clear);
                            }
                        });
                        ui.label(
                            egui::RichText::new("Combinar selección")
                                .size(11.0)
                                .color(t.weak),
                        );
                        ui.horizontal_wrapped(|ui| {
                            for (m, n) in [
                                (Mode::New, "Nueva"),
                                (Mode::Add, "Añadir"),
                                (Mode::Subtract, "Restar"),
                                (Mode::Intersect, "Intersecar"),
                            ] {
                                ui.radio_value(&mut self.ed.mode, m, n).on_hover_text(match m {
                                    Mode::New => "La nueva selección sustituye a la anterior.",
                                    Mode::Add => "Suma los puntos nuevos a la selección (Mayús).",
                                    Mode::Subtract => "Resta los puntos nuevos de la selección (Alt).",
                                    _ => "Deja solo los puntos que están en las dos selecciones (Ctrl).",
                                });
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut self.ed.slab, "Rebanada en 3D ±").on_hover_text(
                                "En la vista 3D, la selección solo alcanza los puntos a esta distancia del plano del objetivo (el centro de giro): evita coger ramas del otro lado del tronco.",
                            );
                            ui.add_enabled(
                                self.ed.slab,
                                egui::DragValue::new(&mut self.ed.slab_half)
                                    .range(0.01..=1000.0)
                                    .speed(0.05)
                                    .suffix(" m"),
                            );
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                            if ribbon::small_button(
                                ui,
                                &t,
                                "@selrect",
                                "Aislar selección",
                                self.ed.focus.is_some(),
                            )
                            .clicked()
                            {
                                self.queue(Cmd::Focus);
                            }
                            if self.ed.focus.is_some()
                                && ribbon::small_button(ui, &t, "@selall", "Volver a la nube", false)
                                    .clicked()
                            {
                                self.queue(Cmd::Unfocus);
                            }
                        });
                        ui.label(
                            egui::RichText::new(format!(
                                "Seleccionados: {} puntos en pantalla (≈ {} en el archivo)",
                                self.ed.sel_count,
                                self.ed.sel_count as u64
                                    * self.store.as_ref().map_or(1, |s| s.stride)
                            ))
                            .strong(),
                        );
                        self.attr_filter(ui, &t);
                    });
                crate::collapsing(egui::RichText::new("Editar").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                            if ribbon::small_button(ui, &t, "@trash", "Eliminar", false)
                                .clicked()
                            {
                                self.queue(Cmd::Delete);
                            }
                            if ribbon::small_button(ui, &t, "@reclass", "Cambiar clase", false)
                                .clicked()
                            {
                                self.queue(Cmd::Reclass(self.ed.class));
                            }
                        });
                        self.class_combo(ui);
                        ui.horizontal_wrapped(|ui| {
                            for (c, n) in [
                                (2u8, "Suelo"),
                                (3, "Veg. baja"),
                                (4, "Veg. media"),
                                (5, "Veg. alta"),
                                (6, "Edificio"),
                                (7, "Ruido"),
                                (1, "Sin clasificar"),
                            ] {
                                if ui
                                    .small_button(n)
                                    .on_hover_text(format!(
                                        "Asigna la clase {c} ({}) a los puntos seleccionados.",
                                        self.ctable.name(c)
                                    ))
                                    .clicked()
                                {
                                    self.ed.class = c;
                                    self.queue(Cmd::Reclass(c));
                                }
                            }
                        });
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                            if ribbon::small_button(
                                ui,
                                &t,
                                "@hist",
                                "Histogramas",
                                self.ed.hist.open,
                            )
                            .clicked()
                            {
                                self.ed.hist.open = !self.ed.hist.open;
                            }
                        });
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                            if ribbon::small_button(
                                ui,
                                &t,
                                "@save",
                                "Guardar nube editada…",
                                dirty,
                            )
                            .clicked()
                            {
                                self.ed.want_save = true;
                            }
                            if ribbon::small_button(ui, &t, "@discard", "Descartar todo", false)
                                .clicked()
                            {
                                self.queue(Cmd::Discard);
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                            if ribbon::small_button(ui, &t, "📝", "Guardar ediciones…", false)
                                .clicked()
                            {
                                self.save_edits_file();
                            }
                            if ribbon::small_button(ui, &t, "📂", "Cargar ediciones…", false)
                                .clicked()
                            {
                                self.load_edits_file();
                            }
                        });
                        if let Some(n) = self.ed.recover.as_ref().map(Vec::len) {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Quedaron {n} ediciones sin guardar de una sesión anterior."
                                ))
                                .size(11.0)
                                .strong(),
                            );
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Recuperar").clicked()
                                    && let Some(list) = self.ed.recover.take()
                                {
                                    self.queue(Cmd::Apply(list));
                                }
                                if ui.button("Descartar").clicked() {
                                    self.ed.recover = None;
                                    if let Some(f) = &self.ed.session_file {
                                        let _ = std::fs::remove_file(f);
                                    }
                                }
                            });
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(2.0);
                            if ribbon::small_button(ui, &t, "@undo", "Deshacer", false).clicked() {
                                self.queue(Cmd::Undo);
                            }
                            if ribbon::small_button(ui, &t, "@redo", "Rehacer", false).clicked() {
                                self.queue(Cmd::Redo);
                            }
                        });
                        ui.label(
                            egui::RichText::new(
                                self.edit_status().unwrap_or_else(|| "sin cambios".into()),
                            )
                            .size(11.0)
                            .color(t.weak),
                        );
                    });
                self.history(ui, &t);
            });
    }

    /// El historial de ediciones. Cada paso tiene una casilla: al desmarcarla se deshace esa
    /// edición sola y las demás se quedan como están.
    fn history(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let (n_edits, n_saved) = (
            self.ed.session.as_ref().map_or(0, |s| s.edits.len()),
            self.ed.saved,
        );
        crate::collapsing(egui::RichText::new(format!("Historial ({n_edits})")).strong())
            .default_open(true)
            .show(ui, |ui| {
                if n_edits == 0 {
                    ui.label(
                        egui::RichText::new("Aún no hay ediciones.")
                            .size(11.0)
                            .color(t.weak),
                    );
                    return;
                }
                ui.label(
                    egui::RichText::new("Quita la marca de una edición para deshacerla sin tocar las demás (✔ = ya guardada).")
                        .size(11.0)
                        .color(t.weak),
                );
                let mut toggle = None;
                if let Some(ses) = &self.ed.session {
                    for (k, e) in ses.edits.iter().enumerate() {
                        let mut on = e.enabled;
                        let mark = if k < n_saved { "✔ " } else { "" };
                        let text = egui::RichText::new(format!("{mark}{}. {}", k + 1, e.label));
                        let text = if on { text } else { text.strikethrough().color(t.weak) };
                        if ui
                            .checkbox(&mut on, text)
                            .on_hover_text(
                                "Una edición posterior que elige por clase ve los puntos como están ahora.",
                            )
                            .changed()
                        {
                            toggle = Some(k);
                        }
                    }
                }
                if let Some(k) = toggle {
                    self.queue(Cmd::Toggle(k));
                }
            });
    }

    /// El filtro de atributos por el que pasa toda selección.
    fn attr_filter(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        crate::collapsing("Filtro de atributos de las selecciones")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Solo se seleccionan puntos que cumplan estas condiciones.",
                    )
                    .size(11.0)
                    .color(t.weak),
                );
                ui.horizontal_wrapped(|ui| {
                    for c in 0..=255u8 {
                        if self.class_counts[usize::from(c)] == 0 {
                            continue;
                        }
                        let mut on = self.ed.attr.classes.contains(c);
                        if ui
                            .checkbox(&mut on, format!("{c}"))
                            .on_hover_text(self.ctable.name(c))
                            .changed()
                        {
                            self.ed.attr.classes.toggle(c);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Z de");
                    ui.add(egui::TextEdit::singleline(&mut self.ed.z_lo).desired_width(55.0));
                    ui.label("a");
                    ui.add(egui::TextEdit::singleline(&mut self.ed.z_hi).desired_width(55.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Intensidad de");
                    ui.add(egui::TextEdit::singleline(&mut self.ed.i_lo).desired_width(45.0));
                    ui.label("a");
                    ui.add(egui::TextEdit::singleline(&mut self.ed.i_hi).desired_width(45.0));
                });
                ui.horizontal_wrapped(|ui| {
                    for (k, n) in ["Primeros", "Intermedios", "Últimos", "Únicos"]
                        .iter()
                        .enumerate()
                    {
                        let mut on = self.ed.attr.returns & (1 << k) != 0;
                        if ui.checkbox(&mut on, *n).changed() {
                            self.ed.attr.returns ^= 1 << k;
                        }
                    }
                });
                let (zl, zh, il, ih) = (
                    parse(&self.ed.z_lo),
                    parse(&self.ed.z_hi),
                    parse(&self.ed.i_lo),
                    parse(&self.ed.i_hi),
                );
                self.ed.attr.z = (zl.is_some() || zh.is_some())
                    .then(|| (zl.unwrap_or(f64::NEG_INFINITY), zh.unwrap_or(f64::INFINITY)));
                self.ed.attr.intensity = (il.is_some() || ih.is_some()).then(|| {
                    (
                        il.unwrap_or(0.0).max(0.0) as u16,
                        ih.unwrap_or(65535.0).min(65535.0) as u16,
                    )
                });
                ui.horizontal(|ui| {
                    if ui
                        .button("Seleccionar por atributos")
                        .on_hover_text(
                            "Selecciona los puntos que cumplen las condiciones de arriba.",
                        )
                        .clicked()
                    {
                        let op = SelOp {
                            mode: self.ed.mode,
                            region: Region::All,
                            attr: self.ed.attr.clone(),
                        };
                        self.queue(Cmd::AddOp(Box::new(op)));
                    }
                    if ui
                        .button("Restablecer")
                        .on_hover_text("Borra las condiciones del filtro.")
                        .clicked()
                    {
                        self.ed.attr = Attr::default();
                        self.ed.z_lo.clear();
                        self.ed.z_hi.clear();
                        self.ed.i_lo.clear();
                        self.ed.i_hi.clear();
                    }
                });
            });
    }
}

/// La ventana a toda densidad de un foco: la caja de los puntos que elige con un margen (la muestra
/// de la pantalla no llega hasta el borde de la selección) y cuántos son.
fn focus_window(s: &PointStore, picked: &[bool]) -> Option<([f64; 4], usize)> {
    let (mut lo, mut hi, mut n) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2], 0usize);
    for (i, on) in picked.iter().enumerate() {
        if !*on {
            continue;
        }
        let p = s.xyz(i);
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let (dx, dy) = (hi[0] - lo[0], hi[1] - lo[1]);
    let cell = (dx.max(1e-6) * dy.max(1e-6) / n as f64).sqrt();
    let scene = (s.bounds[3] - s.bounds[0]).max(s.bounds[4] - s.bounds[1]);
    let pad = (0.1 * dx.max(dy)).max(3.0 * cell).max(1e-3 * scene);
    Some(([lo[0] - pad, lo[1] - pad, hi[0] + pad, hi[1] + pad], n))
}

/// Los atributos de una selección acotados a lo que los filtros de la vista (clases y retornos)
/// dejan ver (clases, retornos y rango de cotas): lo oculto no se selecciona ni se edita.
/// ponytail: el filtro por archivo no se aplica, la selección no conoce el archivo de cada punto.
fn visible_attr(style: &Style, mut a: Attr) -> Attr {
    a.classes = a.classes.intersection(&style.classes);
    a.returns &= style.returns;
    if let Some([lo, hi]) = style.zlim {
        a.z = Some(a.z.map_or((lo, hi), |(l, h)| (l.max(lo), h.min(hi))));
    }
    a
}
