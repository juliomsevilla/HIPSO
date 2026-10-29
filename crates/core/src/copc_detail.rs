//! The detail of a window of COPC clouds: when the viewer zooms in, the points of the whole
//! cloud it holds (an even sample) are too few for the window, but the COPC files keep every
//! point in an octree whose levels are denser and denser. This reads the levels of the nodes
//! that touch the window, down to the deepest one that fits a budget of points, and no deeper than one point per `px` map units (a pixel).

use rayon::prelude::*;

use crate::copc::{Copc, Node};
use crate::las::Reader;
use crate::view::PointStore;
use crate::{Error, Result};

/// One decoded record: x, y, z, intensity, return byte, class, source id, scan angle, colour.
type Row = (f64, f64, f64, u16, u8, u8, u16, i16, Option<[u8; 3]>);

fn u16_at(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

/// The points inside `window` `[x0, y0, x1, y1]` of COPC clouds, as a store that uses the
/// `origin` and the file order of the store of the same clouds, so that the styles of the
/// viewer work on it unchanged. About `budget` points at most: the levels of the octree are
/// taken from the root down while they fit. `None` when a file is not COPC (a plain LAZ cannot
/// give a window without reading all of it).
pub fn load_detail(
    paths: &[std::path::PathBuf],
    window: [f64; 4],
    budget: usize,
    px: f64,
    origin: [f64; 3],
    files: &[String],
) -> Result<Option<PointStore>> {
    let mut clouds = Vec::new();
    for p in paths {
        match Copc::open(p)? {
            Some(c) => clouds.push(c),
            None => return Ok(None),
        }
    }
    let nodes: Vec<Vec<Node>> = clouds
        .iter()
        .map(|c| c.nodes(Some(window), None))
        .collect::<Result<_>>()?;
    // the deepest level whose nodes (all of them, a node reaches past the window) fit
    let max_level = nodes.iter().flatten().map(|n| n.level).max().unwrap_or(0);
    // the level whose spacing is about a pixel (`px` map units): finer would only overdraw
    let spacing = clouds
        .iter()
        .map(|c| c.spacing())
        .fold(f64::INFINITY, f64::min);
    let wanted = if px > 0.0 && spacing.is_finite() {
        (spacing / px).log2().ceil().max(0.0) as i32
    } else {
        max_level
    };
    let mut level = 0;
    for l in 0..=max_level {
        let pts: u64 = nodes
            .iter()
            .flatten()
            .filter(|n| n.level <= l)
            .map(|n| u64::from(n.points))
            .sum();
        if (pts > budget as u64 * 2 || l > wanted) && l > 0 {
            break;
        }
        level = l;
    }
    let mut s = PointStore {
        origin,
        files: files.to_vec(),
        stride: 1,
        ..PointStore::default()
    };
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let has_rgb = clouds
        .iter()
        .any(|c| matches!(c.header().format, 2 | 3 | 5 | 7 | 8 | 10));
    for (fi, (c, ns)) in clouds.iter().zip(&nodes).enumerate() {
        let sel: Vec<Node> = ns.iter().filter(|n| n.level <= level).copied().collect();
        let r: &Reader = c.reader();
        let h = &r.header;
        let rec_len = usize::from(h.record_len);
        let (src_at, rgb_at) = (
            if h.format >= 6 { 20 } else { 18 },
            match h.format {
                2 => Some(20),
                3 | 5 => Some(28),
                7 | 8 | 10 => Some(30),
                _ => None,
            },
        );
        c.read_raw(&sel, |bytes| {
            // decoding is the slow part: all the cores share it
            let rows: Vec<Vec<Row>> = bytes
                .par_chunks(rec_len * 16_384)
                .map(|block| {
                    block
                        .chunks_exact(rec_len)
                        .filter_map(|rec| {
                            let p = r.decode(rec)?;
                            if p.x < window[0]
                                || p.x > window[2]
                                || p.y < window[1]
                                || p.y > window[3]
                            {
                                return None;
                            }
                            let rgb = rgb_at.map(|a| {
                                let q = |k: usize| (u16_at(rec, a + 2 * k) >> 8) as u8;
                                [q(0), q(1), q(2)]
                            });
                            Some((
                                p.x,
                                p.y,
                                p.z,
                                p.intensity,
                                p.return_number | (p.number_of_returns << 4),
                                p.class,
                                u16_at(rec, src_at),
                                (r.decode_extra(rec).scan_angle * 10.0).round() as i16,
                                rgb,
                            ))
                        })
                        .collect()
                })
                .collect();
            for (x, y, z, i, ret, class, src, scan, rgb) in rows.into_iter().flatten() {
                s.x.push((x - origin[0]) as f32);
                s.y.push((y - origin[1]) as f32);
                s.z.push((z - origin[2]) as f32);
                s.intensity.push(i);
                s.ret.push(ret);
                s.class.push(class);
                s.src.push(src);
                s.scan.push(scan);
                s.file.push(fi.min(255) as u8);
                if has_rgb {
                    s.rgb.push(rgb.unwrap_or([150, 150, 150]));
                }
                for (k, v) in [x, y, z].into_iter().enumerate() {
                    lo[k] = lo[k].min(v);
                    hi[k] = hi[k].max(v);
                }
            }
            Ok(())
        })
        .map_err(|e| Error::Format(format!("COPC: {e}")))?;
    }
    s.bounds = if s.x.is_empty() {
        [0.0; 6]
    } else {
        [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
    };
    s.total = s.x.len() as u64;
    Ok(Some(s))
}
