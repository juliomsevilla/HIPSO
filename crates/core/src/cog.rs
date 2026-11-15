//! Cloud Optimized GeoTIFF: el mismo ráster en teselas de 512 x 512, con copias reducidas (vistas
//! reducidas) y todos los directorios del archivo al principio, de modo que un visor o un servidor
//! pueda leer una ventana o una vista gruesa con unas pocas peticiones por rango en lugar del
//! archivo entero.
//!
//! La disposición sigue la habitual: cabecera, luego los directorios de la imagen completa y de
//! cada vista reducida (la resolución completa primero) y luego las teselas desde la vista reducida
//! más pequeña hasta las de la imagen completa. Las teselas se comprimen con deflate. Las muestras
//! son decimales de 32 bits; las celdas vacías (NaN) se escriben como el valor nodata de
//! [`Grid::write_tiff`].

use std::io::Write;
use std::path::Path;

use rayon::prelude::*;

use crate::grid::{Grid, NODATA};
use crate::{Error, Result, check_cancel};

const TILE: usize = 512;

/// Un nivel: su tamaño y sus muestras (NaN para sin datos).
struct Level {
    cols: usize,
    rows: usize,
    data: Vec<f32>,
}

/// Copia a la mitad de tamaño: cada celda es la media de las válidas de las 2 x 2 de debajo (o la
/// primera de ellas, con `nearest`, para clases e identificadores).
fn reduce(l: &Level, nearest: bool) -> Level {
    let (cols, rows) = (l.cols.div_ceil(2), l.rows.div_ceil(2));
    let mut data = vec![f32::NAN; cols * rows];
    data.par_chunks_mut(cols).enumerate().for_each(|(r, row)| {
        for (c, out) in row.iter_mut().enumerate() {
            let (mut sum, mut n, mut first) = (0f32, 0u32, f32::NAN);
            for dr in 0..2 {
                for dc in 0..2 {
                    let (rr, cc) = (2 * r + dr, 2 * c + dc);
                    if rr < l.rows && cc < l.cols {
                        let v = l.data[rr * l.cols + cc];
                        if !v.is_nan() {
                            if n == 0 {
                                first = v;
                            }
                            sum += v;
                            n += 1;
                        }
                    }
                }
            }
            *out = if n == 0 {
                f32::NAN
            } else if nearest {
                first
            } else {
                sum / n as f32
            };
        }
    });
    Level { cols, rows, data }
}

fn deflate(raw: &[u8]) -> Result<Vec<u8>> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(raw)?;
    Ok(z.finish()?)
}

/// Las teselas comprimidas de un nivel, en orden de filas de teselas.
fn tiles(l: &Level) -> Result<Vec<Vec<u8>>> {
    let (tc, tr) = (l.cols.div_ceil(TILE), l.rows.div_ceil(TILE));
    (0..tc * tr)
        .into_par_iter()
        .map(|t| {
            let (r0, c0) = ((t / tc) * TILE, (t % tc) * TILE);
            let mut raw = Vec::with_capacity(TILE * TILE * 4);
            for r in r0..r0 + TILE {
                for c in c0..c0 + TILE {
                    let v = if r < l.rows && c < l.cols {
                        l.data[r * l.cols + c]
                    } else {
                        f32::NAN
                    };
                    raw.extend((if v.is_nan() { NODATA } else { v }).to_le_bytes());
                }
            }
            deflate(&raw)
        })
        .collect()
}

/// Una entrada de un directorio: etiqueta, tipo (3 corto, 4 largo, 12 doble, 2 ascii), recuento,
/// bytes.
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    data: Vec<u8>,
}

fn long(tag: u16, v: u32) -> Entry {
    Entry {
        tag,
        kind: 4,
        count: 1,
        data: v.to_le_bytes().to_vec(),
    }
}

fn short(tag: u16, v: u16) -> Entry {
    Entry {
        tag,
        kind: 3,
        count: 1,
        data: v.to_le_bytes().to_vec(),
    }
}

fn longs(tag: u16, v: &[u32]) -> Entry {
    Entry {
        tag,
        kind: 4,
        count: v.len() as u32,
        data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
    }
}

