//! Ventana «Ver nubes TLS» de HIPSO.
//!
//! Abre una ventana propia con un visor de nubes terrestres densas (millones de puntos con todo el
//! detalle al acercarse). Dibuja con el visor de código abierto Potree (licencia BSD de 2
//! cláusulas, véase `THIRD_PARTY_LICENSES.md`) dentro de una vista web del sistema, pero no abre
//! ningún navegador ni ningún servidor: los recursos van dentro del ejecutable y los datos se
//! leen del disco por un protocolo propio (`hipso://`), con soporte de lecturas por rango.
//!
//! Uso: `hipso-tls [carpeta convertida | nube.las | nube.laz] [--tema claro|oscuro] [--titulo texto]`.
//! La carpeta es la que deja la conversión: `metadata.json`, `hierarchy.bin` y `octree.bin`. Con un
//! LAS o LAZ la convierte primero (queda en la caché) y sin nada abre la ventana vacía: el botón
//! «Abrir nube…» de la cinta elige una.

use std::borrow::Cow;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use include_dir::{Dir, include_dir};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::window::{Icon, Theme, WindowBuilder};
use wry::http::{Request, Response, StatusCode, header};
use wry::{WebViewBuilder, WebViewId};

/// Recursos del visor (la página, Potree y sus dependencias), incrustados en el ejecutable.
static WEB: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/web");

/// Icono de la ventana (solo lo usan Windows y Linux; en macOS lo da el paquete de la aplicación).
const ICONO: &[u8] = include_bytes!("../../../assets/icon-256.png");

/// Los argumentos de la orden.
struct Args {
    /// Lo que se abre al empezar: una carpeta convertida o una nube por convertir.
    datos: Option<PathBuf>,
    oscuro: bool,
    titulo: String,
}

fn leer_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut datos = None;
    let mut oscuro = false;
    let mut titulo = String::from("Visor de nubes");
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tema" => oscuro = it.next().as_deref() == Some("oscuro"),
            "--titulo" => titulo = it.next().unwrap_or(titulo),
            _ if datos.is_none() => datos = Some(PathBuf::from(a)),
            _ => return Err(format!("argumento no reconocido: {a}")),
        }
    }
    if let Some(d) = &datos
        && !d.join("metadata.json").is_file()
        && !es_nube(d)
        && listar_nubes(d).is_empty()
    {
        return Err(format!("{} no es una carpeta convertida ni un LAS o LAZ", d.display()));
    }
    Ok(Args {
        datos,
        oscuro,
        titulo,
    })
}

/// Los nombres de las subcarpetas de `raiz` que son nubes convertidas, en orden.
fn listar_nubes(raiz: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(raiz)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("metadata.json").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    v.sort();
    v
}

/// La carpeta `nubes` que viaja junto al visor (en macOS, junto a la aplicación `.app`), con las
/// nubes que se entregan a un cliente.
fn carpeta_cliente() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut candidatas = vec![dir.join("nubes")];
    if let Some(fuera) = dir.parent().and_then(Path::parent).and_then(Path::parent)
        && dir.ends_with("Contents/MacOS")
    {
        candidatas.push(fuera.join("nubes"));
    }
    candidatas
        .into_iter()
        .find(|c| c.is_dir() && !listar_nubes(c).is_empty())
}

/// Si `p` es un archivo LAS o LAZ.
fn es_nube(p: &Path) -> bool {
    p.is_file()
        && p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("las") || e.eq_ignore_ascii_case("laz"))
}

/// El texto de `s` como literal de JavaScript entre comillas.
fn js_texto(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' | '\r' => o.push(' '),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// El tipo MIME de un archivo por su extensión.
fn mime(ruta: &str) -> &'static str {
    match ruta.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Un rango `bytes=a-b` (o `bytes=a-`, o `bytes=-n`) como posiciones inclusivas dentro de `len`.
fn rango(cabecera: &str, len: u64) -> Option<(u64, u64)> {
    let r = cabecera.trim().strip_prefix("bytes=")?;
    let (a, b) = r.split_once('-')?;
    if len == 0 {
        return None;
    }
    let (ini, fin) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len - 1)
    } else {
        let ini: u64 = a.parse().ok()?;
        let fin = if b.is_empty() {
            len - 1
        } else {
            b.parse::<u64>().ok()?.min(len - 1)
        };
        (ini, fin)
    };
    (ini <= fin && ini < len).then_some((ini, fin))
}

