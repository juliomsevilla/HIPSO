//! Points of interest and areas of interest drawn over a cloud, saved as shapefiles with their
//! name, colour and comment (the viewer's POI and AOI tools).

use std::path::Path;

use shapefile::dbase::{FieldName, FieldValue, Record, TableWriterBuilder};

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct Poi {
    pub name: String,
    pub color: [u8; 3],
    pub comment: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Aoi {
    pub name: String,
    pub color: [u8; 3],
    pub comment: String,
    pub points: Vec<[f64; 2]>,
}

fn fmt_err(e: impl std::fmt::Display) -> Error {
    Error::Format(e.to_string())
}

fn table() -> Result<TableWriterBuilder> {
    let n = |s: &str| FieldName::try_from(s).map_err(|e| Error::Param(format!("{e:?}")));
    Ok(TableWriterBuilder::new()
        .add_character_field(n("NAME")?, 50)
        .add_character_field(n("COLOR")?, 7)
        .add_character_field(n("COMMENT")?, 200))
}

fn record(name: &str, color: [u8; 3], comment: &str) -> Record {
    let mut r = Record::default();
    r.insert(
        "NAME".into(),
        FieldValue::Character(Some(name.chars().take(50).collect())),
    );
    r.insert(
        "COLOR".into(),
        FieldValue::Character(Some(format!(
            "#{:02X}{:02X}{:02X}",
            color[0], color[1], color[2]
        ))),
    );
    r.insert(
        "COMMENT".into(),
        FieldValue::Character(Some(comment.chars().take(200).collect())),
    );
    r
}

fn text(r: &Record, field: &str) -> String {
    match r.get(field) {
        Some(FieldValue::Character(Some(s))) => s.trim().to_string(),
        _ => String::new(),
    }
}

fn parse_color(s: &str) -> [u8; 3] {
    let h = s.trim_start_matches('#');
    let v = |i: usize| {
        h.get(i..i + 2)
            .and_then(|t| u8::from_str_radix(t, 16).ok())
            .unwrap_or(255)
    };
    [v(0), v(2), v(4)]
}

/// Writes the `.prj` beside a shapefile.
fn write_prj(path: &Path, prj: Option<&str>) -> Result<()> {
    if let Some(text) = prj {
        std::fs::write(path.with_extension("prj"), text)?;
    }
    Ok(())
}

/// Saves the points of interest; `prj` is the coordinate system of the cloud they were placed on.
pub fn save_poi(path: &Path, list: &[Poi], prj: Option<&str>) -> Result<()> {
    let mut w = shapefile::Writer::from_path(path, table()?).map_err(fmt_err)?;
    for p in list {
        w.write_shape_and_record(
            &shapefile::PointZ::new(p.x, p.y, p.z, 0.0),
            &record(&p.name, p.color, &p.comment),
        )
        .map_err(fmt_err)?;
    }
    drop(w);
    write_prj(path, prj)
}

pub fn load_poi(path: &Path) -> Result<Vec<Poi>> {
    let mut rd = shapefile::Reader::from_path(path).map_err(fmt_err)?;
    let mut out = Vec::new();
    for item in rd.iter_shapes_and_records() {
        let (shape, rec) = item.map_err(fmt_err)?;
        let (x, y, z) = match shape {
            shapefile::Shape::PointZ(p) => (p.x, p.y, p.z),
            shapefile::Shape::Point(p) => (p.x, p.y, 0.0),
            shapefile::Shape::PointM(p) => (p.x, p.y, 0.0),
            _ => continue,
        };
        out.push(Poi {
            name: text(&rec, "NAME"),
            color: parse_color(&text(&rec, "COLOR")),
            comment: text(&rec, "COMMENT"),
            x,
            y,
            z,
        });
    }
    Ok(out)
}

/// Saves the areas of interest; `prj` is the coordinate system of the cloud they were drawn on.
pub fn save_aoi(path: &Path, list: &[Aoi], prj: Option<&str>) -> Result<()> {
    use shapefile::{Point, Polygon, PolygonRing};
    let mut w = shapefile::Writer::from_path(path, table()?).map_err(fmt_err)?;
    for a in list.iter().filter(|a| a.points.len() >= 3) {
        let mut pts: Vec<Point> = a.points.iter().map(|p| Point::new(p[0], p[1])).collect();
        // shapefile outer rings run clockwise and close on their first vertex
        let area: f64 = a
            .points
            .iter()
            .zip(a.points.iter().cycle().skip(1))
            .map(|(p, q)| p[0] * q[1] - q[0] * p[1])
            .sum();
        if area > 0.0 {
            pts.reverse();
        }
        if pts.first() != pts.last() {
            pts.push(pts[0]);
        }
        w.write_shape_and_record(
            &Polygon::new(PolygonRing::Outer(pts)),
            &record(&a.name, a.color, &a.comment),
        )
        .map_err(fmt_err)?;
    }
    drop(w);
    write_prj(path, prj)
}

pub fn load_aoi(path: &Path) -> Result<Vec<Aoi>> {
    let mut rd = shapefile::Reader::from_path(path).map_err(fmt_err)?;
    let mut out = Vec::new();
    for item in rd.iter_shapes_and_records() {
        let (shape, rec) = item.map_err(fmt_err)?;
        let shapefile::Shape::Polygon(poly) = shape else {
            continue;
        };
        for ring in poly.rings() {
            let mut points: Vec<[f64; 2]> = ring.points().iter().map(|p| [p.x, p.y]).collect();
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            out.push(Aoi {
                name: text(&rec, "NAME"),
                color: parse_color(&text(&rec, "COLOR")),
                comment: text(&rec, "COMMENT"),
                points,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poi_and_aoi_round_trip() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl-markup-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let poi = vec![Poi {
            name: "a".into(),
            color: [10, 20, 30],
            comment: "hola".into(),
            x: 1.5,
            y: 2.5,
            z: 3.5,
        }];
        save_poi(&dir.join("p.shp"), &poi, Some("PROJCS[\"test\"]"))?;
        assert!(dir.join("p.prj").is_file());
        assert_eq!(load_poi(&dir.join("p.shp"))?, poi);
        let aoi = vec![Aoi {
            name: "z".into(),
            color: [255, 0, 0],
            comment: String::new(),
            points: vec![[0.0, 0.0], [0.0, 5.0], [5.0, 5.0], [5.0, 0.0]],
        }];
        save_aoi(&dir.join("a.shp"), &aoi, Some("PROJCS[\"test\"]"))?;
        assert!(dir.join("a.prj").is_file());
        let back = load_aoi(&dir.join("a.shp"))?;
        assert_eq!(
            (back.len(), back[0].points.len(), &back[0].name),
            (1, 4, &"z".to_string())
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Writes sample files to the folder named by `RUSTLIDAR_MARKUP_OUT`, for opening them in
    /// other software (`cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn write_samples() -> Result<()> {
        let Some(dir) = std::env::var_os("RUSTLIDAR_MARKUP_OUT") else {
            return Ok(());
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir)?;
        let prj = crate::vector::prj_for_epsg(25830);
        let poi = [Poi {
            name: "p1".into(),
            color: [255, 200, 0],
            comment: "c".into(),
            x: 580200.0,
            y: 4802000.0,
            z: 550.0,
        }];
        save_poi(&dir.join("poi.shp"), &poi, prj.as_deref())?;
        let aoi = [Aoi {
            name: "a1".into(),
            color: [60, 200, 255],
            comment: String::new(),
            points: vec![
                [580100.0, 4801900.0],
                [580300.0, 4801900.0],
                [580300.0, 4802100.0],
            ],
        }];
        save_aoi(&dir.join("aoi.shp"), &aoi, prj.as_deref())?;
        assert!(dir.join("poi.prj").is_file() && dir.join("aoi.prj").is_file());
        Ok(())
    }
}
