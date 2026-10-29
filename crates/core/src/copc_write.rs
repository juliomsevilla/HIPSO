//! Writes a cloud as a Cloud Optimized Point Cloud (COPC 1.0): a LAZ 1.4 file whose compressed
//! chunks are the nodes of an octree, so that a viewer or a server reads only the part and the
//! detail it needs. It stays a valid LAZ for every program that reads LAZ.
//!
//! The octree: a cube around the cloud; each node keeps one point per cell of a 128 x 128 x 128
//! grid over its cube (the first that falls in the cell), and passes the rest to its eight
//! children, until a node holds few enough points to keep them all. Coarse levels are then an
//! even sample of the whole cloud, finer levels add detail.
//!
//! The layout follows the specification: the `copc` record first among the headers, then
//! `laszip` (variable chunks: one per node), the coordinate system as WKT, the other records of
//! the input; the chunks; and the hierarchy (one page with every node) as an extended record at
//! the end. The cloud is held in memory while the tree is built (about 40 bytes per point).

use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use rayon::prelude::*;

use crate::las::{self, Reader};
use crate::{Error, Result};

/// Cells per side of a node's sampling grid.
const GRID: u32 = 128;
/// A node with at most this many points keeps them all.
const LEAF: usize = 100_000;
/// Deepest level of the tree.
const MAX_LEVEL: i32 = 20;
/// Fewest points of the nodes read together when the records are read again for each batch.
const STREAM_BATCH: usize = 12_000_000;
/// Points of the nodes compressed together.
const BATCH_POINTS: usize = 4_000_000;
/// Clouds up to this many points are held whole in memory while they are written.
const HOLD_MAX: u64 = 30_000_000;
/// Most points written to one COPC (the cloud is held in memory).
const MAX_POINTS: u64 = 400_000_000;

/// A node of the tree and the points it keeps (indices into the cloud).
struct Node {
    key: [i32; 4],
    points: Vec<u32>,
}

/// What a writing did.
#[derive(Clone, Debug, PartialEq)]
pub struct CopcReport {
    pub points: u64,
    pub nodes: usize,
    pub levels: i32,
    pub format: u8,
    pub wkt: bool,
}

fn bad(what: impl std::fmt::Display) -> Error {
    Error::Format(format!("COPC: {what}"))
}

/// The point format a cloud is written in: COPC admits only 6, 7 and 8.
fn copc_format(format: u8) -> Result<u8> {
    match format {
        6..=8 => Ok(format),
        0 | 1 => Ok(6),
        2 | 3 => Ok(7),
        f => Err(Error::Unsupported(format!(
            "formato de punto {f}: COPC solo admite los formatos 6, 7 y 8, y este (con forma de onda) no se convierte"
        ))),
    }
}

/// Splits `idx` (points inside the cube of `key`) into the points the node keeps and the
/// nodes below it. Positions are relative to the corner of the root cube, of side `size`.
fn build(key: [i32; 4], idx: Vec<u32>, pos: &[[f32; 3]], size: f64) -> Vec<Node> {
    let [level, kx, ky, kz] = key;
    if idx.len() <= LEAF || level >= MAX_LEVEL {
        return vec![Node { key, points: idx }];
    }
    let side = size / f64::from(1u32 << level);
    let lo = [
        f64::from(kx) * side,
        f64::from(ky) * side,
        f64::from(kz) * side,
    ];
    let cell = side / f64::from(GRID);
    let at = |v: f32, o: f64| {
        ((f64::from(v) - o) / cell)
            .floor()
            .clamp(0.0, f64::from(GRID - 1)) as u32
    };
    let mut taken = vec![false; (GRID * GRID * GRID) as usize];
    let mut keep = Vec::new();
    let mut child: [Vec<u32>; 8] = Default::default();
    for i in idx {
        let p = pos[i as usize];
        let (cx, cy, cz) = (at(p[0], lo[0]), at(p[1], lo[1]), at(p[2], lo[2]));
        let c = ((cz * GRID + cy) * GRID + cx) as usize;
        if !taken[c] {
            taken[c] = true;
            keep.push(i);
        } else {
            let o = usize::from(cx >= GRID / 2)
                | usize::from(cy >= GRID / 2) << 1
                | usize::from(cz >= GRID / 2) << 2;
            child[o].push(i);
        }
    }
    drop(taken);
    let mut out = vec![Node { key, points: keep }];
    let below: Vec<Vec<Node>> = child
        .into_par_iter()
        .enumerate()
        .filter(|(_, v)| !v.is_empty())
        .map(|(o, v)| {
            let k = [
                level + 1,
                2 * kx + (o & 1) as i32,
                2 * ky + ((o >> 1) & 1) as i32,
                2 * kz + ((o >> 2) & 1) as i32,
            ];
            build(k, v, pos, size)
        })
        .collect();
    out.extend(below.into_iter().flatten());
    out
}

