//! COPC from a cloud too big to hold in memory, by cells.
//!
//! The cube of the cloud is cut into the cells of one level of its octree (the level is the
//! shallowest at which no cell holds more points than fit in memory). One pass spreads the
//! records over the cells in a temporary file; then each cell is read, its tree is built as the
//! subtree of the cell, and its chunks are compressed and appended to the output. The nodes
//! above the cells (the coarse levels, an even sample of the whole cloud) take, as the points of
//! each cell go through them, the first point of each of their grid cells; those points are kept
//! in the same temporary file and written last. Memory is about one cell, whatever the size of
//! the cloud; the temporary file takes the size of the cloud as COPC records.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::copc_write::{CopcReport, CubeInfo, GRID, bad, build, copc_format, finish, prepare};
use crate::las::{self, Reader};
use crate::{Error, Result};

/// Cells per side of the histogram that chooses the level of the cells.
const HIST: usize = 32;
/// Deepest level of the cells (`HIST` is 2 to this power).
const HIST_LEVEL: u32 = 5;

/// The records of every cell (and of every coarse node), kept in one temporary file: each
/// bucket collects a block in memory and appends it to the file when it is full.
struct Spool {
    file: File,
    path: PathBuf,
    end: u64,
    block: usize,
    bufs: Vec<Vec<u8>>,
    blocks: Vec<Vec<(u64, u32)>>,
    counts: Vec<u64>,
}

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Spool {
    fn create(path: &Path, buckets: usize) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        // the blocks in memory take at most 256 MB together
        let block = ((256usize << 20) / buckets.max(1)).clamp(4 << 10, 64 << 10);
        Ok(Self {
            file,
            path: path.to_path_buf(),
            end: 0,
            block,
            bufs: vec![Vec::new(); buckets],
            blocks: vec![Vec::new(); buckets],
            counts: vec![0; buckets],
        })
    }

    fn add_bucket(&mut self) -> usize {
        self.bufs.push(Vec::new());
        self.blocks.push(Vec::new());
        self.counts.push(0);
        self.bufs.len() - 1
    }

    fn push(&mut self, b: usize, rec: &[u8]) -> Result<()> {
        self.bufs[b].extend_from_slice(rec);
        self.counts[b] += 1;
        if self.bufs[b].len() >= self.block {
            self.flush(b)?;
        }
        Ok(())
    }

    fn flush(&mut self, b: usize) -> Result<()> {
        if self.bufs[b].is_empty() {
            return Ok(());
        }
        self.file.seek(SeekFrom::Start(self.end))?;
        self.file.write_all(&self.bufs[b])?;
        self.blocks[b].push((self.end, self.bufs[b].len() as u32));
        self.end += self.bufs[b].len() as u64;
        self.bufs[b] = Vec::new();
        Ok(())
    }

    /// All the records of a bucket, in the order they came.
    fn read(&mut self, b: usize) -> Result<Vec<u8>> {
        self.flush(b)?;
        let total: usize = self.blocks[b].iter().map(|x| x.1 as usize).sum();
        let mut out = vec![0u8; total];
        let mut at = 0;
        for &(off, len) in &self.blocks[b] {
            self.file.seek(SeekFrom::Start(off))?;
            self.file.read_exact(&mut out[at..at + len as usize])?;
            at += len as usize;
        }
        Ok(out)
    }
}

/// What one pass over the cloud learns.
struct Survey {
    lo: [f64; 3],
    hi: [f64; 3],
    t0: f64,
    t1: f64,
    returns: [u64; 15],
    hist: Vec<u32>,
    count: u64,
}

impl Survey {
    fn new() -> Self {
        Self {
            lo: [f64::INFINITY; 3],
            hi: [f64::NEG_INFINITY; 3],
            t0: f64::INFINITY,
            t1: f64::NEG_INFINITY,
            returns: [0; 15],
            hist: vec![0; HIST * HIST * HIST],
            count: 0,
        }
    }

