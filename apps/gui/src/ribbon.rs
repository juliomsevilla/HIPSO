//! Ribbon widgets in a ribbon style: a tab strip, groups with a caption, large buttons (icon
//! over label) and small ones (icon beside label).

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::theme::{BRAND, BRAND_HOVER, Tokens};

fn fill(t: &Tokens, r: &Response, selected: bool) -> Color32 {
    if selected {
        t.selection
    } else if r.is_pointer_button_down_on() {
        t.border
    } else if r.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    }
}

/// A 32 px icon over an 11.5 pt label.
pub fn big_button(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, selected: bool) -> Response {
    let r = big_button_body(ui, t, icon, label, selected);
    match crate::tips::tip(label) {
        Some(tip) => r.on_hover_text(tip),
        None => r,
    }
}

fn big_button_body(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, selected: bool) -> Response {
    let lines: Vec<&str> = label.split('\n').collect();
    let w = big_width(ui, label);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 66.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, 4.0, fill(t, &r, selected));
        if selected {
            p.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.0, t.accent),
                egui::StrokeKind::Inside,
            );
        }
        draw_icon(
            p,
            Pos2::new(rect.center().x, rect.top() + 21.0),
            26.0,
            icon,
            if selected { t.accent } else { t.text },
        );
        for (k, l) in lines.iter().enumerate() {
            p.text(
                Pos2::new(rect.center().x, rect.top() + 43.0 + k as f32 * 12.0),
                Align2::CENTER_CENTER,
                *l,
                FontId::proportional(11.5),
                t.text,
            );
        }
    }
    r
}

/// An 18 px icon beside a label, in a 22 px row.
pub fn small_button(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, selected: bool) -> Response {
    let r = small_button_body(ui, t, icon, label, selected);
    match crate::tips::tip(label) {
        Some(tip) => r.on_hover_text(tip),
        None => r,
    }
}

fn small_button_body(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, selected: bool) -> Response {
    let w = small_width(ui, label);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, 4.0, fill(t, &r, selected));
        draw_icon(
            p,
            Pos2::new(rect.left() + 12.0, rect.center().y),
            15.0,
            icon,
            if selected { t.accent } else { t.text },
        );
        p.text(
            Pos2::new(rect.left() + 26.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.0),
            t.text,
        );
    }
    r
}

/// A small button that opens a menu below it, drawn like [`small_button`] with a ▾ after the
/// label, so that menus sit in the ribbon as the other commands do.
pub fn small_menu(
    ui: &mut Ui,
    t: &Tokens,
    icon: &str,
    label: &str,
    enabled: bool,
    add: impl FnOnce(&mut Ui),
) -> Response {
    let text = format!("{label}  ▾");
    let r = ui
        .add_enabled_ui(enabled, |ui| small_button_body(ui, t, icon, &text, false))
        .inner;
    egui::Popup::menu(&r).show(add);
    r
}

/// One command of a ribbon group: its icon and label, whether its window is open, and what a
/// click does.
pub struct Item<A> {
    pub icon: &'static str,
    pub label: &'static str,
    pub on: bool,
    pub action: A,
}

/// A ribbon group described as data, so that it can be drawn whole or folded into one button.
pub struct Group<A> {
    pub caption: &'static str,
    /// The icon of the group when it is folded.
    pub icon: &'static str,
    /// Large buttons, then columns of up to three small ones.
    pub big: Vec<Item<A>>,
    pub columns: Vec<Vec<Item<A>>>,
}

/// The width of a text in the ribbon's font.
fn text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(size), Color32::WHITE)
        .size()
        .x
}

fn big_width(ui: &Ui, label: &str) -> f32 {
    let w = label
        .split('\n')
        .map(|l| text_width(ui, l, 11.5))
        .fold(0.0, f32::max);
    (w + 16.0).max(52.0)
}

fn small_width(ui: &Ui, label: &str) -> f32 {
    (text_width(ui, label, 12.0) + 34.0).max(60.0)
}

impl<A> Group<A> {
    /// The width the group takes when drawn whole.
    fn width(&self, ui: &Ui) -> f32 {
        let big: f32 = self.big.iter().map(|i| big_width(ui, i.label) + 2.0).sum();
        let cols: f32 = self
            .columns
            .iter()
            .map(|c| {
                c.iter()
                    .map(|i| small_width(ui, i.label))
                    .fold(0.0, f32::max)
                    + 2.0
            })
            .sum();
        (big + cols).max(text_width(ui, self.caption, 11.0)) + 12.0
    }

