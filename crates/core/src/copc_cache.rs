//! A COPC copy of every plain LAS/LAZ the viewer opens, kept in the user's cache folder, so that
//! zooming into a cloud can read the detail of the window from its octree instead of showing
//! only the sample the viewer holds. The original is never touched; the copy is made once and
//! found again by the path, size and date of the original.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::Result;

/// The folder of the copies.
fn cache_dir() -> PathBuf {
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
        .join("EasyLiDAR")
        .join("copc")
}

/// Where the COPC copy of `src` goes (whether or not it exists yet).
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

/// The COPC to read windows of `src` from: `src` itself when it is COPC, else the copy in the
/// cache when it exists. `None` when there is none yet.
pub fn find(src: &Path) -> Option<PathBuf> {
    if crate::copc::Copc::open(src).ok().flatten().is_some() {
        return Some(src.to_path_buf());
    }
    let p = path_for(src);
    if p.is_file() && crate::copc::Copc::open(&p).ok().flatten().is_some() {
        // used now: the last to be removed when the cache is pruned
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&p) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Some(p);
    }
    None
}

/// Most bytes the cache keeps before [`prune`] removes the copies not used for longest.
pub const MAX_BYTES: u64 = 20 << 30;

/// Removes the copies used least recently until the cache holds at most `max_bytes`, never those
/// in `keep`, and the half-written ones left by an interrupted writing. Returns the bytes freed.
pub fn prune(max_bytes: u64, keep: &[PathBuf]) -> u64 {
    let Ok(dir) = std::fs::read_dir(cache_dir()) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut files = Vec::new();
    let mut freed = 0u64;
    for e in dir.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let path = e.path();
        let used = m.modified().unwrap_or(now);
        if path.extension().is_some_and(|x| x == "part") {
            // a writing in progress is young; an old one was interrupted
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

/// The memory of the machine in bytes, when it can be told.
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

/// Bytes of memory the writing of a COPC holds per point: a cloud that fits is held whole
/// (measured: 70 on 30 M points); a bigger one only positions and the node of each point
/// (measured: 35 to 50 on 50 to 94 M points, batch of records included).
fn bytes_needed(points: u64, mem: u64) -> u64 {
    if points <= 30_000_000 {
        points * 75
    } else {
        points * 30 + mem / 10
    }
}

/// Makes the copy of `src` in the cache unless there is one; returns its path. It is written
/// under another name and renamed when done, so a copy that exists is a whole one.
pub fn ensure(src: &Path) -> Result<PathBuf> {
    if let Some(p) = find(src) {
        return Ok(p);
    }
    // the copy is built in memory: a cloud that would take over half of it is left alone
    if let (Some(mem), Ok(r)) = (total_memory(), crate::las::Reader::open(src))
        && bytes_needed(r.header.count, mem) > mem / 2
    {
        return Err(crate::Error::Param(format!(
            "{} puntos son demasiados para la memoria de este equipo; conviértela antes a COPC por teselas",
            r.header.count
        )));
    }
    let out = path_for(src);
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
