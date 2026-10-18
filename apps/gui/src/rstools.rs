//! Multiband images and mosaics made from rasters (Teledetección): the bands of several
//! rasters stacked into one image, and a mosaic that says what to do where pixels overlap.

use super::*;
use rustlidar_core::paths;
use rustlidar_core::rastertools::{self, Dtype, Extent, Overlap, RasterJob, Resample, Source};

pub struct RasterTools {
    /// 0: multiband image, 1: mosaic.
    pub open: Option<u8>,
    pub files: Vec<(PathBuf, String)>,
    pub overlap: Overlap,
    pub resample: Resample,
    pub extent: Extent,
    pub cell: String,
    /// 0: as the first raster, 1: 8 bits, 2: 16 bits, 3: 32-bit float.
    pub dtype: usize,
    pub src_nodata: String,
    pub nodata: String,
    pub out: String,
    pub out_manual: bool,
}

impl Default for RasterTools {
    fn default() -> Self {
        Self {
            open: None,
            files: Vec::new(),
            overlap: Overlap::First,
            resample: Resample::Nearest,
            extent: Extent::Union,
            cell: String::new(),
            dtype: 0,
            src_nodata: String::new(),
            nodata: "0".into(),
            out: String::new(),
            out_manual: false,
        }
    }
}

fn parse_bands(text: &str) -> Vec<usize> {
    text.split([',', ' ', ';'])
        .filter_map(|t| t.trim().parse().ok())
        .collect()
}

