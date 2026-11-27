//! El panel de contenido: el mapa con sus capas en orden de dibujo y las nubes de puntos, como
//! árbol al estilo de una tabla de contenido de un SIG. Cada fila tiene una flecha de despliegue,
//! una casilla de visibilidad, el símbolo de la capa y su nombre; las acciones de una fila aparecen
//! cuando el puntero está sobre ella, y una capa se puede arrastrar a otro lugar del orden de
//! dibujo.

use super::*;
use crate::layers::{GroupCmd, Item, LayerCmd};
use crate::symbology::SymKind;
use eframe::egui::Response;

const ROW_H: f32 = 23.0;
const INDENT: f32 = 16.0;

/// Casilla de visibilidad de una fila: todo, parte o nada de lo que contiene.
#[derive(Clone, Copy, PartialEq)]
enum Check {
    On,
    Off,
    Mixed,
}

/// Qué dibujar en una fila.
struct RowSpec<'a> {
    depth: usize,
    /// `Some(open)`: la fila tiene hijos y muestra una flecha.
    arrow: Option<bool>,
    check: Option<Check>,
    icon: Option<(&'a str, Color32)>,
    /// Una muestra de color en lugar de un icono (las clases de una simbología).
    swatch: Option<Color32>,
    label: String,
    bold: bool,
    selected: bool,
    dim: bool,
    /// Acciones que se muestran a la derecha mientras el puntero está en la fila: (glifo, ayuda
    /// emergente).
    actions: &'a [(&'a str, &'a str)],
}

/// Lo que ocurrió en una fila.
#[derive(Default)]
struct RowOut {
    row: Option<Response>,
    arrow: bool,
    check: bool,
    action: Option<usize>,
}

fn draw_check(p: &egui::Painter, r: Rect, c: Check, t: &Tokens) {
    let b = Rect::from_center_size(r.center(), Vec2::splat(14.0));
    p.rect_stroke(b, 3.0, Stroke::new(1.2, t.weak), egui::StrokeKind::Inside);
    match c {
        Check::On => {
            p.rect_filled(b, 3.0, t.accent);
            let (a, m, e) = (
                Pos2::new(b.left() + 3.0, b.center().y + 0.5),
                Pos2::new(b.center().x - 0.8, b.bottom() - 3.5),
                Pos2::new(b.right() - 3.0, b.top() + 3.5),
            );
            let s = Stroke::new(1.8, Color32::WHITE);
            p.line_segment([a, m], s);
            p.line_segment([m, e], s);
        }
        Check::Mixed => {
            p.rect_filled(b.shrink(4.0), 1.5, t.accent);
        }
        Check::Off => {}
    }
}

fn draw_arrow(p: &egui::Painter, r: Rect, open: bool, t: &Tokens) {
    let c = r.center();
    let pts = if open {
        vec![
            Pos2::new(c.x - 4.0, c.y - 2.0),
            Pos2::new(c.x + 4.0, c.y - 2.0),
            Pos2::new(c.x, c.y + 3.0),
        ]
    } else {
        vec![
            Pos2::new(c.x - 2.0, c.y - 4.0),
            Pos2::new(c.x - 2.0, c.y + 4.0),
            Pos2::new(c.x + 3.0, c.y),
        ]
    };
    p.add(egui::Shape::convex_polygon(pts, t.weak, Stroke::NONE));
}

