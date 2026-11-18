//! Lo que comparten las ventanas de los procesos (LiDAR, ráster, vectorial, terreno…) para que se
//! vean y se manejen igual: dónde aparecen y cuánto miden, el párrafo de ayuda, las filas que eligen
//! archivos (de un archivo o de las capas cargadas) y la barra de ejecutar y detener.
//!
//! El orden de toda ventana de proceso es el mismo: se entiende la herramienta, se decide con las
//! opciones, se elige dónde guardar y se ejecuta.
//! 1. la descripción de lo que hace y, debajo, los avisos en naranja (`begin`, `note`);
//! 2. las entradas, con «Archivo…» y «De las capas cargadas» (`pick::raster`, `pick::vector`,
//!    `layers_menu`), y los ajustes;
//! 3. al pie, «Guardar en» y la barra de ejecutar y detener (`footer`).

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

/// Los avisos de una ventana: los de este fotograma y los que se enseñan.
#[derive(Default)]
struct Notes {
    new: Vec<String>,
    shown: Vec<String>,
}

thread_local! {
    /// Los avisos de cada ventana de proceso abierta, y la ventana que se está dibujando. Como las
    /// ventanas se dibujan de arriba abajo y los avisos van arriba, se enseñan los del fotograma
    /// anterior (un fotograma de retraso que nadie nota).
    static NOTES: RefCell<(std::collections::HashMap<egui::Id, Notes>, Option<egui::Id>)> =
        RefCell::new((std::collections::HashMap::new(), None));
}

/// Un aviso de la ventana (algo que falta, algo que no cuadra): sale en naranja bajo la descripción,
/// en lugar de en el sitio del control al que se refiere. Se llama cada fotograma mientras el aviso
/// valga, y siempre después de [`begin`], que fija de qué ventana es.
pub fn note(text: impl Into<String>) {
    let text = text.into();
    NOTES.with(|n| {
        let mut n = n.borrow_mut();
        let Some(id) = n.1 else { return };
        let notes = n.0.entry(id).or_default();
        if !notes.new.contains(&text) {
            notes.new.push(text);
        }
    });
}

/// El color de los avisos de toda ventana de proceso.
pub const WARNING: egui::Color32 = egui::Color32::from_rgb(230, 160, 60);

/// El principio de toda ventana de proceso: la descripción de lo que hace, en gris y letra pequeña,
/// y debajo los avisos vigentes, en naranja.
pub fn begin(ui: &mut egui::Ui, weak: egui::Color32, description: &str) {
    let id = ui.id();
    let shown = NOTES.with(|n| {
        let mut n = n.borrow_mut();
        n.1 = Some(id);
        n.0.entry(id).or_default().shown.clone()
    });
    ui.label(egui::RichText::new(description).color(weak).size(11.0));
    for n in &shown {
        ui.label(egui::RichText::new(n).color(WARNING).size(11.0));
    }
    ui.add_space(4.0);
    ui.separator();
}

/// Cierra el fotograma de avisos de la ventana: lo que se avisó ahora es lo que se enseña en el
/// fotograma siguiente. Lo hace [`footer`]; las ventanas sin pie lo llaman a mano.
pub fn end(ctx: &egui::Context) {
    let changed = NOTES.with(|n| {
        let mut n = n.borrow_mut();
        let Some(id) = n.1 else { return false };
        let notes = n.0.entry(id).or_default();
        let new = std::mem::take(&mut notes.new);
        let changed = new != notes.shown;
        notes.shown = new;
        changed
    });
    if changed {
        ctx.request_repaint();
    }
}

/// Si la ventana que se dibuja aún no ha dado ningún aviso en este fotograma.
fn no_notes() -> bool {
    NOTES.with(|n| {
        let n = n.borrow();
        n.1.and_then(|id| n.0.get(&id)).is_none_or(|x| x.new.is_empty())
    })
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

/// El pie de toda ventana de proceso: dónde se guarda el resultado y, debajo, la barra de ejecutar.
/// Es lo último que se dibuja, y cierra el fotograma de avisos. Se entiende la herramienta, se decide
/// con las opciones, se elige dónde guardar y se ejecuta.
#[allow(clippy::too_many_arguments)]
pub fn footer(
    ui: &mut egui::Ui,
    out_dir: &mut String,
    accent: egui::Color32,
    label: &str,
    ready: bool,
    busy: bool,
    why: &str,
    stop: &mut bool,
) -> bool {
    ui.separator();
    crate::tools::output_row(ui, out_dir);
    ui.add_space(2.0);
    if !ready && !busy && no_notes() {
        note(why);
    }
    end(ui.ctx());
    run_bar(ui, accent, label, ready, busy, why, stop)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(id: egui::Id) -> Vec<String> {
        NOTES.with(|n| n.borrow().0.get(&id).map(|x| x.shown.clone()).unwrap_or_default())
    }

    #[test]
    fn each_window_keeps_its_own_notes_and_shows_them_a_frame_later() {
        let ctx = egui::Context::default();
        let ids = std::cell::RefCell::new(Vec::<egui::Id>::new());
        let frame = |warn_a: bool| {
            ids.borrow_mut().clear();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
                ..egui::RawInput::default()
            };
            let out = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                for (name, warn) in [("A", warn_a), ("B", false)] {
                    egui::Window::new(name).show(&ctx, |ui| {
                        begin(ui, egui::Color32::GRAY, "descripción");
                        ids.borrow_mut().push(ui.id());
                        if warn {
                            note("falta algo");
                        }
                        let mut stop = false;
                        footer(ui, &mut String::new(), egui::Color32::BLUE, "Ejecutar", true, false, "", &mut stop);
                    });
                }
            });
            out.drop_without_applying_deltas();
        };
        frame(true);
        let (a, b) = (ids.borrow()[0], ids.borrow()[1]);
        // el aviso se da en el fotograma 1 y se enseña en el 2, solo en su ventana
        assert!(shown(a) == vec!["falta algo".to_string()] && shown(b).is_empty());
        frame(true);
        assert!(shown(a) == vec!["falta algo".to_string()] && shown(b).is_empty());
        // cuando el aviso deja de valer, desaparece
        frame(false);
        assert!(shown(a).is_empty());
    }

    #[test]
    fn a_window_that_cannot_run_says_why_when_nothing_else_does() {
        let ctx = egui::Context::default();
        let id = std::cell::Cell::new(egui::Id::NULL);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            ..egui::RawInput::default()
        };
        let out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            egui::Window::new("C").show(&ctx, |ui| {
                begin(ui, egui::Color32::GRAY, "descripción");
                id.set(ui.id());
                let mut stop = false;
                footer(ui, &mut String::new(), egui::Color32::BLUE, "Ejecutar", false, false, "Elige un ráster", &mut stop);
            });
        });
        out.drop_without_applying_deltas();
        assert!(shown(id.get()) == vec!["Elige un ráster".to_string()]);
    }
}
