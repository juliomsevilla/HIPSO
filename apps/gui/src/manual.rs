//! El manual de usuario, mostrado dentro de la aplicación (vista Archivo → Manual, o F1). El texto
//! es `docs/MANUAL.md`, dibujado por un pequeño visualizador del Markdown que usa: encabezados,
//! listas, tablas, bloques de código, negrita y código en línea.

use eframe::egui::{self, Align, Color32, FontId, RichText, text::LayoutJob};

pub(crate) const TEXT: &str = include_str!("../../../docs/MANUAL.md");

#[derive(Default)]
pub struct Manual {
    goto: Option<usize>,
    filter: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Plain,
    Bold,
    Italic,
    Code,
}

/// Las piezas de una línea: `**negrita**`, `*cursiva*` y `` `código` `` aparte del texto normal. Un
/// `*` que no abre ni cierra un énfasis (un signo de producto, `* 100`) se queda como está.
fn spans(text: &str) -> Vec<(Style, &str)> {
    let mut out = Vec::new();
    let mut plain_from = 0;
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let found = if let Some(inner) = rest.strip_prefix("**") {
            inner
                .find("**")
                .filter(|e| *e > 0)
                .map(|e| (Style::Bold, 2, e))
        } else if let Some(inner) = rest.strip_prefix('`') {
            inner.find('`').map(|e| (Style::Code, 1, e))
        } else if let Some(inner) = rest.strip_prefix('*') {
            // la estrella de apertura toca una palabra, la de cierre también
            let opens = inner
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace() && c != '*');
            let close = inner
                .char_indices()
                .find(|(k, c)| *c == '*' && *k > 0 && !inner[..*k].ends_with(char::is_whitespace));
            close.filter(|_| opens).map(|(e, _)| (Style::Italic, 1, e))
        } else {
            None
        };
        match found {
            Some((style, open, len)) => {
                if at > plain_from {
                    out.push((Style::Plain, &text[plain_from..at]));
                }
                out.push((style, &rest[open..open + len]));
                at += open + len + open;
                plain_from = at;
            }
            None => at += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    if text.len() > plain_from {
        out.push((Style::Plain, &text[plain_from..]));
    }
    out
}

/// Texto en línea con negrita, cursiva y código como un solo bloque.
fn inline(ui: &egui::Ui, text: &str, size: f32) -> LayoutJob {
    let visuals = ui.visuals();
    let (body, strong, code_bg) = (
        visuals.text_color(),
        visuals.strong_text_color(),
        visuals.code_bg_color,
    );
    let mut job = LayoutJob::default();
    for (style, s) in spans(text) {
        let mut f = plain(
            if style == Style::Bold { strong } else { body },
            if style == Style::Code {
                size - 1.0
            } else {
                size
            },
        );
        match style {
            Style::Italic => f.italics = true,
            Style::Code => {
                f.font_id = FontId::monospace(size - 1.0);
                f.background = code_bg;
            }
            _ => {}
        }
        job.append(s, 0.0, f);
    }
    job
}

fn plain(color: Color32, size: f32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: FontId::proportional(size),
        color,
        ..Default::default()
    }
}

fn cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

/// El ancho de cada columna de una tabla para que el conjunto quepa en `ui.available_width()`: una
/// columna conserva el ancho de su celda más larga cuando hay sitio para todas; si no, las columnas
/// que necesitan menos que una parte igual conservan lo que necesitan y el resto se reparte lo que
/// queda en proporción a lo que necesitan (su texto se ajusta).
fn column_widths(ui: &egui::Ui, rows: &[Vec<&str>], size: f32, gap: f32) -> Vec<f32> {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return Vec::new();
    }
    let mut need = vec![40.0f32; cols];
    for row in rows {
        for (ci, c) in row.iter().enumerate() {
            let galley = ui.painter().layout_job(inline(ui, c, size));
            need[ci] = need[ci].max(galley.size().x + 4.0);
        }
    }
    // las franjas de la cuadrícula y la barra de desplazamiento ocupan algo más que los huecos
    let room = (ui.available_width() - gap * (cols as f32 - 1.0) - 16.0).max(80.0 * cols as f32);
    if need.iter().sum::<f32>() <= room {
        return need;
    }
    let mut fixed = vec![false; cols];
    let mut left = room;
    loop {
        let open: Vec<usize> = (0..cols).filter(|c| !fixed[*c]).collect();
        if open.is_empty() {
            break;
        }
        let share = left / open.len() as f32;
        let small: Vec<usize> = open.iter().copied().filter(|c| need[*c] <= share).collect();
        if small.is_empty() {
            let total: f32 = open.iter().map(|c| need[*c]).sum();
            for c in open {
                need[c] = (left * need[c] / total).max(60.0);
            }
            break;
        }
        for c in small {
            fixed[c] = true;
            left -= need[c];
        }
    }
    need
}

impl Manual {
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let lines: Vec<&str> = TEXT.lines().collect();
        let heads: Vec<(usize, &str)> = lines
            .iter()
            .enumerate()
            .filter_map(|(k, l)| l.strip_prefix("## ").map(|t| (k, t)))
            .collect();
        {
            ui.horizontal_wrapped(|ui| {
                ui.label("Ir a");
                egui::ComboBox::from_id_salt("manual_toc")
                    .width(300.0)
                    .selected_text("Capítulos")
                    .show_ui(ui, |ui| {
                        for (k, (_, t)) in heads.iter().enumerate() {
                            if ui.selectable_label(false, *t).clicked() {
                                self.goto = Some(k);
                            }
                        }
                    });
                ui.label("Buscar");
                ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(160.0));
            });
            ui.separator();
            let needle = self.filter.trim().to_lowercase();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| render(ui, &lines, &mut self.goto, &needle, "manual_table"));
        }
    }
}

