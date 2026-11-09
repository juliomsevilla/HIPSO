//! KML and KMZ layers: read for display and as regions, and written from any vector layer.
//!
//! KML is always longitude/latitude (WGS 84, EPSG:4326), so a layer is read in that system and
//! the program moves it to the one of the work, as it does with any other layer; on writing the
//! vertices are moved to EPSG:4326 first. A KMZ is a zip holding a KML (`doc.kml`). Altitudes are
//! ignored. Placemarks give the features: `name` and `description` become fields of those names,
//! and `ExtendedData` (`Data` and `SimpleData`) the rest. Styles, folders and overlays are not read.

use std::io::Write;
use std::path::Path;

use crate::geojson::{Feat, Layer, orient_rings};
use crate::view::{GeomKind, VectorData};
use crate::{Error, Result};

/// Largest KML accepted once inflated from a KMZ (the files are hostile until proven otherwise).
const MAX_KML: u64 = 512 << 20;

fn is_kmz(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("kmz"))
}

/// The KML text of a `.kml` or `.kmz` file.
fn text_of_file(path: &Path) -> Result<String> {
    if !is_kmz(path) {
        let bytes = std::fs::read(path)?;
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    let zip = std::fs::read(path)?;
    let parts = crate::xlsx::zip_parts(&zip, |n| n.to_ascii_lowercase().ends_with(".kml"))
        .map_err(|_| Error::Format(format!("{}: no es un KMZ (zip con un KML dentro)", path.display())))?;
    let best = parts
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("doc.kml"))
        .or_else(|| parts.first())
        .ok_or_else(|| Error::Format(format!("{}: el KMZ no contiene ningún KML", path.display())))?;
    if best.1.len() as u64 > MAX_KML {
        return Err(Error::Format(format!("{}: KML demasiado grande", path.display())));
    }
    Ok(String::from_utf8_lossy(&best.1).into_owned())
}

// ------------------------------------------------------------------------------------- reading

enum Ev<'a> {
    Start(&'a str, &'a str),
    End(&'a str),
    Text(String),
}

/// The name of a tag without its namespace prefix.
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The value of attribute `name` in the text of a start tag (single or double quotes).
fn attr_of(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before_ok = rest[..i].chars().next_back().is_none_or(char::is_whitespace);
        let after = rest[i + name.len()..].trim_start();
        if before_ok && let Some(v) = after.strip_prefix('=') {
            let v = v.trim_start();
            let q = v.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            let end = v[1..].find(q)?;
            return Some(crate::xlsx::unescape(&v[1..1 + end]));
        }
        rest = &rest[i + name.len()..];
    }
    None
}

/// Walks the tags and texts of an XML document.
fn walk<'a>(xml: &'a str, mut f: impl FnMut(Ev<'a>)) {
    let mut rest = xml;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            f(Ev::Text(crate::xlsx::unescape(rest)));
            return;
        };
        if lt > 0 {
            f(Ev::Text(crate::xlsx::unescape(&rest[..lt])));
        }
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map_or("", |e| &r[e + 3..]);
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let e = r.find("]]>").unwrap_or(r.len());
            f(Ev::Text(r[..e].to_string()));
            rest = r.get(e + 3..).unwrap_or("");
        } else if let Some(r) = rest.strip_prefix("<?") {
            rest = r.find("?>").map_or("", |e| &r[e + 2..]);
        } else if let Some(r) = rest.strip_prefix("<!") {
            rest = r.find('>').map_or("", |e| &r[e + 1..]);
        } else if let Some(r) = rest.strip_prefix("</") {
            let e = r.find('>').unwrap_or(r.len());
            f(Ev::End(local(r[..e].trim())));
            rest = r.get(e + 1..).unwrap_or("");
        } else {
            let r = &rest[1..];
            // the end of the tag: the first `>` outside quotes
            let mut quote = None;
            let mut end = r.len();
            for (i, c) in r.char_indices() {
                match (quote, c) {
                    (None, '"' | '\'') => quote = Some(c),
                    (Some(q), c) if c == q => quote = None,
                    (None, '>') => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            let body = &r[..end];
            let self_closing = body.ends_with('/');
            let body = body.strip_suffix('/').unwrap_or(body);
            let name_end = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
            let name = local(&body[..name_end]);
            f(Ev::Start(name, &body[name_end..]));
            if self_closing {
                f(Ev::End(name));
            }
            rest = r.get(end + 1..).unwrap_or("");
        }
    }
}

