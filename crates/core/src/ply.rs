//! PLY point clouds (ASCII and binary, either byte order) turned into LAS/LAZ, so that every tool
//! reads them. The scanners and photogrammetry programs that write PLY give positions, often a
//! colour and sometimes an intensity; normals and other properties are left out.
//!
//! The conversion reads the file twice (the first time for the extent, which sets the offset of
//! the integer coordinates) and writes in a stream, so memory does not depend on the number of
//! points.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::las::{Reader, Writer, WriterOptions, epsg_record};
use crate::{Error, Result, check_cancel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Ascii,
    Little,
    Big,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Kind {
    fn parse(t: &str) -> Option<Self> {
        Some(match t {
            "char" | "int8" => Kind::I8,
            "uchar" | "uint8" => Kind::U8,
            "short" | "int16" => Kind::I16,
            "ushort" | "uint16" => Kind::U16,
            "int" | "int32" => Kind::I32,
            "uint" | "uint32" => Kind::U32,
            "float" | "float32" => Kind::F32,
            "double" | "float64" => Kind::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Kind::I8 | Kind::U8 => 1,
            Kind::I16 | Kind::U16 => 2,
            Kind::I32 | Kind::U32 | Kind::F32 => 4,
            Kind::F64 => 8,
        }
    }

    fn is_float(self) -> bool {
        matches!(self, Kind::F32 | Kind::F64)
    }

    fn read(self, b: &[u8], big: bool) -> f64 {
        macro_rules! num {
            ($t:ty, $n:expr) => {{
                let a: [u8; $n] = b[..$n].try_into().unwrap_or([0; $n]);
                f64::from(if big {
                    <$t>::from_be_bytes(a)
                } else {
                    <$t>::from_le_bytes(a)
                })
            }};
        }
        match self {
            Kind::I8 => f64::from(b[0] as i8),
            Kind::U8 => f64::from(b[0]),
            Kind::I16 => num!(i16, 2),
            Kind::U16 => num!(u16, 2),
            Kind::I32 => num!(i32, 4),
            Kind::U32 => num!(u32, 4),
            Kind::F32 => num!(f32, 4),
            Kind::F64 => num!(f64, 8),
        }
    }
}

struct Prop {
    name: String,
    kind: Kind,
    offset: usize,
}

struct Header {
    format: Format,
    vertices: u64,
    props: Vec<Prop>,
    stride: usize,
}

fn bad(what: &str) -> Error {
    Error::Format(format!("PLY: {what}"))
}

fn read_header<R: BufRead>(r: &mut R) -> Result<Header> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    if line.trim() != "ply" {
        return Err(bad("no empieza por «ply»"));
    }
    let (mut format, mut vertices, mut props, mut stride) = (None, None, Vec::new(), 0usize);
    let mut element = String::new();
    let mut elements_seen = 0;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Err(bad("la cabecera no termina"));
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["end_header"] => break,
            ["format", f, _] => {
                format = Some(match *f {
                    "ascii" => Format::Ascii,
                    "binary_little_endian" => Format::Little,
                    "binary_big_endian" => Format::Big,
                    other => return Err(bad(&format!("formato «{other}» desconocido"))),
                })
            }
            ["element", name, n] => {
                elements_seen += 1;
                element = (*name).to_string();
                if element == "vertex" {
                    if elements_seen != 1 {
                        return Err(Error::Unsupported(
                            "PLY: los vértices deben ser el primer elemento".into(),
                        ));
                    }
                    vertices = Some(n.parse::<u64>().map_err(|_| bad("número de vértices"))?);
                }
            }
            ["property", "list", ..] if element == "vertex" => {
                return Err(Error::Unsupported(
                    "PLY: propiedades en lista dentro de los vértices".into(),
                ));
            }
            ["property", t, name] if element == "vertex" => {
                let kind = Kind::parse(t).ok_or_else(|| bad(&format!("tipo «{t}» desconocido")))?;
                props.push(Prop {
                    name: (*name).to_string(),
                    kind,
                    offset: stride,
                });
                stride += kind.size();
            }
            _ => {}
        }
    }
    let (Some(format), Some(vertices)) = (format, vertices) else {
        return Err(bad("falta el formato o el elemento «vertex»"));
    };
    for need in ["x", "y", "z"] {
        if !props.iter().any(|p| p.name == need) {
            return Err(bad(&format!("los vértices no tienen «{need}»")));
        }
    }
    Ok(Header {
        format,
        vertices,
        props,
        stride,
    })
}

/// One vertex: position, intensity (as it is in the file) and colour (as in the file).
struct Vertex {
    x: f64,
    y: f64,
    z: f64,
    intensity: Option<f64>,
    rgb: Option<[f64; 3]>,
}

