//! The user manual, shown inside the app (Ayuda → Manual, or F1). The text is `docs/MANUAL.md`,
//! drawn by a small renderer of the Markdown it uses: headings, lists, tables, code blocks,
//! bold and inline code.

use eframe::egui::{self, Align, Color32, FontId, RichText, text::LayoutJob};

const TEXT: &str = include_str!("../../../docs/MANUAL.md");

#[derive(Default)]
pub struct Manual {
    pub open: bool,
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

/// The pieces of a line: `**bold**`, `*italic*` and `` `code` `` apart from plain text. A `*` that
/// does not open and close an emphasis (a product sign, `* 100`) stays as it is.
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
            // the opening star touches a word, the closing one too
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

/// Inline text with bold, italic and code as one galley.
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

impl Manual {
    pub fn window(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = true;
        let lines: Vec<&str> = TEXT.lines().collect();
        let heads: Vec<(usize, &str)> = lines
            .iter()
            .enumerate()
            .filter_map(|(k, l)| l.strip_prefix("## ").map(|t| (k, t)))
            .collect();
        egui::Window::new("Manual de uso")
            .open(&mut open)
            .default_size([860.0, 640.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
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
                    .show(ui, |ui| {
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
                                ui.label(RichText::new(l).monospace().size(12.0));
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
                                egui::Grid::new(("manual_table", start))
                                    .striped(true)
                                    .spacing([14.0, 4.0])
                                    .show(ui, |ui| {
                                        for (ri, row) in rows.iter().enumerate() {
                                            for c in row {
                                                let mut job = inline(ui, c, 13.0);
                                                if ri == 0 {
                                                    for s in &mut job.sections {
                                                        s.format.color =
                                                            ui.visuals().strong_text_color();
                                                    }
                                                }
                                                ui.label(job);
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
                                if self.goto == Some(head_no) {
                                    r.scroll_to_me(Some(Align::TOP));
                                    self.goto = None;
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
                                let item = t
                                    .strip_prefix("- ")
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
                    });
            });
        self.open = open;
    }
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