/// The positions of a `coordinates` text: `lon,lat[,alt]` separated by white space.
fn positions(t: &str) -> Vec<[f64; 2]> {
    t.split_whitespace()
        .filter_map(|c| {
            let mut it = c.split(',');
            let x: f64 = it.next()?.trim().parse().ok()?;
            let y: f64 = it.next()?.trim().parse().ok()?;
            (x.is_finite() && y.is_finite()).then_some([x, y])
        })
        .collect()
}

#[derive(Default)]
struct Placemark {
    props: Vec<(String, String)>,
    geoms: Vec<(GeomKind, Vec<Vec<[f64; 2]>>)>,
    /// The rings of the polygon being read.
    poly: Option<Vec<Vec<[f64; 2]>>>,
    /// The field named by the `Data` or `SimpleData` being read.
    data: Option<String>,
}

/// The layer in a KML text.
fn parse(xml: &str) -> Layer {
    let mut layer = Layer { kind: GeomKind::Point, feats: Vec::new(), epsg: Some(4326), skipped: 0 };
    let mut first = true;
    let mut stack: Vec<&str> = Vec::new();
    let mut cur: Option<Placemark> = None;
    walk(xml, |ev| match ev {
        Ev::Start(name, attrs) => {
            stack.push(name);
            match name {
                "Placemark" => cur = Some(Placemark::default()),
                "Polygon" => {
                    if let Some(c) = &mut cur {
                        c.poly = Some(Vec::new());
                    }
                }
                "Data" | "SimpleData" => {
                    if let Some(c) = &mut cur {
                        c.data = attr_of(attrs, "name");
                    }
                }
                _ => {}
            }
        }
        Ev::End(name) => {
            while let Some(top) = stack.pop() {
                if top == name {
                    break;
                }
            }
            match name {
                "Polygon" => {
                    if let Some(c) = &mut cur
                        && let Some(rings) = c.poly.take()
                        && !rings.is_empty()
                    {
                        c.geoms.push((GeomKind::Polygon, orient_rings(rings)));
                    }
                }
                "Data" | "SimpleData" => {
                    if let Some(c) = &mut cur {
                        c.data = None;
                    }
                }
                "Placemark" => {
                    if let Some(pm) = cur.take() {
                        place(&mut layer, &mut first, pm);
                    }
                }
                _ => {}
            }
        }
        Ev::Text(t) => {
            let (Some(c), Some(&top)) = (&mut cur, stack.last()) else { return };
            let parent = stack.len().checked_sub(2).and_then(|i| stack.get(i)).copied().unwrap_or("");
            match top {
                "coordinates" => {
                    let pts = positions(&t);
                    match parent {
                        "Point" => {
                            if let Some(p) = pts.first() {
                                c.geoms.push((GeomKind::Point, vec![vec![*p]]));
                            }
                        }
                        "LineString" if pts.len() >= 2 => c.geoms.push((GeomKind::Line, vec![pts])),
                        "LinearRing" if pts.len() >= 3 => {
                            if let Some(p) = &mut c.poly {
                                p.push(pts);
                            }
                        }
                        _ => {}
                    }
                }
                "name" | "description" if parent == "Placemark" => {
                    let t = t.trim();
                    if !t.is_empty() {
                        c.props.push((top.to_string(), t.to_string()));
                    }
                }
                "value" if parent == "Data" => {
                    if let Some(n) = &c.data {
                        c.props.push((n.clone(), t.trim().to_string()));
                    }
                }
                "SimpleData" => {
                    if let Some(n) = &c.data {
                        c.props.push((n.clone(), t.trim().to_string()));
                    }
                }
                _ => {}
            }
        }
    });
    layer
}

