//! Los campos de una capa o las columnas de una tabla se eligen en una lista desplegable con los que
//! tiene el archivo, en lugar de escribirse a mano: un nombre mal escrito (o con otras
//! mayúsculas) es un error que la lista no deja cometer.
//!
//! Los nombres se leen del archivo (de un shapefile, solo la cabecera de su tabla) y se guardan
//! hasta que el archivo cambia, de modo que dibujar la lista en cada fotograma no cuesta nada.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use eframe::egui::{self, Color32};
use hipso_core::model::{ArgSpec, FieldSource};
use hipso_core::vecops::{FieldInfo, field_list};

use crate::theme::Tokens;

/// Lo que se sabe de un archivo: cuándo cambió por última vez y sus campos (o por qué no se leen).
type Entry = (Option<SystemTime>, Result<Vec<FieldInfo>, String>);

static CACHE: Mutex<Option<HashMap<PathBuf, Entry>>> = Mutex::new(None);

/// Cuándo cambió un archivo; de un shapefile cuenta también su tabla `.dbf`.
fn stamp(path: &Path) -> Option<SystemTime> {
    let m = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (m(path), m(&path.with_extension("dbf"))) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// Los campos del archivo `path` (una capa o una tabla), leídos de nuevo solo si el archivo cambió.
pub fn fields_of(path: &Path) -> Result<Vec<FieldInfo>, String> {
    let now = stamp(path);
    if let Ok(mut guard) = CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        if let Some((t, r)) = map.get(path)
            && *t == now
        {
            return r.clone();
        }
        let r = field_list(path).map_err(|e| e.to_string());
        map.insert(path.to_path_buf(), (now, r.clone()));
        return r;
    }
    field_list(path).map_err(|e| e.to_string())
}

const STALE: Color32 = Color32::from_rgb(255, 150, 60);

/// Los campos que se pueden elegir: todos o solo los numéricos.
fn choices(fields: &[FieldInfo], numeric: bool) -> Vec<&FieldInfo> {
    fields.iter().filter(|f| !numeric || f.numeric).collect()
}

/// Una lista desplegable con los campos; devuelve si cambió el valor. Un campo opcional ofrece
/// «(ninguno)»; un valor que ya no está entre los campos se marca en naranja.
pub fn combo(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, value: &mut String, fields: &[FieldInfo], numeric: bool, optional: bool, hint: &str) -> bool {
    let list = choices(fields, numeric);
    let mut changed = false;
    let known = value.is_empty() || list.iter().any(|f| f.name.eq_ignore_ascii_case(value));
    let shown = if value.is_empty() {
        if hint.is_empty() { "(elige un campo)".to_string() } else { format!("(por defecto: {hint})") }
    } else if known {
        value.clone()
    } else {
        format!("{value} (no está)")
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(egui::RichText::new(shown).color(if known { ui.visuals().text_color() } else { STALE }))
        .width(190.0)
        .show_ui(ui, |ui| {
            if optional && ui.selectable_label(value.is_empty(), "(ninguno)").clicked() {
                value.clear();
                changed = true;
            }
            for f in &list {
                if ui.selectable_label(f.name.eq_ignore_ascii_case(value), &f.name).clicked() {
                    *value = f.name.clone();
                    changed = true;
                }
            }
            if list.is_empty() {
                ui.label(if numeric { "No hay campos numéricos" } else { "No hay campos" });
            }
        });
    changed
}

/// Una lista de campos con casillas, para elegir varios; devuelve si cambió.
pub fn multi(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, chosen: &mut Vec<String>, fields: &[FieldInfo], numeric: bool) -> bool {
    let list = choices(fields, numeric);
    let mut changed = false;
    let summary = match chosen.len() {
        0 => "(ninguno)".to_string(),
        1..=3 => chosen.join(", "),
        n => format!("{n} campos"),
    };
    let r = ui.button(format!("{summary}  ▾"));
    egui::Popup::menu(&r)
        .id(egui::Id::new(id))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.horizontal(|ui| {
                if ui.small_button("Todos").clicked() {
                    *chosen = list.iter().map(|f| f.name.clone()).collect();
                    changed = true;
                }
                if ui.small_button("Ninguno").clicked() && !chosen.is_empty() {
                    chosen.clear();
                    changed = true;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for f in &list {
                    let mut on = chosen.iter().any(|c| c.eq_ignore_ascii_case(&f.name));
                    if ui.checkbox(&mut on, &f.name).changed() {
                        if on {
                            chosen.push(f.name.clone());
                        } else {
                            chosen.retain(|c| !c.eq_ignore_ascii_case(&f.name));
                        }
                        changed = true;
                    }
                }
                if list.is_empty() {
                    ui.label(if numeric { "No hay campos numéricos" } else { "No hay campos" });
                }
            });
        });
    changed
}

