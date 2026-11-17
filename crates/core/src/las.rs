//! LAS 1.0–1.4 y LAZ, leídos como flujo: los puntos se decodifican lote a lote a partir de sus
//! registros en bruto y se entregan a una función de retorno, de modo que la memoria nunca crece
//! con el tamaño de la nube.
//!
//! La cabecera y la disposición de los registros siguen el enfoque del lector LiDAR `rustgis-data`
//! de RustGIS (Apache-2.0, https://github.com/opengeos/RustGIS).

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::{Error, Result};

/// El registro más pequeño de cada formato de punto.
const RECORD_LEN: [u16; 11] = [20, 28, 26, 34, 57, 63, 30, 36, 38, 59, 67];

/// Puntos decodificados por lote (LAZ: redondeados hacia arriba a fragmentos enteros por el
/// descompresor).
const BATCH: usize = 1 << 16;

/// Máximo de puntos LAZ descomprimidos a la vez (en paralelo): unos 100 MB de registros como mucho.
const MAX_LAZ_BATCH: usize = 1 << 21;

/// Registros de longitud variable leídos, y el máximo de bytes que puede contener uno.
const MAX_VLRS: u32 = 1000;
const MAX_VLR_BYTES: u64 = 1 << 20;

/// What a LAS header says.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub version: (u8, u8),
    pub format: u8,
    pub compressed: bool,
    pub record_len: u16,
    pub point_offset: u32,
    pub count: u64,
    pub scale: [f64; 3],
    pub offset: [f64; 3],
    /// min x, min y, min z, max x, max y, max z.
    pub bounds: [f64; 6],
    /// El código EPSG proyectado (o, si no, geográfico) de las claves GeoTIFF, cuando se da.
    pub epsg: Option<u16>,
    /// El código EPSG de la referencia vertical (alturas), cuando el archivo declara una.
    pub vertical_epsg: Option<u16>,
}

/// Un punto: lo que las herramientas forestales necesitan de un registro.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub intensity: u16,
    pub return_number: u8,
    pub number_of_returns: u8,
    pub class: u8,
    pub withheld: bool,
    /// Ángulo de barrido en grados, positivo hacia un lado de la pasada (0 cuando el archivo no lo
    /// tiene).
    pub scan_angle: f32,
    /// La pasada (ID de fuente del punto; 0 cuando el archivo no la tiene).
    pub source_id: u16,
}

impl Point {
    /// El ruido (clases 7 y 18) y los puntos retenidos no participan en ningún producto.
    pub fn is_noise(&self) -> bool {
        self.withheld || self.class == 7 || self.class == 18
    }

    pub fn is_first(&self) -> bool {
        self.return_number <= 1
    }
}

/// Lo que dice un archivo de sí mismo además de lo que necesitan las herramientas: para informes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub file_source_id: u16,
    pub global_encoding: u16,
    pub system_id: String,
    pub software: String,
    /// Día del año y año de creación.
    pub creation: (u16, u16),
    pub header_size: u16,
    /// Puntos por número de retorno 1–5 como los cuenta la cabecera (LAS 1.4: hasta 15).
    pub by_return: Vec<u64>,
    pub vlrs: Vec<VlrInfo>,
    /// El sistema de coordenadas como WKT, cuando el archivo lo lleva.
    pub wkt: Option<String>,
    pub file_size: u64,
}

/// Un registro de longitud variable del bloque de cabecera.
#[derive(Clone, Debug, PartialEq)]
pub struct VlrInfo {
    pub user: String,
    pub record: u16,
    pub len: u64,
    pub description: String,
}

/// Campos más allá de [`Point`], leídos del mismo registro.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Extra {
    /// La pasada (ID de fuente del punto).
    pub source_id: u16,
    pub user_data: u8,
    /// Ángulo de barrido en grados (con signo).
    pub scan_angle: f32,
    pub gps_time: f64,
    /// Color de 16 bits, cuando el formato lo tiene.
    pub rgb: Option<[u16; 3]>,
    pub synthetic: bool,
    pub key_point: bool,
    pub overlap: bool,
}

/// La longitud de los campos del formato de punto `format`, sin atributos adicionales.
pub(crate) fn core_len(format: u8) -> Option<u16> {
    RECORD_LEN.get(usize::from(format)).copied()
}

/// Dónde guarda cada formato sus campos opcionales: tiempo GPS y luego color.
fn gps_at(format: u8) -> Option<usize> {
    match format {
        1 | 3 | 4 | 5 => Some(20),
        6..=10 => Some(22),
        _ => None,
    }
}

fn rgb_at(format: u8) -> Option<usize> {
    match format {
        2 => Some(20),
        3 | 5 => Some(28),
        7 | 8 | 10 => Some(30),
        _ => None,
    }
}

/// Decodifica los campos más allá de [`Point`] de un registro.
pub(crate) fn decode_extra(h: &Header, rec: &[u8]) -> Extra {
    let ext = h.format >= 6;
    let (source_id, user_data, scan_angle) = if ext {
        (
            u16_at(rec, 20).unwrap_or(0),
            rec.get(17).copied().unwrap_or(0),
            // Los registros extendidos cuentan el ángulo en pasos de 0,006 grados.
            f32::from(u16_at(rec, 18).unwrap_or(0) as i16) * 0.006,
        )
    } else {
        (
            u16_at(rec, 18).unwrap_or(0),
            rec.get(17).copied().unwrap_or(0),
            f32::from(rec.get(16).copied().unwrap_or(0) as i8),
        )
    };
    let flags = rec.get(15).copied().unwrap_or(0);
    let (synthetic, key_point, overlap) = if ext {
        (flags & 1 != 0, flags & 2 != 0, flags & 8 != 0)
    } else {
        (flags & 0x20 != 0, flags & 0x40 != 0, false)
    };
    Extra {
        source_id,
        user_data,
        scan_angle,
        gps_time: gps_at(h.format)
            .and_then(|at| f64_at(rec, at))
            .unwrap_or(0.0),
        rgb: rgb_at(h.format)
            .and_then(|at| Some([u16_at(rec, at)?, u16_at(rec, at + 2)?, u16_at(rec, at + 4)?])),
        synthetic,
        key_point,
        overlap,
    }
}

/// Escribe coordenadas en un registro con la escala y el desplazamiento de la cabecera; falso
/// cuando una queda fuera de lo que cabe en una coordenada de 32 bits.
pub fn set_xyz(h: &Header, rec: &mut [u8], x: f64, y: f64, z: f64) -> bool {
    for (k, v) in [x, y, z].into_iter().enumerate() {
        let q = ((v - h.offset[k]) / h.scale[k]).round();
        if !q.is_finite() || q.abs() > f64::from(i32::MAX) {
            return false;
        }
        match rec.get_mut(4 * k..4 * k + 4) {
            Some(s) => s.copy_from_slice(&(q as i32).to_le_bytes()),
            None => return false,
        }
    }
    true
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2)
        .and_then(|s| s.try_into().ok())
        .map(u16::from_le_bytes)
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
}
fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    b.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map(i32::from_le_bytes)
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
}
fn f64_at(b: &[u8], at: usize) -> Option<f64> {
    b.get(at..at + 8)
        .and_then(|s| s.try_into().ok())
        .map(f64::from_le_bytes)
}

fn bad(what: &str) -> Error {
    Error::Format(format!("no es un archivo LAS legible ({what})"))
}

