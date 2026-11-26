//! La conversación con el servicio de IA: una petición de chat en streaming (API de OpenAI, que
//! hablan también DeepSeek, OpenRouter y la capa de compatibilidad de Claude) y el bucle de
//! herramientas. Todo ocurre en un hilo y llega a la interfaz como eventos.
//!
//! Para gastar pocos tokens, cada pregunta envía las instrucciones (iguales siempre, de modo que
//! los servicios que guardan el prefijo en caché lo reutilizan), las últimas respuestas recortadas
//! y la pregunta, y no los resultados de las herramientas de preguntas anteriores.

use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use hipso_core::json::Json;

use super::config::{Config, provider};
use super::docs::cut;
use super::jsonw::{arr, dump, n, num, obj, s, text};
use super::store::{Message, Who};
use super::tools::{self, Env};

/// Lo que el hilo cuenta a la interfaz.
pub enum Event {
    Token(String),
    /// El asistente consulta una herramienta.
    Tool(String),
    Flow(std::path::PathBuf),
    /// Tokens enviados y recibidos en toda la respuesta.
    Usage(u32, u32),
    Done,
    Error(String),
}

pub struct Request {
    pub cfg: Config,
    pub history: Vec<Message>,
    pub question: String,
    pub env: Env,
}

/// Rondas de herramientas como mucho en una respuesta.
const MAX_ROUNDS: usize = 6;
/// Las respuestas anteriores a las dos últimas se recortan a esto, en caracteres.
const OLD_ANSWER: usize = 1200;

const BASE: &str = "Eres el asistente integrado en HIPSO, un programa de LiDAR forestal, ráster y vectorial. Respondes siempre en español.
Reglas:
- No inventes funciones, opciones ni cifras. Lo que dices de HIPSO sale de las herramientas de documentación (que_es_hipso, listar_herramientas, ayuda_herramienta, buscar_manual, leer_seccion); no las vuelvas a llamar si ya tienes la respuesta en la conversación.
- Gasta pocos tokens: una búsqueda concreta, lee solo el apartado necesario, no repitas lo ya dicho ni pegues documentación literal.
- Para los datos de la persona usa capas_abiertas, describir_archivo y analisis_exploratorio, e interpreta los números en lugar de repetirlos todos.
- Para crear flujos usa crear_flujo (valida contra las herramientas reales); si devuelve errores, corrígelos y vuelve a llamar; después resume el flujo en pocas líneas. Tú no ejecutas herramientas: la persona ejecuta el flujo.
- Recomienda la mejor forma de hacer cada cosa en HIPSO (qué herramienta, con qué parámetros y por qué) y avisa de supuestos y límites. Si falta un dato imprescindible, pregunta una sola cosa.
- Las órdenes se escriben `hipso <herramienta> …`; nunca inventes comandos ni opciones.";

/// Las instrucciones del asistente: las reglas, la extensión de las respuestas y el perfil elegido.
/// No llevan nada que cambie de una pregunta a otra.
pub fn system_prompt(cfg: &Config) -> String {
    let mut out = format!("{BASE}\n{}", cfg.style.prompt());
    let role = cfg.role_text();
    if !role.is_empty() {
        out.push_str(&format!(
            "\n\nPerfil elegido por la persona ({}). Define tu enfoque y tu estilo; las reglas anteriores siguen mandando.\n{role}",
            cfg.role_name()
        ));
    }
    out
}

