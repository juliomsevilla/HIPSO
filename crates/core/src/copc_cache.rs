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
    (p.is_file() && crate::copc::Copc::open(&p).ok().flatten().is_some()).then_some(p)
}

/// The memory of the machine in bytes, when it can be told.
fn total_memory() -> Option<u64> {
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

/// Bytes of memory the writing of a COPC holds per point (measured: 70 on a 51 M point cloud).
const BYTES_PER_POINT: u64 = 75;

/// Makes the copy of `src` in the cache unless there is one; returns its path. It is written
/// under another name and renamed when done, so a copy that exists is a whole one.
pub fn ensure(src: &Path) -> Result<PathBuf> {
    if let Some(p) = find(src) {
        return Ok(p);
    }
    // the copy is built in memory: a cloud that would take over half of it is left alone
    if let (Some(mem), Ok(r)) = (total_memory(), crate::las::Reader::open(src))
        && r.header.count * BYTES_PER_POINT > mem / 2
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