/// Los bytes de un directorio colocado en `at`, seguidos de los valores que no caben en sus
/// entradas.
fn directory(mut entries: Vec<Entry>, at: u32) -> Vec<u8> {
    entries.sort_by_key(|e| e.tag);
    let table = 2 + 12 * entries.len() + 4;
    let mut out = Vec::new();
    out.extend((entries.len() as u16).to_le_bytes());
    let mut outside: Vec<u8> = Vec::new();
    for e in &entries {
        out.extend(e.tag.to_le_bytes());
        out.extend(e.kind.to_le_bytes());
        out.extend(e.count.to_le_bytes());
        if e.data.len() <= 4 {
            let mut v = e.data.clone();
            v.resize(4, 0);
            out.extend(v);
        } else {
            let place = at + table as u32 + outside.len() as u32;
            out.extend(place.to_le_bytes());
            outside.extend(&e.data);
            if outside.len() % 2 == 1 {
                outside.push(0);
            }
        }
    }
    // aquí no hay directorio siguiente; quien llama lo corrige
    out.extend(0u32.to_le_bytes());
    out.extend(outside);
    out
}

/// Escribe `g` como Cloud Optimized GeoTIFF con vistas reducidas hasta una tesela. `nearest` reduce
/// tomando una celda en lugar de promediar (para clases). `epsg` es el sistema proyectado, si se
/// conoce.
pub fn write(g: &Grid, path: &Path, epsg: Option<u16>, nearest: bool) -> Result<()> {
    let mut levels = vec![Level {
        cols: g.cols,
        rows: g.rows,
        data: g.data.clone(),
    }];
    while levels
        .last()
        .is_some_and(|l| l.cols > TILE || l.rows > TILE)
    {
        check_cancel()?;
        let next = reduce(&levels[levels.len() - 1], nearest);
        levels.push(next);
    }
    let packed: Vec<Vec<Vec<u8>>> = levels.iter().map(tiles).collect::<Result<_>>()?;
    let top = g.y0 + g.rows as f64 * g.cell;
    let mut keys: Vec<u16> = vec![1, 1, 0, 2, 1024, 0, 1, 1, 1025, 0, 1, 1];
    if let Some(code) = epsg {
        keys[3] = 3;
        keys.extend([3072, 0, 1, code]);
    }
    let doubles = |v: &[f64]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let nodata = format!("{NODATA}\0");
    // las entradas del directorio de cada nivel, sin los desplazamientos de teselas (sus posiciones
    // no se conocen hasta que se conocen los tamaños de los directorios)
    let entries = |k: usize, offsets: &[u32]| -> Vec<Entry> {
        let l = &levels[k];
        let counts: Vec<u32> = packed[k].iter().map(|t| t.len() as u32).collect();
        let mut e = vec![
            long(256, l.cols as u32),
            long(257, l.rows as u32),
            Entry {
                tag: 258,
                kind: 3,
                count: 1,
                data: 32u16.to_le_bytes().to_vec(),
            },
            short(259, 8),
            short(262, 1),
            short(277, 1),
            short(284, 1),
            long(322, TILE as u32),
            long(323, TILE as u32),
            longs(324, offsets),
            longs(325, &counts),
            short(339, 3),
        ];
        if k == 0 {
            e.push(Entry {
                tag: 33550,
                kind: 12,
                count: 3,
                data: doubles(&[g.cell, g.cell, 0.0]),
            });
            e.push(Entry {
                tag: 33922,
                kind: 12,
                count: 6,
                data: doubles(&[0.0, 0.0, 0.0, g.x0, top, 0.0]),
            });
            e.push(Entry {
                tag: 34735,
                kind: 3,
                count: keys.len() as u32,
                data: keys.iter().flat_map(|x| x.to_le_bytes()).collect(),
            });
            e.push(Entry {
                tag: 42113,
                kind: 2,
                count: nodata.len() as u32,
                data: nodata.clone().into_bytes(),
            });
        } else {
            // una copia reducida de la imagen de arriba
            e.push(long(254, 1));
        }
        e
    };
    // tamaños de los directorios y luego dónde empieza cada uno
    let zero = |k: usize| vec![0u32; packed[k].len()];
    let sizes: Vec<usize> = (0..levels.len())
        .map(|k| directory(entries(k, &zero(k)), 0).len())
        .collect();
    let mut dir_at = Vec::new();
    let mut pos = 8usize;
    for s in &sizes {
        dir_at.push(pos);
        pos += s + (s % 2);
    }
    // las teselas: la vista reducida más pequeña primero
    let mut offsets: Vec<Vec<u32>> = vec![Vec::new(); levels.len()];
    for k in (0..levels.len()).rev() {
        for t in &packed[k] {
            offsets[k].push(u32::try_from(pos).map_err(|_| {
                Error::Unsupported("el COG superaría los 4 GB: usa un ráster más pequeño".into())
            })?);
            pos += t.len();
        }
    }
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    file.write_all(b"II")?;
    file.write_all(&42u16.to_le_bytes())?;
    file.write_all(&(dir_at[0] as u32).to_le_bytes())?;
    for k in 0..levels.len() {
        let mut d = directory(entries(k, &offsets[k]), dir_at[k] as u32);
        // el enlace al directorio siguiente está justo después de las entradas
        let entries_len = 2 + 12 * u16::from_le_bytes([d[0], d[1]]) as usize;
        let next = dir_at.get(k + 1).map_or(0, |a| *a as u32);
        d[entries_len..entries_len + 4].copy_from_slice(&next.to_le_bytes());
        if d.len() % 2 == 1 {
            d.push(0);
        }
        file.write_all(&d)?;
    }
    for k in (0..levels.len()).rev() {
        for t in &packed[k] {
            file.write_all(t)?;
        }
    }
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiff::decoder::{Decoder, DecodingResult};

    #[test]
    fn a_cog_reads_back_and_has_its_overviews_up_front() -> Result<()> {
        // 1300 x 700: 3 x 2 teselas, dos vistas reducidas (650 x 350 y luego 325 x 175)
        let mut g = Grid::new(500_000.0, 4_000_000.0, 2.0, 1300, 700)?;
        for (i, v) in g.data.iter_mut().enumerate() {
            let (c, r) = (i % 1300, i / 1300);
            *v = if c < 5 && r < 5 {
                f32::NAN
            } else {
                (c + 2 * r) as f32 / 10.0
            };
        }
        let path = std::env::temp_dir().join(format!("rl_cog_{}.tif", std::process::id()));
        write(&g, &path, Some(25830), false)?;
        // la imagen completa se lee como cualquier GeoTIFF
        let back = Grid::read_tiff(&path)?;
        assert_eq!((back.cols, back.rows, back.cell), (1300, 700, 2.0));
        assert_eq!((back.x0, back.y0), (500_000.0, 4_000_000.0));
        assert_eq!(Grid::read_epsg(&path), Some(25830));
        for i in [0usize, 7, 1299, 1300 * 5 + 5, 1300 * 699 + 1299] {
            let (a, b) = (g.data[i], back.data[i]);
            assert!(
                (a.is_nan() && b.is_nan()) || (a - b).abs() < 1e-5,
                "{i}: {a} {b}"
            );
        }
        // las vistas reducidas están en el archivo, y todo directorio va antes de la primera tesela
        let mut d = Decoder::new(std::io::BufReader::new(std::fs::File::open(&path)?))
            .map(|d| d.with_limits(tiff::decoder::Limits::unlimited()))
            .map_err(|e| Error::Format(e.to_string()))?;
        let mut sizes = vec![d.dimensions().map_err(|e| Error::Format(e.to_string()))?];
        while d.more_images() {
            d.next_image().map_err(|e| Error::Format(e.to_string()))?;
            sizes.push(d.dimensions().map_err(|e| Error::Format(e.to_string()))?);
        }
        assert_eq!(sizes, vec![(1300, 700), (650, 350), (325, 175)]);
        // la vista reducida más pequeña es una media de 4 x 4 celdas de la imagen completa (lejos
        // del hueco)
        d.seek_to_image(2)
            .map_err(|e| Error::Format(e.to_string()))?;
        let DecodingResult::F32(small) =
            d.read_image().map_err(|e| Error::Format(e.to_string()))?
        else {
            return Err(Error::Format("tipo".into()));
        };
        let at = |c: usize, r: usize| small[r * 325 + c];
        let mean: f32 = (0..4)
            .flat_map(|dr| (0..4).map(move |dc| ((40 + dc) + 2 * (80 + dr)) as f32 / 10.0))
            .sum::<f32>()
            / 16.0;
        assert!((at(10, 20) - mean).abs() < 1e-3, "{} {mean}", at(10, 20));
        // la cabecera primero: donde empieza la primera tesela está tras el último directorio
        let bytes = std::fs::read(&path)?;
        let first_dir = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        assert_eq!(first_dir, 8);
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn a_small_raster_is_one_level() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 40, 30)?;
        g.data.fill(3.0);
        let path = std::env::temp_dir().join(format!("rl_cog_small_{}.tif", std::process::id()));
        write(&g, &path, None, true)?;
        let back = Grid::read_tiff(&path)?;
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.data.len(), 1200);
        assert!(back.data.iter().all(|v| *v == 3.0));
        Ok(())
    }
}
