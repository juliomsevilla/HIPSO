//! Histograms of the point attributes, in the editor: the bars picked with the mouse, or the
//! tails of the odd values a statistic finds, become a selection of points. The editing tools
//! then delete those points or give them another class.

use std::collections::HashMap;

use super::*;
use crate::editing::Cmd;
use rustlidar_core::edit::{self, Attr, AttrKind, Mode, RangeSel, Region, SelOp};
use rustlidar_core::histogram::{self, Histogram, Method};

/// How the odd values of an attribute are told from the rest.
#[derive(Clone, Copy, PartialEq)]
enum Odd {
    Tukey,
    Mad,
    Sigma,
    Percentile,
}

impl Odd {
    const ALL: [Odd; 4] = [Odd::Tukey, Odd::Mad, Odd::Sigma, Odd::Percentile];

    fn name(self) -> &'static str {
        match self {
            Odd::Tukey => "Tukey: fuera de Q1 − k·RIC … Q3 + k·RIC",
            Odd::Mad => "Robusto: lejos de la mediana (MAD)",
            Odd::Sigma => "Desviaciones típicas: media ± k·σ",
            Odd::Percentile => "Percentiles: las colas más extremas",
        }
    }

    /// The label, default value and range of its parameter.
    fn param(self) -> (&'static str, f64, std::ops::RangeInclusive<f64>) {
        match self {
            Odd::Tukey => ("k (1,5 usual; 3 solo extremos)", 1.5, 0.1..=20.0),
            Odd::Mad => ("z robusto (3,5 usual)", 3.5, 0.5..=50.0),
            Odd::Sigma => ("k (3 usual)", 3.0, 0.5..=20.0),
            Odd::Percentile => ("% en cada cola", 1.0, 0.01..=25.0),
        }
    }

    fn method(self, k: f64) -> Method {
        match self {
            Odd::Tukey => Method::Tukey(k),
            Odd::Mad => Method::Mad(k),
            Odd::Sigma => Method::Sigma(k),
            Odd::Percentile => Method::Percentile(k),
        }
    }
}

/// What a cached histogram was made from: attribute, bars, only the selected points, and the
/// versions of the points and of the selection.
type Key = (AttrKind, usize, bool, u64, u64);

pub struct State {
    pub open: bool,
    kind: AttrKind,
    bins: f64,
    only_selected: bool,
    log: bool,
    /// The intervals marked on each attribute; a point must lie in the marked ones of all.
    ranges: HashMap<AttrKind, Vec<(f64, f64)>>,
    odd: Odd,
    k: f64,
    cache: Option<(Key, Histogram)>,
    /// A drag over the bars: where it began and whether it marks or clears.
    paint: Option<(usize, bool)>,
    notice: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            kind: AttrKind::Z,
            bins: 60.0,
            only_selected: false,
            log: true,
            ranges: HashMap::new(),
            odd: Odd::Tukey,
            k: 1.5,
            cache: None,
            paint: None,
            notice: String::new(),
        }
    }
}

fn compute(store: &PointStore, kind: AttrKind, bins: usize, only: bool) -> Option<Histogram> {
    histogram::histogram(
        store.len(),
        |i| {
            if store.deleted.get(i).copied().unwrap_or(false) {
                return None;
            }
            if only && !store.selected.get(i).copied().unwrap_or(false) {
                return None;
            }
            Some(edit::value_of(store, i, kind))
        },
        bins,
        kind.is_integer(),
    )
}

fn fmt(kind: AttrKind, v: f64) -> String {
    if !v.is_finite() {
        "—".into()
    } else if kind.is_integer() {
        format!("{v:.0}")
    } else if v.abs() >= 1000.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.3}")
    }
}

const PICKED: Color32 = Color32::from_rgb(0, 255, 255);

impl App {
    pub fn histogram_window(&mut self, ctx: &egui::Context) {
        if !self.ed.hist.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Histogramas de atributos")
            .open(&mut open)
            .default_width(600.0)
            .default_pos([360.0, 120.0])
            .show(ctx, |ui| {
                let mut st = std::mem::take(&mut self.ed.hist);
                self.histogram_body(ui, &mut st);
                self.ed.hist = st;
            });
        self.ed.hist.open = open;
    }

