//! Big vector layers drawn as one picture. Millions of vertices as egui shapes or meshes cost
//! gigabytes in the renderer and seconds per frame; the same layer rasterised on the CPU into a
//! texture costs the size of the screen, whatever the number of vertices. While the view moves
//! the last picture is shifted and scaled (as a raster layer is), and it is drawn again once the
//! view has been still for a moment.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use resvg::tiny_skia::{self, FillRule, Paint, PathBuilder, Pixmap, Transform};

use super::*;
use crate::layers::VecLayer;
use crate::symbology::SymKind;

/// Above this many vertices a layer is drawn as a picture.
pub const HEAVY_VERTICES: usize = 150_000;
/// Above this many features too: a point layer has one vertex each.
pub const HEAVY_FEATURES: usize = 4_000;
/// How long the view must be still before the picture is made again.
const SETTLE: Duration = Duration::from_millis(150);

/// The picture of a layer and what it was made for.
pub struct Cache {
    key: u64,
    /// The view the picture shows, and the size it was made for.
    plan: Plan,
    size: (usize, usize),
    tex: Option<egui::TextureHandle>,
    /// The view of the last frame and since when it has not changed.
    seen: (u64, u64, u64, usize, usize),
    changed: Instant,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            key: 0,
            plan: Plan {
                cx: 0.0,
                cy: 0.0,
                scale: 1.0,
            },
            size: (0, 0),
            tex: None,
            seen: (0, 0, 0, 0, 0),
            changed: Instant::now(),
        }
    }
}

impl VecLayer {
    pub fn is_heavy(&self) -> bool {
        self.geom.nverts > HEAVY_VERTICES || self.data.features.len() > HEAVY_FEATURES
    }

    /// What the picture depends on besides the view.
    fn style_key(&self, opacity: f32) -> u64 {
        hash_of((
            self.rev,
            opacity.to_bits(),
            self.color,
            self.fill,
            self.fill_alpha.to_bits(),
            self.width.to_bits(),
            self.point_size.to_bits(),
            std::mem::discriminant(&self.sym.kind),
            self.sym.field,
            &self.sym.colors,
            self.sym.breaks.len(),
        ))
    }
}

