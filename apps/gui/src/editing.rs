//! Point editing in the viewer, with the tools of a vector editor: select points with a
//! rectangle, polygon, lasso or circle (in the plan or in a profile), combine selections
//! (new, add with Shift, subtract with Alt, intersect with Ctrl, invert), restrict them by
//! attributes, delete them or change their class, undo and redo, and save a new file. The edits
//! are rules the engine applies to every point of the file when saving.

use super::*;
use rustlidar_core::edit::{self, Attr, Edit, Mode, Region, SelOp, Session};
use rustlidar_core::vector::Polygon;

pub enum Cmd {
    AddOp(Box<SelOp>),
    Clear,
    All,
    Invert,
    Delete,
    Reclass(u8),
    Undo,
    Redo,
    Discard,
}

pub struct Editing {
    pub session: Option<Session>,
    pub ops: Vec<SelOp>,
    pub mode: Mode,
    /// Polygon vertices or lasso trail in plan coordinates, and in profile ones (distance, z).
    pub draft: Vec<[f64; 2]>,
    pub pdraft: Vec<[f64; 2]>,
    pub pstart: Option<Pos2>,
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
    /// Edits already written to a file.
    pub saved: usize,
    pub want_save: bool,
    /// Save without asking where (command-line `--save-edited`).
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
            want_save: false,
            auto_save: false,
        }
    }
}

fn parse(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse().ok()
}

pub fn is_select(n: Nav) -> bool {
    matches!(
        n,
        Nav::SelRect | Nav::SelPoly | Nav::SelLasso | Nav::SelCircle
    )
}