/// Los mensajes que se envían: instrucciones, historia reciente (las respuestas viejas, recortadas)
/// y la pregunta.
pub fn build_messages(cfg: &Config, history: &[Message], question: &str) -> Json {
    let mut msgs = vec![obj(vec![("role", s("system")), ("content", s(system_prompt(cfg)))])];
    let kept: Vec<&Message> = history.iter().filter(|m| !m.error && !m.text.trim().is_empty()).collect();
    let skip = kept.len().saturating_sub(cfg.history);
    let total = kept.len();
    for (i, m) in kept.into_iter().enumerate().skip(skip) {
        let recent = i + 2 >= total;
        let content = if m.who == Who::Assistant && !recent { cut(&m.text, OLD_ANSWER) } else { m.text.clone() };
        msgs.push(obj(vec![("role", s(if m.who == Who::User { "user" } else { "assistant" })), ("content", s(content))]));
    }
    msgs.push(obj(vec![("role", s("user")), ("content", s(question))]));
    arr(msgs)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent("HIPSO-asistente")
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(120)))
        .timeout_global(Some(Duration::from_secs(900)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn authorize(req: ureq::RequestBuilder<ureq::typestate::WithBody>, cfg: &Config) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
    let key = cfg.current().key;
    let req = req.header("Authorization", format!("Bearer {}", key.trim())).header("Content-Type", "application/json");
    match cfg.provider.as_str() {
        "openrouter" => req.header("X-Title", "HIPSO"),
        "anthropic" => req.header("x-api-key", key.trim()).header("anthropic-version", "2023-06-01"),
        _ => req,
    }
}

/// El motivo que da el servicio ante un error, en una frase.
fn explain(code: u16, body: &str) -> String {
    let detail = Json::parse(body)
        .ok()
        .and_then(|j| {
            j.get("error")
                .and_then(|e| e.get("message").and_then(Json::as_str).or(e.as_str()))
                .or_else(|| j.get("message").and_then(Json::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| cut(body.trim(), 200));
    let hint = match code {
        401 | 403 => " Revisa la clave en Ajustes → Asistente de IA.",
        402 => " Puede que la cuenta no tenga saldo.",
        404 => " Revisa el nombre del modelo.",
        429 => " Demasiadas peticiones o sin cuota: espera unos segundos.",
        _ => "",
    };
    format!("Error {code} del servicio: {}{hint}", if detail.is_empty() { "sin detalle".into() } else { detail })
}

fn net_error(e: &ureq::Error) -> String {
    format!("Sin conexión con el servicio de IA ({e}). Comprueba Internet; el resto de HIPSO funciona sin ella.")
}

struct Round {
    content: String,
    calls: Vec<(String, String, String)>,
    usage: Option<(u32, u32)>,
}

fn stream_round(cfg: &Config, messages: &Json, with_tools: bool, tx: &Sender<Event>, cancel: &AtomicBool) -> Result<Round, String> {
    let p = provider(&cfg.provider);
    let mut members = vec![
        ("model", s(cfg.model())),
        ("messages", messages.clone()),
        ("stream", Json::Bool(true)),
        ("stream_options", obj(vec![("include_usage", Json::Bool(true))])),
        (if p.openai_style { "max_completion_tokens" } else { "max_tokens" }, n(f64::from(cfg.max_output))),
    ];
    if !p.openai_style {
        members.push(("temperature", n(0.3)));
    }
    if with_tools {
        members.push(("tools", tools::definitions()));
    }
    let url = format!("{}/chat/completions", cfg.base());
    let mut resp = authorize(agent().post(&url), cfg).send(dump(&obj(members))).map_err(|e| net_error(&e))?;
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(explain(code, &body));
    }
    let mut round = Round { content: String::new(), calls: Vec::new(), usage: None };
    let reader = BufReader::new(resp.body_mut().as_reader());
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Detenido.".into());
        }
        let line = line.map_err(|e| format!("Se cortó la conexión con la IA: {e}"))?;
        let Some(data) = line.trim().strip_prefix("data:") else { continue };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(chunk) = Json::parse(data) else { continue };
        if let Some(u) = chunk.get("usage") {
            round.usage = Some((num(u, "prompt_tokens").unwrap_or(0.0) as u32, num(u, "completion_tokens").unwrap_or(0.0) as u32));
        }
        let Some(delta) = chunk.get("choices").and_then(Json::as_array).and_then(|c| c.first()).and_then(|c| c.get("delta")) else { continue };
        if let Some(t) = delta.get("content").and_then(Json::as_str)
            && !t.is_empty()
        {
            round.content.push_str(t);
            let _ = tx.send(Event::Token(t.to_string()));
        }
        for tc in delta.get("tool_calls").and_then(Json::as_array).unwrap_or(&[]) {
            let i = num(tc, "index").unwrap_or(0.0) as usize;
            while round.calls.len() <= i {
                round.calls.push((String::new(), String::new(), String::new()));
            }
            if let Some(id) = tc.get("id").and_then(Json::as_str) {
                round.calls[i].0 = id.to_string();
            }
            if let Some(f) = tc.get("function") {
                round.calls[i].1.push_str(&text(f, "name"));
                round.calls[i].2.push_str(&text(f, "arguments"));
            }
        }
    }
    round.calls.retain(|c| !c.1.is_empty());
    Ok(round)
}

/// La respuesta completa: rondas de modelo y herramientas hasta que el modelo contesta.
fn answer(req: &Request, tx: &Sender<Event>, cancel: &AtomicBool) -> Result<(), String> {
    let cfg = &req.cfg;
    if !cfg.has_key() {
        return Err("Falta la clave de la API: ponla en Ajustes → Asistente de IA.".into());
    }
    if cfg.base().is_empty() {
        return Err("Falta la dirección del servicio: ponla en Ajustes → Asistente de IA.".into());
    }
    let mut messages = match build_messages(cfg, &req.history, &req.question) {
        Json::Arr(a) => a,
        _ => Vec::new(),
    };
    let (mut sent, mut got) = (0u32, 0u32);
    for round_no in 0..=MAX_ROUNDS {
        let last = round_no == MAX_ROUNDS;
        let round = stream_round(cfg, &Json::Arr(messages.clone()), !last, tx, cancel)?;
        if let Some((a, b)) = round.usage {
            sent += a;
            got += b;
        }
        if round.calls.is_empty() {
            break;
        }
        // lo que el modelo dijo antes de llamar a las herramientas queda separado de lo que sigue
        if !round.content.trim().is_empty() {
            let _ = tx.send(Event::Token("\n\n".into()));
        }
        let calls: Vec<Json> = round
            .calls
            .iter()
            .map(|(id, name, args)| {
                obj(vec![
                    ("id", s(id)),
                    ("type", s("function")),
                    ("function", obj(vec![("name", s(name)), ("arguments", s(if args.trim().is_empty() { "{}" } else { args }))])),
                ])
            })
            .collect();
        messages.push(obj(vec![("role", s("assistant")), ("content", s(&round.content)), ("tool_calls", arr(calls))]));
        for (id, name, args) in &round.calls {
            if cancel.load(Ordering::Relaxed) {
                return Err("Detenido.".into());
            }
            let _ = tx.send(Event::Tool(name.clone()));
            let parsed = Json::parse(if args.trim().is_empty() { "{}" } else { args }).unwrap_or(Json::Obj(Vec::new()));
            let out = tools::run(name, &parsed, &req.env);
            if let Some(f) = out.flow {
                let _ = tx.send(Event::Flow(f));
            }
            messages.push(obj(vec![("role", s("tool")), ("tool_call_id", s(id)), ("content", s(out.text))]));
        }
    }
    if sent + got > 0 {
        let _ = tx.send(Event::Usage(sent, got));
    }
    Ok(())
}

/// Lanza la respuesta en un hilo.
pub fn spawn(req: Request, tx: Sender<Event>, cancel: Arc<AtomicBool>, ctx: eframe::egui::Context) {
    std::thread::spawn(move || {
        let result = answer(&req, &tx, &cancel);
        let _ = tx.send(match result {
            Ok(()) => Event::Done,
            Err(e) => Event::Error(e),
        });
        ctx.request_repaint();
    });
}

/// Comprueba la clave y el modelo con una petición mínima.
pub fn test(cfg: &Config) -> Result<String, String> {
    if !cfg.has_key() {
        return Err("Falta la clave de la API.".into());
    }
    let p = provider(&cfg.provider);
    let body = obj(vec![
        ("model", s(cfg.model())),
        ("messages", arr(vec![obj(vec![("role", s("user")), ("content", s("Responde solo: ok"))])])),
        (if p.openai_style { "max_completion_tokens" } else { "max_tokens" }, n(32.0)),
    ]);
    let url = format!("{}/chat/completions", cfg.base());
    let mut resp = authorize(agent().post(&url), cfg).send(dump(&body)).map_err(|e| net_error(&e))?;
    let code = resp.status().as_u16();
    if (200..300).contains(&code) {
        return Ok(format!("Conexión correcta con {} (modelo {}).", p.name, cfg.model()));
    }
    Err(explain(code, &resp.body_mut().read_to_string().unwrap_or_default()))
}

/// Los modelos que ofrece el servicio.
pub fn list_models(cfg: &Config) -> Result<Vec<String>, String> {
    if !cfg.has_key() {
        return Err("Pega primero la clave de la API.".into());
    }
    let url = format!("{}/models", cfg.base());
    let key = cfg.current().key;
    let mut req = agent().get(&url).header("Authorization", format!("Bearer {}", key.trim()));
    if cfg.provider == "anthropic" {
        req = req.header("x-api-key", key.trim()).header("anthropic-version", "2023-06-01");
    }
    let mut resp = req.call().map_err(|e| net_error(&e))?;
    let code = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if !(200..300).contains(&code) {
        return Err(format!("{} Escribe el nombre del modelo a mano.", explain(code, &body)));
    }
    let j = Json::parse(&body).map_err(|e| e.to_string())?;
    let mut ids: Vec<String> = j.get("data").and_then(Json::as_array).unwrap_or(&[]).iter().map(|m| text(m, "id")).filter(|i| !i.is_empty()).collect();
    ids.sort();
    if ids.is_empty() {
        return Err("El servicio no devolvió ningún modelo; escribe el nombre a mano.".into());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::Style;

    fn msgs(j: &Json) -> Vec<(String, String)> {
        j.as_array().unwrap_or(&[]).iter().map(|m| (text(m, "role"), text(m, "content"))).collect()
    }

    #[test]
    fn the_prompt_is_stable_small_and_carries_the_role() {
        let mut c = Config::default();
        let base = system_prompt(&c);
        assert_eq!(base, system_prompt(&c), "debe ser idéntico siempre para que el servicio lo guarde en caché");
        assert!(base.chars().count() < 2200, "{}", base.chars().count());
        assert!(!base.contains("Perfil elegido"));
        c.role = "gis_editor".into();
        c.style = Style::Detailed;
        let with = system_prompt(&c);
        assert!(with.contains("Perfil elegido por la persona (GIS Editor)") && with.contains("Editor de Datos GIS"));
        assert!(with.starts_with(BASE));
    }

    #[test]
    fn the_history_is_windowed_and_old_answers_are_cut() {
        let mut c = Config::default();
        c.history = 4;
        let long = "x".repeat(3000);
        let h: Vec<Message> = (0..10)
            .flat_map(|i| [Message::user(format!("p{i}")), Message::assistant(format!("{long}{i}"))])
            .collect();
        let m = msgs(&build_messages(&c, &h, "nueva"));
        // sistema + 4 del historial + la pregunta
        assert_eq!(m.len(), 6);
        assert_eq!(m[0].0, "system");
        assert_eq!(m[5], ("user".to_string(), "nueva".to_string()));
        // de las respuestas, la penúltima va recortada y la última entera
        let answers: Vec<&String> = m.iter().filter(|x| x.0 == "assistant").map(|x| &x.1).collect();
        assert_eq!(answers.len(), 2);
        assert!(answers[0].chars().count() <= OLD_ANSWER + 1);
        assert!(answers[1].ends_with("9") && answers[1].chars().count() > 3000);
        // los errores y los mensajes vacíos no se envían
        let mut e = Message::assistant("falló");
        e.error = true;
        let m2 = msgs(&build_messages(&c, &[e, Message::user("  ")], "q"));
        assert_eq!(m2.len(), 2);
    }

    #[test]
    fn service_errors_say_why() {
        let e = explain(401, r#"{"error":{"message":"Invalid API key"}}"#);
        assert!(e.contains("Invalid API key") && e.contains("clave"), "{e}");
        assert!(explain(402, "{}").contains("saldo"));
        assert!(explain(500, "caído").contains("caído"));
    }

    trait ErrText {
        fn unwrap_err_or_default(self) -> String;
    }
    impl ErrText for Result<(), String> {
        fn unwrap_err_or_default(self) -> String {
            self.err().unwrap_or_default()
        }
    }

    /// Un servicio de mentira que habla la API de OpenAI en streaming: la primera petición pide una
    /// herramienta, la segunda contesta con el resultado.
    fn mock_server() -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("puerto");
        let addr = listener.local_addr().expect("dirección");
        let handle = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for call in 0..2 {
                let Ok((mut sock, _)) = listener.accept() else { break };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (mut header_end, mut want) = (None, 0usize);
                loop {
                    let n = sock.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if header_end.is_none()
                        && let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(p + 4);
                            let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                            want = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                        }
                    if let Some(h) = header_end
                        && buf.len() >= h + want {
                            break;
                        }
                }
                let h = header_end.unwrap_or(0);
                bodies.push(String::from_utf8_lossy(&buf[h..]).into_owned());
                let sse = if call == 0 {
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"que_es_hipso\",\"arguments\":\"\"}}]}}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{}\"}}]}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":5}}\n\ndata: [DONE]\n\n"
                } else {
                    "data: {\"choices\":[{\"delta\":{\"content\":\"HIPSO \"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"es libre.\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":400,\"completion_tokens\":7}}\n\ndata: [DONE]\n\n"
                };
                let _ = write!(sock, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{sse}", sse.len());
            }
            bodies
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn a_full_answer_with_a_tool_round_against_a_local_service() {
        let (base, server) = mock_server();
        let mut cfg = Config::default();
        cfg.provider = "custom".into();
        cfg.entry("custom").base = base;
        cfg.entry("custom").key = "sk-prueba".into();
        cfg.entry("custom").model = "m".into();
        let req = Request { cfg, history: vec![Message::user("antes"), Message::assistant("respuesta")], question: "¿Qué es HIPSO?".into(), env: Env::default() };
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(false);
        assert_eq!(answer(&req, &tx, &cancel), Ok(()));
        drop(tx);
        let (mut text, mut tools_used, mut usage) = (String::new(), Vec::new(), (0, 0));
        for ev in rx {
            match ev {
                Event::Token(t) => text.push_str(&t),
                Event::Tool(n) => tools_used.push(n),
                Event::Usage(a, b) => usage = (a, b),
                _ => {}
            }
        }
        assert_eq!(text, "HIPSO es libre.");
        assert_eq!(tools_used, vec!["que_es_hipso".to_string()]);
        assert_eq!(usage, (500, 12));
        let bodies = server.join().unwrap_or_default();
        assert_eq!(bodies.len(), 2);
        // la primera petición lleva las instrucciones, la historia, la pregunta y las herramientas
        let first = Json::parse(&bodies[0]).unwrap_or(Json::Null);
        let m = first.get("messages").and_then(Json::as_array).map_or(0, <[Json]>::len);
        assert_eq!(m, 4);
        assert!(first.get("tools").is_some() && first.get("stream") == Some(&Json::Bool(true)));
        // la segunda añade lo que dijo el asistente y el resultado de la herramienta (el resumen de HIPSO)
        let second = Json::parse(&bodies[1]).unwrap_or(Json::Null);
        let msgs = second.get("messages").and_then(Json::as_array).unwrap_or(&[]);
        assert_eq!(msgs.len(), 6);
        assert_eq!(text_of(&msgs[4], "role"), "assistant");
        assert_eq!(text_of(&msgs[5], "role"), "tool");
        assert!(text_of(&msgs[5], "content").contains("HIPSO es un programa libre"));
    }

    fn text_of(j: &Json, k: &str) -> String {
        text(j, k)
    }

    #[test]
    fn a_missing_key_is_said_clearly() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let req = Request { cfg: Config::default(), history: Vec::new(), question: "hola".into(), env: Env::default() };
        let e = answer(&req, &tx, &AtomicBool::new(false)).unwrap_err_or_default();
        assert!(e.contains("clave"), "{e}");
    }

    /// Prueba contra el servicio real; se ejecuta a mano con `HIPSO_AI_KEY` y, si hace falta,
    /// `HIPSO_AI_PROVIDER` (por defecto deepseek): `cargo test -p hipso-gui real_service -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_service_answers_with_tools() {
        let key = std::env::var("HIPSO_AI_KEY").unwrap_or_default();
        if key.is_empty() {
            return;
        }
        let mut cfg = Config::default();
        cfg.provider = std::env::var("HIPSO_AI_PROVIDER").unwrap_or_else(|_| "deepseek".into());
        cfg.entry(&cfg.provider.clone()).key = key;
        cfg.style = crate::ai::config::Style::Brief;
        let env = Env { layers: Vec::new(), schema: None, flows_dir: Some(std::env::temp_dir().join("hipso_ai_real_flows")) };
        for q in [
            "¿Qué es HIPSO? Respóndeme en 5 líneas.",
            "Explícame qué hace la herramienta canopy-gaps y sus opciones principales.",
            "Crea un flujo llamado prueba que haga el MDT de una nube y luego el CHM usando ese MDT.",
        ] {
            let req = Request { cfg: cfg.clone(), history: Vec::new(), question: q.into(), env: env.clone() };
            let (tx, rx) = std::sync::mpsc::channel();
            let r = answer(&req, &tx, &AtomicBool::new(false));
            drop(tx);
            let (mut text, mut tools_used, mut usage, mut flow) = (String::new(), Vec::new(), (0, 0), None);
            for ev in rx {
                match ev {
                    Event::Token(t) => text.push_str(&t),
                    Event::Tool(n) => tools_used.push(n),
                    Event::Usage(a, b) => usage = (a, b),
                    Event::Flow(f) => flow = Some(f),
                    _ => {}
                }
            }
            println!("\n=== {q}\nresultado: {r:?}\nherramientas: {tools_used:?}\ntokens: {usage:?}\nflujo: {flow:?}\n{text}");
        }
    }
}