fn respuesta(
    estado: StatusCode,
    tipo: &str,
    cuerpo: Vec<u8>,
    extra: &[(&str, String)],
) -> Response<Cow<'static, [u8]>> {
    let mut r = Response::builder()
        .status(estado)
        .header(header::CONTENT_TYPE, tipo)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "no-store");
    for (k, v) in extra {
        r = r.header(*k, v.as_str());
    }
    r.body(Cow::Owned(cuerpo)).unwrap_or_else(|_| {
        let mut vacia = Response::new(Cow::Borrowed(&[][..]));
        *vacia.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        vacia
    })
}

fn error(estado: StatusCode) -> Response<Cow<'static, [u8]>> {
    respuesta(estado, "text/plain; charset=utf-8", Vec::new(), &[])
}

/// Sirve un archivo de la carpeta de datos, entero o por rango.
fn servir_dato(datos: &Path, relativa: &str, cabecera_rango: Option<&str>) -> Response<Cow<'static, [u8]>> {
    let rel = Path::new(relativa);
    // nada de salir de la carpeta de datos
    if rel
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return error(StatusCode::BAD_REQUEST);
    }
    let ruta = datos.join(rel);
    let Ok(mut f) = File::open(&ruta) else {
        return error(StatusCode::NOT_FOUND);
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let tipo = mime(relativa);
    match cabecera_rango.and_then(|h| rango(h, len)) {
        Some((ini, fin)) => {
            let n = fin - ini + 1;
            let mut buf = vec![0u8; n as usize];
            if f.seek(SeekFrom::Start(ini)).is_err() || f.read_exact(&mut buf).is_err() {
                return error(StatusCode::INTERNAL_SERVER_ERROR);
            }
            respuesta(
                StatusCode::PARTIAL_CONTENT,
                tipo,
                buf,
                &[(header::CONTENT_RANGE.as_str(), format!("bytes {ini}-{fin}/{len}"))],
            )
        }
        None => {
            let mut buf = Vec::new();
            if f.read_to_end(&mut buf).is_err() {
                return error(StatusCode::INTERNAL_SERVER_ERROR);
            }
            respuesta(StatusCode::OK, tipo, buf, &[])
        }
    }
}

/// Sirve un recurso del visor: incrustado, o de una carpeta si `HIPSO_TLS_WEB` la indica (para
/// probar cambios en la página sin recompilar).
fn servir_web(relativa: &str) -> Response<Cow<'static, [u8]>> {
    let relativa = if relativa.is_empty() { "index.html" } else { relativa };
    if let Some(dir) = std::env::var_os("HIPSO_TLS_WEB") {
        let rel = Path::new(relativa);
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return error(StatusCode::BAD_REQUEST);
        }
        return match std::fs::read(Path::new(&dir).join(rel)) {
            Ok(b) => respuesta(StatusCode::OK, mime(relativa), b, &[]),
            Err(_) => error(StatusCode::NOT_FOUND),
        };
    }
    match WEB.get_file(relativa) {
        Some(f) => respuesta(StatusCode::OK, mime(relativa), f.contents().to_vec(), &[]),
        None => error(StatusCode::NOT_FOUND),
    }
}

fn atender(datos: &Mutex<Option<PathBuf>>, req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let ruta = req.uri().path().trim_start_matches('/');
    let ruta = ruta.replace("%20", " ");
    match ruta.strip_prefix("data/") {
        Some(dato) => {
            let rango = req
                .headers()
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok());
            let carpeta = datos.lock().ok().and_then(|d| d.clone());
            if dato == "lista.json"
                && let Some(c) = &carpeta
            {
                let nombres: Vec<String> = listar_nubes(c).iter().map(|n| js_texto(n)).collect();
                let cuerpo = format!("[{}]", nombres.join(",")).into_bytes();
                return respuesta(StatusCode::OK, "application/json", cuerpo, &[]);
            }
            match carpeta {
                Some(c) => servir_dato(&c, dato, rango),
                None => error(StatusCode::NOT_FOUND),
            }
        }
        None => servir_web(&ruta),
    }
}

/// El icono de la ventana, o `None` si el PNG no es RGBA de 8 bits.
fn icono() -> Option<Icon> {
    let dec = png::Decoder::new(std::io::Cursor::new(ICONO));
    let mut lector = dec.read_info().ok()?;
    let mut buf = vec![0u8; lector.output_buffer_size()?];
    let info = lector.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Icon::from_rgba(buf, info.width, info.height).ok()
}

