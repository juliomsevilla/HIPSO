//! Pictures whose coordinate system is not the work's, drawn as a grid of small triangles whose
//! corners are moved to where they belong: straight edges become the curves they should be.

use eframe::egui::{self, Color32, Pos2, Rect};
use rustlidar_core::reproject::Transformer;
use rustlidar_core::view::Plan;

/// Paints the texture of an image that covers `extent` (min x, min y, max x, max y) in the
/// system `tf` converts from, in the system of the plan. `n` is how many cells the grid has along
/// each side: more cells follow the curvature better.
#[allow(clippy::too_many_arguments)]
pub fn paint_warped(
    painter: &egui::Painter,
    tex: egui::TextureId,
    extent: [f64; 4],
    tf: &Transformer,
    plan: &Plan,
    rect: Rect,
    w: usize,
    h: usize,
    tint: Color32,
    n: usize,
) {
    let n = n.max(1);
    let clip = painter.with_clip_rect(rect);
    let mut mesh = egui::Mesh::with_texture(tex);
    let side = n + 1;
    let mut ok = vec![false; side * side];
    for j in 0..side {
        for i in 0..side {
            let (fx, fy) = (i as f64 / n as f64, j as f64 / n as f64);
            let (x, y) = (
                extent[0] + (extent[2] - extent[0]) * fx,
                extent[3] - (extent[3] - extent[1]) * fy,
            );
            let (pos, good) = match tf.transform(x, y) {
                Ok((mx, my)) => {
                    let (sx, sy) = plan.to_screen(mx, my, w, h);
                    (
                        Pos2::new(sx as f32, sy as f32) + rect.min.to_vec2(),
                        sx.is_finite() && sy.is_finite(),
                    )
                }
                Err(_) => (Pos2::ZERO, false),
            };
            ok[j * side + i] = good;
            mesh.vertices.push(egui::epaint::Vertex {
                pos,
                uv: Pos2::new(fx as f32, fy as f32),
                color: tint,
            });
        }
    }
    for j in 0..n {
        for i in 0..n {
            let a = (j * side + i) as u32;
            let (b, c, d) = (a + 1, a + side as u32, a + side as u32 + 1);
            if ok[j * side + i]
                && ok[j * side + i + 1]
                && ok[(j + 1) * side + i]
                && ok[(j + 1) * side + i + 1]
            {
                mesh.indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
    }
    if !mesh.indices.is_empty() {
        clip.add(egui::Shape::mesh(mesh));
    }
}