/// Lee el bloque de cabecera (sus primeros 375 bytes como mucho).
fn parse_header(b: &[u8]) -> Result<Header> {
    if !b.starts_with(b"LASF") {
        return Err(bad("no empieza por la firma LASF"));
    }
    let version = (
        *b.get(24).ok_or_else(|| bad("versión"))?,
        *b.get(25).ok_or_else(|| bad("versión"))?,
    );
    if version.0 != 1 || version.1 > 4 {
        return Err(Error::Unsupported(format!(
            "LAS {}.{}",
            version.0, version.1
        )));
    }
    let header_size = u16_at(b, 94).ok_or_else(|| bad("tamaño de la cabecera"))?;
    let point_offset = u32_at(b, 96).ok_or_else(|| bad("posición de los puntos"))?;
    let raw_format = *b.get(104).ok_or_else(|| bad("formato de punto"))?;
    // LAZ marca sus formatos con los bits altos.
    let compressed = raw_format & 0xC0 != 0;
    let format = raw_format & 0x3F;
    let min_len = *RECORD_LEN
        .get(usize::from(format))
        .ok_or_else(|| Error::Unsupported(format!("formato de punto LAS {format}")))?;
    let record_len = u16_at(b, 105).ok_or_else(|| bad("longitud de registro"))?;
    if record_len < min_len {
        return Err(bad("registros más cortos que su formato"));
    }
    let legacy = u64::from(u32_at(b, 107).ok_or_else(|| bad("número de puntos"))?);
    let count = if version.1 >= 4 && header_size >= 375 {
        u64_at(b, 247).filter(|n| *n > 0).unwrap_or(legacy)
    } else {
        legacy
    };
    let f = |at| {
        f64_at(b, at)
            .filter(|x| x.is_finite())
            .ok_or_else(|| bad("escala, desplazamiento o extensión"))
    };
    let scale = [f(131)?, f(139)?, f(147)?];
    if scale.contains(&0.0) {
        return Err(bad("una escala nula"));
    }
    let offset = [f(155)?, f(163)?, f(171)?];
    // En disco: x máxima, x mínima, y máxima, y mínima, z máxima, z mínima.
    let bounds = [f(187)?, f(203)?, f(219)?, f(179)?, f(195)?, f(211)?];
    if u32::from(header_size) > point_offset || header_size < 227 {
        return Err(bad("tamaño de la cabecera"));
    }
    Ok(Header {
        version,
        format,
        compressed,
        record_len,
        point_offset,
        count,
        scale,
        offset,
        bounds,
        epsg: None,
        vertical_epsg: None,
    })
}

struct Vlr {
    user: String,
    record: u16,
    data: Vec<u8>,
}

/// Los registros que usa este lector (claves GeoTIFF, WKT, los de LASzip) y una lista de todos
/// ellos.
fn read_vlrs<R: Read + Seek>(
    r: &mut R,
    at: u64,
    n: u32,
    end: u64,
) -> Result<(Vec<Vlr>, Vec<VlrInfo>)> {
    let mut out = Vec::new();
    let mut all = Vec::new();
    let mut pos = at;
    for _ in 0..n.min(MAX_VLRS) {
        let mut h = [0u8; 54];
        r.seek(SeekFrom::Start(pos))?;
        if r.read_exact(&mut h).is_err() {
            break;
        }
        let text = |a: usize, b: usize| {
            String::from_utf8_lossy(h.get(a..b).unwrap_or(&[]))
                .trim_end_matches('\0')
                .trim()
                .to_string()
        };
        let user = text(2, 18);
        let record = u16_at(&h, 18).unwrap_or(0);
        let len = u64::from(u16_at(&h, 20).unwrap_or(0));
        let data_at = pos + 54;
        if data_at.saturating_add(len) > end {
            break;
        }
        all.push(VlrInfo {
            user: user.clone(),
            record,
            len,
            description: text(22, 54),
        });
        let wanted = (user == "LASF_Projection" && matches!(record, 34735 | 2111 | 2112))
            || (user == "laszip encoded" && record == 22204);
        if wanted && len <= MAX_VLR_BYTES {
            let mut data = vec![0u8; len as usize];
            r.read_exact(&mut data)?;
            out.push(Vlr { user, record, data });
        }
        pos = data_at.saturating_add(len);
    }
    Ok((out, all))
}

/// La parte descriptiva del bloque de cabecera.
fn parse_meta(b: &[u8]) -> Meta {
    let text = |a: usize, z: usize| {
        String::from_utf8_lossy(b.get(a..z).unwrap_or(&[]))
            .split('\0')
            .next()
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let version_minor = b.get(25).copied().unwrap_or(0);
    let by_return = if version_minor >= 4 && b.len() >= 375 {
        (0..15).filter_map(|k| u64_at(b, 255 + 8 * k)).collect()
    } else {
        (0..5)
            .filter_map(|k| u32_at(b, 111 + 4 * k).map(u64::from))
            .collect()
    };
    Meta {
        file_source_id: if version_minor >= 1 {
            u16_at(b, 4).unwrap_or(0)
        } else {
            0
        },
        global_encoding: if version_minor >= 2 {
            u16_at(b, 6).unwrap_or(0)
        } else {
            0
        },
        system_id: text(26, 58),
        software: text(58, 90),
        creation: (u16_at(b, 90).unwrap_or(0), u16_at(b, 92).unwrap_or(0)),
        header_size: u16_at(b, 94).unwrap_or(0),
        by_return,
        ..Meta::default()
    }
}

/// El texto del primer bloque entre corchetes de un WKT que empieza con `key` (`VERT_CS[` ...).
fn wkt_block<'a>(wkt: &'a str, keys: &[&str]) -> Option<&'a str> {
    let start = keys.iter().filter_map(|k| wkt.find(k)).min()?;
    let mut depth = 0i32;
    let mut quoted = false;
    for (i, c) in wkt[start..].char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' if !quoted => depth += 1,
            ']' | ')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some(&wkt[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

const VERTICAL_KEYS: [&str; 3] = ["VERT_CS[", "VERTCRS[", "VERTICALCRS["];

/// El código EPSG del sistema vertical de un WKT compuesto.
pub fn vertical_epsg_from_wkt(wkt: &str) -> Option<u16> {
    epsg_from_wkt_plain(wkt_block(wkt, &VERTICAL_KEYS)?)
}

/// El código EPSG de un WKT: para uno compuesto, el de su parte horizontal.
pub fn epsg_from_wkt(wkt: &str) -> Option<u16> {
    if wkt_block(wkt, &VERTICAL_KEYS).is_some() {
        let h = wkt_block(wkt, &["PROJCS[", "PROJCRS[", "PROJECTEDCRS["])
            .or_else(|| wkt_block(wkt, &["GEOGCS[", "GEOGCRS[", "GEODCRS[", "GEOGRAPHICCRS["]))?;
        return epsg_from_wkt_plain(h);
    }
    epsg_from_wkt_plain(wkt)
}

/// El código EPSG con que termina un WKT (la autoridad más externa), al estilo de WKT1 o de WKT2.
fn epsg_from_wkt_plain(wkt: &str) -> Option<u16> {
    let mut best = None;
    for pat in ["AUTHORITY[\"EPSG\",\"", "ID[\"EPSG\","] {
        let mut from = 0;
        while let Some(i) = wkt.get(from..).and_then(|t| t.find(pat)) {
            let at = from + i + pat.len();
            let digits: String = wkt
                .get(at..)
                .unwrap_or("")
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(code) = digits.parse::<u16>() {
                best = match best {
                    Some((pos, _)) if pos > at => best,
                    _ => Some((at, code)),
                };
            }
            from = at;
        }
    }
    best.map(|(_, code)| code)
}