impl App {
    pub fn raster_tools_window(&mut self, ctx: &egui::Context) {
        let Some(mode) = self.rt.open else { return };
        let (title, tag) = if mode == 0 {
            ("Crear imagen multibanda", "multibanda")
        } else {
            ("Mosaico de rásteres", "mosaico")
        };
        // rasters already on the map
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. } | RefKind::Image(_)))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let mut open = true;
        let mut go = false;
        let weak = self.tokens().weak;
        egui::Window::new(title)
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                let rt = &mut self.rt;
                ui.label(
                    egui::RichText::new(if mode == 0 {
                        "Cada banda de cada ráster de la lista pasa a ser una banda de la imagen nueva, en el orden de la lista (como apilar canales sueltos R, G, B, NIR en un solo archivo). En «Bandas» se pueden elegir las de cada ráster (por ejemplo 1,3); vacío = todas."
                    } else {
                        "Une rásteres que se tocan o se solapan en uno solo. Dónde hay píxeles de varios, el valor sale de la regla elegida. Todos deben tener el mismo número de bandas."
                    })
                    .color(weak)
                    .size(11.0),
                );
                ui.horizontal(|ui| {
                    if ui.button("➕ Añadir archivos…").clicked()
                        && let Some(v) = rfd::FileDialog::new()
                            .add_filter("GeoTIFF", &["tif", "tiff"])
                            .pick_files()
                    {
                        for p in v {
                            if !rt.files.iter().any(|f| f.0 == p) {
                                rt.files.push((p, String::new()));
                            }
                        }
                    }
                    ui.menu_button("➕ De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                if !rt.files.iter().any(|f| &f.0 == path) {
                                    rt.files.push((path.clone(), String::new()));
                                }
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                    if ui.button("Vaciar lista").clicked() {
                        rt.files.clear();
                    }
                });
                let mut remove = None;
                let mut swap = None;
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    for k in 0..rt.files.len() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}.", k + 1));
                            if ui.small_button("▲").clicked() && k > 0 {
                                swap = Some((k - 1, k));
                            }
                            if ui.small_button("▼").clicked() && k + 1 < rt.files.len() {
                                swap = Some((k, k + 1));
                            }
                            if ui.small_button("🗑").clicked() {
                                remove = Some(k);
                            }
                            ui.label(paths::stem(&rt.files[k].0));
                            if mode == 0 {
                                ui.label("Bandas");
                                ui.add(
                                    egui::TextEdit::singleline(&mut rt.files[k].1)
                                        .desired_width(60.0)
                                        .hint_text("todas"),
                                );
                            }
                        });
                    }
                });
                if let Some((a, b)) = swap {
                    rt.files.swap(a, b);
                }
                if let Some(k) = remove {
                    rt.files.remove(k);
                }
                ui.separator();
                if mode == 1 {
                    egui::ComboBox::from_label("Donde se solapan")
                        .selected_text(rt.overlap.name())
                        .show_ui(ui, |ui| {
                            for o in Overlap::ALL {
                                ui.selectable_value(&mut rt.overlap, o, o.name());
                            }
                        });
                }
                egui::ComboBox::from_label("Zona que cubre")
                    .selected_text(match rt.extent {
                        Extent::Union => "Unión de todos",
                        Extent::Intersection => "Solo donde coinciden todos",
                        Extent::First => "La del primero",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut rt.extent, Extent::Union, "Unión de todos");
                        ui.selectable_value(&mut rt.extent, Extent::Intersection, "Solo donde coinciden todos");
                        ui.selectable_value(&mut rt.extent, Extent::First, "La del primero");
                    });
                egui::ComboBox::from_label("Remuestreo")
                    .selected_text(match rt.resample {
                        Resample::Nearest => "Vecino más cercano",
                        Resample::Bilinear => "Bilineal",
                        Resample::Average => "Media (para reducir)",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut rt.resample, Resample::Nearest, "Vecino más cercano");
                        ui.selectable_value(&mut rt.resample, Resample::Bilinear, "Bilineal");
                        ui.selectable_value(&mut rt.resample, Resample::Average, "Media (para reducir)");
                    });
                egui::ComboBox::from_label("Tipo de dato")
                    .selected_text(["Como el primero", "8 bits", "16 bits", "Decimal de 32 bits"][rt.dtype])
                    .show_ui(ui, |ui| {
                        for (k, n) in ["Como el primero", "8 bits", "16 bits", "Decimal de 32 bits"].iter().enumerate() {
                            ui.selectable_value(&mut rt.dtype, k, *n);
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Tamaño de píxel");
                    ui.add(egui::TextEdit::singleline(&mut rt.cell).desired_width(70.0).hint_text("el menor"));
                    ui.label("Sin datos en la entrada");
                    ui.add(egui::TextEdit::singleline(&mut rt.src_nodata).desired_width(60.0).hint_text("ninguno"));
                    ui.label("en la salida");
                    ui.add(egui::TextEdit::singleline(&mut rt.nodata).desired_width(60.0));
                });
                // the output: beside the first raster, named after the tool, until edited
                if !rt.out_manual {
                    rt.out = rt.files.first().map_or(String::new(), |f| {
                        let t = if mode == 1 { format!("{tag}_{}", rt.overlap.tag()) } else { tag.to_string() };
                        paths::unique(&paths::product(&f.0, &t, "tif")).display().to_string()
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Salida");
                    if ui.add(egui::TextEdit::singleline(&mut rt.out).desired_width(380.0)).changed() {
                        rt.out_manual = true;
                    }
                    if ui.button("…").clicked()
                        && let Some(p) = rfd::FileDialog::new().add_filter("GeoTIFF", &["tif"]).set_file_name("salida.tif").save_file()
                    {
                        rt.out = p.display().to_string();
                        rt.out_manual = true;
                    }
                    if rt.out_manual && ui.button("auto").clicked() {
                        rt.out_manual = false;
                    }
                });
                ui.label(egui::RichText::new("La salida se escribe por franjas comprimidas (deflate), sin cargar las imágenes enteras.").color(weak).size(11.0));
                let ready = !rt.files.is_empty() && !rt.out.trim().is_empty() && !(mode == 1 && rt.files.len() < 2);
                if ui.add_enabled(ready && self.running.is_none(), egui::Button::new("▶ Crear")).clicked() {
                    go = true;
                }
            });
        if go {
            let rt = &self.rt;
            let out = PathBuf::from(rt.out.trim());
            if out.exists() && rt.out_manual {
                self.log("La salida elegida a mano ya existe: cámbiala o bórrala.");
            } else {
                let out = if out.exists() {
                    paths::unique(&out)
                } else {
                    out
                };
                let mut job = RasterJob::new(
                    rt.files
                        .iter()
                        .map(|f| Source {
                            path: f.0.clone(),
                            bands: if mode == 0 {
                                parse_bands(&f.1)
                            } else {
                                Vec::new()
                            },
                        })
                        .collect(),
                    out.clone(),
                );
                job.overlap = rt.overlap;
                job.resample = rt.resample;
                job.extent = rt.extent;
                job.cell = rt.cell.trim().replace(',', ".").parse().ok();
                job.dtype = [None, Some(Dtype::U8), Some(Dtype::U16), Some(Dtype::F32)][rt.dtype];
                job.src_nodata = rt.src_nodata.trim().replace(',', ".").parse().ok();
                job.nodata = rt.nodata.trim().replace(',', ".").parse().unwrap_or(0.0);
                let task: tools::Task = Box::new(move || {
                    if mode == 0 {
                        rastertools::stack(&job)
                    } else {
                        rastertools::mosaic(&job)
                    }
                });
                self.start(
                    ctx,
                    Launch {
                        label: title.to_string(),
                        task,
                        show: vec![out.clone()],
                        outputs: vec![out],
                    },
                );
            }
        }
        if !open {
            self.rt.open = None;
        }
    }
}
