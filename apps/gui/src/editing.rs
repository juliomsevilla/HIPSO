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
    /// Las selecciones en 3D alcanzan solo tantos metros a cada lado del plano del objetivo.
    pub slab: bool,
    pub slab_half: f64,
    pub sel_rev: u64,
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
            slab: false,
            slab_half: 1.0,
            sel_rev: 0,
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
        }
    }
}

fn parse(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse().ok()
}

pub fn is_select(n: Nav) -> bool {
    is_select_3d(n) || is_brush(n) || is_line(n)
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
    matches!(n, Nav::SelAbove | Nav::SelBelow)
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
            let above = self.nav == Nav::SelAbove;
            if resp.drag_stopped_by(egui::PointerButton::Primary)
                && let (Some(a), Some(b)) = (self.ed.pstart, resp.interact_pointer_pos())
                && (a.x - b.x).abs() >= 4.0
            {
                let op = SelOp {
                    mode: self.mode_now(ctx),
                    region: Region::Side {
                        bx,
                        a: to(a),
                        b: to(b),
                        above,
                        height: self.ed.side_height.max(0.0),
                    },
                    attr: self.ed.attr.clone(),
                };
                self.queue(Cmd::AddOp(Box::new(op)));
            }
            if let (true, Some(a), Some(b)) = (
                resp.dragged_by(egui::PointerButton::Primary),
                self.ed.pstart,
                resp.interact_pointer_pos(),
            ) {
                painter.line_segment([a, b], Stroke::new(2.0, color));
                // el lado que se toma, sombreado a lo largo de la línea (con la altura elegida,
                // la franja a escala)
                let hgt = self.ed.side_height.max(0.0);
                if hgt > 0.0 {
                    let (pa, pb) = (to(a), to(b));
                    let dz = if above { hgt } else { -hgt };
                    painter.add(egui::Shape::convex_polygon(
                        vec![a, b, back(pb[0], pb[1] + dz), back(pa[0], pa[1] + dz)],
                        color.gamma_multiply(0.18),
                        Stroke::NONE,
                    ));
                } else {
                    let off = if above { -14.0 } else { 14.0 };
                    painter.line_segment(
                        [a + Vec2::new(0.0, off), b + Vec2::new(0.0, off)],
                        Stroke::new(6.0, color.gamma_multiply(0.25)),
                    );
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
        (!pairs.is_empty()).then(|| (paths, pairs, self.ed.sel_count, self.ed.ops.clone()))
    }

    /// Escribe los puntos seleccionados en archivos `…_sel` y muestra solo esos, a la densidad del
    /// archivo: un árbol recortado de un escaneo se puede editar punto a punto en lugar de con el
    /// adelgazamiento de toda la escena.
    pub fn isolate_selection(&mut self, ctx: &egui::Context) {
        if self
            .ed
            .session
            .as_ref()
            .is_some_and(|s| s.edits.len() > self.ed.saved || self.ed.toggled)
        {
            self.log
                .push("Guarda o descarta los cambios antes de aislar la selección.".into());
            return;
        }
        let Some((outputs, pairs, _, ops)) = self.selection_inputs() else {
            self.log
                .push("No hay puntos seleccionados que aislar.".into());
            return;
        };
        self.isolate = true;
        let launch = Launch {
            outputs,
            label: "Aislar selección".into(),
            task: Box::new(move || {
                let mut n = 0;
                for (i, o) in &pairs {
                    hipso_core::check_cancel()?;
                    n += edit::extract_selected(i, &ops, o)?;
                }
                Ok(format!("{n} puntos aislados"))
            }),
            show: vec![],
        };
        self.start(ctx, launch);
    }

    pub fn edit_status(&self) -> Option<String> {
        let s = self.store.as_ref()?;
        let ses = self.ed.session.as_ref()?;
        let mut parts = Vec::new();
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
        for c in cmds {
            match c {
                Cmd::AddOp(op) => {
                    if op.mode == Mode::New {
                        self.ed.ops.clear();
                    }
                    self.ed.ops.push(*op);
                }
                Cmd::Clear => self.ed.ops.clear(),
                Cmd::All => {
                    self.ed.ops = vec![SelOp {
                        mode: Mode::New,
                        region: Region::All,
                        attr: self.ed.attr.clone(),
                    }];
                }
                Cmd::Invert => self.ed.ops.push(SelOp {
                    mode: Mode::Invert,
                    region: Region::All,
                    attr: Attr::default(),
                }),
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
                    let n = edit::selection(store, &self.ed.ops)
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
                    let e = Edit::new(std::mem::take(&mut self.ed.ops), action, label.clone());
                    if let Some(ses) = &mut self.ed.session {
                        ses.apply(store, e);
                    }
                    self.log.push(format!("Edición: {label} (en pantalla)"));
                    edited = true;
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
            edit::selection(store, &self.ed.ops)
        };
        self.ed.sel_count = store.selected.iter().filter(|b| **b).count();
        if edited {
            self.class_counts = [0; 256];
            for (i, c) in store.class.iter().enumerate() {
                if !store.deleted.get(i).copied().unwrap_or(false) {
                    self.class_counts[usize::from(*c)] += 1;
                }
            }
            self.rev += 1;
            self.tin = None;
        }
        self.ed.sel_rev += 1;
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
        let launch = Launch {
            outputs,
            label: "Guardar nube editada".into(),
            task: Box::new(move || {
                let mut lines = Vec::new();
                for (i, o) in &pairs {
                    let r = edit::save(i, o, &edits)?;
                    lines.push(edit::report_text(&r, o));
                }
                Ok(lines.join("\n"))
            }),
            show: vec![],
        };
        self.start(ctx, launch);
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
                            ] {
                                if ribbon::tool_button(ui, &t, icon, name, self.nav == n).clicked()
                                {
                                    self.act(ctx, Action::Nav(n));
                                }
                            }
                        });
                        if is_brush(self.nav) {
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
                            ui.label(
                                egui::RichText::new(
                                    "En el perfil, arrastra para trazar una línea: se selecciona todo lo que queda por encima (o por debajo) a lo largo de ella, dentro de la caja del perfil.",
                                )
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
                        if ribbon::small_button(ui, &t, "@selrect", "Aislar selección", false)
                            .clicked()
                        {
                            self.isolate_selection(ctx);
                        }
                        if ribbon::small_button(
                            ui,
                            &t,
                            "📤",
                            "Exportar puntos…",
                            self.tools.open == Some(ToolId::Export),
                        )
                        .clicked()
                        {
                            self.act(ctx, Action::Tool(ToolId::Export));
                        }
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
                                "@dups",
                                "Quitar duplicados",
                                self.tools.open == Some(ToolId::Dups),
                            )
                            .clicked()
                            {
                                self.tools.open = Some(ToolId::Dups);
                            }
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
