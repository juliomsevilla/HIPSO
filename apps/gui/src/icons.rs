//! Iconos SVG planos (`assets/icons`), rasterizados una vez por tamaño y color en texturas de egui.
//!
//! Cada SVG se dibuja en una cuadrícula de 24 px con un trazo de 1,5 px. Los trazos neutros usan
//! `currentColor`, que se sustituye por el tinte que pide quien llama; los acentos conservan sus
//! propios colores.
//!
//! Para añadir un icono, déjese `assets/icons/<nombre>.svg` (véase el LÉEME de allí) y úsese
//! `"@<nombre>"`; `build.rs` lo recoge. Un nombre sin archivo recurre a `@missing`.

use eframe::egui::{
    self, Color32, ColorImage, Id, Pos2, Rect, TextureHandle, TextureOptions, Vec2,
};
use resvg::{tiny_skia, usvg};

include!(concat!(env!("OUT_DIR"), "/icons_table.rs"));

/// El nombre de icono que hay detrás de un glifo que todavía pasan lugares de llamada antiguos.
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

/// El nombre de icono para `icon` (`@nombre` o un glifo conocido), cuando hay un SVG para él.
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

/// Pinta el icono `name` centrado en `c` en una caja de `size` x `size`; falso cuando no puede.
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
