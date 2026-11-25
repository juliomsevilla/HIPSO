//! Texto con halo blanco para lo que se pinta sobre la nube de puntos: se lee con puntos de
//! cualquier color detrás, también con el tema oscuro.

use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Vec2};

/// El color del texto: el pedido si ya es oscuro y, si no, el mismo más oscuro, porque el halo es
/// blanco y un texto claro no se leería sobre él.
fn legible(c: Color32) -> Color32 {
    let luz = (0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b())) / 255.0;
    if luz < 0.5 {
        return c;
    }
    let k = 0.38;
    Color32::from_rgb(
        (f32::from(c.r()) * k) as u8,
        (f32::from(c.g()) * k) as u8,
        (f32::from(c.b()) * k) as u8,
    )
}

/// Pinta `texto` con un halo blanco alrededor. Devuelve el rectángulo del texto.
pub fn text(
    painter: &Painter,
    pos: Pos2,
    anchor: Align2,
    texto: impl ToString,
    font: FontId,
    color: Color32,
) -> Rect {
    let texto = texto.to_string();
    for (dx, dy) in [
        (-1.6, 0.0),
        (1.6, 0.0),
        (0.0, -1.6),
        (0.0, 1.6),
        (-1.2, -1.2),
        (1.2, -1.2),
        (-1.2, 1.2),
        (1.2, 1.2),
    ] {
        painter.text(
            pos + Vec2::new(dx, dy),
            anchor,
            &texto,
            font.clone(),
            Color32::WHITE,
        );
    }
    painter.text(pos, anchor, texto, font, legible(color))
}

/// Un cuadrado de color con un borde blanco, para que no se confunda con los puntos de detrás.
pub fn swatch(painter: &Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect.expand(2.0), 3.0, Color32::WHITE);
    painter.rect_filled(rect, 2.0, color);
}
