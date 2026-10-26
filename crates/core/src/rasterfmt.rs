//! The common raster formats that are not GeoTIFF or JPEG 2000: PNG and JPEG with a world file,
//! ENVI and ESRI binary rasters with their `.hdr`, and SRTM `.hgt` elevation tiles. They are read
//! whole into memory (they are small next to an aerial orthophoto) and served as an [`Image`].

use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::{Path, PathBuf};

use crate::raster::Image;
use crate::{Error, Result};

/// A raster read into memory with what the file says about its system.
pub struct Loaded {
    pub image: Image,
    pub epsg: Option<u16>,
    /// Bits of each sample.
    pub bits: u8,
}

fn bad(path: &Path, what: &str) -> Error {
    Error::Format(format!("{}: {what}", path.display()))
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// What kind of file this is, by its first bytes and its name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Png,
    Jpeg,
    Hgt,
    /// A flat binary raster whose `.hdr` describes it (ENVI or ESRI).
    Binary,
}

pub fn kind_of(path: &Path) -> Option<Kind> {
    let mut head = [0u8; 8];
    let n = File::open(path).and_then(|mut f| f.read(&mut head)).ok()?;
    if head.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(Kind::Png);
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Kind::Jpeg);
    }
    let ext = ext_of(path);
    if ext == "hgt" {
        return Some(Kind::Hgt);
    }
    if n > 0
        && matches!(
            ext.as_str(),
            "bil" | "bsq" | "bip" | "img" | "dat" | "raw" | "flt" | "envi" | ""
        )
    {
        return header_of(path).map(|_| Kind::Binary);
    }
    None
}

fn header_of(path: &Path) -> Option<PathBuf> {
    let mut a = path.as_os_str().to_owned();
    a.push(".hdr");
    [path.with_extension("hdr"), PathBuf::from(a)]
        .into_iter()
        .find(|p| p.is_file())
}

/// Reads the raster if it is one of these formats; `None` when it is not.
pub fn load(path: &Path) -> Result<Option<Loaded>> {
    Ok(Some(match kind_of(path) {
        Some(Kind::Png) => load_png(path)?,
        Some(Kind::Jpeg) => load_jpeg(path)?,
        Some(Kind::Hgt) => load_hgt(path)?,
        Some(Kind::Binary) => load_binary(path)?,
        None => return Ok(None),
    }))
}

// ------------------------------------------------------------------------------------ world file

/// The pixel grid a world file gives: west edge, north edge and cell size.
fn world_file(path: &Path, exts: &[&str]) -> Result<(f64, f64, f64)> {
    let file = exts
        .iter()
        .map(|e| path.with_extension(e))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            bad(
                path,
                &format!(
                    "imagen sin georreferenciar: falta su archivo de coordenadas (.{})",
                    exts.first().unwrap_or(&"wld")
                ),
            )
        })?;
    let text = std::fs::read_to_string(&file)?;
    let v: Vec<f64> = text
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    let [a, d, b, e, c, f] = v
        .get(..6)
        .and_then(|s| <[f64; 6]>::try_from(s).ok())
        .ok_or_else(|| bad(&file, "un archivo de coordenadas necesita seis números"))?;
    if d.abs() > 1e-12 * a.abs() || b.abs() > 1e-12 * a.abs() {
        return Err(Error::Unsupported(format!(
            "{}: imagen girada; remuestréala antes a una cuadrícula norte-arriba",
            file.display()
        )));
    }
    if (a.abs() - e.abs()).abs() > 1e-6 * a.abs() || a == 0.0 {
        return Err(Error::Unsupported(format!(
            "{}: píxeles no cuadrados ({} x {})",
            file.display(),
            a.abs(),
            e.abs()
        )));
    }
    // the file gives the centre of the upper-left pixel
    Ok((c - a.abs() / 2.0, f + e.abs() / 2.0, a.abs()))
}

fn prj_epsg(path: &Path) -> Option<u16> {
    std::fs::read_to_string(path.with_extension("prj"))
        .ok()
        .and_then(|t| crate::vector::epsg_from_prj(&t))
}

fn image_from(
    x0: f64,
    y_top: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    bands: Vec<Vec<f32>>,
) -> Image {
    Image {
        x0,
        y0: y_top - rows as f64 * cell,
        cell,
        cols,
        rows,
        bands,
    }
}