/// El nombre de un sistema de coordenadas a partir de su WKT: la primera cadena entrecomillada.
pub fn wkt_name(wkt: &str) -> Option<String> {
    let start = wkt.find('"')? + 1;
    let len = wkt.get(start..)?.find('"')?;
    wkt.get(start..start + len).map(str::to_string)
}

/// VerticalCSTypeGeoKey (4096) del directorio de claves.
fn vertical_epsg_of(vlrs: &[Vlr]) -> Option<u16> {
    let keys = vlrs
        .iter()
        .find(|v| v.user == "LASF_Projection" && v.record == 34735)?;
    let n = usize::from(u16_at(&keys.data, 6)?).min(256);
    for k in 0..n {
        let at = 8 + k * 8;
        let (Some(id), Some(loc), Some(val)) = (
            u16_at(&keys.data, at),
            u16_at(&keys.data, at + 2),
            u16_at(&keys.data, at + 6),
        ) else {
            break;
        };
        if id == 4096 && loc == 0 && val != 32767 && val != 0 {
            return Some(val);
        }
    }
    None
}

/// ProjectedCSTypeGeoKey, else GeographicTypeGeoKey; 32767 means "user defined".
fn epsg_of(vlrs: &[Vlr]) -> Option<u16> {
    let keys = vlrs
        .iter()
        .find(|v| v.user == "LASF_Projection" && v.record == 34735)?;
    let n = usize::from(u16_at(&keys.data, 6)?).min(256);
    let mut code = None;
    for k in 0..n {
        let at = 8 + k * 8;
        let (Some(id), Some(loc), Some(val)) = (
            u16_at(&keys.data, at),
            u16_at(&keys.data, at + 2),
            u16_at(&keys.data, at + 6),
        ) else {
            break;
        };
        if loc == 0 && val != 32767 && val != 0 && (id == 3072 || (id == 2048 && code.is_none())) {
            code = Some(val);
        }
    }
    code
}

/// Decodifica un registro (`rec` contiene al menos la longitud mínima del formato).
#[inline]
pub(crate) fn decode(h: &Header, rec: &[u8]) -> Option<Point> {
    let c = |i: usize| -> Option<f64> {
        Some(f64::from(i32_at(rec, i * 4)?) * h.scale[i] + h.offset[i])
    };
    let intensity = u16_at(rec, 12)?;
    let (return_number, number_of_returns, class, withheld) = if h.format >= 6 {
        let b14 = *rec.get(14)?;
        let flags = *rec.get(15)?;
        (b14 & 0x0F, b14 >> 4, *rec.get(16)?, flags & 0x04 != 0)
    } else {
        let b14 = *rec.get(14)?;
        let b15 = *rec.get(15)?;
        (b14 & 0x07, (b14 >> 3) & 0x07, b15 & 0x1F, b15 & 0x80 != 0)
    };
    // los formatos antiguos guardan grados enteros; los formatos 1.4, pasos de 0,006 grados
    let scan_angle = if h.format >= 6 {
        f32::from(u16_at(rec, 18).unwrap_or(0) as i16) * 0.006
    } else {
        f32::from(rec.get(16).copied().unwrap_or(0) as i8)
    };
    Some(Point {
        x: c(0)?,
        y: c(1)?,
        z: c(2)?,
        intensity,
        return_number,
        number_of_returns,
        class,
        withheld,
        scan_angle,
        source_id: if h.format >= 6 {
            u16_at(rec, 20).unwrap_or(0)
        } else {
            u16_at(rec, 18).unwrap_or(0)
        },
    })
}

/// La fecha de una marca de tiempo GPS como `AAAA-MM-DD`. El tiempo GPS estándar ajustado (bit 0 de
/// la codificación global) cuenta segundos desde 1980-01-06 menos 1e9; un tiempo de menos de una
/// semana es un tiempo de semana y no lleva fecha, así que la sustituye la fecha de creación del
/// archivo `(día del año, año)`. Un tiempo estándar (sin ajustar) por debajo de 4e8 s situaría el
/// vuelo antes de 1993, así que se lee como ajustado.
pub fn gps_date(t: f64, global_encoding: u16, creation: (u16, u16)) -> Option<String> {
    let days_from_civil = |y: i64, m: i64, d: i64| {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    };
    let mut days = if t.is_finite() && (global_encoding & 1 != 0 || t > 604_800.0 * 1.01) {
        // los archivos antiguos (LAS 1.0) guardan tiempo ajustado sin el indicador: ningún vuelo es
        // anterior a 1992
        let secs = if global_encoding & 1 != 0 || t < 4e8 {
            t + 1e9
        } else {
            t
        };
        // 1980-01-06, la época GPS, es el día 3657 de la cuenta de Unix
        3657 + (secs / 86_400.0).floor() as i64
    } else if creation.1 > 1900 && creation.0 >= 1 {
        days_from_civil(i64::from(creation.1), 1, 1) + i64::from(creation.0) - 1
    } else {
        return None;
    };
    // Una fecha futura no puede ser la del vuelo: algunos escáneres (GeoSLAM entre ellos) marcan
    // los puntos con tiempo Unix, que leído como tiempo GPS cae años por delante.
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(i64::MAX, |d| (d.as_secs() / 86_400) as i64);
    if t.is_finite() && t > 604_800.0 * 1.01 && days > today.saturating_add(1) {
        let unix = (t / 86_400.0).floor() as i64;
        if unix <= today.saturating_add(1) {
            days = unix;
        }
    }
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Fija la clasificación de un registro.
pub fn set_class(format: u8, rec: &mut [u8], class: u8) {
    if format >= 6 {
        if let Some(b) = rec.get_mut(16) {
            *b = class;
        }
    } else if let Some(b) = rec.get_mut(15) {
        *b = (*b & 0xE0) | (class & 0x1F);
    }
}

/// Marca un registro como retenido (withheld): el bit 0x80 del byte 15 en los formatos antiguos,
/// el bit 0x04 de las banderas (byte 15) en los formatos 6 y superiores.
pub fn set_withheld(format: u8, rec: &mut [u8]) {
    if let Some(b) = rec.get_mut(15) {
        *b |= if format >= 6 { 0x04 } else { 0x80 };
    }
}

/// Fija el identificador de fuente de punto de un registro (la pasada): bytes 18-19 en los formatos
/// antiguos, 20-21 en los formatos 6 y superiores.
pub fn set_source_id(format: u8, rec: &mut [u8], id: u16) {
    let at = if format >= 6 { 20 } else { 18 };
    if let Some(b) = rec.get_mut(at..at + 2) {
        b.copy_from_slice(&id.to_le_bytes());
    }
}

/// Fija el byte de datos de usuario de un registro (byte 17 en todos los formatos).
pub fn set_user_data(rec: &mut [u8], value: u8) {
    if let Some(b) = rec.get_mut(17) {
        *b = value;
    }
}

/// El formato de punto LAS 1.4 que conserva lo que conserva un formato antiguo `format` y tiene un
/// byte de clase completo (las clases por encima de 31 no caben en los 5 bits de los formatos 0–5).
/// `None`: aquí no hay tal formato.
pub fn upgraded_format(format: u8) -> Option<u8> {
    match format {
        0 | 1 => Some(6),
        2 | 3 => Some(7),
        6..=10 => Some(format),
        _ => None,
    }
}

/// Convierte el registro de un formato antiguo `old` al formato 1.4 `new` (6 o 7); se conservan los
/// bytes posteriores a los campos propios del formato (atributos adicionales). Los ángulos de
/// barrido pasan de grados enteros a pasos de 0,006°.
pub fn upgrade_record(old: u8, new: u8, rec: &[u8], out: &mut Vec<u8>) {
    out.clear();
    let at = |i: usize| rec.get(i).copied().unwrap_or(0);
    out.extend((0..14).map(at));
    let (b14, b15) = (at(14), at(15));
    out.push((b14 & 7) | (((b14 >> 3) & 7) << 4));
    out.push(
        ((b15 >> 5) & 1)
            | (((b15 >> 6) & 1) << 1)
            | (((b15 >> 7) & 1) << 2)
            | (((b14 >> 6) & 1) << 6)
            | ((b14 >> 7) << 7),
    );
    out.push(b15 & 0x1F);
    out.push(at(17));
    let angle = i32::from(at(16) as i8) * 1000 / 6;
    out.extend((angle as i16).to_le_bytes());
    out.extend([at(18), at(19)]);
    match gps_at(old) {
        Some(g) => out.extend((g..g + 8).map(at)),
        None => out.extend([0; 8]),
    }
    if new == 7 {
        match rgb_at(old) {
            Some(g) => out.extend((g..g + 6).map(at)),
            None => out.extend([0; 6]),
        }
    }
    let old_core = usize::from(RECORD_LEN.get(usize::from(old)).copied().unwrap_or(0));
    out.extend((old_core..rec.len()).map(at));
}

/// Cualquier cosa que se lee como flujo de puntos: un archivo o una tesela en memoria.
pub trait Cloud {
    /// min x, min y, min z, max x, max y, max z.
    fn bounds(&self) -> [f64; 6];
    fn scan<F: FnMut(&Point)>(&self, f: F) -> Result<u64>;
}

impl Cloud for Reader {
    fn bounds(&self) -> [f64; 6] {
        self.header.bounds
    }
    fn scan<F: FnMut(&Point)>(&self, f: F) -> Result<u64> {
        Reader::scan(self, f)
    }
}

/// Puntos en memoria, con su extensión.
pub struct Points {
    pub pts: Vec<Point>,
    pub bounds: [f64; 6],
}

impl Points {
    pub fn new(pts: Vec<Point>) -> Self {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in &pts {
            b = [
                b[0].min(p.x),
                b[1].min(p.y),
                b[2].min(p.z),
                b[3].max(p.x),
                b[4].max(p.y),
                b[5].max(p.z),
            ];
        }
        if pts.is_empty() {
            b = [0.0; 6];
        }
        Self { pts, bounds: b }
    }
}

impl Cloud for Points {
    fn bounds(&self) -> [f64; 6] {
        self.bounds
    }
    fn scan<F: FnMut(&Point)>(&self, mut f: F) -> Result<u64> {
        self.pts.iter().for_each(&mut f);
        Ok(self.pts.len() as u64)
    }
}

/// Dónde está un atributo adicional en un registro y su código de tipo (tabla de LAS 1.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtraField {
    pub at: usize,
    pub code: u8,
}

