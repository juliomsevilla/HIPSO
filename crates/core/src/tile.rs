//! Tiling with a buffer: one pass spreads the points of any number of files over square tiles
//! (each point also goes to the neighbouring tiles whose buffer it falls in), kept in temporary
//! files; tiles are then loaded one per thread and processed in parallel. Memory is the
//! threads times a tile, whatever the size of the cloud.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;

use rayon::prelude::*;

use crate::las::{Header, Points, Reader};
use crate::{Error, Result};

/// Square tiles of `size` from `(x0, y0)`, each read with `buffer` around it.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub x0: f64,
    pub y0: f64,
    pub size: f64,
    pub buffer: f64,
    pub cols: usize,
    pub rows: usize,
}

impl Layout {
    /// Tiles covering `[min x, min y, max x, max y]`, aligned on multiples of `align` (the
    /// output cell size, so tiles share one raster lattice).
    pub fn new(b: [f64; 4], size: f64, buffer: f64, align: f64) -> Result<Self> {
        if !(size > 0.0) || !(buffer >= 0.0) || !(align > 0.0) {
            return Err(Error::Param(
                "tiles need a positive size and a buffer of 0 or more".into(),
            ));
        }
        let size = (size / align).round().max(1.0) * align;
        let x0 = (b[0] / align).floor() * align;
        let y0 = (b[1] / align).floor() * align;
        let cols = ((b[2] - x0) / size).floor() as usize + 1;
        let rows = ((b[3] - y0) / size).floor() as usize + 1;
        if cols.saturating_mul(rows) > 1 << 20 {
            return Err(Error::Param(
                "too many tiles: use a larger tile size".into(),
            ));
        }
        Ok(Self {
            x0,
            y0,
            size,
            buffer,
            cols,
            rows,
        })
    }

