//! Diagnostic graphics of a transition model: observed against predicted, residuals against
//! fitted values, normal Q–Q of the residuals and Cook's distance. They are read from the plot
//! table (`…_parcelas.csv`) the model writes, and drawn with the painter: no plotting library.

use super::*;
use eframe::egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use rustlidar_core::aba::normal_quantile;

/// One plot of the model as the table has it.
#[derive(Clone)]
pub struct Row {
    pub key: String,
    pub obs: f64,
    pub fit: f64,
    pub cv: f64,
    pub stud: f64,
    pub cook: f64,
    /// The table flagged it (outlier, leverage, influential).
    pub flag: bool,
}

/// The diagnostics of the last model.
pub struct Diag {
    pub rows: Vec<Row>,
    pub response: String,
}

impl Diag {
    /// Reads the plot table of a model.
    pub fn read(path: &Path, response: &str) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut rows = Vec::new();
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split(',').collect();
            let n = |i: usize| f.get(i).and_then(|s| s.trim().parse::<f64>().ok());
            if let (Some(obs), Some(fit), Some(cv), Some(stud), Some(cook)) =
                (n(1), n(2), n(4), n(7), n(8))
            {
                rows.push(Row {
                    key: f[0].to_string(),
                    obs,
                    fit,
                    cv,
                    stud,
                    cook,
                    flag: f.get(9).is_some_and(|s| !s.trim().is_empty()),
                });
            }
        }
        (!rows.is_empty()).then(|| Self {
            rows,
            response: response.to_string(),
        })
    }
}

struct Pt {
    x: f64,
    y: f64,
    label: String,
    color: Color32,
    hollow: bool,
}

/// A line in data coordinates.
struct Ln {
    a: [f64; 2],
    b: [f64; 2],
    color: Color32,
    dashed: bool,
}

/// A rounded step that gives about `n` ticks over `span`.
fn nice_step(span: f64, n: f64) -> f64 {
    let raw = span / n;
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    mag * if f < 1.5 {
        1.0
    } else if f < 3.5 {
        2.0
    } else if f < 7.5 {
        5.0
    } else {
        10.0
    }
}

fn tick_text(v: f64, step: f64) -> String {
    let decimals = if step >= 1.0 {
        0
    } else {
        (-step.log10().floor()) as usize
    };
    format!("{v:.decimals$}")
}

