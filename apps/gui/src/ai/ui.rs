//! El panel de conversación del asistente y su sección de Ajustes.

use crate::*;

use super::config::{PROVIDERS, Storage, Style, clean_key, provider};
use super::store::{self, Who, stamp};
use super::tools::{Env, LayerInfo};
use super::{Assistant, client, docs, roles};

/// Las preguntas que se ofrecen en una conversación vacía.
const SUGGESTIONS: [&str; 5] = [
    "¿Qué es HIPSO y qué puedo hacer con él?",
    "Explícame en profundidad cómo clasifica HIPSO una nube de puntos",
    "Crea un flujo que haga el MDT, el CHM y segmente los árboles de mi nube",
    "Describe y explora los datos que tengo abiertos",
    "¿Cómo compruebo la exactitud de mi MDT?",
];

fn open_folder(dir: &std::path::Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(dir).spawn();
}

fn approx_tokens(chars: usize) -> usize {
    chars.div_ceil(4)
}

impl App {
    /// Lo que el asistente puede ver ahora: las capas abiertas y el esquema de las herramientas.
    fn ai_env(&mut self) -> Env {
        let mut layers: Vec<LayerInfo> = Vec::new();
        for p in &self.clouds {
            layers.push(LayerInfo {
                name: p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
                kind: "nube de puntos".into(),
                path: p.display().to_string(),
                note: String::new(),
            });
        }
        for r in &self.refs {
            let kind = match &r.kind {
                RefKind::Raster { .. } => "ráster",
                RefKind::Vector(_) => "vector",
                RefKind::Image(_) => "imagen",
            };
            layers.push(LayerInfo { name: r.name.clone(), kind: kind.into(), path: r.path.display().to_string(), note: String::new() });
        }
        Env {
            layers,
            schema: self.tool_schema(),
            flows_dir: prefs::config_dir().map(|d| d.join("flujos_asistente")),
        }
    }

