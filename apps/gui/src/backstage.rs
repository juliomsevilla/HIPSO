//! La vista «Archivo», al estilo de la trastienda de un paquete ofimático: cubre el área de trabajo
//! con las órdenes de archivo a la izquierda (abrir, proyectos, información, cerrar) y los archivos
//! recientes a la derecha, de modo que todo lo relativo a los archivos está en un solo sitio.

use super::*;

/// Lo que muestra el lado derecho de la vista.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Los archivos recientes.
    Home,
    About,
    Manual,
    Shortcuts,
    Settings,
}

/// Lo que el usuario eligió en la vista.
enum Pick {
    Act(Action),
    Reopen(PathBuf),
    ClearRecent,
    Back,
    Page(Page),
}

/// Una orden de la columna izquierda: una fila de todo el ancho que se ilumina bajo el puntero.
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
    /// Abre la vista en una de sus páginas (Acerca de, Manual, Atajos).
    pub fn show_backstage(&mut self, page: Page) {
        self.backstage = true;
        self.backstage_page = page;
    }

    /// Dibuja la vista en lugar del área de trabajo. No devuelve nada: lo que hace pasa por `act`,
    /// y se cierra sola cuando se ha elegido algo.
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
                if command(ui, &t, "⚙", "Ajustes") {
                    pick = Some(Pick::Page(Page::Settings));
                }
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
                    Page::Settings => return self.settings_body(ui),
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
                    let projects: Vec<&PathBuf> =
                        recent.iter().filter(|p| project::is_project(p)).collect();
                    let files: Vec<&PathBuf> =
                        recent.iter().filter(|p| !project::is_project(p)).collect();
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
            Some(Pick::Page(p)) => {
                self.backstage_page = p;
                // el tamaño de la caché se vuelve a leer cada vez que se abre la página
                self.settings.stats = None;
            }
            Some(Pick::ClearRecent) => prefs::clear_recent(),
            Some(Pick::Reopen(f)) => {
                self.backstage = false;
                self.open_files(ctx, vec![f]);
            }
            Some(Pick::Act(a)) => {
                // los diálogos y ventanas que abren estas órdenes pertenecen al área de trabajo
                self.backstage = false;
                self.act(ctx, a);
            }
            None => {}
        }
    }
}

/// Lo que conserva la página «Ajustes» entre fotogramas.
#[derive(Default)]
pub struct Settings {
    /// La carpeta de la caché tal como se está escribiendo.
    dir_text: String,
    /// Los gigabytes que guarda la caché, tal como se están editando.
    gb: u64,
    started: bool,
    /// Lo que ocupan la caché y las teselas guardadas: (copias, bytes de copias, teselas, bytes de
    /// teselas).
    stats: Option<(usize, u64, usize, u64)>,
    /// La ventana que pregunta antes de borrar la caché está abierta.
    confirm: bool,
    /// Lo que hizo la última acción.
    result: String,
}