/// The shape a selection tool just finished, from the pointer's events. `to` turns a screen
/// position into the coordinates of the view; `px` is how many pixels make one unit of them.
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
                // a click picks what lies within a few pixels
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

    /// Selection tools in the plan window.
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

    /// Draws the selection in progress over the plan.
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
        paint_shape(
            painter,
            self.nav,
            resp,
            self.drag_start,
            &trail,
            Color32::from_rgb(0, 255, 255),
        );
    }

    /// Selection tools in the profile window: the shapes live in (distance, elevation).
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

    /// When points are selected: the files that hold them as `…_sel` paths, the (cloud, `_sel` file)
    /// pairs to write, the number of points and the selection itself.
    #[allow(clippy::type_complexity)]
    pub fn selection_inputs(
        &self,
    ) -> Option<(Vec<PathBuf>, Vec<(PathBuf, PathBuf)>, usize, Vec<SelOp>)> {
        let s = self.store.as_ref()?;
        if self.ed.ops.is_empty() || self.ed.sel_count == 0 || s.selected.is_empty() {
            return None;
        }
        // file k of the store is the k-th LAS or LAZ cloud
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
            let out = rustlidar_core::paths::results_dir(c)
                .join(format!("{}_sel.{ext}", rustlidar_core::paths::stem(c)));
            paths.push(out.clone());
            pairs.push(((*c).clone(), out));
        }
        (!pairs.is_empty()).then(|| (paths, pairs, self.ed.sel_count, self.ed.ops.clone()))
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
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Applies the queued edit commands. Runs between frames, when nothing else holds the store.
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
                    let e = Edit {
                        sel: std::mem::take(&mut self.ed.ops),
                        action,
                        label: label.clone(),
                    };
                    if let Some(ses) = &mut self.ed.session {
                        ses.apply(store, e);
                    }
                    self.log.push(format!("Edición: {label} (en pantalla)"));
                    edited = true;
                }
                Cmd::Undo | Cmd::Redo | Cmd::Discard => {
                    self.ed.ops.clear();
                    if let Some(ses) = &mut self.ed.session {
                        match c {
                            Cmd::Undo => edited |= ses.undo(store),
                            Cmd::Redo => edited |= ses.redo(store),
                            _ => {
                                ses.discard(store);
                                self.ed.saved = 0;
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

    /// Writes the edited clouds: every point of each file goes through the edit rules.
    pub fn save_edited(&mut self, ctx: &egui::Context) {
        let Some(ses) = &self.ed.session else { return };
        if ses.edits.is_empty() {
            self.log.push("No hay cambios que guardar.".into());
            return;
        }
        let edits = ses.edits.clone();
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
            let auto =
                rustlidar_core::paths::unique(&rustlidar_core::paths::product(i, "editada", &ext));
            let out = if inputs.len() == 1 && !self.ed.auto_save {
                match rfd::FileDialog::new()
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
        self.ed.saved = edits.len();
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
        // the vector tools have their own keys
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

    // ---------------------------------------------------------------------------- ribbon

    pub fn edit_ribbon(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let has = self.store.is_some();
        ribbon::group(ui, t, "Navegación", |ui| {
            for (icon, name, n) in [("🔍", "Zoom", Nav::Zoom), ("✋", "Desplazar", Nav::Pan)] {
                if ribbon::big_button(ui, t, icon, name, self.nav == n).clicked() {
                    self.nav = n;
                    self.ed.draft.clear();
                    self.ed.pdraft.clear();
                }
            }
            ui.vertical(|ui| {
                if ribbon::small_button(ui, t, "@full", "Extensión completa", false).clicked() {
                    self.prev_plans.push(self.plan);
                    self.plan_fit = true;
                }
                if ribbon::small_button(ui, t, "@prev", "Zoom anterior", false).clicked()
                    && let Some(p) = self.prev_plans.pop()
                {
                    self.plan = p;
                }
            });
        });
        ribbon::group(ui, t, "Selección", |ui| {
            for (icon, name, n) in [
                ("@selrect", "Rectángulo", Nav::SelRect),
                ("@aoi", "Polígono", Nav::SelPoly),
                ("@lasso", "Lazo", Nav::SelLasso),
                ("@circle", "Círculo", Nav::SelCircle),
            ] {
                if ribbon::big_button(ui, t, icon, name, self.nav == n).clicked() {
                    self.nav = n;
                    self.ed.draft.clear();
                    self.ed.pdraft.clear();
                }
            }
            ui.vertical(|ui| {
                if ribbon::small_button(ui, t, "@selall", "Todo (Ctrl+A)", false).clicked() && has {
                    self.queue(Cmd::All);
                }
                if ribbon::small_button(ui, t, "@invert", "Invertir (Ctrl+I)", false).clicked()
                    && has
                {
                    self.queue(Cmd::Invert);
                }
                if ribbon::small_button(ui, t, "@clear", "Quitar (Esc)", false).clicked() {
                    self.queue(Cmd::Clear);
                }
            });
        });
        ribbon::group(ui, t, "Combinar con la selección", |ui| {
            ui.vertical(|ui| {
                for (m, n) in [
                    (Mode::New, "Nueva"),
                    (Mode::Add, "Añadir (Mayús)"),
                    (Mode::Subtract, "Restar (Alt)"),
                    (Mode::Intersect, "Intersecar (Ctrl)"),
                ] {
                    ui.radio_value(&mut self.ed.mode, m, n);
                }
            });
        });
        ribbon::group(ui, t, "Estadística", |ui| {
            if ribbon::big_button(ui, t, "@hist", "Histogramas", self.ed.hist.open).clicked() && has
            {
                self.ed.hist.open = !self.ed.hist.open;
            }
        });
        ribbon::group(ui, t, "Editar", |ui| {
            if ribbon::big_button(ui, t, "@trash", "Eliminar\n(Supr)", false).clicked() {
                self.queue(Cmd::Delete);
            }
            if ribbon::big_button(ui, t, "@reclass", "Cambiar\nclase", false).clicked() {
                self.queue(Cmd::Reclass(self.ed.class));
            }
            ui.vertical(|ui| {
                self.class_combo(ui);
                if ribbon::small_button(ui, t, "@prev", "Deshacer (Ctrl+Z)", false).clicked() {
                    self.queue(Cmd::Undo);
                }
                if ribbon::small_button(ui, t, "@redo", "Rehacer (Ctrl+Y)", false).clicked() {
                    self.queue(Cmd::Redo);
                }
            });
        });
        ribbon::group(ui, t, "Guardar", |ui| {
            let dirty = self
                .ed
                .session
                .as_ref()
                .is_some_and(|s| s.edits.len() > self.ed.saved);
            if ribbon::big_button(ui, t, "@save", "Guardar nube\neditada", dirty).clicked() {
                self.ed.want_save = true;
            }
            ui.vertical(|ui| {
                if ribbon::small_button(ui, t, "@clear", "Descartar cambios", false).clicked() {
                    self.queue(Cmd::Discard);
                }
                ui.label(
                    egui::RichText::new(self.edit_status().unwrap_or_else(|| "sin cambios".into()))
                        .size(11.0)
                        .color(t.weak),
                );
            });
        });
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
            });
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

    // ---------------------------------------------------------------------------- side tab

    pub fn edit_side(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        if self.store.is_none() {
            ui.label("Abre una nube para editar sus puntos.");
            return;
        }
        ui.label(egui::RichText::new("Elige una herramienta de selección en la pestaña Edición y dibuja sobre la planta o el perfil. Las ediciones se aplican a todos los puntos del archivo al guardar, no solo a los que se ven.").size(11.0).color(t.weak));
        ui.separator();
        ui.label(
            egui::RichText::new(format!(
                "Selección: {} puntos en pantalla (≈ {} en el archivo)",
                self.ed.sel_count,
                self.ed.sel_count as u64 * self.store.as_ref().map_or(1, |s| s.stride)
            ))
            .strong(),
        );
        ui.horizontal(|ui| {
            if ui.button("Todo").clicked() {
                self.queue(Cmd::All);
            }
            if ui.button("Invertir").clicked() {
                self.queue(Cmd::Invert);
            }
            if ui.button("Quitar").clicked() {
                self.queue(Cmd::Clear);
            }
        });
        egui::CollapsingHeader::new("Filtro de atributos de las selecciones")
            .default_open(true)
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
                ui.horizontal(|ui| {
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
                    if ui.button("Seleccionar por atributos").clicked() {
                        let op = SelOp {
                            mode: self.ed.mode,
                            region: Region::All,
                            attr: self.ed.attr.clone(),
                        };
                        self.queue(Cmd::AddOp(Box::new(op)));
                    }
                    if ui.button("Restablecer").clicked() {
                        self.ed.attr = Attr::default();
                        self.ed.z_lo.clear();
                        self.ed.z_hi.clear();
                        self.ed.i_lo.clear();
                        self.ed.i_hi.clear();
                    }
                });
            });
        egui::CollapsingHeader::new("Qué hacer con la selección")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Eliminar").clicked() {
                        self.queue(Cmd::Delete);
                    }
                    if ui.button("Asignar clase").clicked() {
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
                        if ui.small_button(n).clicked() {
                            self.ed.class = c;
                            self.queue(Cmd::Reclass(c));
                        }
                    }
                });
            });
        ui.separator();
        let (n_edits, n_saved) = (
            self.ed.session.as_ref().map_or(0, |s| s.edits.len()),
            self.ed.saved,
        );
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("Historial ({n_edits})")).strong());
            if ui.button("Deshacer").clicked() {
                self.queue(Cmd::Undo);
            }
            if ui.button("Rehacer").clicked() {
                self.queue(Cmd::Redo);
            }
        });
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                if let Some(ses) = &self.ed.session {
                    for (k, e) in ses.edits.iter().enumerate() {
                        let mark = if k < n_saved { "✔" } else { "•" };
                        ui.label(format!("{mark} {}. {}", k + 1, e.label));
                    }
                }
            });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    n_edits > n_saved,
                    egui::Button::new("Guardar nube editada…"),
                )
                .clicked()
            {
                self.ed.want_save = true;
            }
            if ui
                .add_enabled(n_edits > 0, egui::Button::new("Descartar todo"))
                .clicked()
            {
                self.queue(Cmd::Discard);
            }
        });
    }
}
