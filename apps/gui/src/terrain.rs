//! Terrain analysis (Superficies): slope, aspect, hillshade, curvatures, roughness, TRI, TPI,
//! depressions, flow accumulation, wetness, streams, subbasins and viewshed of a DTM.

use super::*;
use crate::numfield::NumField;
use rustlidar_core::paths;
use rustlidar_core::terrain::{self, Product, TerrainJob};

pub struct TerrainTool {
    pub open: bool,
    pub input: Option<PathBuf>,
    pub product: Product,
    pub percent: bool,
    pub z_factor: f64,
    pub azimuth: f64,
    pub altitude: f64,
    pub radius: f64,
    /// Drained area that makes a stream, hectares.
    pub threshold_ha: f64,
    pub observer_x: String,
    pub observer_y: String,
    pub eye: f64,
    pub target: f64,
    pub max_distance: f64,
    pub out: String,
    pub out_manual: bool,
}

impl Default for TerrainTool {
    fn default() -> Self {
        Self {
            open: false,
            input: None,
            product: Product::Slope,
            percent: false,
            z_factor: 1.0,
            azimuth: 315.0,
            altitude: 45.0,
            radius: 10.0,
            threshold_ha: 1.0,
            observer_x: String::new(),
            observer_y: String::new(),
            eye: 1.7,
            target: 0.0,
            max_distance: 1000.0,
            out: String::new(),
            out_manual: false,
        }
    }
}

fn help(p: Product) -> &'static str {
    match p {
        Product::Slope => "Inclinación del terreno en cada celda (método de Horn, ventana de 3×3).",
        Product::Aspect => {
            "Hacia dónde mira la ladera: grados desde el norte en sentido horario. Las celdas llanas quedan vacías."
        }
        Product::Hillshade => "Iluminación del relieve de 0 a 255, para ver el modelo con relieve.",
        Product::PlanCurvature => {
            "Curvatura transversal a la pendiente (Zevenbergen y Thorne). Positiva: convexa, como un espolón; negativa: cóncava, como un barranco."
        }
        Product::ProfileCurvature => {
            "Curvatura en el sentido de la pendiente. Positiva: convexa (el terreno se acelera); negativa: cóncava (se frena)."
        }
        Product::Curvature => "Curvatura total. Positiva en cumbres y lomas, negativa en vaguadas.",
        Product::Roughness => {
            "Diferencia entre la celda más alta y la más baja de la ventana de 3×3."
        }
        Product::Ruggedness => {
            "Media de las diferencias de altura con las ocho celdas vecinas (Wilson et al. 2007)."
        }
        Product::Position => {
            "Altura de la celda menos la media de las que la rodean en el radio: positivo en lomas, negativo en vaguadas."
        }
        Product::FillDepth => {
            "Cuánto se rellenaría cada depresión para que el agua pueda salir del modelo. Sirve también para detectar errores del MDT."
        }
        Product::FlowAccumulation => {
            "Superficie (m²) que vierte a cada celda, con flujo D8 y depresiones rellenadas."
        }
        Product::Wetness => {
            "Índice topográfico de humedad ln(a / tan β): alto donde se acumula el agua y la pendiente es baja."
        }
        Product::Streams => {
            "Celdas por las que vierte al menos la superficie indicada: la red de drenaje."
        }
        Product::Subbasins => "Cuenca de cada tramo de la red de drenaje, numeradas desde 1.",
        Product::Viewshed => {
            "Qué celdas ve un observador (1 las ve, 0 no). El resto, fuera del alcance, queda vacío."
        }
    }
}