struct Columns {
    x: usize,
    y: usize,
    z: usize,
    intensity: Option<usize>,
    rgb: Option<[usize; 3]>,
}

impl Columns {
    fn of(h: &Header) -> Self {
        let at = |names: &[&str]| {
            h.props
                .iter()
                .position(|p| names.contains(&p.name.as_str()))
        };
        let find = |n: &str| at(&[n]).unwrap_or(0);
        let colour = |a: &str, b: &str| at(&[a, b]);
        Self {
            x: find("x"),
            y: find("y"),
            z: find("z"),
            intensity: at(&[
                "intensity",
                "scalar_intensity",
                "scalar_Intensity",
                "reflectance",
            ]),
            rgb: match (
                colour("red", "r"),
                colour("green", "g"),
                colour("blue", "b"),
            ) {
                (Some(r), Some(g), Some(b)) => Some([r, g, b]),
                _ => None,
            },
        }
    }
}

/// Calls `f` on each vertex, in file order.
fn each_vertex<R: BufRead>(
    r: &mut R,
    h: &Header,
    cols: &Columns,
    mut f: impl FnMut(Vertex) -> Result<()>,
) -> Result<()> {
    match h.format {
        Format::Ascii => {
            let mut line = String::new();
            for i in 0..h.vertices {
                line.clear();
                if r.read_line(&mut line)? == 0 {
                    return Err(bad("el archivo termina antes de los vértices que anuncia"));
                }
                let v: Vec<f64> = line
                    .split_whitespace()
                    .map(|t| t.parse::<f64>().unwrap_or(f64::NAN))
                    .collect();
                if v.len() < h.props.len() {
                    return Err(bad(&format!(
                        "la línea del vértice {} está incompleta",
                        i + 1
                    )));
                }
                f(Vertex {
                    x: v[cols.x],
                    y: v[cols.y],
                    z: v[cols.z],
                    intensity: cols.intensity.map(|k| v[k]),
                    rgb: cols.rgb.map(|c| [v[c[0]], v[c[1]], v[c[2]]]),
                })?;
                if i % 65_536 == 0 {
                    check_cancel()?;
                }
            }
        }
        Format::Little | Format::Big => {
            let big = h.format == Format::Big;
            let per = (1usize << 20) / h.stride.max(1);
            let mut buf = vec![0u8; per.max(1) * h.stride];
            let mut left = h.vertices;
            while left > 0 {
                check_cancel()?;
                let n = left.min(per.max(1) as u64) as usize;
                r.read_exact(&mut buf[..n * h.stride])
                    .map_err(|_| bad("el archivo termina antes de los vértices que anuncia"))?;
                for rec in buf[..n * h.stride].chunks_exact(h.stride) {
                    let val = |k: usize| h.props[k].kind.read(&rec[h.props[k].offset..], big);
                    f(Vertex {
                        x: val(cols.x),
                        y: val(cols.y),
                        z: val(cols.z),
                        intensity: cols.intensity.map(val),
                        rgb: cols.rgb.map(|c| [val(c[0]), val(c[1]), val(c[2])]),
                    })?;
                }
                left -= n as u64;
            }
        }
    }
    Ok(())
}

/// What a conversion did.
#[derive(Clone, Debug, PartialEq)]
pub struct PlyReport {
    pub points: u64,
    pub colour: bool,
    pub intensity: bool,
    /// Points left out for not having finite coordinates.
    pub skipped: u64,
}

