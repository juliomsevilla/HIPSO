//! Cloud Optimized Point Clouds (COPC): a LAZ 1.4 file whose chunks are the nodes of an octree,
//! with the tree stored in the file. A query by area reads and decompresses only the nodes that
//! touch it; a coarse view reads only the first levels. A COPC file is still a valid LAZ, so
//! everything that reads LAZ reads it whole; this module is for the cases where reading less is
//! the point.
//!
//! Layout used (from the format's specification): a `copc` record (160 bytes: the root cube, the
//! spacing and where the hierarchy is) and the `laszip` record among the headers, and the hierarchy
//! in an extended record at the end. A hierarchy page is a list of 32-byte entries: the node's key
//! (level, x, y, z), where its compressed chunk starts, how many bytes it has and how many points;
//! an entry with -1 points is a link to another page.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::las::{Point, Reader};
use crate::{Error, Result, check_cancel};

/// One node of the octree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Node {
    pub level: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Where its compressed chunk starts in the file, and its length.
    pub offset: u64,
    pub bytes: u32,
    pub points: u32,
}

pub struct Copc {
    path: PathBuf,
    reader: Reader,
    vlr: laz::LazVlr,
    center: [f64; 3],
    halfsize: f64,
    spacing: f64,
    root: (u64, u64),
}