/// Adds the features of a placemark to the layer: a point per point, one feature for the lines
/// or the rings. Geometry of another kind than the layer's is left out and counted.
fn place(layer: &mut Layer, first: &mut bool, pm: Placemark) {
    let Some(kind) = pm.geoms.first().map(|g| g.0) else {
        layer.skipped += 1;
        return;
    };
    let mut parts = Vec::new();
    for (k, p) in pm.geoms {
        if k == kind {
            parts.extend(p);
        } else {
            layer.skipped += 1;
        }
    }
    if *first {
        (layer.kind, *first) = (kind, false);
    }
    if kind != layer.kind {
        layer.skipped += 1;
        return;
    }
    if kind == GeomKind::Point {
        for p in parts {
            layer.feats.push(Feat { parts: vec![p], props: pm.props.clone() });
        }
    } else {
        layer.feats.push(Feat { parts, props: pm.props });
    }
}

pub(crate) fn read_layer(path: &Path) -> Result<Layer> {
    let text = text_of_file(path)?;
    if !text.contains("<kml") && !text.contains("<Placemark") {
        return Err(Error::Format(format!("{}: no es un KML", path.display())));
    }
    Ok(parse(&text))
}

/// A KML or KMZ layer for display (always in EPSG:4326), with the number of features left out.
pub fn read_vector(path: &Path) -> Result<(VectorData, GeomKind, Option<u16>, usize)> {
    Ok(crate::geojson::vector_of(read_layer(path)?))
}

/// The polygons of a KML or KMZ as regions.
pub fn read_regions(path: &Path, opts: &crate::vector::ReadOptions) -> Result<Vec<crate::vector::Region>> {
    crate::geojson::regions_of(&read_layer(path)?, opts)
}

// ------------------------------------------------------------------------------------- writing

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

fn coords(pts: &[[f64; 2]]) -> String {
    pts.iter().map(|p| format!("{},{},0", p[0], p[1])).collect::<Vec<_>>().join(" ")
}

fn geometry(parts: &[Vec<[f64; 2]>], kind: GeomKind) -> String {
    let mut items: Vec<String> = Vec::new();
    match kind {
        GeomKind::Point => {
            for p in parts.iter().flatten() {
                items.push(format!("<Point><coordinates>{},{},0</coordinates></Point>", p[0], p[1]));
            }
        }
        GeomKind::Line => {
            for l in parts.iter().filter(|l| l.len() >= 2) {
                items.push(format!("<LineString><coordinates>{}</coordinates></LineString>", coords(l)));
            }
        }
        GeomKind::Polygon => {
            for poly in crate::geojson::polygons_of(parts) {
                let ring = |r: &Vec<[f64; 2]>| format!("<LinearRing><coordinates>{}</coordinates></LinearRing>", coords(r));
                let mut s = format!("<Polygon><outerBoundaryIs>{}</outerBoundaryIs>", ring(&poly[0]));
                for h in &poly[1..] {
                    s.push_str(&format!("<innerBoundaryIs>{}</innerBoundaryIs>", ring(h)));
                }
                s.push_str("</Polygon>");
                items.push(s);
            }
        }
    }
    match items.len() {
        0 => String::new(),
        1 => items.remove(0),
        _ => format!("<MultiGeometry>{}</MultiGeometry>", items.concat()),
    }
}

