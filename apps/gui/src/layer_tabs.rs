//! The contextual ribbon tabs that appear when a layer is selected, as in a desktop GIS:
//! symbology and labels of a vector layer, its editing tools, and the rendering of a raster.

use super::*;
use crate::layers::VecLayer;
use crate::rs::{GridMode, GridStyle, Stretch, render_grid};
use crate::symbology::{RAMPS, SymKind, Symbology};

/// The names of the contextual tabs for the selected layer.
pub fn tabs_for(kind: Option<&RefKind>) -> &'static [&'static str] {
    match kind {
        Some(RefKind::Vector(_)) => &["Simbología", "Edición vectorial"],
        Some(_) => &["Ráster"],
        None => &[],
    }
}

fn percent(v: &mut f32, ui: &mut egui::Ui) -> egui::Response {
    ui.add(
        crate::numfield::NumField::new(v)
            .range(0.0..=1.0)
            .speed(0.01)
            .custom_formatter(|n, _| format!("{:.0} %", n * 100.0))
            .custom_parser(|s| {
                s.trim()
                    .trim_end_matches('%')
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .map(|p| p / 100.0)
            }),
    )
}

fn ramp_combo(ui: &mut egui::Ui, id: &str, ramp: &mut usize, width: f32) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(RAMPS[(*ramp).min(RAMPS.len() - 1)].0)
        .show_ui(ui, |ui| {
            for (k, (name, _)) in RAMPS.iter().enumerate() {
                changed |= ui.selectable_value(ramp, k, *name).changed();
            }
        });
    changed
}

impl App {
    pub fn layer_tab(&mut self, ui: &mut egui::Ui, t: &Tokens, ctx: &egui::Context, tab: usize) {
        let Some(k) = self.sel_layer.filter(|k| *k < self.refs.len()) else {
            ui.label("Elige una capa en el panel «Capas».");
            return;
        };
        match (&self.refs[k].kind, tab) {
            (RefKind::Vector(_), 5) => self.vector_symbology(ui, t, k),
            (RefKind::Vector(_), _) => self.vector_edit_tab(ui, t, k),
            (RefKind::Image(_), _) => self.image_tab(ui, t, ctx, k),
            (RefKind::Raster { .. }, _) => self.grid_tab(ui, t, ctx, k),
        }
    }

