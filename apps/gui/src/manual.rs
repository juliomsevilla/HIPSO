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

/// Inline text with `**bold**` and `` `code` `` as one galley.
fn inline(ui: &egui::Ui, text: &str, size: f32) -> LayoutJob {
    let visuals = ui.visuals();
    let (body, strong, code_bg) = (
        visuals.text_color(),
        visuals.strong_text_color(),
        visuals.code_bg_color,
    );
    let mut job = LayoutJob::default();
    let mut rest = text;
    while !rest.is_empty() {
        let (b, c) = (rest.find("**"), rest.find('`'));
        let next = match (b, c) {
            (Some(b), Some(c)) => b.min(c),
            (Some(b), None) => b,
            (None, Some(c)) => c,
            (None, None) => {
                job.append(rest, 0.0, plain(body, size));
                break;
            }
        };
        if next > 0 {
            job.append(&rest[..next], 0.0, plain(body, size));
        }
        let tail = &rest[next..];
        if let Some(inner) = tail.strip_prefix("**")
            && let Some(end) = inner.find("**")
        {
            job.append(&inner[..end], 0.0, plain(strong, size));
            rest = &inner[end + 2..];
        } else if let Some(inner) = tail.strip_prefix('`')
            && let Some(end) = inner.find('`')
        {
            let mut f = plain(body, size - 1.0);
            f.font_id = FontId::monospace(size - 1.0);
            f.background = code_bg;
            job.append(&inner[..end], 0.0, f);
            rest = &inner[end + 1..];
        } else {
            job.append(&tail[..1], 0.0, plain(body, size));
            rest = &tail[1..];
        }
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
