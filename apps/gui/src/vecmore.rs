//! More tools for editing vector layers: join the selected polygons, split and gather the parts
//! of features, select by location, the field calculator, and the question asked before a
//! GeoPackage is rewritten with tables HIPSO does not keep.

use super::*;
use hipso_core::{fieldcalc, geomops, vecops};
use std::collections::HashSet;

pub struct CalcDlg {
    pub field: String,
    pub expr: String,
    pub only_selected: bool,
    pub msg: String,
}

pub struct LocDlg {
    /// The layer to compare with.
    pub other: Option<usize>,
    pub rel: vecops::Relation,
    /// 0: new selection, 1: add to it, 2: remove from it.
    pub mode: u8,
    pub msg: String,
}

pub struct ExprDlg {
    pub expr: String,
    /// 0: new selection, 1: add to it, 2: remove from it.
    pub mode: u8,
    pub msg: String,
}

#[derive(Default)]
pub struct State {
    pub calc: Option<CalcDlg>,
    pub expr: Option<ExprDlg>,
    pub loc: Option<LocDlg>,
    /// A layer to save over its GeoPackage and what the file holds that would be lost.
    pub gpkg_loss: Option<(usize, Vec<String>)>,
}

impl App {
    /// Joins the selected polygons into one that takes the place and the attributes of the first
    /// of them.
    pub fn merge_selected(&mut self) {
        let Some(v) = self.active_vec_mut() else { return };
        if v.kind != view::GeomKind::Polygon {
            self.log("Unir solo vale para capas de polígonos.");
            return;
        }
        let mut idx: Vec<usize> = v.selected.iter().copied().filter(|i| *i < v.data.features.len()).collect();
        idx.sort_unstable();
        if idx.len() < 2 {
            self.log("Selecciona al menos dos polígonos para unirlos.");
            return;
        }
        v.push_undo();
        let sets: Vec<&[Vec<[f64; 2]>]> = idx.iter().map(|&i| v.data.features[i].parts.as_slice()).collect();
        let parts = geomops::merge_parts(&sets);
        v.data.features[idx[0]].parts = parts;
        let rest: HashSet<usize> = idx[1..].iter().copied().collect();
        v.remove_features(&rest);
        v.selected.insert(idx[0]);
        let n = idx.len();
        self.log(format!("{n} polígonos unidos en uno (con los atributos del primero)."));
    }

    /// Splits each selected feature that has several parts into one feature per part.
    pub fn explode_selected(&mut self) {
        let Some(v) = self.active_vec_mut() else { return };
        let kind = v.kind;
        let any = v
            .selected
            .iter()
            .filter_map(|&i| v.data.features.get(i))
            .any(|f| vecops::split_parts(&f.parts, kind).len() > 1);
        if !any {
            self.log("Ninguno de los elementos seleccionados tiene más de una parte.");
            return;
        }
        v.push_undo();
        let which = v.selected.clone();
        let (n, sel) = vecops::explode(&mut v.data, kind, &which);
        v.rebuild();
        v.selected = sel;
        v.dirty = true;
        v.reclassify();
        self.log(format!("{n} elemento(s) separados en sus partes."));
    }

    /// Gathers the selected features into one that has all their parts (the geometry is not
    /// joined); it keeps the attributes of the first.
    pub fn gather_selected(&mut self) {
        let Some(v) = self.active_vec_mut() else { return };
        if v.selected.len() < 2 {
            self.log("Selecciona al menos dos elementos para agruparlos.");
            return;
        }
        v.push_undo();
        let which = v.selected.clone();
        let Some(first) = vecops::gather(&mut v.data, &which) else { return };
        v.rebuild();
        v.selected = [first].into_iter().collect();
        v.dirty = true;
        v.reclassify();
        let n = which.len();
        self.log(format!("{n} elementos agrupados en uno con varias partes."));
    }

    pub fn vecmore_windows(&mut self, ctx: &egui::Context) {
        self.calc_window(ctx);
        self.expr_window(ctx);
        self.loc_window(ctx);
        self.gpkg_loss_window(ctx);
    }