impl ExtraField {
    /// El valor en `rec`.
    pub fn get(&self, rec: &[u8]) -> Option<f64> {
        let b = rec.get(self.at..)?;
        Some(match self.code {
            1 => f64::from(*b.first()?),
            2 => f64::from(*b.first()? as i8),
            3 => f64::from(u16::from_le_bytes(b.get(..2)?.try_into().ok()?)),
            4 => f64::from(i16::from_le_bytes(b.get(..2)?.try_into().ok()?)),
            5 => f64::from(u32::from_le_bytes(b.get(..4)?.try_into().ok()?)),
            6 => f64::from(i32::from_le_bytes(b.get(..4)?.try_into().ok()?)),
            7 => u64::from_le_bytes(b.get(..8)?.try_into().ok()?) as f64,
            8 => i64::from_le_bytes(b.get(..8)?.try_into().ok()?) as f64,
            9 => f64::from(f32::from_le_bytes(b.get(..4)?.try_into().ok()?)),
            10 => f64::from_le_bytes(b.get(..8)?.try_into().ok()?),
            _ => return None,
        })
    }

    /// El valor como identificador: entero y positivo; si no, 0.
    pub fn id(&self, rec: &[u8]) -> u32 {
        self.get(rec)
            .filter(|v| v.is_finite() && *v > 0.0)
            .map_or(0, |v| v.round().min(f64::from(u32::MAX)) as u32)
    }
}

/// Un archivo LAS o LAZ abierto.
pub struct Reader {
    path: std::path::PathBuf,
    pub header: Header,
    laz_vlr: Option<Vec<u8>>,
    /// El bloque de cabecera tal como se escribió y los registros de longitud variable salvo los de
    /// LASzip, para escribir archivos como este.
    head: Vec<u8>,
    vlrs: Vec<u8>,
    n_vlrs: u32,
    /// Lo que dice el archivo de sí mismo, para informes.
    pub meta: Meta,
    _spool: Option<Spool>,
}