    pub fn len(&self) -> usize {
        self.cols * self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The tile's own extent `[min x, min y, max x, max y]` (tile 0 is the south-west one).
    pub fn core(&self, t: usize) -> [f64; 4] {
        let (c, r) = ((t % self.cols) as f64, (t / self.cols) as f64);
        let (x, y) = (self.x0 + c * self.size, self.y0 + r * self.size);
        [x, y, x + self.size, y + self.size]
    }

    /// The tile a point belongs to.
    fn home(&self, x: f64, y: f64) -> (usize, usize) {
        let c = (((x - self.x0) / self.size).floor().max(0.0) as usize).min(self.cols - 1);
        let r = (((y - self.y0) / self.size).floor().max(0.0) as usize).min(self.rows - 1);
        (c, r)
    }

    /// Calls `f(tile, is_home)` for every tile whose buffered extent holds the point.
    fn visit(&self, x: f64, y: f64, mut f: impl FnMut(usize, bool)) {
        let (hc, hr) = self.home(x, y);
        for r in hr.saturating_sub(1)..=(hr + 1).min(self.rows - 1) {
            for c in hc.saturating_sub(1)..=(hc + 1).min(self.cols - 1) {
                let t = r * self.cols + c;
                let e = self.core(t);
                let home = (c, r) == (hc, hr);
                let b = self.buffer;
                if home || (x >= e[0] - b && x < e[2] + b && y >= e[1] - b && y < e[3] + b) {
                    f(t, home);
                }
            }
        }
    }
}

/// A temporary folder, deleted with everything in it when dropped.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One loaded tile: its points (all, buffer included), their raw records, and which are its own.
pub struct Tile {
    pub index: usize,
    pub core: [f64; 4],
    pub points: Points,
    pub records: Vec<u8>,
    pub own: Vec<bool>,
}

/// Points spread over tiles, in temporary files.
pub struct Tiled {
    pub layout: Layout,
    pub header: Header,
    counts: Vec<u64>,
    dir: TempDir,
    /// Whether loaded tiles keep their raw records (needed only to write points out).
    pub keep_records: bool,
}

/// Bytes of spooled records held, over all tiles, before they are appended to their files.
const SPOOL_BUDGET: usize = 64 << 20;

impl Tiled {
    /// Spreads the points of `inputs` over the tiles of `layout`, in one pass. All inputs must
    /// share the first one's point format and record length; coordinates are converted to the
    /// first one's scale and offset.
    pub fn spool(inputs: &[Reader], layout: Layout) -> Result<Self> {
        let first = inputs
            .first()
            .ok_or_else(|| Error::Param("no input files".into()))?;
        let header = first.header.clone();
        let rec_len = usize::from(header.record_len);
        let dir = std::env::temp_dir().join(format!(
            "rustlidar-tiles-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir)?;
        let dir = TempDir(dir);
        let n = layout.len();
        let flush_at = (SPOOL_BUDGET / n.max(1)).clamp(64 << 10, 8 << 20);
        let mut bufs: Vec<Vec<u8>> = vec![Vec::new(); n];
        let mut counts = vec![0u64; n];
        let flush = |t: usize, buf: &mut Vec<u8>| -> Result<()> {
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.0.join(format!("{t}.bin")))?;
            f.write_all(buf)?;
            buf.clear();
            Ok(())
        };
        for input in inputs {
            let h = &input.header;
            if h.format != header.format || h.record_len != header.record_len {
                return Err(Error::Unsupported(format!(
                    "{}: point format {} ({} bytes) differs from the first file's {} ({} bytes)",
                    input.path().display(),
                    h.format,
                    h.record_len,
                    header.format,
                    header.record_len
                )));
            }
            let same = h.scale == header.scale && h.offset == header.offset;
            let mut err = None;
            input.scan_raw(|bytes| {
                for rec in bytes.chunks_exact(rec_len) {
                    let Some(p) = input.decode(rec) else { continue };
                    let mut rec = rec.to_vec();
                    if !same && !crate::las::set_xyz(&header, &mut rec, p.x, p.y, p.z) {
                        return Err(Error::Param(
                            "a point outside the first file's coordinate range".into(),
                        ));
                    }
                    layout.visit(p.x, p.y, |t, home| {
                        if let (Some(b), Some(c)) = (bufs.get_mut(t), counts.get_mut(t)) {
                            b.extend_from_slice(&rec);
                            b.push(u8::from(home));
                            *c += 1;
                            if b.len() >= flush_at
                                && let Err(e) = flush(t, b)
                            {
                                err = Some(e);
                            }
                        }
                    });
                }
                Ok(())
            })?;
            if let Some(e) = err {
                return Err(e);
            }
        }
        for (t, b) in bufs.iter_mut().enumerate() {
            if !b.is_empty() {
                flush(t, b)?;
            }
        }
        Ok(Self {
            layout,
            header,
            counts,
            dir,
            keep_records: true,
        })
    }

    /// The tiles holding points, in order.
    pub fn tiles(&self) -> Vec<usize> {
        (0..self.counts.len())
            .filter(|&t| self.counts[t] > 0)
            .collect()
    }

    fn path(&self, t: usize) -> PathBuf {
        self.dir.0.join(format!("{t}.bin"))
    }

    /// Loads one tile, and deletes its file.
    pub fn load(&self, t: usize) -> Result<Tile> {
        let rec_len = usize::from(self.header.record_len);
        let n = self.counts.get(t).copied().unwrap_or(0) as usize;
        // Read in blocks, so the file's bytes and the decoded points are never held together.
        let mut file = BufReader::new(File::open(self.path(t))?);
        let mut raw = vec![0u8; (rec_len + 1) * 65536];
        let mut records = Vec::with_capacity(if self.keep_records { n * rec_len } else { 0 });
        let mut own = Vec::with_capacity(n);
        let mut pts = Vec::with_capacity(n);
        let mut left = n;
        while left > 0 {
            let k = left.min(65536);
            let block = raw
                .get_mut(..k * (rec_len + 1))
                .ok_or_else(|| Error::Format("tile block".into()))?;
            file.read_exact(block)?;
            for chunk in block.chunks_exact(rec_len + 1) {
                let (rec, flag) = chunk.split_at(rec_len);
                if let Some(p) = crate::las::decode(&self.header, rec) {
                    pts.push(p);
                    if self.keep_records {
                        records.extend_from_slice(rec);
                    }
                    own.push(flag.first() == Some(&1));
                }
            }
            left -= k;
        }
        drop(file);
        let _ = std::fs::remove_file(self.path(t));
        Ok(Tile {
            index: t,
            core: self.layout.core(t),
            points: Points::new(pts),
            records,
            own,
        })
    }

    /// Runs `f` on every tile, `threads` at a time (0: one per core), handing each batch's
    /// results to `sink` in tile order before the next batch is loaded.
    pub fn process<R: Send>(
        &self,
        threads: usize,
        f: impl Fn(Tile) -> Result<R> + Sync,
        mut sink: impl FnMut(R) -> Result<()>,
    ) -> Result<()> {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map_err(|e| Error::Param(format!("threads: {e}")))?;
        let tiles = self.tiles();
        let batch = pool.current_num_threads().max(1);
        for group in tiles.chunks(batch) {
            crate::check_cancel()?;
            let results: Vec<Result<R>> = pool.install(|| {
                group
                    .par_iter()
                    .map(|&t| self.load(t).and_then(&f))
                    .collect()
            });
            for r in results {
                sink(r?)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_near_edges_go_to_neighbours_too() -> Result<()> {
        let l = Layout::new([0.0, 0.0, 199.0, 99.0], 100.0, 10.0, 1.0)?;
        assert_eq!((l.cols, l.rows), (2, 1));
        let mut seen = Vec::new();
        l.visit(95.0, 50.0, |t, home| seen.push((t, home)));
        assert_eq!(seen, vec![(0, true), (1, false)]);
        seen.clear();
        l.visit(50.0, 50.0, |t, home| seen.push((t, home)));
        assert_eq!(seen, vec![(0, true)]);
        Ok(())
    }
}
