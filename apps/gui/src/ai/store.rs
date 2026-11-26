//! Las conversaciones con el asistente, guardadas para que sean baratas de escribir, de listar y de
//! abrir: un archivo por conversación en líneas JSON (`.jsonl`). Cada mensaje se añade al final, sin
//! reescribir lo anterior, y la primera línea lleva el título y la fecha, de modo que listar las
//! conversaciones lee una línea de cada archivo, no su contenido. De los resultados de las
//! herramientas solo se guarda el nombre de las que se usaron: nunca se vuelven a enviar, y
//! guardarlos solo haría crecer el archivo.
//!
//! La carpeta la elige la persona (la de configuración del programa, una suya o ninguna).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hipso_core::json::Json;

use super::jsonw::{arr, dump, n, num, obj, s, text};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub who: Who,
    pub text: String,
    /// Las herramientas que el asistente usó para esta respuesta.
    pub tools: Vec<String>,
    /// Tokens enviados y recibidos en esta respuesta, si el servicio los dio.
    pub tokens: Option<(u32, u32)>,
    /// El flujo que se creó con esta respuesta.
    pub flow: Option<PathBuf>,
    /// Es un error, no una respuesta (no se guarda ni se reenvía).
    pub error: bool,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message { who: Who::User, text: text.into(), tools: Vec::new(), tokens: None, flow: None, error: false }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Message { who: Who::Assistant, text: text.into(), tools: Vec::new(), tokens: None, flow: None, error: false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub id: String,
    pub title: String,
    /// Segundos desde 1970.
    pub created: u64,
    pub updated: u64,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct Conversation {
    pub meta: Meta,
    pub messages: Vec<Message>,
}

fn now() -> std::time::Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

/// Un título corto sacado de la primera pregunta.
pub fn title_of(first: &str) -> String {
    let one: String = first.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut t: String = one.chars().take(60).collect();
    if one.chars().count() > 60 {
        t.push('…');
    }
    if t.is_empty() { "Conversación".into() } else { t }
}

impl Conversation {
    pub fn new() -> Self {
        let t = now();
        Conversation {
            meta: Meta {
                id: format!("{:x}", t.as_millis()),
                title: "Nueva conversación".into(),
                created: t.as_secs(),
                updated: t.as_secs(),
                path: None,
            },
            messages: Vec::new(),
        }
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

/// El almacén: una carpeta (o ninguna, y entonces las conversaciones solo viven en memoria).
pub struct Store {
    pub dir: Option<PathBuf>,
}

fn message_line(m: &Message) -> String {
    let mut members = vec![
        ("t", s("m")),
        ("r", s(if m.who == Who::User { "user" } else { "assistant" })),
        ("c", s(&m.text)),
    ];
    if !m.tools.is_empty() {
        members.push(("tools", arr(m.tools.iter().map(s).collect())));
    }
    if let Some((p, c)) = m.tokens {
        members.push(("u", arr(vec![n(f64::from(p)), n(f64::from(c))])));
    }
    if let Some(f) = &m.flow {
        members.push(("flow", s(f.to_string_lossy())));
    }
    dump(&obj(members))
}

fn meta_line(m: &Meta) -> String {
    dump(&obj(vec![("t", s("meta")), ("id", s(&m.id)), ("title", s(&m.title)), ("created", n(m.created as f64))]))
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

fn parse_meta(line: &str, path: &Path) -> Option<Meta> {
    let j = Json::parse(line).ok()?;
    (text(&j, "t") == "meta").then(|| Meta {
        id: text(&j, "id"),
        title: text(&j, "title"),
        created: num(&j, "created").unwrap_or(0.0) as u64,
        updated: mtime(path),
        path: Some(path.to_path_buf()),
    })
}

impl Store {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Store { dir }
    }

    /// Las conversaciones guardadas, la más reciente primero. Lee solo la primera línea de cada una.
    pub fn list(&self) -> Vec<Meta> {
        let Some(dir) = &self.dir else { return Vec::new() };
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut out: Vec<Meta> = rd
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .filter_map(|p| {
                let mut first = String::new();
                BufReader::new(std::fs::File::open(&p).ok()?).read_line(&mut first).ok()?;
                parse_meta(first.trim(), &p)
            })
            .collect();
        out.sort_by(|a, b| b.updated.cmp(&a.updated).then(b.created.cmp(&a.created)));
        out
    }

    pub fn load(&self, path: &Path) -> Option<Conversation> {
        let reader = BufReader::new(std::fs::File::open(path).ok()?);
        let mut lines = reader.lines().map_while(Result::ok);
        let meta = parse_meta(lines.next()?.trim(), path)?;
        let mut messages = Vec::new();
        for l in lines {
            let Ok(j) = Json::parse(&l) else { continue };
            if text(&j, "t") != "m" {
                continue;
            }
            let who = if text(&j, "r") == "user" { Who::User } else { Who::Assistant };
            let tools = j
                .get("tools")
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            let tokens = j.get("u").and_then(Json::as_array).and_then(|a| {
                Some((a.first()?.as_f64()? as u32, a.get(1)?.as_f64()? as u32))
            });
            let flow = Some(text(&j, "flow")).filter(|f| !f.is_empty()).map(PathBuf::from);
            messages.push(Message { who, text: text(&j, "c"), tools, tokens, flow, error: false });
        }
        Some(Conversation { meta, messages })
    }

    /// Añade un mensaje; la primera vez crea el archivo (con el título sacado de la primera
    /// pregunta). No hace nada si no se guardan conversaciones.
    pub fn append(&self, conv: &mut Conversation, m: &Message) -> std::io::Result<()> {
        let Some(dir) = &self.dir else { return Ok(()) };
        if m.error {
            return Ok(());
        }
        if conv.meta.path.is_none() {
            std::fs::create_dir_all(dir)?;
            if conv.messages.is_empty() || conv.meta.title == "Nueva conversación" {
                conv.meta.title = title_of(&m.text);
            }
            let path = dir.join(format!("{}.jsonl", conv.meta.id));
            std::fs::write(&path, format!("{}\n", meta_line(&conv.meta)))?;
            conv.meta.path = Some(path);
        }
        if let Some(p) = &conv.meta.path {
            let mut f = std::fs::OpenOptions::new().append(true).open(p)?;
            writeln!(f, "{}", message_line(m))?;
            conv.meta.updated = now().as_secs();
        }
        Ok(())
    }

    pub fn delete(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }

    /// Cambia el título reescribiendo el archivo (es raro, y los archivos son pequeños).
    pub fn rename(&self, conv: &mut Conversation, title: &str) -> std::io::Result<()> {
        conv.meta.title = title_of(title);
        let Some(p) = conv.meta.path.clone() else { return Ok(()) };
        let body = std::fs::read_to_string(&p)?;
        let rest = body.split_once('\n').map_or("", |(_, r)| r);
        hipso_core::paths::write_atomic(&p, format!("{}\n{rest}", meta_line(&conv.meta)))
    }
}

/// La fecha y la hora (UTC) de unos segundos desde 1970, como `2026-11-23 08:15`.
pub fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // días desde el 1 de marzo de 0000 (algoritmo de Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, rem % 3600 / 60)
}

/// La conversación como Markdown, para guardarla o compartirla.
pub fn to_markdown(conv: &Conversation) -> String {
    let mut out = format!("# {}\n\n_{}_\n\n", conv.meta.title, stamp(conv.meta.created));
    for m in &conv.messages {
        if m.error {
            continue;
        }
        out.push_str(match m.who {
            Who::User => "## Tú\n\n",
            Who::Assistant => "## Asistente\n\n",
        });
        out.push_str(m.text.trim());
        out.push_str("\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hipso_ai_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_conversation_is_appended_listed_and_read_back() -> std::io::Result<()> {
        let dir = tmp("store");
        let store = Store::new(Some(dir.clone()));
        let mut c = Conversation::new();
        assert!(store.list().is_empty());
        store.append(&mut c, &Message::user("¿Cómo hago un MDT?\ncon varias líneas"))?;
        let mut a = Message::assistant("Con `hipso dtm`.");
        a.tools = vec!["buscar_manual".into()];
        a.tokens = Some((120, 40));
        a.flow = Some(PathBuf::from("/x/flujo.model.json"));
        c.messages.push(Message::user("¿Cómo hago un MDT?"));
        store.append(&mut c, &a)?;
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "¿Cómo hago un MDT? con varias líneas");
        let back = store.load(list[0].path.as_deref().unwrap_or(Path::new("")));
        let back = back.map(|b| b.messages);
        assert_eq!(back.as_ref().map(Vec::len), Some(2));
        let back = back.unwrap_or_default();
        assert_eq!(back[0].text, "¿Cómo hago un MDT?\ncon varias líneas");
        assert_eq!(back[1].tools, vec!["buscar_manual".to_string()]);
        assert_eq!(back[1].tokens, Some((120, 40)));
        assert_eq!(back[1].flow, Some(PathBuf::from("/x/flujo.model.json")));
        // renombrar conserva los mensajes
        store.rename(&mut c, "Otro título")?;
        let again = store.load(c.meta.path.as_deref().unwrap_or(Path::new("")));
        assert_eq!(again.as_ref().map(|c| c.meta.title.clone()), Some("Otro título".to_string()));
        assert_eq!(again.map(|c| c.messages.len()), Some(2));
        store.delete(c.meta.path.as_deref().unwrap_or(Path::new("")))?;
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn without_a_folder_nothing_is_written() -> std::io::Result<()> {
        let store = Store::new(None);
        let mut c = Conversation::new();
        store.append(&mut c, &Message::user("hola"))?;
        assert!(c.meta.path.is_none() && store.list().is_empty());
        Ok(())
    }

    #[test]
    fn errors_are_not_saved() -> std::io::Result<()> {
        let dir = tmp("err");
        let store = Store::new(Some(dir.clone()));
        let mut c = Conversation::new();
        let mut e = Message::assistant("falló");
        e.error = true;
        store.append(&mut c, &e)?;
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn dates_and_markdown() {
        assert_eq!(stamp(0), "1970-01-01 00:00");
        assert_eq!(stamp(1_700_000_000), "2023-11-14 22:13");
        assert_eq!(title_of("  a   b  "), "a b");
        assert_eq!(title_of(&"x".repeat(80)).chars().count(), 61);
        let mut c = Conversation::new();
        c.messages = vec![Message::user("hola"), Message::assistant("¡Hola!")];
        let md = to_markdown(&c);
        assert!(md.contains("## Tú\n\nhola") && md.contains("## Asistente\n\n¡Hola!"));
    }
}