    fn folded_width(&self, ui: &Ui) -> f32 {
        big_width(ui, &format!("{} ▾", self.caption)) + 12.0
    }
}

/// Draws the groups whole when they fit the width of the ribbon; when they do not, the groups
/// from the right are folded, each into one large button with a menu of its commands, so every
/// command stays one click away whatever the size of the window.
pub fn adaptive_groups<A: Copy>(
    ui: &mut Ui,
    t: &Tokens,
    groups: &[Group<A>],
    mut act: impl FnMut(A),
) {
    let avail = ui.available_width() - 24.0;
    let mut folded = vec![false; groups.len()];
    let mut total: f32 = groups.iter().map(|g| g.width(ui)).sum();
    // from the right, and only where folding saves room
    for k in (0..groups.len()).rev() {
        if total <= avail {
            break;
        }
        let saved = groups[k].width(ui) - groups[k].folded_width(ui);
        if saved > 0.0 {
            total -= saved;
            folded[k] = true;
        }
    }
    for (g, fold) in groups.iter().zip(folded) {
        if fold {
            group(ui, t, g.caption, |ui| {
                let any_on = g.big.iter().chain(g.columns.iter().flatten()).any(|i| i.on);
                let label = format!("{} ▾", g.caption);
                let r = big_button_body(ui, t, g.icon, &label, any_on);
                egui::Popup::menu(&r).show(|ui| {
                    for i in g.big.iter().chain(g.columns.iter().flatten()) {
                        if small_button(ui, t, i.icon, &i.label.replace('\n', " "), i.on).clicked()
                        {
                            act(i.action);
                        }
                    }
                });
            });
            continue;
        }
        group(ui, t, g.caption, |ui| {
            for i in &g.big {
                if big_button(ui, t, i.icon, i.label, i.on).clicked() {
                    act(i.action);
                }
            }
            for col in &g.columns {
                ui.vertical(|ui| {
                    for i in col {
                        if small_button(ui, t, i.icon, i.label, i.on).clicked() {
                            act(i.action);
                        }
                    }
                });
            }
        });
    }
}

/// The spacing of the form controls inside a ribbon group: three rows fit the height of a
/// group, every control has the same height and the rows line up across groups.
pub fn compact(ui: &mut Ui) {
    let s = ui.spacing_mut();
    s.interact_size.y = 20.0;
    s.button_padding = Vec2::new(6.0, 1.0);
    s.item_spacing = Vec2::new(6.0, 3.0);
}

/// A − and a + button side by side, with their tooltips; `step(-1)` or `step(1)` on a click.
pub fn stepper(ui: &mut Ui, less: &str, more: &str, mut step: impl FnMut(i32)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if ui
            .add(egui::Button::new("−").min_size(Vec2::new(26.0, 20.0)))
            .on_hover_text(less)
            .clicked()
        {
            step(-1);
        }
        if ui
            .add(egui::Button::new("+").min_size(Vec2::new(26.0, 20.0)))
            .on_hover_text(more)
            .clicked()
        {
            step(1);
        }
    });
}

/// A square 28 px icon button for panels, lit when `selected`, with its name as a tooltip.
pub fn tool_button(ui: &mut Ui, t: &Tokens, icon: &str, tip: &str, selected: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(30.0, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, 4.0, fill(t, &r, selected));
        if selected {
            p.rect_stroke(
                rect,
                4.0,
                Stroke::new(1.0, t.accent),
                egui::StrokeKind::Inside,
            );
        }
        draw_icon(
            p,
            rect.center(),
            18.0,
            icon,
            if selected { t.accent } else { t.text },
        );
    }
    r.on_hover_text(tip)
}

/// A group of controls with its caption centred below and a rule after it.
pub fn group(ui: &mut Ui, t: &Tokens, caption: &str, add: impl FnOnce(&mut Ui)) {
    let r = ui
        .vertical(|ui| {
            // controls start at the top of the group, as in every ribbon: rows of different
            // groups line up and a taller group never pushes its caption out of the ribbon
            let inner = ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);
                ui.set_min_height(70.0);
                ui.set_max_height(70.0);
                add(ui);
            });
            let w = inner
                .response
                .rect
                .width()
                .max(caption.chars().count() as f32 * 6.0);
            ui.allocate_ui_with_layout(
                Vec2::new(w, 14.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.label(egui::RichText::new(caption).size(11.0).color(t.weak));
                },
            );
        })
        .response;
    let x = r.rect.right() + 4.0;
    ui.painter().line_segment(
        [
            Pos2::new(x, r.rect.top() + 4.0),
            Pos2::new(x, r.rect.bottom() - 2.0),
        ],
        Stroke::new(1.0, t.border),
    );
    ui.add_space(8.0);
}

