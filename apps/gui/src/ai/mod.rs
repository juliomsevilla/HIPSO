//! El asistente de IA de HIPSO: un panel de conversación acoplado a la derecha, que conoce el
//! programa (consulta el manual y la referencia en lugar de llevarlos en la cabeza), explica las
//! herramientas, describe y explora los datos abiertos y construye flujos. Funciona con DeepSeek,
//! OpenRouter, Claude (Anthropic) y ChatGPT (OpenAI) con la clave de la persona, y con un perfil
//! (rol) a elegir en Ajustes.
//!
//! Gasta pocos tokens por diseño: instrucciones fijas y cortas, la documentación se consulta por
//! apartados, los resultados de las herramientas se recortan y no se reenvían, y de la
//! conversación solo se envían los últimos mensajes. Las conversaciones se guardan en archivos
//! `.jsonl` (un mensaje por línea) en la carpeta que la persona elige.

mod client;
mod config;
mod docs;
mod jsonw;
mod roles;
mod store;
mod tools;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

pub use config::Config;
use store::{Conversation, Message, Meta, Store};

/// Una respuesta que se está recibiendo.
struct Stream {
    rx: Receiver<client::Event>,
    cancel: Arc<AtomicBool>,
    text: String,
    tools: Vec<String>,
    flow: Option<PathBuf>,
    usage: Option<(u32, u32)>,
}

/// Todo el estado del asistente.
pub struct Assistant {
    pub open: bool,
    pub cfg: Config,
    store: Store,
    conv: Conversation,
    list: Vec<Meta>,
    list_dirty: bool,
    input: String,
    stream: Option<Stream>,
    /// Los tokens enviados y recibidos en esta conversación, mientras el programa está abierto.
    spent: (u32, u32),
    /// El flujo que la persona pidió abrir desde una respuesta.
    pub open_flow: Option<PathBuf>,
    /// Pedir al programa que abra los ajustes.
    pub want_settings: bool,
    // ajustes
    models: Vec<String>,
    models_rx: Option<Receiver<Result<Vec<String>, String>>>,
    test_rx: Option<Receiver<Result<String, String>>>,
    settings_msg: String,
    show_key: bool,
    confirm_delete_all: bool,
    scroll_down: bool,
    focus_input: bool,
    /// La carpeta anterior de las conversaciones, cuando se ha cambiado y tenía alguna.
    pending_move: Option<PathBuf>,
    rename_buf: String,
    resumed: bool,
}

impl Assistant {
    pub fn new() -> Self {
        let cfg = Config::load();
        let store = Store::new(cfg.conversations_dir());
        Assistant {
            open: false,
            cfg,
            store,
            conv: Conversation::new(),
            list: Vec::new(),
            list_dirty: true,
            input: String::new(),
            stream: None,
            spent: (0, 0),
            open_flow: None,
            want_settings: false,
            models: Vec::new(),
            models_rx: None,
            test_rx: None,
            settings_msg: String::new(),
            show_key: false,
            confirm_delete_all: false,
            scroll_down: false,
            focus_input: false,
            pending_move: None,
            rename_buf: String::new(),
            resumed: false,
        }
    }

    /// Al abrir el panel, el cursor va a la caja de pregunta.
    pub fn want_focus(&mut self) {
        self.focus_input = self.open;
    }

    pub fn busy(&self) -> bool {
        self.stream.is_some()
    }

    /// Guarda los ajustes y vuelve a apuntar el almacén a la carpeta elegida.
    fn apply_cfg(&mut self) {
        if let Err(e) = self.cfg.save() {
            self.settings_msg = format!("No se pudieron guardar los ajustes: {e}");
        }
        self.store = Store::new(self.cfg.conversations_dir());
        self.list_dirty = true;
    }

    fn new_conversation(&mut self) {
        self.cancel();
        self.conv = Conversation::new();
        self.spent = (0, 0);
        self.input.clear();
        self.focus_input = true;
    }

    fn open_conversation(&mut self, meta: &Meta) {
        self.cancel();
        if let Some(c) = meta.path.as_deref().and_then(|p| self.store.load(p)) {
            self.conv = c;
            self.spent = (0, 0);
            self.scroll_down = true;
        }
    }