    fn merge(mut self, o: Self) -> Self {
        for k in 0..3 {
            self.lo[k] = self.lo[k].min(o.lo[k]);
            self.hi[k] = self.hi[k].max(o.hi[k]);
        }
        self.t0 = self.t0.min(o.t0);
        self.t1 = self.t1.max(o.t1);
        for k in 0..15 {
            self.returns[k] += o.returns[k];
        }
        for (a, b) in self.hist.iter_mut().zip(&o.hist) {
            *a += b;
        }
        self.count += o.count;
        self
    }
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The coordinates of a record (all the point formats start with three 32-bit integers).
fn coords(rec: &[u8], scale: &[f64; 3], offset: &[f64; 3]) -> [f64; 3] {
    [
        f64::from(i32_at(rec, 0)) * scale[0] + offset[0],
        f64::from(i32_at(rec, 4)) * scale[1] + offset[1],
        f64::from(i32_at(rec, 8)) * scale[2] + offset[2],
    ]
}

/// The cell of a coordinate on a grid of `n` cells of side `cell` from `lo`.
fn cell_of(v: f64, lo: f64, cell: f64, n: usize) -> usize {
    (((v - lo) / cell).floor().max(0.0) as usize).min(n - 1)
}

/// A record in the output format, exactly `new_len` bytes.
fn convert<'a>(old: u8, new: u8, new_len: usize, rec: &'a [u8], buf: &'a mut Vec<u8>) -> &'a [u8] {
    if old == new && rec.len() == new_len {
        return rec;
    }
    if old == new {
        buf.clear();
        buf.extend_from_slice(&rec[..new_len.min(rec.len())]);
    } else {
        las::upgrade_record(old, new, rec, buf);
    }
    buf.resize(new_len, 0);
    buf
}

/// Compresses `chunks` (one node each) and returns the compressed bytes of all of them and the
/// size of each.
fn compress(vlr: &laz::LazVlr, chunks: Vec<Vec<u8>>) -> Result<(Vec<u8>, Vec<u64>)> {
    let mut comp =
        laz::ParLasZipCompressor::new(Cursor::new(Vec::<u8>::new()), vlr.clone()).map_err(bad)?;
    comp.compress_chunks(chunks).map_err(bad)?;
    comp.done().map_err(bad)?;
    let buf = comp.into_inner().into_inner();
    let table_at = u64::from_le_bytes(buf[0..8].try_into().map_err(|_| bad("trozo vacío"))?);
    let table_at = usize::try_from(table_at).map_err(|_| bad("tabla fuera de rango"))?;
    // the table reads the pointer at the start of the buffer and goes to it
    let mut cur = Cursor::new(&buf[..]);
    let table = laz::laszip::ChunkTable::read_from(&mut cur, vlr).map_err(bad)?;
    let sizes: Vec<u64> = table.as_ref().iter().map(|e| e.byte_count).collect();
    Ok((buf[8..table_at].to_vec(), sizes))
}

/// The free space of the disk that holds `path`, when it can be told.
fn free_bytes(path: &Path) -> Option<u64> {
    if cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let kb: u64 = text
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;
    Some(kb * 1024)
}

/// A coarse node: which cells of its grid have a point already, and where its points go.
struct Coarse {
    taken: Vec<u64>,
    bucket: usize,
}

/// Writes `input` as COPC at `output` by cells of at most about `cell_max` points.
pub fn write(input: &Path, output: &Path, cell_max: usize) -> Result<CopcReport> {
    let r = Reader::open(input)?;
    let h = r.header.clone();
    if h.count == 0 {
        return Err(bad("la nube no tiene puntos"));
    }
    let old = h.format;
    let new = copc_format(old)?;
    let old_core = las::core_len(old).unwrap_or(0);
    let extra = h.record_len.saturating_sub(old_core);
    let new_len = usize::from(las::core_len(new).unwrap_or(30) + extra);
    let rec_len = usize::from(h.record_len);
    crate::paths::ensure_parent(output)?;
    // the disk must hold the records of the cells while they wait, and then the output
    if let Some(free) = free_bytes(output.parent().unwrap_or(Path::new(".")))
        && free < h.count * new_len as u64 * 13 / 10
    {
        return Err(Error::Param(format!(
            "no hay disco libre para convertir {}: hacen falta unos {} GB temporales junto a la copia",
            input.display(),
            (h.count * new_len as u64 * 13 / 10) >> 30
        )));
    }

    // the cube: the extent of the header (it only has to hold the cloud; it is tightened below)
    let b = h.bounds;
    let center = [
        (b[0] + b[3]) / 2.0,
        (b[1] + b[4]) / 2.0,
        (b[2] + b[5]) / 2.0,
    ];
    let half = ((b[3] - b[0]).max(b[4] - b[1]).max(b[5] - b[2]) / 2.0).max(0.5) * 1.0001;
    let cube_lo = [center[0] - half, center[1] - half, center[2] - half];
    let size = 2.0 * half;

    // pass 1: the extent, the times, the returns, and how many points each cell of a fine grid has
    let hcell = size / HIST as f64;
    let sv = {
        let mut total = Survey::new();
        r.scan_raw(|bytes| {
            let part = bytes
                .par_chunks(rec_len * 16_384)
                .fold(Survey::new, |mut s, block| {
                    for rec in block.chunks_exact(rec_len) {
                        let p = coords(rec, &h.scale, &h.offset);
                        for (k, v) in p.iter().enumerate() {
                            s.lo[k] = s.lo[k].min(*v);
                            s.hi[k] = s.hi[k].max(*v);
                        }
                        let t = r.decode_extra(rec).gps_time;
                        if t.is_finite() {
                            s.t0 = s.t0.min(t);
                            s.t1 = s.t1.max(t);
                        }
                        let rn = if old >= 6 { rec[14] & 15 } else { rec[14] & 7 };
                        s.returns[usize::from(rn.clamp(1, 15)) - 1] += 1;
                        let (cx, cy, cz) = (
                            cell_of(p[0], cube_lo[0], hcell, HIST),
                            cell_of(p[1], cube_lo[1], hcell, HIST),
                            cell_of(p[2], cube_lo[2], hcell, HIST),
                        );
                        s.hist[(cz * HIST + cy) * HIST + cx] += 1;
                        s.count += 1;
                    }
                    s
                })
                .reduce(Survey::new, Survey::merge);
            total = std::mem::replace(&mut total, Survey::new()).merge(part);
            crate::check_cancel()
        })?;
        total
    };
    if sv.count == 0 {
        return Err(bad("ningún punto se ha podido leer"));
    }

    // the shallowest level whose biggest cell fits
    let biggest = |level: u32| -> usize {
        let n = 1usize << level;
        let step = HIST / n;
        let mut cells = vec![0u64; n * n * n];
        for z in 0..HIST {
            for y in 0..HIST {
                for x in 0..HIST {
                    let c = ((z / step) * n + y / step) * n + x / step;
                    cells[c] += u64::from(sv.hist[(z * HIST + y) * HIST + x]);
                }
            }
        }
        cells.into_iter().max().unwrap_or(0) as usize
    };
    let level = (1..=HIST_LEVEL)
        .find(|&l| biggest(l) <= cell_max)
        .unwrap_or(HIST_LEVEL);
    if biggest(level) > cell_max * 2 {
        return Err(Error::Param(format!(
            "{}: la nube es tan densa en un punto que una celda de {} puntos no cabe en memoria",
            input.display(),
            biggest(level)
        )));
    }
    let n_side = 1usize << level;
    let ncell = n_side * n_side * n_side;
    let cell_side = size / n_side as f64;

    // pass 2: the records, converted, to the file of the cells
    let spool_path = output.with_extension("spool");
    let mut spool = Spool::create(&spool_path, ncell)?;
    let mut buf = Vec::with_capacity(new_len);
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let p = coords(rec, &h.scale, &h.offset);
            let c = (cell_of(p[2], cube_lo[2], cell_side, n_side) * n_side
                + cell_of(p[1], cube_lo[1], cell_side, n_side))
                * n_side
                + cell_of(p[0], cube_lo[0], cell_side, n_side);
            spool.push(c, convert(old, new, new_len, rec, &mut buf))?;
        }
        crate::check_cancel()
    })?;

    // the headers, with the extent the points really have
    let prep = prepare(&r, new, extra, new_len, sv.count, sv.lo, sv.hi, &sv.returns)?;
    let out_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)?;
    let mut out = BufWriter::with_capacity(1 << 20, out_file);
    out.write_all(&prep.head)?;
    out.write_all(&prep.vlrs)?;
    out.write_all(&[0u8; 8])?; // where the chunk table will be
    let mut entries: Vec<([i32; 4], u64, i32)> = Vec::new();
    let mut table = laz::laszip::ChunkTable::default();
    let mut coarse: HashMap<[i32; 4], Coarse> = HashMap::new();
    let grid = GRID as usize;
    let bits_len = grid * grid * grid / 64;
    let scale = h.scale;
    let offset = h.offset;

    // each cell: take what the coarse nodes above it keep, build the subtree, write its chunks
    for c in 0..ncell {
        if spool.counts[c] == 0 {
            continue;
        }
        crate::check_cancel()?;
        let (cx, cy, cz) = (c % n_side, (c / n_side) % n_side, c / (n_side * n_side));
        let recs = spool.read(c)?;
        spool.bufs[c] = Vec::new();
        let n = recs.len() / new_len;
        let pos: Vec<[f32; 3]> = (0..n)
            .into_par_iter()
            .map(|i| {
                let p = coords(&recs[i * new_len..], &scale, &offset);
                [
                    (p[0] - cube_lo[0]) as f32,
                    (p[1] - cube_lo[1]) as f32,
                    (p[2] - cube_lo[2]) as f32,
                ]
            })
            .collect();
        // the coarse nodes above this cell, from the root
        let keys: Vec<[i32; 4]> = (0..level as i32)
            .map(|l| {
                let s = (level as i32 - l) as u32;
                [l, (cx >> s) as i32, (cy >> s) as i32, (cz >> s) as i32]
            })
            .collect();
        let mut mine: Vec<Coarse> = keys
            .iter()
            .map(|k| {
                coarse.remove(k).unwrap_or_else(|| Coarse {
                    taken: vec![0; bits_len],
                    bucket: spool.add_bucket(),
                })
            })
            .collect();
        let mut remaining: Vec<u32> = Vec::with_capacity(n);
        'points: for (i, p) in pos.iter().enumerate() {
            for (k, anc) in keys.iter().zip(mine.iter_mut()) {
                let side = size / f64::from(1u32 << k[0]);
                let cell = side / f64::from(GRID);
                let at = |v: f32, o: i32| cell_of(f64::from(v), f64::from(o) * side, cell, grid);
                let (gx, gy, gz) = (at(p[0], k[1]), at(p[1], k[2]), at(p[2], k[3]));
                let g = (gz * grid + gy) * grid + gx;
                if anc.taken[g / 64] & (1 << (g % 64)) == 0 {
                    anc.taken[g / 64] |= 1 << (g % 64);
                    spool.push(anc.bucket, &recs[i * new_len..(i + 1) * new_len])?;
                    continue 'points;
                }
            }
            remaining.push(i as u32);
        }
        for (k, anc) in keys.iter().zip(mine) {
            coarse.insert(*k, anc);
        }
        let mut nodes = build(
            [level as i32, cx as i32, cy as i32, cz as i32],
            remaining,
            &pos,
            size,
        );
        drop(pos);
        nodes.retain(|nd| !nd.points.is_empty());
        nodes.sort_by_key(|nd| nd.key);
        let chunks: Vec<Vec<u8>> = nodes
            .par_iter()
            .map(|nd| {
                let mut ch = Vec::with_capacity(nd.points.len() * new_len);
                for &i in &nd.points {
                    let at = i as usize * new_len;
                    ch.extend_from_slice(&recs[at..at + new_len]);
                }
                ch
            })
            .collect();
        drop(recs);
        let (bytes, sizes) = compress(&prep.vlr, chunks)?;
        out.write_all(&bytes)?;
        for (nd, sz) in nodes.iter().zip(&sizes) {
            entries.push((nd.key, *sz, nd.points.len() as i32));
            table.push(laz::laszip::ChunkTableEntry {
                point_count: nd.points.len() as u64,
                byte_count: *sz,
            });
        }
    }

    // the coarse nodes last, in groups that fit in memory
    let mut keys: Vec<[i32; 4]> = coarse.keys().copied().collect();
    keys.sort();
    let mut i = 0;
    while i < keys.len() {
        let mut group: Vec<([i32; 4], Vec<u8>)> = Vec::new();
        let mut pts = 0usize;
        while i < keys.len() && (group.is_empty() || pts < cell_max) {
            let k = keys[i];
            let recs = spool.read(coarse[&k].bucket)?;
            pts += recs.len() / new_len;
            group.push((k, recs));
            i += 1;
        }
        let counts: Vec<usize> = group.iter().map(|g| g.1.len() / new_len).collect();
        let ks: Vec<[i32; 4]> = group.iter().map(|g| g.0).collect();
        let chunks: Vec<Vec<u8>> = group.into_iter().map(|g| g.1).collect();
        let (bytes, sizes) = compress(&prep.vlr, chunks)?;
        out.write_all(&bytes)?;
        for ((k, n), sz) in ks.iter().zip(&counts).zip(&sizes) {
            entries.push((*k, *sz, *n as i32));
            table.push(laz::laszip::ChunkTableEntry {
                point_count: *n as u64,
                byte_count: *sz,
            });
        }
    }
    drop(spool);

    // the chunk table, where the points end, and the pointer to it
    out.flush()?;
    let mut file = out.into_inner().map_err(|e| bad(e.error()))?;
    let table_at = file.seek(SeekFrom::End(0))?;
    table.write_to(&mut file, &prep.vlr).map_err(bad)?;
    file.seek(SeekFrom::Start(prep.point_offset as u64))?;
    file.write_all(&(table_at as i64).to_le_bytes())?;
    let cube = CubeInfo {
        center,
        half,
        spacing: size / f64::from(GRID),
        t0: sv.t0,
        t1: sv.t1,
    };
    finish(&mut file, &prep, &entries, &cube)?;
    let levels = entries.iter().map(|e| e.0[0]).max().unwrap_or(0) + 1;
    Ok(CopcReport {
        points: sv.count,
        nodes: entries.len(),
        levels,
        format: new,
        wkt: prep.has_wkt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::las::Writer;

    #[test]
    fn a_cloud_written_by_cells_keeps_every_point_and_reads_back_by_area() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("el_copct_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let tpl = crate::ply::template(&dir, 0, 0.001, [0.0, 0.0, 0.0], Some(25830))?;
        let cloud = dir.join("n.las");
        let mut w = Writer::create(&cloud, &tpl)?;
        let mut state = 99u64;
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
        let out = dir.join("t.copc.laz");
        // cells of at most 20 000 points: the cloud is cut into several levels of cells
        let rep = write(&cloud, &out, 20_000)?;
        assert_eq!(rep.points, 300_000);
        assert!(rep.levels >= 3, "{rep:?}");
        assert!(!out.with_extension("spool").exists());
        // it is a COPC whose nodes hold every point once
        let copc = crate::copc::Copc::open(&out)?.ok_or_else(|| bad("no es COPC"))?;
        let all = copc.nodes(None, None)?;
        assert_eq!(
            all.iter().map(|n| u64::from(n.points)).sum::<u64>(),
            300_000
        );
        assert!(all.iter().any(|n| n.level == 0), "the root is there");
        // read as a plain LAZ, the same points as the source, with their classes
        let key = |p: &las::Point| {
            (
                (p.x * 1000.0).round() as i64,
                (p.y * 1000.0).round() as i64,
                (p.z * 1000.0).round() as i64,
                p.class,
            )
        };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        Reader::open(&cloud)?.scan(|p| a.push(key(p)))?;
        Reader::open(&out)?.scan(|p| b.push(key(p)))?;
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        // a query by area reads less than the whole, and finds the points inside
        let window = [10.0, 10.0, 30.0, 30.0];
        let part = copc.nodes(Some(window), None)?;
        assert!(part.iter().map(|n| u64::from(n.points)).sum::<u64>() < 300_000);
        let inside = a
            .iter()
            .filter(|p| p.0 >= 10_000 && p.0 <= 30_000 && p.1 >= 10_000 && p.1 <= 30_000)
            .count() as u64;
        assert_eq!(copc.scan_in(window, None, |_| {})?, inside);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