    fn vector_symbology(&mut self, ui: &mut egui::Ui, t: &Tokens, k: usize) {
        let mut zoom = false;
        let mut table = false;
        let r = &mut self.refs[k];
        let RefLayer {
            opacity,
            kind: RefKind::Vector(v),
            name,
            ..
        } = r
        else {
            return;
        };
        let mut classify = false;
        let numeric = Symbology::numeric_fields(&v.data);
        ribbon::group(ui, t, "Símbolo", |ui| {
            ui.vertical(|ui| {
                let before = v.sym.kind;
                egui::ComboBox::from_id_salt("sym_kind")
                    .width(150.0)
                    .selected_text(v.sym.kind.name())
                    .show_ui(ui, |ui| {
                        for kind in SymKind::ALL {
                            if kind == SymKind::Proportional && v.kind != view::GeomKind::Point {
                                continue;
                            }
                            ui.selectable_value(&mut v.sym.kind, kind, kind.name());
                        }
                    });
                if v.sym.kind != before {
                    if v.sym.kind != SymKind::Single && v.sym.field.is_none() {
                        v.sym.field = if v.sym.kind == SymKind::Unique {
                            (!v.data.fields.is_empty()).then_some(0)
                        } else {
                            numeric.first().copied()
                        };
                    }
                    if v.sym.kind != SymKind::Single && v.fill_alpha < 0.5 {
                        v.fill_alpha = 0.8;
                    }
                    classify = true;
                }
                ui.add_enabled_ui(v.sym.kind != SymKind::Single, |ui| {
                    let text = v
                        .sym
                        .field
                        .and_then(|c| v.data.fields.get(c))
                        .map_or("Campo…".to_string(), Clone::clone);
                    egui::ComboBox::from_id_salt("sym_field")
                        .width(150.0)
                        .selected_text(text)
                        .show_ui(ui, |ui| {
                            let all: Vec<usize> = if v.sym.kind == SymKind::Unique {
                                (0..v.data.fields.len()).collect()
                            } else {
                                numeric.clone()
                            };
                            for c in all {
                                if ui
                                    .selectable_value(&mut v.sym.field, Some(c), &v.data.fields[c])
                                    .changed()
                                {
                                    classify = true;
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(v.sym.kind == SymKind::Graduated, |ui| {
                        ui.label("Clases");
                        classify |= ui
                            .add(crate::numfield::NumField::new(&mut v.sym.classes).range(2..=12))
                            .changed();
                        classify |= ui.checkbox(&mut v.sym.quantile, "Cuantiles").changed();
                    });
                });
            });
        });
        ribbon::group(ui, t, "Colores", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(if v.kind == view::GeomKind::Point {
                        "Color"
                    } else {
                        "Trazo"
                    });
                    egui::color_picker::color_edit_button_srgb(ui, &mut v.color);
                    if v.kind == view::GeomKind::Polygon {
                        ui.label("Relleno");
                        egui::color_picker::color_edit_button_srgb(ui, &mut v.fill);
                    }
                });
                let coloured = matches!(v.sym.kind, SymKind::Graduated | SymKind::Unique);
                ui.add_enabled_ui(coloured, |ui| {
                    ui.horizontal(|ui| {
                        classify |= ramp_combo(ui, "sym_ramp", &mut v.sym.ramp, 120.0);
                        classify |= ui.checkbox(&mut v.sym.invert, "Invertir").changed();
                    });
                });
                ui.add_enabled_ui(v.sym.kind == SymKind::Proportional, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Radio");
                        ui.add(crate::numfield::NumField::new(&mut v.sym.size.0).range(1.0..=40.0));
                        ui.label("a");
                        ui.add(crate::numfield::NumField::new(&mut v.sym.size.1).range(1.0..=60.0));
                    });
                });
            });
        });
        ribbon::group(ui, t, "Aspecto", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Opacidad");
                    percent(opacity, ui);
                });
                if v.kind == view::GeomKind::Point {
                    ui.horizontal(|ui| {
                        ui.label("Tamaño");
                        ui.add(crate::numfield::NumField::new(&mut v.point_size).range(1.0..=40.0));
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Grosor");
                        ui.add(
                            crate::numfield::NumField::new(&mut v.width)
                                .range(0.5..=12.0)
                                .speed(0.1),
                        );
                    });
                }
                if v.kind == view::GeomKind::Polygon {
                    ui.horizontal(|ui| {
                        ui.label("Relleno");
                        percent(&mut v.fill_alpha, ui);
                    });
                }
            });
        });
        ribbon::group(ui, t, "Etiquetas", |ui| {
            ui.vertical(|ui| {
                let text = v
                    .labels
                    .field
                    .and_then(|c| v.data.fields.get(c))
                    .map_or("Sin etiquetas".to_string(), Clone::clone);
                egui::ComboBox::from_id_salt("lab_field")
                    .width(150.0)
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut v.labels.field, None, "Sin etiquetas");
                        for (c, f) in v.data.fields.iter().enumerate() {
                            ui.selectable_value(&mut v.labels.field, Some(c), f);
                        }
                    });
                ui.add_enabled_ui(v.labels.field.is_some(), |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            crate::numfield::NumField::new(&mut v.labels.size)
                                .range(7.0..=40.0)
                                .suffix(" pt"),
                        );
                        egui::color_picker::color_edit_button_srgb(ui, &mut v.labels.color);
                        ui.checkbox(&mut v.labels.halo, "Halo");
                    });
                });
            });
        });
        ribbon::group(ui, t, "Capa", |ui| {
            if ribbon::big_button(ui, t, "@full", "Zoom a\nla capa", false).clicked() {
                zoom = true;
            }
            if ribbon::big_button(ui, t, "@table", "Tabla de\natributos", false).clicked() {
                table = true;
            }
            ui.label(egui::RichText::new(&*name).weak());
        });
        if classify {
            v.reclassify();
        }
        if zoom {
            self.zoom_to_layer(k);
        }
        if table {
            self.table_layer = Some(k);
        }
    }

    fn vector_edit_tab(&mut self, ui: &mut egui::Ui, t: &Tokens, k: usize) {
        self.active = Some(k);
        let (has_sel, can_undo, can_redo, dirty) =
            self.active_vec().map_or((false, false, false, false), |v| {
                (
                    !v.selected.is_empty(),
                    !v.undo.is_empty(),
                    !v.redo.is_empty(),
                    v.dirty,
                )
            });
        ribbon::group(ui, t, "Capa", |ui| {
            if ribbon::big_button(ui, t, "@newlayer", "Nueva\ncapa", false).clicked() {
                self.new_layer = Some(layers::NewLayerDlg::default());
            }
            if ribbon::big_button(
                ui,
                t,
                "@save",
                if dirty { "Guardar *" } else { "Guardar" },
                dirty,
            )
            .clicked()
            {
                self.save_layer(k, false);
            }
            ui.vertical(|ui| {
                if ribbon::small_button(ui, t, "@save", "Guardar como…", false).clicked() {
                    self.save_layer(k, true);
                }
                if ribbon::small_button(
                    ui,
                    t,
                    "@table",
                    "Tabla de atributos",
                    self.table_layer == Some(k),
                )
                .clicked()
                {
                    self.table_layer = Some(k);
                }
            });
        });
        ribbon::group(ui, t, "Herramientas", |ui| {
            for (icon, name, n) in [
                ("@pencil", "Digitalizar", Nav::Digitize),
                ("@selrect", "Seleccionar", Nav::VecSelect),
                ("@vertex", "Vértices", Nav::VecVertex),
                ("@move", "Mover", Nav::VecMove),
            ] {
                if ribbon::big_button(ui, t, icon, name, self.nav == n).clicked() {
                    self.nav = n;
                    self.dig_draft.clear();
                }
            }
            ui.vertical(|ui| {
                ui.checkbox(&mut self.vedit.snap, "Ajustar a vértices");
            });
        });
        ribbon::group(ui, t, "Selección", |ui| {
            ui.vertical(|ui| {
                if ribbon::small_button(ui, t, "@selall", "Seleccionar todo", false).clicked()
                    && let Some(v) = self.active_vec_mut()
                {
                    v.selected = (0..v.data.features.len()).collect();
                }
                if ribbon::small_button(ui, t, "@invert", "Invertir selección", false).clicked()
                    && let Some(v) = self.active_vec_mut()
                {
                    v.selected = (0..v.data.features.len())
                        .filter(|i| !v.selected.contains(i))
                        .collect();
                }
                if ribbon::small_button(ui, t, "@clear", "Limpiar selección", false).clicked() {
                    self.clear_selection();
                }
            });
            ui.vertical(|ui| {
                if ui
                    .add_enabled(has_sel, egui::Button::new("Zoom a la selección"))
                    .clicked()
                    && let Some(b) = self.active_vec().and_then(VecLayer::selection_bounds)
                {
                    self.zoom_to_bounds(b);
                }
                if ui
                    .add_enabled(has_sel, egui::Button::new("Eliminar seleccionados"))
                    .clicked()
                {
                    self.delete_selected();
                }
            });
        });
        ribbon::group(ui, t, "Historial", |ui| {
            if ui
                .add_enabled(can_undo, egui::Button::new("↶ Deshacer"))
                .clicked()
            {
                self.vec_undo(false);
            }
            if ui
                .add_enabled(can_redo, egui::Button::new("↷ Rehacer"))
                .clicked()
            {
                self.vec_undo(true);
            }
        });
    }

    fn image_tab(&mut self, ui: &mut egui::Ui, t: &Tokens, ctx: &egui::Context, k: usize) {
        let (mut zoom, mut indices) = (false, false);
        let r = &mut self.refs[k];
        let RefLayer {
            opacity,
            kind: RefKind::Image(il),
            name,
            ..
        } = r
        else {
            return;
        };
        let nb = il.work.bands.len();
        let mut changed = false;
        ribbon::group(ui, t, "Bandas", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    for (label, bands, need) in [
                        ("Color natural", [0, 1, 2], 3),
                        ("Infrarrojo", [3, 0, 1], 4),
                        ("Gris", [0, 0, 0], 1),
                    ] {
                        if ui
                            .add_enabled(
                                nb >= need,
                                egui::Button::new(label).selected(il.bands == bands),
                            )
                            .clicked()
                        {
                            il.bands = bands;
                            changed = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    for (c, label) in ["R", "V", "A"].iter().enumerate() {
                        ui.label(*label);
                        egui::ComboBox::from_id_salt(("band", c))
                            .width(62.0)
                            .selected_text(format!("B{}", il.bands[c] + 1))
                            .show_ui(ui, |ui| {
                                for b in 0..nb {
                                    changed |= ui
                                        .selectable_value(
                                            &mut il.bands[c],
                                            b,
                                            format!("Banda {}", b + 1),
                                        )
                                        .changed();
                                }
                            });
                    }
                });
            });
        });
        ribbon::group(ui, t, "Estirado", |ui| {
            ui.vertical(|ui| {
                let before = il.style();
                let was = il.stretch;
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("stretch")
                        .width(160.0)
                        .selected_text(il.stretch.name())
                        .show_ui(ui, |ui| {
                            for s in Stretch::ALL {
                                changed |=
                                    ui.selectable_value(&mut il.stretch, s, s.name()).changed();
                            }
                        });
                    if il.stretch == Stretch::Manual {
                        // the first time, start from the limits that were showing
                        if was != Stretch::Manual {
                            il.manual = [before.lo[0], before.hi[0]];
                        }
                        ui.label("Mín");
                        changed |= ui
                            .add(crate::numfield::NumField::new(&mut il.manual[0]).speed(1.0))
                            .changed();
                        ui.label("Máx");
                        changed |= ui
                            .add(crate::numfield::NumField::new(&mut il.manual[1]).speed(1.0))
                            .changed();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Brillo");
                    changed |= ui
                        .add(
                            crate::numfield::NumField::new(&mut il.brightness)
                                .range(-0.5..=0.5)
                                .speed(0.01)
                                .fixed_decimals(2),
                        )
                        .changed();
                    ui.label("Contraste");
                    changed |= ui
                        .add(
                            crate::numfield::NumField::new(&mut il.contrast)
                                .range(0.3..=3.0)
                                .speed(0.01)
                                .fixed_decimals(2),
                        )
                        .changed();
                    ui.label("Gamma");
                    changed |= ui
                        .add(
                            crate::numfield::NumField::new(&mut il.gamma)
                                .range(0.3..=3.0)
                                .speed(0.01)
                                .fixed_decimals(2),
                        )
                        .changed();
                });
                if ui.button("Restablecer").clicked() {
                    (il.brightness, il.contrast, il.gamma) = (0.0, 1.0, 1.0);
                    changed = true;
                }
            });
        });
        ribbon::group(ui, t, "Capa", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Opacidad");
                    percent(opacity, ui);
                });
                ui.label(
                    egui::RichText::new(format!(
                        "{nb} bandas · {} niveles · {:.2} m/píxel",
                        il.pyr.levels.len(),
                        il.pyr.cell
                    ))
                    .size(11.0),
                );
                ui.label(egui::RichText::new(&*name).weak().size(11.0));
            });
            if ribbon::big_button(ui, t, "@full", "Zoom a\nla capa", false).clicked() {
                zoom = true;
            }
            if ribbon::big_button(ui, t, "@bands", "Índices\nespectrales", false).clicked() {
                indices = true;
            }
        });
        if changed {
            let n = name.clone();
            il.restyle(ctx, &n);
        }
        if zoom {
            self.zoom_to_layer(k);
        }
        if indices {
            self.rs.open = true;
            self.rs.layer = k;
        }
    }

    fn grid_tab(&mut self, ui: &mut egui::Ui, t: &Tokens, ctx: &egui::Context, k: usize) {
        let mut zoom = false;
        let r = &mut self.refs[k];
        let RefLayer {
            opacity,
            kind: RefKind::Raster { grid, tex, style },
            name,
            ..
        } = r
        else {
            return;
        };
        let mut changed = false;
        ribbon::group(ui, t, "Representación", |ui| {
            ui.vertical(|ui| {
                egui::ComboBox::from_id_salt("grid_mode")
                    .width(150.0)
                    .selected_text(style.mode.name())
                    .show_ui(ui, |ui| {
                        for m in GridMode::ALL {
                            changed |= ui.selectable_value(&mut style.mode, m, m.name()).changed();
                        }
                    });
                ui.horizontal(|ui| {
                    changed |= ramp_combo(ui, "grid_ramp", &mut style.ramp, 120.0);
                    changed |= ui.checkbox(&mut style.invert, "Invertir").changed();
                });
            });
        });
        ribbon::group(ui, t, "Valores", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Mín");
                    changed |= ui
                        .add(crate::numfield::NumField::new(&mut style.lo).speed(0.05))
                        .changed();
                    ui.label("Máx");
                    changed |= ui
                        .add(crate::numfield::NumField::new(&mut style.hi).speed(0.05))
                        .changed();
                });
                ui.horizontal(|ui| {
                    if ui.button("Percentiles 2–98 %").clicked() {
                        (style.lo, style.hi) = rustlidar_core::raster::stretch(&grid.data);
                        changed = true;
                    }
                    if ui.button("Mín–máx").clicked() {
                        let finite = grid.data.iter().copied().filter(|v| v.is_finite());
                        let (lo, hi) = finite.fold((f32::INFINITY, f32::NEG_INFINITY), |a, v| {
                            (a.0.min(v), a.1.max(v))
                        });
                        if lo.is_finite() {
                            (style.lo, style.hi) = (lo, hi);
                            changed = true;
                        }
                    }
                });
                ui.add_enabled_ui(style.mode == GridMode::Classified, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Clases");
                        changed |= ui
                            .add(crate::numfield::NumField::new(&mut style.classes).range(2..=20))
                            .changed();
                    });
                });
            });
        });
        ribbon::group(ui, t, "Relieve", |ui| {
            ui.add_enabled_ui(style.mode == GridMode::Hillshade, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label("Azimut");
                        changed |= ui
                            .add(
                                crate::numfield::NumField::new(&mut style.azimuth)
                                    .range(0.0..=360.0)
                                    .suffix("°"),
                            )
                            .changed();
                        ui.label("Altura");
                        changed |= ui
                            .add(
                                crate::numfield::NumField::new(&mut style.altitude)
                                    .range(1.0..=90.0)
                                    .suffix("°"),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Exageración");
                        changed |= ui
                            .add(
                                crate::numfield::NumField::new(&mut style.zfactor)
                                    .range(0.1..=20.0)
                                    .speed(0.05),
                            )
                            .changed();
                        ui.label("Color");
                        changed |= percent(&mut style.blend, ui).changed();
                    });
                });
            });
        });
        ribbon::group(ui, t, "Capa", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label("Opacidad");
                    percent(opacity, ui);
                });
                ui.label(egui::RichText::new(&*name).weak().size(11.0));
            });
            if ribbon::big_button(ui, t, "@full", "Zoom a\nla capa", false).clicked() {
                zoom = true;
            }
        });
        if changed {
            let st: GridStyle = style.clone();
            *tex = to_texture(
                ctx,
                &format!("ref-{name}"),
                &render_grid(grid, &st),
                egui::TextureOptions::LINEAR,
            );
        }
        if zoom {
            self.zoom_to_layer(k);
        }
    }
}