    fn calc_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.vm.calc.take() else { return };
        let Some((fields, nsel, kind)) = self.active_vec().map(|v| (v.data.fields.clone(), v.selected.len(), v.kind)) else {
            return;
        };
        let weak = self.tokens().weak;
        let (mut open, mut test, mut apply) = (true, false, false);
        egui::Window::new("Calculadora de campos")
            .open(&mut open)
            .collapsible(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Campo");
                    ui.add(egui::TextEdit::singleline(&mut d.field).desired_width(150.0));
                    ui.menu_button("Existentes", |ui| {
                        for f in &fields {
                            if ui.button(f).clicked() {
                                d.field = f.clone();
                                ui.close();
                            }
                        }
                    });
                    ui.label(egui::RichText::new("Si no existe, se crea.").color(weak).size(11.0));
                });
                ui.label("Expresión");
                ui.add(egui::TextEdit::multiline(&mut d.expr).desired_rows(3).desired_width(f32::INFINITY).code_editor());
                ui.horizontal(|ui| {
                    ui.menu_button("Variables", |ui| {
                        for (v, what) in [
                            ("$area", "superficie del polígono"),
                            ("$perimeter", "perímetro del polígono"),
                            ("$length", "longitud de la línea"),
                            ("$x", "coordenada X (centroide)"),
                            ("$y", "coordenada Y (centroide)"),
                            ("$id", "número de fila"),
                            ("$npoints", "número de vértices"),
                        ] {
                            if ui.button(format!("{v}  ·  {what}")).clicked() {
                                d.expr.push_str(v);
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Campos", |ui| {
                        for f in &fields {
                            if ui.button(f).clicked() {
                                d.expr.push_str(&format!("\"{f}\""));
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Funciones", |ui| {
                        for f in [
                            "round(x, 2)", "floor(x)", "ceil(x)", "abs(x)", "sqrt(x)", "ln(x)", "log10(x)", "pow(x, y)", "min(a, b)", "max(a, b)",
                            "upper(t)", "lower(t)", "trim(t)", "len(t)", "concat(a, b)", "left(t, n)", "right(t, n)", "substr(t, desde, n)",
                            "replace(t, 'a', 'b')", "if(cond, si, no)", "coalesce(a, b)",
                        ] {
                            if ui.button(f).clicked() {
                                d.expr.push_str(f);
                                ui.close();
                            }
                        }
                    });
                });
                ui.add_enabled(nsel > 0, egui::Checkbox::new(&mut d.only_selected, format!("Solo los {nsel} seleccionados")));
                let units = if kind == view::GeomKind::Polygon { "$area está en las unidades de la capa (m² en un sistema proyectado; 1 ha = 10000 m²)." } else { "Las longitudes están en las unidades de la capa." };
                ui.label(egui::RichText::new(format!("Ejemplos:  $area / 10000   ·   round($length, 1)   ·   \"especie\" || '-' || $id   ·   if(\"n\" > 10, 'alto', 'bajo')\n{units}")).color(weak).size(11.0));
                ui.horizontal(|ui| {
                    test = ui.button("Probar").clicked();
                    apply = ui.add_enabled(!d.expr.trim().is_empty() && !d.field.trim().is_empty(), egui::Button::new("▶ Calcular")).clicked();
                });
                if !d.msg.is_empty() {
                    ui.label(&d.msg);
                }
            });
        if let Some(v) = self.active_vec_mut() {
            let only: Option<HashSet<usize>> = (d.only_selected && !v.selected.is_empty()).then(|| v.selected.clone());
            if test {
                let few: HashSet<usize> = (0..v.data.features.len()).filter(|i| only.as_ref().is_none_or(|s| s.contains(i))).take(3).collect();
                d.msg = match fieldcalc::evaluate(&v.data, v.kind, &d.expr, Some(&few)) {
                    Ok(r) => r.iter().map(|(i, t)| format!("fila {} → {}", i + 1, if t.is_empty() { "(vacío)" } else { t })).collect::<Vec<_>>().join("   ·   "),
                    Err(e) => format!("Error: {e}"),
                };
            }
            if apply {
                v.push_undo();
                match fieldcalc::apply(&mut v.data, v.kind, &d.field, &d.expr, only.as_ref()) {
                    Ok(n) => {
                        v.dirty = true;
                        v.reclassify();
                        d.msg = format!("{n} elementos calculados en «{}».", d.field.trim());
                    }
                    Err(e) => {
                        v.undo.pop();
                        d.msg = format!("Error: {e}");
                    }
                }
            }
        }
        if open {
            self.vm.calc = Some(d);
        }
    }

    fn expr_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.vm.expr.take() else { return };
        let Some((fields, kind)) = self.active_vec().map(|v| (v.data.fields.clone(), v.kind)) else {
            return;
        };
        let weak = self.tokens().weak;
        let (mut open, mut go) = (true, false);
        egui::Window::new("Seleccionar por expresión")
            .open(&mut open)
            .collapsible(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label("Se seleccionan los elementos para los que la expresión es verdadera");
                ui.add(egui::TextEdit::multiline(&mut d.expr).desired_rows(3).desired_width(f32::INFINITY).code_editor());
                ui.horizontal(|ui| {
                    ui.menu_button("Campos", |ui| {
                        for f in &fields {
                            if ui.button(f).clicked() {
                                d.expr.push_str(&format!("\"{f}\""));
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Variables", |ui| {
                        for v in ["$area", "$perimeter", "$length", "$x", "$y", "$id", "$npoints"] {
                            if ui.button(v).clicked() {
                                d.expr.push_str(v);
                                ui.close();
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.mode, 0, "Selección nueva");
                    ui.radio_value(&mut d.mode, 1, "Añadir a la selección");
                    ui.radio_value(&mut d.mode, 2, "Quitar de la selección");
                });
                let area = if kind == view::GeomKind::Polygon { "   ·   $area > 10000   ·   \"n\" >= 3 and $area < 5000" } else { "" };
                ui.label(egui::RichText::new(format!("Ejemplos:  \"especie\" = 'Pino'   ·   \"n\" > 10 or \"n\" = 0{area}")).color(weak).size(11.0));
                go = ui.add_enabled(!d.expr.trim().is_empty(), egui::Button::new("▶ Seleccionar")).clicked();
                if !d.msg.is_empty() {
                    ui.label(&d.msg);
                }
            });
        if go && let Some(v) = self.active_vec_mut() {
            match fieldcalc::select(&v.data, v.kind, &d.expr) {
                Ok(hits) => {
                    d.msg = format!("{} elementos cumplen la expresión.", hits.len());
                    match d.mode {
                        0 => v.selected = hits.into_iter().collect(),
                        1 => v.selected.extend(hits),
                        _ => {
                            for i in hits {
                                v.selected.remove(&i);
                            }
                        }
                    }
                }
                Err(e) => d.msg = format!("Error: {e}"),
            }
        }
        if open {
            self.vm.expr = Some(d);
        }
    }

    fn loc_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.vm.loc.take() else { return };
        let Some(k) = self.active else { return };
        let others: Vec<(usize, String)> = self
            .refs
            .iter()
            .enumerate()
            .filter(|(i, r)| *i != k && matches!(r.kind, RefKind::Vector(_)))
            .map(|(i, r)| (i, r.name.clone()))
            .collect();
        if d.other.is_none_or(|o| !others.iter().any(|x| x.0 == o)) {
            d.other = others.first().map(|x| x.0);
        }
        let (mut open, mut go) = (true, false);
        egui::Window::new("Seleccionar por localización")
            .open(&mut open)
            .collapsible(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                if others.is_empty() {
                    ui.label("Hace falta otra capa vectorial cargada con la que comparar.");
                    return;
                }
                let name_of = |o: Option<usize>| others.iter().find(|x| Some(x.0) == o).map_or(String::new(), |x| x.1.clone());
                ui.label("Elementos de la capa activa que…");
                egui::ComboBox::from_id_salt("loc_rel").selected_text(d.rel.name()).show_ui(ui, |ui| {
                    for r in vecops::Relation::ALL {
                        ui.selectable_value(&mut d.rel, r, r.name());
                    }
                });
                ui.label("…algún elemento de la capa");
                egui::ComboBox::from_id_salt("loc_other").selected_text(name_of(d.other)).show_ui(ui, |ui| {
                    for (i, n) in &others {
                        ui.selectable_value(&mut d.other, Some(*i), n);
                    }
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.mode, 0, "Selección nueva");
                    ui.radio_value(&mut d.mode, 1, "Añadir a la selección");
                    ui.radio_value(&mut d.mode, 2, "Quitar de la selección");
                });
                go = ui.button("▶ Seleccionar").clicked();
                if !d.msg.is_empty() {
                    ui.label(&d.msg);
                }
            });
        if go && let Some(o) = d.other {
            let hits = match (&self.refs.get(k).map(|r| &r.kind), &self.refs.get(o).map(|r| &r.kind)) {
                (Some(RefKind::Vector(a)), Some(RefKind::Vector(b))) => {
                    Some(vecops::select_by_location(&a.data, a.kind, &b.data, b.kind, d.rel))
                }
                _ => None,
            };
            if let (Some(hits), Some(v)) = (hits, self.active_vec_mut()) {
                d.msg = format!("{} elementos cumplen la condición.", hits.len());
                match d.mode {
                    0 => v.selected = hits.into_iter().collect(),
                    1 => v.selected.extend(hits),
                    _ => {
                        for i in hits {
                            v.selected.remove(&i);
                        }
                    }
                }
            }
        }
        if open {
            self.vm.loc = Some(d);
        }
    }

    /// The question asked before a GeoPackage is rewritten: the file holds tables HIPSO does
    /// not keep (styles saved by QGIS, attribute tables, tiles).
    fn gpkg_loss_window(&mut self, ctx: &egui::Context) {
        let Some((k, lost)) = self.vm.gpkg_loss.take() else { return };
        let Some(name) = self.refs.get(k).map(|r| r.name.clone()) else { return };
        let (mut over, mut new, mut cancel) = (false, false, false);
        egui::Window::new("Guardar sobre el GeoPackage")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Para guardar «{name}» sobre su tabla hay que reescribir el archivo, y este contiene datos que HIPSO no conserva:"));
                ui.label(egui::RichText::new(lost.join(", ")).strong());
                ui.label("Las demás tablas de objetos sí se conservan.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    over = ui.button("Guardar y perder esos datos").clicked();
                    new = ui.button("Guardar como capa nueva…").clicked();
                    cancel = ui.button("Cancelar").clicked();
                });
            });
        if over {
            self.save_gpkg_over(k);
        } else if new {
            self.save_layer(k, true);
        } else if !cancel {
            self.vm.gpkg_loss = Some((k, lost));
        }
    }
}