    fn histogram_body(&mut self, ui: &mut egui::Ui, st: &mut State) {
        let t = self.tokens();
        let Some(store) = self.store.clone() else {
            ui.label("Abre una nube para ver sus histogramas.");
            return;
        };
        let weak = |s: &str| egui::RichText::new(s.to_string()).color(t.weak).size(11.0);
        ui.label(weak("Cada barra cuenta los puntos de un intervalo del atributo. Marca barras con un clic o arrastrando; con «Seleccionar puntos» se seleccionan los que caen en las barras marcadas de todos los atributos, y después se eliminan o se les cambia la clase. La escala logarítmica deja ver las barras con pocos puntos, que son las de los valores raros."));
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("hist_attr")
                .width(190.0)
                .selected_text(st.kind.name())
                .show_ui(ui, |ui| {
                    for k in AttrKind::ALL {
                        let n = st.ranges.get(&k).map_or(0, Vec::len);
                        let label = if n > 0 {
                            format!("{} ●", k.name())
                        } else {
                            k.name().to_string()
                        };
                        ui.selectable_value(&mut st.kind, k, label);
                    }
                });
            if !st.kind.is_integer() {
                ui.label("Barras");
                ui.add(
                    crate::numfield::NumField::new(&mut st.bins)
                        .range(5.0..=400.0)
                        .speed(1.0),
                );
            }
            ui.checkbox(&mut st.log, "Escala logarítmica");
            ui.checkbox(&mut st.only_selected, "Solo los puntos seleccionados");
        });
        let key: Key = (
            st.kind,
            st.bins as usize,
            st.only_selected,
            self.rev,
            if st.only_selected { self.ed.sel_rev } else { 0 },
        );
        if st.cache.as_ref().is_none_or(|c| c.0 != key) {
            st.cache =
                compute(&store, st.kind, st.bins as usize, st.only_selected).map(|h| (key, h));
        }
        let cache = st.cache.take();
        match &cache {
            Some((_, h)) => self.histogram_chart(ui, st, h, &t),
            None => {
                ui.label("No hay puntos que contar.");
            }
        }
        st.cache = cache;
        self.histogram_actions(ui, st, &store, &t);
    }

    fn histogram_chart(&mut self, ui: &mut egui::Ui, st: &mut State, h: &Histogram, t: &Tokens) {
        let kind = st.kind;
        let nb = h.counts.len();
        let marked = st.ranges.get(&kind).cloned().unwrap_or_default();
        let mut picked = h.bars_in(&marked);
        picked.resize(nb, false);
        ui.label(
            egui::RichText::new(format!(
                "n = {}   mín {}   máx {}   media {}   σ {}",
                h.n,
                fmt(kind, h.min),
                fmt(kind, h.max),
                fmt(kind, h.mean),
                fmt(kind, h.std)
            ))
            .size(12.0),
        );
        ui.label(
            egui::RichText::new(format!(
                "mediana {}   Q1 {}   Q3 {}   MAD {}",
                fmt(kind, h.median),
                fmt(kind, h.q1),
                fmt(kind, h.q3),
                fmt(kind, h.mad)
            ))
            .size(12.0),
        );
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(320.0), 170.0),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, t.canvas);
        let max = h.counts.iter().copied().max().unwrap_or(1).max(1) as f64;
        let height = |c: u64| -> f32 {
            let v = if st.log {
                (1.0 + c as f64).ln() / (1.0 + max).ln()
            } else {
                c as f64 / max
            };
            let px = v as f32 * (rect.height() - 6.0);
            if c > 0 { px.max(2.0) } else { 0.0 }
        };
        let bw = rect.width() / nb as f32;
        let bar_at = |x: f32| (((x - rect.left()) / bw).max(0.0) as usize).min(nb - 1);
        let hover = resp.hover_pos().map(|p| bar_at(p.x));
        for (i, c) in h.counts.iter().enumerate() {
            let x0 = rect.left() + i as f32 * bw;
            let top = rect.bottom() - 3.0 - height(*c);
            let color = if picked[i] {
                PICKED
            } else if hover == Some(i) {
                t.accent
            } else {
                t.accent.gamma_multiply(0.6)
            };
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(x0, top),
                    Pos2::new((x0 + bw).max(x0 + 1.0), rect.bottom() - 3.0),
                ),
                0.0,
                color,
            );
        }
        // where the chosen statistic would cut
        let span = h.width * nb as f64;
        let x_of =
            |v: f64| rect.left() + (((v - h.lo) / span) as f32).clamp(0.0, 1.0) * rect.width();
        if let Some((lo, hi)) = h.limits(st.odd.method(st.k)) {
            for v in [lo, hi] {
                if v > h.lo && v < h.lo + span {
                    painter.line_segment(
                        [
                            Pos2::new(x_of(v), rect.top()),
                            Pos2::new(x_of(v), rect.bottom()),
                        ],
                        Stroke::new(1.5, Color32::from_rgb(255, 160, 40)),
                    );
                }
            }
        }
        let median_x = x_of(h.median);
        painter.line_segment(
            [
                Pos2::new(median_x, rect.bottom() - 8.0),
                Pos2::new(median_x, rect.bottom()),
            ],
            Stroke::new(2.0, t.text),
        );
        let mut changed = false;
        if let Some(pos) = resp.interact_pointer_pos() {
            let bar = bar_at(pos.x);
            if resp.drag_started() {
                st.paint = Some((bar, !picked[bar]));
            }
            if resp.dragged()
                && let Some((from, on)) = st.paint
            {
                let span = from.min(bar)..=from.max(bar);
                for p in picked.iter_mut().skip(*span.start()).take(span.count()) {
                    if *p != on {
                        *p = on;
                        changed = true;
                    }
                }
            } else if resp.clicked() {
                picked[bar] = !picked[bar];
                changed = true;
            }
        }
        if resp.drag_stopped() {
            st.paint = None;
        }
        if changed {
            let r = h.ranges(&picked);
            if r.is_empty() {
                st.ranges.remove(&kind);
            } else {
                st.ranges.insert(kind, r);
            }
            st.notice.clear();
        }
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(fmt(kind, h.lo))
                    .size(11.0)
                    .color(t.weak),
            );
            let info = hover.map_or(String::new(), |b| {
                let (lo, hi) = h.bar(b);
                let hi = if kind.is_integer() { lo } else { hi };
                let range = if kind.is_integer() {
                    fmt(kind, lo)
                } else {
                    format!("{} a {}", fmt(kind, lo), fmt(kind, hi))
                };
                format!("{range}: {} puntos", h.counts[b])
            });
            ui.label(egui::RichText::new(info).size(11.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(fmt(kind, h.lo + span))
                        .size(11.0)
                        .color(t.weak),
                );
            });
        });
        let marked_now = st.ranges.get(&kind).cloned().unwrap_or_default();
        if !marked_now.is_empty() {
            let bars = h.bars_in(&marked_now);
            ui.label(
                egui::RichText::new(format!(
                    "Marcado en {}: {} barra(s), ≈ {} puntos",
                    kind.name(),
                    bars.iter().filter(|b| **b).count(),
                    h.picked_count(&bars)
                ))
                .color(PICKED)
                .size(12.0),
            );
        }
        ui.separator();
        ui.label(egui::RichText::new("Valores raros según un estadístico").strong());
        ui.horizontal(|ui| {
            let before = st.odd;
            egui::ComboBox::from_id_salt("hist_odd")
                .width(300.0)
                .selected_text(st.odd.name())
                .show_ui(ui, |ui| {
                    for o in Odd::ALL {
                        ui.selectable_value(&mut st.odd, o, o.name());
                    }
                });
            if st.odd != before {
                st.k = st.odd.param().1;
            }
        });
        let (label, _, range) = st.odd.param();
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add(
                crate::numfield::NumField::new(&mut st.k)
                    .range(range)
                    .speed(0.05),
            );
        });
        match h.limits(st.odd.method(st.k)) {
            Some((lo, hi)) => {
                ui.label(
                    egui::RichText::new(format!(
                        "Raros: menos de {} o más de {} (líneas naranjas)",
                        fmt(kind, lo),
                        fmt(kind, hi)
                    ))
                    .color(Color32::from_rgb(255, 160, 40))
                    .size(12.0),
                );
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "Casi todos los valores son iguales: este estadístico no puede decidir.",
                    )
                    .color(t.weak)
                    .size(12.0),
                );
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Marcar los raros de este atributo").clicked() {
                match h.tails(st.odd.method(st.k)) {
                    Some(tails) => {
                        st.ranges.insert(kind, tails);
                        st.notice.clear();
                    }
                    None => "No hay límites para este atributo con este estadístico."
                        .clone_into(&mut st.notice),
                }
            }
            if ui.button("Quitar las marcas de este atributo").clicked() {
                st.ranges.remove(&kind);
            }
        });
    }

    fn histogram_actions(
        &mut self,
        ui: &mut egui::Ui,
        st: &mut State,
        store: &PointStore,
        t: &Tokens,
    ) {
        ui.separator();
        let active: Vec<AttrKind> = AttrKind::ALL
            .into_iter()
            .filter(|k| st.ranges.get(k).is_some_and(|r| !r.is_empty()))
            .collect();
        if active.is_empty() {
            ui.label(
                egui::RichText::new("Aún no hay barras marcadas.")
                    .color(t.weak)
                    .size(12.0),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label("Marcas activas (todas deben cumplirse):");
                for k in &active {
                    if ui
                        .small_button(format!("{} ✕", k.name()))
                        .on_hover_text("Quitar estas marcas")
                        .clicked()
                    {
                        st.ranges.remove(k);
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !active.is_empty(),
                    egui::Button::new("Seleccionar puntos"),
                )
                .on_hover_text("Los que cumplen las marcas de todos los atributos, a la vez. Se combina con la selección según el modo de la cinta.")
                .clicked()
            {
                let ranges: Vec<RangeSel> = active
                    .iter()
                    .filter_map(|k| {
                        st.ranges.get(k).map(|b| RangeSel {
                            kind: *k,
                            bins: b.clone(),
                        })
                    })
                    .collect();
                let op = SelOp {
                    mode: self.ed.mode,
                    region: Region::All,
                    attr: Attr {
                        ranges,
                        ..self.ed.attr.clone()
                    },
                };
                self.queue(Cmd::AddOp(Box::new(op)));
            }
            if ui
                .button("Seleccionar los raros de cualquier atributo")
                .on_hover_text("Aplica el estadístico elegido a cada atributo y selecciona los puntos raros en alguno de ellos. Sustituye la selección (o la amplía si el modo es «Añadir»).")
                .clicked()
            {
                let mut ops = Vec::new();
                for k in AttrKind::ALL {
                    let Some(h) = compute(store, k, st.bins as usize, st.only_selected) else {
                        continue;
                    };
                    let Some(tails) = h.tails(st.odd.method(st.k)) else {
                        continue;
                    };
                    ops.push(SelOp {
                        mode: if ops.is_empty() && self.ed.mode == Mode::New {
                            Mode::New
                        } else {
                            Mode::Add
                        },
                        region: Region::All,
                        attr: Attr {
                            ranges: vec![RangeSel {
                                kind: k,
                                bins: tails,
                            }],
                            ..self.ed.attr.clone()
                        },
                    });
                }
                if ops.is_empty() {
                    "Ningún atributo tiene valores raros con este estadístico.".clone_into(&mut st.notice);
                } else {
                    st.notice.clear();
                    for op in ops {
                        self.queue(Cmd::AddOp(Box::new(op)));
                    }
                }
            }
        });
        if !st.notice.is_empty() {
            ui.colored_label(Color32::from_rgb(230, 160, 60), st.notice.clone());
        }
        ui.label(
            egui::RichText::new(format!(
                "Seleccionados: {} puntos en pantalla (≈ {} en el archivo)",
                self.ed.sel_count,
                self.ed.sel_count as u64 * store.stride
            ))
            .strong(),
        );
        ui.horizontal(|ui| {
            if ui.button("Eliminar").clicked() {
                self.queue(Cmd::Delete);
            }
            if ui.button("Asignar clase").clicked() {
                self.queue(Cmd::Reclass(self.ed.class));
            }
            self.class_combo(ui);
        });
        ui.label(
            egui::RichText::new("Las ediciones se aplican a todos los puntos del archivo al guardar (pestaña Edición > Guardar nube editada).")
                .color(t.weak)
                .size(11.0),
        );
    }
}
