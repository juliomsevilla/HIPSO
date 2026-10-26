//! What the app remembers between sessions: the folder files were last opened from (where every
//! file dialog starts) and the files opened most recently. Kept in a small text file in the
//! user's configuration folder; if it cannot be read or written the app works as before.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// How many recent files are kept.
const RECENT: usize = 10;

struct Prefs {
    dir: Option<PathBuf>,
    recent: Vec<PathBuf>,
}

static PREFS: RwLock<Option<Prefs>> = RwLock::new(None);

fn file() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("EasyLiDAR").join("preferencias.txt"))
}

fn load() -> Prefs {
    let text = file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .unwrap_or_default();
    let mut p = Prefs {
        dir: None,
        recent: Vec::new(),
    };
    for line in text.lines() {
        if let Some(d) = line.strip_prefix("carpeta=") {
            p.dir = Some(PathBuf::from(d)).filter(|d| d.is_dir());
        } else if let Some(f) = line.strip_prefix("reciente=") {
            p.recent.push(PathBuf::from(f));
        }
    }
    p
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
    if let Some(parent) = f.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(f, text);
}

fn with<T>(f: impl FnOnce(&mut Prefs) -> T) -> Option<T> {
    let mut guard = PREFS.write().ok()?;
    let p = guard.get_or_insert_with(load);
    Some(f(p))
}

/// Notes files just opened: their folder is the next dialogs' and they go first in the recent
/// list.
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

/// The files opened most recently that still exist, newest first.
pub fn recent() -> Vec<PathBuf> {
    with(|p| p.recent.iter().filter(|f| f.exists()).cloned().collect()).unwrap_or_default()
}

/// Forgets the recent files.
pub fn clear_recent() {
    let _ = with(|p| {
        p.recent.clear();
        save(p);
    });
}

/// A file dialog that starts in the folder files were last opened from.
pub fn dialog() -> rfd::FileDialog {
    let d = rfd::FileDialog::new();
    match with(|p| p.dir.clone()).flatten() {
        Some(dir) => d.set_directory(dir),
        None => d,
    }
}

/// The name a recent file is shown with: the file and its folder.
pub fn label(p: &Path) -> String {
    let name = p
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    match p.parent().and_then(|d| d.file_name()) {
        Some(d) => format!("{name}  —  {}", d.to_string_lossy()),
        None => name,
    }
}
