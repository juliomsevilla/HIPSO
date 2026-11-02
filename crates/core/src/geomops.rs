//! Operations on polygon layers: validity check and repair, buffer, dissolve and union.
//!
//! Geometry comes from the `geo` crate's algorithms (OGC validity rules; repair after Ledoux,
//! Arroyo Ohori and Meijers 2014; boolean operations; buffer). A layer keeps its attribute
//! table: operations that merge features say what the attributes of the result are.

use std::collections::BTreeMap;
use std::path::Path;

use geo::{Area, BooleanOps, BoundingRect, Buffer, Coord, LineString, MultiPolygon, Polygon as GPoly};
use geo::algorithm::repair_polygon::MakeValid;
use geo::algorithm::validation::Validation;

use crate::vector::{self, ColKind, Region, Shape2, Table, Val};
use crate::{Error, Result};

/// One feature: its geometry and its attribute values (field, text).
#[derive(Clone, Debug)]
pub struct Feature {
    pub geom: MultiPolygon<f64>,
    pub attrs: Vec<(String, String)>,
}

/// A polygon layer.
#[derive(Clone, Debug, Default)]
pub struct PolyLayer {
    pub features: Vec<Feature>,
    /// The `.prj` text of the file read, written back with the result.
    pub prj: Option<String>,
}

/// The polygons (with holes and parts) of a set of rings, whose orientation is not trusted: a
/// ring is a hole when an odd number of other rings hold it, and belongs to the innermost ring
/// that holds it.
pub fn multipolygon_of(rings: &[Vec<[f64; 2]>]) -> MultiPolygon<f64> {
    let rings: Vec<&Vec<[f64; 2]>> = rings.iter().filter(|r| r.len() >= 3).collect();
    let polys: Vec<vector::Polygon> = rings.iter().map(|r| vector::Polygon::new(vec![(*r).clone()])).collect();
    let holds = |outer: usize, inner: usize| {
        polys[outer].contains_exact(rings[inner][0][0], rings[inner][0][1])
    };
    let depth: Vec<usize> = (0..rings.len())
        .map(|i| (0..rings.len()).filter(|&j| j != i && holds(j, i)).count())
        .collect();
    let line = |r: &Vec<[f64; 2]>| LineString::new(r.iter().map(|p| Coord { x: p[0], y: p[1] }).collect());
    let mut out = Vec::new();
    for o in (0..rings.len()).filter(|&i| depth[i] % 2 == 0) {
        let holes: Vec<LineString<f64>> = (0..rings.len())
            .filter(|&h| depth[h] == depth[o] + 1 && holds(o, h))
            .map(|h| line(rings[h]))
            .collect();
        out.push(GPoly::new(line(rings[o]), holes));
    }
    MultiPolygon::new(out)
}

pub(crate) fn rings_of(region: &Region) -> Vec<Vec<[f64; 2]>> {
    match &region.shape {
        Shape2::Polygon(p) => p.rings.clone(),
        other => crate::plots::rings(other),
    }
}

/// Reads the polygons of a shapefile, GeoJSON or GeoPackage layer with their attributes.
pub fn read_polygons(path: &Path) -> Result<PolyLayer> {
    let regions = vector::read_regions(path, &vector::ReadOptions::default())?;
    let features = regions
        .iter()
        .map(|r| Feature {
            geom: multipolygon_of(&rings_of(r)),
            attrs: r.attrs.clone(),
        })
        .collect();
    let prj = std::fs::read_to_string(path.with_extension("prj")).ok();
    Ok(PolyLayer { features, prj })
}

