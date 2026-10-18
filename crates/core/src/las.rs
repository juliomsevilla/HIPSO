//! LAS 1.0–1.4 and LAZ, read as a stream: points are decoded batch by batch from their raw
//! records and handed to a callback, so memory never grows with the size of the cloud.
//!
//! The header and record layout follow the approach of RustGIS's `rustgis-data` LiDAR reader
//! (Apache-2.0, https://github.com/opengeos/RustGIS).

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::{Error, Result};

/// The smallest record of each point format.
const RECORD_LEN: [u16; 11] = [20, 28, 26, 34, 57, 63, 30, 36, 38, 59, 67];

/// Points decoded per batch (LAZ: rounded up to whole chunks by the decompressor).
const BATCH: usize = 1 << 16;

/// Most LAZ points decompressed at once (in parallel): about 100 MB of records at most.
const MAX_LAZ_BATCH: usize = 1 << 21;

/// Variable length records read, and the most bytes one may hold.
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
    /// The projected (else geographic) EPSG code of the GeoTIFF keys, when given.
    pub epsg: Option<u16>,
}

/// One point: what the forestry tools need from a record.
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
}

impl Point {
    /// Noise (classes 7 and 18) and withheld points take no part in any product.
    pub fn is_noise(&self) -> bool {
        self.withheld || self.class == 7 || self.class == 18
    }

    pub fn is_first(&self) -> bool {
        self.return_number <= 1
    }
}

/// What a file says about itself besides what the tools need: for reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub file_source_id: u16,
    pub global_encoding: u16,
    pub system_id: String,
    pub software: String,
    /// Creation day of the year and year.
    pub creation: (u16, u16),
    pub header_size: u16,
    /// Points by return number 1–5 as the header counts them (LAS 1.4: up to 15).
    pub by_return: Vec<u64>,
    pub vlrs: Vec<VlrInfo>,
    /// The coordinate system as WKT, when the file carries it.
    pub wkt: Option<String>,
    pub file_size: u64,
}

/// A variable length record in the header block.
#[derive(Clone, Debug, PartialEq)]
pub struct VlrInfo {
    pub user: String,
    pub record: u16,
    pub len: u64,
    pub description: String,
}

/// Fields beyond [`Point`], read from the same record.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Extra {
    /// The flight line (point source ID).
    pub source_id: u16,
    pub user_data: u8,
    /// Scan angle in degrees (signed).
    pub scan_angle: f32,
    pub gps_time: f64,
    /// 16-bit colour, when the format has it.
    pub rgb: Option<[u16; 3]>,
    pub synthetic: bool,
    pub key_point: bool,
    pub overlap: bool,
}

/// Where each format keeps its optional fields: GPS time, then colour.
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