/// Los renombrados `viejo=nuevo`: por cada uno, el campo en una lista y el nombre nuevo escrito.
/// Devuelve si cambió algo.
pub fn rename_rows(ui: &mut egui::Ui, id: &str, pairs: &mut Vec<String>, fields: &[FieldInfo], t: &Tokens) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (k, pair) in pairs.iter_mut().enumerate() {
        let (mut old, mut new) = match pair.split_once('=') {
            Some((o, n)) => (o.to_string(), n.to_string()),
            None => (pair.clone(), String::new()),
        };
        let before = (old.clone(), new.clone());
        ui.horizontal(|ui| {
            if ui.small_button("🗑").on_hover_text("Quitar este renombrado").clicked() {
                remove = Some(k);
            }
            combo(ui, (id, "old", k), &mut old, fields, false, false, "");
            ui.label("→");
            ui.add(egui::TextEdit::singleline(&mut new).hint_text("nombre nuevo").desired_width(130.0));
            if new.trim().is_empty() {
                ui.label(egui::RichText::new("falta el nombre").size(11.0).color(STALE));
            }
        });
        if (old.clone(), new.clone()) != before {
            *pair = format!("{old}={new}");
            changed = true;
        }
    }
    if let Some(k) = remove {
        pairs.remove(k);
        changed = true;
    }
    let used: Vec<&str> = pairs.iter().filter_map(|p| p.split_once('=').map(|x| x.0)).collect();
    let next = fields.iter().find(|f| !used.iter().any(|u| u.eq_ignore_ascii_case(&f.name)));
    if ui.add_enabled(next.is_some(), egui::Button::new("Añadir un renombrado")).clicked()
        && let Some(f) = next
    {
        pairs.push(format!("{}=", f.name));
        changed = true;
    }
    let _ = t;
    changed
}

/// Dibuja una opción que nombra un campo de la tabla del archivo `source` de la herramienta (véase
/// [`hipso_core::model::field_source`]). Devuelve `false` si no se pudo dibujar como lista (el
/// archivo no se lee): quien llama la dibuja entonces como un cuadro de texto.
#[allow(clippy::too_many_arguments)]
pub fn row(
    ui: &mut egui::Ui,
    t: &Tokens,
    tool: &str,
    a: &ArgSpec,
    title: &str,
    fs: FieldSource,
    source_label: &str,
    source: Option<&str>,
    args: &mut BTreeMap<String, Vec<String>>,
) -> bool {
    let help = |ui: &mut egui::Ui| {
        if !a.help.is_empty() {
            ui.label(egui::RichText::new(a.help.lines().next().unwrap_or("")).size(11.0).color(t.weak));
        }
    };
    let Some(path) = source.filter(|p| !p.trim().is_empty()) else {
        ui.horizontal(|ui| {
            ui.label(title).on_hover_text(&a.help);
            ui.add_enabled(false, egui::Button::new(format!("(elige antes {source_label})")));
        });
        help(ui);
        return true;
    };
    let fields = match fields_of(Path::new(path)) {
        Ok(f) => f,
        Err(e) => {
            ui.label(egui::RichText::new(format!("No se leen los campos de «{}»: {e}. Escríbelos a mano.", Path::new(path).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()))).size(11.0).color(STALE));
            return false;
        }
    };
    let id = ("field_arg", tool, a.id.as_str());
    if a.id == "rename" {
        ui.label(title).on_hover_text(&a.help);
        let mut pairs = args.get("rename").cloned().unwrap_or_default();
        if rename_rows(ui, "cli_rename", &mut pairs, &fields, t) {
            if pairs.is_empty() {
                args.remove("rename");
            } else {
                args.insert("rename".into(), pairs);
            }
        }
    } else if a.multiple {
        let mut chosen = args.get(&a.id).cloned().unwrap_or_default();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(title).on_hover_text(&a.help);
            changed = multi(ui, id, &mut chosen, &fields, fs.numeric);
        });
        if changed {
            if chosen.is_empty() {
                args.remove(&a.id);
            } else {
                args.insert(a.id.clone(), chosen);
            }
        }
    } else {
        let mut value = args.get(&a.id).and_then(|v| v.first().cloned()).unwrap_or_default();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(title).on_hover_text(&a.help);
            changed = combo(ui, id, &mut value, &fields, fs.numeric, !a.required, a.default.as_deref().unwrap_or(""));
        });
        if changed {
            if value.is_empty() {
                args.remove(&a.id);
            } else {
                args.insert(a.id.clone(), vec![value]);
            }
        }
    }
    if fs.numeric {
        ui.label(egui::RichText::new("Solo se ofrecen los campos numéricos.").size(10.5).color(t.weak));
    }
    help(ui);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fields_of_a_file_are_cached_until_it_changes() {
        let dir = std::env::temp_dir().join(format!("hipso_fieldpick_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.csv");
        std::fs::write(&p, "a;b\n1;x\n").unwrap();
        let f = fields_of(&p).unwrap();
        assert_eq!(f.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        // otro contenido (y otra fecha): se lee de nuevo
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&p, "a;b;c\n1;x;2\n").unwrap();
        assert_eq!(fields_of(&p).unwrap().len(), 3);
        assert!(fields_of(&dir.join("no_existe.csv")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_numeric_fields_are_offered_when_the_option_asks_for_them() {
        let f = vec![
            FieldInfo { name: "especie".into(), numeric: false },
            FieldInfo { name: "altura".into(), numeric: true },
        ];
        assert_eq!(choices(&f, true).len(), 1);
        assert_eq!(choices(&f, false).len(), 2);
    }
}