/// The tab strip: a brand-coloured first button, then the tabs; returns true when the brand
/// button was clicked.
pub fn tab_strip(
    ui: &mut Ui,
    t: &Tokens,
    brand: &str,
    tabs: &[&str],
    contextual_from: usize,
    active: &mut usize,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let w = brand.chars().count() as f32 * 7.5 + 26.0;
        let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
        ui.painter()
            .rect_filled(rect, 4.0, if r.hovered() { BRAND_HOVER } else { BRAND });
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            brand,
            FontId::proportional(12.5),
            Color32::WHITE,
        );
        clicked = r.clicked();
        ui.add_space(6.0);
        for (k, name) in tabs.iter().enumerate() {
            let w = name.chars().count() as f32 * 7.2 + 24.0;
            let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
            let on = *active == k;
            if on {
                ui.painter().rect_filled(
                    Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.max.y + 2.0)),
                    egui::CornerRadius {
                        nw: 4,
                        ne: 4,
                        sw: 0,
                        se: 0,
                    },
                    t.ribbon,
                );
                ui.painter().line_segment(
                    [
                        Pos2::new(rect.left() + 4.0, rect.bottom() - 1.0),
                        Pos2::new(rect.right() - 4.0, rect.bottom() - 1.0),
                    ],
                    Stroke::new(2.0, t.accent),
                );
            } else if r.hovered() {
                ui.painter().rect_filled(rect, 4.0, t.hover);
            }
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                *name,
                FontId::proportional(12.5),
                if k >= contextual_from {
                    t.accent
                } else if on {
                    t.text
                } else {
                    t.weak
                },
            );
            if r.clicked() {
                *active = k;
            }
        }
    });
    clicked
}

type Line = (Vec<[f32; 2]>, bool);