/// Splits interleaved samples into one vector per band.
fn deinterleave(raw: impl Iterator<Item = f32>, n: usize, nb: usize) -> Vec<Vec<f32>> {
    let mut bands = vec![Vec::with_capacity(n); nb];
    for (i, v) in raw.enumerate() {
        bands[i % nb].push(v);
    }
    bands
}

// ------------------------------------------------------------------------------------------ PNG

fn load_png(path: &Path) -> Result<Loaded> {
    let (x0, y_top, cell) = world_file(path, &["pgw", "pngw", "wld"])?;
    let mut dec = png::Decoder::new(BufReader::new(crate::open_file(path)?));
    dec.set_transformations(png::Transformations::EXPAND);
    dec.set_limits(png::Limits { bytes: usize::MAX });
    let mut reader = dec
        .read_info()
        .map_err(|e| bad(path, &format!("PNG: {e}")))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| bad(path, "PNG demasiado grande"))?;
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| bad(path, &format!("PNG: {e}")))?;
    let (color, depth) = reader.output_color_type();
    let (cols, rows) = (info.width as usize, info.height as usize);
    let nb_all = color.samples();
    // an alpha channel is not a band
    let nb = match color {
        png::ColorType::GrayscaleAlpha => 1,
        png::ColorType::Rgba => 3,
        _ => nb_all,
    };
    let bytes = if depth == png::BitDepth::Sixteen {
        2
    } else {
        1
    };
    let bits: u8 = if bytes == 2 { 16 } else { 8 };
    let data = &buf[..info.buffer_size()];
    let sample = move |i: usize| -> f32 {
        if bytes == 2 {
            f32::from(u16::from_be_bytes([data[2 * i], data[2 * i + 1]]))
        } else {
            f32::from(data[i])
        }
    };
    let n = cols * rows;
    let mut bands = vec![Vec::with_capacity(n); nb];
    for p in 0..n {
        for (b, band) in bands.iter_mut().enumerate() {
            band.push(sample(p * nb_all + b));
        }
    }
    Ok(Loaded {
        image: image_from(x0, y_top, cell, cols, rows, bands),
        epsg: prj_epsg(path),
        bits,
    })
}

// ----------------------------------------------------------------------------------------- JPEG

fn load_jpeg(path: &Path) -> Result<Loaded> {
    let (x0, y_top, cell) = world_file(path, &["jgw", "jpgw", "jpegw", "wld"])?;
    let data = std::fs::read(path)?;
    let mut dec = zune_jpeg::JpegDecoder::new(Cursor::new(data));
    let px = dec
        .decode()
        .map_err(|e| bad(path, &format!("JPEG: {e:?}")))?;
    let info = dec.info().ok_or_else(|| bad(path, "JPEG sin cabecera"))?;
    let (cols, rows) = (usize::from(info.width), usize::from(info.height));
    let n = cols * rows;
    if n == 0 || px.len() % n != 0 {
        return Err(bad(path, "JPEG: los píxeles no cuadran con el tamaño"));
    }
    let nb = px.len() / n;
    let bands = deinterleave(px.into_iter().map(f32::from), n, nb);
    Ok(Loaded {
        image: image_from(x0, y_top, cell, cols, rows, bands),
        epsg: prj_epsg(path),
        bits: 8,
    })
}

// ------------------------------------------------------------------------------------------ HGT

/// SRTM tiles: big-endian 16-bit metres, 1201 or 3601 samples a side, the name giving the
/// south-west corner (`N37W006.hgt`).
fn load_hgt(path: &Path) -> Result<Loaded> {
    let data = std::fs::read(path)?;
    let side = match data.len() {
        n if n == 1201 * 1201 * 2 => 1201usize,
        n if n == 3601 * 3601 * 2 => 3601,
        n if n == 1801 * 1801 * 2 => 1801,
        _ => {
            return Err(bad(
                path,
                "un .hgt de SRTM mide 1201 o 3601 muestras por lado",
            ));
        }
    };
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_uppercase())
        .unwrap_or_default();
    let name_err = || bad(path, "el nombre debe ser como N37W006 (esquina suroeste)");
    let lat_sign = match stem.chars().next() {
        Some('N') => 1.0,
        Some('S') => -1.0,
        _ => return Err(name_err()),
    };
    let lon_at = stem.find(['E', 'W']).ok_or_else(name_err)?;
    let lat: f64 = stem
        .get(1..lon_at)
        .and_then(|s| s.parse().ok())
        .ok_or_else(name_err)?;
    let lon: f64 = stem
        .get(lon_at + 1..)
        .and_then(|s| s.get(..3))
        .and_then(|s| s.parse().ok())
        .ok_or_else(name_err)?;
    let lon_sign = if stem.as_bytes()[lon_at] == b'W' {
        -1.0
    } else {
        1.0
    };
    let (west, south) = (lon_sign * lon, lat_sign * lat);
    // samples sit on the degree lines: the tile spans one degree between the first and last
    let cell = 1.0 / (side - 1) as f64;
    let band: Vec<f32> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| {
            let v = i16::from_be_bytes([b[0], b[1]]);
            if v == -32768 { f32::NAN } else { f32::from(v) }
        })
        .collect();
    Ok(Loaded {
        image: image_from(
            west - cell / 2.0,
            south + 1.0 + cell / 2.0,
            cell,
            side,
            side,
            vec![band],
        ),
        epsg: Some(4326),
        bits: 16,
    })
}