/// Codifica `t` para ir en una dirección.
fn codificar(t: &str) -> String {
    t.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Convierte `src` en segundo plano, cuenta el avance a la página y, al acabar, la recarga con la
/// nube nueva.
fn abrir_nube(
    src: PathBuf,
    datos: Arc<Mutex<Option<PathBuf>>>,
    proxy: EventLoopProxy<String>,
    oscuro: bool,
) {
    std::thread::spawn(move || {
        let titulo = hipso_core::paths::stem(&src);
        let _ = proxy.send_event(format!(
            "progreso({})",
            js_texto(&format!("Preparando «{titulo}»…"))
        ));
        let p = proxy.clone();
        let r = hipso_core::tls_octree::convertir(&src, &|n| {
            let _ = p.send_event(format!(
                "progreso({})",
                js_texto(&format!("Preparando «{titulo}»… {n:.0} %"))
            ));
        });
        match r {
            Ok(c) => {
                if let Ok(mut d) = datos.lock() {
                    *d = Some(c);
                }
                let _ = proxy.send_event(format!(
                    "location.href = \"index.html?tema={}&titulo={}\"",
                    if oscuro { "oscuro" } else { "claro" },
                    codificar(&titulo)
                ));
            }
            Err(e) => {
                let _ = proxy.send_event(format!("progreso({})", js_texto(&e)));
            }
        }
    });
}

fn principal() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = leer_args()?;
    // sin nada que abrir, el visor de un cliente: las nubes entregadas junto al programa
    if args.datos.is_none() {
        args.datos = carpeta_cliente();
        if let Some(c) = &args.datos
            && let Some(t) = c
                .parent()
                .and_then(|p| std::fs::read_to_string(p.join("titulo.txt")).ok())
            && !t.trim().is_empty()
        {
            args.titulo = t.trim().to_string();
        }
    }
    let cliente = args
        .datos
        .as_ref()
        .is_some_and(|d| d.is_dir() && !d.join("metadata.json").is_file());
    let datos: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(
        args.datos
            .clone()
            .filter(|d| d.join("metadata.json").is_file() || cliente),
    ));
    let evento: EventLoop<String> = EventLoopBuilder::with_user_event().build();
    let proxy = evento.create_proxy();
    let mut ventana = WindowBuilder::new()
        .with_title(format!("HIPSO · {}", args.titulo))
        .with_inner_size(LogicalSize::new(1500.0, 950.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .with_theme(Some(if args.oscuro { Theme::Dark } else { Theme::Light }));
    if let Some(i) = icono() {
        ventana = ventana.with_window_icon(Some(i));
    }
    let ventana = ventana.build(&evento)?;

    let mut url = format!(
        "hipso://localhost/index.html?tema={}&titulo={}",
        if args.oscuro { "oscuro" } else { "claro" },
        codificar(&args.titulo)
    );
    if cliente {
        url.push_str("&cliente=1");
    }
    // parámetros extra de arranque (color, filtros, cámara) para pruebas
    if let Ok(extra) = std::env::var("HIPSO_TLS_EXTRA") {
        url.push('&');
        url.push_str(&extra);
    }
    let (datos_p, datos_ipc, proxy_ipc, oscuro) = (
        datos.clone(),
        datos.clone(),
        proxy.clone(),
        args.oscuro,
    );
    let vista = WebViewBuilder::new()
        .with_asynchronous_custom_protocol(
            "hipso".into(),
            move |_id: WebViewId, req, responder| {
                let datos = datos_p.clone();
                std::thread::spawn(move || responder.respond(atender(&datos, &req)));
            },
        )
        .with_ipc_handler(move |req| {
            if req.body() == "abrir" {
                // el diálogo se abre en el hilo principal, que es donde corre este manejador
                let elegido = rfd::FileDialog::new()
                    .set_title("Abrir una nube de puntos")
                    .add_filter("Nubes de puntos (LAS, LAZ)", &["las", "laz", "LAS", "LAZ"])
                    .pick_file();
                if let Some(f) = elegido {
                    abrir_nube(f, datos_ipc.clone(), proxy_ipc.clone(), oscuro);
                }
            } else {
                eprintln!("[visor] {}", req.body());
            }
        })
        .with_devtools(std::env::var_os("HIPSO_TLS_DEBUG").is_some())
        .with_url(url)
        .build(&ventana)?;

    // una nube dada como archivo se convierte al empezar
    if let Some(d) = &args.datos
        && es_nube(d)
    {
        abrir_nube(d.clone(), datos.clone(), proxy.clone(), args.oscuro);
    }

    evento.run(move |ev, _, flujo| {
        *flujo = ControlFlow::Wait;
        match ev {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *flujo = ControlFlow::Exit,
            Event::UserEvent(js) => {
                let _ = vista.evaluate_script(&js);
            }
            _ => {}
        }
    });
}

fn main() {
    if let Err(e) = principal() {
        eprintln!("hipso-tls: {e}");
        std::process::exit(1);
    }
}
