//! Colours, spacing and fonts: dark and light looks of EasyLiDAR.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Vec2,
};

pub const BRAND: Color32 = Color32::from_rgb(0xB8, 0x4A, 0x2C);
pub const BRAND_HOVER: Color32 = Color32::from_rgb(0x96, 0x3A, 0x22);

#[derive(Clone, Copy)]
pub struct Tokens {
    pub header: Color32,
    pub ribbon: Color32,
    pub canvas: Color32,
    pub pane: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub dark: bool,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const DARK: Tokens = Tokens {
    header: rgb(0x171B21),
    ribbon: rgb(0x242931),
    canvas: rgb(0x12151A),
    pane: rgb(0x1E232A),
    border: rgb(0x343B45),
    text: rgb(0xE6EAEF),
    weak: rgb(0x98A1AD),
    accent: rgb(0x5B8DF6),
    selection: rgb(0x26406E),
    hover: rgb(0x2E3540),
    dark: true,
};

pub const LIGHT: Tokens = Tokens {
    header: rgb(0xE6E9EE),
    ribbon: rgb(0xF6F7F9),
    canvas: rgb(0xDDE1E7),
    pane: rgb(0xF0F2F5),
    border: rgb(0xCDD3DB),
    text: rgb(0x1F2430),
    weak: rgb(0x6A7280),
    accent: rgb(0x3B6FE0),
    selection: rgb(0xCADBFB),
    hover: rgb(0xE4E8EE),
    dark: false,
};

pub fn apply(ctx: &egui::Context, t: &Tokens) {
    let mut v = if t.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = t.pane;
    v.window_fill = t.pane;
    v.extreme_bg_color = t.canvas;
    v.faint_bg_color = t.ribbon;
    v.override_text_color = Some(t.text);
    v.hyperlink_color = t.accent;
    v.selection.bg_fill = t.selection;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    let r = CornerRadius::same(4);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.inactive.bg_fill = t.ribbon;
    v.widgets.inactive.weak_bg_fill = t.ribbon;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.accent);
    v.widgets.active.bg_fill = t.selection;
    v.widgets.active.weak_bg_fill = t.selection;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(6.0, 5.0);
        s.spacing.button_padding = Vec2::new(8.0, 3.0);
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(12.5));
        s.text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(12.5));
        s.text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
        s.text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));
        s.text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(16.0));
    });
}

/// Segoe UI (Windows), SF/Helvetica (macOS) or Noto/DejaVu (Linux) in front of egui's own
/// fonts, which stay as the fallback for symbols.
pub fn install_fonts(ctx: &egui::Context) {
    let candidates = [
        "C:/Windows/Fonts/segoeui.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    let mut fonts = FontDefinitions::default();
    if let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert(
            "system".into(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "system".into());
        }
    }
    // Segoe UI has no ▾ or ▶ (they drew as a square); the symbol fonts of the system do
    let symbols = [
        "C:/Windows/Fonts/seguisym.ttf",
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    if let Some(bytes) = symbols.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert(
            "symbols".into(),
            std::sync::Arc::new(FontData::from_owned(bytes)),
        );
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&fam) {
                list.push("symbols".into());
            }
        }
    }
    ctx.set_fonts(fonts);
}