// ------------------------------------------------------------------------------- ENVI / ESRI

#[derive(Default)]
struct Header {
    cols: usize,
    rows: usize,
    bands: usize,
    /// ENVI data type code.
    dtype: u32,
    interleave: String,
    big_endian: bool,
    offset: usize,
    nodata: Option<f64>,
    /// West edge, north edge, cell size.
    grid: Option<(f64, f64, f64)>,
    epsg: Option<u16>,
}

fn parse_header(path: &Path) -> Result<Header> {
    let text = std::fs::read_to_string(path)?;
    let mut h = Header {
        bands: 1,
        interleave: "bsq".into(),
        ..Header::default()
    };
    let esri = !text.trim_start().to_ascii_uppercase().starts_with("ENVI");
    // `key = value`; a value in braces may run over several lines
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let sep = if esri {
            line.find(char::is_whitespace)
        } else {
            line.find('=')
        };
        let Some(i) = sep else { continue };
        let key = line[..i].trim().to_ascii_lowercase();
        let mut val = line[i + 1..].trim().to_string();
        if val.starts_with('{') {
            while !val.contains('}') {
                match lines.next() {
                    Some(l) => val.push_str(l.trim()),
                    None => break,
                }
            }
        }
        pairs.push((key, val.trim_matches(['{', '}']).trim().to_string()));
    }
    let get = |k: &str| pairs.iter().find(|p| p.0 == k).map(|p| p.1.as_str());
    let num = |k: &str| get(k).and_then(|v| v.trim().parse::<f64>().ok());
    if esri {
        h.cols = num("ncols").unwrap_or(0.0) as usize;
        h.rows = num("nrows").unwrap_or(0.0) as usize;
        h.bands = num("nbands").unwrap_or(1.0) as usize;
        let bits = num("nbits").unwrap_or(32.0) as u32;
        let float = get("pixeltype").is_some_and(|p| p.eq_ignore_ascii_case("float"))
            || bits == 32 && get("pixeltype").is_none();
        let signed =
            get("pixeltype").is_none_or(|p| !p.to_ascii_lowercase().starts_with("unsigned"));
        h.dtype = match (float, bits, signed) {
            (true, 64, _) => 5,
            (true, _, _) => 4,
            (_, 8, _) => 1,
            (_, 16, true) => 2,
            (_, 16, false) => 12,
            (_, 32, true) => 3,
            (_, 32, false) => 13,
            _ => 0,
        };
        h.interleave = get("layout").unwrap_or("bil").to_ascii_lowercase();
        h.big_endian = get("byteorder").is_some_and(|b| b.to_ascii_uppercase().starts_with('M'));
        h.nodata = num("nodata_value").or_else(|| num("nodata"));
        let cell = num("cellsize").or_else(|| num("xdim"));
        if let (Some(c), Some(x), Some(y)) = (
            cell,
            num("xllcorner").or_else(|| num("ulxmap").map(|u| u - cell.unwrap_or(0.0) / 2.0)),
            num("yllcorner"),
        ) {
            h.grid = Some((x, y + h.rows as f64 * c, c));
        } else if let (Some(c), Some(ux), Some(uy)) = (cell, num("ulxmap"), num("ulymap")) {
            h.grid = Some((ux - c / 2.0, uy + c / 2.0, c));
        }
    } else {
        h.cols = num("samples").unwrap_or(0.0) as usize;
        h.rows = num("lines").unwrap_or(0.0) as usize;
        h.bands = num("bands").unwrap_or(1.0) as usize;
        h.dtype = num("data type").unwrap_or(0.0) as u32;
        h.interleave = get("interleave").unwrap_or("bsq").to_ascii_lowercase();
        h.big_endian = num("byte order").unwrap_or(0.0) as u32 == 1;
        h.offset = num("header offset").unwrap_or(0.0) as usize;
        h.nodata = num("data ignore value");
        if let Some(m) = get("map info") {
            let f: Vec<&str> = m.split(',').map(str::trim).collect();
            let n = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
            if let (Some(rx), Some(ry), Some(ex), Some(ny), Some(px)) =
                (n(1), n(2), n(3), n(4), n(5))
            {
                let py = n(6).unwrap_or(px);
                if (px - py).abs() > 1e-6 * px {
                    return Err(Error::Unsupported("ENVI: píxeles no cuadrados".into()));
                }
                // the reference pixel is 1-based and its corner is meant
                h.grid = Some((ex - (rx - 1.0) * px, ny + (ry - 1.0) * px, px));
                let kind = f
                    .first()
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_default();
                let datum = f
                    .iter()
                    .rev()
                    .find(|s| {
                        s.contains("WGS")
                            || s.contains("ETRS")
                            || s.contains("ED50")
                            || s.contains("NAD83")
                    })
                    .map(|s| s.to_ascii_uppercase())
                    .unwrap_or_default();
                if kind.starts_with("geographic") {
                    h.epsg = Some(if datum.contains("ETRS") { 4258 } else { 4326 });
                } else if kind.starts_with("utm") {
                    let zone = n(7).map(|z| z as u16);
                    let south = f
                        .get(8)
                        .is_some_and(|s| s.to_ascii_lowercase().starts_with('s'));
                    h.epsg = zone.and_then(|z| {
                        if datum.contains("WGS") {
                            Some(if south { 32700 } else { 32600 } + z)
                        } else if datum.contains("ETRS") && !south {
                            Some(25800 + z)
                        } else if datum.contains("ED50") && !south {
                            Some(23000 + z)
                        } else {
                            None
                        }
                    });
                }
            }
        }
        if h.epsg.is_none()
            && let Some(cs) = get("coordinate system string")
        {
            h.epsg = crate::vector::epsg_from_prj(cs);
        }
    }
    Ok(h)
}

