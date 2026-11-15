//! El nombre, el descriptor y el eslogan, y las piezas de la marca dibujadas en la interfaz
//! (`assets/brand`). Cambiar el nombre del programa en pantalla es cuestión de cambiar `NAME`.

use eframe::egui::{self, ColorImage, Id, TextureHandle, TextureOptions, Ui};
use resvg::{tiny_skia, usvg};

pub const NAME: &str = "HIPSO";
pub const DESCRIPTOR: &str = "Procesamiento y edición de nubes de puntos";
pub const CLAIM: &str = "Visualiza. Edita. Procesa.";

/// Lo que dibuja la interfaz de la marca.
#[derive(Clone, Copy)]
pub enum Art {
    /// La nube de puntos naranja sobre su rejilla: el logotipo del programa.
    Logo,
    /// HIPSO escrito: dondequiera que aparece el nombre del programa.
    Wordmark,
}

/// La H sobre grafito: icono de la ventana y de la barra de tareas.
const ICON_H: &[u8] = include_bytes!("../../../assets/icon-256.png");

/// El SVG de una pieza: líneas de grafito sobre superficies claras, blancas sobre superficies
/// oscuras.
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

/// Dibuja una pieza de la marca de `height` puntos de alto (su ancho la sigue), nítida a cualquier
/// escala.
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

/// El icono de la ventana y de la barra de tareas.
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
