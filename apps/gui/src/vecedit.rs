//! Editing vector layers on the map and in the attribute table: digitising with snapping,
//! selecting, moving vertices and features, inserting and deleting vertices, undo and redo, and
//! selecting by attribute.

use super::*;
use crate::layers::{VecLayer, dist_to_segment};
use std::collections::HashMap;

/// A comparison for selecting by attribute.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
}

impl Op {
    pub const ALL: [Op; 7] = [Op::Eq, Op::Ne, Op::Lt, Op::Le, Op::Gt, Op::Ge, Op::Contains];

    pub fn name(self) -> &'static str {
        match self {
            Op::Eq => "=",
            Op::Ne => "≠",
            Op::Lt => "<",
            Op::Le => "≤",
            Op::Gt => ">",
            Op::Ge => "≥",
            Op::Contains => "contiene",
        }
    }

    fn test(self, text: &str, num: f64, value: &str) -> bool {
        let want = value.trim();
        if let (Ok(w), true) = (want.replace(',', ".").parse::<f64>(), num.is_finite()) {
            return match self {
                Op::Eq => num == w,
                Op::Ne => num != w,
                Op::Lt => num < w,
                Op::Le => num <= w,
                Op::Gt => num > w,
                Op::Ge => num >= w,
                Op::Contains => text.contains(want),
            };
        }
        let (t, w) = (text.to_lowercase(), want.to_lowercase());
        match self {
            Op::Eq => t == w,
            Op::Ne => t != w,
            Op::Lt => t < w,
            Op::Le => t <= w,
            Op::Gt => t > w,
            Op::Ge => t >= w,
            Op::Contains => t.contains(&w),
        }
    }
}

/// A vertex: feature, part, index in the part.
type Vertex = (usize, usize, usize);

pub struct VecEditState {
    /// Digitising and moving vertices snap to the vertices of the visible layers.
    pub snap: bool,
    drag: Option<Vertex>,
    moving: Option<[f64; 2]>,
    pub rect_start: Option<Pos2>,
    pub hover: Option<Vertex>,
    pub q_field: usize,
    pub q_op: Op,
    pub q_value: String,
    pub only_selected: bool,
    last_row: Option<usize>,
    /// The cell being typed in: feature, field and its text on entering.
    cell_old: Option<(usize, usize, String)>,
    /// The cell being edited: feature and field.
    editing: Option<(usize, usize)>,
    /// Width of every column of the table of each layer (index 0 is the row number).
    col_w: HashMap<usize, Vec<f32>>,
    /// Field the table is sorted by and whether it goes up.
    sort: Option<(usize, bool)>,
    /// The order of the rows for `sort_key` (layer, sort, number of rows).
    order: Vec<usize>,
    sort_key: Option<(usize, Option<(usize, bool)>, usize)>,
}

impl Default for VecEditState {
    fn default() -> Self {
        Self {
            snap: true,
            drag: None,
            moving: None,
            rect_start: None,
            hover: None,
            q_field: 0,
            q_op: Op::Eq,
            q_value: String::new(),
            only_selected: false,
            last_row: None,
            cell_old: None,
            editing: None,
            col_w: HashMap::new(),
            sort: None,
            order: Vec::new(),
            sort_key: None,
        }
    }
}

impl VecLayer {
    /// Moves the selected features.
    fn translate_selected(&mut self, dx: f64, dy: f64) {
        let sel: Vec<usize> = self.selected.iter().copied().collect();
        for i in sel {
            for q in self.data.features[i].parts.iter_mut().flatten() {
                q[0] += dx;
                q[1] += dy;
            }
            self.update_feature(i);
        }
    }

    /// Sets a vertex, keeping closed rings closed.
    fn set_vertex(&mut self, (f, p, v): Vertex, pos: [f64; 2]) {
        let part = &mut self.data.features[f].parts[p];
        let closed = part.len() > 3 && part.first() == part.last();
        let last = part.len() - 1;
        part[v] = pos;
        if closed && v == 0 {
            part[last] = pos;
        } else if closed && v == last {
            part[0] = pos;
        }
        self.update_feature(f);
    }

    /// Selects by an attribute; `mode` 0 replaces the selection, 1 adds to it, 2 narrows it.
    fn select_by(&mut self, field: usize, op: Op, value: &str, mode: u8) {
        let hits: HashSet<usize> = (0..self.data.features.len())
            .filter(|&i| {
                let text = self
                    .data
                    .text
                    .get(i)
                    .and_then(|r| r.get(field))
                    .map_or("", String::as_str);
                let num = self
                    .data
                    .num
                    .get(i)
                    .and_then(|r| r.get(field))
                    .copied()
                    .unwrap_or(f64::NAN);
                op.test(text, num, value)
            })
            .collect();
        match mode {
            0 => self.selected = hits,
            1 => self.selected.extend(hits),
            _ => self.selected.retain(|i| hits.contains(i)),
        }
    }
}

impl App {
    pub fn active_vec(&self) -> Option<&VecLayer> {
        match self.refs.get(self.active?).map(|r| &r.kind) {
            Some(RefKind::Vector(v)) => Some(v),
            _ => None,
        }
    }

    pub fn active_vec_mut(&mut self) -> Option<&mut VecLayer> {
        let k = self.active?;
        match self.refs.get_mut(k).map(|r| &mut r.kind) {
            Some(RefKind::Vector(v)) => Some(v),
            _ => None,
        }
    }

    fn tol(&self, px: f64) -> f64 {
        px / self.plan.scale.max(1e-12)
    }