/// The KML text of a layer whose vertices are already longitude and latitude.
fn kml_text(d: &VectorData, kind: GeomKind, title: &str) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<kml xmlns=\"http://www.opengis.net/kml/2.2\">\n<Document>\n",
    );
    s.push_str(&format!("<name>{}</name>\n", esc(title)));
    let find = |want: &str| d.fields.iter().position(|f| f.trim().eq_ignore_ascii_case(want));
    let (name_col, desc_col) = (find("name"), find("description"));
    for (i, f) in d.features.iter().enumerate() {
        let geom = geometry(&f.parts, kind);
        if geom.is_empty() {
            continue;
        }
        let value = |c: usize| d.text.get(i).and_then(|r| r.get(c)).map_or("", String::as_str);
        s.push_str("<Placemark>");
        if let Some(c) = name_col {
            s.push_str(&format!("<name>{}</name>", esc(value(c))));
        }
        if let Some(c) = desc_col {
            s.push_str(&format!("<description>{}</description>", esc(value(c))));
        }
        let extra: Vec<usize> = (0..d.fields.len()).filter(|c| Some(*c) != name_col && Some(*c) != desc_col).collect();
        if !extra.is_empty() {
            s.push_str("<ExtendedData>");
            for c in extra {
                s.push_str(&format!("<Data name=\"{}\"><value>{}</value></Data>", esc(&d.fields[c]), esc(value(c))));
            }
            s.push_str("</ExtendedData>");
        }
        s.push_str(&geom);
        s.push_str("</Placemark>\n");
    }
    s.push_str("</Document>\n</kml>\n");
    s
}

/// A zip holding one file, deflated.
fn zip_one(name: &str, data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data)?;
    let packed = enc.finish()?;
    let mut crc = flate2::Crc::new();
    crc.update(data);
    let (crc, csize, usize_, nl) = (crc.sum(), packed.len() as u32, data.len() as u32, name.len() as u16);
    let mut z = Vec::new();
    let head = |z: &mut Vec<u8>| {
        z.extend_from_slice(&20u16.to_le_bytes()); // version needed
        z.extend_from_slice(&0x0800u16.to_le_bytes()); // names in UTF-8
        z.extend_from_slice(&8u16.to_le_bytes()); // deflate
        z.extend_from_slice(&0u16.to_le_bytes()); // time
        z.extend_from_slice(&33u16.to_le_bytes()); // date: 1980-01-01
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&csize.to_le_bytes());
        z.extend_from_slice(&usize_.to_le_bytes());
        z.extend_from_slice(&nl.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes()); // extra
    };
    z.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
    head(&mut z);
    z.extend_from_slice(name.as_bytes());
    z.extend_from_slice(&packed);
    let cd_at = z.len() as u32;
    z.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
    z.extend_from_slice(&20u16.to_le_bytes()); // version made by
    head(&mut z);
    z.extend_from_slice(&0u16.to_le_bytes()); // comment
    z.extend_from_slice(&0u16.to_le_bytes()); // disk
    z.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
    z.extend_from_slice(&0u32.to_le_bytes()); // external attributes
    z.extend_from_slice(&0u32.to_le_bytes()); // offset of the local header
    z.extend_from_slice(name.as_bytes());
    let cd_size = z.len() as u32 - cd_at;
    z.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    z.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
    z.extend_from_slice(&cd_size.to_le_bytes());
    z.extend_from_slice(&cd_at.to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes());
    Ok(z)
}

