//! `hipso mcp`: un servidor del Protocolo de Contexto de Modelo (MCP) por la entrada y la salida
//! estándar. Cada orden de `hipso` es una herramienta cuyas opciones salen del mismo esquema que usa
//! el constructor de flujos (`hipso schema`), de modo que una orden nueva aparece sola. Una llamada
//! lanza este mismo ejecutable con la orden y devuelve lo que escribió.
//!
//! El transporte es JSON-RPC 2.0, un mensaje por línea. Las llamadas se ejecutan cada una en su
//! hilo, de modo que una orden larga no impide contestar a las demás peticiones.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use hipso_core::json::{Json, quote};
use hipso_core::model::{ArgSpec, Schema, ToolSpec};
use hipso_core::{Error, Result};

/// Versión del protocolo que se ofrece cuando el cliente no pide una.
const PROTOCOL: &str = "2025-06-18";

/// Cuánto de la salida de una orden se devuelve (los últimos caracteres).
const MAX_OUTPUT: usize = 30_000;

fn property(a: &ArgSpec) -> String {
    let kind = if a.flag {
        "boolean"
    } else if a.multiple {
        "array"
    } else if a
        .default
        .as_deref()
        .is_some_and(|d| d.parse::<f64>().is_ok())
    {
        "number"
    } else {
        "string"
    };
    let mut help = a.help.clone();
    if a.path {
        help.push_str(" (ruta de archivo o carpeta)");
    }
    if let Some(d) = a.default.as_deref().filter(|d| !d.is_empty()) {
        help.push_str(&format!(" (por defecto: {d})"));
    }
    let mut s = format!(
        "{{\"type\":\"{kind}\",\"description\":{}",
        quote(help.trim())
    );
    if kind == "array" {
        s.push_str(",\"items\":{\"type\":\"string\"}");
    }
    if !a.values.is_empty() && !a.multiple {
        s.push_str(&format!(
            ",\"enum\":[{}]",
            a.values
                .iter()
                .map(|v| quote(v))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    s.push('}');
    s
}

fn tool_json(t: &ToolSpec) -> String {
    let props: Vec<String> = t
        .args
        .iter()
        .map(|a| format!("{}:{}", quote(&a.id), property(a)))
        .collect();
    let required: Vec<String> = t
        .args
        .iter()
        .filter(|a| a.required)
        .map(|a| quote(&a.id))
        .collect();
    format!(
        "{{\"name\":{},\"description\":{},\"inputSchema\":{{\"type\":\"object\",\"properties\":{{{}}},\"required\":[{}]}}}}",
        quote(&t.name),
        quote(&t.about),
        props.join(","),
        required.join(",")
    )
}

/// Los valores de un argumento JSON como textos: un número, un texto o una lista.
fn texts(v: &Json) -> Vec<String> {
    match v {
        Json::Str(s) => vec![s.clone()],
        Json::Num(n) => vec![if n.fract() == 0.0 && n.abs() < 1e15 {
            format!("{}", *n as i64)
        } else {
            n.to_string()
        }],
        Json::Bool(b) => vec![b.to_string()],
        Json::Arr(a) => a.iter().flat_map(texts).collect(),
        _ => Vec::new(),
    }
}

/// La línea de órdenes de una llamada.
fn argv(spec: &ToolSpec, args: &Json) -> Result<Vec<String>> {
    let mut given: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    if let Json::Obj(members) = args {
        for (k, v) in members {
            let Some(a) = spec.arg(k) else {
                return Err(Error::Param(format!(
                    "«{k}» no es una opción de {} (hay: {})",
                    spec.name,
                    spec.args
                        .iter()
                        .map(|a| a.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            };
            given.insert(a.id.as_str(), texts(v));
        }
    }
    let mut opts = Vec::new();
    let mut positional = Vec::new();
    for a in &spec.args {
        let Some(values) = given.get(a.id.as_str()) else {
            if a.required {
                return Err(Error::Param(format!("falta «{}»", a.id)));
            }
            continue;
        };
        if values.is_empty() {
            continue;
        }
        if a.positional {
            positional.extend(values.iter().cloned());
        } else if let Some(l) = &a.long {
            if a.flag {
                if values[0] == "true" {
                    opts.push(format!("--{l}"));
                }
            } else if a.multiple {
                for v in values {
                    opts.push(format!("--{l}"));
                    opts.push(v.clone());
                }
            } else {
                opts.push(format!("--{l}"));
                opts.push(values[0].clone());
            }
        }
    }
    let mut line = vec![spec.name.clone()];
    line.extend(opts);
    // `--` antes de los posicionales: una ruta que empieza por `-` no se toma por opción
    line.push("--".into());
    line.extend(positional);
    Ok(line)
}

fn tail(s: &str) -> String {
    if s.chars().count() <= MAX_OUTPUT {
        return s.to_string();
    }
    let skip = s.chars().count() - MAX_OUTPUT;
    format!("…{}", s.chars().skip(skip).collect::<String>())
}

/// Ejecuta una herramienta y devuelve su salida y si salió bien.
fn call(schema: &Schema, name: &str, args: &Json) -> (String, bool) {
    let Some(spec) = schema.tool(name) else {
        return (format!("no hay una herramienta «{name}»"), true);
    };
    let line = match argv(spec, args) {
        Ok(l) => l,
        Err(e) => return (e.to_string(), true),
    };
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return (format!("no se encuentra el ejecutable: {e}"), true),
    };
    match std::process::Command::new(exe)
        .args(&line)
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.trim().is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&err);
            }
            (tail(text.trim_end()), !o.status.success())
        }
        Err(e) => (format!("no se pudo lanzar la orden: {e}"), true),
    }
}

fn send(out: &Mutex<std::io::Stdout>, line: &str) {
    if let Ok(mut o) = out.lock() {
        let _ = writeln!(o, "{line}");
        let _ = o.flush();
    }
}

fn id_json(id: &Json) -> String {
    match id {
        Json::Num(n) => format!("{}", *n as i64),
        Json::Str(s) => quote(s),
        _ => "null".into(),
    }
}

fn error(id: &Json, code: i64, message: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":{code},\"message\":{}}}}}",
        id_json(id),
        quote(message)
    )
}

/// Cuántas órdenes se ejecutan a la vez; las demás llamadas esperan su turno.
const MAX_RUNNING: usize = 4;

/// Un cupo de [`MAX_RUNNING`] órdenes a la vez.
#[derive(Default)]
struct Slots {
    taken: Mutex<usize>,
    freed: Condvar,
}

impl Slots {
    /// Ejecuta `f` cuando haya un hueco.
    fn run<T>(&self, f: impl FnOnce() -> T) -> T {
        let guard = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        let mut guard = self
            .freed
            .wait_while(guard, |n| *n >= MAX_RUNNING)
            .unwrap_or_else(PoisonError::into_inner);
        *guard += 1;
        drop(guard);
        let out = f();
        *self.taken.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
        self.freed.notify_one();
        out
    }
}

/// Atiende peticiones hasta que se cierra la entrada.
pub fn serve(schema: Schema) -> Result<()> {
    let schema = Arc::new(schema);
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let slots = Arc::new(Slots::default());
    let mut workers = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg = match Json::parse(&line) {
            Ok(m) => m,
            Err(e) => {
                send(&out, &error(&Json::Null, -32700, &e.to_string()));
                continue;
            }
        };
        let method = msg
            .get("method")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_string();
        let id = msg.get("id").cloned();
        // una notificación no tiene id ni respuesta
        let Some(id) = id else { continue };
        match method.as_str() {
            "initialize" => {
                let version = msg
                    .get("params")
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(Json::as_str)
                    .unwrap_or(PROTOCOL);
                send(
                    &out,
                    &format!(
                        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":{},\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"hipso\",\"version\":{}}},\"instructions\":{}}}}}",
                        id_json(&id),
                        quote(version),
                        quote(env!("CARGO_PKG_VERSION")),
                        quote(
                            "Cada herramienta es una orden de hipso (procesado de nubes de puntos LiDAR, ráster y vectorial). Las rutas son del equipo donde corre el servidor."
                        )
                    ),
                );
            }
            "ping" => send(
                &out,
                &format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{}}}}",
                    id_json(&id)
                ),
            ),
            "tools/list" => {
                let tools: Vec<String> = schema.tools.iter().map(tool_json).collect();
                send(
                    &out,
                    &format!(
                        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"tools\":[{}]}}}}",
                        id_json(&id),
                        tools.join(",")
                    ),
                );
            }
            "tools/call" => {
                let params = msg.get("params").cloned().unwrap_or(Json::Null);
                let name = params
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_string();
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(Json::Obj(Vec::new()));
                let (schema, out) = (Arc::clone(&schema), Arc::clone(&out));
                let slots = Arc::clone(&slots);
                workers.push(std::thread::spawn(move || {
                    let (text, failed) = slots.run(|| call(&schema, &name, &args));
                    send(
                        &out,
                        &format!(
                            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":{}}}],\"isError\":{failed}}}}}",
                            id_json(&id),
                            quote(&text)
                        ),
                    );
                }));
                workers.retain(|w| !w.is_finished());
            }
            _ => send(
                &out,
                &error(&id, -32601, &format!("método desconocido: {method}")),
            ),
        }
    }
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "chm".into(),
            about: "x".into(),
            args: vec![
                ArgSpec {
                    id: "inputs".into(),
                    positional: true,
                    required: true,
                    path: true,
                    multiple: true,
                    ..Default::default()
                },
                ArgSpec {
                    id: "cell".into(),
                    long: Some("cell".into()),
                    default: Some("1".into()),
                    ..Default::default()
                },
                ArgSpec {
                    id: "pitfree".into(),
                    long: Some("pitfree".into()),
                    flag: true,
                    ..Default::default()
                },
                ArgSpec {
                    id: "class".into(),
                    long: Some("class".into()),
                    multiple: true,
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn arguments_become_a_command_line() -> Result<()> {
        let args = Json::parse(
            r#"{"inputs":["a.laz","-b.laz"],"cell":0.5,"pitfree":true,"class":["2","5"]}"#,
        )?;
        let l = argv(&spec(), &args)?;
        assert_eq!(
            l,
            [
                "chm",
                "--cell",
                "0.5",
                "--pitfree",
                "--class",
                "2",
                "--class",
                "5",
                "--",
                "a.laz",
                "-b.laz"
            ]
        );
        Ok(())
    }

    #[test]
    fn false_flags_and_missing_required_are_handled() -> Result<()> {
        let l = argv(
            &spec(),
            &Json::parse(r#"{"inputs":"a.laz","pitfree":false}"#)?,
        )?;
        assert_eq!(l, ["chm", "--", "a.laz"]);
        assert!(argv(&spec(), &Json::parse("{}")?).is_err());
        assert!(argv(&spec(), &Json::parse(r#"{"inputs":"a","nada":1}"#)?).is_err());
        Ok(())
    }

    #[test]
    fn the_tool_description_is_valid_json() -> Result<()> {
        let j = Json::parse(&tool_json(&spec()))?;
        assert_eq!(j.get("name").and_then(Json::as_str), Some("chm"));
        let props = j.get("inputSchema").and_then(|s| s.get("properties"));
        assert_eq!(
            props
                .and_then(|p| p.get("cell"))
                .and_then(|c| c.get("type"))
                .and_then(Json::as_str),
            Some("number")
        );
        assert_eq!(
            props
                .and_then(|p| p.get("pitfree"))
                .and_then(|c| c.get("type"))
                .and_then(Json::as_str),
            Some("boolean")
        );
        Ok(())
    }
}