fn load_binary(path: &Path) -> Result<Loaded> {
    let hdr = header_of(path).ok_or_else(|| bad(path, "falta el archivo .hdr"))?;
    let h = parse_header(&hdr)?;
    let (cols, rows, nb) = (h.cols, h.rows, h.bands.max(1));
    if cols == 0 || rows == 0 {
        return Err(bad(&hdr, "faltan las dimensiones de la imagen"));
    }
    let (size, bits) = match h.dtype {
        1 => (1, 8),
        2 | 12 => (2, 16),
        3 | 13 | 4 => (4, 32),
        5 | 14 | 15 => (8, 64),
        t => {
            return Err(Error::Unsupported(format!(
                "{}: tipo de dato {t} no soportado",
                hdr.display()
            )));
        }
    };
    if matches!(h.dtype, 14 | 15) {
        return Err(Error::Unsupported(format!(
            "{}: no se admiten enteros de 64 bits",
            hdr.display()
        )));
    }
    let n = cols * rows;
    let need = h.offset + n * nb * size;
    let mut data = Vec::new();
    crate::open_file(path)?.read_to_end(&mut data)?;
    if data.len() < need {
        return Err(bad(path, "el archivo es más corto de lo que dice el .hdr"));
    }
    let data = &data[h.offset..need];
    let be = h.big_endian;
    let at = |i: usize| -> f32 {
        let b = &data[i * size..(i + 1) * size];
        match (h.dtype, size) {
            (1, _) => f32::from(b[0]),
            (2, _) => f32::from(if be {
                i16::from_be_bytes([b[0], b[1]])
            } else {
                i16::from_le_bytes([b[0], b[1]])
            }),
            (12, _) => f32::from(if be {
                u16::from_be_bytes([b[0], b[1]])
            } else {
                u16::from_le_bytes([b[0], b[1]])
            }),
            (3, _) => {
                (if be {
                    i32::from_be_bytes([b[0], b[1], b[2], b[3]])
                } else {
                    i32::from_le_bytes([b[0], b[1], b[2], b[3]])
                }) as f32
            }
            (13, _) => {
                (if be {
                    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
                } else {
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                }) as f32
            }
            (4, _) => {
                if be {
                    f32::from_be_bytes([b[0], b[1], b[2], b[3]])
                } else {
                    f32::from_le_bytes([b[0], b[1], b[2], b[3]])
                }
            }
            _ => {
                let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
                (if be {
                    f64::from_be_bytes(a)
                } else {
                    f64::from_le_bytes(a)
                }) as f32
            }
        }
    };
    let nodata = h.nodata.map(|v| v as f32);
    let clean = |v: f32| {
        if Some(v) == nodata || !v.is_finite() {
            f32::NAN
        } else {
            v
        }
    };
    let mut bands = vec![vec![0f32; n]; nb];
    for (b, band) in bands.iter_mut().enumerate() {
        for y in 0..rows {
            for x in 0..cols {
                let idx = match h.interleave.as_str() {
                    "bip" => (y * cols + x) * nb + b,
                    "bil" => (y * nb + b) * cols + x,
                    _ => b * n + y * cols + x,
                };
                band[y * cols + x] = clean(at(idx));
            }
        }
    }
    let (x0, y_top, cell) = h.grid.ok_or_else(|| {
        bad(
            &hdr,
            "imagen sin georreferenciar (falta «map info» o sus equivalentes)",
        )
    })?;
    Ok(Loaded {
        image: image_from(x0, y_top, cell, cols, rows, bands),
        epsg: h.epsg.or_else(|| prj_epsg(path)),
        bits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("easylidar-rasterfmt-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap_or_default();
        d
    }

    #[test]
    fn png_with_a_world_file_is_placed_on_the_ground() -> Result<()> {
        let dir = scratch("png");
        let p = dir.join("a.png");
        let mut enc = png::Encoder::new(File::create(&p)?, 3, 2);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc
            .write_header()
            .map_err(|e| Error::Format(e.to_string()))?;
        w.write_image_data(&[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
        ])
        .map_err(|e| Error::Format(e.to_string()))?;
        drop(w);
        std::fs::write(p.with_extension("pgw"), "2\n0\n0\n-2\n1001\n1999\n")?;
        let l = load(&p)?.ok_or_else(|| Error::Format("no PNG".into()))?;
        assert_eq!((l.image.cols, l.image.rows, l.image.bands.len()), (3, 2, 3));
        assert_eq!(l.image.x0, 1000.0);
        assert_eq!(l.image.y0, 1996.0);
        assert_eq!(l.image.bands[0][1], 4.0);
        assert_eq!(l.image.bands[2][5], 18.0);
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn envi_bil_with_map_info() -> Result<()> {
        let dir = scratch("envi");
        let p = dir.join("a.bil");
        // 2 columns, 2 rows, 2 bands, 16-bit little-endian, line-interleaved
        let vals: [i16; 8] = [1, 2, 10, 20, 3, 4, 30, 40];
        std::fs::write(
            &p,
            vals.iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>(),
        )?;
        std::fs::write(
            dir.join("a.hdr"),
            "ENVI\nsamples = 2\nlines = 2\nbands = 2\ndata type = 2\ninterleave = bil\nbyte order = 0\nmap info = {UTM, 1, 1, 500000, 4000000, 10, 10, 30, North, WGS-84}\n",
        )?;
        let l = load(&p)?.ok_or_else(|| Error::Format("no ENVI".into()))?;
        assert_eq!(l.epsg, Some(32630));
        assert_eq!(l.image.x0, 500000.0);
        assert_eq!(l.image.y0, 4000000.0 - 20.0);
        assert_eq!(l.image.bands[0], vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(l.image.bands[1], vec![10.0, 20.0, 30.0, 40.0]);
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn hgt_is_placed_by_its_name_and_voids_are_empty() -> Result<()> {
        let dir = scratch("hgt");
        let p = dir.join("N37W006.hgt");
        let mut b = vec![0u8; 1201 * 1201 * 2];
        b[0..2].copy_from_slice(&500i16.to_be_bytes());
        b[2..4].copy_from_slice(&(-32768i16).to_be_bytes());
        std::fs::write(&p, b)?;
        let l = load(&p)?.ok_or_else(|| Error::Format("no HGT".into()))?;
        assert_eq!(l.epsg, Some(4326));
        assert!(l.image.bands[0][1].is_nan());
        assert_eq!(l.image.bands[0][0], 500.0);
        assert!((l.image.x0 + 6.0 + 1.0 / 2400.0).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
