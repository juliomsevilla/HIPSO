//! Edición de capas vectoriales en el mapa y en la tabla de atributos: digitalización con ajuste,
//! selección, movimiento de vértices y elementos, inserción y borrado de vértices, deshacer y
//! rehacer, y selección por atributo.

use super::*;
use crate::layers::{VecLayer, dist_to_segment};
use hipso_core::fieldtype::FieldType;
use std::collections::HashMap;

/// Una comparación para seleccionar por atributo.
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

    /// Si una celda pasa, leyendo el valor escrito según el tipo de su campo: números y fechas por
    /// su número, sí/no como igual o no, texto como texto sin distinguir mayúsculas.
    fn test(self, ty: FieldType, text: &str, num: f64, value: &str) -> bool {
        let want = value.trim();
        if self == Op::Contains {
            return text.to_lowercase().contains(&want.to_lowercase());
        }
        let ordered = |o: std::cmp::Ordering| match self {
            Op::Eq => o.is_eq(),
            Op::Ne => o.is_ne(),
            Op::Lt => o.is_lt(),
            Op::Le => o.is_le(),
            Op::Gt => o.is_gt(),
            _ => o.is_ge(),
        };
        match ty {
            FieldType::Integer | FieldType::Real | FieldType::Date | FieldType::Bool => {
                // un valor que no es del tipo no selecciona nada (o, para ≠, todo lo que tiene
                // valor)
                let Ok((_, w)) = ty.parse(want) else { return false };
                if w.is_nan() || num.is_nan() {
                    return self == Op::Ne && !text.is_empty();
                }
                ordered(num.total_cmp(&w))
            }
            FieldType::Text | FieldType::Link => ordered(text.to_lowercase().cmp(&want.to_lowercase())),
        }
    }
}

/// Un vértice: elemento, parte, índice en la parte.
type Vertex = (usize, usize, usize);

pub struct VecEditState {
    /// La digitalización y el movimiento de vértices se ajustan a los vértices de las capas
    /// visibles.
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
    /// La celda en la que se está escribiendo: elemento, campo y su texto al entrar.
    cell_old: Option<(usize, usize, String)>,
    /// La celda que se está editando: elemento y campo.
    editing: Option<(usize, usize)>,
    /// Lo que se escribe en la celda que se edita (comprobado contra el tipo de su campo al salir).
    edit_buf: String,
    /// Por qué se rechazó el último valor escrito o el último cambio de tipo.
    pub err: Option<String>,
    /// El tipo elegido para un campo que se va a añadir.
    pub new_type: hipso_core::fieldtype::FieldType,
    /// Ancho de cada columna de la tabla de cada capa (el índice 0 es el número de fila).
    col_w: HashMap<usize, Vec<f32>>,
    /// Campo por el que se ordena la tabla y si sube.
    sort: Option<(usize, bool)>,
    /// El orden de las filas para `sort_key` (capa, orden, número de filas).
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
            edit_buf: String::new(),
            err: None,
            new_type: hipso_core::fieldtype::FieldType::Text,
            col_w: HashMap::new(),
            sort: None,
            order: Vec::new(),
            sort_key: None,
        }
    }
}

impl VecLayer {
    /// Mueve los elementos seleccionados.
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

    /// Fija un vértice, manteniendo cerrados los anillos cerrados.
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