impl Reader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut r = BufReader::new(crate::open_file(path)?);
        let mut head = vec![0u8; 375];
        let got = read_up_to(&mut r, &mut head)?;
        head.truncate(got);
        let mut header = parse_header(&head)?;
        let n_vlrs = u32_at(&head, 100).unwrap_or(0);
        let header_size = u16_at(&head, 94).unwrap_or(0);
        if usize::from(header_size) > head.len() {
            // Una cabecera ampliada más allá de LAS 1.4: se lee entera.
            head = vec![0u8; usize::from(header_size)];
            r.seek(SeekFrom::Start(0))?;
            r.read_exact(&mut head)?;
        }
        head.truncate(usize::from(header_size));
        let end = u64::from(header.point_offset);
        let (vlrs, vlr_list) = read_vlrs(&mut r, u64::from(header_size), n_vlrs, end)?;
        let mut meta = parse_meta(&head);
        meta.vlrs = vlr_list;
        meta.file_size = std::fs::metadata(path).map_or(0, |m| m.len());
        meta.wkt = vlrs
            .iter()
            .find(|v| v.user == "LASF_Projection" && v.record == 2112)
            .map(|v| {
                String::from_utf8_lossy(&v.data)
                    .trim_end_matches('\0')
                    .trim()
                    .to_string()
            })
            .filter(|w| !w.is_empty());
        header.epsg = epsg_of(&vlrs).or_else(|| meta.wkt.as_deref().and_then(epsg_from_wkt));
        header.vertical_epsg = vertical_epsg_of(&vlrs)
            .or_else(|| meta.wkt.as_deref().and_then(vertical_epsg_from_wkt));
        let laz_vlr = vlrs
            .into_iter()
            .find(|v| v.user == "laszip encoded" && v.record == 22204)
            .map(|v| v.data);
        if header.compressed && laz_vlr.is_none() {
            return Err(bad("un LAZ sin su registro laszip"));
        }
        let (raw_vlrs, kept) = raw_vlrs(&mut r, u64::from(header_size), n_vlrs, end)?;
        Ok(Self {
            path: path.to_path_buf(),
            header,
            laz_vlr,
            head,
            vlrs: raw_vlrs,
            n_vlrs: kept,
            meta,
            _spool: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Dónde está en cada registro el atributo adicional llamado `name` (sin distinguir
    /// mayúsculas), cuando el archivo lo declara en su registro de Extra Bytes y su tipo es un
    /// número simple.
    pub fn extra_field(&self, name: &str) -> Option<ExtraField> {
        let raw = &self.vlrs;
        let mut pos = 0usize;
        while let Some(h) = raw.get(pos..pos + 54) {
            let len = usize::from(u16_at(h, 20).unwrap_or(0));
            let data = raw.get(pos + 54..pos + 54 + len).unwrap_or(&[]);
            pos += 54 + len;
            if h.get(2..12) != Some(b"LASF_Spec\0".as_slice()) || u16_at(h, 18) != Some(4) {
                continue;
            }
            let mut at = usize::from(core_len(self.header.format)?);
            for d in data.as_chunks::<192>().0 {
                let code = d[2];
                let size = match code {
                    0 => usize::from(d[3]),
                    1 | 2 => 1,
                    3 | 4 => 2,
                    5 | 6 | 9 => 4,
                    7 | 8 | 10 => 8,
                    // los pares, tríos y similares no son números simples
                    _ => return None,
                };
                let label = String::from_utf8_lossy(&d[4..36]);
                if label.trim_end_matches('\0').trim().eq_ignore_ascii_case(name) && code != 0 {
                    return Some(ExtraField { at, code });
                }
                at += size;
            }
        }
        None
    }

    /// El bloque de cabecera tal como está en el archivo.
    pub(crate) fn raw_header(&self) -> &[u8] {
        &self.head
    }

    /// Los registros de longitud variable distintos de los de proyección y de LASzip, en bruto, y
    /// cuántos son.
    pub(crate) fn other_records(&self) -> (Vec<u8>, u32) {
        split_projection(&self.vlrs, false)
    }

    /// Los registros de proyección (`LASF_Projection`) de este archivo como bytes en bruto, y
    /// cuántos son.
    pub fn projection_records(&self) -> (Vec<u8>, u32) {
        split_projection(&self.vlrs, true)
    }

    /// Este lector con otra proyección: los archivos escritos a partir de él llevan `records`
    /// (registros en bruto, como los da [`Reader::projection_records`]) en lugar de la proyección
    /// que tenía.
    pub fn with_projection(
        mut self,
        records: &[u8],
        count: u32,
        epsg: Option<u16>,
        wkt: Option<String>,
    ) -> Self {
        let (mut others, n_others) = split_projection(&self.vlrs, false);
        others.extend_from_slice(records);
        self.vlrs = others;
        self.n_vlrs = n_others + count;
        self.header.epsg = epsg;
        self.meta.wkt = wkt;
        self
    }

    /// Este lector con `bits` activados en la codificación global de los archivos escritos a partir
    /// de él (bit 4: el sistema de coordenadas es WKT).
    pub fn with_global_encoding_bits(mut self, bits: u16) -> Self {
        self.meta.global_encoding |= bits;
        if let Some(b) = self.head.get_mut(6..8) {
            let v = u16::from_le_bytes([b[0], b[1]]) | bits;
            b.copy_from_slice(&v.to_le_bytes());
        }
        self
    }

    /// Este lector con otro desplazamiento de coordenadas: los archivos escritos a partir de él
    /// guardan sus puntos relativos a `offset`, como necesitan los datos llevados a otro sistema de
    /// coordenadas.
    pub fn with_offset(mut self, offset: [f64; 3]) -> Self {
        for (k, v) in offset.iter().enumerate() {
            put(&mut self.head, 155 + 8 * k, &v.to_le_bytes());
        }
        self.header.offset = offset;
        self
    }

    /// Este lector como archivo LAS 1.4 de formato de punto `new` (véase [`upgraded_format`]): los
    /// archivos escritos a partir de él tienen la cabecera de 375 bytes y los registros que hace
    /// [`upgrade_record`].
    pub fn upgraded(mut self, new: u8) -> Self {
        let extra = self.header.record_len.saturating_sub(
            RECORD_LEN
                .get(usize::from(self.header.format))
                .copied()
                .unwrap_or(0),
        );
        self.head.resize(375, 0);
        put(&mut self.head, 24, &[1, 4]);
        put(&mut self.head, 94, &375u16.to_le_bytes());
        self.header.version = (1, 4);
        self.header.format = new;
        self.header.record_len = RECORD_LEN.get(usize::from(new)).copied().unwrap_or(30) + extra;
        self
    }

    /// El día en que se capturaron los puntos, `AAAA-MM-DD`, a partir del tiempo GPS de los
    /// primeros puntos (la fecha de creación del archivo cuando no tiene un tiempo GPS utilizable).
    /// Lee un lote, no el archivo.
    pub fn capture_date(&self) -> Option<String> {
        let h = &self.header;
        let creation = self.meta.creation;
        if !matches!(h.format, 1 | 3 | 4 | 5..=10) {
            return gps_date(f64::NAN, self.meta.global_encoding, creation);
        }
        let rec_len = usize::from(h.record_len);
        let mut lo = f64::INFINITY;
        // la lectura se detiene tras el primer lote: el error centinela termina el recorrido
        let _ = self.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len).take(100_000) {
                lo = lo.min(self.decode_extra(rec).gps_time);
            }
            Err(Error::Param("stop".into()))
        });
        gps_date(lo, self.meta.global_encoding, creation)
    }

    /// Decodifica un registro de este archivo.
    pub fn decode(&self, rec: &[u8]) -> Option<Point> {
        decode(&self.header, rec)
    }

    /// Los campos más allá de [`Point`] de un registro de este archivo.
    pub fn decode_extra(&self, rec: &[u8]) -> Extra {
        decode_extra(&self.header, rec)
    }

    /// Para herramientas que leen la nube varias veces: un archivo LAZ se descomprime una vez a un
    /// archivo temporal de registros en bruto (se borra con el lector), de modo que las pasadas
    /// posteriores se saltan la descompresión; un archivo LAS se lee en su sitio. Cuesta disco,
    /// nunca memoria.
    pub fn for_passes(self) -> Result<Self> {
        if self.laz_vlr.is_none() {
            return Ok(self);
        }
        // atajo: el volcado va a la carpeta temporal del sistema, que debe contener todos los
        // registros sin comprimir; añadir una opción --temp-dir cuando eso se agote.
        let path = std::env::temp_dir().join(format!(
            "hipso-{}-{}.rec",
            std::process::id(),
            self.header.count
        ));
        let spool = Spool(path.clone());
        let mut w = std::io::BufWriter::with_capacity(1 << 20, File::create(&path)?);
        self.scan_raw(|bytes| w.write_all(bytes).map_err(Error::Io))?;
        w.flush()?;
        drop(w);
        let mut header = self.header.clone();
        header.point_offset = 0;
        header.compressed = false;
        Ok(Self {
            path,
            header,
            laz_vlr: None,
            head: self.head.clone(),
            vlrs: self.vlrs.clone(),
            n_vlrs: self.n_vlrs,
            meta: self.meta.clone(),
            _spool: Some(spool),
        })
    }

    /// Lee cada punto en el orden del archivo, llamando a `f` con cada uno; cuántos se leyeron. Un
    /// archivo cortado es un error: los productos de una parte de una nube parecerían completos.
    pub fn scan(&self, mut f: impl FnMut(&Point)) -> Result<u64> {
        let h = &self.header;
        let rec_len = usize::from(h.record_len);
        self.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                if let Some(p) = decode(h, rec) {
                    f(&p);
                }
            }
            Ok(())
        })
    }

    /// Entrega cada lote de registros en bruto a `f`, en el orden del archivo.
    pub fn scan_raw(&self, mut f: impl FnMut(&[u8]) -> Result<()>) -> Result<u64> {
        let h = &self.header;
        let rec_len = usize::from(h.record_len);
        let mut r = BufReader::with_capacity(1 << 20, crate::open_file(&self.path)?);
        r.seek(SeekFrom::Start(u64::from(h.point_offset)))?;
        let mut batch = BATCH;
        let mut src = match &self.laz_vlr {
            Some(v) => {
                let vlr =
                    laz::LazVlr::from_buffer(v).map_err(|e| Error::Format(format!("LAZ: {e}")))?;
                if vlr.items_size() != u64::from(h.record_len) {
                    return Err(bad("el tamaño de registro LAZ no coincide con la cabecera"));
                }
                // Dos fragmentos por núcleo, para que el descompresor tenga trabajo para cada hilo.
                let chunk = if vlr.uses_variable_size_chunks() {
                    50_000
                } else {
                    vlr.chunk_size() as usize
                };
                let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
                batch = (chunk.clamp(1, 1 << 20) * cores * 2).clamp(BATCH, MAX_LAZ_BATCH);
                Source::Laz(
                    laz::ParLasZipDecompressor::new(r, vlr)
                        .map_err(|e| Error::Format(format!("LAZ: {e}")))?,
                )
            }
            None => Source::Las(r),
        };
        let mut buf = vec![0u8; batch * rec_len];
        let mut left = h.count;
        while left > 0 {
            crate::check_cancel()?;
            let n = (left.min(batch as u64)) as usize;
            let bytes = buf.get_mut(..n * rec_len).ok_or_else(|| bad("batch"))?;
            match &mut src {
                Source::Laz(d) => d
                    .decompress_many(bytes)
                    .map_err(|e| Error::Format(format!("LAZ: {e}")))?,
                Source::Las(p) => p.read_exact(bytes).map_err(|_| {
                    bad("el archivo se acaba antes de sus últimos puntos: está incompleto o dañado")
                })?,
            }
            f(bytes)?;
            left -= n as u64;
        }
        Ok(h.count)
    }
}

