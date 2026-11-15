//! División en teselas con margen: una pasada reparte los puntos de cualquier número de archivos en
//! teselas cuadradas (cada punto va también a las teselas vecinas en cuyo margen cae), guardadas en
//! archivos temporales; después se carga una tesela por hilo y se procesan en paralelo. La memoria
//! son los hilos por una tesela, sea cual sea el tamaño de la nube.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;

use rayon::prelude::*;

use crate::las::{Header, Points, Reader};
use crate::{Error, Result};

/// Teselas cuadradas de `size` desde `(x0, y0)`, cada una leída con `buffer` a su alrededor.
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
    /// Teselas que cubren `[x mínima, y mínima, x máxima, y máxima]`, alineadas en múltiplos de
    /// `align` (el tamaño de celda de salida, de modo que las teselas comparten un retículo
    /// ráster).
    pub fn new(b: [f64; 4], size: f64, buffer: f64, align: f64) -> Result<Self> {
        if !(size > 0.0) || !(buffer >= 0.0) || !(align > 0.0) {
            return Err(Error::Param(
                "las teselas necesitan un tamaño positivo y un margen de 0 o más".into(),
            ));
        }
        let size = (size / align).round().max(1.0) * align;
        let x0 = (b[0] / align).floor() * align;
        let y0 = (b[1] / align).floor() * align;
        let cols = ((b[2] - x0) / size).floor() as usize + 1;
        let rows = ((b[3] - y0) / size).floor() as usize + 1;
        if cols.saturating_mul(rows) > 1 << 20 {
            return Err(Error::Param(
                "demasiadas teselas: usa teselas más grandes".into(),
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

    /// La extensión propia de la tesela `[x mínima, y mínima, x máxima, y máxima]` (la tesela 0 es
    /// la suroeste).
    pub fn core(&self, t: usize) -> [f64; 4] {
        let (c, r) = ((t % self.cols) as f64, (t / self.cols) as f64);
        let (x, y) = (self.x0 + c * self.size, self.y0 + r * self.size);
        [x, y, x + self.size, y + self.size]
    }

    /// La tesela a la que pertenece un punto.
    fn home(&self, x: f64, y: f64) -> (usize, usize) {
        let c = (((x - self.x0) / self.size).floor().max(0.0) as usize).min(self.cols - 1);
        let r = (((y - self.y0) / self.size).floor().max(0.0) as usize).min(self.rows - 1);
        (c, r)
    }

    /// Llama a `f(tesela, es_propia)` para cada tesela cuya extensión con margen contiene el punto.
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

/// Una carpeta temporal, borrada con todo lo que contiene al soltarla.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Una tesela cargada: sus puntos (todos, margen incluido), sus registros en bruto y cuáles son
/// propios.
pub struct Tile {
    pub index: usize,
    pub core: [f64; 4],
    pub points: Points,
    pub records: Vec<u8>,
    pub own: Vec<bool>,
}

/// Puntos repartidos en teselas, en archivos temporales.
pub struct Tiled {
    pub layout: Layout,
    pub header: Header,
    counts: Vec<u64>,
    dir: TempDir,
    /// Si las teselas cargadas conservan sus registros en bruto (solo hacen falta para escribir
    /// puntos).
    pub keep_records: bool,
}

/// Bytes de registros volcados retenidos, entre todas las teselas, antes de añadirse a sus
/// archivos.
const SPOOL_BUDGET: usize = 64 << 20;

impl Tiled {
    /// Reparte los puntos de `inputs` entre las teselas de `layout`, en una pasada. Todas las
    /// entradas deben compartir el formato de punto y la longitud de registro de la primera; las
    /// coordenadas se convierten a la escala y el desplazamiento de la primera.
    pub fn spool(inputs: &[Reader], layout: Layout) -> Result<Self> {
        Self::spool_with_context(inputs, &[], layout)
    }

    /// [`Tiled::spool`] con `context`: archivos cuyos puntos solo ayudan a decidir sobre los puntos
    /// de `inputs` (las hojas vecinas de un bloque). Sus puntos se guardan en las teselas a las que
    /// llegan, pero ninguno es propio de una tesela, así que no se escribe ninguno; los que están
    /// más allá del margen del trazado se dejan fuera.
    pub fn spool_with_context(
        inputs: &[Reader],
        context: &[Reader],
        layout: Layout,
    ) -> Result<Self> {
        let first = inputs
            .first()
            .ok_or_else(|| Error::Param("no hay archivos de entrada".into()))?;
        let header = first.header.clone();
        let rec_len = usize::from(header.record_len);
        // el reloj de algunos sistemas avanza en microsegundos: un contador mantiene separados dos
        // volcados iniciados a la vez
        static SPOOLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "hipso-tiles-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
            SPOOLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)?;
        let dir = TempDir(dir);
        let n = layout.len();
        let flush_at = (SPOOL_BUDGET / n.max(1)).clamp(64 << 10, 8 << 20);
        let mut bufs: Vec<Vec<u8>> = vec![Vec::new(); n];
        let mut counts = vec![0u64; n];
        // leer y repartir los puntos es aproximadamente la mitad del trabajo, procesar las teselas
        // el resto
        crate::progress::begin(
            2 * inputs
                .iter()
                .chain(context)
                .map(|r| r.header.count)
                .sum::<u64>(),
        );
        // donde los puntos del contexto todavía importan: el trazado y su margen
        let reach = [
            layout.x0 - layout.buffer,
            layout.y0 - layout.buffer,
            layout.x0 + layout.cols as f64 * layout.size + layout.buffer,
            layout.y0 + layout.rows as f64 * layout.size + layout.buffer,
        ];
        let flush = |t: usize, buf: &mut Vec<u8>| -> Result<()> {
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.0.join(format!("{t}.bin")))?;
            f.write_all(buf)?;
            buf.clear();
            Ok(())
        };
        for (n, input) in inputs.iter().chain(context).enumerate() {
            let is_context = n >= inputs.len();
            let h = &input.header;
            if h.format != header.format || h.record_len != header.record_len {
                return Err(Error::Unsupported(format!(
                    "{}: el formato de punto {} ({} bytes) es distinto del del primer archivo, {} ({} bytes)",
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
                crate::progress::advance((bytes.len() / rec_len.max(1)) as u64);
                for rec in bytes.chunks_exact(rec_len) {
                    let Some(p) = input.decode(rec) else { continue };
                    if is_context
                        && (p.x < reach[0] || p.x >= reach[2] || p.y < reach[1] || p.y >= reach[3])
                    {
                        continue;
                    }
                    let mut rec = rec.to_vec();
                    if !same && !crate::las::set_xyz(&header, &mut rec, p.x, p.y, p.z) {
                        return Err(Error::Param(
                            "un punto fuera del rango de coordenadas del primer archivo".into(),
                        ));
                    }
                    layout.visit(p.x, p.y, |t, home| {
                        if let (Some(b), Some(c)) = (bufs.get_mut(t), counts.get_mut(t)) {
                            b.extend_from_slice(&rec);
                            b.push(u8::from(home && !is_context));
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

    /// Las teselas que contienen puntos, en orden.
    pub fn tiles(&self) -> Vec<usize> {
        (0..self.counts.len())
            .filter(|&t| self.counts[t] > 0)
            .collect()
    }

    fn path(&self, t: usize) -> PathBuf {
        self.dir.0.join(format!("{t}.bin"))
    }

    /// Carga una tesela y borra su archivo.
    pub fn load(&self, t: usize) -> Result<Tile> {
        let rec_len = usize::from(self.header.record_len);
        let n = self.counts.get(t).copied().unwrap_or(0) as usize;
        // Leído por bloques, de modo que los bytes del archivo y los puntos decodificados nunca
        // están a la vez.
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

    /// Ejecuta `f` sobre cada tesela, `threads` a la vez (0: uno por núcleo), entregando los
    /// resultados de cada lote a `sink` en orden de tesela antes de cargar el siguiente.
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
            crate::progress::advance(
                group
                    .iter()
                    .map(|&t| self.counts.get(t).copied().unwrap_or(0))
                    .sum(),
            );
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
