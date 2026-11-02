//! The "Archivo" view, in the style of an office suite's backstage: it covers the work area with
//! the file commands on the left (open, projects, information, close) and the recent files on the
//! right, so that everything about files is in one place.

use super::*;

/// What the right side of the view shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// The recent files.
    Home,
    About,
    Manual,
    Shortcuts,
}

/// What the user chose in the view.
enum Pick {
    Act(Action),
    Reopen(PathBuf),
    ClearRecent,
    Back,
    Page(Page),
}

fn is_project(p: &Path) -> bool {
    p.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(project::EXT))
}

/// A command of the left column: a full-width row that lights up under the pointer.
fn command(ui: &mut egui::Ui, t: &theme::Tokens, glyph: &str, label: &str) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let y = rect.center().y;
    ui.painter().text(
        egui::pos2(rect.left() + 18.0, y),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(14.0),
        t.weak,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 38.0, y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.5),
        t.text,
    );
    r.clicked()
}

impl App {
    /// Opens the view on one of its pages (Acerca de, Manual, Atajos).
    pub fn show_backstage(&mut self, page: Page) {
        self.backstage = true;
        self.backstage_page = page;
    }

    /// Draws the view in place of the work area. Returns nothing: what it does goes through
    /// `act`, and it closes itself when something was chosen.
    pub fn backstage_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        let recent = prefs::recent();
        let mut pick = None;
        egui::Panel::left("backstage_commands")
            .exact_size(230.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(t.header)
                    .inner_margin(egui::Margin::symmetric(10, 12)),
            )
            .show(ui, |ui| {
                if command(ui, &t, "←", "Volver") {
                    pick = Some(Pick::Back);
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                if command(ui, &t, "🕘", "Recientes") {
                    pick = Some(Pick::Page(Page::Home));
                }
                let items: [(&str, &str, Action); 3] = [
                    ("📂", "Abrir…", Action::Open),
                    ("📁", "Abrir proyecto…", Action::OpenProject),
                    ("💾", "Guardar proyecto…", Action::SaveProject),
                ];
                for (glyph, label, action) in items {
                    if command(ui, &t, glyph, label) {
                        pick = Some(Pick::Act(action));
                    }
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                if command(ui, &t, "?", "Manual") {
                    pick = Some(Pick::Page(Page::Manual));
                }
                if command(ui, &t, "⌨", "Atajos de teclado") {
                    pick = Some(Pick::Page(Page::Shortcuts));
                }
                if command(ui, &t, "©", &format!("Acerca de {}", brand::NAME)) {
                    pick = Some(Pick::Page(Page::About));
                }
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(t.canvas)
                    .inner_margin(egui::Margin::symmetric(36, 24)),
            )
            .show(ui, |ui| {
                match self.backstage_page {
                    Page::Home => {}
                    Page::About => return self.about_body(ui),
                    Page::Manual => return self.manual.show(ui),
                    Page::Shortcuts => return shortcuts(ui),
                }
                ui.horizontal(|ui| {
                    brand::paint(ui, brand::Art::Wordmark, self.dark, 22.0);
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(brand::CLAIM)
                            .strong()
                            .color(theme::brand_text(&t)),
                    );
                });
                ui.add_space(18.0);
                if recent.is_empty() {
                    ui.label(
                        egui::RichText::new("Aún no se ha abierto ningún archivo.").color(t.weak),
                    );
                    ui.label(
                        egui::RichText::new(
                            "Usa «Abrir…» o arrastra aquí nubes, rásteres, capas o proyectos.",
                        )
                        .color(t.weak),
                    );
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let projects: Vec<&PathBuf> = recent.iter().filter(|p| is_project(p)).collect();
                    let files: Vec<&PathBuf> = recent.iter().filter(|p| !is_project(p)).collect();
                    for (title, list) in [("Proyectos recientes", projects), ("Recientes", files)] {
                        if list.is_empty() {
                            continue;
                        }
                        ui.label(egui::RichText::new(title).size(16.0).strong().color(t.text));
                        ui.add_space(6.0);
                        for f in list {
                            if recent_row(ui, &t, f) {
                                pick = Some(Pick::Reopen(f.clone()));
                            }
                        }
                        ui.add_space(16.0);
                    }
                    if ui.button("Vaciar la lista").clicked() {
                        pick = Some(Pick::ClearRecent);
                    }
                });
            });
        match pick {
            Some(Pick::Back) => self.backstage = false,
            Some(Pick::Page(p)) => self.backstage_page = p,
            Some(Pick::ClearRecent) => prefs::clear_recent(),
            Some(Pick::Reopen(f)) => {
                self.backstage = false;
                self.open_files(ctx, vec![f]);
            }
            Some(Pick::Act(a)) => {
                // the dialogs and windows these open belong to the work area
                self.backstage = false;
                self.act(ctx, a);
            }
            None => {}
        }
    }
}

/// A recent file: its name, and below it the folder; the full path in the tooltip.
fn recent_row(ui: &mut egui::Ui, t: &theme::Tokens, f: &Path) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let name = f
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let dir = f
        .parent()
        .map_or(String::new(), |d| d.display().to_string());
    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.top() + 14.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(14.0),
        t.text,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.top() + 32.0),
        egui::Align2::LEFT_CENTER,
        dir,
        egui::FontId::proportional(11.0),
        t.weak,
    );
    r.on_hover_text(f.display().to_string()).clicked()
}

/// The keyboard shortcuts.
fn shortcuts(ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("shortcuts")
            .num_columns(2)
            .spacing([24.0, 4.0])
            .show(ui, |ui| {
                for (k, d) in [
                    ("Ctrl+O", "Abrir nubes"),
                    ("Ctrl+K", "Buscar comando"),
                    ("F1", "Manual"),
                    ("Z / X", "Zoom / desplazar"),
                    ("Q / W", "Consultar / medir"),
                    ("T", "TIN sombreado"),
                    ("N", "Norte arriba (vista 3D)"),
                    ("+ / −", "Tamaño del punto"),
                    ("Flechas", "Mover el perfil"),
                    ("Clic / clic derecho", "Zoom: acerca ×2 / aleja"),
                    ("Arrastrar · rueda", "Zoom: ventana / continuo"),
                    ("Botón central", "Zoom anterior"),
                    ("Ctrl+Z / Ctrl+Y", "Deshacer / rehacer (edición)"),
                    ("Ctrl+A / Ctrl+I", "Seleccionar todo / invertir"),
                    ("Esc", "Quitar la selección"),
                    ("Supr", "Eliminar los puntos seleccionados"),
                    ("Mayús", "Selección: añadir"),
                    ("Alt", "Selección: restar"),
                    ("Ctrl", "Selección: intersecar"),
                ] {
                    ui.label(egui::RichText::new(k).strong());
                    ui.label(d);
                    ui.end_row();
                }
            });
    });
}