/// Los registros de longitud variable en bruto salvo los de LASzip y COPC, y cuántos son.
fn raw_vlrs<R: Read + Seek>(r: &mut R, at: u64, n: u32, end: u64) -> Result<(Vec<u8>, u32)> {
    let (mut out, mut kept, mut pos) = (Vec::new(), 0u32, at);
    for _ in 0..n.min(MAX_VLRS) {
        let mut h = [0u8; 54];
        r.seek(SeekFrom::Start(pos))?;
        if r.read_exact(&mut h).is_err() {
            break;
        }
        let len = u64::from(u16_at(&h, 20).unwrap_or(0));
        if pos + 54 + len > end || out.len() as u64 + 54 + len > 16 * MAX_VLR_BYTES {
            break;
        }
        let laszip =
            h.get(2..16) == Some(b"laszip encoded".as_slice()) && u16_at(&h, 18) == Some(22204);
        // el registro COPC describe el octree del archivo del que vino: un archivo hecho a partir
        // de él no lo es, y un lector que creyera el registro buscaría fragmentos que no están
        let copc = h.get(2..6) == Some(b"copc".as_slice());
        if !laszip && !copc {
            let mut data = vec![0u8; len as usize];
            r.read_exact(&mut data)?;
            out.extend_from_slice(&h);
            out.extend_from_slice(&data);
            kept += 1;
        }
        pos += 54 + len;
    }
    Ok((out, kept))
}

enum Sink {
    Las(std::io::BufWriter<File>),
    Laz(laz::ParLasZipCompressor<std::io::BufWriter<File>>),
}

/// El tipo de un atributo adicional guardado tras cada registro.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExtraKind {
    U32,
    F32,
}

impl ExtraKind {
    fn size(self) -> usize {
        4
    }

    /// El código de tipo de datos del registro Extra Bytes de LAS.
    fn code(self) -> u8 {
        match self {
            ExtraKind::U32 => 5,
            ExtraKind::F32 => 9,
        }
    }
}

/// Un atributo adicional por punto, guardado en los bytes adicionales del registro y descrito por
/// un registro Extra Bytes, como espera otro software LAS (el `treeID` de lidR es uno).
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraAttr {
    pub name: String,
    pub description: String,
    pub kind: ExtraKind,
}

/// Cómo se prepara un [`Writer`].
#[derive(Clone, Debug, PartialEq)]
pub struct WriterOptions {
    /// Puntos retenidos antes de comprimirlos (solo LAZ): grande para un archivo, pequeño para
    /// muchos.
    pub flush_points: usize,
    /// Búfer del archivo, en bytes.
    pub buffer: usize,
    pub extra: Vec<ExtraAttr>,
}

impl Default for WriterOptions {
    fn default() -> Self {
        Self {
            flush_points: MAX_LAZ_BATCH / 4,
            buffer: 1 << 20,
            extra: Vec::new(),
        }
    }
}

impl WriterOptions {
    /// Para muchos archivos abiertos a la vez.
    pub fn small() -> Self {
        Self {
            flush_points: 50_000,
            buffer: 64 << 10,
            extra: Vec::new(),
        }
    }
}

/// El descriptor de 192 bytes de un atributo adicional.
fn extra_descriptor(a: &ExtraAttr) -> Vec<u8> {
    let mut d = vec![0u8; 192];
    d[2] = a.kind.code();
    for (k, c) in a.name.bytes().take(31).enumerate() {
        d[4 + k] = c;
    }
    for (k, c) in a.description.bytes().take(31).enumerate() {
        d[160 + k] = c;
    }
    d
}

/// Los registros de longitud variable de un archivo nuevo: los de la plantilla, con `attrs`
/// añadidos a su registro de Extra Bytes (o uno nuevo), y cuántos registros hay.
fn merge_extra_vlrs(raw: &[u8], n: u32, attrs: &[ExtraAttr]) -> (Vec<u8>, u32) {
    if attrs.is_empty() {
        return (raw.to_vec(), n);
    }
    let descriptors: Vec<u8> = attrs.iter().flat_map(extra_descriptor).collect();
    let mut out = Vec::with_capacity(raw.len() + descriptors.len() + 54);
    let (mut pos, mut merged) = (0usize, false);
    while let Some(h) = raw.get(pos..pos + 54) {
        let len = usize::from(u16_at(h, 20).unwrap_or(0));
        let data = raw.get(pos + 54..pos + 54 + len).unwrap_or(&[]);
        let is_extra = h.get(2..12) == Some(b"LASF_Spec\0".as_slice()) && u16_at(h, 18) == Some(4);
        if is_extra && !merged {
            let mut head = h.to_vec();
            let total = u16::try_from(len + descriptors.len()).unwrap_or(u16::MAX);
            put(&mut head, 20, &total.to_le_bytes());
            out.extend_from_slice(&head);
            out.extend_from_slice(data);
            out.extend_from_slice(&descriptors);
            merged = true;
        } else {
            out.extend_from_slice(h);
            out.extend_from_slice(data);
        }
        pos += 54 + len;
    }
    if merged {
        return (out, n);
    }
    let mut head = vec![0u8; 54];
    head[2..11].copy_from_slice(b"LASF_Spec");
    put(&mut head, 18, &4u16.to_le_bytes());
    let total = u16::try_from(descriptors.len()).unwrap_or(u16::MAX);
    put(&mut head, 20, &total.to_le_bytes());
    head[22..22 + 11].copy_from_slice(b"Extra bytes");
    out.extend_from_slice(&head);
    out.extend_from_slice(&descriptors);
    (out, n + 1)
}

/// Escribe LAS, o LAZ cuando la ruta termina en `.laz`, con la cabecera, el sistema de coordenadas
/// y la disposición de registros de un archivo plantilla (más los atributos adicionales que haya);
/// los recuentos, la extensión y los recuentos de retornos se fijan al terminar.
pub struct Writer {
    sink: Sink,
    template: Header,
    head: Vec<u8>,
    pending: Vec<u8>,
    count: u64,
    by_return: [u64; 15],
    bounds: [f64; 6],
    /// Bytes de atributos adicionales tras cada registro.
    extra_len: usize,
    flush_bytes: usize,
}

