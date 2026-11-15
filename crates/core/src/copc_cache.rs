//! Una copia COPC de cada LAS/LAZ normal que abre el visor, guardada en la carpeta de caché del
//! usuario, para que al acercar la vista a una nube se pueda leer el detalle de la ventana desde su
//! octree en lugar de mostrar solo la muestra que tiene el visor. El original nunca se toca; la
//! copia se hace una vez y se vuelve a encontrar por la ruta, el tamaño y la fecha del original.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::Result;

static CHOSEN: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);
static LIMIT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(MAX_BYTES);

/// Fija la carpeta de las copias elegida por el usuario (`None`: vuelve a la de por defecto). Las
/// copias ya hechas en otra carpeta no se mueven.
pub fn set_dir(dir: Option<PathBuf>) {
    if let Ok(mut c) = CHOSEN.write() {
        *c = dir;
    }
}

/// Fija el máximo de bytes que guarda la caché (véase [`prune`]).
pub fn set_limit(bytes: u64) {
    LIMIT.store(bytes, std::sync::atomic::Ordering::Relaxed);
}

/// El máximo de bytes que guarda la caché.
pub fn limit() -> u64 {
    LIMIT.load(std::sync::atomic::Ordering::Relaxed)
}

/// La carpeta de las copias cuando el usuario no ha elegido una (`HIPSO_CACHE_DIR` si está
/// definida; si no, la carpeta de caché del sistema).
pub fn default_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("HIPSO_CACHE_DIR") {
        return PathBuf::from(d);
    }
    let home = |sub: &str| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(sub));
    let dir = if cfg!(target_os = "macos") {
        home("Library/Caches")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home(".cache"))
    };
    dir.unwrap_or_else(std::env::temp_dir)
        .join("HIPSO")
        .join("copc")
}

/// La carpeta de las copias ahora: la que eligió el usuario o, si no, la de por defecto.
pub fn cache_dir() -> PathBuf {
    CHOSEN
        .read()
        .ok()
        .and_then(|c| c.clone())
        .unwrap_or_else(default_dir)
}

/// Archivos y bytes que guarda la caché ahora (las copias y las que están a medio escribir).
pub fn size() -> (usize, u64) {
    let Ok(dir) = std::fs::read_dir(cache_dir()) else {
        return (0, 0);
    };
    dir.flatten()
        .filter(|e| is_ours(&e.path()))
        .filter_map(|e| e.metadata().ok())
        .fold((0, 0), |(n, b), m| (n + 1, b + m.len()))
}

/// Si un archivo de la carpeta de la caché es nuestro: la carpeta puede ser una que eligió el
/// usuario y contener otras cosas, que nunca se tocan.
fn is_ours(p: &Path) -> bool {
    let name = p.to_string_lossy();
    name.ends_with(".copc.laz")
        || p.extension()
            .is_some_and(|x| x == "part" || x == "spool")
}

/// Borra todas las copias de la caché salvo las de `keep` (las de las nubes abiertas). Devuelve los
/// archivos y bytes borrados.
pub fn clear(keep: &[PathBuf]) -> (usize, u64) {
    let Ok(dir) = std::fs::read_dir(cache_dir()) else {
        return (0, 0);
    };
    let mut done = (0, 0);
    for e in dir.flatten() {
        let p = e.path();
        if !is_ours(&p) || keep.contains(&p) {
            continue;
        }
        let len = e.metadata().map_or(0, |m| m.len());
        if std::fs::remove_file(&p).is_ok() {
            done.0 += 1;
            done.1 += len;
        }
    }
    done
}

/// Dónde va la copia COPC de `src` (exista ya o no).
pub fn path_for(src: &Path) -> PathBuf {
    let meta = std::fs::metadata(src).ok();
    let mut h = DefaultHasher::new();
    std::fs::canonicalize(src)
        .unwrap_or_else(|_| src.to_path_buf())
        .hash(&mut h);
    if let Some(m) = &meta {
        m.len().hash(&mut h);
        m.modified().ok().hash(&mut h);
    }
    cache_dir().join(format!(
        "{}_{:016x}.copc.laz",
        crate::paths::stem(src),
        h.finish()
    ))
}

/// El COPC del que leer ventanas de `src`: el propio `src` si es COPC o, si no, la copia de la
/// caché cuando existe. `None` si todavía no hay ninguna.
pub fn find(src: &Path) -> Option<PathBuf> {
    if crate::copc::Copc::open(src).ok().flatten().is_some() {
        return Some(src.to_path_buf());
    }
    let p = path_for(src);
    if p.is_file() && crate::copc::Copc::open(&p).ok().flatten().is_some() {
        // se usa ahora: será la última en borrarse cuando se pode la caché
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&p) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Some(p);
    }
    None
}

/// Máximo de bytes que guarda la caché por defecto antes de que [`prune`] borre las copias que
/// llevan más tiempo sin usarse.
pub const MAX_BYTES: u64 = 20 << 30;

/// Borra las copias usadas hace más tiempo hasta que la caché guarda como mucho `max_bytes`, nunca
/// las de `keep`, y las que quedaron a medio escribir por una escritura interrumpida. Devuelve los
/// bytes liberados.
pub fn prune(max_bytes: u64, keep: &[PathBuf]) -> u64 {
    prune_in(&cache_dir(), max_bytes, keep)
}

