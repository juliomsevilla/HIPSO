//! Flat SVG icons (`assets/icons`), rasterised once per size and colour into egui textures.
//!
//! Each SVG is drawn on a 24 px grid with a 1.5 px stroke. Neutral strokes use `currentColor`,
//! which is replaced by the tint requested by the caller; accents keep their own colours.
//!
//! To add an icon, drop `assets/icons/<name>.svg` (see the README there) and use `"@<name>"`;
//! `build.rs` picks it up. A name with no file falls back to `@missing`.

use eframe::egui::{
    self, Color32, ColorImage, Id, Pos2, Rect, TextureHandle, TextureOptions, Vec2,
};
use resvg::{tiny_skia, usvg};

include!(concat!(env!("OUT_DIR"), "/icons_table.rs"));

/// The icon name behind a glyph that older call sites still pass.
fn glyph_name(glyph: &str) -> Option<&'static str> {
    Some(match glyph {
        "➕" => "add",
        "ℹ" => "info",
        "🖼" => "image",
        "✖" => "close",
        "🗺" => "layers",
        "☀" => "sun",
        "🌙" => "moon",
        "📜" => "log",
        "📊" => "metrics",
        "🌲" => "segment",
        "✂" => "clip",
        "📤" => "export",
        "〰" => "contour",
        "🔍" => "search",
        "✋" => "hand",
        "❓" => "query",
        "📏" => "measure",
        "📈" => "profile",
        "🗑" => "trash",
        _ => return None,
    })
}

fn source(name: &str) -> Option<&'static str> {
    SOURCES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// The icon name for `icon` (`@name` or a known glyph), when there is an SVG for it.
pub fn resolve(icon: &str) -> Option<&'static str> {
    let name = match icon.strip_prefix('@') {
        Some(n) => n,
        None => glyph_name(icon)?,
    };
    SOURCES.iter().find(|(n, _)| *n == name).map(|(n, _)| *n)
}

fn rasterise(svg: &str, px: u32, tint: Color32) -> Option<ColorImage> {
    let hex = format!("#{:02x}{:02x}{:02x}", tint.r(), tint.g(), tint.b());
    let svg = svg.replace("currentColor", &hex);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let k = px as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(k, k),
        &mut pixmap.as_mut(),
    );
    Some(ColorImage::from_rgba_premultiplied(
        [px as usize, px as usize],
        pixmap.data(),
    ))
}

/// Paints the icon `name` centred on `c` in a `size` x `size` box; false when it cannot.
pub fn paint(p: &egui::Painter, c: Pos2, size: f32, name: &str, tint: Color32) -> bool {
    let Some(svg) = source(name) else {
        return false;
    };
    let ctx = p.ctx();
    let px = (size * ctx.pixels_per_point()).round().clamp(8.0, 256.0) as u32;
    let id = Id::new(("svg-icon", name, px, tint.to_array()));
    let tex = match ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        Some(t) => t,
        None => {
            let Some(img) = rasterise(svg, px, tint) else {
                return false;
            };
            let t = ctx.load_texture(format!("icon-{name}"), img, TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, t.clone()));
            t
        }
    };
    let rect = Rect::from_center_size(c, Vec2::splat(size));
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    p.image(tex.id(), rect, uv, Color32::WHITE);
    true
}