impl App {
    pub fn terrain_window(&mut self, ctx: &egui::Context) {
        if !self.tt.open {
            return;
        }
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("Análisis de terreno")
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                let t = &mut self.tt;
                ui.label(
                    egui::RichText::new("Productos derivados de un MDT en un sistema métrico (UTM). El resultado es un GeoTIFF del mismo tamaño que el MDT.")
                        .color(weak)
                        .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.label("MDT");
                    let shown = t.input.as_deref().map_or("(elige un MDT)".to_string(), paths::stem);
                    ui.label(shown);
                    if ui.button("Archivo…").clicked()
                        && let Some(p) = crate::prefs::dialog()
                            .add_filter("Ráster", rustlidar_core::raster::EXTENSIONS)
                            .pick_file()
                    {
                        t.input = Some(p);
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                t.input = Some(path.clone());
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Producto");
                    egui::ComboBox::from_id_salt("Producto")
                    .selected_text(t.product.name())
                    .height(420.0)
                    .show_ui(ui, |ui| {
                        for (title, items) in [
                            ("Relieve", &Product::ALL[0..3]),
                            ("Forma del terreno", &Product::ALL[3..9]),
                            ("Hidrología", &Product::ALL[9..14]),
                            ("Visibilidad", &Product::ALL[14..]),
                        ] {
                            ui.label(egui::RichText::new(title).color(weak).size(11.0));
                            for p in items {
                                ui.selectable_value(&mut t.product, *p, p.name());
                            }
                        }
                    });
                });
                ui.label(egui::RichText::new(help(t.product)).color(weak).size(11.0));
                ui.separator();
                match t.product {
                    Product::Slope => {
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut t.percent, false, "Grados");
                            ui.radio_value(&mut t.percent, true, "Porcentaje");
                        });
                    }
                    Product::Hillshade => {
                        ui.horizontal(|ui| {
                            ui.label("Dirección de la luz");
                            ui.add(NumField::new(&mut t.azimuth).range(0.0..=360.0).suffix("°"));
                            ui.label("Altura de la luz");
                            ui.add(NumField::new(&mut t.altitude).range(1.0..=90.0).suffix("°"));
                        });
                    }
                    Product::Position => {
                        ui.horizontal(|ui| {
                            ui.label("Radio");
                            ui.add(NumField::new(&mut t.radius).range(0.1..=10000.0).suffix(" m"));
                        });
                    }
                    Product::Streams | Product::Subbasins => {
                        ui.horizontal(|ui| {
                            ui.label("Superficie mínima que vierte");
                            ui.add(NumField::new(&mut t.threshold_ha).range(0.0001..=100000.0).suffix(" ha"));
                        });
                    }
                    Product::Viewshed => {
                        ui.horizontal(|ui| {
                            ui.label("Observador X");
                            ui.add(egui::TextEdit::singleline(&mut t.observer_x).desired_width(90.0));
                            ui.label("Y");
                            ui.add(egui::TextEdit::singleline(&mut t.observer_y).desired_width(90.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Altura de los ojos");
                            ui.add(NumField::new(&mut t.eye).range(0.0..=1000.0).suffix(" m"));
                            ui.label("Altura a ver");
                            ui.add(NumField::new(&mut t.target).range(0.0..=1000.0).suffix(" m"));
                            ui.label("Alcance");
                            ui.add(NumField::new(&mut t.max_distance).range(1.0..=100000.0).suffix(" m"));
                        });
                    }
                    _ => {}
                }
                ui.horizontal(|ui| {
                    ui.label("Factor Z");
                    ui.add(NumField::new(&mut t.z_factor).range(0.0001..=100000.0));
                    ui.label(egui::RichText::new("1 si las alturas están en metros").color(weak).size(11.0));
                });
                if !t.out_manual {
                    t.out = t.input.as_ref().map_or(String::new(), |f| {
                        paths::unique(&paths::product(f, t.product.tag(), "tif")).display().to_string()
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Salida");
                    if ui.add(egui::TextEdit::singleline(&mut t.out).desired_width(360.0)).changed() {
                        t.out_manual = true;
                    }
                    if ui.button("…").clicked()
                        && let Some(p) = crate::prefs::dialog().add_filter("GeoTIFF", &["tif"]).set_file_name("salida.tif").save_file()
                    {
                        t.out = p.display().to_string();
                        t.out_manual = true;
                    }
                    if t.out_manual && ui.button("auto").clicked() {
                        t.out_manual = false;
                    }
                });
                let observer_ok = t.product != Product::Viewshed
                    || (num(&t.observer_x).is_some() && num(&t.observer_y).is_some());
                if !observer_ok {
                    ui.label(egui::RichText::new("Escribe las coordenadas X e Y del observador (las del mapa).").color(weak).size(11.0));
                }
                let ready = t.input.is_some() && !t.out.trim().is_empty() && observer_ok;
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Calcular")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.tt;
            let (Some(input), out) = (t.input.clone(), PathBuf::from(t.out.trim())) else {
                return;
            };
            if out.exists() && t.out_manual {
                self.log("La salida elegida a mano ya existe: cámbiala o bórrala.");
            } else {
                let out = if out.exists() {
                    paths::unique(&out)
                } else {
                    out
                };
                let mut job = TerrainJob::new(input, out.clone(), t.product);
                job.percent = t.percent;
                job.z_factor = t.z_factor;
                (job.azimuth, job.altitude) = (t.azimuth, t.altitude);
                job.radius = t.radius;
                job.threshold = t.threshold_ha * 10_000.0;
                job.observer = num(&t.observer_x)
                    .zip(num(&t.observer_y))
                    .map(|(x, y)| [x, y]);
                (job.observer_height, job.target_height) = (t.eye, t.target);
                job.max_distance = t.max_distance;
                let label = t.product.name().to_string();
                let task: tools::Task = Box::new(move || terrain::run(&job));
                self.start(
                    ctx,
                    Launch {
                        label,
                        task,
                        show: vec![out.clone()],
                        outputs: vec![out],
                    },
                );
            }
        }
        if !open {
            self.tt.open = false;
        }
    }
}

fn num(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse().ok()
}
