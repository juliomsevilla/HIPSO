//! Lo que comparten las ventanas de los procesos (LiDAR, ráster, vectorial, terreno…) para que se
//! vean y se manejen igual: dónde aparecen y cuánto miden, el párrafo de ayuda, las filas que eligen
//! archivos (de un archivo o de las capas cargadas) y la barra de ejecutar y detener.
//!
//! El orden de toda ventana de proceso es el mismo:
//! 1. «Guardar en» (`tools::output_row`) y una línea de separación;
//! 2. un párrafo de ayuda que dice qué hace el proceso y cómo (`help`);
//! 3. las entradas, con «Archivo…» y «De las capas cargadas» (`pick::raster`, `pick::vector`,
//!    `files_row`);
//! 4. los ajustes y, si algo falta, qué falta;
//! 5. la barra de ejecutar al pie (`run_bar`).

use super::*;

use std::cell::RefCell;
use std::path::PathBuf;

/// Qué capas cargadas ofrece [`layers_menu`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layers {
    Raster,
    Vector,
    Cloud,
}

type Loaded = Vec<(String, PathBuf)>;

thread_local! {
    /// Los rásteres, las capas vectoriales y las nubes que hay en el mapa; el programa lo renueva cada
    /// fotograma para que cualquier fila de archivo pueda ofrecerlos sin recibirlos por parámetro.
    static LOADED: RefCell<(Loaded, Loaded, Loaded)> = const { RefCell::new((Vec::new(), Vec::new(), Vec::new())) };
}

/// Fija las capas del mapa que ofrecen las filas de archivo.
pub fn set_loaded(rasters: Loaded, vectors: Loaded, clouds: Loaded) {
    LOADED.with(|l| *l.borrow_mut() = (rasters, vectors, clouds));
}

/// «De las capas cargadas»: un menú con las capas del mapa de ese tipo; elegir una pone su ruta en
/// `text`. Es lo que acompaña a «…» o «Archivo…» en toda fila que pide un archivo.
pub fn layers_menu(ui: &mut egui::Ui, kind: Layers, text: &mut String) {
    let (list, none) = LOADED.with(|l| {
        let l = l.borrow();
        match kind {
            Layers::Raster => (l.0.clone(), "No hay rásteres cargados"),
            Layers::Vector => (l.1.clone(), "No hay capas vectoriales cargadas"),
            Layers::Cloud => (l.2.clone(), "No hay nubes cargadas"),
        }
    });
    ui.menu_button("De las capas cargadas", |ui| {
        for (name, path) in &list {
            if ui.button(name).clicked() {
                *text = path.display().to_string();
                ui.close();
            }
        }
        if list.is_empty() {
            ui.label(none);
        }
    });
}

/// Ancho con el que se abren las ventanas de proceso.
pub const WIDTH: f32 = 560.0;

/// La ventana de un proceso con la posición, el ancho y el desplazamiento de todas: a la derecha,
/// bajo la cinta, fuera del camino del mapa.
pub fn window<'a>(ctx: &egui::Context, title: impl Into<egui::WidgetText>) -> egui::Window<'a> {
    egui::Window::new(title)
        .default_pos([(ctx.content_rect().right() - 600.0).max(320.0), 190.0])
        .vscroll(true)
        .max_height((ctx.content_rect().height() - 90.0).max(240.0))
        .default_width(WIDTH)
}

/// El párrafo de ayuda de una ventana: qué hace el proceso, en gris y letra pequeña.
pub fn help(ui: &mut egui::Ui, weak: egui::Color32, text: &str) {
    ui.label(egui::RichText::new(text).color(weak).size(11.0));
    ui.add_space(4.0);
}

/// El botón principal de una ventana de proceso con el aspecto de la barra de ejecutar, para las
/// ventanas cuyo botón no va al pie.
pub fn primary(label: &str, accent: egui::Color32) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(format!("▶  {label}"))
            .strong()
            .color(egui::Color32::WHITE),
    )
    .fill(accent)
    .min_size(egui::vec2(120.0, 30.0))
}

/// La barra de ejecutar, al pie: el botón principal a la derecha (con el color de acento) y, mientras
/// hay un proceso en marcha, un botón para detenerlo y la rueda. `ready` es si se puede ejecutar;
/// `why` es lo que dice la ayuda del botón cuando no. Devuelve si se pidió ejecutar. Pedir detener
/// activa `stop`, que el programa convierte en la cancelación del proceso.
pub fn run_bar(
    ui: &mut egui::Ui,
    accent: egui::Color32,
    label: &str,
    ready: bool,
    busy: bool,
    why: &str,
    stop: &mut bool,
) -> bool {
    ui.separator();
    let mut run = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let go = egui::Button::new(
            egui::RichText::new(format!("▶  {label}"))
                .strong()
                .color(egui::Color32::WHITE),
        )
        .fill(accent)
        .min_size(egui::vec2(120.0, 30.0));
        let hint = if busy { "Hay un proceso en marcha" } else { why };
        if ui
            .add_enabled(ready && !busy, go)
            .on_disabled_hover_text(hint)
            .clicked()
        {
            run = true;
        }
        if busy {
            if ui.button("■ Detener y borrar lo parcial").clicked() {
                *stop = true;
            }
            ui.spinner();
        }
    });
    run
}