/// The attribute table of a layer: a column is numeric when every value is a number, else text.
/// Names are cut to the 10 characters of the format and kept distinct.
pub fn table_of(features: &[Feature]) -> Table {
    let mut names: Vec<String> = Vec::new();
    for f in features {
        for (k, _) in &f.attrs {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    let mut used: Vec<String> = Vec::new();
    let mut columns = Vec::new();
    let mut kinds = Vec::new();
    for n in &names {
        let vals: Vec<&str> = features
            .iter()
            .filter_map(|f| f.attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str()))
            .collect();
        let numeric = !vals.is_empty()
            && vals.iter().all(|v| v.is_empty() || v.trim().parse::<f64>().is_ok_and(f64::is_finite));
        let int = numeric && vals.iter().all(|v| v.is_empty() || v.trim().parse::<i64>().is_ok());
        let mut short: String = n.chars().take(10).collect();
        let mut k = 2;
        while used.contains(&short) {
            let tail = format!("_{k}");
            short = format!("{}{tail}", n.chars().take(10 - tail.len()).collect::<String>());
            k += 1;
        }
        used.push(short.clone());
        columns.push((
            short,
            if int {
                ColKind::Num(18, 0)
            } else if numeric {
                ColKind::Num(24, 8)
            } else {
                ColKind::Text(254)
            },
        ));
        kinds.push((n.clone(), numeric, int));
    }
    let rows = features
        .iter()
        .map(|f| {
            kinds
                .iter()
                .map(|(n, numeric, int)| {
                    let v = f.attrs.iter().find(|(k, _)| k == n).map_or("", |(_, v)| v.as_str());
                    if *numeric {
                        match v.trim().parse::<f64>() {
                            Ok(x) if *int => Val::Int(x as i64),
                            Ok(x) => Val::Num(x),
                            Err(_) => Val::Text(String::new()),
                        }
                    } else {
                        Val::Text(v.to_string())
                    }
                })
                .collect()
        })
        .collect();
    Table { columns, rows }
}

/// Writes a layer as a shapefile (with its `.prj` when it has one).
pub fn write_polygons(layer: &PolyLayer, path: &Path) -> Result<()> {
    crate::paths::ensure_parent(path)?;
    let feats: Vec<Vec<(Vec<[f64; 2]>, bool)>> = layer
        .features
        .iter()
        .map(|f| {
            let mut v = Vec::new();
            for p in &f.geom.0 {
                let ring = |l: &LineString<f64>| l.0.iter().map(|c| [c.x, c.y]).collect::<Vec<_>>();
                v.push((ring(p.exterior()), false));
                for h in p.interiors() {
                    v.push((ring(h), true));
                }
            }
            v
        })
        .collect();
    vector::write_polygon_layer(path, &feats, &table_of(&layer.features), layer.prj.as_deref())
}

/// Features that are not valid, with what is wrong: `(feature index, reasons)`.
pub fn invalid(layer: &PolyLayer) -> Vec<(usize, String)> {
    layer
        .features
        .iter()
        .enumerate()
        .filter_map(|(i, f)| {
            let errs = f.geom.validation_errors();
            (!errs.is_empty()).then(|| {
                let mut reasons: Vec<String> = errs.iter().map(ToString::to_string).collect();
                reasons.dedup();
                (i, reasons.join("; "))
            })
        })
        .collect()
}

/// The layer with every invalid feature repaired (even-odd rule): self-crossings become parts,
/// doubled edges cancel. Valid features are not touched. The count of features changed is
/// returned.
pub fn make_valid(layer: &PolyLayer) -> Result<(PolyLayer, usize)> {
    let mut out = layer.clone();
    let mut changed = 0;
    for f in &mut out.features {
        if f.geom.is_valid() {
            continue;
        }
        f.geom = f
            .geom
            .make_valid()
            .map_err(|e| Error::Param(format!("no se puede reparar una geometría: {e:?}")))?;
        changed += 1;
    }
    Ok((out, changed))
}

fn require_valid(layer: &PolyLayer, what: &str) -> Result<()> {
    let bad = invalid(layer);
    if let Some((i, why)) = bad.first() {
        return Err(Error::Param(format!(
            "{} entidades no son válidas (la primera, la {}: {why}): repáralas antes de {what} (make-valid)",
            bad.len(),
            i + 1
        )));
    }
    Ok(())
}

/// Each feature grown by `dist` map units (shrunk if negative), with rounded corners. Features
/// that vanish when shrunk keep their row with no geometry.
pub fn buffer(layer: &PolyLayer, dist: f64) -> Result<PolyLayer> {
    require_valid(layer, "hacer el buffer")?;
    let mut out = layer.clone();
    for f in &mut out.features {
        f.geom = f.geom.buffer(dist);
    }
    Ok(out)
}

/// Merges features. With `field`, those sharing its value become one feature (that value kept,
/// plus `n` the number merged); without, the whole layer becomes one feature with `n`.
pub fn dissolve(layer: &PolyLayer, field: Option<&str>) -> Result<PolyLayer> {
    require_valid(layer, "disolver")?;
    let mut groups: BTreeMap<String, Vec<&Feature>> = BTreeMap::new();
    for f in &layer.features {
        let key = match field {
            Some(name) => f
                .attrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
                .ok_or_else(|| Error::Param(format!("no hay un campo «{name}»")))?,
            None => String::new(),
        };
        groups.entry(key).or_default().push(f);
    }
    let features = groups
        .into_iter()
        .map(|(key, members)| {
            let geom = geo::algorithm::bool_ops::unary_union(members.iter().map(|f| &f.geom));
            let mut attrs = Vec::new();
            if let Some(name) = field {
                attrs.push((name.to_string(), key));
            }
            attrs.push(("n".into(), members.len().to_string()));
            Feature { geom, attrs }
        })
        .collect();
    Ok(PolyLayer { features, prj: layer.prj.clone() })
}

/// The union of two layers as an overlay: pieces in both (with the attributes of both, those of
/// `b` after a `b_` prefix), in `a` only and in `b` only. Pieces with no area are dropped.
pub fn union(a: &PolyLayer, b: &PolyLayer) -> Result<PolyLayer> {
    require_valid(a, "la unión")?;
    require_valid(b, "la unión")?;
    let bbox = |g: &MultiPolygon<f64>| g.bounding_rect();
    let overlaps = |x: &MultiPolygon<f64>, y: &MultiPolygon<f64>| match (bbox(x), bbox(y)) {
        (Some(p), Some(q)) => p.min().x <= q.max().x && q.min().x <= p.max().x && p.min().y <= q.max().y && q.min().y <= p.max().y,
        _ => false,
    };
    let eps = 1e-9;
    let mut features = Vec::new();
    let prefixed = |f: &Feature| -> Vec<(String, String)> { f.attrs.iter().map(|(k, v)| (format!("b_{k}"), v.clone())).collect() };
    for fa in &a.features {
        let mut rest = fa.geom.clone();
        for fb in b.features.iter().filter(|fb| overlaps(&fa.geom, &fb.geom)) {
            let both = fa.geom.intersection(&fb.geom);
            if both.unsigned_area() > eps {
                let mut attrs = fa.attrs.clone();
                attrs.extend(prefixed(fb));
                features.push(Feature { geom: both, attrs });
                rest = rest.difference(&fb.geom);
            }
        }
        if rest.unsigned_area() > eps {
            features.push(Feature { geom: rest, attrs: fa.attrs.clone() });
        }
    }
    for fb in &b.features {
        let mut rest = fb.geom.clone();
        for fa in a.features.iter().filter(|fa| overlaps(&fa.geom, &fb.geom)) {
            rest = rest.difference(&fa.geom);
        }
        if rest.unsigned_area() > eps {
            features.push(Feature { geom: rest, attrs: prefixed(fb) });
        }
    }
    Ok(PolyLayer { features, prj: a.prj.clone().or_else(|| b.prj.clone()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(x: f64, y: f64, s: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y]]
    }

    fn feat(rings: Vec<Vec<[f64; 2]>>, attrs: &[(&str, &str)]) -> Feature {
        Feature {
            geom: multipolygon_of(&rings),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    fn layer(f: Vec<Feature>) -> PolyLayer {
        PolyLayer { features: f, prj: None }
    }

    #[test]
    fn rings_are_sorted_into_parts_and_holes() {
        let m = multipolygon_of(&[sq(0.0, 0.0, 10.0), sq(2.0, 2.0, 3.0), sq(20.0, 0.0, 4.0)]);
        assert_eq!(m.0.len(), 2);
        assert!((m.unsigned_area() - (100.0 - 9.0 + 16.0)).abs() < 1e-9);
    }

    #[test]
    fn a_bowtie_is_invalid_and_repair_makes_two_triangles() {
        let bow = vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]];
        let l = layer(vec![feat(vec![bow], &[("id", "1")]), feat(vec![sq(20.0, 0.0, 5.0)], &[("id", "2")])]);
        let bad = invalid(&l);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, 0);
        let (fixed, n) = make_valid(&l).unwrap();
        assert_eq!(n, 1);
        assert!(invalid(&fixed).is_empty());
        assert_eq!(fixed.features[0].geom.0.len(), 2);
        assert!((fixed.features[0].geom.unsigned_area() - 50.0).abs() < 1e-6);
        // the other is untouched and the table follows
        assert_eq!(fixed.features[1].attrs[0].1, "2");
        // the operations refuse invalid input and say what to do
        let e = buffer(&l, 1.0).unwrap_err().to_string();
        assert!(e.contains("make-valid"), "{e}");
    }

    #[test]
    fn buffer_grows_by_the_distance() {
        let l = layer(vec![feat(vec![sq(0.0, 0.0, 100.0)], &[])]);
        let g = &buffer(&l, 10.0).unwrap().features[0].geom;
        // a square grown by 10 with round corners: 100² + 4·100·10 + π·10²
        let expect = 10_000.0 + 4_000.0 + std::f64::consts::PI * 100.0;
        assert!((g.unsigned_area() - expect).abs() / expect < 0.01, "{}", g.unsigned_area());
        let s = &buffer(&l, -10.0).unwrap().features[0].geom;
        assert!((s.unsigned_area() - 6_400.0).abs() < 1.0);
        let gone = &buffer(&l, -60.0).unwrap().features[0].geom;
        assert!(gone.unsigned_area() == 0.0);
    }

    #[test]
    fn dissolve_merges_by_field_and_counts() {
        let l = layer(vec![
            feat(vec![sq(0.0, 0.0, 10.0)], &[("sp", "pino")]),
            feat(vec![sq(10.0, 0.0, 10.0)], &[("sp", "pino")]),
            feat(vec![sq(50.0, 0.0, 10.0)], &[("sp", "encina")]),
        ]);
        let d = dissolve(&l, Some("sp")).unwrap();
        assert_eq!(d.features.len(), 2);
        let pino = d.features.iter().find(|f| f.attrs[0].1 == "pino").unwrap();
        assert_eq!(pino.geom.0.len(), 1);
        assert!((pino.geom.unsigned_area() - 200.0).abs() < 1e-9);
        assert_eq!(pino.attrs[1], ("n".to_string(), "2".to_string()));
        let all = dissolve(&l, None).unwrap();
        assert_eq!(all.features.len(), 1);
        assert_eq!(all.features[0].geom.0.len(), 2);
        assert!(dissolve(&l, Some("nada")).is_err());
    }

    #[test]
    fn union_splits_overlaps_and_keeps_both_tables() {
        let a = layer(vec![feat(vec![sq(0.0, 0.0, 10.0)], &[("a", "1")])]);
        let b = layer(vec![feat(vec![sq(5.0, 0.0, 10.0)], &[("k", "x")])]);
        let u = union(&a, &b).unwrap();
        assert_eq!(u.features.len(), 3);
        let total: f64 = u.features.iter().map(|f| f.geom.unsigned_area()).sum();
        assert!((total - 150.0).abs() < 1e-6);
        let both = u.features.iter().find(|f| f.attrs.len() == 2).unwrap();
        assert!((both.geom.unsigned_area() - 50.0).abs() < 1e-6);
        assert_eq!(both.attrs[1].0, "b_k");
    }

    #[test]
    fn a_layer_round_trips_through_a_shapefile_with_holes_and_types() {
        let dir = std::env::temp_dir().join(format!("hp_geomops_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let l = layer(vec![
            feat(vec![sq(0.0, 0.0, 10.0), sq(2.0, 2.0, 3.0)], &[("nombre", "A"), ("ha", "1.5"), ("id", "7")]),
            feat(vec![sq(30.0, 0.0, 10.0)], &[("nombre", "B"), ("ha", "2"), ("id", "8")]),
        ]);
        let path = dir.join("t.shp");
        write_polygons(&l, &path).unwrap();
        let back = read_polygons(&path).unwrap();
        assert_eq!(back.features.len(), 2);
        assert!((back.features[0].geom.unsigned_area() - 91.0).abs() < 1e-9);
        assert_eq!(back.features[1].attrs.iter().find(|(k, _)| k == "nombre").unwrap().1, "B");
        let t = table_of(&l.features);
        assert!(matches!(t.columns[2].1, ColKind::Num(18, 0)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