    fn cancel(&mut self) {
        if let Some(s) = self.stream.take() {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Envía una pregunta: la guarda, pide la respuesta en un hilo y deja el panel esperándola.
    fn send(&mut self, ctx: &eframe::egui::Context, question: String, env: tools::Env) {
        let question = question.trim().to_string();
        if question.is_empty() || self.stream.is_some() {
            return;
        }
        let history = self.conv.messages.clone();
        let user = Message::user(&question);
        if let Err(e) = self.store.append(&mut self.conv, &user) {
            self.settings_msg = format!("No se pudo guardar la conversación: {e}");
        }
        self.conv.messages.push(user);
        self.list_dirty = true;
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        client::spawn(client::Request { cfg: self.cfg.clone(), history, question, env }, tx, cancel.clone(), ctx.clone());
        self.stream = Some(Stream { rx, cancel, text: String::new(), tools: Vec::new(), flow: None, usage: None });
        self.scroll_down = true;
    }

    /// Recoge lo que ha llegado de la respuesta en marcha.
    fn poll(&mut self) {
        let mut finished: Option<Result<(), String>> = None;
        if let Some(s) = &mut self.stream {
            while let Ok(ev) = s.rx.try_recv() {
                match ev {
                    client::Event::Token(t) => s.text.push_str(&t),
                    client::Event::Tool(name) => {
                        if !s.tools.contains(&name) {
                            s.tools.push(name);
                        }
                    }
                    client::Event::Flow(p) => s.flow = Some(p),
                    client::Event::Usage(a, b) => s.usage = Some((a, b)),
                    client::Event::Done => finished = Some(Ok(())),
                    client::Event::Error(e) => finished = Some(Err(e)),
                }
            }
        }
        let Some(result) = finished else { return };
        let Some(s) = self.stream.take() else { return };
        if let Some((a, b)) = s.usage {
            self.spent.0 += a;
            self.spent.1 += b;
        }
        let mut m = Message::assistant(s.text.trim());
        m.tools = s.tools;
        m.flow = s.flow;
        m.tokens = s.usage;
        match result {
            Ok(()) if m.text.is_empty() => {
                m.text = "No he podido contestar. Prueba a reformular la pregunta.".into();
                m.error = true;
            }
            Ok(()) => {}
            Err(e) => {
                // lo recibido antes del error se conserva; el aviso va aparte
                if !m.text.is_empty() {
                    if let Err(w) = self.store.append(&mut self.conv, &m) {
                        self.settings_msg = format!("No se pudo guardar la conversación: {w}");
                    }
                    self.conv.messages.push(m.clone());
                }
                m = Message::assistant(e);
                m.error = true;
            }
        }
        if let Err(w) = self.store.append(&mut self.conv, &m) {
            self.settings_msg = format!("No se pudo guardar la conversación: {w}");
        }
        self.conv.messages.push(m);
        self.list_dirty = true;
        self.scroll_down = true;
    }

    /// La primera vez que se abre el panel en una sesión, continúa la última conversación.
    fn resume_last(&mut self) {
        if self.resumed {
            return;
        }
        self.resumed = true;
        if self.conv.messages.is_empty()
            && let Some(m) = self.store.list().into_iter().next()
        {
            self.open_conversation(&m);
        }
    }

    fn refresh_list(&mut self) {
        if self.list_dirty {
            self.list = self.store.list();
            self.list_dirty = false;
        }
    }

    /// Las conversaciones guardadas en una carpeta, para moverlas a otra.
    fn move_conversations(&mut self, from: &std::path::Path) -> Result<usize, String> {
        let Some(to) = self.cfg.conversations_dir() else { return Err("Ahora no se guardan conversaciones.".into()) };
        std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
        let mut moved = 0;
        for e in std::fs::read_dir(from).map_err(|e| e.to_string())?.filter_map(Result::ok) {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "jsonl") {
                let target = to.join(e.file_name());
                if target.exists() {
                    continue;
                }
                // renombrar falla entre discos: se copia y se borra
                if std::fs::rename(&p, &target).is_err() {
                    std::fs::copy(&p, &target).map_err(|e| e.to_string())?;
                    std::fs::remove_file(&p).map_err(|e| e.to_string())?;
                }
                moved += 1;
            }
        }
        Ok(moved)
    }
}

impl Default for Assistant {
    fn default() -> Self {
        Self::new()
    }
}