/// The drawn icons (names starting with `@`), as polylines in a unit square plus dots.
fn icon_shapes(name: &str) -> (Vec<Line>, Vec<[f32; 2]>) {
    let rect: Vec<[f32; 2]> = vec![[0.1, 0.15], [0.9, 0.15], [0.9, 0.85], [0.1, 0.85]];
    let open = |v: &[[f32; 2]]| (v.to_vec(), false);
    match name {
        "classify" => (
            vec![open(&[
                [0.05, 0.78],
                [0.3, 0.62],
                [0.55, 0.74],
                [0.95, 0.52],
            ])],
            vec![
                [0.2, 0.3],
                [0.45, 0.2],
                [0.7, 0.3],
                [0.4, 0.42],
                [0.85, 0.2],
            ],
        ),
        "dtm" => (
            vec![open(&[
                [0.05, 0.85],
                [0.3, 0.35],
                [0.5, 0.62],
                [0.72, 0.2],
                [0.95, 0.85],
            ])],
            vec![],
        ),
        "terrain" => (
            vec![
                open(&[
                    [0.05, 0.85],
                    [0.35, 0.3],
                    [0.55, 0.6],
                    [0.75, 0.15],
                    [0.95, 0.85],
                ]),
                open(&[[0.15, 0.7], [0.4, 0.85], [0.65, 0.7], [0.85, 0.85]]),
            ],
            vec![],
        ),
        "chm" => (
            vec![
                (vec![[0.5, 0.08], [0.88, 0.66], [0.12, 0.66]], true),
                open(&[[0.5, 0.66], [0.5, 0.92]]),
            ],
            vec![],
        ),
        "plan" => (vec![(rect, true)], vec![]),
        "pp" => (
            vec![(rect, true), open(&[[0.1, 0.58], [0.9, 0.58]])],
            vec![],
        ),
        "p3" => (
            vec![(rect, true), open(&[[0.5, 0.15], [0.5, 0.85]])],
            vec![],
        ),
        "all" => (
            vec![
                (rect, true),
                open(&[[0.58, 0.15], [0.58, 0.85]]),
                open(&[[0.58, 0.5], [0.9, 0.5]]),
            ],
            vec![],
        ),
        "shape" => (
            vec![(
                vec![
                    [0.5, 0.08],
                    [0.9, 0.3],
                    [0.9, 0.7],
                    [0.5, 0.92],
                    [0.1, 0.7],
                    [0.1, 0.3],
                ],
                true,
            )],
            vec![],
        ),
        "tin" => (
            vec![
                (vec![[0.08, 0.82], [0.5, 0.12], [0.92, 0.82]], true),
                open(&[[0.5, 0.12], [0.5, 0.82]]),
                open(&[[0.08, 0.82], [0.5, 0.5], [0.92, 0.82]]),
            ],
            vec![],
        ),
        "aoi" => (
            vec![(
                vec![
                    [0.5, 0.1],
                    [0.9, 0.4],
                    [0.75, 0.88],
                    [0.25, 0.88],
                    [0.1, 0.4],
                ],
                true,
            )],
            vec![
                [0.5, 0.1],
                [0.9, 0.4],
                [0.75, 0.88],
                [0.25, 0.88],
                [0.1, 0.4],
            ],
        ),
        "density" => (
            vec![],
            (0..9)
                .map(|k| [0.2 + 0.3 * (k % 3) as f32, 0.2 + 0.3 * (k / 3) as f32])
                .collect(),
        ),
        "full" => (
            vec![
                open(&[[0.1, 0.35], [0.1, 0.1], [0.35, 0.1]]),
                open(&[[0.65, 0.1], [0.9, 0.1], [0.9, 0.35]]),
                open(&[[0.9, 0.65], [0.9, 0.9], [0.65, 0.9]]),
                open(&[[0.35, 0.9], [0.1, 0.9], [0.1, 0.65]]),
            ],
            vec![],
        ),
        "prev" => (
            vec![
                open(&[
                    [0.85, 0.35],
                    [0.55, 0.22],
                    [0.25, 0.4],
                    [0.2, 0.68],
                    [0.5, 0.85],
                ]),
                open(&[[0.4, 0.08], [0.55, 0.22], [0.4, 0.34]]),
            ],
            vec![],
        ),
        "clear" => (
            vec![
                open(&[[0.2, 0.2], [0.8, 0.8]]),
                open(&[[0.8, 0.2], [0.2, 0.8]]),
            ],
            vec![],
        ),
        "north" => (
            vec![
                open(&[[0.5, 0.92], [0.5, 0.12]]),
                open(&[[0.28, 0.36], [0.5, 0.1], [0.72, 0.36]]),
            ],
            vec![],
        ),
        "exup" => (
            vec![
                open(&[[0.5, 0.9], [0.5, 0.1]]),
                open(&[[0.3, 0.3], [0.5, 0.1], [0.7, 0.3]]),
                open(&[[0.3, 0.7], [0.5, 0.9], [0.7, 0.7]]),
            ],
            vec![],
        ),
        "exdn" => (
            vec![
                open(&[[0.5, 0.9], [0.5, 0.1]]),
                open(&[[0.3, 0.1], [0.5, 0.3], [0.7, 0.1]]),
                open(&[[0.3, 0.9], [0.5, 0.7], [0.7, 0.9]]),
            ],
            vec![],
        ),
        "selrect" => (
            vec![(
                vec![[0.12, 0.2], [0.88, 0.2], [0.88, 0.8], [0.12, 0.8]],
                true,
            )],
            vec![
                [0.12, 0.2],
                [0.88, 0.2],
                [0.88, 0.8],
                [0.12, 0.8],
                [0.5, 0.5],
            ],
        ),
        "lasso" => (
            vec![
                (
                    vec![
                        [0.2, 0.55],
                        [0.12, 0.3],
                        [0.35, 0.12],
                        [0.7, 0.15],
                        [0.9, 0.38],
                        [0.75, 0.62],
                        [0.45, 0.68],
                        [0.3, 0.6],
                    ],
                    true,
                ),
                open(&[[0.3, 0.62], [0.28, 0.82], [0.4, 0.92]]),
            ],
            vec![],
        ),
        "circle" => (vec![(circle_pts(), true)], vec![[0.5, 0.5]]),
        "selall" => (
            vec![
                (rect, true),
                open(&[[0.25, 0.3], [0.75, 0.3]]),
                open(&[[0.25, 0.5], [0.75, 0.5]]),
                open(&[[0.25, 0.7], [0.75, 0.7]]),
            ],
            vec![],
        ),
        "invert" => (
            vec![
                (rect.clone(), true),
                (
                    vec![[0.1, 0.15], [0.5, 0.15], [0.5, 0.85], [0.1, 0.85]],
                    true,
                ),
            ],
            vec![],
        ),
        "trash" => (
            vec![
                (
                    vec![[0.22, 0.28], [0.78, 0.28], [0.7, 0.9], [0.3, 0.9]],
                    true,
                ),
                open(&[[0.12, 0.28], [0.88, 0.28]]),
                open(&[[0.38, 0.28], [0.4, 0.12], [0.6, 0.12], [0.62, 0.28]]),
                open(&[[0.42, 0.42], [0.43, 0.78]]),
                open(&[[0.58, 0.42], [0.57, 0.78]]),
            ],
            vec![],
        ),
        "reclass" => (
            vec![
                open(&[[0.1, 0.3], [0.78, 0.3]]),
                open(&[[0.62, 0.12], [0.8, 0.3], [0.62, 0.48]]),
                open(&[[0.9, 0.7], [0.22, 0.7]]),
                open(&[[0.38, 0.52], [0.2, 0.7], [0.38, 0.88]]),
            ],
            vec![],
        ),
        "redo" => (
            vec![
                open(&[
                    [0.15, 0.35],
                    [0.45, 0.22],
                    [0.75, 0.4],
                    [0.8, 0.68],
                    [0.5, 0.85],
                ]),
                open(&[[0.6, 0.08], [0.45, 0.22], [0.6, 0.34]]),
            ],
            vec![],
        ),
        "save" => (
            vec![
                (
                    vec![
                        [0.12, 0.12],
                        [0.78, 0.12],
                        [0.9, 0.24],
                        [0.9, 0.9],
                        [0.12, 0.9],
                    ],
                    true,
                ),
                (
                    vec![[0.28, 0.12], [0.28, 0.38], [0.7, 0.38], [0.7, 0.12]],
                    false,
                ),
                (
                    vec![[0.25, 0.9], [0.25, 0.62], [0.75, 0.62], [0.75, 0.9]],
                    false,
                ),
            ],
            vec![],
        ),
        "crs" => (
            vec![
                (circle_pts(), true),
                open(&[[0.1, 0.5], [0.9, 0.5]]),
                open(&[[0.5, 0.1], [0.5, 0.9]]),
                open(&[[0.5, 0.1], [0.3, 0.5], [0.5, 0.9]]),
                open(&[[0.5, 0.1], [0.7, 0.5], [0.5, 0.9]]),
            ],
            vec![],
        ),
        "newlayer" => (
            vec![
                (
                    vec![[0.1, 0.25], [0.6, 0.15], [0.7, 0.55], [0.25, 0.7]],
                    true,
                ),
                open(&[[0.78, 0.55], [0.78, 0.95]]),
                open(&[[0.58, 0.75], [0.98, 0.75]]),
            ],
            vec![],
        ),
        "pencil" => (
            vec![
                (
                    vec![[0.15, 0.85], [0.2, 0.6], [0.7, 0.1], [0.9, 0.3], [0.4, 0.8]],
                    true,
                ),
                open(&[[0.6, 0.2], [0.8, 0.4]]),
            ],
            vec![],
        ),
        "table" => (
            vec![
                (
                    vec![[0.1, 0.15], [0.9, 0.15], [0.9, 0.85], [0.1, 0.85]],
                    true,
                ),
                open(&[[0.1, 0.4], [0.9, 0.4]]),
                open(&[[0.1, 0.62], [0.9, 0.62]]),
                open(&[[0.42, 0.15], [0.42, 0.85]]),
            ],
            vec![],
        ),
        "rs" => (
            vec![
                (
                    vec![[0.08, 0.15], [0.92, 0.15], [0.92, 0.85], [0.08, 0.85]],
                    true,
                ),
                open(&[
                    [0.08, 0.7],
                    [0.35, 0.4],
                    [0.55, 0.62],
                    [0.72, 0.45],
                    [0.92, 0.7],
                ]),
            ],
            vec![[0.75, 0.3]],
        ),
        "bands" => (
            vec![
                open(&[[0.1, 0.25], [0.9, 0.25]]),
                open(&[[0.1, 0.5], [0.9, 0.5]]),
                open(&[[0.1, 0.75], [0.9, 0.75]]),
            ],
            vec![[0.3, 0.25], [0.6, 0.5], [0.45, 0.75]],
        ),
        "lpoint" => (vec![], vec![[0.5, 0.5]]),
        "lline" => (
            vec![open(&[[0.1, 0.85], [0.4, 0.3], [0.65, 0.65], [0.92, 0.12]])],
            vec![],
        ),
        "lpoly" => (
            vec![(
                vec![[0.12, 0.25], [0.7, 0.1], [0.92, 0.6], [0.4, 0.9]],
                true,
            )],
            vec![],
        ),
        "lraster" => (
            vec![
                (rect.clone(), true),
                open(&[[0.1, 0.5], [0.9, 0.5]]),
                open(&[[0.5, 0.15], [0.5, 0.85]]),
            ],
            vec![],
        ),
        "vertex" => (
            vec![open(&[[0.1, 0.8], [0.4, 0.3], [0.7, 0.6], [0.92, 0.15]])],
            vec![[0.1, 0.8], [0.4, 0.3], [0.7, 0.6], [0.92, 0.15]],
        ),
        "move" => (
            vec![
                open(&[[0.5, 0.08], [0.5, 0.92]]),
                open(&[[0.08, 0.5], [0.92, 0.5]]),
                open(&[[0.4, 0.2], [0.5, 0.08], [0.6, 0.2]]),
                open(&[[0.4, 0.8], [0.5, 0.92], [0.6, 0.8]]),
                open(&[[0.2, 0.4], [0.08, 0.5], [0.2, 0.6]]),
                open(&[[0.8, 0.4], [0.92, 0.5], [0.8, 0.6]]),
            ],
            vec![],
        ),
        "open" => (
            vec![(
                vec![
                    [0.08, 0.25],
                    [0.38, 0.25],
                    [0.48, 0.38],
                    [0.92, 0.38],
                    [0.92, 0.82],
                    [0.08, 0.82],
                ],
                true,
            )],
            vec![],
        ),
        "manual" => (
            vec![
                (
                    vec![[0.15, 0.1], [0.85, 0.1], [0.85, 0.9], [0.15, 0.9]],
                    true,
                ),
                open(&[[0.3, 0.3], [0.7, 0.3]]),
                open(&[[0.3, 0.5], [0.7, 0.5]]),
                open(&[[0.3, 0.7], [0.55, 0.7]]),
            ],
            vec![],
        ),
        "dsm" => (
            vec![
                open(&[
                    [0.05, 0.85],
                    [0.2, 0.55],
                    [0.3, 0.7],
                    [0.45, 0.25],
                    [0.6, 0.6],
                    [0.72, 0.4],
                    [0.95, 0.85],
                ]),
                open(&[[0.05, 0.85], [0.95, 0.85]]),
            ],
            vec![],
        ),
        "thin" => (
            vec![
                open(&[[0.08, 0.9], [0.92, 0.9]]),
                open(&[[0.08, 0.1], [0.08, 0.9]]),
            ],
            vec![
                [0.2, 0.2],
                [0.2, 0.45],
                [0.2, 0.7],
                [0.4, 0.3],
                [0.4, 0.6],
                [0.6, 0.2],
                [0.6, 0.45],
                [0.6, 0.7],
                [0.8, 0.3],
                [0.8, 0.6],
            ],
        ),
        "tree" => (
            vec![
                open(&[[0.5, 0.95], [0.5, 0.5]]),
                (vec![[0.5, 0.08], [0.22, 0.5], [0.78, 0.5]], true),
                (vec![[0.5, 0.3], [0.15, 0.78], [0.85, 0.78]], true),
            ],
            vec![],
        ),
        "volume" => (
            vec![
                open(&[
                    [0.05, 0.85],
                    [0.25, 0.5],
                    [0.5, 0.2],
                    [0.75, 0.5],
                    [0.95, 0.85],
                ]),
                open(&[[0.05, 0.85], [0.95, 0.85]]),
                open(&[[0.3, 0.85], [0.3, 0.5]]),
                open(&[[0.5, 0.85], [0.5, 0.25]]),
                open(&[[0.7, 0.85], [0.7, 0.5]]),
            ],
            vec![],
        ),
        "hist" => (
            vec![
                open(&[[0.08, 0.9], [0.92, 0.9]]),
                open(&[[0.08, 0.1], [0.08, 0.9]]),
                open(&[[0.18, 0.9], [0.18, 0.6], [0.34, 0.6], [0.34, 0.9]]),
                open(&[[0.4, 0.9], [0.4, 0.25], [0.56, 0.25], [0.56, 0.9]]),
                open(&[[0.62, 0.9], [0.62, 0.5], [0.78, 0.5], [0.78, 0.9]]),
            ],
            vec![],
        ),
        "dups" => (
            vec![
                (
                    vec![[0.1, 0.1], [0.62, 0.1], [0.62, 0.62], [0.1, 0.62]],
                    true,
                ),
                (
                    vec![[0.38, 0.38], [0.9, 0.38], [0.9, 0.9], [0.38, 0.9]],
                    true,
                ),
            ],
            vec![],
        ),
        "compare" => (
            vec![
                open(&[[0.05, 0.7], [0.3, 0.55], [0.55, 0.65], [0.95, 0.45]]),
                open(&[[0.05, 0.4], [0.3, 0.2], [0.55, 0.3], [0.95, 0.1]]),
            ],
            vec![[0.7, 0.75], [0.85, 0.85]],
        ),
        "fit" => (
            vec![
                open(&[[0.05, 0.6], [0.4, 0.4], [0.7, 0.5], [0.95, 0.3]]),
                open(&[[0.2, 0.9], [0.5, 0.65]]),
                open(&[[0.5, 0.65], [0.38, 0.68]]),
                open(&[[0.5, 0.65], [0.47, 0.78]]),
            ],
            vec![],
        ),
        "swath" => (
            vec![
                (
                    vec![[0.05, 0.2], [0.65, 0.2], [0.65, 0.6], [0.05, 0.6]],
                    true,
                ),
                (
                    vec![[0.35, 0.4], [0.95, 0.4], [0.95, 0.8], [0.35, 0.8]],
                    true,
                ),
            ],
            vec![],
        ),
        "stack" => (
            vec![
                (vec![[0.1, 0.25], [0.5, 0.1], [0.9, 0.25], [0.5, 0.4]], true),
                open(&[[0.1, 0.5], [0.5, 0.65], [0.9, 0.5]]),
                open(&[[0.1, 0.75], [0.5, 0.9], [0.9, 0.75]]),
            ],
            vec![],
        ),
        "mosaic" => (
            vec![
                (vec![[0.1, 0.1], [0.9, 0.1], [0.9, 0.9], [0.1, 0.9]], true),
                open(&[[0.5, 0.1], [0.5, 0.9]]),
                open(&[[0.1, 0.5], [0.9, 0.5]]),
            ],
            vec![[0.3, 0.3], [0.7, 0.7]],
        ),
        _ => (vec![], vec![]),
    }
}