fn vlr_header(user: &str, record: u16, len: usize, description: &str) -> Result<Vec<u8>> {
    let mut h = vec![0u8; 54];
    h[2..2 + user.len().min(16)].copy_from_slice(&user.as_bytes()[..user.len().min(16)]);
    h[18..20].copy_from_slice(&record.to_le_bytes());
    let len = u16::try_from(len).map_err(|_| bad("un registro de cabecera es demasiado largo"))?;
    h[20..22].copy_from_slice(&len.to_le_bytes());
    let d = description.as_bytes();
    h[22..22 + d.len().min(32)].copy_from_slice(&d[..d.len().min(32)]);
    Ok(h)
}

/// Writes `input` as a COPC file at `output`.
pub fn write(input: &Path, output: &Path) -> Result<CopcReport> {
    write_with(input, output, HOLD_MAX)
}

/// [`write`] holding the records in memory only for clouds of up to `hold_max` points.
pub fn write_with(input: &Path, output: &Path, hold_max: u64) -> Result<CopcReport> {
    let r = Reader::open(input)?;
    let h = r.header.clone();
    if h.count == 0 {
        return Err(bad("la nube no tiene puntos"));
    }
    if h.count > MAX_POINTS {
        return Err(Error::Param(format!(
            "{} tiene {} puntos: para COPC la nube se guarda entera en memoria; divídela antes en teselas (easylidar tile)",
            input.display(),
            h.count
        )));
    }
    let old = h.format;
    let new = copc_format(old)?;
    let old_core = las::core_len(old).unwrap_or(0);
    let extra = h.record_len.saturating_sub(old_core);
    let new_len = usize::from(las::core_len(new).unwrap_or(30) + extra);
    // the records in the new format, and positions relative to the corner of the cloud
    let n = usize::try_from(h.count).map_err(|_| bad("demasiados puntos"))?;
    // A cloud that fits is held whole as records; a big one is read once for the positions that
    // build the tree and again, a batch of nodes at a time, for the records of each chunk: about
    // 20 bytes a point in memory instead of 70.
    let hold = h.count <= hold_max;
    let mut skipped = false;
    let mut recs: Vec<u8> = Vec::with_capacity(if hold { n * new_len } else { 0 });
    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(n);
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let (mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut returns = [0u64; 15];
    let rec_len = usize::from(h.record_len);
    let mut buf = Vec::with_capacity(new_len);
    // a first look at the extent, so positions can be relative and small
    let b = h.bounds;
    let origin = [b[0], b[1], b[2]];
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let Some(p) = r.decode(rec) else {
                skipped = true;
                continue;
            };
            if hold {
                if old == new {
                    buf.clear();
                    buf.extend_from_slice(rec);
                } else {
                    las::upgrade_record(old, new, rec, &mut buf);
                }
                recs.extend_from_slice(&buf[..new_len.min(buf.len())]);
                if buf.len() < new_len {
                    recs.extend(std::iter::repeat_n(0u8, new_len - buf.len()));
                }
            }
            pos.push([
                (p.x - origin[0]) as f32,
                (p.y - origin[1]) as f32,
                (p.z - origin[2]) as f32,
            ]);
            for (k, v) in [p.x, p.y, p.z].into_iter().enumerate() {
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
            let t = r.decode_extra(rec).gps_time;
            if t.is_finite() {
                t0 = t0.min(t);
                t1 = t1.max(t);
            }
            let k = usize::from(p.return_number.clamp(1, 15)) - 1;
            returns[k] += 1;
        }
        Ok(())
    })?;
    let count = pos.len();
    if count == 0 {
        return Err(bad("ningún punto se ha podido leer"));
    }
    if !hold && skipped {
        return Err(bad(
            "la nube tiene registros que no se pueden leer y es demasiado grande para convertirla así",
        ));
    }
    // the cube: centred on the cloud, as wide as its widest side
    let center = [
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    ];
    let half = ((hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]) / 2.0).max(0.5) * 1.0001;
    let cube_lo = [center[0] - half, center[1] - half, center[2] - half];
    let size = 2.0 * half;
    let spacing = size / f64::from(GRID);
    // positions relative to the cube's corner, which the tree works with
    let shift = [
        (origin[0] - cube_lo[0]) as f32,
        (origin[1] - cube_lo[1]) as f32,
        (origin[2] - cube_lo[2]) as f32,
    ];
    pos.par_iter_mut().for_each(|p| {
        p[0] += shift[0];
        p[1] += shift[1];
        p[2] += shift[2];
    });
    crate::check_cancel()?;
    let all: Vec<u32> = (0..count as u32).collect();
    let mut nodes = build([0, 0, 0, 0], all, &pos, size);
    drop(pos);
    // level by level, the root first
    nodes.sort_by_key(|n| n.key);
    let counts: Vec<usize> = nodes.iter().map(|n| n.points.len()).collect();
    // a big cloud keeps, for each point, the node it goes to; the lists of points are let go
    let mut owner: Vec<u32> = Vec::new();
    if !hold {
        owner = vec![u32::MAX; count];
        for (ni, nd) in nodes.iter_mut().enumerate() {
            for &i in &nd.points {
                owner[i as usize] = ni as u32;
            }
            nd.points = Vec::new();
        }
    }
    let levels = nodes.iter().map(|n| n.key[0]).max().unwrap_or(0) + 1;

    // the headers
    let items = laz::LazItemRecordBuilder::default_for_point_format_id(new, extra)
        .map_err(|e| Error::Unsupported(format!("LAZ: {e}")))?;
    let vlr = laz::LazVlrBuilder::default()
        .with_laz_items(items)
        .with_variable_chunk_size()
        .build();
    let mut laz_payload = Vec::new();
    vlr.write_to(&mut laz_payload)?;
    let wkt = r.meta.wkt.clone().or_else(|| {
        h.epsg.and_then(|code| {
            rustlidar_crs::SpatialRef::from_epsg(u32::from(code))
                .ok()
                .and_then(|c| c.to_wkt())
        })
    });
    let (others, n_others) = r.other_records();
    let mut vlrs = Vec::new();
    vlrs.extend(vlr_header("copc", 1, 160, "COPC info")?);
    let info_at = 375 + vlrs.len();
    vlrs.extend(vec![0u8; 160]);
    vlrs.extend(vlr_header(
        "laszip encoded",
        22204,
        laz_payload.len(),
        "EasyLiDAR",
    )?);
    vlrs.extend(&laz_payload);
    let mut n_vlrs = 2 + n_others;
    if let Some(w) = &wkt {
        let mut data = w.as_bytes().to_vec();
        data.push(0);
        vlrs.extend(vlr_header("LASF_Projection", 2112, data.len(), "OGC WKT")?);
        vlrs.extend(data);
        n_vlrs += 1;
    }
    vlrs.extend(&others);
    let point_offset = 375 + vlrs.len();

    let src = r.raw_header();
    let mut head = vec![0u8; 375];
    head[..4].copy_from_slice(b"LASF");
    // file source id, global encoding, GUID; system identifier; creation day and year
    head[4..24].copy_from_slice(src.get(4..24).unwrap_or(&[0; 20]));
    head[26..58].copy_from_slice(src.get(26..58).unwrap_or(&[0; 32]));
    head[90..94].copy_from_slice(src.get(90..94).unwrap_or(&[0; 4]));
    let enc = u16::from_le_bytes([head[6], head[7]]) | 0x10;
    head[6..8].copy_from_slice(&enc.to_le_bytes());
    head[24] = 1;
    head[25] = 4;
    let soft = b"EasyLiDAR";
    head[58..58 + soft.len()].copy_from_slice(soft);
    head[94..96].copy_from_slice(&375u16.to_le_bytes());
    head[96..100].copy_from_slice(&(point_offset as u32).to_le_bytes());
    head[100..104].copy_from_slice(&n_vlrs.to_le_bytes());
    head[104] = new | 0x80;
    head[105..107].copy_from_slice(&(new_len as u16).to_le_bytes());
    for k in 0..3 {
        head[131 + 8 * k..139 + 8 * k].copy_from_slice(&h.scale[k].to_le_bytes());
        head[155 + 8 * k..163 + 8 * k].copy_from_slice(&h.offset[k].to_le_bytes());
    }
    for (at, v) in [
        (179, hi[0]),
        (187, lo[0]),
        (195, hi[1]),
        (203, lo[1]),
        (211, hi[2]),
        (219, lo[2]),
    ] {
        head[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    head[247..255].copy_from_slice(&(count as u64).to_le_bytes());
    for (k, c) in returns.iter().enumerate() {
        head[255 + 8 * k..263 + 8 * k].copy_from_slice(&c.to_le_bytes());
    }

    // the chunks, one per node, compressed in parallel
    crate::paths::ensure_parent(output)?;
    // read back as well: the chunk table the compressor writes says where each node went
    let out_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)?;
    let mut f = BufWriter::with_capacity(1 << 20, out_file);
    f.write_all(&head)?;
    f.write_all(&vlrs)?;
    let mut comp = laz::ParLasZipCompressor::new(f, vlr.clone()).map_err(bad)?;
    // in batches, so that the chunks never exist all at once next to the records
    // (the records of a batch are held while it is read: a tenth of the memory of the machine)
    let batch = if hold {
        BATCH_POINTS
    } else {
        let mem = crate::copc_cache::total_memory().unwrap_or(8 << 30);
        ((mem / 10 / new_len as u64) as usize).clamp(STREAM_BATCH, 60_000_000)
    };
    let mut from = 0;
    while from < nodes.len() {
        let mut to = from;
        let mut pts = 0usize;
        while to < nodes.len() && (to == from || pts + counts[to] <= batch) {
            pts += counts[to];
            to += 1;
        }
        let chunks: Vec<Vec<u8>> = if hold {
            nodes[from..to]
                .par_iter()
                .map(|nd| {
                    let mut c = Vec::with_capacity(nd.points.len() * new_len);
                    for &i in &nd.points {
                        let at = i as usize * new_len;
                        c.extend_from_slice(&recs[at..at + new_len]);
                    }
                    c
                })
                .collect()
        } else {
            // the records of this batch of nodes, in the order of the points of the file (which
            // is the order of the points of each node)
            let mut bufs: Vec<Vec<u8>> = (from..to)
                .map(|ni| Vec::with_capacity(counts[ni] * new_len))
                .collect();
            let mut i = 0usize;
            r.scan_raw(|bytes| {
                for rec in bytes.chunks_exact(rec_len) {
                    let o = owner.get(i).copied().unwrap_or(u32::MAX) as usize;
                    i += 1;
                    if o < from || o >= to {
                        continue;
                    }
                    let b = &mut bufs[o - from];
                    if old == new {
                        b.extend_from_slice(&rec[..new_len.min(rec.len())]);
                    } else {
                        buf.clear();
                        las::upgrade_record(old, new, rec, &mut buf);
                        b.extend_from_slice(&buf[..new_len.min(buf.len())]);
                    }
                    let want = b.len().div_ceil(new_len) * new_len;
                    b.resize(want, 0);
                }
                Ok(())
            })?;
            bufs
        };
        crate::check_cancel()?;
        comp.compress_chunks(chunks).map_err(bad)?;
        from = to;
    }
    drop(recs);
    comp.done().map_err(bad)?;
    let mut f = comp.into_inner();
    f.flush()?;
    let mut file = f.into_inner().map_err(|e| bad(e.error()))?;
    let evlr_at = file.seek(SeekFrom::End(0))?;
    // where each chunk went, from the table the compressor wrote
    file.seek(SeekFrom::Start(point_offset as u64))?;
    let table = laz::laszip::ChunkTable::read_from(&mut file, &vlr).map_err(bad)?;
    if table.len() != nodes.len() {
        return Err(bad("la tabla de trozos no coincide con los nodos"));
    }
    // the hierarchy: one page, the root first
    let mut page = Vec::with_capacity(nodes.len() * 32);
    let mut at = point_offset as u64 + 8;
    for (ni, (nd, e)) in nodes.iter().zip(table.as_ref()).enumerate() {
        for v in nd.key {
            page.extend(v.to_le_bytes());
        }
        page.extend(at.to_le_bytes());
        let bytes = i32::try_from(e.byte_count).map_err(|_| bad("un nodo es demasiado grande"))?;
        page.extend(bytes.to_le_bytes());
        page.extend((counts[ni] as i32).to_le_bytes());
        at += e.byte_count;
    }
    let mut evlr = vec![0u8; 60];
    evlr[2..6].copy_from_slice(b"copc");
    evlr[18..20].copy_from_slice(&1000u16.to_le_bytes());
    evlr[20..28].copy_from_slice(&(page.len() as u64).to_le_bytes());
    evlr[28..37].copy_from_slice(b"EPT hiera");
    file.seek(SeekFrom::Start(evlr_at))?;
    file.write_all(&evlr)?;
    file.write_all(&page)?;
    // the COPC record and the pointer to the extended record
    let mut info = vec![0u8; 160];
    for (k, v) in [center[0], center[1], center[2], half, spacing]
        .iter()
        .enumerate()
    {
        info[8 * k..8 * k + 8].copy_from_slice(&v.to_le_bytes());
    }
    info[40..48].copy_from_slice(&(evlr_at + 60).to_le_bytes());
    info[48..56].copy_from_slice(&(page.len() as u64).to_le_bytes());
    let (g0, g1) = if t0 <= t1 { (t0, t1) } else { (0.0, 0.0) };
    info[56..64].copy_from_slice(&g0.to_le_bytes());
    info[64..72].copy_from_slice(&g1.to_le_bytes());
    file.seek(SeekFrom::Start(info_at as u64))?;
    file.write_all(&info)?;
    file.seek(SeekFrom::Start(235))?;
    file.write_all(&evlr_at.to_le_bytes())?;
    file.write_all(&1u32.to_le_bytes())?;
    file.flush()?;
    Ok(CopcReport {
        points: count as u64,
        nodes: nodes.len(),
        levels,
        format: new,
        wkt: wkt.is_some(),
    })
}

