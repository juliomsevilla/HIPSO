//! GeoPackage vector layers (OGC GeoPackage 1.x): the geometry of each feature in GeoPackage
//! binary (a small header and standard WKB) and its attributes. Read only.

use std::path::Path;

use crate::geojson::{Feat, Layer, orient_rings};
use crate::sqlite::{Db, Value};
use crate::view::GeomKind;
use crate::{Error, Result};

/// A feature table of a GeoPackage.
#[derive(Clone, Debug)]
pub struct LayerInfo {
    pub table: String,
    pub geometry_column: String,
    pub srs_id: i64,
    pub geometry_type: String,
}

/// The feature tables of a GeoPackage, in the order the file lists them.
pub fn layers(path: &Path) -> Result<Vec<LayerInfo>> {
    layers_in(&Db::open(path)?)
}

fn layers_in(db: &Db) -> Result<Vec<LayerInfo>> {
    let t = db.table("gpkg_geometry_columns")?;
    let (Some(tc), Some(gc), Some(sc), Some(ty)) = (
        t.column("table_name"),
        t.column("column_name"),
        t.column("srs_id"),
        t.column("geometry_type_name"),
    ) else {
        return Err(Error::Format("GeoPackage sin tabla de geometrías".into()));
    };
    Ok(t.rows
        .iter()
        .map(|r| LayerInfo {
            table: r[tc].display(),
            geometry_column: r[gc].display(),
            srs_id: r[sc].as_i64().unwrap_or(0),
            geometry_type: r[ty].display().to_ascii_uppercase(),
        })
        .collect())
}

/// The EPSG code of a GeoPackage spatial reference system id, when it is one.
fn epsg_of_srs(db: &Db, srs_id: i64) -> Option<u16> {
    match srs_id {
        4326 => return Some(4326),
        i64::MIN..=0 => return None,
        _ => {}
    }
    let t = db.table("gpkg_spatial_ref_sys").ok()?;
    let (id, org, code) = (
        t.column("srs_id")?,
        t.column("organization")?,
        t.column("organization_coordsys_id")?,
    );
    t.rows
        .iter()
        .find(|r| r[id].as_i64() == Some(srs_id))
        .filter(|r| r[org].display().eq_ignore_ascii_case("EPSG"))
        .and_then(|r| r[code].as_i64())
        .and_then(|c| u16::try_from(c).ok())
}

/// The EPSG code the first feature table of a GeoPackage declares.
pub fn epsg_hint(path: &Path) -> Option<u16> {
    let db = Db::open(path).ok()?;
    let first = layers_in(&db).ok()?.into_iter().next()?;
    epsg_of_srs(&db, first.srs_id)
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
    le: bool,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at + n)?;
        self.at += n;
        Some(s)
    }

    fn u32(&mut self) -> Option<u32> {
        let s: [u8; 4] = self.take(4)?.try_into().ok()?;
        Some(if self.le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    }

    fn f64(&mut self) -> Option<f64> {
        let s: [u8; 8] = self.take(8)?.try_into().ok()?;
        Some(if self.le {
            f64::from_le_bytes(s)
        } else {
            f64::from_be_bytes(s)
        })
    }
}

/// What a geometry adds to a layer: its kind and its parts (a ring or a line per part, a single
/// vertex per point). `dims` counts the coordinates a vertex has beyond x and y.
fn wkb(c: &mut Cursor, out: &mut Vec<(GeomKind, Vec<Vec<[f64; 2]>>)>, depth: usize) -> Option<()> {
    if depth > 8 {
        return None;
    }
    c.le = *c.take(1)?.first()? == 1;
    let raw = c.u32()?;
    // ISO codes add 1000 for Z, 2000 for M, 3000 for both; EWKB sets flag bits instead
    let (flags, iso) = (raw & 0xE000_0000, raw & 0x1FFF_FFFF);
    let dims = usize::from(flags & 0x8000_0000 != 0)
        + usize::from(flags & 0x4000_0000 != 0)
        + match iso / 1000 {
            1 | 2 => 1,
            3 => 2,
            _ => 0,
        };
    let kind = iso % 1000;
    let vertex = |c: &mut Cursor| -> Option<[f64; 2]> {
        let (x, y) = (c.f64()?, c.f64()?);
        for _ in 0..dims {
            c.f64()?;
        }
        Some([x, y])
    };
    let line = |c: &mut Cursor| -> Option<Vec<[f64; 2]>> {
        let n = c.u32()? as usize;
        if n > 50_000_000 {
            return None;
        }
        (0..n).map(|_| vertex(c)).collect()
    };
    let polygon = |c: &mut Cursor| -> Option<Vec<Vec<[f64; 2]>>> {
        let n = c.u32()? as usize;
        let rings: Vec<Vec<[f64; 2]>> = (0..n).map(|_| line(c)).collect::<Option<_>>()?;
        Some(orient_rings(rings))
    };
    match kind {
        1 => {
            let v = vertex(c)?;
            if v[0].is_finite() && v[1].is_finite() {
                out.push((GeomKind::Point, vec![vec![v]]));
            }
        }
        2 => out.push((GeomKind::Line, vec![line(c)?])),
        3 => out.push((GeomKind::Polygon, polygon(c)?)),
        4..=7 => {
            let n = c.u32()?;
            for _ in 0..n {
                wkb(c, out, depth + 1)?;
            }
        }
        _ => return None,
    }
    Some(())
}

