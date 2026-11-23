//! Los modos de color de los puntos: el nombre, el icono y la ayuda de cada uno, y el menú del
//! botón grande de la cinta que los reúne.

use eframe::egui::{self, Ui};
use hipso_core::view::ColorBy;

use crate::ribbon;
use crate::theme::Tokens;

/// Un modo de color: cuál es, su icono (`assets/icons/c_*.svg`), su nombre y qué enseña.
pub struct Mode {
    pub by: ColorBy,
    pub icon: &'static str,
    pub name: &'static str,
    pub help: &'static str,
}

/// Todos los modos, en el orden en que salen en el menú.
pub const MODES: [Mode; 13] = [
    Mode {
        by: ColorBy::Elevation,
        icon: "@c_elev",
        name: "Elevación",
        help: "Rampa de azul (abajo) a rojo (arriba) según la cota. El rango se ajusta en «Rango Z».",
    },
    Mode {
        by: ColorBy::ElevationEarth,
        icon: "@c_earth",
        name: "Elevación (tierra)",
        help: "La misma idea con una rampa de verdes, ocres y marrones, que se parece más al terreno.",
    },
    Mode {
        by: ColorBy::HeightAboveGround,
        icon: "@c_hag",
        name: "Altura sobre el suelo",
        help: "Colorea cada punto por lo que se separa del suelo clasificado (clase 2): la copa de un árbol sale a su altura real, aunque esté en una ladera. Es una aproximación para la vista, no se guarda. Sin suelo clasificado no hay referencia.",
    },
    Mode {
        by: ColorBy::Class,
        icon: "@c_class",
        name: "Clasificación",
        help: "El color de cada clase (suelo, vegetación, edificios, cables…). Los colores se cambian en «Clases y colores…».",
    },
    Mode {
        by: ColorBy::ClassIntensity,
        icon: "@c_classint",
        name: "Clase + intensidad",
        help: "El color de la clase, más claro o más oscuro según la intensidad del retorno. Deja ver la clase sin perder el relieve que da la intensidad.",
    },
    Mode {
        by: ColorBy::ElevationIntensity,
        icon: "@c_elevint",
        name: "Elevación + intensidad",
        help: "La rampa de elevación, más clara o más oscura según la intensidad del retorno.",
    },
    Mode {
        by: ColorBy::Intensity,
        icon: "@c_int",
        name: "Intensidad",
        help: "Tonos de gris según la intensidad del retorno.",
    },
    Mode {
        by: ColorBy::Return,
        icon: "@c_ret",
        name: "Retorno",
        help: "Según el tipo de retorno del pulso: único, primero, intermedio o último.",
    },
    Mode {
        by: ColorBy::ScanAngle,
        icon: "@c_scan",
        name: "Ángulo de barrido",
        help: "Con signo: azul a un lado de la pasada, gris en el nadir y rojo al otro lado (llega al extremo a 30°). Sirve para ver los bordes de pasada y los puntos oblicuos. Necesita archivos con ángulo de barrido.",
    },
    Mode {
        by: ColorBy::Source,
        icon: "@c_src",
        name: "ID de fuente",
        help: "Un color por pasada de vuelo (ID de fuente de punto). Se ve el solape entre pasadas.",
    },
    Mode {
        by: ColorBy::File,
        icon: "@c_file",
        name: "Archivo",
        help: "Un color por archivo de origen, para distinguir las hojas de una nube.",
    },
    Mode {
        by: ColorBy::Rgb,
        icon: "@c_rgb",
        name: "RGB",
        help: "El color real que trae cada punto, si el archivo lo tiene.",
    },
    Mode {
        by: ColorBy::Tree,
        icon: "@c_tree",
        name: "ID de árbol (treeID)",
        help: "Un color por individuo, a partir del atributo treeID que escribe «Segmentar con CHM».",
    },
];

/// El modo de `by`.
pub fn mode(by: ColorBy) -> &'static Mode {
    MODES.iter().find(|m| m.by == by).unwrap_or(&MODES[0])
}

/// El nombre corto del modo para el título de la vista.
pub fn title(by: ColorBy) -> &'static str {
    match by {
        ColorBy::Elevation | ColorBy::ElevationEarth => "elevación",
        ColorBy::Class => "clasificación",
        ColorBy::Intensity => "intensidad",
        ColorBy::Return => "retorno",
        ColorBy::Source => "fuente",
        ColorBy::File => "archivo",
        ColorBy::Rgb => "RGB",
        ColorBy::Tree => "ID de árbol",
        ColorBy::HeightAboveGround => "altura sobre el suelo",
        ColorBy::ScanAngle => "ángulo de barrido",
        ColorBy::ClassIntensity => "clase + intensidad",
        ColorBy::ElevationIntensity => "elevación + intensidad",
    }
}

/// La lista de modos con su icono; devuelve el elegido.
fn list(ui: &mut Ui, t: &Tokens, current: ColorBy) -> Option<ColorBy> {
    let mut chosen = None;
    for m in &MODES {
        if ribbon::small_button(ui, t, m.icon, m.name, current == m.by)
            .on_hover_text(m.help)
            .clicked()
        {
            chosen = Some(m.by);
        }
    }
    chosen
}

/// El botón grande «Color» con la lista de modos desplegable. En el menú emergente de un grupo
/// plegado la lista es un submenú. Devuelve el modo elegido.
pub fn button(ui: &mut Ui, ctx: &egui::Context, t: &Tokens, current: ColorBy) -> Option<ColorBy> {
    let m = mode(current);
    let mut chosen = None;
    if ribbon::in_folded_popup(ctx) {
        ui.menu_button(format!("Color: {}  ▾", m.name), |ui| {
            if let Some(c) = list(ui, t, current) {
                chosen = Some(c);
                ui.close();
            }
        })
        .response
        .on_hover_text("Atributo con el que se colorean los puntos.");
    } else {
        let r = ribbon::big_button_body(ui, t, m.icon, &format!("Color\n{} ▾", short(m.name)), false)
            .on_hover_text("Atributo con el que se colorean los puntos: elevación, altura sobre el suelo, clase, intensidad…");
        egui::Popup::menu(&r).show(|ui| {
            if let Some(c) = list(ui, t, current) {
                chosen = Some(c);
            }
        });
    }
    chosen
}

/// El nombre del modo recortado para que quepa en el botón grande.
fn short(name: &str) -> &str {
    match name {
        "Elevación (tierra)" => "Tierra",
        "Altura sobre el suelo" => "Alt. suelo",
        "Clase + intensidad" => "Clase+int.",
        "Elevación + intensidad" => "Elev.+int.",
        "Ángulo de barrido" => "Áng. barrido",
        "ID de árbol (treeID)" => "ID árbol",
        n => n,
    }
}