    pub fn digitize_hint(&self) -> String {
        match self.active_vec().map(|v| v.kind) {
            None => "Digitalizar: crea una capa (Nueva capa) o elige una capa vectorial en el panel de capas".to_string(),
            Some(view::GeomKind::Point) => "Digitalizar puntos: clic = añadir un punto".to_string(),
            Some(k) => format!(
                "Digitalizar {}: clic = vértice ({} puestos) · botón derecho, doble clic o Intro = terminar · Retroceso = quitar el último · Esc = cancelar{}",
                if k == view::GeomKind::Line { "líneas" } else { "polígonos" },
                self.dig_draft.len(),
                if k == view::GeomKind::Polygon {
                    " · Adosar: empieza y termina dentro de polígonos (uno o varios) · Cortar: atraviesa uno o varios polígonos"
                } else {
                    ""
                }
            ),
        }
    }

    pub fn vec_tool_hint(&self) -> Option<String> {
        Some(match self.nav {
            Nav::Digitize => self.digitize_hint(),
            Nav::VecSelect => "Seleccionar: clic = un elemento · arrastrar = rectángulo · Mayús/Ctrl = añadir o quitar · Supr = eliminar".to_string(),
            Nav::VecVertex => "Vértices: arrastra un vértice · doble clic en un lado = nuevo vértice · botón derecho en un vértice = quitarlo".to_string(),
            Nav::VecMove => "Mover: arrastra los elementos seleccionados (o el que está bajo el cursor)".to_string(),
            _ => return None,
        })
    }

    /// The nearest vertex of the visible layers to a map point, within `tol` (`skip`: a layer and
    /// feature whose vertices do not count).
    fn snap_to(&self, m: (f64, f64), skip: Option<(usize, usize)>) -> [f64; 2] {
        let tol = self.tol(10.0);
        let (mut best, mut d) = ([m.0, m.1], tol);
        for (k, r) in self.refs.iter().enumerate().filter(|(_, r)| r.visible) {
            let RefKind::Vector(v) = &r.kind else {
                continue;
            };
            for i in v.in_window([m.0 - tol, m.1 - tol, m.0 + tol, m.1 + tol]) {
                if skip == Some((k, i)) {
                    continue;
                }
                for q in v.data.features[i].parts.iter().flatten() {
                    let dd = (q[0] - m.0).hypot(q[1] - m.1);
                    if dd < d {
                        (d, best) = (dd, *q);
                    }
                }
            }
        }
        best
    }

    pub fn digitize_click(&mut self, m: (f64, f64)) {
        let p = if self.vedit.snap {
            self.snap_to(m, None)
        } else {
            [m.0, m.1]
        };
        match self.active_vec().map(|v| v.kind) {
            None => {
                self.log("Crea una capa vectorial (Nueva capa) o elígela en el panel de capas.")
            }
            Some(view::GeomKind::Point) => self.add_feature(vec![vec![p]]),
            Some(_) => self.dig_draft.push(p),
        }
    }

    pub fn digitize_finish(&mut self) {
        let Some(kind) = self.active_vec().map(|v| v.kind) else {
            return;
        };
        let mut d = std::mem::take(&mut self.dig_draft);
        // a double click adds its vertex twice
        if d.len() >= 2 && d.last() == d.get(d.len() - 2) {
            d.pop();
        }
        if kind == view::GeomKind::Polygon && self.polygon_edit(&d) {
            return;
        }
        match kind {
            view::GeomKind::Line if d.len() >= 2 => self.add_feature(vec![d]),
            view::GeomKind::Polygon if d.len() >= 3 => {
                d.push(d[0]);
                self.add_feature(vec![d]);
            }
            _ => {}
        }
    }

