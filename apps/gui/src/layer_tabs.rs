//! What the side panel shows for the layer selected in the contents: its editing tools (the
//! Edición tab) and its symbology and rendering (the Simbología tab).

use super::*;
use crate::layers::VecLayer;
use crate::rs::{GridMode, GridStyle, Stretch, render_grid};
use crate::symbology::{RAMPS, SymKind, Symbology};

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
    fn vector_symbology(&mut self, ui: &mut egui::Ui, t: &Tokens, k: usize) {
        let r = &mut self.refs[k];
        let RefLayer {
            opacity,
            kind: RefKind::Vector(v),
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
                        let polygon = v.kind == view::GeomKind::Polygon;
                        ui.label(if polygon {
                            "Grosor (0 = sin borde)"
                        } else {
                            "Grosor"
                        });
                        ui.add(
                            crate::numfield::NumField::new(&mut v.width)
                                .range(if polygon { 0.0..=12.0 } else { 0.5..=12.0 })
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
        if classify {
            v.reclassify();
        }
    }

    /// The tools of editing a vector layer, for the side panel: sections of buttons that wrap.
    fn vector_edit_tab(&mut self, ui: &mut egui::Ui, t: &Tokens, k: usize) {
        self.active = Some(k);
        let mut toggle: Option<Nav> = None;
        let (has_sel, can_undo, can_redo, dirty) =
            self.active_vec().map_or((false, false, false, false), |v| {
                (
                    !v.selected.is_empty(),
                    !v.undo.is_empty(),
                    !v.redo.is_empty(),
                    v.dirty,
                )
            });
        let polygons = self.active_vec().is_some_and(|v| v.kind == view::GeomKind::Polygon);
        ribbon::group(ui, t, "Capa", |ui| {
            ui.horizontal_wrapped(|ui| {
                if ribbon::small_button(ui, t, "@newlayer", "Nueva capa", false).clicked() {
                    self.new_layer = Some(layers::NewLayerDlg::default());
                }
                if ribbon::small_button(ui, t, "@save", if dirty { "Guardar *" } else { "Guardar" }, dirty).clicked() {
                    self.save_layer(k, false);
                }
                if ribbon::small_button(ui, t, "@save", "Guardar como…", false).clicked() {
                    self.save_layer(k, true);
                }
            });
        });
        ribbon::group(ui, t, "Edición", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (icon, name, n) in [
                    ("@pencil", "Digitalizar", Nav::Digitize),
                    ("@vertex", "Vértices", Nav::VecVertex),
                    ("@move", "Mover", Nav::VecMove),
                ] {
                    if ribbon::small_button(ui, t, icon, name, self.nav == n).clicked() {
                        toggle = Some(n);
                    }
                }
            });
            ui.checkbox(&mut self.vedit.snap, "Ajustar a vértices");
            ui.horizontal_wrapped(|ui| {
                if ribbon::small_button_on(ui, t, "@undo", "Deshacer", false, can_undo).clicked() {
                    self.vec_undo(false);
                }
                if ribbon::small_button_on(ui, t, "@redo", "Rehacer", false, can_redo).clicked() {
                    self.vec_undo(true);
                }
            });
        });
        ribbon::group(ui, t, "Selección", |ui| {
            ui.horizontal_wrapped(|ui| {
                if ribbon::small_button(ui, t, "@selrect", "Seleccionar", self.nav == Nav::VecSelect).clicked() {
                    toggle = Some(Nav::VecSelect);
                }
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
                if ribbon::small_button(ui, t, "@deselect", "Limpiar selección", false).clicked() {
                    self.clear_selection();
                }
                if ribbon::small_button(ui, t, "@locsel", "Seleccionar por localización", false).clicked() {
                    self.vm.loc = Some(crate::vecmore::LocDlg {
                        other: None,
                        rel: hipso_core::vecops::Relation::Intersects,
                        mode: 0,
                        msg: String::new(),
                    });
                }
                if ribbon::small_button(ui, t, "@exprsel", "Seleccionar por expresión", false).clicked() {
                    self.vm.expr = Some(crate::vecmore::ExprDlg { expr: String::new(), mode: 0, msg: String::new() });
                }
                if ribbon::small_button_on(ui, t, "@zoomsel", "Zoom a la selección", false, has_sel).clicked()
                    && let Some(b) = self.active_vec().and_then(VecLayer::selection_bounds)
                {
                    self.zoom_to_bounds(b);
                }
            });
        });
        ribbon::group(ui, t, "Geometría y atributos", |ui| {
            ui.horizontal_wrapped(|ui| {
                if ribbon::small_button_on(ui, t, "@merge", "Unir seleccionados", false, has_sel && polygons).clicked() {
                    self.merge_selected();
                }
                if ribbon::small_button_on(ui, t, "@explode", "Separar partes", false, has_sel).clicked() {
                    self.explode_selected();
                }
                if ribbon::small_button_on(ui, t, "@multipart", "Agrupar en multiparte", false, has_sel).clicked() {
                    self.gather_selected();
                }
                if ribbon::small_button_on(ui, t, "@trash", "Eliminar seleccionados", false, has_sel).clicked() {
                    self.delete_selected();
                }
                if ribbon::small_button(ui, t, "@table", "Tabla de atributos", self.table_layer == Some(k)).clicked() {
                    self.table_layer = Some(k);
                }
                if ribbon::small_button(ui, t, "@calc", "Calculadora de campos", false).clicked() {
                    self.vm.calc = Some(crate::vecmore::CalcDlg {
                        field: String::new(),
                        expr: String::new(),
                        only_selected: false,
                        msg: String::new(),
                    });
                }
            });
        });
        if let Some(n) = toggle {
            self.toggle_edit_tool(n);
        }
    }

    /// The Edición tab of the side panel: the tools of the layer selected in the contents (a
    /// vector layer), else those of editing point clouds.
    pub fn edit_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        let Some(k) = self.sel_layer.filter(|k| *k < self.refs.len()) else {
            self.edit_side(ui, ctx);
            return;
        };
        if !matches!(self.refs[k].kind, RefKind::Vector(_)) {
            ui.label("Las capas ráster y de imagen no tienen herramientas de edición. Elige una capa vectorial, o una nube de puntos, en «Contenido».");
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ribbon::panel_groups(ui, true);
            self.vector_edit_tab(ui, &t, k);
            ribbon::panel_groups(ui, false);
        });
    }

    /// The Simbología tab of the side panel: how the selected layer is drawn.
    pub fn symbology_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        let Some(k) = self.sel_layer.filter(|k| *k < self.refs.len()) else {
            ui.label("Elige una capa vectorial, ráster o de imagen en «Contenido» para ver cómo se dibuja.");
            return;
        };
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            ribbon::panel_groups(ui, true);
            match &self.refs[k].kind {
                RefKind::Vector(_) => self.vector_symbology(ui, &t, k),
                RefKind::Image(_) => self.image_tab(ui, &t, ctx, k),
                RefKind::Raster { .. } => self.grid_tab(ui, &t, ctx, k),
            }
            ribbon::panel_groups(ui, false);
        });
    }

    fn image_tab(&mut self, ui: &mut egui::Ui, t: &Tokens, ctx: &egui::Context, k: usize) {
        let mut indices = false;
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
                    for (icon, label, bands, need) in [
                        ("@image", "Color natural", [0, 1, 2], 3),
                        ("@tree", "Infrarrojo", [3, 0, 1], 4),
                        ("@stack", "Gris", [0, 0, 0], 1),
                    ] {
                        if ribbon::small_button_on(ui, t, icon, label, il.bands == bands, nb >= need).clicked() {
                            il.bands = bands;
                            changed = true;
                        }
                    }
                    if ribbon::small_button(ui, t, "@bands", "Índices espectrales", false).clicked() {
                        indices = true;
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
                if ribbon::small_button(ui, t, "@clear", "Restablecer", false).clicked() {
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
        });
        if changed {
            let n = name.clone();
            il.restyle(ctx, &n);
        }
        if indices {
            self.rs.open = true;
            self.rs.layer = k;
        }
    }

    fn grid_tab(&mut self, ui: &mut egui::Ui, t: &Tokens, ctx: &egui::Context, k: usize) {
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
                    if ribbon::small_button(ui, t, "@hist", "Percentiles 2–98 %", false).clicked() {
                        (style.lo, style.hi) = hipso_core::raster::stretch(&grid.data);
                        changed = true;
                    }
                    if ribbon::small_button(ui, t, "@full", "Mín–máx", false).clicked() {
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
    }
}