    /// El panel del asistente, acoplado a la derecha.
    pub fn ai_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(p) = self.ai.open_flow.take() {
            self.flow_load_path(&p);
        }
        if std::mem::take(&mut self.ai.want_settings) {
            self.show_backstage(backstage::Page::Settings);
        }
        if !self.ai.open || self.backstage {
            return;
        }
        self.ai.resume_last();
        self.ai.poll();
        if self.ai.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
        self.ai.refresh_list();
        let t = self.tokens();
        egui::Panel::right("assistant")
            .default_size(430.0)
            .size_range(320.0..=820.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(t.pane).inner_margin(egui::Margin::same(8)).stroke(Stroke::new(1.0, t.border)))
            .show(ui, |ui| self.ai_body(ui, ctx, &t));
    }

    fn ai_body(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &theme::Tokens) {
        self.ai_header(ui, t);
        ui.add_space(4.0);
        if !self.ai.cfg.has_key() {
            egui::Frame::new().fill(t.selection).inner_margin(egui::Margin::same(8)).corner_radius(6).show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Falta la clave de {}. El asistente la usa para hablar con el servicio; se guarda solo en este equipo.", provider(&self.ai.cfg.provider).name))
                        .color(t.text),
                );
                if ui.button("Configurar el asistente").clicked() {
                    self.ai.want_settings = true;
                }
            });
            ui.add_space(4.0);
        }
        let input_h = 112.0;
        let msgs_h = (ui.available_height() - input_h).max(80.0);
        let mut send: Option<String> = None;
        egui::ScrollArea::vertical()
            .id_salt("ai_messages")
            .max_height(msgs_h)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.ai.conv.messages.is_empty() && !self.ai.busy() {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("¿En qué te ayudo?").size(17.0).strong().color(theme::brand_text(t)));
                    ui.label(
                        egui::RichText::new("Pregúntame qué hace HIPSO o cualquiera de sus herramientas, pídeme que explore tus datos o que arme un flujo.")
                            .color(t.weak),
                    );
                    ui.add_space(8.0);
                    for s in SUGGESTIONS {
                        if ui.add(egui::Button::new(s).wrap().min_size(Vec2::new(ui.available_width(), 0.0))).clicked() {
                            send = Some(s.to_string());
                        }
                    }
                }
                let n = self.ai.conv.messages.len();
                for i in 0..n {
                    let m = self.ai.conv.messages[i].clone();
                    ui.push_id(i, |ui| self.ai_message(ui, ctx, t, &m, i));
                    ui.add_space(6.0);
                }
                if let Some(s) = &self.ai.stream {
                    ui.push_id("streaming", |ui| {
                        egui::Frame::new().fill(t.canvas).inner_margin(egui::Margin::same(8)).corner_radius(8).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if s.text.trim().is_empty() {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    let what = s.tools.last().map_or("Pensando…".to_string(), |t| format!("Consultando: {}…", tool_label(t)));
                                    ui.label(egui::RichText::new(what).color(t.weak));
                                });
                            } else {
                                manual::markdown(ui, &s.text, "ai_stream");
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    if let Some(tl) = s.tools.last() {
                                        ui.label(egui::RichText::new(format!("{}…", tool_label(tl))).size(11.0).color(t.weak));
                                    }
                                });
                            }
                        });
                    });
                }
                if std::mem::take(&mut self.ai.scroll_down) {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
        ui.add_space(4.0);
        // la caja de pregunta
        let id = egui::Id::new("ai_input");
        let focused = ctx.memory(|m| m.has_focus(id));
        let enter = focused && ctx.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift && !i.modifiers.command);
        if enter {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        }
        let r = ui.add(
            egui::TextEdit::multiline(&mut self.ai.input)
                .id(id)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("Escribe tu pregunta (Intro envía, Mayús+Intro salta de línea)"),
        );
        if std::mem::take(&mut self.ai.focus_input) {
            r.request_focus();
        }
        ui.horizontal(|ui| {
            if self.ai.busy() {
                if ui.button("■ Detener").clicked() {
                    self.ai.cancel();
                }
            } else {
                let can = !self.ai.input.trim().is_empty();
                if ui.add_enabled(can, egui::Button::new("Enviar ➤")).clicked() || (enter && can) {
                    send = Some(std::mem::take(&mut self.ai.input));
                }
            }
            if self.ai.spent != (0, 0) {
                ui.label(egui::RichText::new(format!("↑{} ↓{} tokens", kilo(self.ai.spent.0), kilo(self.ai.spent.1))).size(11.0).color(t.weak))
                    .on_hover_text("Tokens enviados y recibidos en esta conversación desde que se abrió el programa");
            }
        });
        if let Some(q) = send {
            let env = self.ai_env();
            self.ai.send(ctx, q, env);
        }
    }

    fn ai_header(&mut self, ui: &mut egui::Ui, t: &theme::Tokens) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Asistente").size(15.0).strong().color(theme::brand_text(t)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✖").on_hover_text("Cerrar el asistente").clicked() {
                    self.ai.open = false;
                }
                if ui.small_button("⚙").on_hover_text("Ajustes del asistente: servicio, claves, perfil y conversaciones").clicked() {
                    self.ai.want_settings = true;
                }
                ui.menu_button("🕘", |ui| self.ai_history_menu(ui, t)).response.on_hover_text("Conversaciones guardadas");
                if ui.small_button("＋").on_hover_text("Nueva conversación").clicked() {
                    self.ai.new_conversation();
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let cfg = &self.ai.cfg;
            ui.label(egui::RichText::new(format!("{} · {}", provider(&cfg.provider).name, cfg.model())).size(11.0).color(t.weak));
            let mut role = cfg.role.clone();
            let before = role.clone();
            egui::ComboBox::from_id_salt("ai_role_quick")
                .selected_text(egui::RichText::new(self.ai.cfg.role_name()).size(11.0))
                .width(150.0)
                .show_ui(ui, |ui| role_options(ui, &mut role, &self.ai.cfg.custom_name, !self.ai.cfg.custom_text.trim().is_empty()));
            if role != before {
                self.ai.cfg.role = role;
                self.ai.apply_cfg();
            }
        });
    }

    fn ai_history_menu(&mut self, ui: &mut egui::Ui, t: &theme::Tokens) {
        ui.set_min_width(300.0);
        if self.ai.cfg.storage == Storage::Off {
            ui.label(egui::RichText::new("Las conversaciones no se guardan (Ajustes → Asistente de IA).").color(t.weak));
        } else if self.ai.list.is_empty() {
            ui.label(egui::RichText::new("Aún no hay conversaciones guardadas.").color(t.weak));
        }
        let mut open: Option<store::Meta> = None;
        let mut delete: Option<store::Meta> = None;
        for m in self.ai.list.iter().take(30) {
            ui.horizontal(|ui| {
                let current = self.ai.conv.meta.id == m.id;
                let label = format!("{}  ·  {}", docs::cut(&m.title, 34), stamp(m.updated));
                if ui.selectable_label(current, label).clicked() {
                    open = Some(m.clone());
                    ui.close();
                }
                if ui.small_button("🗑").on_hover_text("Borrar esta conversación").clicked() {
                    delete = Some(m.clone());
                }
            });
        }
        ui.separator();
        if ui.add_enabled(!self.ai.conv.messages.is_empty(), egui::Button::new("Exportar esta conversación (Markdown)…")).clicked() {
            let name = format!("{}.md", docs::fold(&self.ai.conv.meta.title).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>());
            if let Some(p) = prefs::dialog().add_filter("Markdown", &["md"]).set_file_name(name).save_file()
                && let Err(e) = std::fs::write(&p, store::to_markdown(&self.ai.conv)) {
                    self.notice = Some(format!("No se pudo exportar: {e}"));
                }
            ui.close();
        }
        if self.ai.conv.meta.path.is_some() {
            ui.horizontal(|ui| {
                ui.label("Título");
                let r = ui.add(egui::TextEdit::singleline(&mut self.ai.rename_buf).desired_width(180.0).hint_text(docs::cut(&self.ai.conv.meta.title, 24)));
                if (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))) && !self.ai.rename_buf.trim().is_empty() {
                    let title = std::mem::take(&mut self.ai.rename_buf);
                    if let Err(e) = self.ai.store.rename(&mut self.ai.conv, &title) {
                        self.notice = Some(format!("No se pudo renombrar: {e}"));
                    }
                    self.ai.list_dirty = true;
                }
            });
        }
        if let Some(m) = open {
            self.ai.open_conversation(&m);
        }
        if let Some(m) = delete {
            if let Some(p) = &m.path {
                let _ = self.ai.store.delete(p);
            }
            if self.ai.conv.meta.id == m.id {
                self.ai.new_conversation();
            }
            self.ai.list_dirty = true;
        }
    }

    fn ai_message(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &theme::Tokens, m: &store::Message, i: usize) {
        match m.who {
            Who::User => {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    egui::Frame::new().fill(t.selection).inner_margin(egui::Margin::same(8)).corner_radius(8).show(ui, |ui| {
                        ui.set_max_width(ui.available_width() * 0.92);
                        ui.add(egui::Label::new(egui::RichText::new(&m.text).color(t.text)).wrap());
                    });
                });
            }
            Who::Assistant if m.error => {
                egui::Frame::new().fill(t.canvas).inner_margin(egui::Margin::same(8)).corner_radius(8).stroke(Stroke::new(1.0, Color32::from_rgb(0xE8, 0x91, 0x2D))).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(egui::RichText::new(&m.text).color(Color32::from_rgb(0xE8, 0x91, 0x2D))).wrap());
                    if ui.small_button("Ajustes del asistente").clicked() {
                        self.ai.want_settings = true;
                    }
                });
            }
            Who::Assistant => {
                egui::Frame::new().fill(t.canvas).inner_margin(egui::Margin::same(8)).corner_radius(8).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    manual::markdown(ui, &m.text, "ai_msg");
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if let Some(f) = &m.flow
                            && ui.button("Abrir en Flujos").on_hover_text(f.display().to_string()).clicked() {
                                self.ai.open_flow = Some(f.clone());
                            }
                        if ui.small_button("Copiar").on_hover_text("Copia la respuesta").clicked() {
                            ctx.copy_text(m.text.clone());
                        }
                        let mut info = String::new();
                        if !m.tools.is_empty() {
                            info.push_str(&format!("Consultó: {}", m.tools.iter().map(|t| tool_label(t)).collect::<Vec<_>>().join(", ")));
                        }
                        if let Some((a, b)) = m.tokens {
                            info.push_str(&format!("{}↑{} ↓{}", if info.is_empty() { "" } else { " · " }, kilo(a), kilo(b)));
                        }
                        if !info.is_empty() {
                            ui.label(egui::RichText::new(info).size(10.5).color(t.weak));
                        }
                    });
                    let _ = i;
                });
            }
        }
    }

    /// La sección «Asistente de IA» de la página de Ajustes.
    pub fn ai_settings(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        // lo que han devuelto las peticiones en segundo plano
        if let Some(rx) = &self.ai.models_rx
            && let Ok(r) = rx.try_recv() {
                match r {
                    Ok(list) => {
                        self.ai.settings_msg = format!("{} modelos disponibles.", list.len());
                        self.ai.models = list;
                    }
                    Err(e) => self.ai.settings_msg = e,
                }
                self.ai.models_rx = None;
            }
        if let Some(rx) = &self.ai.test_rx
            && let Ok(r) = rx.try_recv() {
                self.ai.settings_msg = r.unwrap_or_else(|e| e);
                self.ai.test_rx = None;
            }
        if self.ai.models_rx.is_some() || self.ai.test_rx.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.add_space(18.0);
        ui.label(egui::RichText::new("Asistente de IA").size(16.0).strong().color(t.text));
        ui.label(
            egui::RichText::new("Un asistente que conoce HIPSO: explica el programa y sus herramientas, explora tus datos y crea flujos. Usa tu propia clave de API; las consultas las factura tu proveedor.")
                .color(t.weak),
        );
        ui.add_space(8.0);
        let mut changed = false;

        // --- servicio ---
        ui.label(egui::RichText::new("Servicio").strong());
        let current = self.ai.cfg.provider.clone();
        egui::ComboBox::from_id_salt("ai_provider").selected_text(provider(&current).name).width(280.0).show_ui(ui, |ui| {
            for p in &PROVIDERS {
                if ui.selectable_label(current == p.id, p.name).clicked() && current != p.id {
                    self.ai.cfg.provider = p.id.to_string();
                    self.ai.models.clear();
                    self.ai.settings_msg.clear();
                    changed = true;
                }
            }
        });
        let pid = self.ai.cfg.provider.clone();
        let p = provider(&pid);
        if p.id == "custom" {
            ui.horizontal(|ui| {
                ui.label("Dirección base");
                let b = &mut self.ai.cfg.entry("custom").base;
                changed |= ui.add(egui::TextEdit::singleline(b).desired_width(320.0).hint_text("https://…/v1")).changed();
            });
            if super::config::unencrypted(&self.ai.cfg.entry("custom").base) {
                ui.label(egui::RichText::new("Atención: con http:// la clave y las consultas viajan sin cifrar. Usa https:// si el servicio no está en este equipo.").size(11.5).color(ui.visuals().warn_fg_color));
            }
        } else {
            ui.label(egui::RichText::new(format!("Dirección: {}", p.base)).size(11.5).color(t.weak));
        }
        ui.horizontal(|ui| {
            ui.label("Clave de API");
            let show = self.ai.show_key;
            let e = self.ai.cfg.entry(&pid);
            let r = ui.add(egui::TextEdit::singleline(&mut e.key).password(!show).desired_width(300.0).hint_text("se guarda solo en este equipo"));
            if r.lost_focus() {
                e.key = clean_key(&e.key);
            }
            changed |= r.changed();
            if ui.small_button(if show { "Ocultar" } else { "Ver" }).clicked() {
                self.ai.show_key = !show;
            }
        });
        ui.label(egui::RichText::new(format!("La clave se saca en {}. Se guarda {} en un archivo de tu carpeta de configuración{}.", p.keys_at, "como texto", if cfg!(unix) { ", que solo tu usuario puede leer" } else { ": protege tu cuenta de Windows, porque no va cifrada" })).size(11.5).color(t.weak));
        ui.horizontal(|ui| {
            ui.label("Modelo");
            let default = p.model;
            let e = self.ai.cfg.entry(&pid);
            changed |= ui.add(egui::TextEdit::singleline(&mut e.model).desired_width(240.0).hint_text(default)).changed();
            if !self.ai.models.is_empty() {
                let mut pick: Option<String> = None;
                egui::ComboBox::from_id_salt("ai_models").selected_text("Elegir de la lista").width(170.0).show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for m in &self.ai.models {
                            if ui.selectable_label(false, m).clicked() {
                                pick = Some(m.clone());
                            }
                        }
                    });
                });
                if let Some(m) = pick {
                    self.ai.cfg.entry(&pid).model = m;
                    changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(self.ai.models_rx.is_none(), egui::Button::new("Cargar modelos")).clicked() {
                let cfg = self.ai.cfg.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(client::list_models(&cfg));
                });
                self.ai.models_rx = Some(rx);
                self.ai.settings_msg = "Consultando los modelos…".into();
            }
            if ui.add_enabled(self.ai.test_rx.is_none(), egui::Button::new("Probar conexión")).clicked() {
                let cfg = self.ai.cfg.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(client::test(&cfg));
                });
                self.ai.test_rx = Some(rx);
                self.ai.settings_msg = "Probando…".into();
            }
        });
        if !self.ai.settings_msg.is_empty() {
            ui.label(egui::RichText::new(&self.ai.settings_msg).color(theme::brand_text(&t)));
        }
        if pid == "anthropic" {
            ui.label(egui::RichText::new("Claude se usa con la capa de compatibilidad con la API de OpenAI de Anthropic.").size(11.5).color(t.weak));
        }

        // --- perfil ---
        ui.add_space(14.0);
        ui.label(egui::RichText::new("Perfil (rol)").strong());
        ui.label(egui::RichText::new("Por defecto, sin rol: el asistente de HIPSO tal cual. Un perfil le da el enfoque y el estilo de un especialista.").size(11.5).color(t.weak));
        let before = self.ai.cfg.role.clone();
        let mut role = before.clone();
        egui::ComboBox::from_id_salt("ai_role").selected_text(self.ai.cfg.role_name()).width(280.0).show_ui(ui, |ui| {
            role_options(ui, &mut role, &self.ai.cfg.custom_name, true);
        });
        if role != before {
            self.ai.cfg.role = role;
            changed = true;
        }
        if self.ai.cfg.role == roles::CUSTOM {
            ui.horizontal(|ui| {
                ui.label("Nombre");
                changed |= ui.add(egui::TextEdit::singleline(&mut self.ai.cfg.custom_name).desired_width(240.0)).changed();
            });
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut self.ai.cfg.custom_text)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY)
                        .hint_text("Quién es, qué prioriza, cómo responde y qué evita. Escríbelo como instrucciones: «Actúa como…»"),
                )
                .changed();
        } else if let Some(r) = roles::find(&self.ai.cfg.role) {
            ui.collapsing("Ver el texto del perfil", |ui| {
                ui.add(egui::Label::new(egui::RichText::new(r.text).size(12.0).color(t.weak)).wrap());
            });
        }
        let role_chars = self.ai.cfg.role_text().chars().count();
        if role_chars > 0 {
            ui.label(egui::RichText::new(format!("El perfil añade unos {} tokens a cada pregunta (los servicios que guardan el inicio del mensaje en caché lo cobran mucho más barato).", approx_tokens(role_chars))).size(11.5).color(t.weak));
        }

        // --- respuestas ---
        ui.add_space(14.0);
        ui.label(egui::RichText::new("Respuestas y gasto de tokens").strong());
        ui.horizontal(|ui| {
            ui.label("Extensión");
            let cur = self.ai.cfg.style;
            egui::ComboBox::from_id_salt("ai_style").selected_text(cur.label()).show_ui(ui, |ui| {
                for s in Style::ALL {
                    if ui.selectable_label(cur == s, s.label()).clicked() && cur != s {
                        self.ai.cfg.style = s;
                        changed = true;
                    }
                }
            });
            ui.label("Máx. de la respuesta");
            changed |= ui.add(egui::DragValue::new(&mut self.ai.cfg.max_output).range(200..=16000).speed(50).suffix(" tokens")).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Mensajes que se reenvían de la conversación");
            changed |= ui.add(egui::DragValue::new(&mut self.ai.cfg.history).range(2..=40)).changed();
        });
        ui.label(
            egui::RichText::new("El asistente no lleva el manual en la cabeza: consulta el apartado que necesita, y los resultados de esas consultas no se reenvían. Menos mensajes reenviados y respuestas más breves gastan menos.")
                .size(11.5)
                .color(t.weak),
        );

        // --- conversaciones ---
        ui.add_space(14.0);
        ui.label(egui::RichText::new("Conversaciones").strong());
        let storage = self.ai.cfg.storage;
        let app_dir = prefs::config_dir().map(|d| d.join("conversaciones"));
        let mut new_storage = storage;
        ui.radio_value(&mut new_storage, Storage::App, format!("En la carpeta de HIPSO ({})", app_dir.as_ref().map_or("—".into(), |d| d.display().to_string())));
        ui.radio_value(&mut new_storage, Storage::Folder, "En una carpeta que elijo (un disco externo, una carpeta sincronizada…)");
        ui.radio_value(&mut new_storage, Storage::Off, "No guardarlas (solo duran mientras el programa está abierto)");
        let old_dir = self.ai.cfg.conversations_dir();
        if new_storage != storage {
            self.ai.cfg.storage = new_storage;
            changed = true;
        }
        if self.ai.cfg.storage == Storage::Folder {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.ai.cfg.folder).desired_width(360.0).hint_text("Carpeta"));
                if ui.button("Elegir…").clicked()
                    && let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.ai.cfg.folder = d.display().to_string();
                        changed = true;
                    }
            });
        }
        if changed {
            let new_dir = self.ai.cfg.conversations_dir();
            self.ai.apply_cfg();
            // si cambió la carpeta y la anterior tenía conversaciones, se ofrece moverlas
            if let (Some(o), Some(n)) = (old_dir, new_dir)
                && o != n
                && Assistant::count_in(&o) > 0
            {
                self.ai.pending_move = Some(o);
            }
        }
        if let Some(dir) = self.ai.cfg.conversations_dir() {
            ui.horizontal(|ui| {
                if ui.button("Abrir la carpeta").clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    open_folder(&dir);
                }
                ui.label(egui::RichText::new(format!("{} conversaciones guardadas", Assistant::count_in(&dir))).size(11.5).color(t.weak));
            });
        }
        if let Some(from) = self.ai.pending_move.clone() {
            ui.horizontal(|ui| {
                ui.label(format!("Hay {} conversaciones en la carpeta anterior.", Assistant::count_in(&from)));
                if ui.button("Moverlas a la nueva").clicked() {
                    self.ai.settings_msg = match self.ai.move_conversations(&from) {
                        Ok(n) => format!("{n} conversaciones movidas."),
                        Err(e) => e,
                    };
                    self.ai.list_dirty = true;
                    self.ai.pending_move = None;
                }
                if ui.button("Dejarlas donde están").clicked() {
                    self.ai.pending_move = None;
                }
            });
        }
        if self.ai.cfg.storage != Storage::Off {
            ui.horizontal(|ui| {
                if self.ai.confirm_delete_all {
                    ui.label("¿Borrar todas las conversaciones guardadas?");
                    if ui.button("Sí, borrarlas").clicked() {
                        for m in self.ai.store.list() {
                            if let Some(p) = &m.path {
                                let _ = self.ai.store.delete(p);
                            }
                        }
                        self.ai.new_conversation();
                        self.ai.list_dirty = true;
                        self.ai.confirm_delete_all = false;
                    }
                    if ui.button("No").clicked() {
                        self.ai.confirm_delete_all = false;
                    }
                } else if ui.button("Borrar todas las conversaciones…").clicked() {
                    self.ai.confirm_delete_all = true;
                }
            });
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new("A los servicios solo llegan textos y cifras: tus preguntas, lo que devuelven las herramientas del asistente (documentación, descripciones y estadísticos de tus archivos) y los flujos. Nunca los archivos ni las imágenes.").size(11.5).color(t.weak));
    }
}