    /// A line drawn on a polygon layer can edit the polygons instead of making a new one: starting and
    /// ending inside polygons (the same or different ones), it adds the ground it covers outside
    /// them as a new polygon next to all of them; any other line cuts every polygon it splits, one
    /// or many. Returns whether it did either.
    fn polygon_edit(&mut self, d: &[[f64; 2]]) -> bool {
        use hipso_core::geomops;
        let (Some(first), Some(last)) = (d.first().copied(), d.last().copied()) else {
            return false;
        };
        let Some(v) = self.active_vec() else {
            return false;
        };
        let at = |p: [f64; 2]| v.feature_at(p[0], p[1], 0.0);
        let (fi, li) = (at(first), at(last));
        let b = d.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
            [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
        });
        let near = v.in_window(b);
        if d.len() >= 3 && fi.is_some() && li.is_some() {
            let others: Vec<&[Vec<[f64; 2]>]> = near.iter().map(|&i| v.data.features[i].parts.as_slice()).collect();
            let parts = geomops::attach(d, &others);
            if !parts.is_empty() {
                let many = fi != li;
                self.add_feature(parts);
                self.log(if many {
                    "Polígono nuevo adosado a los existentes."
                } else {
                    "Polígono nuevo adosado al existente."
                });
                return true;
            }
        }
        // any other line cuts every polygon it splits, whatever the polygons its ends are in
        if d.len() >= 2 {
            let cuts: Vec<(usize, Vec<Vec<Vec<[f64; 2]>>>)> = near
                .iter()
                .filter_map(|&i| {
                    let pieces = geomops::cut_by_line(&v.data.features[i].parts, d);
                    (pieces.len() >= 2).then_some((i, pieces))
                })
                .collect();
            if cuts.is_empty() {
                return false;
            }
            let Some(v) = self.active_vec_mut() else {
                return false;
            };
            v.push_undo();
            let mut made = 0;
            for (i, pieces) in cuts {
                let mut it = pieces.into_iter();
                if let Some(first) = it.next() {
                    v.data.features[i].parts = first;
                    v.update_feature(i);
                }
                for parts in it {
                    let k = v.push_feature(parts);
                    v.data.text[k] = v.data.text[i].clone();
                    v.data.num[k] = v.data.num[i].clone();
                    made += 1;
                }
            }
            v.selected.clear();
            v.reclassify();
            self.log(format!("Polígono cortado: {made} parte(s) nueva(s)."));
            return true;
        }
        false
    }

    fn add_feature(&mut self, parts: Vec<Vec<[f64; 2]>>) {
        let Some(v) = self.active_vec_mut() else {
            return;
        };
        v.push_undo();
        let i = v.push_feature(parts);
        v.selected.clear();
        v.selected.insert(i);
    }

    /// Switches an editing tool on or off. Switching one off ends the editing session of the
    /// layer: if it changed, the user is asked what to do with the changes.
    pub fn toggle_edit_tool(&mut self, n: Nav) {
        self.dig_draft.clear();
        if self.nav == n {
            self.end_edit_session();
            return;
        }
        if n != Nav::VecSelect
            && let Some(v) = self.active_vec_mut()
            && v.edit_base.is_none()
        {
            v.edit_base = Some((v.data.clone(), v.dirty));
            v.edit_touched = false;
        }
        self.nav = n;
    }

    /// Leaves the editing tools; asks about the changes if there are any.
    pub fn end_edit_session(&mut self) {
        let was = self.nav;
        self.nav = Nav::Zoom;
        self.dig_draft.clear();
        let Some(k) = self.active else { return };
        let touched = self.active_vec().is_some_and(|v| v.edit_touched);
        if matches!(was, Nav::Digitize | Nav::VecVertex | Nav::VecMove) && touched {
            self.edit_confirm = Some((k, was));
        } else if let Some(v) = self.active_vec_mut() {
            v.edit_base = None;
        }
    }

    /// The question asked when an editing tool is switched off after changes.
    pub fn edit_confirm_window(&mut self, ctx: &egui::Context) {
        let Some((k, tool)) = self.edit_confirm else { return };
        let Some(r) = self.refs.get(k) else {
            self.edit_confirm = None;
            return;
        };
        let name = r.name.clone();
        let (mut save, mut save_new, mut discard, mut keep, mut back) = (false, false, false, false, false);
        egui::Window::new("Cambios en la capa")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Has modificado la capa «{name}» en esta sesión de edición."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    save = ui.button("Guardar").on_hover_text("Sobre la capa que has editado (en un GeoPackage, sobre su tabla)").clicked();
                    save_new = ui.button("Guardar como capa nueva…").clicked();
                    discard = ui.button("Descartar los cambios").clicked();
                    keep = ui.button("Dejar sin guardar").on_hover_text("Los cambios siguen en la capa, marcada con *").clicked();
                    back = ui.button("Seguir editando").clicked();
                });
            });
        if back {
            self.nav = tool;
            self.edit_confirm = None;
        } else if save || save_new {
            self.save_layer(k, save_new);
            if let Some(RefKind::Vector(v)) = self.refs.get_mut(k).map(|r| &mut r.kind) {
                v.edit_base = None;
            }
            self.edit_confirm = None;
        } else if discard {
            if let Some(RefKind::Vector(v)) = self.refs.get_mut(k).map(|r| &mut r.kind)
                && let Some((data, dirty)) = v.edit_base.take()
            {
                v.data = data;
                v.undo.clear();
                v.redo.clear();
                v.selected.clear();
                v.rebuild();
                v.reclassify();
                v.dirty = dirty;
            }
            self.edit_confirm = None;
        } else if keep {
            if let Some(RefKind::Vector(v)) = self.refs.get_mut(k).map(|r| &mut r.kind) {
                v.edit_base = None;
            }
            self.edit_confirm = None;
        }
    }

    pub fn clear_selection(&mut self) {
        for r in &mut self.refs {
            if let RefKind::Vector(v) = &mut r.kind {
                v.selected.clear();
            }
        }
    }

    /// Selects the topmost feature at a map point.
    fn select_at(&mut self, m: (f64, f64), additive: bool) {
        let tol = self.tol(6.0);
        let hit = (0..self.refs.len()).rev().find_map(|k| {
            let r = &self.refs[k];
            match &r.kind {
                RefKind::Vector(v) if r.visible => v.feature_at(m.0, m.1, tol).map(|i| (k, i)),
                _ => None,
            }
        });
        if !additive {
            self.clear_selection();
        }
        if let Some((k, i)) = hit {
            self.active = Some(k);
            self.sel_layer = Some(k);
            if let RefKind::Vector(v) = &mut self.refs[k].kind
                && !(additive && v.selected.remove(&i))
            {
                v.selected.insert(i);
            }
        }
    }

    fn select_rect(&mut self, a: [f64; 2], b: [f64; 2], additive: bool) {
        let (lo, hi) = (
            [a[0].min(b[0]), a[1].min(b[1])],
            [a[0].max(b[0]), a[1].max(b[1])],
        );
        if !additive {
            self.clear_selection();
        }
        for r in self.refs.iter_mut().filter(|r| r.visible) {
            if let RefKind::Vector(v) = &mut r.kind {
                let hits: Vec<usize> = (0..v.geom.bbox.len())
                    .filter(|&i| {
                        let g = &v.geom.bbox[i];
                        g[2] >= lo[0] && g[0] <= hi[0] && g[3] >= lo[1] && g[1] <= hi[1]
                    })
                    .collect();
                v.selected.extend(hits);
            }
        }
    }

    /// The vertex of the active layer nearest to a map point, within `tol`.
    fn vertex_near(&self, m: (f64, f64), tol: f64) -> Option<Vertex> {
        let v = self.active_vec()?;
        let mut best: Option<(f64, Vertex)> = None;
        for (i, b) in v.geom.bbox.iter().enumerate() {
            if m.0 < b[0] - tol || m.0 > b[2] + tol || m.1 < b[1] - tol || m.1 > b[3] + tol {
                continue;
            }
            for (p, part) in v.data.features[i].parts.iter().enumerate() {
                for (k, q) in part.iter().enumerate() {
                    let d = (q[0] - m.0).hypot(q[1] - m.1);
                    if d <= tol && best.is_none_or(|b| d < b.0) {
                        best = Some((d, (i, p, k)));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    fn insert_vertex_at(&mut self, m: (f64, f64)) {
        let tol = self.tol(8.0);
        let Some(v) = self.active_vec() else {
            return;
        };
        if v.kind == view::GeomKind::Point {
            return;
        }
        let mut best: Option<(f64, usize, usize, usize)> = None;
        for (i, b) in v.geom.bbox.iter().enumerate() {
            if m.0 < b[0] - tol || m.0 > b[2] + tol || m.1 < b[1] - tol || m.1 > b[3] + tol {
                continue;
            }
            for (p, part) in v.data.features[i].parts.iter().enumerate() {
                for (k, s) in part.windows(2).enumerate() {
                    let d = dist_to_segment([m.0, m.1], s[0], s[1]);
                    if d <= tol && best.is_none_or(|b| d < b.0) {
                        best = Some((d, i, p, k));
                    }
                }
            }
        }
        let Some((_, i, p, k)) = best else {
            return;
        };
        let Some(v) = self.active_vec_mut() else {
            return;
        };
        v.push_undo();
        let s = [
            v.data.features[i].parts[p][k],
            v.data.features[i].parts[p][k + 1],
        ];
        let (dx, dy) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
        let l2 = (dx * dx + dy * dy).max(1e-18);
        let t = (((m.0 - s[0][0]) * dx + (m.1 - s[0][1]) * dy) / l2).clamp(0.0, 1.0);
        v.data.features[i].parts[p].insert(k + 1, [s[0][0] + dx * t, s[0][1] + dy * t]);
        v.update_feature(i);
    }

    fn delete_vertex(&mut self, (i, p, k): Vertex) {
        let Some(v) = self.active_vec_mut() else {
            return;
        };
        let polygon = v.kind == view::GeomKind::Polygon;
        let n = v.data.features[i].parts[p].len();
        if v.kind == view::GeomKind::Point || n <= if polygon { 4 } else { 2 } {
            self.log(
                "No se puede quitar: quedarían muy pocos vértices. Elimina el elemento entero.",
            );
            return;
        }
        v.push_undo();
        let part = &mut v.data.features[i].parts[p];
        let closed = part.first() == part.last();
        part.remove(k);
        if closed && (k == 0 || k == n - 1) {
            // the ring starts and ends at the same vertex: keep it closed
            let first = part[0];
            if k == n - 1 {
                part.push(first);
            } else if let Some(last) = part.last_mut() {
                *last = first;
            }
        }
        v.update_feature(i);
    }

    pub fn delete_selected(&mut self) {
        let Some(v) = self.active_vec_mut() else {
            return;
        };
        if v.selected.is_empty() {
            return;
        }
        v.push_undo();
        let which = v.selected.clone();
        v.remove_features(&which);
    }

    pub fn vec_undo(&mut self, redo: bool) {
        self.dig_draft.clear();
        if let Some(v) = self.active_vec_mut() {
            let _ = if redo { v.redo() } else { v.undo() };
        }
    }

    /// The pointer work of the vector tools in the plan.
    pub fn vector_tool(&mut self, resp: &egui::Response, rect: Rect, w: usize, h: usize) {
        let to_map = |s: &Self, p: Pos2| {
            s.plan.to_map(
                f64::from(p.x - rect.left()),
                f64::from(p.y - rect.top()),
                w,
                h,
            )
        };
        let additive = resp.ctx.input(|i| i.modifiers.shift || i.modifiers.command);
        let pointer = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
        let left_down = resp.dragged_by(egui::PointerButton::Primary);
        // a drag is noticed after the pointer has moved: what it grabbed is under the press
        let press = resp.ctx.input(|i| i.pointer.press_origin());
        match self.nav {
            Nav::Digitize => {
                if resp.clicked()
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    let m = to_map(self, p);
                    self.digitize_click(m);
                }
                // right click or double click close a line or an area (Enter does too)
                if resp.secondary_clicked() || resp.double_clicked() {
                    self.digitize_finish();
                }
            }
            Nav::VecSelect => {
                if resp.drag_started_by(egui::PointerButton::Primary) {
                    self.vedit.rect_start = press.or_else(|| resp.interact_pointer_pos());
                }
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some(a), Some(b)) = (self.vedit.rect_start, resp.interact_pointer_pos())
                        && (a - b).length() > 5.0
                    {
                        let (ma, mb) = (to_map(self, a), to_map(self, b));
                        self.select_rect([ma.0, ma.1], [mb.0, mb.1], additive);
                    }
                    self.vedit.rect_start = None;
                } else if resp.clicked()
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    let m = to_map(self, p);
                    self.select_at(m, additive);
                }
            }
            Nav::VecVertex => {
                let tol = self.tol(8.0);
                let m = pointer.map(|p| to_map(self, p));
                if self.vedit.drag.is_none() {
                    self.vedit.hover = m.and_then(|m| self.vertex_near(m, tol));
                }
                if resp.drag_started_by(egui::PointerButton::Primary)
                    && let Some(vx) = press
                        .map(|p| to_map(self, p))
                        .and_then(|m| self.vertex_near(m, tol))
                {
                    if let Some(v) = self.active_vec_mut() {
                        v.push_undo();
                    }
                    self.vedit.drag = Some(vx);
                }
                if let (Some(vx), true, Some(m)) = (self.vedit.drag, left_down, m) {
                    let skip = self.active.map(|k| (k, vx.0));
                    let pos = if self.vedit.snap {
                        self.snap_to(m, skip)
                    } else {
                        [m.0, m.1]
                    };
                    if let Some(v) = self.active_vec_mut() {
                        v.set_vertex(vx, pos);
                    }
                }
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    self.vedit.drag = None;
                }
                if resp.double_clicked()
                    && let Some(m) = m
                {
                    self.insert_vertex_at(m);
                } else if resp.clicked()
                    && let Some(m) = m
                {
                    self.select_at(m, additive);
                }
                if resp.secondary_clicked()
                    && let Some(vx) = self.vedit.hover
                {
                    self.delete_vertex(vx);
                    self.vedit.hover = None;
                }
            }
            Nav::VecMove => {
                if resp.drag_started_by(egui::PointerButton::Primary)
                    && let Some(p) = press.or_else(|| resp.interact_pointer_pos())
                {
                    let m = to_map(self, p);
                    let tol = self.tol(6.0);
                    let hit = self.active_vec().and_then(|v| v.feature_at(m.0, m.1, tol));
                    if let Some(i) = hit {
                        if let Some(v) = self.active_vec_mut() {
                            if !v.selected.contains(&i) {
                                v.selected.clear();
                                v.selected.insert(i);
                            }
                            v.push_undo();
                        }
                        self.vedit.moving = Some([m.0, m.1]);
                    }
                }
                if let (Some(last), true, Some(p)) =
                    (self.vedit.moving, left_down, resp.interact_pointer_pos())
                {
                    let m = to_map(self, p);
                    let (dx, dy) = (m.0 - last[0], m.1 - last[1]);
                    if let Some(v) = self.active_vec_mut() {
                        v.translate_selected(dx, dy);
                    }
                    self.vedit.moving = Some([m.0, m.1]);
                }
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    self.vedit.moving = None;
                }
                if resp.clicked()
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    let m = to_map(self, p);
                    self.select_at(m, additive);
                }
            }
            _ => {}
        }
    }

    /// Keys of the vector tools: undo, redo, delete, finish and cancel.
    pub fn vector_keys(&mut self, ctx: &egui::Context) {
        if !self.edit_layer_on() {
            return;
        }
        let (undo, redo, del, enter, back) = ctx.input(|i| {
            let c = i.modifiers.command;
            (
                c && i.key_pressed(Key::Z) && !i.modifiers.shift,
                c && (i.key_pressed(Key::Y) || (i.key_pressed(Key::Z) && i.modifiers.shift)),
                i.key_pressed(Key::Delete),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Backspace),
            )
        });
        if undo {
            if self.dig_draft.pop().is_none() {
                self.vec_undo(false);
            }
        } else if redo {
            self.vec_undo(true);
        } else if del {
            self.delete_selected();
        } else if enter {
            self.digitize_finish();
        } else if back {
            self.dig_draft.pop();
        }
    }

    /// The feature being drawn, the selected features, the vertices being edited and the
    /// selection rectangle.
    pub fn paint_draft(
        &self,
        painter: &egui::Painter,
        pointer: Option<Pos2>,
        rect: Rect,
        w: usize,
        h: usize,
    ) {
        let to_s = |p: &[f64; 2]| {
            let s = self.plan.to_screen(p[0], p[1], w, h);
            Pos2::new(s.0 as f32, s.1 as f32) + rect.min.to_vec2()
        };
        let clip = painter.with_clip_rect(rect);
        let cyan = Color32::from_rgb(0, 230, 255);
        for r in self.refs.iter().filter(|r| r.visible) {
            let RefKind::Vector(v) = &r.kind else {
                continue;
            };
            for &i in v.selected.iter().filter(|i| **i < v.data.features.len()) {
                if v.kind == view::GeomKind::Polygon {
                    for t in v.tris(i) {
                        let mut mesh = egui::Mesh::default();
                        for p in &t.verts {
                            mesh.colored_vertex(to_s(p), cyan.gamma_multiply(0.18));
                        }
                        for tri in t.idx.as_chunks::<3>().0 {
                            mesh.add_triangle(tri[0], tri[1], tri[2]);
                        }
                        clip.add(egui::Shape::mesh(mesh));
                    }
                }
                for part in &v.data.features[i].parts {
                    if part.len() == 1 {
                        clip.circle_stroke(
                            to_s(&part[0]),
                            v.point_size + 3.0,
                            Stroke::new(2.5, cyan),
                        );
                    } else {
                        clip.add(egui::Shape::line(
                            part.iter().map(to_s).collect(),
                            Stroke::new(v.width + 2.0, cyan),
                        ));
                    }
                }
            }
        }
        // the vertices of the selected features of the layer being edited
        if matches!(self.nav, Nav::VecVertex | Nav::VecMove)
            && let Some(v) = self.active_vec()
        {
            let show: Vec<usize> = v.selected.iter().copied().take(200).collect();
            for i in show.into_iter().filter(|i| *i < v.data.features.len()) {
                for part in &v.data.features[i].parts {
                    for q in part {
                        let s = to_s(q);
                        clip.rect_filled(
                            Rect::from_center_size(s, Vec2::splat(7.0)),
                            1.0,
                            Color32::WHITE,
                        );
                        clip.rect_stroke(
                            Rect::from_center_size(s, Vec2::splat(7.0)),
                            1.0,
                            Stroke::new(1.0, Color32::BLACK),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }
            if let Some((i, p, k)) = self.vedit.hover
                && let Some(q) = v
                    .data
                    .features
                    .get(i)
                    .and_then(|f| f.parts.get(p))
                    .and_then(|pt| pt.get(k))
            {
                clip.circle(
                    to_s(q),
                    6.0,
                    Color32::from_rgb(255, 160, 0),
                    Stroke::new(1.5, Color32::BLACK),
                );
            }
        }
        let yellow = Color32::from_rgb(255, 220, 0);
        if !self.dig_draft.is_empty() {
            let mut pts: Vec<Pos2> = self.dig_draft.iter().map(to_s).collect();
            for p in &pts {
                clip.circle_filled(*p, 3.0, yellow);
            }
            if let Some(p) = pointer {
                pts.push(p);
            }
            clip.add(egui::Shape::line(
                pts,
                Stroke::new(self.line_weight, yellow),
            ));
        }
        if let (Some(a), Some(b)) = (self.vedit.rect_start, pointer)
            && self.nav == Nav::VecSelect
        {
            let r = Rect::from_two_pos(a, b);
            clip.rect_stroke(r, 0.0, Stroke::new(1.5, cyan), egui::StrokeKind::Inside);
            clip.rect_filled(r, 0.0, cyan.gamma_multiply(0.12));
        }
    }

    // ------------------------------------------------------------------------ the table

    pub fn table_window(&mut self, ctx: &egui::Context) {
        let Some(k) = self.table_layer else { return };
        if !matches!(self.refs.get(k).map(|r| &r.kind), Some(RefKind::Vector(_))) {
            self.table_layer = None;
            return;
        }
        let t = self.tokens();
        let mut ed = std::mem::take(&mut self.vedit);
        let mut new_field = std::mem::take(&mut self.table_new_field);
        let name = self.refs[k].name.clone();
        let (mut open, mut save, mut zoom, mut delete) = (true, false, false, false);
        let mut undo = 0u8;
        let RefKind::Vector(v) = &mut self.refs[k].kind else {
            return;
        };
        const ROW_H: f32 = 22.0;
        const HEAD_H: f32 = 28.0;
        const NUM_W: f32 = 56.0;
        egui::Window::new(format!("Tabla de atributos · {name}"))
            .default_pos([330.0, 190.0])
            .open(&mut open)
            .default_size([560.0, 460.0])
            .max_height(((v.data.features.len() as f32) * ROW_H + HEAD_H + 110.0).min((ctx.content_rect().height() - 90.0).max(240.0)))
            .min_size([360.0, 220.0])
            .resizable(true)
            .show(ctx, |ui| {
                // ---- the toolbar
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Seleccionar todo").clicked() {
                        v.selected = (0..v.data.features.len()).collect();
                    }
                    if ui.button("Invertir").clicked() {
                        v.selected = (0..v.data.features.len()).filter(|i| !v.selected.contains(i)).collect();
                    }
                    if ui.button("Limpiar").clicked() {
                        v.selected.clear();
                    }
                    if ui.add_enabled(!v.selected.is_empty(), egui::Button::new("Zoom a la selección")).clicked() {
                        zoom = true;
                    }
                    if ui.add_enabled(!v.selected.is_empty(), egui::Button::new("Eliminar")).on_hover_text("Elimina los elementos seleccionados").clicked() {
                        delete = true;
                    }
                    ui.separator();
                    if ui.add_enabled(!v.undo.is_empty(), egui::Button::new("↶")).on_hover_text("Deshacer").clicked() {
                        undo = 1;
                    }
                    if ui.add_enabled(!v.redo.is_empty(), egui::Button::new("↷")).on_hover_text("Rehacer").clicked() {
                        undo = 2;
                    }
                    if ui.add_enabled(v.dirty, egui::Button::new("Guardar")).clicked() {
                        save = true;
                    }
                    ui.separator();
                    ui.menu_button("Seleccionar por atributo", |ui| {
                        if v.data.fields.is_empty() {
                            ui.label("La capa no tiene campos");
                            return;
                        }
                        ed.q_field = ed.q_field.min(v.data.fields.len() - 1);
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("q_field")
                                .width(110.0)
                                .selected_text(v.data.fields[ed.q_field].clone())
                                .show_ui(ui, |ui| {
                                    for (c, f) in v.data.fields.iter().enumerate() {
                                        ui.selectable_value(&mut ed.q_field, c, f);
                                    }
                                });
                            egui::ComboBox::from_id_salt("q_op")
                                .width(70.0)
                                .selected_text(ed.q_op.name())
                                .show_ui(ui, |ui| {
                                    for o in Op::ALL {
                                        ui.selectable_value(&mut ed.q_op, o, o.name());
                                    }
                                });
                            ui.add(egui::TextEdit::singleline(&mut ed.q_value).desired_width(90.0));
                        });
                        ui.horizontal(|ui| {
                            for (label, mode) in [("Nueva selección", 0u8), ("Añadir", 1), ("Dentro de la selección", 2)] {
                                if ui.button(label).clicked() {
                                    v.select_by(ed.q_field, ed.q_op, &ed.q_value, mode);
                                    ui.close();
                                }
                            }
                        });
                    });
                    ui.menu_button("Añadir campo", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut new_field).hint_text("nombre del campo").desired_width(130.0));
                        if ui.button("Añadir").clicked() && !new_field.trim().is_empty() {
                            v.push_undo();
                            v.data.fields.push(new_field.trim().to_string());
                            for row in &mut v.data.text {
                                row.push(String::new());
                            }
                            for row in &mut v.data.num {
                                row.push(f64::NAN);
                            }
                            new_field.clear();
                            v.dirty = true;
                            ui.close();
                        }
                    });
                    ui.checkbox(&mut ed.only_selected, "Solo seleccionados");
                });
                ui.add_space(2.0);
                // ---- the order of the rows
                let nfields = v.data.fields.len();
                if ed.sort.is_some_and(|(c, _)| c >= nfields) {
                    ed.sort = None;
                }
                let key = (k, ed.sort, v.data.features.len());
                if ed.sort_key != Some(key) {
                    let mut order: Vec<usize> = (0..v.data.features.len()).collect();
                    if let Some((c, up)) = ed.sort {
                        let text = |i: usize| v.data.text.get(i).and_then(|r| r.get(c)).map_or("", String::as_str);
                        let num = |i: usize| v.data.num.get(i).and_then(|r| r.get(c)).copied().unwrap_or(f64::NAN);
                        let numeric = order.iter().take(200).all(|&i| num(i).is_finite() || text(i).is_empty());
                        order.sort_by(|&a, &b| {
                            let o = if numeric {
                                num(a).partial_cmp(&num(b)).unwrap_or_else(|| num(a).is_nan().cmp(&num(b).is_nan()))
                            } else {
                                text(a).to_lowercase().cmp(&text(b).to_lowercase())
                            };
                            if up { o } else { o.reverse() }
                        });
                    }
                    ed.order = order;
                    ed.sort_key = Some(key);
                }
                let rows: Vec<usize> = if ed.only_selected {
                    ed.order.iter().copied().filter(|i| v.selected.contains(i)).collect()
                } else {
                    ed.order.clone()
                };
                // ---- the columns: their widths are kept for the layer, and fitted to the content
                let widths = ed.col_w.entry(k).or_default();
                widths.resize(nfields + 1, 0.0);
                widths[0] = NUM_W;
                for c in 0..nfields {
                    if widths[c + 1] <= 0.0 {
                        let longest = v
                            .data
                            .text
                            .iter()
                            .take(200)
                            .filter_map(|r| r.get(c))
                            .map(|s| s.chars().count())
                            .max()
                            .unwrap_or(0)
                            .max(v.data.fields[c].chars().count() + 3);
                        widths[c + 1] = (longest as f32 * 7.0 + 18.0).clamp(64.0, 260.0);
                    }
                }
                let total_w: f32 = widths.iter().sum();
                let total_h = HEAD_H + rows.len() as f32 * ROW_H;
                let mut resized: Option<(usize, f32)> = None;
                let mut fit: Option<usize> = None;
                let mut sort_click: Option<usize> = None;
                let mut start_edit: Option<(usize, usize)> = None;
                let mut row_click: Option<(usize, bool, bool)> = None;
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, view| {
                        let (area, _) = ui.allocate_exact_size(Vec2::new(total_w, total_h), Sense::hover());
                        let painter = ui.painter_at(ui.clip_rect());
                        let first = (((view.min.y - HEAD_H) / ROW_H).floor().max(0.0)) as usize;
                        let last = (((view.max.y - HEAD_H) / ROW_H).ceil().max(0.0) as usize + 1).min(rows.len());
                        let mut xs = vec![area.left()];
                        for w in widths.iter() {
                            xs.push(xs.last().copied().unwrap_or(0.0) + w);
                        }
                        for r in first..last {
                            let i = rows[r];
                            let y = area.top() + HEAD_H + r as f32 * ROW_H;
                            let row_rect = Rect::from_min_size(Pos2::new(area.left(), y), Vec2::new(total_w, ROW_H));
                            let on = v.selected.contains(&i);
                            let fill = if on {
                                t.selection
                            } else if r % 2 == 1 {
                                t.hover.gamma_multiply(0.45)
                            } else {
                                Color32::TRANSPARENT
                            };
                            painter.rect_filled(row_rect, 0.0, fill);
                            // row number: click selects the row
                            let num_rect = Rect::from_min_max(row_rect.min, Pos2::new(xs[1], y + ROW_H));
                            let nr = ui.interact(num_rect, ui.id().with(("tn", i)), Sense::click());
                            painter.rect_filled(num_rect, 0.0, t.header);
                            painter.text(
                                Pos2::new(xs[1] - 8.0, y + ROW_H / 2.0),
                                Align2::RIGHT_CENTER,
                                format!("{}", i + 1),
                                FontId::proportional(11.5),
                                if on { t.accent } else { t.weak },
                            );
                            if nr.clicked() {
                                let (shift, cmd) = ui.input(|inp| (inp.modifiers.shift, inp.modifiers.command));
                                row_click = Some((i, shift, cmd));
                            }
                            for c in 0..nfields {
                                let cell = Rect::from_min_max(Pos2::new(xs[c + 1], y), Pos2::new(xs[c + 2], y + ROW_H));
                                if ed.editing == Some((i, c)) {
                                    let Some(text) = v.data.text.get_mut(i).and_then(|row| row.get_mut(c)) else { continue };
                                    let before = text.clone();
                                    let resp = ui.put(cell.shrink2(Vec2::new(1.0, 0.0)), egui::TextEdit::singleline(text).margin(Vec2::new(5.0, 2.0)));
                                    if resp.changed() {
                                        let after = v.data.text[i][c].clone();
                                        let val = after.trim().replace(',', ".").parse().unwrap_or(f64::NAN);
                                        // the undo point is the cell as it was on entering it
                                        if ed.cell_old.as_ref().is_none_or(|o| o.0 != i || o.1 != c) {
                                            let mut snap = v.data.clone();
                                            snap.text[i][c] = before.clone();
                                            snap.num[i][c] = before.trim().replace(',', ".").parse().unwrap_or(f64::NAN);
                                            v.undo.push(snap);
                                            v.redo.clear();
                                            v.edit_touched = true;
                                            ed.cell_old = Some((i, c, before));
                                        }
                                        if let Some(x) = v.data.num.get_mut(i).and_then(|row| row.get_mut(c)) {
                                            *x = val;
                                        }
                                        v.dirty = true;
                                        if ed.sort.is_some_and(|(sc, _)| sc == c) {
                                            ed.sort_key = None;
                                        }
                                        if v.sym.field == Some(c) || v.labels.field == Some(c) {
                                            v.reclassify();
                                        }
                                    }
                                    if resp.lost_focus() {
                                        ed.editing = None;
                                        ed.cell_old = None;
                                    } else if !resp.has_focus() {
                                        resp.request_focus();
                                    }
                                    continue;
                                }
                                let cr = ui.interact(cell, ui.id().with(("tc", i, c)), Sense::click());
                                if cr.double_clicked() {
                                    start_edit = Some((i, c));
                                } else if cr.clicked() {
                                    let (shift, cmd) = ui.input(|inp| (inp.modifiers.shift, inp.modifiers.command));
                                    row_click = Some((i, shift, cmd));
                                }
                                let text = v.data.text.get(i).and_then(|row| row.get(c)).map_or("", String::as_str);
                                let numeric = v.data.num.get(i).and_then(|row| row.get(c)).is_some_and(|x| x.is_finite());
                                let clip = painter.with_clip_rect(cell.shrink2(Vec2::new(0.0, 0.0)).intersect(ui.clip_rect()));
                                clip.text(
                                    if numeric { Pos2::new(cell.right() - 6.0, y + ROW_H / 2.0) } else { Pos2::new(cell.left() + 6.0, y + ROW_H / 2.0) },
                                    if numeric { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER },
                                    text,
                                    FontId::proportional(12.0),
                                    t.text,
                                );
                            }
                            // grid lines
                            painter.line_segment([Pos2::new(row_rect.left(), y + ROW_H), Pos2::new(row_rect.right(), y + ROW_H)], Stroke::new(1.0, t.border.gamma_multiply(0.5)));
                        }
                        for x in &xs[1..] {
                            painter.line_segment(
                                [Pos2::new(*x, area.top() + HEAD_H), Pos2::new(*x, area.bottom())],
                                Stroke::new(1.0, t.border.gamma_multiply(0.5)),
                            );
                        }
                        // ---- the header stays at the top of the view
                        let top = area.top() + view.min.y;
                        let head = Rect::from_min_size(Pos2::new(area.left(), top), Vec2::new(total_w, HEAD_H));
                        painter.rect_filled(head, 0.0, t.header);
                        painter.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.border));
                        for c in 0..nfields {
                            let cell = Rect::from_min_max(Pos2::new(xs[c + 1], top), Pos2::new(xs[c + 2], top + HEAD_H));
                            let hr = ui.interact(cell.with_max_x(cell.right() - 6.0), ui.id().with(("th", c)), Sense::click());
                            if hr.hovered() {
                                painter.rect_filled(cell, 0.0, t.hover);
                            }
                            if hr.clicked() {
                                sort_click = Some(c);
                            }
                            let arrow = match ed.sort {
                                Some((sc, up)) if sc == c => {
                                    if up { " ▲" } else { " ▼" }
                                }
                                _ => "",
                            };
                            painter.with_clip_rect(cell.intersect(ui.clip_rect())).text(
                                Pos2::new(cell.left() + 6.0, top + HEAD_H / 2.0),
                                Align2::LEFT_CENTER,
                                format!("{}{arrow}", v.data.fields[c]),
                                FontId::proportional(12.0),
                                t.text,
                            );
                            // the right edge drags to resize; a double click fits the content
                            let handle = Rect::from_min_max(Pos2::new(cell.right() - 4.0, top), Pos2::new(cell.right() + 4.0, top + HEAD_H));
                            let hd = ui.interact(handle, ui.id().with(("tr", c)), Sense::click_and_drag());
                            if hd.hovered() || hd.dragged() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
                                painter.line_segment([Pos2::new(cell.right(), top + 3.0), Pos2::new(cell.right(), top + HEAD_H - 3.0)], Stroke::new(2.0, t.accent));
                            }
                            if hd.double_clicked() {
                                fit = Some(c);
                            } else if hd.dragged() {
                                resized = Some((c, (cell.width() + hd.drag_delta().x).max(36.0)));
                            }
                        }
                        painter.text(
                            Pos2::new(xs[1] - 8.0, top + HEAD_H / 2.0),
                            Align2::RIGHT_CENTER,
                            "#",
                            FontId::proportional(12.0),
                            t.weak,
                        );
                    });
                if let Some((c, w)) = resized
                    && let Some(slot) = ed.col_w.get_mut(&k).and_then(|ws| ws.get_mut(c + 1))
                {
                    *slot = w;
                }
                if let Some(c) = fit
                    && let Some(slot) = ed.col_w.get_mut(&k).and_then(|ws| ws.get_mut(c + 1))
                {
                    *slot = 0.0;
                }
                if let Some(c) = sort_click {
                    ed.sort = match ed.sort {
                        Some((sc, true)) if sc == c => Some((c, false)),
                        Some((sc, false)) if sc == c => None,
                        _ => Some((c, true)),
                    };
                }
                if let Some((i, shift, cmd)) = row_click {
                    ed.editing = None;
                    if shift && let Some(a) = ed.last_row {
                        let (lo, hi) = (a.min(i), a.max(i));
                        v.selected.extend(lo..=hi);
                    } else if cmd {
                        if !v.selected.remove(&i) {
                            v.selected.insert(i);
                        }
                    } else {
                        v.selected.clear();
                        v.selected.insert(i);
                    }
                    ed.last_row = Some(i);
                }
                if let Some(cell) = start_edit {
                    ed.editing = Some(cell);
                    ed.cell_old = None;
                }
                ui.separator();
                ui.label(
                    egui::RichText::new(format!(
                        "{} de {} elementos seleccionados · {} campos{}",
                        v.selected.len(),
                        v.data.features.len(),
                        nfields,
                        if v.dirty { " · cambios sin guardar" } else { "" }
                    ))
                    .size(11.5)
                    .color(t.weak),
                );
            });
        let sel_bounds = v.selection_bounds();
        match undo {
            1 => {
                v.undo();
            }
            2 => {
                v.redo();
            }
            _ => {}
        }
        if undo != 0 || delete {
            ed.sort_key = None;
        }
        if delete && !v.selected.is_empty() {
            v.push_undo();
            let which = v.selected.clone();
            v.remove_features(&which);
        }
        self.vedit = ed;
        self.table_new_field = new_field;
        if zoom && let Some(b) = sel_bounds {
            self.zoom_to_bounds(b);
        }
        if save {
            self.save_layer(k, false);
        }
        if !open {
            self.table_layer = None;
        }
    }
}