fn prune_in(folder: &Path, max_bytes: u64, keep: &[PathBuf]) -> u64 {
    let Ok(dir) = std::fs::read_dir(folder) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut files = Vec::new();
    let mut freed = 0u64;
    for e in dir.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let path = e.path();
        let used = m.modified().unwrap_or(now);
        if path
            .extension()
            .is_some_and(|x| x == "part" || x == "spool")
        {
            // una escritura en curso es reciente; una antigua quedó interrumpida
            if now.duration_since(used).is_ok_and(|d| d.as_secs() > 86_400)
                && std::fs::remove_file(&path).is_ok()
            {
                freed += m.len();
            }
        } else if path.to_string_lossy().ends_with(".copc.laz") {
            files.push((used, m.len(), path));
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        if keep.contains(&path) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
            freed += len;
        }
    }
    freed
}

/// La memoria del equipo en bytes, cuando se puede saber.
pub fn total_memory() -> Option<u64> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else {
        let t = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: u64 = t
            .lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))?
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .ok()?;
        Some(kb * 1024)
    }
}

/// Bytes de memoria que retiene la escritura de un COPC: una nube que cabe se retiene entera (70
/// por punto, medido con 30 M de puntos), una mayor guarda solo las posiciones y el nodo de cada
/// punto (de 35 a 50 por punto, medido con 50 a 94 M) y una de más de 100 M de puntos se escribe
/// por celdas, que ocupan una décima parte de la memoria del equipo sea cual sea su tamaño.
fn bytes_needed(points: u64, mem: u64) -> u64 {
    if points <= 30_000_000 {
        points * 75
    } else if points <= 100_000_000 {
        points * 30 + mem / 10
    } else {
        mem / 10 * 2
    }
}

/// Bytes libres en el volumen que contiene `folder` (o la carpeta más cercana que exista), cuando
/// se pueden saber.
pub fn free_bytes(folder: &Path) -> Option<u64> {
    let mut at = folder;
    while !at.exists() {
        at = at.parent()?;
    }
    let out = std::process::Command::new("df")
        .arg("-k")
        .arg(at)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // la línea del volumen: sistema de archivos, bloques de 1K, usados, disponibles, ...
    let kb: u64 = text.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

/// Espacio libre que se deja intacto al hacer copias: el equipo tiene que seguir funcionando.
const KEEP_FREE: u64 = 8 << 30;

/// Hace la copia de `src` en la caché si no existe ya; devuelve su ruta. Se escribe con otro nombre
/// y se renombra al terminar, de modo que una copia que existe es una copia completa.
pub fn ensure(src: &Path) -> Result<PathBuf> {
    if let Some(p) = find(src) {
        return Ok(p);
    }
    // la copia se construye en memoria: una nube que ocuparía más de la mitad de ella se deja sin
    // copia
    if let (Some(mem), Ok(r)) = (total_memory(), crate::las::Reader::open(src))
        && bytes_needed(r.header.count, mem) > mem / 2
    {
        return Err(crate::Error::Param(format!(
            "{} puntos son demasiados para la memoria de este equipo; conviértela antes a COPC por teselas",
            r.header.count
        )));
    }
    let out = path_for(src);
    // una copia ocupa unas 1,3 veces el tamaño de la nube: no llenar nunca el disco con la caché
    let need = (std::fs::metadata(src).map_or(0, |m| m.len()) as f64 * 1.4) as u64;
    if let Some(free) = out.parent().and_then(free_bytes)
        && free < need + KEEP_FREE
    {
        return Err(crate::Error::Param(format!(
            "no hay espacio en el disco de la caché para la copia de detalle ({:.1} GB libres, hacen falta {:.1} GB y se dejan {} GB libres): libera espacio o elige otra carpeta para la caché en Ajustes",
            free as f64 / 1e9,
            need as f64 / 1e9,
            KEEP_FREE >> 30
        )));
    }
    let part = out.with_extension("part");
    let r = crate::copc_write::write(src, &part);
    match r {
        Ok(_) => {
            std::fs::rename(&part, &out)?;
            Ok(out)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pruning_removes_the_least_recently_used_and_spares_the_kept() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("el_cache_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let now = std::time::SystemTime::now();
        let ago = |s: u64| now - std::time::Duration::from_secs(s);
        let make = |name: &str, bytes: usize, age: u64| -> std::io::Result<PathBuf> {
            let p = dir.join(name);
            std::fs::write(&p, vec![0u8; bytes])?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&p)?
                .set_modified(ago(age))?;
            Ok(p)
        };
        let old = make("a.copc.laz", 100, 3000)?;
        let kept = make("b.copc.laz", 100, 2000)?;
        let mid = make("c.copc.laz", 100, 1000)?;
        let new = make("d.copc.laz", 100, 10)?;
        let stale = make("e.part", 50, 90_000)?;
        let young = make("f.part", 50, 10)?;
        let freed = prune_in(&dir, 250, std::slice::from_ref(&kept));
        // se borra la más antigua; la conservada se respeta aunque sea la siguiente más antigua;
        // después se borra la intermedia
        assert!(!old.exists() && kept.exists() && !mid.exists() && new.exists());
        assert!(!stale.exists() && young.exists());
        assert_eq!(freed, 100 + 100 + 50);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn clearing_removes_only_the_copies_and_spares_the_kept_and_foreign_files() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("el_cache_clear_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        for (n, len) in [("a.copc.laz", 100), ("b.copc.laz", 100), ("c.part", 40), ("notas.txt", 7)] {
            std::fs::write(dir.join(n), vec![0u8; len])?;
        }
        set_dir(Some(dir.clone()));
        assert_eq!(size(), (3, 240));
        let freed = clear(&[dir.join("b.copc.laz")]);
        set_dir(None);
        assert_eq!(freed, (2, 140));
        assert!(dir.join("b.copc.laz").exists() && dir.join("notas.txt").exists());
        assert!(!dir.join("a.copc.laz").exists() && !dir.join("c.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
