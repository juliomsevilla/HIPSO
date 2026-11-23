//! Lo que la aplicación recuerda entre sesiones: la carpeta de la que se abrieron los últimos
//! archivos (donde empieza cada diálogo de archivos) y los archivos abiertos más recientemente. Se
//! guarda en un pequeño archivo de texto en la carpeta de configuración del usuario; si no se puede
//! leer o escribir, la aplicación funciona como antes.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Cuántos archivos recientes se conservan.
const RECENT: usize = 10;

/// Los ajustes que sin la aplicación se dan con variables de entorno. `None` en un campo deja
/// actuar a la variable (o al comportamiento automático).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Advanced {
    /// Carpetas con rejillas de ajuste de datum y de geoide de otros países.
    pub grid_dirs: Vec<PathBuf>,
    /// Descargar de la CDN de PROJ las rejillas que faltan.
    pub grid_network: Option<bool>,
    /// Descargarlas enteras en lugar de leer solo los bloques necesarios.
    pub grid_download: Option<bool>,
    /// Dirección de la CDN de rejillas (vacía: la de PROJ).
    pub grid_endpoint: String,
    /// Carpeta donde se guardan las rejillas descargadas.
    pub grid_cache: Option<PathBuf>,
    /// Permitir transformaciones aproximadas cuando falta la rejilla exacta.
    pub allow_ballpark: Option<bool>,
    /// Aceptar solo la mejor operación disponible.
    pub only_best: Option<bool>,
    /// Procesar rásteres grandes por franjas: siempre (`Some(true)`), nunca o según el tamaño.
    pub stream: Option<bool>,
    /// Renderizador de la interfaz: `wgpu`, `glow` o vacío (automático). Pide reiniciar.
    pub renderer: String,
}

struct Prefs {
    adv: Advanced,
    dir: Option<PathBuf>,
    recent: Vec<PathBuf>,
    /// La carpeta de la caché de detalle que eligió el usuario (ninguna: la de por defecto).
    cache_dir: Option<PathBuf>,
    /// Los gigabytes que guarda como máximo la caché de detalle.
    cache_gb: u64,
}

/// Gigabytes que guarda la caché cuando el usuario no ha dicho nada.
pub const DEFAULT_CACHE_GB: u64 = 20;

static PREFS: RwLock<Option<Prefs>> = RwLock::new(None);

fn file() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("HIPSO").join("preferencias.txt"))
}

/// La carpeta de configuración de HIPSO del usuario (donde están las preferencias, el borrador del
/// flujo y los ajustes y conversaciones del asistente).
pub fn config_dir() -> Option<PathBuf> {
    file()?.parent().map(Path::to_path_buf)
}

/// El archivo donde se conserva entre sesiones el flujo que se está construyendo.
pub fn draft() -> Option<PathBuf> {
    let f = file()?;
    Some(f.parent()?.join("flujo_actual.model.json"))
}

fn load() -> Prefs {
    let text = file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .unwrap_or_default();
    let mut p = Prefs {
        adv: Advanced::default(),
        dir: None,
        recent: Vec::new(),
        cache_dir: None,
        cache_gb: DEFAULT_CACHE_GB,
    };
    for line in text.lines() {
        if let Some(d) = line.strip_prefix("carpeta=") {
            p.dir = Some(PathBuf::from(d)).filter(|d| d.is_dir());
        } else if let Some(f) = line.strip_prefix("reciente=") {
            p.recent.push(PathBuf::from(f));
        } else if let Some(d) = line.strip_prefix("cache_carpeta=") {
            p.cache_dir = Some(PathBuf::from(d)).filter(|d| !d.as_os_str().is_empty());
        } else if let Some(g) = line.strip_prefix("cache_gb=") {
            p.cache_gb = g.trim().parse().ok().filter(|g| *g > 0).unwrap_or(DEFAULT_CACHE_GB);
        } else if let Some(d) = line.strip_prefix("rejillas_carpeta=") {
            p.adv.grid_dirs.push(PathBuf::from(d));
        } else if let Some(v) = line.strip_prefix("rejillas_red=") {
            p.adv.grid_network = tri(v);
        } else if let Some(v) = line.strip_prefix("rejillas_enteras=") {
            p.adv.grid_download = tri(v);
        } else if let Some(v) = line.strip_prefix("rejillas_cdn=") {
            p.adv.grid_endpoint = v.trim().to_string();
        } else if let Some(d) = line.strip_prefix("rejillas_cache=") {
            p.adv.grid_cache = Some(PathBuf::from(d)).filter(|d| !d.as_os_str().is_empty());
        } else if let Some(v) = line.strip_prefix("aproximada=") {
            p.adv.allow_ballpark = tri(v);
        } else if let Some(v) = line.strip_prefix("solo_mejor=") {
            p.adv.only_best = tri(v);
        } else if let Some(v) = line.strip_prefix("franjas=") {
            p.adv.stream = tri(v);
        } else if let Some(v) = line.strip_prefix("renderizador=") {
            p.adv.renderer = v.trim().to_string();
        }
    }
    apply_cache(&p);
    apply_advanced(&p.adv);
    p
}