    /// Selecciona por un atributo; `mode` 0 sustituye la selección, 1 la amplía, 2 la reduce.
    fn select_by(&mut self, field: usize, op: Op, value: &str, mode: u8) {
        let ty = self.data.field_type(field);
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
                op.test(ty, text, num, value)
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

    /// El vértice más cercano de las capas visibles a un punto del mapa, a menos de `tol` (`skip`:
    /// una capa y elemento cuyos vértices no cuentan).
    fn snap_to(&self, m: (f64, f64), skip: Option<(usize, usize)>) -> [f64; 2] {
        let tol = self.tol(10.0);
        let (mut best, mut d) = ([m.0, m.1], tol);
        let mut on_vertex = false;
        for (k, r) in self.refs.iter().enumerate().filter(|(_, r)| r.visible) {
            let RefKind::Vector(v) = &r.kind else {
                continue;
            };
            for i in v.in_window([m.0 - tol, m.1 - tol, m.0 + tol, m.1 + tol]) {
                if skip == Some((k, i)) {
                    continue;
                }
                if self.vedit.snap {
                    for q in v.data.features[i].parts.iter().flatten() {
                        let dd = (q[0] - m.0).hypot(q[1] - m.1);
                        if dd < d {
                            (d, best, on_vertex) = (dd, *q, true);
                        }
                    }
                }
            }
        }
        // sin vértice a tiro, el punto más cercano de un borde (los vértices mandan sobre los bordes)
        if self.vm.snap_edges && !on_vertex {
            for (k, r) in self.refs.iter().enumerate().filter(|(_, r)| r.visible) {
                let RefKind::Vector(v) = &r.kind else {
                    continue;
                };
                for i in v.in_window([m.0 - tol, m.1 - tol, m.0 + tol, m.1 + tol]) {
                    if skip == Some((k, i)) {
                        continue;
                    }
                    for part in &v.data.features[i].parts {
                        for w in part.windows(2) {
                            let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                            let l2 = dx * dx + dy * dy;
                            if l2 == 0.0 {
                                continue;
                            }
                            let t = (((m.0 - w[0][0]) * dx + (m.1 - w[0][1]) * dy) / l2).clamp(0.0, 1.0);
                            let q = [w[0][0] + t * dx, w[0][1] + t * dy];
                            let dd = (q[0] - m.0).hypot(q[1] - m.1);
                            if dd < d {
                                (d, best) = (dd, q);
                            }
                        }
                    }
                }
            }
        }
        best
    }

    pub fn digitize_click(&mut self, m: (f64, f64)) {
        let p = if self.vedit.snap || self.vm.snap_edges {
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
        // un doble clic añade su vértice dos veces
        if d.len() >= 2 && d.last() == d.get(d.len() - 2) {
            d.pop();
        }
        if kind == view::GeomKind::Polygon && self.vm.reshape {
            self.reshape_polygons(&d);
            return;
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

    /// Una línea dibujada sobre una capa de polígonos puede editar los polígonos en lugar de hacer
    /// uno nuevo: empezando y terminando dentro de polígonos (los mismos o distintos), añade el
    /// terreno que cubre fuera de ellos como un polígono nuevo junto a todos ellos; cualquier otra
    /// línea corta todo polígono que divide, uno o muchos. Devuelve si hizo una de las dos cosas.
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
        // cualquier otra línea corta todo polígono que divide, sean cuales sean los polígonos en
        // que están sus extremos
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

    /// Activa o desactiva una herramienta de edición. Desactivar una termina la sesión de edición
    /// de la capa: si cambió, se pregunta al usuario qué hacer con los cambios.
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

    /// Sale de las herramientas de edición; pregunta por los cambios si los hay.
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

    /// La pregunta que se hace cuando se desactiva una herramienta de edición tras hacer cambios.
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

    /// Selecciona el elemento superior en un punto del mapa.
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

    /// El vértice de la capa activa más cercano a un punto del mapa, a menos de `tol`.
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
            // el anillo empieza y termina en el mismo vértice: se mantiene cerrado
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

    /// El trabajo con el puntero de las herramientas vectoriales en la planta.
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
        // un arrastre se nota después de que el puntero se haya movido: lo que agarró está bajo la
        // pulsación
        let press = resp.ctx.input(|i| i.pointer.press_origin());
        match self.nav {
            Nav::Digitize => {
                if resp.clicked()
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    let m = to_map(self, p);
                    self.digitize_click(m);
                }
                // clic derecho o doble clic cierran una línea o un área (Intro también)
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
                    let pos = if self.vedit.snap || self.vm.snap_edges {
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

    /// Teclas de las herramientas vectoriales: deshacer, rehacer, borrar, terminar y cancelar.
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

    /// El elemento que se dibuja, los elementos seleccionados, los vértices que se editan y el
    /// rectángulo de selección.
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
        // los vértices de los elementos seleccionados de la capa que se edita
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

    // ------------------------------------------------------------------------ la tabla

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
                // ---- la barra de herramientas
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
                            let ty = v.data.field_type(ed.q_field);
                            if ty == FieldType::Bool {
                                egui::ComboBox::from_id_salt("q_bool")
                                    .width(60.0)
                                    .selected_text(if ed.q_value.is_empty() { "Sí" } else { ed.q_value.as_str() })
                                    .show_ui(ui, |ui| {
                                        for o in ["Sí", "No"] {
                                            ui.selectable_value(&mut ed.q_value, o.to_string(), o);
                                        }
                                    });
                                if ed.q_value.is_empty() {
                                    ed.q_value = "Sí".into();
                                }
                            } else {
                                ui.add(
                                    egui::TextEdit::singleline(&mut ed.q_value)
                                        .hint_text(match ty {
                                            FieldType::Date => "AAAA-MM-DD",
                                            FieldType::Integer | FieldType::Real => "número",
                                            _ => "valor",
                                        })
                                        .desired_width(90.0),
                                );
                            }
                        });
                        {
                            let ty = v.data.field_type(ed.q_field);
                            ui.label(egui::RichText::new(format!("Campo de tipo «{}»: {}", ty.name(), ty.help())).size(11.0).weak());
                            if !ed.q_value.trim().is_empty() && ty.parse(&ed.q_value).is_err() && ty != FieldType::Text && ty != FieldType::Link {
                                ui.label(egui::RichText::new(format!("«{}» no es un valor de ese tipo: no selecciona nada", ed.q_value.trim())).size(11.0).color(Color32::from_rgb(200, 60, 40)));
                            }
                        }
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
                        egui::ComboBox::from_id_salt("new_field_type")
                            .selected_text(ed.new_type.name())
                            .show_ui(ui, |ui| {
                                for t in FieldType::ALL {
                                    ui.selectable_value(&mut ed.new_type, t, t.name());
                                }
                            });
                        ui.label(egui::RichText::new(ed.new_type.help()).size(11.0).weak());
                        let dup = v.data.fields.iter().any(|f| f.trim().eq_ignore_ascii_case(new_field.trim()));
                        if dup && !new_field.trim().is_empty() {
                            ui.label(egui::RichText::new("Ya hay un campo con ese nombre").size(11.0).color(Color32::from_rgb(200, 60, 40)));
                        }
                        if ui.add_enabled(!dup, egui::Button::new("Añadir")).clicked() && !new_field.trim().is_empty() {
                            v.push_undo();
                            v.data.add_field(new_field.trim(), ed.new_type);
                            new_field.clear();
                            v.dirty = true;
                            ui.close();
                        }
                    });
                    ui.checkbox(&mut ed.only_selected, "Solo seleccionados");
                });
                ui.add_space(2.0);
                // ---- el orden de las filas
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
                        let numeric = matches!(
                            v.data.field_type(c),
                            FieldType::Integer | FieldType::Real | FieldType::Date | FieldType::Bool
                        );
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
                // ---- las columnas: sus anchos se guardan para la capa y se ajustan al contenido
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
                let types: Vec<FieldType> = (0..nfields).map(|c| v.data.field_type(c)).collect();
                let mut retype: Option<(usize, FieldType)> = None;
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
                            // número de fila: un clic selecciona la fila
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
                                    let resp = ui.put(cell.shrink2(Vec2::new(1.0, 0.0)), egui::TextEdit::singleline(&mut ed.edit_buf).margin(Vec2::new(5.0, 2.0)));
                                    let cancel = ui.input(|inp| inp.key_pressed(egui::Key::Escape));
                                    if resp.lost_focus() {
                                        let before = v.data.text.get(i).and_then(|r| r.get(c)).cloned().unwrap_or_default();
                                        if !cancel && ed.edit_buf != before {
                                            // el valor debe ser de los del tipo de su campo
                                            match v.data.field_type(c).parse(&ed.edit_buf) {
                                                Ok(_) => {
                                                    v.push_undo();
                                                    let _ = v.data.set_value(i, c, &ed.edit_buf);
                                                    v.dirty = true;
                                                    ed.err = None;
                                                    if ed.sort.is_some_and(|(sc, _)| sc == c) {
                                                        ed.sort_key = None;
                                                    }
                                                    if v.sym.field == Some(c) || v.labels.field == Some(c) {
                                                        v.reclassify();
                                                    }
                                                }
                                                Err(m) => {
                                                    ed.err = Some(format!(
                                                        "{m}; el campo «{}» es de tipo {} ({})",
                                                        v.data.fields[c],
                                                        v.data.field_type(c).name(),
                                                        v.data.field_type(c).help()
                                                    ));
                                                }
                                            }
                                        }
                                        ed.editing = None;
                                        ed.cell_old = None;
                                    } else if !resp.has_focus() {
                                        resp.request_focus();
                                    }
                                    continue;
                                }
                                let cr = ui.interact(cell, ui.id().with(("tc", i, c)), Sense::click());
                                let ty = types[c];
                                let text = v.data.text.get(i).and_then(|row| row.get(c)).map_or("", String::as_str);
                                let is_link = ty == FieldType::Link && !text.trim().is_empty();
                                if cr.double_clicked() {
                                    start_edit = Some((i, c));
                                } else if cr.clicked() {
                                    if is_link {
                                        // un clic en un hipervínculo lo abre; el número de fila
                                        // selecciona la fila
                                        let url = text.trim();
                                        let target = if url.contains("://") || url.starts_with("mailto:") {
                                            url.to_string()
                                        } else {
                                            format!("file://{url}")
                                        };
                                        ui.ctx().open_url(egui::OpenUrl::new_tab(target));
                                    } else {
                                        let (shift, cmd) = ui.input(|inp| (inp.modifiers.shift, inp.modifiers.command));
                                        row_click = Some((i, shift, cmd));
                                    }
                                }
                                if is_link {
                                    cr.clone().on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Clic: abrir · doble clic: editar");
                                }
                                let right = matches!(ty, FieldType::Integer | FieldType::Real);
                                let clip = painter.with_clip_rect(cell.intersect(ui.clip_rect()));
                                let ink = if is_link { t.accent } else { t.text };
                                let galley_pos = if right { Pos2::new(cell.right() - 6.0, y + ROW_H / 2.0) } else { Pos2::new(cell.left() + 6.0, y + ROW_H / 2.0) };
                                let r = clip.text(
                                    galley_pos,
                                    if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER },
                                    text,
                                    FontId::proportional(12.0),
                                    ink,
                                );
                                if is_link {
                                    clip.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, ink));
                                }
                            }
                            // líneas de cuadrícula
                            painter.line_segment([Pos2::new(row_rect.left(), y + ROW_H), Pos2::new(row_rect.right(), y + ROW_H)], Stroke::new(1.0, t.border.gamma_multiply(0.5)));
                        }
                        for x in &xs[1..] {
                            painter.line_segment(
                                [Pos2::new(*x, area.top() + HEAD_H), Pos2::new(*x, area.bottom())],
                                Stroke::new(1.0, t.border.gamma_multiply(0.5)),
                            );
                        }
                        // ---- la cabecera se queda arriba en la vista
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
                            let hr = hr.on_hover_text(format!("{} · {}", types[c].name(), types[c].help()));
                            hr.context_menu(|ui| {
                                ui.label(egui::RichText::new(format!("Tipo: {}", types[c].name())).strong());
                                for t2 in FieldType::ALL {
                                    if t2 != types[c] && ui.button(format!("Cambiar a {}", t2.name())).clicked() {
                                        retype = Some((c, t2));
                                        ui.close();
                                    }
                                }
                            });
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
                            // el borde derecho se arrastra para cambiar el tamaño; un doble clic lo
                            // ajusta al contenido
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
                if let Some((c, to)) = retype {
                    // todo valor debe ser válido para el tipo nuevo, o no cambia nada
                    let bad = v.data.text.iter().enumerate().find_map(|(r, row)| {
                        to.parse(row.get(c).map_or("", String::as_str)).err().map(|m| (r, m))
                    });
                    match bad {
                        Some((r, m)) => {
                            ed.err = Some(format!(
                                "No se puede pasar «{}» a {}: fila {}, {m}",
                                v.data.fields[c],
                                to.name(),
                                r + 1
                            ));
                        }
                        None => {
                            v.push_undo();
                            v.data.fix_types();
                            v.data.types[c] = to;
                            for r in 0..v.data.text.len() {
                                let raw = v.data.text[r][c].clone();
                                let _ = v.data.set_value(r, c, &raw);
                            }
                            v.dirty = true;
                            ed.err = None;
                            ed.sort_key = None;
                            v.reclassify_if_data();
                        }
                    }
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
                    ed.err = None;
                    ed.edit_buf = v.data.text.get(cell.0).and_then(|r| r.get(cell.1)).cloned().unwrap_or_default();
                }
                if let Some(m) = &ed.err {
                    ui.label(egui::RichText::new(m).size(11.5).color(Color32::from_rgb(200, 60, 40)));
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
