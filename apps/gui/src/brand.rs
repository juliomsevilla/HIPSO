//! The name, the descriptor and the claim, and the pieces of the brand drawn in the interface
//! (`assets/brand`). Renaming the program on screen is a matter of changing `NAME`; the folders of
//! the preferences and of the caches keep their old name on purpose.

use eframe::egui::{self, ColorImage, Id, TextureHandle, TextureOptions, Ui};
use resvg::{tiny_skia, usvg};

pub const NAME: &str = "HIPSO";
pub const DESCRIPTOR: &str = "Procesamiento y edición de nubes de puntos";
pub const CLAIM: &str = "Visualiza. Edita. Procesa.";

/// What the interface draws of the brand.
#[derive(Clone, Copy)]
pub enum Art {
    /// The orange point cloud over its grid: the logo of the program.
    Logo,
    /// HIPSO written out: wherever the name of the program appears.
    Wordmark,
}

/// The H on graphite: icon of the window and of the task bar.
const ICON_H: &[u8] = include_bytes!("../../../assets/icon-256.png");

/// The SVG of a piece: graphite lines on light surfaces, white ones on dark surfaces.
fn source(art: Art, dark: bool) -> &'static str {
    match (art, dark) {
        (Art::Logo, false) => include_str!("../../../assets/brand/hipso-logo.svg"),
        (Art::Logo, true) => include_str!("../../../assets/brand/hipso-logo-blanco.svg"),
        (Art::Wordmark, false) => include_str!("../../../assets/brand/hipso-wordmark.svg"),
        (Art::Wordmark, true) => include_str!("../../../assets/brand/hipso-wordmark-blanco.svg"),
    }
}

fn rasterise(svg: &str, height_px: u32) -> Option<ColorImage> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let k = height_px as f32 / tree.size().height();
    let width = (tree.size().width() * k).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height_px)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(k, k),
        &mut pixmap.as_mut(),
    );
    Some(ColorImage::from_rgba_premultiplied(
        [width as usize, height_px as usize],
        pixmap.data(),
    ))
}

/// Draws a piece of the brand `height` points tall (its width follows), sharp at any scale.
pub fn paint(ui: &mut Ui, art: Art, dark: bool, height: f32) {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let px = (height * ppp).round().clamp(8.0, 2048.0) as u32;
    let id = Id::new(("brand-art", art as u8, dark, px));
    let tex = match ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        Some(t) => t,
        None => {
            let Some(img) = rasterise(source(art, dark), px) else {
                return;
            };
            let t = ctx.load_texture("brand-art", img, TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, t.clone()));
            t
        }
    };
    ui.image((tex.id(), tex.size_vec2() / ppp));
}

/// The icon of the window and of the task bar.
pub fn app_icon() -> egui::IconData {
    let decoded = (|| {
        let mut r = png::Decoder::new(std::io::Cursor::new(ICON_H))
            .read_info()
            .ok()?;
        let mut buf = vec![0; r.output_buffer_size()?];
        let info = r.next_frame(&mut buf).ok()?;
        buf.truncate(info.buffer_size());
        (info.color_type == png::ColorType::Rgba).then_some((buf, info.width, info.height))
    })();
    decoded.map_or_else(Default::default, |(rgba, width, height)| egui::IconData {
        rgba,
        width,
        height,
    })
}