/// Decodes the fields beyond [`Point`] of one record.
pub(crate) fn decode_extra(h: &Header, rec: &[u8]) -> Extra {
    let ext = h.format >= 6;
    let (source_id, user_data, scan_angle) = if ext {
        (
            u16_at(rec, 20).unwrap_or(0),
            rec.get(17).copied().unwrap_or(0),
            // Extended records count the angle in steps of 0.006 degrees.
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

/// Writes coordinates into a record with the header's scale and offset; false when one lies
/// outside what a 32-bit coordinate can hold.
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
    Error::Format(format!("not a readable LAS file ({what})"))
}

/// Reads the header block (its first 375 bytes at most).
fn parse_header(b: &[u8]) -> Result<Header> {
    if !b.starts_with(b"LASF") {
        return Err(bad("no LASF signature"));
    }
    let version = (
        *b.get(24).ok_or_else(|| bad("version"))?,
        *b.get(25).ok_or_else(|| bad("version"))?,
    );
    if version.0 != 1 || version.1 > 4 {
        return Err(Error::Unsupported(format!(
            "LAS {}.{}",
            version.0, version.1
        )));
    }
    let header_size = u16_at(b, 94).ok_or_else(|| bad("header size"))?;
    let point_offset = u32_at(b, 96).ok_or_else(|| bad("point offset"))?;
    let raw_format = *b.get(104).ok_or_else(|| bad("point format"))?;
    // LAZ marks its formats with the top bits.
    let compressed = raw_format & 0xC0 != 0;
    let format = raw_format & 0x3F;
    let min_len = *RECORD_LEN
        .get(usize::from(format))
        .ok_or_else(|| Error::Unsupported(format!("LAS point format {format}")))?;
    let record_len = u16_at(b, 105).ok_or_else(|| bad("record length"))?;
    if record_len < min_len {
        return Err(bad("records shorter than their format"));
    }
    let legacy = u64::from(u32_at(b, 107).ok_or_else(|| bad("point count"))?);
    let count = if version.1 >= 4 && header_size >= 375 {
        u64_at(b, 247).filter(|n| *n > 0).unwrap_or(legacy)
    } else {
        legacy
    };
    let f = |at| {
        f64_at(b, at)
            .filter(|x| x.is_finite())
            .ok_or_else(|| bad("scale, offset or bounds"))
    };
    let scale = [f(131)?, f(139)?, f(147)?];
    if scale.contains(&0.0) {
        return Err(bad("a zero scale"));
    }
    let offset = [f(155)?, f(163)?, f(171)?];
    // On disk: max x, min x, max y, min y, max z, min z.
    let bounds = [f(187)?, f(203)?, f(219)?, f(179)?, f(195)?, f(211)?];
    if u32::from(header_size) > point_offset || header_size < 227 {
        return Err(bad("header size"));
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
    })
}

struct Vlr {
    user: String,
    record: u16,
    data: Vec<u8>,
}

/// The records this reader uses (GeoTIFF keys, WKT, LASzip's), and a list of all of them.
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

/// The descriptive part of the header block.
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

/// The EPSG code a WKT ends with (the outermost authority), either WKT1 or WKT2 style.
pub fn epsg_from_wkt(wkt: &str) -> Option<u16> {
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

/// The name of a coordinate system from its WKT: the first quoted string.
pub fn wkt_name(wkt: &str) -> Option<String> {
    let start = wkt.find('"')? + 1;
    let len = wkt.get(start..)?.find('"')?;
    wkt.get(start..start + len).map(str::to_string)
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

/// Decodes one record (`rec` holds at least the format's minimum length).
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
    Some(Point {
        x: c(0)?,
        y: c(1)?,
        z: c(2)?,
        intensity,
        return_number,
        number_of_returns,
        class,
        withheld,
    })
}

/// The date of a GPS time stamp as `YYYY-MM-DD`. Adjusted standard GPS time (bit 0 of the global
/// encoding) counts seconds since 1980-01-06 minus 1e9; a time within a week is a time of week
/// and carries no date, so the file's creation date `(day of year, year)` stands in. Standard
/// (unadjusted) time below 4e8 s would put the survey before 1993, so it is read as adjusted.
pub fn gps_date(t: f64, global_encoding: u16, creation: (u16, u16)) -> Option<String> {
    let days_from_civil = |y: i64, m: i64, d: i64| {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    };
    let days = if t.is_finite() && (global_encoding & 1 != 0 || t > 604_800.0 * 1.01) {
        // old files (LAS 1.0) hold adjusted time without the flag: no survey is from before 1992
        let secs = if global_encoding & 1 != 0 || t < 4e8 {
            t + 1e9
        } else {
            t
        };
        // 1980-01-06, the GPS epoch, is day 3657 of the Unix count
        3657 + (secs / 86_400.0).floor() as i64
    } else if creation.1 > 1900 && creation.0 >= 1 {
        days_from_civil(i64::from(creation.1), 1, 1) + i64::from(creation.0) - 1
    } else {
        return None;
    };
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

/// Sets a record's classification.
pub fn set_class(format: u8, rec: &mut [u8], class: u8) {
    if format >= 6 {
        if let Some(b) = rec.get_mut(16) {
            *b = class;
        }
    } else if let Some(b) = rec.get_mut(15) {
        *b = (*b & 0xE0) | (class & 0x1F);
    }
}

/// The LAS 1.4 point format that keeps what a legacy `format` keeps and holds a full class byte
/// (classes above 31 do not fit the 5 bits of formats 0–5). `None`: no such format here.
pub fn upgraded_format(format: u8) -> Option<u8> {
    match format {
        0 | 1 => Some(6),
        2 | 3 => Some(7),
        6..=10 => Some(format),
        _ => None,
    }
}

/// Converts the record of a legacy `old` format to the 1.4 format `new` (6 or 7); bytes after
/// the format's own fields (extra attributes) are kept. Scan angles go from whole degrees to
/// steps of 0.006°.
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

/// Anything read as a stream of points: a file, or a tile held in memory.
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

/// Points held in memory, with their extent.
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

/// An open LAS or LAZ file.
pub struct Reader {
    path: std::path::PathBuf,
    pub header: Header,
    laz_vlr: Option<Vec<u8>>,
    /// The header block as written, and the variable length records but LASzip's, for
    /// writing files like this one.
    head: Vec<u8>,
    vlrs: Vec<u8>,
    n_vlrs: u32,
    /// What the file says about itself, for reports.
    pub meta: Meta,
    _spool: Option<Spool>,
}

impl Reader {
    pub fn open(path: &Path) -> Result<Self> {
        let mut r = BufReader::new(File::open(path)?);
        let mut head = vec![0u8; 375];
        let got = read_up_to(&mut r, &mut head)?;
        head.truncate(got);
        let mut header = parse_header(&head)?;
        let n_vlrs = u32_at(&head, 100).unwrap_or(0);
        let header_size = u16_at(&head, 94).unwrap_or(0);
        if usize::from(header_size) > head.len() {
            // A header extended past LAS 1.4's: read it whole.
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
                    .trim_end_matches(' ')
                    .trim()
                    .to_string()
            })
            .filter(|w| !w.is_empty());
        header.epsg = epsg_of(&vlrs).or_else(|| meta.wkt.as_deref().and_then(epsg_from_wkt));
        let laz_vlr = vlrs
            .into_iter()
            .find(|v| v.user == "laszip encoded" && v.record == 22204)
            .map(|v| v.data);
        if header.compressed && laz_vlr.is_none() {
            return Err(bad("a LAZ file without its laszip record"));
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

    /// The projection records (`LASF_Projection`) of this file as raw bytes, and how many.
    pub fn projection_records(&self) -> (Vec<u8>, u32) {
        split_projection(&self.vlrs, true)
    }

    /// This reader with another projection: files written from it carry `records` (raw records,
    /// as [`Reader::projection_records`] gives) instead of the projection it had.
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

    /// This reader as a LAS 1.4 file of point format `new` (see [`upgraded_format`]): files
    /// written from it have the 375-byte header and records [`upgrade_record`] makes.
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

    /// The day the points were captured, `YYYY-MM-DD`, from the GPS time of the first points
    /// (the file's creation date when it has no usable GPS time). Reads one batch, not the file.
    pub fn capture_date(&self) -> Option<String> {
        let h = &self.header;
        let creation = self.meta.creation;
        if !matches!(h.format, 1 | 3 | 4 | 5..=10) {
            return gps_date(f64::NAN, self.meta.global_encoding, creation);
        }
        let rec_len = usize::from(h.record_len);
        let mut lo = f64::INFINITY;
        // reading stops after the first batch: the sentinel error ends the scan
        let _ = self.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len).take(100_000) {
                lo = lo.min(self.decode_extra(rec).gps_time);
            }
            Err(Error::Param("stop".into()))
        });
        gps_date(lo, self.meta.global_encoding, creation)
    }

    /// Decodes a record of this file.
    pub fn decode(&self, rec: &[u8]) -> Option<Point> {
        decode(&self.header, rec)
    }

    /// The fields beyond [`Point`] of a record of this file.
    pub fn decode_extra(&self, rec: &[u8]) -> Extra {
        decode_extra(&self.header, rec)
    }

    /// For tools that read the cloud several times: a LAZ file is decompressed once into a
    /// temporary file of raw records (deleted with the reader), so later passes skip the
    /// decompression; a LAS file is read in place. Costs disk, never memory.
    pub fn for_passes(self) -> Result<Self> {
        if self.laz_vlr.is_none() {
            return Ok(self);
        }
        // shortcut: the spool goes to the system temp folder, which must hold every record
        // uncompressed; add a --temp-dir option when that runs out.
        let path = std::env::temp_dir().join(format!(
            "rustlidar-{}-{}.rec",
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

    /// Reads every point in file order, calling `f` on each; how many were read. A file cut
    /// short is an error: products from part of a cloud would look complete.
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

    /// Hands every batch of raw records to `f`, in file order.
    pub fn scan_raw(&self, mut f: impl FnMut(&[u8]) -> Result<()>) -> Result<u64> {
        let h = &self.header;
        let rec_len = usize::from(h.record_len);
        let mut r = BufReader::with_capacity(1 << 20, File::open(&self.path)?);
        r.seek(SeekFrom::Start(u64::from(h.point_offset)))?;
        let mut batch = BATCH;
        let mut src = match &self.laz_vlr {
            Some(v) => {
                let vlr =
                    laz::LazVlr::from_buffer(v).map_err(|e| Error::Format(format!("LAZ: {e}")))?;
                if vlr.items_size() != u64::from(h.record_len) {
                    return Err(bad("LAZ record size differs from the header's"));
                }
                // Two chunks per core, so the decompressor has work for every thread.
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
                Source::Las(p) => p
                    .read_exact(bytes)
                    .map_err(|_| bad("the file ends before its last points"))?,
            }
            f(bytes)?;
            left -= n as u64;
        }
        Ok(h.count)
    }
}

/// The raw variable length records but LASzip's, and how many they are.
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
        if !laszip {
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

/// The type of an extra attribute stored after each record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExtraKind {
    U32,
    F32,
}

impl ExtraKind {
    fn size(self) -> usize {
        4
    }

    /// The data type code of the LAS Extra Bytes record.
    fn code(self) -> u8 {
        match self {
            ExtraKind::U32 => 5,
            ExtraKind::F32 => 9,
        }
    }
}

/// An extra per-point attribute, kept in the record's extra bytes and described by an Extra
/// Bytes record, as other LAS software expects (lidR's `treeID` is one).
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraAttr {
    pub name: String,
    pub description: String,
    pub kind: ExtraKind,
}

/// How a [`Writer`] is set up.
#[derive(Clone, Debug, PartialEq)]
pub struct WriterOptions {
    /// Points held before they are compressed (LAZ only): large for one file, small for many.
    pub flush_points: usize,
    /// Buffer of the file, in bytes.
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
    /// For many files open at once.
    pub fn small() -> Self {
        Self {
            flush_points: 50_000,
            buffer: 64 << 10,
            extra: Vec::new(),
        }
    }
}

/// The 192-byte descriptor of an extra attribute.
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

/// The variable length records of a new file: the template's, with `attrs` added to its Extra
/// Bytes record (or a new one), and how many records there are.
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

/// Writes LAS, or LAZ when the path ends in `.laz`, with the header, coordinate system and
/// record layout of a template file (plus any extra attributes); counts, extent and return
/// counts are set on finishing.
pub struct Writer {
    sink: Sink,
    template: Header,
    head: Vec<u8>,
    pending: Vec<u8>,
    count: u64,
    by_return: [u64; 15],
    bounds: [f64; 6],
    /// Bytes of extra attributes after each record.
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
            .ok_or_else(|| bad("point format"))?;
        let extra_len: usize = opts.extra.iter().map(|a| a.kind.size()).sum();
        let out_len = usize::from(h.record_len) + extra_len;
        let (vlrs, n_vlrs) = merge_extra_vlrs(&template.vlrs, template.n_vlrs, &opts.extra);
        let mut laz_vlr = Vec::new();
        let vlr = if laz {
            let items = laz::LazItemRecordBuilder::default_for_point_format_id(
                h.format,
                u16::try_from(out_len).map_err(|_| bad("record length"))? - base,
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
                .map_err(|_| bad("record length"))?
                .to_le_bytes(),
        );
        if head.len() >= 375 {
            // No extended records follow the points.
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

    /// Writes one record in the template's layout (extra attributes, if any, are zero).
    pub fn write(&mut self, rec: &[u8]) -> Result<()> {
        let zeros = [0u8; 64];
        self.write_with_extra(rec, zeros.get(..self.extra_len).unwrap_or(&[]))
    }

    /// Writes one record in the template's layout and the bytes of its extra attributes.
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

    /// Completes the file; how many points it holds.
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

/// The variable length records of a raw block that are (or are not) projection records, with
/// their count.
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

/// The GeoTIFF key directory record that declares EPSG code `epsg` (projected, or geographic
/// for codes in the 4000s), as raw bytes.
pub fn epsg_record(epsg: u16) -> Vec<u8> {
    let geographic = (4000..5000).contains(&epsg);
    let keys: [[u16; 4]; 3] = [
        [1024, 0, 1, if geographic { 2 } else { 1 }],
        [1025, 0, 1, 1],
        [if geographic { 2048 } else { 3072 }, 0, 1, epsg],
    ];
    let mut data: Vec<u8> = Vec::new();
    for v in [1u16, 1, 0, 3] {
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

/// A temporary file, deleted when dropped.
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
    fn gps_times_become_dates() {
        // adjusted GPS time 6.0962773e7 s: 1e9 + 6.0962773e7 s after 1980-01-06
        assert_eq!(
            gps_date(60_962_773.0, 1, (1, 2000)).as_deref(),
            Some("2013-08-19")
        );
        // a time of week has no date: the creation date (day 45 of 2015) stands in
        assert_eq!(
            gps_date(1000.0, 0, (45, 2015)).as_deref(),
            Some("2015-02-14")
        );
        assert_eq!(gps_date(f64::NAN, 0, (0, 0)), None);
    }

    use super::*;

    /// A LAS 1.2 file of point format 1: (x, y, z, return byte, class byte) per point.
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
        // Return 2 of 3, class 5; then a withheld noise point.
        let bytes = las_file(&[
            (50, 25, 1000, 2 | (3 << 3), 5),
            (100, 100, 0, 1 | (1 << 3), 7 | 0x80),
        ]);
        let p = std::env::temp_dir().join(format!("rustlidar_las_test_{}.las", std::process::id()));
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
        b[105] = 10; // records shorter than format 1's
        assert!(parse_header(&b).is_err());
    }
}