/// Dibuja un texto en Markdown (el del manual o la respuesta de un asistente): encabezados,
/// listas, tablas, bloques de código, negrita y código en línea. `goto` es el capítulo al que
/// desplazarse (solo el manual lo usa) y `needle` el texto que filtra las líneas; `salt`
/// distingue las tablas de un texto de las de otro.
pub fn render(ui: &mut egui::Ui, lines: &[&str], goto: &mut Option<usize>, needle: &str, salt: &'static str) {
        let mut k = 0;
        let mut head_no = 0;
        let mut in_code = false;
        while k < lines.len() {
            let l = lines[k];
            if l.trim_start().starts_with("```") {
                in_code = !in_code;
                k += 1;
                continue;
            }
            if in_code {
                // una orden larga se ajusta en el borde de la ventana en lugar de
                // salirse
                ui.add(egui::Label::new(RichText::new(l).monospace().size(12.0)).wrap());
                k += 1;
                continue;
            }
            if l.starts_with('|') {
                let start = k;
                while k < lines.len() && lines[k].starts_with('|') {
                    k += 1;
                }
                let rows: Vec<Vec<&str>> = lines[start..k]
                    .iter()
                    .filter(|r| !r.contains("---"))
                    .map(|r| cells(r))
                    .collect();
                // las columnas comparten el ancho de la ventana: una celda se ajusta en
                // su columna y una tabla nunca se sale del borde
                let gap = 14.0;
                let widths = column_widths(ui, &rows, 13.0, gap);
                egui::Grid::new((salt, start))
                    .striped(true)
                    .spacing([gap, 4.0])
                    .show(ui, |ui| {
                        for (ri, row) in rows.iter().enumerate() {
                            for (ci, c) in row.iter().enumerate() {
                                let mut job = inline(ui, c, 13.0);
                                if ri == 0 {
                                    for s in &mut job.sections {
                                        s.format.color =
                                            ui.visuals().strong_text_color();
                                    }
                                }
                                let w = widths.get(ci).copied().unwrap_or(120.0);
                                ui.scope(|ui| {
                                    ui.set_max_width(w);
                                    ui.add(egui::Label::new(job).wrap());
                                });
                            }
                            ui.end_row();
                        }
                    });
                ui.add_space(4.0);
                continue;
            }
            let mut shown = true;
            if !needle.is_empty() && !l.to_lowercase().contains(&needle) {
                shown = l.starts_with('#');
            }
            if let Some(t) = l.strip_prefix("# ") {
                ui.add_space(4.0);
                ui.label(RichText::new(t).heading().strong());
            } else if let Some(t) = l.strip_prefix("## ") {
                ui.add_space(12.0);
                let r = ui.label(
                    RichText::new(t)
                        .size(19.0)
                        .strong()
                        .color(Color32::from_rgb(0xFF, 0x8A, 0x1F)),
                );
                if *goto == Some(head_no) {
                    r.scroll_to_me(Some(Align::TOP));
                    *goto = None;
                }
                head_no += 1;
            } else if let Some(t) = l.strip_prefix("#### ") {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(t)
                        .size(14.0)
                        .strong()
                        .color(Color32::from_rgb(0xFF, 0x8A, 0x1F)),
                );
            } else if let Some(t) = l.strip_prefix("### ") {
                ui.add_space(6.0);
                ui.label(RichText::new(t).size(15.5).strong());
            } else if l.trim() == "---" {
                ui.separator();
            } else if l.trim().is_empty() {
                ui.add_space(3.0);
            } else if shown {
                let indent = l.len() - l.trim_start().len();
                let t = l.trim_start();
                let item =
                    t.strip_prefix("- ")
                        .map(|r| ("•".to_string(), r))
                        .or_else(|| {
                            let n = t.find(". ")?;
                            t[..n]
                                .chars()
                                .all(|c| c.is_ascii_digit())
                                .then(|| (format!("{}.", &t[..n]), &t[n + 2..]))
                        });
                match item {
                    Some((mark, text)) => {
                        ui.horizontal_top(|ui| {
                            ui.add_space(8.0 + indent as f32 * 6.0);
                            ui.label(mark);
                            let job = inline(ui, text, 13.5);
                            ui.add(egui::Label::new(job).wrap());
                        });
                    }
                    None => {
                        let job = inline(ui, t, 13.5);
                        ui.add(egui::Label::new(job).wrap());
                    }
                }
            }
            k += 1;
        }

}

/// Dibuja un texto en Markdown en el ancho disponible.
pub fn markdown(ui: &mut egui::Ui, text: &str, salt: &'static str) {
    let lines: Vec<&str> = text.lines().collect();
    render(ui, &lines, &mut None, "", salt);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_loses_its_stars_and_products_keep_them() {
        assert_eq!(
            spans("a **b** *c* `d * e` f"),
            vec![
                (Style::Plain, "a "),
                (Style::Bold, "b"),
                (Style::Plain, " "),
                (Style::Italic, "c"),
                (Style::Plain, " "),
                (Style::Code, "d * e"),
                (Style::Plain, " f"),
            ]
        );
        assert_eq!(
            spans("(n) * 100 y 2 * 3"),
            vec![(Style::Plain, "(n) * 100 y 2 * 3")]
        );
        assert_eq!(
            spans("**TIN** *(predeterminado)*:"),
            vec![
                (Style::Bold, "TIN"),
                (Style::Plain, " "),
                (Style::Italic, "(predeterminado)"),
                (Style::Plain, ":"),
            ]
        );
    }
}