impl Writer {
    pub fn create(path: &Path, template: &Reader) -> Result<Self> {
        Self::create_with(path, template, &WriterOptions::default())
    }

    pub fn create_with(path: &Path, template: &Reader, opts: &WriterOptions) -> Result<Self> {
        let laz = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("laz"));
        let h = &template.header;
        let mut head = template.head.clone();
        let base = *RECORD_LEN
            .get(usize::from(h.format))
            .ok_or_else(|| bad("formato de punto"))?;
        let extra_len: usize = opts.extra.iter().map(|a| a.kind.size()).sum();
        let out_len = usize::from(h.record_len) + extra_len;
        let (vlrs, n_vlrs) = merge_extra_vlrs(&template.vlrs, template.n_vlrs, &opts.extra);
        let mut laz_vlr = Vec::new();
        let vlr = if laz {
            let items = laz::LazItemRecordBuilder::default_for_point_format_id(
                h.format,
                u16::try_from(out_len).map_err(|_| bad("longitud de registro"))? - base,
            )
            .map_err(|e| Error::Unsupported(format!("LAZ: {e}")))?;
            let vlr = laz::LazVlr::from_laz_items(items);
            let mut payload = Vec::new();
            vlr.write_to(&mut payload)?;
            laz_vlr.extend_from_slice(&[0, 0]);
            let mut user = [0u8; 16];
            user[..14].copy_from_slice(b"laszip encoded");
            laz_vlr.extend_from_slice(&user);
            laz_vlr.extend_from_slice(&22204u16.to_le_bytes());
            laz_vlr.extend_from_slice(
                &u16::try_from(payload.len())
                    .map_err(|_| bad("LAZ record"))?
                    .to_le_bytes(),
            );
            laz_vlr.extend_from_slice(&[0u8; 32]);
            laz_vlr.extend_from_slice(&payload);
            Some(vlr)
        } else {
            None
        };
        let point_offset = head.len() + vlrs.len() + laz_vlr.len();
        put(
            &mut head,
            96,
            &u32::try_from(point_offset)
                .map_err(|_| bad("header"))?
                .to_le_bytes(),
        );
        put(&mut head, 100, &(n_vlrs + u32::from(laz)).to_le_bytes());
        put(&mut head, 104, &[h.format | if laz { 0x80 } else { 0 }]);
        put(
            &mut head,
            105,
            &u16::try_from(out_len)
                .map_err(|_| bad("longitud de registro"))?
                .to_le_bytes(),
        );
        if head.len() >= 375 {
            // No hay registros extendidos tras los puntos.
            put(&mut head, 235, &[0u8; 12]);
        }
        let mut w = std::io::BufWriter::with_capacity(opts.buffer.max(4096), File::create(path)?);
        w.write_all(&head)?;
        w.write_all(&vlrs)?;
        w.write_all(&laz_vlr)?;
        let sink = match vlr {
            Some(v) => Sink::Laz(
                laz::ParLasZipCompressor::new(w, v)
                    .map_err(|e| Error::Format(format!("LAZ: {e}")))?,
            ),
            None => Sink::Las(w),
        };
        Ok(Self {
            sink,
            template: h.clone(),
            head,
            pending: Vec::new(),
            count: 0,
            by_return: [0; 15],
            bounds: [
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            extra_len,
            flush_bytes: opts.flush_points.max(1000) * out_len,
        })
    }

    /// Escribe un registro con la disposición de la plantilla (los atributos adicionales, si los
    /// hay, son cero).
    pub fn write(&mut self, rec: &[u8]) -> Result<()> {
        let zeros = [0u8; 64];
        self.write_with_extra(rec, zeros.get(..self.extra_len).unwrap_or(&[]))
    }

    /// Escribe un registro con la disposición de la plantilla y los bytes de sus atributos
    /// adicionales.
    pub fn write_with_extra(&mut self, rec: &[u8], extra: &[u8]) -> Result<()> {
        if rec.len() != usize::from(self.template.record_len) || extra.len() != self.extra_len {
            return Err(bad("a record of another layout"));
        }
        if let Some(p) = decode(&self.template, rec) {
            let b = &mut self.bounds;
            *b = [
                b[0].min(p.x),
                b[1].min(p.y),
                b[2].min(p.z),
                b[3].max(p.x),
                b[4].max(p.y),
                b[5].max(p.z),
            ];
            if let Some(n) = self
                .by_return
                .get_mut(usize::from(p.return_number.max(1)) - 1)
            {
                *n += 1;
            }
        }
        self.count += 1;
        match &mut self.sink {
            Sink::Las(w) => {
                w.write_all(rec)?;
                w.write_all(extra)?;
            }
            Sink::Laz(c) => {
                self.pending.extend_from_slice(rec);
                self.pending.extend_from_slice(extra);
                if self.pending.len() >= self.flush_bytes {
                    c.compress_many(&self.pending)?;
                    self.pending.clear();
                }
            }
        }
        Ok(())
    }

    /// Completa el archivo; cuántos puntos contiene.
    pub fn finish(mut self) -> Result<u64> {
        let mut w = match self.sink {
            Sink::Las(w) => w,
            Sink::Laz(mut c) => {
                c.compress_many(&self.pending)?;
                c.done().map_err(|e| Error::Format(format!("LAZ: {e}")))?;
                c.into_inner()
            }
        };
        let h = &mut self.head;
        let legacy = self.template.format < 6 && self.count <= u64::from(u32::MAX);
        put(
            h,
            107,
            &(if legacy { self.count as u32 } else { 0 }).to_le_bytes(),
        );
        for (k, n) in self.by_return.iter().take(5).enumerate() {
            put(
                h,
                111 + 4 * k,
                &(if legacy { *n as u32 } else { 0 }).to_le_bytes(),
            );
        }
        let b = if self.count > 0 {
            self.bounds
        } else {
            [0.0; 6]
        };
        for (at, v) in [
            (179, b[3]),
            (187, b[0]),
            (195, b[4]),
            (203, b[1]),
            (211, b[5]),
            (219, b[2]),
        ] {
            put(h, at, &v.to_le_bytes());
        }
        if h.len() >= 375 {
            put(h, 247, &self.count.to_le_bytes());
            for (k, n) in self.by_return.iter().enumerate() {
                put(h, 255 + 8 * k, &n.to_le_bytes());
            }
        }
        w.flush()?;
        w.seek(SeekFrom::Start(0))?;
        w.write_all(h)?;
        w.flush()?;
        Ok(self.count)
    }
}

fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
    if let Some(s) = buf.get_mut(at..at + bytes.len()) {
        s.copy_from_slice(bytes);
    }
}

/// Los registros de longitud variable de un bloque en bruto que son (o no son) registros de
/// proyección, con su recuento.
fn split_projection(raw: &[u8], want: bool) -> (Vec<u8>, u32) {
    let (mut out, mut n, mut pos) = (Vec::new(), 0u32, 0usize);
    while let Some(h) = raw.get(pos..pos + 54) {
        let len = usize::from(u16_at(h, 20).unwrap_or(0));
        let Some(rec) = raw.get(pos..pos + 54 + len) else {
            break;
        };
        let is_proj = h.get(2..17) == Some(b"LASF_Projection".as_slice());
        if is_proj == want {
            out.extend_from_slice(rec);
            n += 1;
        }
        pos += 54 + len;
    }
    (out, n)
}

/// El registro del directorio de claves GeoTIFF que declara el código EPSG `epsg` (proyectado, o
/// geográfico para códigos de los 4000), como bytes en bruto.
pub fn epsg_record(epsg: u16) -> Vec<u8> {
    epsg_record_with_vertical(epsg, None)
}

