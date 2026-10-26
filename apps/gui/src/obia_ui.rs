//! Object-based analysis (Inventario forestal): group the cells of the canopy height model and of
//! other layers into objects, describe them and classify them.

use super::*;
use crate::numfield::NumField;
use rustlidar_core::filter::Filter;
use rustlidar_core::jobs::{ClassifyBy, ObiaJob, StrataOpts};
use rustlidar_core::obia::classify::ForestParams;
use rustlidar_core::obia::layers::CloudLayer;
use rustlidar_core::obia::segment::{MergeParams, Method, SegmentParams};
use rustlidar_core::paths::{self, Formats};

pub struct ObiaTool {
    pub open: bool,
    pub chm: Option<PathBuf>,
    pub extra: Vec<PathBuf>,
    /// Intensity, returns, ground share, density.
    pub cloud: [bool; 4],
    pub roughness: usize,
    pub mask: bool,
    pub scale: f64,
    pub shape: f64,
    pub compact: f64,
    pub rough_weight: f64,
    pub mask_min: f64,
    pub mask_max: f64,
    pub diagonal: bool,
    pub min_cells: u32,
    /// 0 no classification, 1 rules, 2 samples.
    pub classify: u8,
    pub rules: Option<PathBuf>,
    pub samples: Option<PathBuf>,
    pub class_field: String,
    pub trees: usize,
    /// Strata by similarity: how many.
    pub k: usize,
    pub strata: bool,
    pub strata_min_cells: u32,
    pub plots: usize,
    pub cover_height: f64,
    pub vector: bool,
    pub csv: bool,
    pub simplify: f64,
    pub out: String,
    pub out_manual: bool,
}

impl Default for ObiaTool {
    fn default() -> Self {
        Self {
            open: false,
            chm: None,
            extra: Vec::new(),
            cloud: [true, true, false, false],
            roughness: 2,
            mask: false,
            scale: 8.0,
            shape: 0.1,
            compact: 0.5,
            rough_weight: 0.5,
            mask_min: 2.0,
            mask_max: 100.0,
            diagonal: false,
            min_cells: 10,
            classify: 0,
            rules: None,
            samples: None,
            class_field: "clase".into(),
            trees: 100,
            k: 4,
            strata: false,
            strata_min_cells: 500,
            plots: 0,
            cover_height: 2.0,
            vector: true,
            csv: false,
            simplify: 0.0,
            out: String::new(),
            out_manual: false,
        }
    }
}

const CLOUD_LAYERS: [(CloudLayer, &str); 4] = [
    (CloudLayer::Intensity, "Intensidad media"),
    (CloudLayer::Returns, "Retornos por pulso"),
    (CloudLayer::Ground, "Proporción de suelo"),
    (CloudLayer::Density, "Densidad de puntos"),
];

const RULES_HELP: &str = "Un archivo de texto con una regla por línea:\n  arbolado: chm_media >= 5 and area < 400\n  matorral: chm_media >= 0.5\nSe aplica la primera que cumple un objeto. Los atributos disponibles se ven en la tabla de resultados (area, perimetro, compacidad, alargamiento, <capa>_media, _desv, _min, _max).";