/// A header-only LAS file to take the layout of the output from.
pub(crate) fn template(
    dir: &Path,
    format: u8,
    scale: f64,
    offset: [f64; 3],
    epsg: Option<u16>,
) -> Result<Reader> {
    let rec_len: u16 = if format == 2 { 26 } else { 20 };
    let mut h = vec![0u8; 227];
    h[..4].copy_from_slice(b"LASF");
    h[24] = 1;
    h[25] = 2;
    h[94..96].copy_from_slice(&227u16.to_le_bytes());
    h[96..100].copy_from_slice(&227u32.to_le_bytes());
    h[104] = format;
    h[105..107].copy_from_slice(&rec_len.to_le_bytes());
    for k in 0..3 {
        h[131 + 8 * k..139 + 8 * k].copy_from_slice(&scale.to_le_bytes());
        h[155 + 8 * k..163 + 8 * k].copy_from_slice(&offset[k].to_le_bytes());
    }
    for (at, v) in [
        (179, 0.0f64),
        (187, 0.0),
        (195, 0.0),
        (203, 0.0),
        (211, 0.0),
        (219, 0.0),
    ] {
        h[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    let path = dir.join(format!(
        "rl_ply_template_{}_{format}.las",
        std::process::id()
    ));
    std::fs::write(&path, &h)?;
    let r = Reader::open(&path);
    let _ = std::fs::remove_file(&path);
    let r = r?;
    Ok(match epsg {
        Some(code) => r.with_projection(&epsg_record(code), 1, Some(code), None),
        None => r,
    })
}

/// Converts a PLY file to LAS or LAZ (by the output's extension). `epsg` is the coordinate system
/// the points are in, which a PLY does not say.
pub fn convert(input: &Path, output: &Path, epsg: Option<u16>) -> Result<PlyReport> {
    let mut r = BufReader::with_capacity(1 << 20, File::open(input)?);
    let header = read_header(&mut r)?;
    let cols = Columns::of(&header);
    // pass 1: the extent
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let mut finite = 0u64;
    each_vertex(&mut r, &header, &cols, |v| {
        if [v.x, v.y, v.z].iter().all(|c| c.is_finite()) {
            finite += 1;
            for (k, c) in [v.x, v.y, v.z].into_iter().enumerate() {
                lo[k] = lo[k].min(c);
                hi[k] = hi[k].max(c);
            }
        }
        Ok(())
    })?;
    if finite == 0 {
        return Err(bad("no hay ningún vértice con coordenadas"));
    }
    // millimetres, unless the cloud is so big that they would not fit the integers
    let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    let scale = if span < 2.0e6 { 0.001 } else { 0.01 };
    if span / scale > 2.0e9 {
        return Err(bad("la nube es demasiado grande para coordenadas enteras"));
    }
    let offset = [lo[0].floor(), lo[1].floor(), lo[2].floor()];
    let colour = cols.rgb.is_some();
    let dir = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(std::env::temp_dir, Path::to_path_buf);
    crate::paths::ensure_parent(output)?;
    let tpl = template(&dir, if colour { 2 } else { 0 }, scale, offset, epsg)?;
    let mut w = Writer::create_with(output, &tpl, &WriterOptions::default())?;
    // how the intensity and colour are scaled to 16 bits
    let (int_kind, rgb_kind) = (
        cols.intensity.map(|k| header.props[k].kind),
        cols.rgb.map(|c| header.props[c[0]].kind),
    );
    let mut r = BufReader::with_capacity(1 << 20, File::open(input)?);
    let _ = read_header(&mut r)?;
    // intensity in 0..1 floats becomes 0..65535; integers are kept (clamped)
    let int_scale = if int_kind.is_some_and(Kind::is_float) {
        65535.0
    } else {
        1.0
    };
    let mut rec = vec![0u8; if colour { 26 } else { 20 }];
    let to_int = |v: f64, lo: f64| ((v - lo) / scale).round().clamp(-2.147e9, 2.147e9) as i32;
    let mut written = 0u64;
    let mut max_int = 0.0f64;
    each_vertex(&mut r, &header, &cols, |v| {
        if ![v.x, v.y, v.z].iter().all(|c| c.is_finite()) {
            return Ok(());
        }
        rec[0..4].copy_from_slice(&to_int(v.x, offset[0]).to_le_bytes());
        rec[4..8].copy_from_slice(&to_int(v.y, offset[1]).to_le_bytes());
        rec[8..12].copy_from_slice(&to_int(v.z, offset[2]).to_le_bytes());
        let i = v
            .intensity
            .map_or(0.0, |i| (i * int_scale).clamp(0.0, 65535.0));
        max_int = max_int.max(i);
        rec[12..14].copy_from_slice(&(i.round() as u16).to_le_bytes());
        // first of one return, unclassified
        rec[14] = 0x09;
        rec[15] = 1;
        if let Some(c) = v.rgb {
            let k = match rgb_kind {
                Some(Kind::U8 | Kind::I8) => 257.0,
                Some(Kind::F32 | Kind::F64) => 65535.0,
                _ => 1.0,
            };
            for (n, comp) in c.iter().enumerate() {
                let q = (comp * k).clamp(0.0, 65535.0).round() as u16;
                rec[20 + 2 * n..22 + 2 * n].copy_from_slice(&q.to_le_bytes());
            }
        }
        w.write(&rec)?;
        written += 1;
        Ok(())
    })?;
    w.finish()?;
    Ok(PlyReport {
        points: written,
        colour,
        intensity: cols.intensity.is_some(),
        skipped: header.vertices - written,
    })
}

/// The path of a conversion made for opening: `<name>.laz` in the folder given.
pub fn converted_path(input: &Path, dir: &Path) -> PathBuf {
    dir.join(format!("{}.laz", crate::paths::stem(input)))
}

/// A short text for the report of a conversion.
pub fn describe(r: &PlyReport, output: &Path) -> String {
    let mut s = format!("{} puntos", r.points);
    if r.colour {
        s.push_str(" con color");
    }
    if r.intensity {
        s.push_str(" e intensidad");
    }
    if r.skipped > 0 {
        s.push_str(&format!(
            " ({} vértices sin coordenadas, descartados)",
            r.skipped
        ));
    }
    format!("{s}: {}", output.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rl_ply_{}_{name}", std::process::id()))
    }

    /// x, y, z, intensity and colour of a point read back.
    type Back = (f64, f64, f64, u16, Option<[u16; 3]>);

    fn read_back(path: &Path) -> Result<Vec<Back>> {
        let r = Reader::open(path)?;
        let rec = usize::from(r.header.record_len);
        let mut out = Vec::new();
        r.scan_raw(|bytes| {
            for b in bytes.chunks_exact(rec) {
                let p = r.decode(b).ok_or_else(|| bad("registro"))?;
                let rgb = (rec == 26).then(|| {
                    [
                        u16::from_le_bytes([b[20], b[21]]),
                        u16::from_le_bytes([b[22], b[23]]),
                        u16::from_le_bytes([b[24], b[25]]),
                    ]
                });
                out.push((p.x, p.y, p.z, p.intensity, rgb));
            }
            Ok(())
        })?;
        Ok(out)
    }

    #[test]
    fn ascii_binary_and_big_endian_give_the_same_points() -> Result<()> {
        let pts: [(f64, f64, f64, u8, u8, u8, u8); 2] = [
            (576_010.123, 4_828_020.456, 301.789, 200, 10, 20, 30),
            (576_011.5, 4_828_021.25, 302.0, 250, 255, 0, 128),
        ];
        // ASCII, with an intensity and normals that are left out
        let mut text = String::from(
            "ply\nformat ascii 1.0\ncomment hecho a mano\nelement vertex 2\nproperty double x\nproperty double y\nproperty double z\nproperty float nx\nproperty uchar intensity\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 0\nproperty list uchar int vertex_indices\nend_header\n",
        );
        for p in &pts {
            text.push_str(&format!(
                "{} {} {} 0.0 {} {} {} {}\n",
                p.0, p.1, p.2, p.3, p.4, p.5, p.6
            ));
        }
        let ascii = temp("a.ply");
        std::fs::write(&ascii, &text)?;
        // binary in both byte orders
        let mut files = vec![ascii.clone()];
        for (name, big) in [("l.ply", false), ("b.ply", true)] {
            let mut bytes = format!(
                "ply\nformat {} 1.0\nelement vertex 2\nproperty double x\nproperty double y\nproperty double z\nproperty float nx\nproperty uchar intensity\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n",
                if big { "binary_big_endian" } else { "binary_little_endian" }
            )
            .into_bytes();
            for p in &pts {
                for v in [p.0, p.1, p.2] {
                    bytes.extend(if big {
                        v.to_be_bytes()
                    } else {
                        v.to_le_bytes()
                    });
                }
                bytes.extend(if big {
                    0f32.to_be_bytes()
                } else {
                    0f32.to_le_bytes()
                });
                bytes.extend([p.3, p.4, p.5, p.6]);
            }
            let path = temp(name);
            std::fs::write(&path, bytes)?;
            files.push(path);
        }
        let mut results = Vec::new();
        for f in &files {
            let out = f.with_extension("laz");
            let rep = convert(f, &out, Some(25830))?;
            assert_eq!((rep.points, rep.colour, rep.intensity), (2, true, true));
            assert_eq!(Reader::open(&out)?.header.epsg, Some(25830));
            results.push(read_back(&out)?);
            let _ = std::fs::remove_file(&out);
            let _ = std::fs::remove_file(f);
        }
        for r in &results {
            assert_eq!(r.len(), 2);
            assert!((r[0].0 - 576_010.123).abs() < 1e-3 && (r[0].1 - 4_828_020.456).abs() < 1e-3);
            assert!((r[0].2 - 301.789).abs() < 1e-3);
            assert_eq!(r[1].3, 250);
            // 8-bit colours become 16-bit (×257)
            assert_eq!(r[1].4, Some([255 * 257, 0, 128 * 257]));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0], results[2]);
        Ok(())
    }

    #[test]
    fn broken_files_are_refused() -> Result<()> {
        let path = temp("bad.ply");
        for (text, what) in [
            ("no es ply\n", "ply"),
            (
                "ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nend_header\n1 2\n",
                "z",
            ),
            (
                "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nend_header\n1 2 3\n",
                "termina",
            ),
            (
                "ply\nformat ascii 1.0\nelement vertex 1\nproperty list uchar int x\nend_header\n",
                "lista",
            ),
        ] {
            std::fs::write(&path, text)?;
            let e = convert(&path, &temp("bad.laz"), None)
                .err()
                .map(|e| e.to_string());
            assert!(
                e.as_deref().is_some_and(|e| e.contains(what)),
                "{what}: {e:?}"
            );
        }
        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}