/// The parts of a GeoPackage geometry blob; `None` when it is empty or of an unsupported kind.
fn geometry(blob: &[u8]) -> Option<(GeomKind, Vec<Vec<[f64; 2]>>)> {
    if blob.len() < 8 || &blob[..2] != b"GP" {
        return None;
    }
    let flags = blob[3];
    if flags & 0x10 != 0 {
        return None; // empty
    }
    let envelope = match (flags >> 1) & 7 {
        0 => 0,
        1 => 32,
        2 | 3 => 48,
        4 => 64,
        _ => return None,
    };
    let mut c = Cursor {
        b: blob,
        at: 8 + envelope,
        le: true,
    };
    let mut parts = Vec::new();
    wkb(&mut c, &mut parts, 0)?;
    let kind = parts.first()?.0;
    let all: Vec<Vec<[f64; 2]>> = parts
        .into_iter()
        .filter(|(k, _)| *k == kind)
        .flat_map(|(_, p)| p)
        .collect();
    Some((kind, all))
}

/// A feature table as a layer; the first one when `table` is `None`.
pub(crate) fn read_layer(path: &Path, table: Option<&str>) -> Result<Layer> {
    let db = Db::open(path)?;
    let all = layers_in(&db)?;
    let info = match table {
        Some(t) => all.iter().find(|l| l.table.eq_ignore_ascii_case(t)),
        None => all.first(),
    }
    .ok_or_else(|| {
        Error::Format(format!(
            "{}: el GeoPackage no tiene capas de objetos",
            path.display()
        ))
    })?;
    let t = db.table(&info.table)?;
    let geom = t
        .column(&info.geometry_column)
        .ok_or_else(|| Error::Format(format!("«{}» no tiene columna de geometría", info.table)))?;
    let fields: Vec<usize> = (0..t.columns.len()).filter(|k| *k != geom).collect();
    let mut layer = Layer {
        kind: GeomKind::Point,
        feats: Vec::new(),
        epsg: epsg_of_srs(&db, info.srs_id),
        skipped: 0,
    };
    let mut first = true;
    for row in &t.rows {
        let Value::Blob(blob) = &row[geom] else {
            layer.skipped += 1;
            continue;
        };
        let Some((kind, parts)) = geometry(blob) else {
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
        let props: Vec<(String, String)> = fields
            .iter()
            .map(|k| (t.columns[*k].clone(), row[*k].display()))
            .collect();
        if kind == GeomKind::Point {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_blob_is_read_with_its_envelope() {
        // GP, version 0, flags: little endian + envelope XY, srs 25830, envelope, WKB polygon
        let mut b = vec![b'G', b'P', 0, 0b0000_0011];
        b.extend_from_slice(&25830i32.to_le_bytes());
        for v in [0.0f64, 10.0, 0.0, 10.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.push(1);
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&5u32.to_le_bytes());
        for (x, y) in [
            (0.0f64, 0.0f64),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ] {
            b.extend_from_slice(&x.to_le_bytes());
            b.extend_from_slice(&y.to_le_bytes());
        }
        let (kind, parts) = geometry(&b).unwrap_or((GeomKind::Point, Vec::new()));
        assert_eq!(kind, GeomKind::Polygon);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].len(), 5);
        // the exterior ring is clockwise
        let a: f64 = parts[0]
            .windows(2)
            .map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1])
            .sum();
        assert!(a < 0.0);
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/parcelas.gpkg")
    }

    #[test]
    fn a_real_geopackage_is_read_whole() -> Result<()> {
        let l = layers(&fixture())?;
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].table, "parcelas");
        assert_eq!(epsg_hint(&fixture()), Some(25830));
        let layer = read_layer(&fixture(), None)?;
        // 800 parcels and the big one, none lost to the b-tree or the overflow pages
        assert_eq!(layer.feats.len(), 801);
        assert_eq!(layer.skipped, 0);
        assert_eq!(layer.kind, GeomKind::Polygon);
        let big = layer.feats.last().map(|f| f.parts[0].len());
        assert_eq!(big, Some(3001));
        // the parcel with a hole has two rings; attributes and accents survive
        assert_eq!(layer.feats[7].parts.len(), 2);
        let props = &layer.feats[3].props;
        assert!(props.iter().any(|(k, v)| k == "nombre" && v == "P0004"));
        assert!(props.iter().any(|(k, v)| k == "nota" && v == "café"));
        assert!(
            props
                .iter()
                .any(|(k, v)| k == "area" && v.starts_with("8100"))
        );
        Ok(())
    }

    #[test]
    fn regions_from_a_geopackage_clip_like_a_shapefile() -> Result<()> {
        let regions = crate::vector::read_regions(
            &fixture(),
            &crate::vector::ReadOptions {
                name_field: Some("nombre".into()),
                ..Default::default()
            },
        )?;
        assert_eq!(regions.len(), 801);
        assert_eq!(regions[0].name, "P0001");
        match &regions[0].shape {
            crate::vector::Shape2::Polygon(p) => {
                assert!(p.contains(500045.0, 4500045.0));
                assert!(!p.contains(500095.0, 4500045.0));
            }
            _ => return Err(Error::Param("a polygon".into())),
        }
        // the hole of parcel 8 holds no point, its ring does
        if let crate::vector::Shape2::Polygon(p) = &regions[7].shape {
            assert!(!p.contains(500745.0, 4500045.0));
            assert!(p.contains(500710.0, 4500010.0));
        }
        Ok(())
    }
}
