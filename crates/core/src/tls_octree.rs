//! Conversión de una nube a un octree que lee la ventana de nubes terrestres (`hipso-tls`): dónde
//! queda en la caché, cómo se lanza el conversor y cómo se sigue su avance.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Un ejecutable que viaja junto al programa en marcha.
pub fn junto_al_programa(nombre: &str) -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let n = if cfg!(windows) {
        format!("{nombre}.exe")
    } else {
        nombre.to_string()
    };
    let p = me.parent()?.join(n);
    p.is_file().then_some(p)
}

/// Dónde queda la conversión de `src` (exista ya o no).
pub fn carpeta(src: &Path) -> PathBuf {
    let copia = crate::copc_cache::path_for(src);
    let nombre = copia
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("nube")
        .trim_end_matches(".copc.laz")
        .to_string();
    // junto a la carpeta de copias COPC y no dentro, para que su limpieza no la toque
    let base = crate::copc_cache::cache_dir();
    base.parent()
        .unwrap_or(&base)
        .join("octree")
        .join(nombre)
}

/// La conversión de `src` si ya está hecha.
pub fn lista(src: &Path) -> Option<PathBuf> {
    let d = carpeta(src);
    d.join(".listo").is_file().then_some(d)
}

/// Convierte `src` (si hace falta) y devuelve la carpeta del octree. `avance` recibe el porcentaje
/// (0 a 100) que va dando el conversor.
pub fn convertir(src: &Path, avance: &dyn Fn(f64)) -> Result<PathBuf, String> {
    let destino = carpeta(src);
    if destino.join(".listo").is_file() {
        return Ok(destino);
    }
    let conversor = junto_al_programa("PotreeConverter").ok_or(
        "Falta PotreeConverter junto al programa: no se puede preparar la nube para la ventana.",
    )?;
    let tmp = destino.with_extension("parcial");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("No se pudo crear la caché: {e}"))?;
    let mut hijo = Command::new(conversor)
        // el conversor deja su registro en la carpeta de trabajo: que sea la de la caché
        .current_dir(&tmp)
        .arg(src)
        .arg("-o")
        .arg(&tmp)
        .args(["--encoding", "BROTLI", "--chunkdir"])
        .arg(tmp.join("trozos"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("No se pudo lanzar PotreeConverter: {e}"))?;
    // el conversor informa cada segundo con «[37.52%, 120.3s], …»
    if let Some(salida) = hijo.stdout.take() {
        use std::io::BufRead;
        for linea in std::io::BufReader::new(salida).lines().map_while(Result::ok) {
            if let Some(p) = linea
                .strip_prefix('[')
                .and_then(|r| r.split_once('%'))
                .and_then(|(n, _)| n.trim().parse::<f64>().ok())
            {
                avance(p.clamp(0.0, 100.0));
            }
        }
    }
    let estado = hijo
        .wait()
        .map_err(|e| format!("PotreeConverter falló: {e}"))?;
    let _ = std::fs::remove_dir_all(tmp.join("trozos"));
    if !estado.success() || !tmp.join("metadata.json").is_file() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("La preparación de la nube no terminó.".into());
    }
    let _ = std::fs::remove_dir_all(&destino);
    std::fs::rename(&tmp, &destino).map_err(|e| format!("No se pudo guardar la conversión: {e}"))?;
    std::fs::write(destino.join(".listo"), b"ok")
        .map_err(|e| format!("No se pudo guardar la conversión: {e}"))?;
    Ok(destino)
}