/// A small scatter chart with axes. `stems` draws each point as a bar from zero.
#[allow(clippy::too_many_arguments)]
fn chart(
    ui: &mut egui::Ui,
    size: Vec2,
    title: &str,
    xl: &str,
    yl: &str,
    pts: &[Pt],
    lines: &[Ln],
    stems: bool,
) {
    let (resp, painter) = ui.allocate_painter(size, Sense::hover());
    let v = ui.visuals();
    let (fg, weak) = (v.text_color(), v.weak_text_color());
    let grid = v.widgets.noninteractive.bg_stroke.color;
    let r = resp.rect;
    painter.rect_filled(r, 3.0, v.extreme_bg_color);
    painter.text(
        Pos2::new(r.left() + 6.0, r.top() + 3.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(12.0),
        fg,
    );
    let plot = Rect::from_min_max(
        Pos2::new(r.left() + 62.0, r.top() + 22.0),
        Pos2::new(r.right() - 10.0, r.bottom() - 32.0),
    );
    if pts.is_empty() {
        return;
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for p in pts {
        for (k, val) in [p.x, p.y].into_iter().enumerate() {
            lo[k] = lo[k].min(val);
            hi[k] = hi[k].max(val);
        }
    }
    if stems {
        lo[1] = lo[1].min(0.0);
    }
    for l in lines {
        // lines widen the range only on the axis they are given for (thresholds, not the span)
        if l.a[0] == l.b[0] {
            lo[0] = lo[0].min(l.a[0]);
            hi[0] = hi[0].max(l.a[0]);
        }
        if l.a[1] == l.b[1] {
            lo[1] = lo[1].min(l.a[1]);
            hi[1] = hi[1].max(l.a[1]);
        }
    }
    for k in 0..2 {
        if !(hi[k] > lo[k]) {
            lo[k] -= 1.0;
            hi[k] += 1.0;
        }
        let pad = (hi[k] - lo[k]) * 0.06;
        lo[k] -= pad;
        hi[k] += pad;
    }
    let to = |x: f64, y: f64| {
        Pos2::new(
            plot.left() + ((x - lo[0]) / (hi[0] - lo[0])) as f32 * plot.width(),
            plot.bottom() - ((y - lo[1]) / (hi[1] - lo[1])) as f32 * plot.height(),
        )
    };
    // ticks and grid
    for k in 0..2 {
        let step = nice_step(hi[k] - lo[k], 4.0);
        let mut t = (lo[k] / step).ceil() * step;
        while t <= hi[k] {
            if k == 0 {
                let x = to(t, lo[1]).x;
                painter.line_segment(
                    [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                    Stroke::new(0.5, grid),
                );
                painter.text(
                    Pos2::new(x, plot.bottom() + 3.0),
                    Align2::CENTER_TOP,
                    tick_text(t, step),
                    FontId::proportional(10.0),
                    weak,
                );
            } else {
                let y = to(lo[0], t).y;
                painter.line_segment(
                    [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                    Stroke::new(0.5, grid),
                );
                painter.text(
                    Pos2::new(plot.left() - 4.0, y),
                    Align2::RIGHT_CENTER,
                    tick_text(t, step),
                    FontId::proportional(10.0),
                    weak,
                );
            }
            t += step;
        }
    }
    painter.rect_stroke(plot, 0.0, Stroke::new(1.0, grid), egui::StrokeKind::Inside);
    painter.text(
        Pos2::new(plot.center().x, r.bottom() - 2.0),
        Align2::CENTER_BOTTOM,
        xl,
        FontId::proportional(11.0),
        weak,
    );
    // the label of the vertical axis reads upwards, in its own strip left of the tick numbers
    let galley = painter.layout_no_wrap(yl.to_string(), FontId::proportional(11.0), weak);
    let gw = galley.size().x.min(plot.height());
    let at = Pos2::new(r.left() + 4.0, plot.center().y + gw / 2.0);
    painter.add(
        egui::epaint::TextShape::new(at, galley, weak).with_angle(-std::f32::consts::FRAC_PI_2),
    );
    let clipped = painter.with_clip_rect(plot);
    for l in lines {
        let (a, b) = (to(l.a[0], l.a[1]), to(l.b[0], l.b[1]));
        if l.dashed {
            for s in Shape::dashed_line(&[a, b], Stroke::new(1.0, l.color), 5.0, 4.0) {
                clipped.add(s);
            }
        } else {
            clipped.line_segment([a, b], Stroke::new(1.0, l.color));
        }
    }
    let hover = resp.hover_pos();
    let mut near: Option<(f32, usize)> = None;
    for (i, p) in pts.iter().enumerate() {
        let c = to(p.x, p.y);
        if stems {
            clipped.line_segment([to(p.x, 0.0), c], Stroke::new(3.0, p.color));
        } else if p.hollow {
            clipped.circle_stroke(c, 3.5, Stroke::new(1.3, p.color));
        } else {
            clipped.circle_filled(c, 3.5, p.color);
        }
        if let Some(h) = hover {
            let d = c.distance(h);
            if d < 10.0 && near.is_none_or(|(b, _)| d < b) {
                near = Some((d, i));
            }
        }
    }
    if let Some((_, i)) = near {
        let p = &pts[i];
        let c = to(p.x, p.y);
        painter.circle_stroke(c, 6.0, Stroke::new(1.5, fg));
        painter.text(
            Pos2::new(plot.right() - 4.0, plot.top() + 3.0),
            Align2::RIGHT_TOP,
            format!("{} · {:.3}, {:.3}", p.label, p.x, p.y),
            FontId::proportional(11.0),
            fg,
        );
    }
}

impl Diag {
    /// The window with the four graphics.
    pub fn show(&self, ui: &mut egui::Ui) {
        const BLUE: Color32 = Color32::from_rgb(0, 122, 194);
        const ORANGE: Color32 = Color32::from_rgb(232, 145, 45);
        const RED: Color32 = Color32::from_rgb(214, 69, 69);
        let grey = ui.visuals().weak_text_color();
        let n = self.rows.len();
        let w = ((ui.available_width() - 16.0) / 2.0).max(240.0);
        let size = Vec2::new(w, 250.0);
        let mark = |r: &Row| if r.flag { RED } else { BLUE };
        // 1. observed against predicted, by the fit and by the cross-validation
        let mut obs_pred: Vec<Pt> = self
            .rows
            .iter()
            .map(|r| Pt {
                x: r.fit,
                y: r.obs,
                label: format!("{} (ajuste)", r.key),
                color: mark(r),
                hollow: false,
            })
            .collect();
        obs_pred.extend(self.rows.iter().filter(|r| r.cv.is_finite()).map(|r| Pt {
            x: r.cv,
            y: r.obs,
            label: format!("{} (validación)", r.key),
            color: ORANGE,
            hollow: true,
        }));
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in &obs_pred {
            lo = lo.min(p.x.min(p.y));
            hi = hi.max(p.x.max(p.y));
        }
        // 3. normal Q–Q of the studentized residuals
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| self.rows[a].stud.total_cmp(&self.rows[b].stud));
        let qq: Vec<Pt> = order
            .iter()
            .enumerate()
            .map(|(i, &k)| Pt {
                x: normal_quantile((i as f64 + 1.0 - 0.375) / (n as f64 + 0.25)),
                y: self.rows[k].stud,
                label: self.rows[k].key.clone(),
                color: mark(&self.rows[k]),
                hollow: false,
            })
            .collect();
        let q = qq.iter().fold(2.0f64, |m, p| m.max(p.x.abs()));
        let thr = 4.0 / n as f64;
        egui::Grid::new("mo_diag")
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                chart(
                    ui,
                    size,
                    "Observado frente a estimado",
                    "estimado",
                    &format!("observado ({})", self.response),
                    &obs_pred,
                    &[Ln {
                        a: [lo, lo],
                        b: [hi, hi],
                        color: grey,
                        dashed: true,
                    }],
                    false,
                );
                let resid: Vec<Pt> = self
                    .rows
                    .iter()
                    .map(|r| Pt {
                        x: r.fit,
                        y: r.stud,
                        label: r.key.clone(),
                        color: mark(r),
                        hollow: false,
                    })
                    .collect();
                let fx = |s: f64| {
                    (
                        resid.iter().map(|p| p.x).fold(f64::INFINITY, f64::min) - s.abs(),
                        resid.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max) + s.abs(),
                    )
                };
                let (x0, x1) = fx(0.0);
                chart(
                    ui,
                    size,
                    "Residuos frente a estimados",
                    "estimado",
                    "residuo estudentizado",
                    &resid,
                    &[
                        Ln {
                            a: [x0, 0.0],
                            b: [x1, 0.0],
                            color: grey,
                            dashed: false,
                        },
                        Ln {
                            a: [x0, 2.0],
                            b: [x1, 2.0],
                            color: grey,
                            dashed: true,
                        },
                        Ln {
                            a: [x0, -2.0],
                            b: [x1, -2.0],
                            color: grey,
                            dashed: true,
                        },
                    ],
                    false,
                );
                ui.end_row();
                chart(
                    ui,
                    size,
                    "Normalidad de los residuos (Q–Q)",
                    "cuantil normal teórico",
                    "residuo estudentizado",
                    &qq,
                    &[Ln {
                        a: [-q, -q],
                        b: [q, q],
                        color: grey,
                        dashed: true,
                    }],
                    false,
                );
                let cook: Vec<Pt> = (0..n)
                    .map(|i| Pt {
                        x: i as f64 + 1.0,
                        y: self.rows[i].cook,
                        label: self.rows[i].key.clone(),
                        color: if self.rows[i].cook > thr && self.rows[i].cook > 0.5
                            || self.rows[i].cook > 1.0
                        {
                            RED
                        } else {
                            BLUE
                        },
                        hollow: false,
                    })
                    .collect();
                chart(
                    ui,
                    size,
                    "Influencia de cada parcela (Cook)",
                    "parcela (orden de la tabla)",
                    "distancia de Cook",
                    &cook,
                    &[Ln {
                        a: [0.0, 0.5],
                        b: [n as f64 + 1.0, 0.5],
                        color: grey,
                        dashed: true,
                    }],
                    true,
                );
                ui.end_row();
            });
        ui.label(
            egui::RichText::new("Los puntos sobre la línea discontinua de «observado frente a estimado» son aciertos; los círculos naranjas son la estimación por validación cruzada (sin la parcela). En residuos, la nube debe ser un manchón sin forma ni abanico y casi todo entre −2 y 2; en Q–Q, los puntos sobre la diagonal; en Cook, ninguna barra por encima de 0,5. Rojo: parcelas que la tabla avisa como atípicas, de palanca alta o influyentes. Pasa el ratón por un punto para ver su parcela.")
                .color(ui.visuals().weak_text_color())
                .size(11.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "Parcela,Observado,Ajustado,Residuo,Validacion_cruzada,Error_validacion,Palanca,Residuo_estudentizado,Cook,Aviso\n\
        1,330.0,300.0,30.0,290.0,40.0,0.10,0.9,0.05,\n\
        2,348.0,380.0,-32.0,390.0,-42.0,0.30,-1.1,0.30,palanca\n\
        3,250.0,260.0,-10.0,262.0,-12.0,0.08,-0.3,0.01,\n\
        4,415.0,330.0,85.0,320.0,95.0,0.60,2.8,1.20,atipica influyente\n\
        5,292.0,300.0,-8.0,301.0,-9.0,0.07,-0.2,0.00,\n";

    #[test]
    fn the_table_of_a_model_is_read_and_flags_are_kept() {
        let p = std::env::temp_dir().join("easylidar_diag_table.csv");
        assert!(std::fs::write(&p, TABLE).is_ok());
        let d = Diag::read(&p, "VOL");
        assert!(d.is_some());
        if let Some(d) = d {
            assert_eq!(d.rows.len(), 5);
            assert!(d.rows[3].flag && d.rows[1].flag && !d.rows[0].flag);
            assert!((d.rows[3].cook - 1.2).abs() < 1e-9);
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_four_graphics_draw_without_trouble() {
        let p = std::env::temp_dir().join("easylidar_diag_draw.csv");
        assert!(std::fs::write(&p, TABLE).is_ok());
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 800.0))),
            ..egui::RawInput::default()
        };
        if let Some(d) = Diag::read(&p, "VOL") {
            // two frames: the first one lays the fonts out
            for _ in 0..2 {
                let mut out = ctx.run_ui(raw.clone(), |ui| d.show(ui));
                // nothing paints the textures here: say so, or dropping them panics
                out.textures_delta.clear();
            }
        }
        let _ = std::fs::remove_file(&p);
    }
}