/// The layer in the view `plan` of a `w`×`h` window, `ppp` pixels per point.
fn render(
    v: &VecLayer,
    plan: Plan,
    w: usize,
    h: usize,
    ppp: f32,
    opacity: f32,
) -> Option<egui::ColorImage> {
    let (wp, hp) = (
        (w as f32 * ppp).ceil() as u32,
        (h as f32 * ppp).ceil() as u32,
    );
    let mut pm = Pixmap::new(wp.max(1), hp.max(1))?;
    let (lo, hi) = (
        plan.to_map(0.0, h as f64, w, h),
        plan.to_map(w as f64, 0.0, w, h),
    );
    let pad = 4.0 / plan.scale;
    let visible = v.in_window([lo.0 - pad, lo.1 - pad, hi.0 + pad, hi.1 + pad]);
    let k = f64::from(ppp);
    let (half_w, half_h) = (w as f64 / 2.0, h as f64 / 2.0);
    let px = |p: &[f64; 2]| {
        (
            (((p[0] - plan.cx) * plan.scale + half_w) * k) as f32,
            (((plan.cy - p[1]) * plan.scale + half_h) * k) as f32,
        )
    };
    let polygons = v.kind == view::GeomKind::Polygon;
    let a = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    let stroke = tiny_skia::Stroke {
        width: (v.width * ppp).max(1.0),
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..Default::default()
    };
    // vertices closer than this (squared, in pixels) to the last one drawn are left out
    let min_d2 = 0.6 * ppp * ppp;
    let mut dots: HashSet<(i32, i32)> = HashSet::new();
    for &i in &visible {
        let f = &v.data.features[i];
        let c = v.sym.color_of(&v.data, i, v.color);
        let fc = v.sym.color_of(&v.data, i, v.fill);
        let b = &v.geom.bbox[i];
        let size_px = (b[2] - b[0]).max(b[3] - b[1]) * plan.scale;
        let mut paint = Paint {
            anti_alias: true,
            ..Default::default()
        };
        if v.kind == view::GeomKind::Point {
            let (x, y) = match f.parts.first().and_then(|p| p.first()) {
                Some(p) => px(p),
                None => continue,
            };
            // one dot per 2 px square: points on top of each other would only cost time
            if !dots.insert(((x / (2.0 * ppp)) as i32, (y / (2.0 * ppp)) as i32)) {
                continue;
            }
            let r = v.sym.size_of(&v.data, i, v.point_size) * ppp;
            if let Some(p) = PathBuilder::from_circle(x, y, r) {
                paint.set_color_rgba8(c[0], c[1], c[2], a(opacity));
                pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
                paint.set_color_rgba8(0, 0, 0, a(0.6 * opacity));
                pm.stroke_path(
                    &p,
                    &paint,
                    &tiny_skia::Stroke {
                        width: ppp,
                        ..Default::default()
                    },
                    Transform::identity(),
                    None,
                );
            }
            continue;
        }
        if size_px < 1.0 {
            // a feature under a pixel is a dot
            let (x, y) = px(&[(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]);
            if dots.insert(((x / ppp) as i32, (y / ppp) as i32)) {
                paint.set_color_rgba8(c[0], c[1], c[2], a(opacity));
                if let Some(r) =
                    tiny_skia::Rect::from_xywh(x - 0.6 * ppp, y - 0.6 * ppp, 1.2 * ppp, 1.2 * ppp)
                {
                    pm.fill_rect(r, &paint, Transform::identity(), None);
                }
            }
            continue;
        }
        let mut pb = PathBuilder::new();
        for part in &f.parts {
            if part.len() < 2 {
                continue;
            }
            let mut last = px(&part[0]);
            pb.move_to(last.0, last.1);
            for (n, q) in part.iter().enumerate().skip(1) {
                let s = px(q);
                let d = (s.0 - last.0).powi(2) + (s.1 - last.1).powi(2);
                if d >= min_d2 || n + 1 == part.len() {
                    pb.line_to(s.0, s.1);
                    last = s;
                }
            }
            if polygons {
                pb.close();
            }
        }
        let Some(path) = pb.finish() else { continue };
        if polygons && v.fill_alpha > 0.0 {
            paint.set_color_rgba8(fc[0], fc[1], fc[2], a(v.fill_alpha * opacity));
            pm.fill_path(
                &path,
                &paint,
                FillRule::EvenOdd,
                Transform::identity(),
                None,
            );
        }
        if !polygons || v.width > 0.0 {
            let line = if polygons && v.sym.kind != SymKind::Single {
                [c[0] / 2, c[1] / 2, c[2] / 2]
            } else {
                c
            };
            paint.set_color_rgba8(
                line[0],
                line[1],
                line[2],
                a(opacity.max(if polygons { 0.6 } else { 0.0 })),
            );
            pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
    Some(egui::ColorImage::from_rgba_premultiplied(
        [wp.max(1) as usize, hp.max(1) as usize],
        pm.data(),
    ))
}

impl App {
    /// Draws a heavy layer as its picture, making it again when the view has settled.
    pub fn paint_raster(
        &self,
        clip: &egui::Painter,
        v: &VecLayer,
        rect: Rect,
        w: usize,
        h: usize,
        opacity: f32,
    ) {
        let ctx = clip.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let cur = (
            self.plan.cx.to_bits(),
            self.plan.cy.to_bits(),
            self.plan.scale.to_bits(),
            w,
            h,
        );
        let key = hash_of((cur, v.style_key(opacity)));
        let Ok(mut c) = v.raster.lock() else { return };
        if c.seen != cur {
            c.seen = cur;
            c.changed = Instant::now();
        }
        if c.key != key || c.tex.is_none() {
            let settled = c.changed.elapsed() >= SETTLE;
            if c.tex.is_none() || settled {
                if let Some(img) = render(v, self.plan, w, h, ppp, opacity) {
                    match &mut c.tex {
                        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                        None => {
                            c.tex = Some(ctx.load_texture(
                                "vector-layer",
                                img,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                    }
                    c.key = key;
                    c.plan = self.plan;
                    c.size = (w, h);
                }
            } else {
                ctx.request_repaint_after(SETTLE);
            }
        }
        let Some(tex) = &c.tex else { return };
        // where the picture lies in the current view: its corners, mapped from the view it was
        // made for to the one on screen
        let (pw, ph) = c.size;
        let (tl, br) = (
            c.plan.to_map(0.0, 0.0, pw, ph),
            c.plan.to_map(pw as f64, ph as f64, pw, ph),
        );
        let to_screen = |p: (f64, f64)| {
            let s = self.plan.to_screen(p.0, p.1, w, h);
            Pos2::new(s.0 as f32, s.1 as f32) + rect.min.to_vec2()
        };
        let img = Rect::from_two_pos(to_screen(tl), to_screen(br));
        clip.image(
            tex.id(),
            img,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}