/// Como [`epsg_record`], declarando además la referencia vertical (clave 4096) cuando la hay.
pub fn epsg_record_with_vertical(epsg: u16, vertical: Option<u16>) -> Vec<u8> {
    let geographic = (4000..5000).contains(&epsg);
    let mut keys: Vec<[u16; 4]> = vec![
        [1024, 0, 1, if geographic { 2 } else { 1 }],
        [1025, 0, 1, 1],
        [if geographic { 2048 } else { 3072 }, 0, 1, epsg],
    ];
    if let Some(v) = vertical {
        keys.push([4096, 0, 1, v]);
    }
    let mut data: Vec<u8> = Vec::new();
    for v in [1u16, 1, 0, keys.len() as u16] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    for k in keys {
        for v in k {
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut rec = vec![0u8; 54];
    rec[2..17].copy_from_slice(b"LASF_Projection");
    rec[18..20].copy_from_slice(&34735u16.to_le_bytes());
    rec[20..22].copy_from_slice(&(data.len() as u16).to_le_bytes());
    rec[22..22 + 14].copy_from_slice(b"GeoKeyDirector");
    rec.extend_from_slice(&data);
    rec
}

/// El WKT compuesto de un código EPSG horizontal y uno vertical (`COMPD_CS[…]`).
pub fn compound_wkt(horizontal: u16, vertical: u16) -> Option<String> {
    hipso_crs::SpatialRef::parse(&format!("EPSG:{horizontal}+{vertical}"))
        .ok()
        .and_then(|s| s.to_wkt())
}

/// El registro WKT de OGC (2112) de un archivo LAS 1.4, como bytes en bruto: el texto termina con
/// un NUL.
pub fn wkt_record(wkt: &str) -> Vec<u8> {
    let mut data = wkt.as_bytes().to_vec();
    data.push(0);
    let mut rec = vec![0u8; 54];
    rec[2..17].copy_from_slice(b"LASF_Projection");
    rec[18..20].copy_from_slice(&2112u16.to_le_bytes());
    rec[20..22].copy_from_slice(&(data.len() as u16).to_le_bytes());
    rec[22..22 + 7].copy_from_slice(b"OGC WKT");
    rec.extend_from_slice(&data);
    rec
}

/// Un archivo temporal, borrado al soltarlo.
struct Spool(std::path::PathBuf);

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

enum Source {
    Laz(laz::ParLasZipDecompressor<BufReader<File>>),
    Las(BufReader<File>),
}

fn read_up_to<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(buf.get_mut(got..).unwrap_or_default())? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_id_and_user_data_go_to_their_bytes() {
        let mut old = [0u8; 28];
        set_source_id(1, &mut old, 0x1234);
        set_user_data(&mut old, 7);
        assert_eq!((old[18], old[19], old[17]), (0x34, 0x12, 7));
        assert_eq!(old[20], 0);
        let mut new = [0u8; 40];
        set_source_id(6, &mut new, 0x1234);
        set_user_data(&mut new, 9);
        assert_eq!((new[20], new[21], new[17]), (0x34, 0x12, 9));
        assert_eq!((new[18], new[19]), (0, 0));
        // un registro demasiado corto se deja como está
        let mut short = [0u8; 10];
        set_source_id(6, &mut short, 5);
        set_user_data(&mut short, 5);
        assert_eq!(short, [0u8; 10]);
    }

    #[test]
    fn gps_times_become_dates() {
        // tiempo GPS ajustado 6,0962773e7 s: 1e9 + 6,0962773e7 s después de 1980-01-06
        assert_eq!(
            gps_date(60_962_773.0, 1, (1, 2000)).as_deref(),
            Some("2013-08-19")
        );
        // un tiempo de semana no tiene fecha: la sustituye la fecha de creación (día 45 de 2015)
        assert_eq!(
            gps_date(1000.0, 0, (45, 2015)).as_deref(),
            Some("2015-02-14")
        );
        assert_eq!(gps_date(f64::NAN, 0, (0, 0)), None);
        // El tiempo Unix (1,78e9 s es junio de 2026) leído como tiempo GPS sería de 2036: se lee
        // como Unix
        assert_eq!(
            gps_date(1_781_265_639.0, 0, (0, 0)).as_deref(),
            Some("2026-06-12")
        );
    }

    use super::*;

    /// Un archivo LAS 1.2 de formato de punto 1: (x, y, z, byte de retorno, byte de clase) por
    /// punto.
    fn las_file(points: &[(i32, i32, i32, u8, u8)]) -> Vec<u8> {
        let mut h = vec![0u8; 227];
        h[..4].copy_from_slice(b"LASF");
        h[24] = 1;
        h[25] = 2;
        h[94..96].copy_from_slice(&227u16.to_le_bytes());
        h[96..100].copy_from_slice(&227u32.to_le_bytes());
        h[104] = 1;
        h[105..107].copy_from_slice(&28u16.to_le_bytes());
        h[107..111].copy_from_slice(&(points.len() as u32).to_le_bytes());
        for (at, v) in [
            (131, 0.01),
            (139, 0.01),
            (147, 0.01),
            (155, 1000.0),
            (163, 2000.0),
            (171, 0.0),
        ] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        // max x, min x, max y, min y, max z, min z.
        for (at, v) in [
            (179, 1001.0),
            (187, 1000.0),
            (195, 2001.0),
            (203, 2000.0),
            (211, 10.0),
            (219, 0.0),
        ] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for &(x, y, z, ret, class) in points {
            let mut rec = [0u8; 28];
            rec[..4].copy_from_slice(&x.to_le_bytes());
            rec[4..8].copy_from_slice(&y.to_le_bytes());
            rec[8..12].copy_from_slice(&z.to_le_bytes());
            rec[12..14].copy_from_slice(&77u16.to_le_bytes());
            rec[14] = ret;
            rec[15] = class;
            h.extend_from_slice(&rec);
        }
        h
    }

    #[test]
    fn reads_header_and_points() -> Result<()> {
        // Retorno 2 de 3, clase 5; después un punto de ruido retenido.
        let bytes = las_file(&[
            (50, 25, 1000, 2 | (3 << 3), 5),
            (100, 100, 0, 1 | (1 << 3), 7 | 0x80),
        ]);
        let p = std::env::temp_dir().join(format!("hipso_las_test_{}.las", std::process::id()));
        std::fs::write(&p, &bytes)?;
        let r = Reader::open(&p)?;
        assert_eq!(
            (r.header.format, r.header.count, r.header.compressed),
            (1, 2, false)
        );
        assert_eq!(r.header.bounds, [1000.0, 2000.0, 0.0, 1001.0, 2001.0, 10.0]);
        let mut got = Vec::new();
        let n = r.for_passes()?.scan(|pt| got.push(*pt))?;
        let _ = std::fs::remove_file(&p);
        assert_eq!(n, 2);
        let a = got[0];
        assert!(
            (a.x - 1000.5).abs() < 1e-9
                && (a.y - 2000.25).abs() < 1e-9
                && (a.z - 10.0).abs() < 1e-9
        );
        assert_eq!(
            (a.intensity, a.return_number, a.number_of_returns, a.class),
            (77, 2, 3, 5)
        );
        assert!(!a.is_noise() && !a.is_first());
        assert!(got[1].is_noise() && got[1].withheld);
        Ok(())
    }

    #[test]
    fn rejects_what_is_not_las() {
        assert!(parse_header(b"not a las file").is_err());
        let mut b = las_file(&[]);
        b[105] = 10; // registros más cortos que los del formato 1
        assert!(parse_header(&b).is_err());
    }
}