/// `si`/`no` como un valor opcional; cualquier otra cosa es «automático».
fn tri(v: &str) -> Option<bool> {
    match v.trim() {
        "si" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn tri_text(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "si",
        Some(false) => "no",
        None => "auto",
    }
}

fn save(p: &Prefs) {
    let Some(f) = file() else { return };
    let mut text = String::new();
    if let Some(d) = &p.dir {
        text.push_str(&format!("carpeta={}\n", d.display()));
    }
    for r in &p.recent {
        text.push_str(&format!("reciente={}\n", r.display()));
    }
    if let Some(d) = &p.cache_dir {
        text.push_str(&format!("cache_carpeta={}\n", d.display()));
    }
    text.push_str(&format!("cache_gb={}\n", p.cache_gb));
    let a = &p.adv;
    for d in &a.grid_dirs {
        text.push_str(&format!("rejillas_carpeta={}\n", d.display()));
    }
    text.push_str(&format!("rejillas_red={}\n", tri_text(a.grid_network)));
    text.push_str(&format!("rejillas_enteras={}\n", tri_text(a.grid_download)));
    text.push_str(&format!("rejillas_cdn={}\n", a.grid_endpoint));
    if let Some(d) = &a.grid_cache {
        text.push_str(&format!("rejillas_cache={}\n", d.display()));
    }
    text.push_str(&format!("aproximada={}\n", tri_text(a.allow_ballpark)));
    text.push_str(&format!("solo_mejor={}\n", tri_text(a.only_best)));
    text.push_str(&format!("franjas={}\n", tri_text(a.stream)));
    text.push_str(&format!("renderizador={}\n", a.renderer));
    if let Some(parent) = f.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(f, text);
}

/// Entrega los ajustes de la caché a la biblioteca que los usa.
fn apply_cache(p: &Prefs) {
    hipso_core::copc_cache::set_dir(p.cache_dir.clone());
    hipso_core::copc_cache::set_limit(p.cache_gb << 30);
}

/// Entrega los ajustes avanzados a las bibliotecas que los usan.
fn apply_advanced(a: &Advanced) {
    hipso_core::reproject::set_grid_options(hipso_core::reproject::GridOptions {
        extra_dirs: a.grid_dirs.clone(),
        network: a.grid_network,
        download: a.grid_download,
        endpoint: Some(a.grid_endpoint.clone()).filter(|e| !e.is_empty()),
        cache_dir: a.grid_cache.clone(),
    });
    hipso_core::reproject::set_policy_overrides(a.allow_ballpark, a.only_best);
    hipso_core::terrain::set_stream_mode(a.stream);
}

/// Los ajustes avanzados en vigor.
pub fn advanced() -> Advanced {
    with(|p| p.adv.clone()).unwrap_or_default()
}

/// Cambia los ajustes avanzados, los guarda y los aplica ya.
pub fn set_advanced(a: Advanced) {
    let _ = with(|p| {
        p.adv = a;
        apply_advanced(&p.adv);
        save(p);
    });
}

/// Lee ahora las preferencias, para que los ajustes de la caché estén en vigor antes de que nada la
/// use.
pub fn init() {
    let _ = with(|_| ());
}

/// La carpeta de la caché de detalle que eligió el usuario y los gigabytes que guarda como máximo.
pub fn cache() -> (Option<PathBuf>, u64) {
    with(|p| (p.cache_dir.clone(), p.cache_gb)).unwrap_or((None, DEFAULT_CACHE_GB))
}

/// Cambia la carpeta de la caché de detalle (`None`: la de por defecto) y los gigabytes que guarda.
pub fn set_cache(dir: Option<PathBuf>, gb: u64) {
    let _ = with(|p| {
        p.cache_dir = dir;
        p.cache_gb = gb.max(1);
        apply_cache(p);
        save(p);
    });
}

fn with<T>(f: impl FnOnce(&mut Prefs) -> T) -> Option<T> {
    let mut guard = PREFS.write().ok()?;
    let p = guard.get_or_insert_with(load);
    Some(f(p))
}

/// Anota los archivos recién abiertos: su carpeta es la de los siguientes diálogos y pasan al
/// principio de la lista de recientes.
pub fn opened(files: &[PathBuf]) {
    let _ = with(|p| {
        if let Some(d) = files.last().and_then(|f| f.parent()).filter(|d| d.is_dir()) {
            p.dir = Some(d.to_path_buf());
        }
        for f in files.iter().rev() {
            p.recent.retain(|r| r != f);
            p.recent.insert(0, f.clone());
        }
        p.recent.truncate(RECENT);
        save(p);
    });
}

/// Los archivos abiertos más recientemente que todavía existen, los más nuevos primero.
pub fn recent() -> Vec<PathBuf> {
    with(|p| p.recent.iter().filter(|f| f.exists()).cloned().collect()).unwrap_or_default()
}

/// Olvida los archivos recientes.
pub fn clear_recent() {
    let _ = with(|p| {
        p.recent.clear();
        save(p);
    });
}

/// Un diálogo de archivos que empieza en la carpeta de la que se abrieron los últimos archivos.
pub fn dialog() -> rfd::FileDialog {
    let d = rfd::FileDialog::new();
    match with(|p| p.dir.clone()).flatten() {
        Some(dir) => d.set_directory(dir),
        None => d,
    }
}

/// El nombre con que se muestra un archivo reciente: el archivo y su carpeta.
pub fn label(p: &Path) -> String {
    let name = p
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    match p.parent().and_then(|d| d.file_name()) {
        Some(d) => format!("{name}  —  {}", d.to_string_lossy()),
        None => name,
    }
}