/// A short text for the report of a writing.
pub fn describe(r: &CopcReport, input_format: u8, output: &Path) -> String {
    let mut s = format!(
        "{} puntos en {} nodos y {} niveles (COPC, formato {})",
        r.points, r.nodes, r.levels, r.format
    );
    if r.format != input_format {
        s.push_str(&format!("; la nube era de formato {input_format}"));
    }
    s.push_str(&format!(": {}", output.display()));
    if !r.wkt {
        s.push_str(
            "\nAviso: la nube no declara su sistema de coordenadas y COPC lo pide; asígnalo con `crs` y vuelve a escribirla",
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::las::Writer;

    #[test]
    fn a_cloud_written_as_copc_reads_back_by_area_and_whole() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("el_copcw_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // 300 000 points on a 100 x 100 m square of format 1, with a class each
        let tpl = crate::ply::template(&dir, 0, 0.001, [0.0, 0.0, 0.0], Some(25830))?;
        let cloud = dir.join("n.las");
        let mut w = Writer::create(&cloud, &tpl)?;
        let mut state = 12345u64;
        let mut rnd = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as f64 / f64::from(1u32 << 31)
        };
        for k in 0..300_000u32 {
            let (x, y, z) = (rnd() * 100.0, rnd() * 100.0, rnd() * 20.0);
            let mut rec = vec![0u8; 20];
            rec[0..4].copy_from_slice(&((x / 0.001).round() as i32).to_le_bytes());
            rec[4..8].copy_from_slice(&((y / 0.001).round() as i32).to_le_bytes());
            rec[8..12].copy_from_slice(&((z / 0.001).round() as i32).to_le_bytes());
            rec[12..14].copy_from_slice(&((k % 1000) as u16).to_le_bytes());
            rec[14] = 0x09;
            rec[15] = if x < 50.0 { 2 } else { 5 };
            w.write(&rec)?;
        }
        w.finish()?;
        let out = dir.join("n.copc.laz");
        let rep = write(&cloud, &out)?;
        assert_eq!(rep.points, 300_000);
        assert!(rep.nodes > 1 && rep.levels >= 2, "{rep:?}");
        assert!(rep.wkt);
        // the program's own COPC reader sees the tree
        let copc = crate::copc::Copc::open(&out)?.ok_or_else(|| bad("no es COPC"))?;
        let all = copc.nodes(None, None)?;
        assert_eq!(
            all.iter().map(|n| u64::from(n.points)).sum::<u64>(),
            300_000
        );
        // and, read as a plain LAZ, every point is there with its class
        let r = Reader::open(&out)?;
        assert_eq!(r.header.format, 6);
        let (mut n, mut left) = (0u64, 0u64);
        r.scan(|p| {
            n += 1;
            left += u64::from(p.class == 2 && p.x < 50.0);
        })?;
        assert_eq!(n, 300_000);
        assert!(left > 140_000 && left < 160_000, "{left}");
        // a query by area reads less than the whole
        let part = copc.nodes(Some([0.0, 0.0, 20.0, 20.0]), None)?;
        let read: u64 = part.iter().map(|n| u64::from(n.points)).sum();
        assert!(read < 300_000, "{read}");
        // a cloud too big to hold (here: any) is written by reading the records again, and the
        // file is the same as the one made from the records in memory
        let streamed = dir.join("s.copc.laz");
        let rep2 = write_with(&cloud, &streamed, 0)?;
        assert_eq!((rep2.points, rep2.nodes), (rep.points, rep.nodes));
        assert_eq!(std::fs::read(&out)?, std::fs::read(&streamed)?);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