impl Assistant {
    /// Cuántas conversaciones hay en una carpeta.
    fn count_in(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map_or(0, |rd| rd.filter_map(Result::ok).filter(|e| e.path().extension().is_some_and(|x| x == "jsonl")).count())
    }
}

/// Las opciones de perfil de una lista: sin rol, los predefinidos y el personalizado.
fn role_options(ui: &mut egui::Ui, role: &mut String, custom_name: &str, custom_ready: bool) {
    if ui.selectable_label(role == roles::NONE, "Sin rol (por defecto)").clicked() {
        *role = roles::NONE.into();
    }
    for r in &roles::ROLES {
        if ui.selectable_label(role == r.id, r.name).clicked() {
            *role = r.id.into();
        }
    }
    let label = if custom_ready { format!("{custom_name} (personalizado)") } else { "Personalizado…".to_string() };
    if ui.selectable_label(role == roles::CUSTOM, label).clicked() {
        *role = roles::CUSTOM.into();
    }
}

/// El nombre de una herramienta del asistente en palabras.
fn tool_label(name: &str) -> &str {
    match name {
        "que_es_hipso" => "qué es HIPSO",
        "listar_herramientas" => "lista de herramientas",
        "ayuda_herramienta" => "ayuda de herramienta",
        "buscar_manual" => "manual",
        "leer_seccion" => "apartado del manual",
        "capas_abiertas" => "capas abiertas",
        "describir_archivo" => "descripción del archivo",
        "analisis_exploratorio" => "análisis exploratorio",
        "crear_flujo" => "creación del flujo",
        other => other,
    }
}

/// Un número de tokens corto: 1,2 k.
fn kilo(v: u32) -> String {
    if v >= 1000 { format!("{:.1} k", f64::from(v) / 1000.0) } else { v.to_string() }
}