/// Writes a layer as KML or KMZ (by the extension). `epsg` is the system of its vertices: they
/// are moved to longitude/latitude; without it they must already look like that.
pub fn write_vector(path: &Path, d: &VectorData, kind: GeomKind, epsg: Option<u16>) -> Result<()> {
    let moved;
    let data = match epsg {
        Some(4326) => d,
        Some(e) => {
            let mut c = d.clone();
            crate::reproject::vector_data(&mut c, e, 4326)?;
            moved = c;
            &moved
        }
        None => {
            let b = d.bounds;
            let inside = b.iter().all(|v| v.is_finite()) && b[0] >= -180.0 && b[2] <= 180.0 && b[1] >= -90.0 && b[3] <= 90.0;
            if !d.features.is_empty() && !inside {
                return Err(Error::Param(
                    "la capa no declara su sistema de coordenadas y KML necesita longitud y latitud; guárdala como shapefile o asígnale uno".into(),
                ));
            }
            d
        }
    };
    let title = crate::paths::stem(path);
    let text = kml_text(data, kind, &title);
    crate::paths::ensure_parent(path)?;
    if is_kmz(path) {
        std::fs::write(path, zip_one("doc.kml", text.as_bytes())?)?;
    } else {
        std::fs::write(path, text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Feature;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("hipso_kml_{}_{name}", std::process::id()))
    }

    fn sample() -> VectorData {
        let outer = vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0], [0.0, 0.0]];
        let hole = vec![[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0], [2.0, 2.0]];
        VectorData {
            features: vec![Feature { parts: vec![outer, hole] }],
            polygons: true,
            fields: vec!["name".into(), "rodal".into()],
            text: vec![vec!["Monte <A> & B".into(), "7".into()]],
            num: vec![vec![f64::NAN, 7.0]],
            bounds: [0.0, 0.0, 10.0, 10.0],
            ..VectorData::default()
        }
    }

    #[test]
    fn a_polygon_with_a_hole_and_its_fields_round_trip() -> Result<()> {
        for ext in ["kml", "kmz"] {
            let p = temp(&format!("rt.{ext}"));
            write_vector(&p, &sample(), GeomKind::Polygon, Some(4326))?;
            let (d, kind, epsg, skipped) = read_vector(&p)?;
            std::fs::remove_file(&p).ok();
            assert_eq!((kind, epsg, skipped), (GeomKind::Polygon, Some(4326), 0));
            assert_eq!(d.features.len(), 1);
            assert_eq!(d.features[0].parts.len(), 2);
            let col = |n: &str| d.fields.iter().position(|f| f == n);
            assert_eq!(d.text[0][col("name").ok_or(Error::Param("name".into()))?], "Monte <A> & B");
            assert_eq!(d.text[0][col("rodal").ok_or(Error::Param("rodal".into()))?], "7");
        }
        Ok(())
    }

    #[test]
    fn google_earth_files_with_namespaces_and_cdata_are_read() {
        let xml = r#"<?xml version="1.0"?>
<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2">
<Document><Folder><name>carpeta</name>
<Placemark><name>uno</name><description><![CDATA[<b>hola</b>]]></description>
<ExtendedData><Data name='tipo'><value>pino</value></Data></ExtendedData>
<Point><coordinates>-3.7,40.4,650</coordinates></Point></Placemark>
<Placemark><name>dos</name><MultiGeometry>
<Point><coordinates>-3.0,40.0</coordinates></Point><Point><coordinates>-3.1,40.1</coordinates></Point>
<LineString><coordinates>0,0 1,1</coordinates></LineString></MultiGeometry></Placemark>
</Folder></Document></kml>"#;
        let l = parse(xml);
        // 1 point + 2 points; the line is of another kind
        assert_eq!(l.feats.len(), 3);
        assert_eq!(l.skipped, 1);
        assert_eq!(l.feats[0].props.iter().find(|p| p.0 == "description").map(|p| p.1.as_str()), Some("<b>hola</b>"));
        assert_eq!(l.feats[0].props.iter().find(|p| p.0 == "tipo").map(|p| p.1.as_str()), Some("pino"));
        assert_eq!(l.feats[0].parts, vec![vec![[-3.7, 40.4]]]);
    }

    #[test]
    fn utm_layers_are_moved_to_longitude_latitude() -> Result<()> {
        // 30N, near Madrid
        let d = VectorData {
            features: vec![Feature { parts: vec![vec![[440000.0, 4474000.0]]] }],
            points: true,
            bounds: [440000.0, 4474000.0, 440000.0, 4474000.0],
            ..VectorData::default()
        };
        let p = temp("utm.kml");
        write_vector(&p, &d, GeomKind::Point, Some(25830))?;
        let (r, ..) = read_vector(&p)?;
        std::fs::remove_file(&p).ok();
        let [lon, lat] = r.features[0].parts[0][0];
        assert!((-4.0..-3.0).contains(&lon) && (40.0..41.0).contains(&lat), "{lon} {lat}");
        Ok(())
    }

    #[test]
    fn a_projected_layer_without_a_system_is_refused() {
        let mut d = sample();
        d.bounds = [400000.0, 4400000.0, 410000.0, 4410000.0];
        assert!(write_vector(&temp("no.kml"), &d, GeomKind::Polygon, None).is_err());
    }
}