/// Most bytes of a hierarchy page we are willing to read, and of pages followed.
const MAX_PAGE: u64 = 256 << 20;
const MAX_PAGES: usize = 1 << 20;
/// A batch of nodes decompressed together: at most this many points.
const BATCH_POINTS: u64 = 1 << 22;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}
fn f64_at(b: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn bad(what: &str) -> Error {
    Error::Format(format!("COPC: {what}"))
}

impl Copc {
    /// Opens a COPC file; `None` when the file is not one (a plain LAS or LAZ).
    pub fn open(path: &Path) -> Result<Option<Self>> {
        let mut f = File::open(path)?;
        let mut head = vec![0u8; 375];
        let got = f.read(&mut head)?;
        if got < 375 || !head.starts_with(b"LASF") || head[25] < 4 {
            return Ok(None);
        }
        let header_size = u64::from(u16_at(&head, 94).unwrap_or(0));
        let n_vlrs = u32_at(&head, 100).unwrap_or(0);
        let end = u64::from(u32_at(&head, 96).unwrap_or(0));
        // the records among the headers
        let (mut info, mut laszip) = (None, None);
        let mut pos = header_size;
        for _ in 0..n_vlrs.min(1000) {
            let mut h = [0u8; 54];
            f.seek(SeekFrom::Start(pos))?;
            if f.read_exact(&mut h).is_err() {
                break;
            }
            let user = String::from_utf8_lossy(&h[2..18])
                .trim_end_matches('\0')
                .to_string();
            let record = u16_at(&h, 18).unwrap_or(0);
            let len = u64::from(u16_at(&h, 20).unwrap_or(0));
            if pos + 54 + len > end {
                break;
            }
            let wanted =
                (user == "copc" && record == 1) || (user == "laszip encoded" && record == 22204);
            if wanted {
                let mut data = vec![0u8; len as usize];
                f.read_exact(&mut data)?;
                if user == "copc" {
                    info = Some(data);
                } else {
                    laszip = Some(data);
                }
            }
            pos += 54 + len;
        }
        let (Some(info), Some(laszip)) = (info, laszip) else {
            return Ok(None);
        };
        if info.len() < 48 {
            return Err(bad("el registro copc es demasiado corto"));
        }
        // A file made from a COPC one (a classified copy, a clip) may still carry its COPC record
        // although it is a plain LAZ now: the hierarchy the record points to must be there, and
        // start with the root node, or the file is not a COPC.
        let (ro, rs) = (
            u64_at(&info, 40).unwrap_or(0),
            u64_at(&info, 48).unwrap_or(0),
        );
        let file_len = f.metadata()?.len();
        let in_file = ro.checked_add(rs).is_some_and(|e| e <= file_len);
        if rs < 32 || rs % 32 != 0 || !in_file {
            return Ok(None);
        }
        let mut root = [0u8; 32];
        f.seek(SeekFrom::Start(ro))?;
        if f.read_exact(&mut root).is_err() || root[..16].iter().any(|b| *b != 0) {
            return Ok(None);
        }
        let vlr = laz::LazVlr::from_buffer(&laszip).map_err(|e| bad(&format!("laszip: {e}")))?;
        let reader = Reader::open(path)?;
        let (Some(cx), Some(cy), Some(cz), Some(half), Some(spacing), Some(ro), Some(rs)) = (
            f64_at(&info, 0),
            f64_at(&info, 8),
            f64_at(&info, 16),
            f64_at(&info, 24),
            f64_at(&info, 32),
            u64_at(&info, 40),
            u64_at(&info, 48),
        ) else {
            return Err(bad("el registro copc está incompleto"));
        };
        if !(half > 0.0) || !half.is_finite() {
            return Err(bad("la caja raíz no es válida"));
        }
        if vlr.items_size() != u64::from(reader.header.record_len) {
            return Err(bad(
                "el tamaño de registro del laszip no coincide con la cabecera",
            ));
        }
        Ok(Some(Self {
            path: path.to_path_buf(),
            reader,
            vlr,
            center: [cx, cy, cz],
            halfsize: half,
            spacing,
            root: (ro, rs),
        }))
    }

    pub fn header(&self) -> &crate::las::Header {
        &self.reader.header
    }

    /// Distance between points at the root level; it halves at each level.
    pub fn spacing(&self) -> f64 {
        self.spacing
    }

    /// The cube of a node: min x, min y, min z, max x, max y, max z.
    pub fn cube(&self, level: i32, x: i32, y: i32, z: i32) -> [f64; 6] {
        let side = 2.0 * self.halfsize / f64::from(2u32.pow(level.clamp(0, 30) as u32));
        let low = |c: f64, i: i32| c - self.halfsize + f64::from(i) * side;
        let (x0, y0, z0) = (
            low(self.center[0], x),
            low(self.center[1], y),
            low(self.center[2], z),
        );
        [x0, y0, z0, x0 + side, y0 + side, z0 + side]
    }

    /// The nodes with points whose cube touches `bbox` (min x, min y, max x, max y; everything when
    /// `None`) down to `max_level` (all levels when `None`), in order of level.
    pub fn nodes(&self, bbox: Option<[f64; 4]>, max_level: Option<i32>) -> Result<Vec<Node>> {
        let touches = |level: i32, x: i32, y: i32, z: i32| {
            let c = self.cube(level, x, y, z);
            bbox.is_none_or(|b| c[0] <= b[2] && c[3] >= b[0] && c[1] <= b[3] && c[4] >= b[1])
        };
        let mut f = File::open(&self.path)?;
        let mut pages = vec![self.root];
        let mut out = Vec::new();
        let mut seen = 0usize;
        while let Some((offset, size)) = pages.pop() {
            seen += 1;
            if seen > MAX_PAGES || size > MAX_PAGE || size % 32 != 0 {
                return Err(bad("jerarquía no válida"));
            }
            let mut page = vec![0u8; size as usize];
            f.seek(SeekFrom::Start(offset))?;
            f.read_exact(&mut page)?;
            for e in page.as_chunks::<32>().0.iter() {
                let (Some(level), Some(x), Some(y), Some(z), Some(off), Some(bytes), Some(count)) = (
                    u32_at(e, 0).map(|v| v as i32),
                    u32_at(e, 4).map(|v| v as i32),
                    u32_at(e, 8).map(|v| v as i32),
                    u32_at(e, 12).map(|v| v as i32),
                    u64_at(e, 16),
                    u32_at(e, 24),
                    u32_at(e, 28).map(|v| v as i32),
                ) else {
                    continue;
                };
                if level < 0 || max_level.is_some_and(|m| level > m) || !touches(level, x, y, z) {
                    continue;
                }
                if count == -1 {
                    pages.push((off, u64::from(bytes)));
                } else if count > 0 {
                    out.push(Node {
                        level,
                        x,
                        y,
                        z,
                        offset: off,
                        bytes,
                        points: count as u32,
                    });
                }
            }
        }
        out.sort_unstable_by_key(|n| (n.level, n.x, n.y, n.z));
        Ok(out)
    }

    /// The first levels of the tree that hold at most `budget` points together: a uniform sample of
    /// the whole cloud, for a quick view. At least the root level is returned.
    pub fn overview(&self, budget: u64) -> Result<Vec<Node>> {
        let all = self.nodes(None, None)?;
        let mut taken = 0u64;
        let mut out = Vec::new();
        let mut level = 0;
        while let Some(first) = all.iter().position(|n| n.level >= level) {
            let _ = first;
            let layer: Vec<Node> = all.iter().filter(|n| n.level == level).copied().collect();
            if layer.is_empty() {
                break;
            }
            let n: u64 = layer.iter().map(|n| u64::from(n.points)).sum();
            if taken + n > budget && level > 0 {
                break;
            }
            taken += n;
            out.extend(layer);
            level += 1;
        }
        Ok(out)
    }

    /// Hands the raw records of `nodes` to `f`, a batch of nodes at a time, in the order given.
    /// Returns how many points were read.
    pub fn read_raw(&self, nodes: &[Node], mut f: impl FnMut(&[u8]) -> Result<()>) -> Result<u64> {
        let rec = self.vlr.items_size() as usize;
        let mut file = File::open(&self.path)?;
        let mut total = 0u64;
        let mut i = 0;
        while i < nodes.len() {
            check_cancel()?;
            // a batch: nodes in file order so that the reads are mostly sequential
            let mut j = i;
            let mut points = 0u64;
            while j < nodes.len() && (j == i || points + u64::from(nodes[j].points) <= BATCH_POINTS)
            {
                points += u64::from(nodes[j].points);
                j += 1;
            }
            let mut batch: Vec<Node> = nodes[i..j].to_vec();
            batch.sort_unstable_by_key(|n| n.offset);
            let mut compressed: Vec<u8> = Vec::new();
            let mut table = Vec::with_capacity(batch.len());
            for n in &batch {
                let at = compressed.len();
                compressed.resize(at + n.bytes as usize, 0);
                file.seek(SeekFrom::Start(n.offset))?;
                file.read_exact(&mut compressed[at..])?;
                table.push(laz::laszip::ChunkTableEntry {
                    point_count: u64::from(n.points),
                    byte_count: u64::from(n.bytes),
                });
            }
            let mut out = vec![0u8; points as usize * rec];
            laz::par_decompress_selective(
                &compressed,
                &mut out,
                &self.vlr,
                &table,
                laz::DecompressionSelection::all(),
            )
            .map_err(|e| bad(&format!("no se pudo descomprimir un nodo: {e}")))?;
            f(&out)?;
            total += points;
            i = j;
        }
        Ok(total)
    }

    /// Decoded points of the nodes that touch `bbox`, down to `max_level`, keeping those inside
    /// the box (a node reaches past the edges of the query).
    pub fn scan_in(
        &self,
        bbox: [f64; 4],
        max_level: Option<i32>,
        mut f: impl FnMut(&Point),
    ) -> Result<u64> {
        let rec = usize::from(self.reader.header.record_len);
        let nodes = self.nodes(Some(bbox), max_level)?;
        let mut kept = 0u64;
        self.read_raw(&nodes, |bytes| {
            for r in bytes.chunks_exact(rec) {
                if let Some(p) = self.reader.decode(r)
                    && p.x >= bbox[0]
                    && p.x <= bbox[2]
                    && p.y >= bbox[1]
                    && p.y <= bbox[3]
                {
                    f(&p);
                    kept += 1;
                }
            }
            Ok(())
        })?;
        Ok(kept)
    }

    /// Like [`Copc::scan_in`], handing over the raw records of the points inside the box.
    pub fn scan_raw_in(
        &self,
        bbox: [f64; 4],
        max_level: Option<i32>,
        mut f: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<u64> {
        let rec = usize::from(self.reader.header.record_len);
        let nodes = self.nodes(Some(bbox), max_level)?;
        let mut kept = 0u64;
        self.read_raw(&nodes, |bytes| {
            let mut keep: Vec<u8> = Vec::new();
            for r in bytes.chunks_exact(rec) {
                if let Some(p) = self.reader.decode(r)
                    && p.x >= bbox[0]
                    && p.x <= bbox[2]
                    && p.y >= bbox[1]
                    && p.y <= bbox[3]
                {
                    keep.extend_from_slice(r);
                    kept += 1;
                }
            }
            if keep.is_empty() { Ok(()) } else { f(&keep) }
        })?;
        Ok(kept)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    /// A COPC file made by hand: 400 points in a 100 m cube, 1/4 of them at the root and the rest
    /// in the octants of level 1.
    pub(crate) fn build(path: &Path) -> Result<Vec<[f64; 3]>> {
        let mut pts: Vec<[f64; 3]> = Vec::new();
        let mut s = 12345u64;
        let mut next = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as f64 / (1u64 << 31) as f64) * 99.0
        };
        for _ in 0..400 {
            pts.push([next(), next(), next()]);
        }
        let record = |p: &[f64; 3]| -> Vec<u8> {
            let mut r = vec![0u8; 30];
            for (k, v) in p.iter().enumerate() {
                r[4 * k..4 * k + 4].copy_from_slice(&((v / 0.01).round() as i32).to_le_bytes());
            }
            r[12..14].copy_from_slice(&500u16.to_le_bytes());
            r[14] = 0x11;
            r[16] = 2;
            r
        };
        // nodes: the root takes every fourth point; the others go to the octant they lie in
        let mut root: Vec<u8> = Vec::new();
        let mut octants: Vec<Vec<u8>> = vec![Vec::new(); 8];
        for (i, p) in pts.iter().enumerate() {
            if i % 4 == 0 {
                root.extend(record(p));
            } else {
                let o = usize::from(p[0] >= 50.0)
                    | usize::from(p[1] >= 50.0) << 1
                    | usize::from(p[2] >= 50.0) << 2;
                octants[o].extend(record(p));
            }
        }
        let mut chunks: Vec<(i32, [i32; 3], Vec<u8>)> = vec![(0, [0, 0, 0], root)];
        for (o, data) in octants.into_iter().enumerate() {
            if !data.is_empty() {
                chunks.push((
                    1,
                    [(o & 1) as i32, ((o >> 1) & 1) as i32, ((o >> 2) & 1) as i32],
                    data,
                ));
            }
        }
        let items = laz::LazItemRecordBuilder::default_for_point_format_id(6, 0)
            .map_err(|e| bad(&e.to_string()))?;
        let vlr = laz::LazVlrBuilder::default()
            .with_laz_items(items)
            .with_variable_chunk_size()
            .build();
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut c = laz::ParLasZipCompressor::new(&mut cursor, vlr.clone())
                .map_err(|e| bad(&e.to_string()))?;
            c.compress_chunks(chunks.iter().map(|c| c.2.clone()).collect::<Vec<_>>())?;
            c.done().map_err(|e| bad(&e.to_string()))?;
        }
        let mut block = cursor.into_inner();
        let table = laz::laszip::ChunkTable::read_from(&mut Cursor::new(&block[..]), &vlr)
            .map_err(|e| bad(&e.to_string()))?;
        // the offset to the chunk table is absolute in a file: the block starts after the headers
        let relative = i64::from_le_bytes(block[..8].try_into().unwrap_or([0; 8]));
        let point_start = 375 + 54 + 160 + 54 + {
            let mut v = Vec::new();
            vlr.write_to(&mut v)?;
            v.len()
        };
        block[..8].copy_from_slice(&(relative + point_start as i64).to_le_bytes());
        let mut vlr_bytes = Vec::new();
        vlr.write_to(&mut vlr_bytes)?;
        // the headers
        let mut info = vec![0u8; 160];
        let hier_len = (chunks.len() * 32) as u64;
        let vlr_total = 54 + 160 + 54 + vlr_bytes.len();
        let point_offset = 375 + vlr_total;
        let evlr_at = point_offset + block.len();
        let hier_at = evlr_at as u64 + 60;
        for (k, v) in [50.0f64, 50.0, 50.0, 50.0, 1.0].iter().enumerate() {
            info[8 * k..8 * k + 8].copy_from_slice(&v.to_le_bytes());
        }
        info[40..48].copy_from_slice(&hier_at.to_le_bytes());
        info[48..56].copy_from_slice(&hier_len.to_le_bytes());
        let mut file = vec![0u8; 375];
        file[..4].copy_from_slice(b"LASF");
        file[24] = 1;
        file[25] = 4;
        file[94..96].copy_from_slice(&375u16.to_le_bytes());
        file[96..100].copy_from_slice(&(point_offset as u32).to_le_bytes());
        file[100..104].copy_from_slice(&2u32.to_le_bytes());
        file[104] = 6 | 0x80;
        file[105..107].copy_from_slice(&30u16.to_le_bytes());
        for k in 0..3 {
            file[131 + 8 * k..139 + 8 * k].copy_from_slice(&0.01f64.to_le_bytes());
        }
        for (at, v) in [
            (179, 99.0f64),
            (187, 0.0),
            (195, 99.0),
            (203, 0.0),
            (211, 99.0),
            (219, 0.0),
        ] {
            file[at..at + 8].copy_from_slice(&v.to_le_bytes());
        }
        file[235..243].copy_from_slice(&(evlr_at as u64).to_le_bytes());
        file[243..247].copy_from_slice(&1u32.to_le_bytes());
        file[247..255].copy_from_slice(&400u64.to_le_bytes());
        let vlr_header = |user: &str, record: u16, len: usize| {
            let mut h = vec![0u8; 54];
            h[2..2 + user.len()].copy_from_slice(user.as_bytes());
            h[18..20].copy_from_slice(&record.to_le_bytes());
            h[20..22].copy_from_slice(&(len as u16).to_le_bytes());
            h
        };
        file.extend(vlr_header("copc", 1, 160));
        file.extend(&info);
        file.extend(vlr_header("laszip encoded", 22204, vlr_bytes.len()));
        file.extend(&vlr_bytes);
        file.extend(&block);
        // the extended record with the hierarchy
        let mut evlr = vec![0u8; 60];
        evlr[2..6].copy_from_slice(b"copc");
        evlr[18..20].copy_from_slice(&1000u16.to_le_bytes());
        evlr[20..28].copy_from_slice(&hier_len.to_le_bytes());
        file.extend(evlr);
        let mut start = point_offset as u64 + 8;
        for ((level, key, _), entry) in chunks.iter().zip(table.as_ref()) {
            file.extend(level.to_le_bytes());
            for k in key {
                file.extend(k.to_le_bytes());
            }
            file.extend(start.to_le_bytes());
            file.extend((entry.byte_count as u32).to_le_bytes());
            file.extend((entry.point_count as u32).to_le_bytes());
            start += entry.byte_count;
        }
        std::fs::write(path, file)?;
        Ok(pts)
    }

    #[test]
    fn a_query_reads_only_the_nodes_it_touches() -> Result<()> {
        let path = std::env::temp_dir().join(format!("rl_copc_{}.copc.laz", std::process::id()));
        let pts = build(&path)?;
        // a plain LAZ is not COPC
        let copc = Copc::open(&path)?.ok_or_else(|| bad("el archivo de prueba no se reconoce"))?;
        assert_eq!(copc.nodes(None, None)?.len(), 9);
        assert_eq!(copc.nodes(None, Some(0))?.len(), 1);
        // everything
        let mut all = 0;
        let n = copc.scan_in([-1.0, -1.0, 101.0, 101.0], None, |_| all += 1)?;
        assert_eq!((n, all), (400, 400));
        // a window in the lower-left corner touches the root and one octant of each z only
        let bbox = [10.0, 10.0, 30.0, 30.0];
        let expected = pts
            .iter()
            .filter(|p| p[0] >= 10.0 && p[0] <= 30.0 && p[1] >= 10.0 && p[1] <= 30.0)
            .count() as u64;
        let nodes = copc.nodes(Some(bbox), None)?;
        assert!(nodes.len() < 9 && nodes.len() >= 3, "{nodes:?}");
        let mut seen = 0u64;
        let mut z_sum = 0.0;
        let kept = copc.scan_in(bbox, None, |p| {
            seen += 1;
            z_sum += p.z;
            assert!(p.x >= 10.0 && p.x <= 30.0 && p.y >= 10.0 && p.y <= 30.0);
        })?;
        assert_eq!((kept, seen), (expected, expected));
        let expected_z: f64 = pts
            .iter()
            .filter(|p| p[0] >= 10.0 && p[0] <= 30.0 && p[1] >= 10.0 && p[1] <= 30.0)
            .map(|p| p[2])
            .sum();
        assert!((z_sum - expected_z).abs() < 0.02 * expected as f64);
        // raw records of the same window
        let mut raw = 0usize;
        let n = copc.scan_raw_in(bbox, None, |b| {
            raw += b.len() / 30;
            Ok(())
        })?;
        assert_eq!((n as usize, raw), (expected as usize, expected as usize));
        // the overview: the root alone fits a budget of 100 points, both levels one of 400
        assert_eq!(copc.overview(100)?.len(), 1);
        assert_eq!(copc.overview(1000)?.len(), 9);
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn plain_laz_and_garbage_are_not_copc() -> Result<()> {
        let path = std::env::temp_dir().join(format!("rl_notcopc_{}.laz", std::process::id()));
        std::fs::write(&path, b"nada que ver")?;
        assert!(Copc::open(&path)?.is_none());
        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}