impl App {
    pub fn obia_window(&mut self, ctx: &egui::Context) {
        if !self.ot.open {
            return;
        }
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let clouds = self.clouds.clone();
        let weak = self.tokens().weak;
        let mut open = true;
        let mut go = false;
        egui::Window::new("OBIA: análisis por objetos")
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                let t = &mut self.ot;
                ui.label(
                    egui::RichText::new("Agrupa las celdas en objetos (copas, rodales, tejados…) a partir del CHM y de otras capas, los describe y, si quieres, los clasifica. Todas las capas deben coincidir celda a celda.")
                        .color(weak)
                        .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.label("CHM");
                    ui.label(t.chm.as_deref().map_or("(elige un CHM)".to_string(), paths::stem));
                    if ui.button("Archivo…").clicked()
                        && let Some(p) = crate::prefs::dialog().add_filter("Ráster", rustlidar_core::raster::EXTENSIONS).pick_file()
                    {
                        t.chm = Some(p);
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                t.chm = Some(path.clone());
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Otras capas ráster");
                    if ui.button("Añadir…").clicked()
                        && let Some(ps) = crate::prefs::dialog().add_filter("Ráster", rustlidar_core::raster::EXTENSIONS).pick_files()
                    {
                        t.extra.extend(ps);
                    }
                    if !t.extra.is_empty() && ui.button("Quitar todas").clicked() {
                        t.extra.clear();
                    }
                });
                for p in &t.extra {
                    ui.label(egui::RichText::new(format!("   · {}", paths::stem(p))).size(11.0));
                }
                egui::CollapsingHeader::new("1 · Capas que salen de la nube")
                    .default_open(true)
                    .show(ui, |ui| {
                        if clouds.is_empty() {
                            ui.label(egui::RichText::new("Abre una nube para derivar capas de ella.").color(weak).size(11.0));
                        } else {
                            ui.label(egui::RichText::new(format!("Se leen {} nube(s) cargada(s), una sola vez.", clouds.len())).color(weak).size(11.0));
                        }
                        ui.add_enabled_ui(!clouds.is_empty(), |ui| {
                            for (k, (_, name)) in CLOUD_LAYERS.iter().enumerate() {
                                ui.checkbox(&mut t.cloud[k], *name);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Rugosidad del CHM (radio, 0 = no)");
                            ui.add(NumField::new(&mut t.roughness).range(0.0..=50.0).suffix(" celdas"));
                        });
                    });
                egui::CollapsingHeader::new("2 · Objetos")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut t.mask, false, "Fusión de regiones");
                            ui.radio_value(&mut t.mask, true, "Rango de valores del CHM");
                        });
                        if t.mask {
                            ui.horizontal(|ui| {
                                ui.label("Entre");
                                ui.add(NumField::new(&mut t.mask_min).range(-1.0e6..=1.0e6));
                                ui.label("y");
                                ui.add(NumField::new(&mut t.mask_max).range(-1.0e6..=1.0e6));
                                ui.checkbox(&mut t.diagonal, "unir en diagonal");
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.label("Escala");
                                ui.add(NumField::new(&mut t.scale).range(0.5..=1000.0));
                                ui.label(egui::RichText::new("mayor: objetos más grandes (prueba entre 5 y 15)").color(weak).size(11.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Peso de la forma");
                                ui.add(NumField::new(&mut t.shape).range(0.0..=0.9));
                                ui.label("compacidad");
                                ui.add(NumField::new(&mut t.compact).range(0.0..=1.0));
                            });
                            if t.roughness > 0 {
                                ui.horizontal(|ui| {
                                    ui.label("Peso de la rugosidad");
                                    ui.add(NumField::new(&mut t.rough_weight).range(0.0..=1.0));
                                    ui.label(egui::RichText::new("1 pesa como las demás capas; 0 la deja fuera").color(weak).size(11.0));
                                });
                            }
                        }
                        ui.horizontal(|ui| {
                            ui.label("Objeto mínimo");
                            ui.add(NumField::new(&mut t.min_cells).range(1.0..=1.0e6).suffix(" celdas"));
                        });
                    });
                egui::CollapsingHeader::new("3 · Clasificación")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut t.classify, 0, "Solo describir");
                            ui.radio_value(&mut t.classify, 1, "Con reglas");
                            ui.radio_value(&mut t.classify, 2, "Con muestras (bosque aleatorio)");
                            ui.radio_value(&mut t.classify, 3, "Agrupar por parecido");
                        });
                        if t.classify == 3 {
                            ui.horizontal(|ui| {
                                ui.label("Número de estratos");
                                ui.add(NumField::new(&mut t.k).range(2.0..=64.0));
                            });
                            ui.label(
                                egui::RichText::new("Sin reglas ni muestras: agrupa los objetos según su altura media, su percentil 95, la cobertura de copas y la media de las demás capas, dando más peso a los objetos grandes. El estrato 1 es el más bajo.")
                                    .color(weak)
                                    .size(11.0),
                            );
                        }
                        if t.classify == 1 {
                            ui.horizontal(|ui| {
                                ui.label("Reglas");
                                ui.label(t.rules.as_deref().map_or("(elige un archivo)".to_string(), paths::stem));
                                if ui.button("Archivo…").clicked()
                                    && let Some(p) = crate::prefs::dialog().add_filter("Texto", &["txt"]).pick_file()
                                {
                                    t.rules = Some(p);
                                }
                            });
                            ui.label(egui::RichText::new(RULES_HELP).color(weak).size(11.0));
                        }
                        if t.classify == 2 {
                            ui.horizontal(|ui| {
                                ui.label("Muestras");
                                ui.label(t.samples.as_deref().map_or("(elige un shapefile)".to_string(), paths::stem));
                                if ui.button("Archivo…").clicked()
                                    && let Some(p) = crate::prefs::dialog().add_filter("Shapefile", &["shp"]).pick_file()
                                {
                                    t.samples = Some(p);
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Campo de la clase");
                                ui.add(egui::TextEdit::singleline(&mut t.class_field).desired_width(110.0));
                                ui.label("Árboles");
                                ui.add(NumField::new(&mut t.trees).range(5.0..=2000.0));
                            });
                            ui.label(
                                egui::RichText::new("Dibuja polígonos que llenen los objetos de cada clase (Capas → Nueva capa de áreas, con un campo de texto para la clase). Un objeto cuenta como muestra si los polígonos cubren al menos la mitad de sus celdas.")
                                    .color(weak)
                                    .size(11.0),
                            );
                        }
                    });
                egui::CollapsingHeader::new("4 · Estratos homogéneos")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add_enabled_ui(t.classify != 0, |ui| {
                            ui.checkbox(&mut t.strata, "Unir los objetos vecinos de la misma clase en estratos");
                            if t.strata {
                                ui.horizontal(|ui| {
                                    ui.label("Estrato mínimo");
                                    ui.add(NumField::new(&mut t.strata_min_cells).range(1.0..=1.0e7).suffix(" celdas"));
                                    ui.label(egui::RichText::new("los menores toman la clase de sus vecinos").color(weak).size(11.0));
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Parcelas a repartir");
                                    ui.add(NumField::new(&mut t.plots).range(0.0..=100000.0));
                                    ui.label(egui::RichText::new("0 = no repartir (asignación de Neyman: más a lo grande y variable)").color(weak).size(11.0));
                                });
                            }
                            ui.horizontal(|ui| {
                                ui.label("Cobertura desde");
                                ui.add(NumField::new(&mut t.cover_height).range(0.0..=100.0).suffix(" m"));
                                ui.label(egui::RichText::new("altura a partir de la cual una celda cuenta como copa").color(weak).size(11.0));
                            });
                        });
                        if t.classify == 0 {
                            ui.label(egui::RichText::new("Los estratos salen de una clasificación: elige reglas, muestras o agrupar por parecido.").color(weak).size(11.0));
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Salida");
                    ui.checkbox(&mut t.vector, "polígonos (shp)");
                    ui.checkbox(&mut t.csv, "tabla (csv)");
                    ui.label("Simplificar");
                    ui.add(NumField::new(&mut t.simplify).range(0.0..=100.0).suffix(" m"));
                });
                if !t.out_manual {
                    t.out = t.chm.as_ref().map_or(String::new(), |f| {
                        paths::unique(&paths::product(f, "objetos", "shp")).display().to_string()
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Base del nombre");
                    if ui.add(egui::TextEdit::singleline(&mut t.out).desired_width(380.0)).changed() {
                        t.out_manual = true;
                    }
                    if ui.button("…").clicked()
                        && let Some(p) = crate::prefs::dialog().add_filter("Shapefile", &["shp"]).set_file_name("objetos.shp").save_file()
                    {
                        t.out = p.display().to_string();
                        t.out_manual = true;
                    }
                    if t.out_manual && ui.button("auto").clicked() {
                        t.out_manual = false;
                    }
                });
                let ready = t.chm.is_some()
                    && !t.out.trim().is_empty()
                    && (t.vector || t.csv)
                    && match t.classify {
                        1 => t.rules.is_some(),
                        2 => t.samples.is_some() && !t.class_field.trim().is_empty(),
                        _ => true,
                    };
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Calcular")).clicked() {
                    go = true;
                }
            });
        if go {
            let t = &self.ot;
            let Some(chm) = t.chm.clone() else { return };
            let mut weights = Vec::new();
            if t.roughness > 0 && t.rough_weight < 1.0 {
                weights.push(("rugosidad".to_string(), t.rough_weight as f32));
            }
            let method = if t.mask {
                Method::Mask {
                    layer: paths::stem(&chm),
                    min: t.mask_min as f32,
                    max: t.mask_max as f32,
                    diagonal: t.diagonal,
                }
            } else {
                Method::Merge(MergeParams {
                    scale: t.scale,
                    shape: t.shape,
                    compact: t.compact,
                    weights,
                    ..Default::default()
                })
            };
            let mut rasters = vec![chm];
            rasters.extend(t.extra.iter().cloned());
            let use_cloud = !clouds.is_empty();
            let job = ObiaJob {
                rasters,
                clouds: if use_cloud { clouds } else { Vec::new() },
                cloud_layers: CLOUD_LAYERS
                    .iter()
                    .zip(t.cloud)
                    .filter(|(_, on)| *on && use_cloud)
                    .map(|((l, _), _)| *l)
                    .collect(),
                filter: Filter::default(),
                roughness: t.roughness,
                segment: SegmentParams {
                    method,
                    min_cells: t.min_cells,
                },
                classify: match (t.classify, &t.rules, &t.samples) {
                    (1, Some(f), _) => ClassifyBy::Rules(f.clone()),
                    (2, _, Some(f)) => ClassifyBy::Samples {
                        file: f.clone(),
                        field: t.class_field.trim().to_string(),
                        forest: ForestParams {
                            trees: t.trees,
                            ..Default::default()
                        },
                    },
                    (3, _, _) => ClassifyBy::Kmeans {
                        k: t.k,
                        features: Vec::new(),
                        seed: 1,
                    },
                    _ => ClassifyBy::Nothing,
                },
                output: {
                    let o = PathBuf::from(t.out.trim());
                    if t.out_manual { o } else { paths::unique(&o) }
                },
                formats: Formats {
                    vector: t.vector,
                    csv: t.csv,
                },
                simplify: t.simplify,
                cover_height: t.cover_height,
                strata: (t.strata && t.classify != 0).then_some(StrataOpts {
                    min_cells: t.strata_min_cells,
                    plots: t.plots,
                }),
            };
            let outs = job.outputs();
            if t.out_manual
                && [&outs.polygons, &outs.table]
                    .iter()
                    .filter_map(|p| p.as_ref())
                    .any(|p| p.exists())
            {
                self.log("La salida elegida a mano ya existe: cámbiala o bórrala.");
            } else {
                let mut show: Vec<PathBuf> = Vec::new();
                show.extend(outs.strata_classes.clone());
                show.extend(outs.strata_polygons.clone());
                if outs.strata_classes.is_none() {
                    show.extend(outs.classes.clone());
                    show.extend(outs.polygons.clone());
                }
                let mut outputs = show.clone();
                outputs.extend(outs.classes);
                outputs.extend(outs.polygons);
                outputs.push(outs.objects);
                outputs.extend(outs.table);
                outputs.extend(outs.strata_table);
                let task: tools::Task = Box::new(move || rustlidar_core::jobs::obia(&job));
                self.start(
                    ctx,
                    Launch {
                        label: "OBIA".to_string(),
                        task,
                        show,
                        outputs,
                    },
                );
            }
        }
        if !open {
            self.ot.open = false;
        }
    }
}
