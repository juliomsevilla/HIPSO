//! GeoJSON layers: read for display and as regions, and written from any vector layer. Positions
//! are kept as they are in the file (no reprojection); the coordinate system is the file's `crs`
//! member when it has one (the form GDAL and QGIS write and read for projected data).

use std::io::{BufWriter, Write};
use std::path::Path;

use crate::json::{Json, quote};
use crate::vector::{Polygon, ReadOptions, Region, Shape2};
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result};

/// One feature: its parts (rings, lines or a single point) and its properties as text.
pub(crate) struct Feat {
    pub(crate) parts: Vec<Vec<[f64; 2]>>,
    pub(crate) props: Vec<(String, String)>,
}

pub(crate) struct Layer {
    pub(crate) kind: GeomKind,
    pub(crate) feats: Vec<Feat>,
    pub(crate) epsg: Option<u16>,
    /// Features left out because their geometry was of another kind than the layer's.
    pub(crate) skipped: usize,
}

fn signed_area(r: &[[f64; 2]]) -> f64 {
    r.iter()
        .zip(r.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.0
}

fn position(j: &Json) -> Option<[f64; 2]> {
    let a = j.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

fn line(j: &Json) -> Option<Vec<[f64; 2]>> {
    j.as_array()?.iter().map(position).collect()
}

/// The rings of a polygon, the exterior (the first) clockwise and the holes counter-clockwise.
pub(crate) fn orient_rings(mut rings: Vec<Vec<[f64; 2]>>) -> Vec<Vec<[f64; 2]>> {
    for (k, ring) in rings.iter_mut().enumerate() {
        if (signed_area(ring) > 0.0) == (k == 0) {
            ring.reverse();
        }
    }
    rings
}

/// The rings of a polygon, the exterior clockwise and the holes counter-clockwise (the shapefile
/// convention the rest of the program uses; GeoJSON files may have either).
fn rings(j: &Json) -> Option<Vec<Vec<[f64; 2]>>> {
    let mut out = Vec::new();
    for (k, r) in j.as_array()?.iter().enumerate() {
        let mut ring = line(r)?;
        if (signed_area(&ring) > 0.0) == (k == 0) {
            ring.reverse();
        }
        out.push(ring);
    }
    Some(out)
}

/// The parts a geometry adds to a layer: for points, one part per point.
fn geometry(g: &Json) -> Option<(GeomKind, Vec<Vec<[f64; 2]>>)> {
    let coords = g.get("coordinates");
    match g.get("type")?.as_str()? {
        "Point" => Some((GeomKind::Point, vec![vec![position(coords?)?]])),
        "MultiPoint" => Some((
            GeomKind::Point,
            coords?
                .as_array()?
                .iter()
                .map(|p| position(p).map(|q| vec![q]))
                .collect::<Option<_>>()?,
        )),
        "LineString" => Some((GeomKind::Line, vec![line(coords?)?])),
        "MultiLineString" => Some((
            GeomKind::Line,
            coords?
                .as_array()?
                .iter()
                .map(line)
                .collect::<Option<_>>()?,
        )),
        "Polygon" => Some((GeomKind::Polygon, rings(coords?)?)),
        "MultiPolygon" => {
            let mut all = Vec::new();
            for p in coords?.as_array()? {
                all.extend(rings(p)?);
            }
            Some((GeomKind::Polygon, all))
        }
        _ => None,
    }
}

fn text_of(v: &Json) -> String {
    match v {
        Json::Null => String::new(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        Json::Str(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

/// The EPSG code a `crs` member names (`urn:ogc:def:crs:EPSG::25830`, `EPSG:25830`).
fn epsg_of_crs(root: &Json) -> Option<u16> {
    let name = root.get("crs")?.get("properties")?.get("name")?.as_str()?;
    let at = name.to_ascii_uppercase().rfind("EPSG")?;
    let digits: String = name[at + 4..]
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn read_layer(path: &Path) -> Result<Layer> {
    let text = std::fs::read_to_string(path)?;
    let root = Json::parse(&text)?;
    drop(text);
    let epsg = epsg_of_crs(&root);
    let items: Vec<&Json> = match root.get("type").and_then(Json::as_str) {
        Some("FeatureCollection") => root
            .get("features")
            .and_then(Json::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default(),
        Some("Feature") => vec![&root],
        _ => {
            return Err(Error::Format(format!(
                "{}: no es un GeoJSON de objetos (FeatureCollection o Feature)",
                path.display()
            )));
        }
    };
    let mut layer = Layer {
        kind: GeomKind::Point,
        feats: Vec::new(),
        epsg,
        skipped: 0,
    };
    let mut first = true;
    for item in items {
        let props: Vec<(String, String)> = match item.get("properties") {
            Some(Json::Obj(m)) => m.iter().map(|(k, v)| (k.clone(), text_of(v))).collect(),
            _ => Vec::new(),
        };
        let Some((kind, parts)) = item.get("geometry").and_then(geometry) else {
            layer.skipped += 1;
            continue;
        };
        if first {
            (layer.kind, first) = (kind, false);
        }
        if kind != layer.kind {
            layer.skipped += 1;
            continue;
        }
        if kind == GeomKind::Point {
            // a multi-point is as many features as points
            for p in parts {
                layer.feats.push(Feat {
                    parts: vec![p],
                    props: props.clone(),
                });
            }
        } else {
            layer.feats.push(Feat { parts, props });
        }
    }
    Ok(layer)
}

/// A GeoJSON layer for display, with the number of features left out (another kind of geometry
/// than the first feature's) and its coordinate system if the file names one.
pub fn read_vector(path: &Path) -> Result<(VectorData, GeomKind, Option<u16>, usize)> {
    Ok(vector_of(read_layer(path)?))
}

/// A layer of any origin as the data the viewer draws.
pub(crate) fn vector_of(layer: Layer) -> (VectorData, GeomKind, Option<u16>, usize) {
    let mut fields: Vec<String> = Vec::new();
    for f in &layer.feats {
        for (k, _) in &f.props {
            if !fields.contains(k) {
                fields.push(k.clone());
            }
        }
    }
    let mut v = VectorData {
        bounds: [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        polygons: layer.kind == GeomKind::Polygon,
        points: layer.kind == GeomKind::Point,
        ..VectorData::default()
    };
    for f in &layer.feats {
        for q in f.parts.iter().flatten() {
            v.bounds = [
                v.bounds[0].min(q[0]),
                v.bounds[1].min(q[1]),
                v.bounds[2].max(q[0]),
                v.bounds[3].max(q[1]),
            ];
        }
        let text: Vec<String> = fields
            .iter()
            .map(|k| {
                f.props
                    .iter()
                    .find(|(n, _)| n == k)
                    .map_or(String::new(), |(_, t)| t.clone())
            })
            .collect();
        v.num.push(
            text.iter()
                .map(|t| t.trim().parse().unwrap_or(f64::NAN))
                .collect(),
        );
        v.text.push(text);
        v.features.push(Feature {
            parts: f.parts.clone(),
        });
    }
    v.fields = fields;
    (v, layer.kind, layer.epsg, layer.skipped)
}

/// The polygons of a GeoJSON file as regions (and points as circles when a radius is given), named
/// as `opts` says.
pub fn read_regions(path: &Path, opts: &ReadOptions) -> Result<Vec<Region>> {
    regions_of(&read_layer(path)?, opts)
}

/// The regions of a layer of any origin.
pub(crate) fn regions_of(layer: &Layer, opts: &ReadOptions) -> Result<Vec<Region>> {
    let mut out = Vec::new();
    for (n, f) in layer.feats.iter().enumerate() {
        let by_name = |name: &str| {
            f.props
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        let name = match &opts.name_field {
            Some(k) => match k.parse::<usize>() {
                Ok(i) if by_name(k).is_none() => {
                    f.props.get(i.saturating_sub(1)).map(|(_, v)| v.clone())
                }
                _ => by_name(k),
            },
            None => None,
        }
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{:03}", n + 1));
        let shape = match layer.kind {
            GeomKind::Polygon => Shape2::Polygon(Polygon::new(f.parts.clone())),
            GeomKind::Point => {
                let r = opts
                    .radius_field
                    .as_deref()
                    .and_then(by_name)
                    .and_then(|v| v.parse::<f64>().ok())
                    .or(opts.radius)
                    .ok_or_else(|| {
                        Error::Param(
                            "las parcelas dadas como puntos necesitan un radio".to_string(),
                        )
                    })?;
                let p = f.parts[0][0];
                Shape2::Circle {
                    cx: p[0],
                    cy: p[1],
                    r,
                }
            }
            GeomKind::Line => continue,
        };
        out.push(Region {
            name,
            shape,
            attrs: f.props.clone(),
        });
    }
    Ok(out)
}

/// The coordinate system a GeoJSON file names, looking only at the start of the file.
pub fn epsg_hint(path: &Path) -> Option<u16> {
    use std::io::Read;
    let mut head = vec![0u8; 8192];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let text = String::from_utf8_lossy(&head[..n]).into_owned();
    let at = text.find("\"crs\"")?;
    let tail = &text[at..];
    let e = tail.to_ascii_uppercase().find("EPSG")?;
    let digits: String = tail[e + 4..]
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn coords(p: &[f64; 2]) -> String {
    format!("[{},{}]", p[0], p[1])
}

fn coord_line(l: &[[f64; 2]]) -> String {
    format!("[{}]", l.iter().map(coords).collect::<Vec<_>>().join(","))
}

/// The geometry of a feature as GeoJSON text. Polygon exteriors come out counter-clockwise and
/// holes clockwise (RFC 7946); a ring turning the other way than the exterior before it is a hole.
fn geometry_text(parts: &[Vec<[f64; 2]>], kind: GeomKind) -> String {
    match kind {
        GeomKind::Point => match parts {
            [one] if one.len() == 1 => {
                format!("{{\"type\":\"Point\",\"coordinates\":{}}}", coords(&one[0]))
            }
            many => format!(
                "{{\"type\":\"MultiPoint\",\"coordinates\":[{}]}}",
                many.iter()
                    .flatten()
                    .map(coords)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        },
        GeomKind::Line => match parts {
            [one] => format!(
                "{{\"type\":\"LineString\",\"coordinates\":{}}}",
                coord_line(one)
            ),
            many => format!(
                "{{\"type\":\"MultiLineString\",\"coordinates\":[{}]}}",
                many.iter()
                    .map(|l| coord_line(l))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        },
        GeomKind::Polygon => {
            // group the rings: an exterior (clockwise in a shapefile) opens a polygon, the
            // counter-clockwise ones that follow are its holes
            let mut polygons: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
            for r in parts.iter().filter(|r| r.len() >= 3) {
                let mut ring = r.clone();
                let exterior = signed_area(&ring) <= 0.0;
                // the program keeps exteriors clockwise and holes counter-clockwise; RFC 7946 wants
                // the opposite
                ring.reverse();
                match (exterior, polygons.last_mut()) {
                    (false, Some(last)) => last.push(ring),
                    _ => polygons.push(vec![ring]),
                }
            }
            let poly = |p: &Vec<Vec<[f64; 2]>>| {
                format!(
                    "[{}]",
                    p.iter()
                        .map(|r| coord_line(r))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            if polygons.len() == 1 {
                format!(
                    "{{\"type\":\"Polygon\",\"coordinates\":{}}}",
                    poly(&polygons[0])
                )
            } else {
                format!(
                    "{{\"type\":\"MultiPolygon\",\"coordinates\":[{}]}}",
                    polygons.iter().map(poly).collect::<Vec<_>>().join(",")
                )
            }
        }
    }
}

/// Writes a layer as GeoJSON, one feature per line so that a large file can be read by streaming
/// tools. `epsg` becomes the `crs` member.
pub fn write_vector(path: &Path, d: &VectorData, kind: GeomKind, epsg: Option<u16>) -> Result<()> {
    let mut out = BufWriter::new(std::fs::File::create(path)?);
    write!(out, "{{\"type\":\"FeatureCollection\"")?;
    if let Some(e) = epsg {
        write!(
            out,
            ",\"crs\":{{\"type\":\"name\",\"properties\":{{\"name\":\"urn:ogc:def:crs:EPSG::{e}\"}}}}"
        )?;
    }
    writeln!(out, ",\"features\":[")?;
    let mut first = true;
    for (i, f) in d.features.iter().enumerate() {
        if f.parts.iter().all(|p| p.is_empty()) {
            continue;
        }
        if !first {
            writeln!(out, ",")?;
        }
        first = false;
        let props: Vec<String> = d
            .fields
            .iter()
            .enumerate()
            .map(|(k, name)| {
                let t = d
                    .text
                    .get(i)
                    .and_then(|r| r.get(k))
                    .cloned()
                    .unwrap_or_default();
                let n = d
                    .num
                    .get(i)
                    .and_then(|r| r.get(k))
                    .copied()
                    .unwrap_or(f64::NAN);
                let value = if n.is_finite() && !t.is_empty() {
                    if n.fract() == 0.0 && n.abs() < 1e15 {
                        format!("{}", n as i64)
                    } else {
                        format!("{n}")
                    }
                } else {
                    quote(&t)
                };
                format!("{}:{value}", quote(name))
            })
            .collect();
        write!(
            out,
            "{{\"type\":\"Feature\",\"properties\":{{{}}},\"geometry\":{}}}",
            props.join(","),
            geometry_text(&f.parts, kind)
        )?;
    }
    writeln!(out, "\n]}}")?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rl_gj_{}_{name}", std::process::id()))
    }

    #[test]
    fn a_polygon_with_a_hole_round_trips() -> Result<()> {
        // clockwise exterior and counter-clockwise hole, as in a shapefile
        let outer = vec![
            [0.0, 0.0],
            [0.0, 10.0],
            [10.0, 10.0],
            [10.0, 0.0],
            [0.0, 0.0],
        ];
        let hole = vec![[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0], [2.0, 2.0]];
        let d = VectorData {
            features: vec![Feature {
                parts: vec![outer.clone(), hole.clone()],
            }],
            polygons: true,
            fields: vec!["nombre".into(), "area".into()],
            text: vec![vec!["a \"b\"".into(), "96".into()]],
            num: vec![vec![f64::NAN, 96.0]],
            ..VectorData::default()
        };
        let path = temp("poly.geojson");
        write_vector(&path, &d, GeomKind::Polygon, Some(25830))?;
        let text = std::fs::read_to_string(&path)?;
        assert!(
            text.contains("EPSG::25830") && text.contains("\"Polygon\""),
            "{text}"
        );
        assert_eq!(epsg_hint(&path), Some(25830));
        let (back, kind, epsg, skipped) = read_vector(&path)?;
        let _ = std::fs::remove_file(&path);
        assert_eq!((kind, epsg, skipped), (GeomKind::Polygon, Some(25830), 0));
        assert_eq!(back.features.len(), 1);
        let parts = &back.features[0].parts;
        assert_eq!(parts.len(), 2);
        // exterior clockwise (negative area), hole counter-clockwise, as the program expects
        assert!(signed_area(&parts[0]) < 0.0 && signed_area(&parts[1]) > 0.0);
        assert!((signed_area(&parts[0]) + 100.0).abs() < 1e-9);
        assert_eq!(back.text[0], vec!["a \"b\"".to_string(), "96".to_string()]);
        assert_eq!(back.num[0][1], 96.0);
        Ok(())
    }

    #[test]
    fn reads_points_lines_and_multi_geometries() -> Result<()> {
        let path = temp("mix.geojson");
        std::fs::write(
            &path,
            r#"{"type":"FeatureCollection","features":[
              {"type":"Feature","properties":{"id":1},"geometry":{"type":"MultiPoint","coordinates":[[1,2],[3,4]]}},
              {"type":"Feature","properties":{"id":2},"geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]}},
              {"type":"Feature","properties":null,"geometry":null}]}"#,
        )?;
        let (v, kind, epsg, skipped) = read_vector(&path)?;
        let _ = std::fs::remove_file(&path);
        // two points from the multi-point; the line and the empty feature are left out
        assert_eq!(
            (kind, epsg, skipped, v.features.len()),
            (GeomKind::Point, None, 2, 2)
        );
        assert_eq!(v.text[1][0], "1");
        Ok(())
    }

    #[test]
    fn polygons_become_regions_and_other_files_are_refused() -> Result<()> {
        let path = temp("regions.geojson");
        std::fs::write(
            &path,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"parcela":"P1"},
              "geometry":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]]]}}]}"#,
        )?;
        let regions = read_regions(
            &path,
            &ReadOptions {
                name_field: Some("parcela".into()),
                ..Default::default()
            },
        )?;
        assert_eq!(regions[0].name, "P1");
        assert!(regions[0].contains(5.0, 5.0) && !regions[0].contains(15.0, 5.0));
        std::fs::write(&path, r#"{"type":"Polygon","coordinates":[]}"#)?;
        assert!(read_vector(&path).is_err());
        let _ = std::fs::remove_file(&path);
        Ok(())
    }
}