/// An icon: a drawn one when `icon` starts with `@`, else a glyph.
pub fn draw_icon(p: &egui::Painter, c: Pos2, size: f32, icon: &str, color: Color32) {
    if let Some(name) = crate::icons::resolve(icon)
        && crate::icons::paint(p, c, size, name, color)
    {
        return;
    }
    let Some(name) = icon.strip_prefix('@') else {
        p.text(
            c,
            Align2::CENTER_CENTER,
            icon,
            FontId::proportional(size),
            color,
        );
        return;
    };
    let (lines, dots) = icon_shapes(name);
    if lines.is_empty() && dots.is_empty() && crate::icons::paint(p, c, size, "missing", color) {
        return;
    }
    let s = size * 0.9;
    let at = |q: &[f32; 2]| Pos2::new(c.x + (q[0] - 0.5) * s, c.y + (q[1] - 0.5) * s);
    let stroke = Stroke::new((size / 14.0).max(1.3), color);
    for (pts, closed) in lines {
        let mut v: Vec<Pos2> = pts.iter().map(&at).collect();
        if closed && let Some(f) = v.first().copied() {
            v.push(f);
        }
        p.add(egui::Shape::line(v, stroke));
    }
    for d in dots {
        p.circle_filled(at(&d), (size / 12.0).max(1.4), color);
    }
}

/// A small square icon button for the header.
pub fn icon_button(ui: &mut Ui, t: &Tokens, icon: &str, tip: &str) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(24.0, 24.0), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    draw_icon(ui.painter(), rect.center(), 15.0, icon, t.text);
    r.on_hover_text(tip)
}

fn circle_pts() -> Vec<[f32; 2]> {
    (0..16)
        .map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            [0.5 + 0.4 * a.cos(), 0.5 + 0.4 * a.sin()]
        })
        .collect()
}