/// Un tamaño para personas: gigabytes, megabytes o kilobytes.
fn size_text(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

impl App {
    /// Las copias de la caché de detalle de las nubes abiertas: nunca se borran.
    fn open_copies(&self) -> Vec<PathBuf> {
        self.clouds
            .iter()
            .map(|p| hipso_core::copc_cache::path_for(p))
            .collect()
    }

    /// Guarda la carpeta (vacía: la de por defecto) y los gigabytes, y recorta la caché hasta
    /// ajustarla a ellos.
    fn apply_cache_settings(&mut self) {
        let text = self.settings.dir_text.trim().to_string();
        let dir = (!text.is_empty()).then(|| PathBuf::from(&text));
        if let Some(d) = &dir
            && let Err(e) = std::fs::create_dir_all(d)
        {
            self.settings.result = format!("No se pudo usar la carpeta: {e}.");
            return;
        }
        let gb = self.settings.gb.max(1);
        prefs::set_cache(dir, gb);
        let freed = hipso_core::copc_cache::prune(gb << 30, &self.open_copies());
        self.settings.result = if freed > 0 {
            format!(
                "Guardado. Se han borrado {} de las copias menos usadas para ajustarse al límite.",
                size_text(freed)
            )
        } else {
            "Guardado.".to_string()
        };
        self.settings.stats = None;
    }

    /// La página «Ajustes».
    fn settings_body(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        if !self.settings.started {
            let (dir, gb) = prefs::cache();
            self.settings.dir_text = dir.map_or(String::new(), |d| d.display().to_string());
            self.settings.gb = gb;
            self.settings.started = true;
        }
        if self.settings.stats.is_none() {
            let (n, b) = hipso_core::copc_cache::size();
            let (tn, tb) = basemap::tiles_size();
            self.settings.stats = Some((n, b, tn, tb));
        }
        let stats = self.settings.stats.unwrap_or_default();
        ui.label(egui::RichText::new("Ajustes").size(18.0).strong().color(t.text));
        ui.add_space(14.0);
        ui.label(egui::RichText::new("Caché").size(16.0).strong().color(t.text));
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Al abrir una nube grande se guarda una copia con el detalle organizado por zonas, \
                 para que al acercar se lea solo lo que se ve. La primera apertura tarda; las \
                 siguientes, no. El mapa base también guarda aquí las teselas descargadas. Los \
                 archivos originales nunca se tocan.",
            )
            .color(t.weak),
        );
        ui.add_space(10.0);
        ui.label("Carpeta de la caché");
        let default = hipso_core::copc_cache::default_dir();
        let mut apply = false;
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.settings.dir_text)
                    .hint_text(default.display().to_string())
                    .desired_width((ui.available_width() - 260.0).max(200.0)),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                apply = true;
            }
            if ui.button("Elegir carpeta…").clicked()
                && let Some(d) = rfd::FileDialog::new().pick_folder()
            {
                self.settings.dir_text = d.display().to_string();
                apply = true;
            }
            if ui.button("Por defecto").clicked() {
                self.settings.dir_text.clear();
                apply = true;
            }
        });
        ui.label(
            egui::RichText::new(format!(
                "En uso: {}. Las copias que ya existen no se mueven al cambiar de carpeta.",
                hipso_core::copc_cache::cache_dir().display()
            ))
            .size(11.5)
            .color(t.weak),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Tamaño máximo de las copias");
            let r = ui.add(
                egui::DragValue::new(&mut self.settings.gb)
                    .range(1..=100_000)
                    .suffix(" GB"),
            );
            if r.drag_stopped() || r.lost_focus() {
                apply = true;
            }
        });
        ui.label(
            egui::RichText::new(
                "Al superarlo se borran las copias que más tiempo llevan sin usarse; las de las \
                 nubes abiertas se respetan.",
            )
            .size(11.5)
            .color(t.weak),
        );
        if apply {
            self.apply_cache_settings();
        }
        ui.add_space(10.0);
        ui.label(format!(
            "Ocupan ahora: copias de detalle, {} ({} archivos); teselas del mapa base, {} ({} archivos).",
            size_text(stats.1),
            stats.0,
            size_text(stats.3),
            stats.2
        ));
        ui.add_space(6.0);
        if ui.button("Borrar caché…").clicked() {
            self.settings.confirm = true;
            self.settings.result.clear();
        }
        if !self.settings.result.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&self.settings.result).color(theme::brand_text(&t)));
        }
        if self.settings.confirm {
            let mut answer = None;
            egui::Window::new("Borrar la caché")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size([430.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.label(format!(
                        "Se borrarán las copias de detalle de las nubes ({}) y las teselas del mapa base ({}).",
                        size_text(stats.1),
                        size_text(stats.3)
                    ));
                    ui.add_space(6.0);
                    ui.label(
                        "No se pierde ningún dato: los archivos originales, los proyectos y los \
                         resultados no se tocan. Lo que cambia es que la próxima vez que se abra \
                         una nube grande la copia tardará de nuevo en prepararse (se rehace en \
                         segundo plano) y el mapa base volverá a descargarse. Las copias de las \
                         nubes que están abiertas ahora se conservan. No se puede deshacer.",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Borrar").clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Cancelar").clicked() {
                            answer = Some(false);
                        }
                    });
                });
            if let Some(yes) = answer {
                self.settings.confirm = false;
                if yes {
                    let (n, b) = hipso_core::copc_cache::clear(&self.open_copies());
                    let (tn, tb) = basemap::clear_tiles();
                    self.settings.result = format!(
                        "Caché borrada: {} liberados ({} archivos).",
                        size_text(b + tb),
                        n + tn
                    );
                    self.settings.stats = None;
                }
            }
        }
    }
}

/// Un archivo reciente: su nombre y, debajo, la carpeta; la ruta completa, en la ayuda emergente.
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

/// Los atajos de teclado.
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