/// Dibuja una fila e informa de los clics en ella.
fn row(ui: &mut egui::Ui, t: &Tokens, id: egui::Id, s: &RowSpec) -> RowOut {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
    let mut out = RowOut::default();
    let main = ui.interact(rect, id, Sense::click_and_drag());
    let hovered = main.hovered() || ui.rect_contains_pointer(rect);
    let p = ui.painter();
    if s.selected {
        p.rect_filled(rect, 3.0, t.selection);
    } else if hovered {
        p.rect_filled(rect, 3.0, t.hover);
    }
    let mut x = rect.left() + 2.0 + s.depth as f32 * INDENT;
    let slot = |x: f32, w: f32| Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, ROW_H));
    // la flecha mantiene su sitio aunque la fila no tenga ninguna, de modo que las casillas se
    // alinean
    let ar = slot(x, 16.0);
    if let Some(open) = s.arrow {
        draw_arrow(p, ar, open, t);
        out.arrow = ui.interact(ar, id.with("arrow"), Sense::click()).clicked();
    }
    x += 16.0;
    if let Some(c) = s.check {
        let cr = slot(x, 20.0);
        draw_check(ui.painter(), cr, c, t);
        out.check = ui.interact(cr, id.with("check"), Sense::click()).clicked();
        x += 20.0;
    }
    let ir = slot(x, 20.0);
    if let Some((icon, tint)) = s.icon {
        ribbon::draw_icon(ui.painter(), ir.center(), 15.0, icon, tint);
        x += 20.0;
    } else if let Some(c) = s.swatch {
        let sw = Rect::from_center_size(ir.center(), Vec2::splat(11.0));
        ui.painter().rect_filled(sw, 2.0, c);
        ui.painter().rect_stroke(
            sw,
            2.0,
            Stroke::new(0.8, t.border),
            egui::StrokeKind::Inside,
        );
        x += 20.0;
    }
    // las acciones de la derecha, desde el borde hacia dentro
    let mut right = rect.right() - 4.0;
    let show_actions = hovered || s.selected;
    if show_actions {
        for (i, (glyph, tip)) in s.actions.iter().enumerate().rev() {
            let ar =
                Rect::from_min_size(Pos2::new(right - 20.0, rect.top()), Vec2::new(20.0, ROW_H));
            let r = ui.interact(ar, id.with(("action", i)), Sense::click());
            if r.hovered() {
                ui.painter()
                    .rect_filled(ar.shrink2(Vec2::new(1.0, 2.0)), 3.0, t.border);
            }
            let ink = if r.hovered() { t.text } else { t.weak };
            if *glyph == "solo" {
                // un destino: la única nube que se queda
                ui.painter()
                    .circle_stroke(ar.center(), 5.5, Stroke::new(1.4, ink));
                ui.painter().circle_filled(ar.center(), 2.0, ink);
            } else {
                ribbon::draw_icon(ui.painter(), ar.center(), 14.0, glyph, ink);
            }
            if r.on_hover_text(*tip).clicked() {
                out.action = Some(i);
            }
            right -= 20.0;
        }
    }
    let width = (right - x - 4.0).max(20.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        s.label.clone(),
        if s.bold {
            FontId::proportional(13.0)
        } else {
            FontId::proportional(12.5)
        },
        t.text,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let color = if s.dim { t.weak } else { t.text };
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        Pos2::new(x + 2.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    out.row = Some(main);
    out
}

/// Una línea fina de encabezado para el filtro y las herramientas sobre el árbol.
fn memory_flag(ui: &egui::Ui, id: egui::Id, default: bool) -> bool {
    ui.ctx().data_mut(|d| *d.get_temp_mut_or(id, default))
}

fn set_flag(ui: &egui::Ui, id: egui::Id, v: bool) {
    ui.ctx().data_mut(|d| d.insert_temp(id, v));
}

impl App {
    /// La pestaña «Contenido» del panel lateral.
    pub fn contents_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        let filter_id = egui::Id::new("contents_filter");
        let mut filter: String = ui
            .ctx()
            .data_mut(|d| d.get_temp::<String>(filter_id))
            .unwrap_or_default();
        // herramientas sobre el árbol
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 76.0).max(60.0);
            ui.add(
                egui::TextEdit::singleline(&mut filter)
                    .hint_text("Buscar en el contenido")
                    .desired_width(w),
            );
            if ribbon::icon_button(ui, &t, "@add", "Abrir nubes, rásteres o capas vectoriales")
                .clicked()
            {
                self.act(ctx, Action::Open);
            }
            if ribbon::icon_button(ui, &t, "@selall", "Mostrar todo").clicked() {
                self.style.files = u64::MAX;
                self.show_points = true;
                for r in &mut self.refs {
                    r.visible = true;
                }
            }
        });
        ui.ctx()
            .data_mut(|d| d.insert_temp(filter_id, filter.clone()));
        let needle = filter.trim().to_lowercase();
        let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
        // el grupo solo cuenta mientras contiene la capa que está seleccionada
        if self.sel_layer.is_none_or(|k| !self.sel_layers.contains(&k)) {
            self.sel_layers.clear();
        }
        let mut group: Option<GroupCmd> = None;
        if self.sel_layers.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{} capas seleccionadas", self.sel_layers.len()))
                        .size(11.5)
                        .color(t.weak),
                );
                for (icon, tip, c) in [
                    ("@selall", "Mostrar las capas seleccionadas", GroupCmd::Show),
                    ("@deselect", "Ocultar las capas seleccionadas", GroupCmd::Hide),
                    ("@full", "Zoom a las capas seleccionadas", GroupCmd::Zoom),
                    ("@above", "Subir las capas seleccionadas en el orden de dibujo", GroupCmd::Up),
                    ("@below", "Bajar las capas seleccionadas en el orden de dibujo", GroupCmd::Down),
                    ("@close", "Quitar las capas seleccionadas", GroupCmd::Remove),
                ] {
                    if ribbon::icon_button(ui, &t, icon, tip).clicked() {
                        group = Some(c);
                    }
                }
            });
        }
        ui.add_space(2.0);
        let n_clouds = self.clouds.len();
        let n_layers = self.refs.len();
        // el orden del dibujo, mostrado desde arriba
        let items: Vec<Item> = self.order_items();
        let on_top_first: Vec<Item> = items.iter().rev().copied().collect();
        // lo que se arrastra y dónde está cada fila de nivel superior en este fotograma
        let drag_id = egui::Id::new("contents_drag");
        let mut rows: Vec<(Item, Rect, f32)> = Vec::new();
        let mut reorder: Option<Vec<Item>> = None;
        let mut act: Vec<(usize, LayerCmd)> = Vec::new();
        let mut cloud_solo: Option<usize> = None;
        let mut cloud_zoom: Option<usize> = None;
        let mut cloud_remove: Option<usize> = None;
        let mut clouds_zoom = false;
        let mut clouds_remove_all = false;
        let mut clouds_step: Option<bool> = None;
        // las filas de las nubes, en su orden de dibujo, y la nube que cambia de puesto
        let stack = self.cloud_stack();
        let mut cloud_rows: Vec<(usize, Rect)> = Vec::new();
        let mut cloud_move: Option<(usize, usize)> = None;
        let sel_cloud_id = egui::Id::new("contents_sel_cloud");
        let mut sel_cloud: Option<usize> = ui.ctx().data_mut(|d| d.get_temp::<usize>(sel_cloud_id));
        // las nubes en otro sistema que la primera caerían en otro sitio
        let cloud_crs: Vec<Option<u16>> =
            self.clouds.clone().iter().map(|p| self.crs_of(p)).collect();
        let first_crs = cloud_crs.first().copied().flatten();
        let project_crs = self.project_crs();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                // ---- el mapa
                let map_id = egui::Id::new("contents_map");
                let map_open = memory_flag(ui, map_id, true);
                let all_vis = self.show_points && self.refs.iter().all(|r| r.visible);
                let any_vis = self.show_points || self.refs.iter().any(|r| r.visible);
                let m = row(
                    ui,
                    &t,
                    map_id.with("row"),
                    &RowSpec {
                        depth: 0,
                        arrow: Some(map_open),
                        check: Some(if all_vis {
                            Check::On
                        } else if any_vis {
                            Check::Mixed
                        } else {
                            Check::Off
                        }),
                        icon: Some(("@layers", t.accent)),
                        swatch: None,
                        label: "Mapa".to_string(),
                        bold: true,
                        selected: false,
                        dim: false,
                        actions: &[],
                    },
                );
                if m.arrow || m.row.as_ref().is_some_and(|r| r.double_clicked()) {
                    set_flag(ui, map_id, !map_open);
                }
                if m.check {
                    let on = !all_vis;
                    self.show_points = on;
                    for r in &mut self.refs {
                        r.visible = on;
                    }
                }
                if !map_open {
                    // el mapa base no forma parte del grupo: se queda cuando el grupo se pliega
                    ui.add_space(4.0);
                    self.basemap_row(ui, ctx, &t);
                    return;
                }
                if n_layers == 0 && n_clouds == 0 && !self.img_jobs.is_empty() {
                    // las imágenes se abren en segundo plano: se dice cuáles y desde cuándo
                    ui.add_space(6.0);
                    for j in &self.img_jobs {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let name = j.path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                            ui.label(egui::RichText::new(format!("abriendo {name}… {:.0} s", j.started.elapsed().as_secs_f64())).color(t.weak).size(12.0));
                        });
                    }
                    ctx.request_repaint_after(std::time::Duration::from_millis(500));
                } else if n_layers == 0 && n_clouds == 0 {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(
                            "Nada abierto. Usa «Abrir», o arrastra archivos a la ventana.",
                        )
                        .color(t.weak)
                        .size(12.0),
                    );
                }
                for item in &on_top_first {
                    match *item {
                        Item::Layer(k) => {
                            let (name, icon, tint, legend, visible, is_vec) = {
                                let r = &self.refs[k];
                                let dirty = matches!(&r.kind, RefKind::Vector(v) if v.dirty);
                                let (icon, tint, legend) = match &r.kind {
                                    RefKind::Vector(v) => (
                                        match v.kind {
                                            view::GeomKind::Point => "@lpoint",
                                            view::GeomKind::Line => "@lline",
                                            view::GeomKind::Polygon => "@lpoly",
                                        },
                                        color32(if v.kind == view::GeomKind::Polygon {
                                            v.fill
                                        } else {
                                            v.color
                                        }),
                                        if v.sym.kind == SymKind::Single {
                                            Vec::new()
                                        } else {
                                            v.sym.legend(v.color)
                                        },
                                    ),
                                    _ => ("@lraster", t.text, Vec::new()),
                                };
                                (
                                    format!("{}{}", r.name, if dirty { " *" } else { "" }),
                                    icon,
                                    tint,
                                    legend,
                                    r.visible,
                                    matches!(r.kind, RefKind::Vector(_)),
                                )
                            };
                            if !matches(&name) {
                                continue;
                            }
                            let own_path = self.refs[k].path.clone();
                            let layer_crs = self.crs_of(&own_path);
                            let mismatch = match (layer_crs, project_crs) {
                                (Some(a), Some(b)) => {
                                    hipso_core::crs::relation(a, b)
                                        == hipso_core::crs::Relation::Different
                                }
                                _ => false,
                            };
                            let tip = format!(
                                "{}{}",
                                if own_path.as_os_str().is_empty() {
                                    name.clone()
                                } else {
                                    own_path.display().to_string()
                                },
                                match (layer_crs, project_crs) {
                                    (Some(a), Some(b)) if mismatch => format!(
                                        "\n⚠ Está en {}, y la nube en {}: no coinciden en el mapa. Las herramientas la pasarán al sistema de la nube al combinarlas (solo X e Y).",
                                        hipso_core::crs::name(a),
                                        hipso_core::crs::name(b)
                                    ),
                                    (Some(a), _) => format!("\n{}", hipso_core::crs::name(a)),
                                    _ => String::new(),
                                }
                            );
                            let lid = egui::Id::new(("contents_layer", k));
                            let open = memory_flag(ui, lid.with("open"), true);
                            let editing = self.active == Some(k) && self.edit_layer_on();
                            // tabla (solo capas vectoriales), zoom y quitar
                            let actions: &[(&str, &str)] = if is_vec {
                                &[
                                    ("@table", "Tabla de atributos"),
                                    ("@full", "Zoom a la capa"),
                                    ("@close", "Quitar la capa"),
                                ]
                            } else {
                                &[("@full", "Zoom a la capa"), ("@close", "Quitar la capa")]
                            };
                            let o = row(
                                ui,
                                &t,
                                lid,
                                &RowSpec {
                                    depth: 1,
                                    arrow: (!legend.is_empty()).then_some(open),
                                    check: Some(if visible { Check::On } else { Check::Off }),
                                    icon: Some((icon, tint)),
                                    swatch: None,
                                    label: format!(
                                        "{}{}{}",
                                        if mismatch { "⚠ " } else { "" },
                                        name,
                                        if editing { "  ✎" } else { "" }
                                    ),
                                    bold: false,
                                    selected: self.sel_layer == Some(k) || self.sel_layers.contains(&k),
                                    dim: !visible,
                                    actions,
                                },
                            );
                            let mut bottom = o.row.as_ref().map_or(0.0, |r| r.rect.bottom());
                            if o.arrow {
                                set_flag(ui, lid.with("open"), !open);
                            }
                            if o.check {
                                // en una capa de la selección, la casilla gira toda la selección
                                if self.sel_layers.len() > 1 && self.sel_layers.contains(&k) {
                                    for j in self.sel_layers.clone() {
                                        if let Some(r) = self.refs.get_mut(j) {
                                            r.visible = !visible;
                                        }
                                    }
                                } else {
                                    self.refs[k].visible = !visible;
                                }
                            }
                            if let Some(i) = o.action {
                                let cmd = match (is_vec, i) {
                                    (true, 0) => LayerCmd::Table,
                                    (true, 1) | (false, 0) => LayerCmd::Zoom,
                                    _ => LayerCmd::Remove,
                                };
                                act.push((k, cmd));
                            }
                            if let Some(rr) = o.row {
                                rows.push((*item, rr.rect, 0.0));
                                let rr = rr.on_hover_text(tip);
                                if rr.clicked() {
                                    sel_cloud = None;
                                    let m = ui.input(|i| i.modifiers);
                                    if m.command {
                                        // Ctrl / Cmd: añade la capa a la selección o la quita
                                        if let Some(p) = self.sel_layer
                                            && self.sel_layers.is_empty()
                                        {
                                            self.sel_layers.insert(p);
                                        }
                                        if !self.sel_layers.remove(&k) {
                                            self.sel_layers.insert(k);
                                            self.sel_layer = Some(k);
                                        } else if self.sel_layer == Some(k) {
                                            self.sel_layer = self.sel_layers.iter().next_back().copied();
                                        }
                                    } else if m.shift && self.sel_layer.is_some() {
                                        // Mayús: todas las capas entre la última seleccionada y
                                        // esta
                                        let shown: Vec<usize> = on_top_first
                                            .iter()
                                            .filter_map(|i| match i {
                                                Item::Layer(j) => Some(*j),
                                                Item::Clouds => None,
                                            })
                                            .collect();
                                        let a = shown.iter().position(|j| Some(*j) == self.sel_layer);
                                        let b = shown.iter().position(|j| *j == k);
                                        if let (Some(a), Some(b)) = (a, b) {
                                            self.sel_layers.extend(shown[a.min(b)..=a.max(b)].iter().copied());
                                        }
                                        self.sel_layer = Some(k);
                                    } else {
                                        self.sel_layers.clear();
                                        self.sel_layers.insert(k);
                                        act.push((k, LayerCmd::Select));
                                    }
                                }
                                if rr.double_clicked() {
                                    act.push((k, LayerCmd::Zoom));
                                }
                                if rr.drag_started() && needle.is_empty() {
                                    ui.ctx().data_mut(|d| d.insert_temp(drag_id, (0u8, k)));
                                }
                                let is_img = matches!(self.refs[k].kind, RefKind::Image(_));
                                if self.sel_layers.len() > 1 && self.sel_layers.contains(&k) {
                                    let n = self.sel_layers.len();
                                    rr.context_menu(|ui| {
                                        for (text, c) in [
                                            (format!("Mostrar las {n} capas"), GroupCmd::Show),
                                            (format!("Ocultar las {n} capas"), GroupCmd::Hide),
                                            ("Zoom a las capas".to_string(), GroupCmd::Zoom),
                                            ("Subir en el orden de dibujo".to_string(), GroupCmd::Up),
                                            ("Bajar en el orden de dibujo".to_string(), GroupCmd::Down),
                                            (format!("Quitar las {n} capas"), GroupCmd::Remove),
                                        ] {
                                            if ui.button(text).clicked() {
                                                group = Some(c);
                                                ui.close();
                                            }
                                        }
                                    });
                                } else {
                                rr.context_menu(|ui| {
                                    let mut item = |ui: &mut egui::Ui, text: &str, c: LayerCmd| {
                                        if ui.button(text).clicked() {
                                            act.push((k, c));
                                            ui.close();
                                        }
                                    };
                                    item(ui, "Zoom a la capa", LayerCmd::Zoom);
                                    if is_vec {
                                        item(ui, "Tabla de atributos", LayerCmd::Table);
                                        item(ui, "Editar esta capa", LayerCmd::Edit);
                                        item(ui, "Guardar", LayerCmd::Save);
                                        item(ui, "Guardar como…", LayerCmd::SaveAs);
                                    }
                                    if is_img {
                                        item(ui, "Bandas e índices…", LayerCmd::Bands);
                                    }
                                    ui.separator();
                                    item(ui, "Subir en el orden de dibujo", LayerCmd::Up);
                                    item(ui, "Bajar en el orden de dibujo", LayerCmd::Down);
                                    item(ui, "Quitar", LayerCmd::Remove);
                                });
                                }
                            }
                            // la simbología, como hijos de la capa
                            if open {
                                for (text, c) in legend.iter().take(14) {
                                    let r = row(
                                        ui,
                                        &t,
                                        lid.with(("class", text.as_str())),
                                        &RowSpec {
                                            depth: 2,
                                            arrow: None,
                                            check: None,
                                            icon: None,
                                            swatch: Some(color32(*c)),
                                            label: text.clone(),
                                            bold: false,
                                            selected: false,
                                            dim: !visible,
                                            actions: &[],
                                        },
                                    );
                                    bottom = r.row.map_or(bottom, |r| r.rect.bottom());
                                }
                                if legend.len() > 14 {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "        … {} más",
                                            legend.len() - 14
                                        ))
                                        .size(11.0)
                                        .color(t.weak),
                                    );
                                    bottom = ui.min_rect().bottom();
                                }
                            }
                            if let Some(last) = rows.last_mut() {
                                last.2 = bottom;
                            }
                        }
                        Item::Clouds => {
                            if n_clouds == 0 {
                                continue;
                            }
                            let cid = egui::Id::new("contents_clouds");
                            let open = memory_flag(ui, cid, true);
                            let bits = self.style.files;
                            let files_on = move |k: usize| k >= 64 || bits & (1u64 << k) != 0;
                            let on_count = (0..n_clouds).filter(|k| files_on(*k)).count();
                            let state = if !self.show_points || on_count == 0 {
                                Check::Off
                            } else if on_count == n_clouds {
                                Check::On
                            } else {
                                Check::Mixed
                            };
                            let g = row(
                                ui,
                                &t,
                                cid.with("row"),
                                &RowSpec {
                                    depth: 1,
                                    arrow: Some(open),
                                    check: Some(state),
                                    icon: Some(("@lcloud", t.accent)),
                                    swatch: None,
                                    label: format!("Nubes de puntos ({n_clouds})"),
                                    bold: true,
                                    selected: false,
                                    dim: false,
                                    actions: &[
                                        ("@full", "Zoom a todas las nubes"),
                                        ("@close", "Cerrar todas las nubes"),
                                    ],
                                },
                            );
                            if g.arrow || g.row.as_ref().is_some_and(|r| r.double_clicked()) {
                                set_flag(ui, cid, !open);
                            }
                            if g.check {
                                let on = state != Check::On;
                                self.style.files = if on { u64::MAX } else { 0 };
                                self.show_points = on;
                            }
                            match g.action {
                                Some(0) => clouds_zoom = true,
                                Some(_) => clouds_remove_all = true,
                                None => {}
                            }
                            let header = g.row.as_ref().map(|r| r.rect);
                            if let Some(rr) = g.row {
                                if rr.drag_started() && needle.is_empty() {
                                    ui.ctx().data_mut(|d| d.insert_temp(drag_id, (1u8, 0usize)));
                                }
                                rr.context_menu(|ui| {
                                    if ui.button("Subir en el orden de dibujo").clicked() {
                                        clouds_step = Some(true);
                                        ui.close();
                                    }
                                    if ui.button("Bajar en el orden de dibujo").clicked() {
                                        clouds_step = Some(false);
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Ver todas").clicked() {
                                        self.style.files = u64::MAX;
                                        self.show_points = true;
                                        ui.close();
                                    }
                                    if ui.button("Zoom a todas las nubes").clicked() {
                                        clouds_zoom = true;
                                        ui.close();
                                    }
                                    if ui.button("Cerrar todas las nubes").clicked() {
                                        clouds_remove_all = true;
                                        ui.close();
                                    }
                                });
                            }
                            let mut bottom = header.map_or(0.0, |r| r.bottom());
                            if open {
                                for (place, &k) in stack.iter().enumerate() {
                                    let p = self.clouds[k].clone();
                                    let name = p
                                        .file_name()
                                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                                    if !matches(&name) {
                                        continue;
                                    }
                                    let warn = (|| {
                                        let (a, b) =
                                            (cloud_crs.get(k).copied().flatten()?, first_crs?);
                                        (hipso_core::crs::relation(a, b)
                                            == hipso_core::crs::Relation::Different)
                                            .then(|| {
                                                format!(
                                                    "⚠ Está en {}, y la primera nube en {}: al mezclarlas los puntos quedarían en coordenadas incompatibles.",
                                                    hipso_core::crs::name(a),
                                                    hipso_core::crs::name(b)
                                                )
                                            })
                                    })();
                                    let on = files_on(k);
                                    let o = row(
                                        ui,
                                        &t,
                                        egui::Id::new(("contents_cloud", k)),
                                        &RowSpec {
                                            depth: 2,
                                            arrow: None,
                                            check: Some(if on { Check::On } else { Check::Off }),
                                            icon: Some(("@lcloud", t.accent)),
                                            swatch: None,
                                            label: format!(
                                                "{}{}",
                                                if warn.is_some() { "⚠ " } else { "" },
                                                name
                                            ),
                                            bold: false,
                                            selected: sel_cloud == Some(k),
                                            dim: !on || !self.show_points,
                                            actions: &[
                                                ("@full", "Zoom a esta tesela"),
                                                ("@close", "Cerrar esta nube"),
                                            ],
                                        },
                                    );
                                    if o.check && k < 64 {
                                        self.style.files ^= 1u64 << k;
                                    }
                                    match o.action {
                                        Some(0) => cloud_zoom = Some(k),
                                        Some(_) => cloud_remove = Some(k),
                                        None => {}
                                    }
                                    if let Some(rr) = o.row {
                                        bottom = rr.rect.bottom();
                                        cloud_rows.push((k, rr.rect));
                                        if rr.drag_started() && needle.is_empty() {
                                            ui.ctx().data_mut(|d| d.insert_temp(drag_id, (2u8, k)));
                                        }
                                        let rr = rr.on_hover_text(match &warn {
                                            Some(w) => format!("{}\n{w}", p.display()),
                                            None => p.display().to_string(),
                                        });
                                        if rr.clicked() {
                                            sel_cloud = Some(k);
                                            self.sel_layer = None;
                                        }
                                        if rr.double_clicked() {
                                            cloud_zoom = Some(k);
                                        }
                                        rr.context_menu(|ui| {
                                            if ui.button("Zoom a esta tesela").clicked() {
                                                cloud_zoom = Some(k);
                                                ui.close();
                                            }
                                            if ui.button("Ver solo esta nube").clicked() {
                                                cloud_solo = Some(k);
                                                ui.close();
                                            }
                                            ui.separator();
                                            if ui.add_enabled(place > 0, egui::Button::new("Subir en el orden de dibujo")).clicked() {
                                                cloud_move = Some((k, place - 1));
                                                ui.close();
                                            }
                                            if ui.add_enabled(place + 1 < n_clouds, egui::Button::new("Bajar en el orden de dibujo")).clicked() {
                                                cloud_move = Some((k, place + 1));
                                                ui.close();
                                            }
                                            ui.separator();
                                            if ui.button("Cerrar esta nube").clicked() {
                                                cloud_remove = Some(k);
                                                ui.close();
                                            }
                                        });
                                    }
                                }
                                if let Some((_, st)) = &self.loading {
                                    ui.horizontal(|ui| {
                                        ui.add_space(46.0);
                                        ui.spinner();
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "cargando… {:.0} s{}",
                                                st.elapsed().as_secs_f64(),
                                                progress_text(*st)
                                                    .map_or(String::new(), |p| format!(" · {p}"))
                                            ))
                                            .size(11.5)
                                            .color(t.weak),
                                        );
                                    });
                                    bottom = ui.min_rect().bottom();
                                }
                            }
                            if let Some(h) = header {
                                rows.push((Item::Clouds, h, bottom));
                            }
                        }
                    }
                }
                // ---- arrastrar una capa o las nubes: la línea donde caería y el soltado
                let dragging: Option<(u8, usize)> =
                    ui.ctx().data_mut(|d| d.get_temp::<(u8, usize)>(drag_id));
                // una nube se mueve entre las nubes de su grupo
                if let Some((2, k)) = dragging {
                    let ptr = ui.input(|i| i.pointer.interact_pos());
                    if let (Some(p), Some(first), Some(last)) = (ptr, cloud_rows.first(), cloud_rows.last()) {
                        let slot = cloud_rows.iter().filter(|(_, r)| r.center().y < p.y).count();
                        let y = cloud_rows.get(slot).map_or(last.1.bottom(), |(_, r)| r.top());
                        ui.painter().line_segment(
                            [Pos2::new(first.1.left(), y), Pos2::new(first.1.right(), y)],
                            Stroke::new(2.0, t.accent),
                        );
                        if ui.input(|i| i.pointer.any_released())
                            && let Some(from) = cloud_rows.iter().position(|(j, _)| *j == k)
                        {
                            let to = if slot > from { slot - 1 } else { slot };
                            if to != from {
                                cloud_move = Some((k, to));
                            }
                        }
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        ui.ctx().data_mut(|d| d.remove::<(u8, usize)>(drag_id));
                    } else {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                } else if let Some((kind, k)) = dragging {
                    let from = if kind == 1 { Item::Clouds } else { Item::Layer(k) };
                    let ptr = ui.input(|i| i.pointer.interact_pos());
                    if let (Some(p), false) = (ptr, rows.is_empty()) {
                        // la posición: cuántas filas tienen su centro por encima del puntero
                        let slot = rows.iter().filter(|(_, r, _)| r.center().y < p.y).count();
                        let y = if slot < rows.len() {
                            rows[slot].1.top()
                        } else {
                            rows[rows.len() - 1].2.max(rows[rows.len() - 1].1.bottom())
                        };
                        let (x0, x1) = (rows[0].1.left(), rows[0].1.right());
                        ui.painter().line_segment(
                            [Pos2::new(x0, y), Pos2::new(x1, y)],
                            Stroke::new(2.0, t.accent),
                        );
                        if ui.input(|i| i.pointer.any_released()) {
                            let mut shown: Vec<Item> = rows.iter().map(|r| r.0).collect();
                            if shown.contains(&from) {
                                // una capa de la selección se lleva consigo toda la selección
                                let moving: Vec<Item> = if kind == 0
                                    && self.sel_layers.len() > 1
                                    && self.sel_layers.contains(&k)
                                {
                                    shown
                                        .iter()
                                        .copied()
                                        .filter(|i| matches!(i, Item::Layer(j) if self.sel_layers.contains(j)))
                                        .collect()
                                } else {
                                    vec![from]
                                };
                                let above = shown.iter().take(slot).filter(|i| moving.contains(i)).count();
                                shown.retain(|i| !moving.contains(i));
                                let dest = slot - above;
                                for (n, item) in moving.into_iter().enumerate() {
                                    shown.insert((dest + n).min(shown.len()), item);
                                }
                                // desde abajo; las nubes, cuando no hay ninguna abierta, conservan
                                // su sitio
                                let mut bottom_up: Vec<Item> = shown.into_iter().rev().collect();
                                if !bottom_up.contains(&Item::Clouds) {
                                    let at = self.cloud_slot.min(bottom_up.len());
                                    bottom_up.insert(at, Item::Clouds);
                                }
                                if bottom_up != items {
                                    reorder = Some(bottom_up);
                                }
                            }
                        }
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        ui.ctx().data_mut(|d| d.remove::<(u8, usize)>(drag_id));
                    } else {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                }
                // ---- el mapa base: la última fila, bajo todo lo que se dibuja encima
                ui.add_space(4.0);
                self.basemap_row(ui, ctx, &t);
            });
        ui.ctx().data_mut(|d| {
            match sel_cloud {
                Some(k) => {
                    d.insert_temp(sel_cloud_id, k);
                }
                None => {
                    d.remove::<usize>(sel_cloud_id);
                }
            };
        });
        for (k, c) in act {
            self.layer_command(ctx, k, c);
        }
        if let Some(c) = group {
            self.group_command(c);
        }
        if let Some(order) = reorder {
            self.apply_order(&order);
        }
        if let Some(up) = clouds_step {
            self.step_item(Item::Clouds, up);
        }
        if let Some((k, to)) = cloud_move {
            self.move_cloud(k, to);
        }
        if let Some(k) = cloud_solo
            && k < 64
        {
            self.style.files = 1u64 << k;
            self.show_points = true;
        }
        if let Some(k) = cloud_zoom {
            // la tesela: su extensión, a partir de la cabecera del archivo
            if let Ok(r) = hipso_core::las::Reader::open(&self.clouds[k]) {
                let b = r.header.bounds;
                self.zoom_to_bounds([b[0], b[1], b[3], b[4]]);
            }
        }
        if clouds_zoom {
            self.plan_fit = true;
        }
        if clouds_remove_all {
            self.act(ctx, Action::CloseAll);
        } else if let Some(k) = cloud_remove {
            if self.clouds.len() > 1 && self.drop_cloud_in_memory(ctx, k) {
                // las demás se quedan como están: no se vuelve a leer nada
            } else {
                self.drop_cloud(k);
                if self.clouds.is_empty() {
                    self.act(ctx, Action::CloseAll);
                } else {
                    self.reload(ctx);
                }
            }
        }
    }

    /// El mapa base como última fila de la lista: una casilla lo activa y desactiva, y mientras
    /// está activado su origen y opacidad se muestran debajo.
    fn basemap_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &Tokens) {
        let id = egui::Id::new("contents_basemap");
        let open = memory_flag(ui, id, true);
        let on = self.basemap.as_ref().is_some_and(|b| b.on);
        let r = row(
            ui,
            t,
            id.with("row"),
            &RowSpec {
                depth: 0,
                arrow: on.then_some(open),
                check: Some(if on { Check::On } else { Check::Off }),
                icon: Some(("@basemap", t.accent)),
                swatch: None,
                label: "Mapa base".to_string(),
                bold: false,
                selected: false,
                dim: !on,
                actions: &[],
            },
        );
        if r.check || r.row.as_ref().is_some_and(|x| x.double_clicked()) {
            self.act(ctx, Action::ToggleBasemap);
        }
        if r.arrow {
            set_flag(ui, id, !open);
        }
        let Some(bm) = self.basemap.as_mut().filter(|b| b.on && open) else {
            return;
        };
        ui.horizontal(|ui| {
            ui.add_space(INDENT + 36.0);
            ui.vertical(|ui| {
                let current = basemap::PROVIDERS
                    .get(bm.provider)
                    .map_or("Otro (dirección propia)", |p| p.name);
                egui::ComboBox::from_id_salt("basemap_provider")
                    .selected_text(current)
                    .width(ui.available_width().min(220.0))
                    .show_ui(ui, |ui| {
                        for (k, p) in basemap::PROVIDERS.iter().enumerate() {
                            ui.selectable_value(&mut bm.provider, k, p.name);
                        }
                        ui.selectable_value(
                            &mut bm.provider,
                            basemap::CUSTOM,
                            "Otro (dirección propia)",
                        );
                    });
                if bm.provider == basemap::CUSTOM {
                    ui.add(
                        egui::TextEdit::singleline(&mut bm.custom)
                            .hint_text("https://…/{z}/{x}/{y}.png")
                            .desired_width(ui.available_width().min(220.0)),
                    );
                }
            });
        });
    }
}
