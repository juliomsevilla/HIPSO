//! Operations on vector layers of any kind (points, lines, polygons) that work on the data the
//! viewer holds: selection by location, spatial join of attributes, centroids and points on
//! the surface, and the split and gathering of the parts of features.
//!
//! Geometry comes from the `geo` crate (relations after the DE-9IM model of the OGC).

use std::collections::HashSet;
use std::path::Path;

use geo::{Centroid, Coord, Geometry, InteriorPoint, LineString, MultiLineString, MultiPoint, Point, Relate};

use crate::geomops::multipolygon_of;
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result};

/// The kind of geometry of a layer read for display.
pub fn kind_of(d: &VectorData) -> GeomKind {
    if d.points {
        GeomKind::Point
    } else if d.polygons {
        GeomKind::Polygon
    } else {
        GeomKind::Line
    }
}

/// A layer from a file as the viewer holds it, with its kind and the `.prj` text to write back
/// (the file's own, or the one of its EPSG code).
pub fn read_layer(path: &Path) -> Result<(VectorData, GeomKind, Option<String>)> {
    let d = crate::view::read_vector(path)?;
    let kind = kind_of(&d);
    let prj = std::fs::read_to_string(path.with_extension("prj"))
        .ok()
        .or_else(|| crate::crs::epsg_of(path).and_then(crate::vector::prj_for_epsg));
    Ok((d, kind, prj))
}

fn bounds_of(d: &mut VectorData) {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in d.features.iter().flat_map(|f| f.parts.iter().flatten()) {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    d.bounds = b;
}

fn number(t: &str) -> f64 {
    t.trim().parse().unwrap_or(f64::NAN)
}

/// The geometry of a feature for the relations; `None` when it has none.
pub fn geometry_of(parts: &[Vec<[f64; 2]>], kind: GeomKind) -> Option<Geometry<f64>> {
    let line = |p: &Vec<[f64; 2]>| LineString::new(p.iter().map(|c| Coord { x: c[0], y: c[1] }).collect());
    match kind {
        GeomKind::Point => {
            let pts: Vec<Point<f64>> = parts.iter().flatten().map(|p| Point::new(p[0], p[1])).collect();
            (!pts.is_empty()).then(|| Geometry::MultiPoint(MultiPoint::new(pts)))
        }
        GeomKind::Line => {
            let ls: Vec<LineString<f64>> = parts.iter().filter(|p| p.len() >= 2).map(line).collect();
            (!ls.is_empty()).then(|| Geometry::MultiLineString(MultiLineString::new(ls)))
        }
        GeomKind::Polygon => {
            let m = multipolygon_of(parts);
            (!m.0.is_empty()).then_some(Geometry::MultiPolygon(m))
        }
    }
}

fn bbox(parts: &[Vec<[f64; 2]>]) -> [f64; 4] {
    let mut b = [f64::NAN; 4];
    let mut first = true;
    for p in parts.iter().flatten() {
        if first {
            b = [p[0], p[1], p[0], p[1]];
            first = false;
        } else {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
    }
    b
}

/// How a feature of one layer must stand to a feature of another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Intersects,
    /// The feature lies inside the other one.
    Within,
    /// The feature contains the other one.
    Contains,
    Touches,
    Crosses,
    /// The feature meets none of the others.
    Disjoint,
}

impl Relation {
    pub const ALL: [Relation; 6] =
        [Relation::Intersects, Relation::Within, Relation::Contains, Relation::Touches, Relation::Crosses, Relation::Disjoint];

    pub fn name(self) -> &'static str {
        match self {
            Relation::Intersects => "Intersecta con",
            Relation::Within => "Está dentro de",
            Relation::Contains => "Contiene a",
            Relation::Touches => "Toca (solo el borde) a",
            Relation::Crosses => "Cruza a",
            Relation::Disjoint => "No toca a",
        }
    }

    fn holds(self, a: &Geometry<f64>, b: &Geometry<f64>) -> bool {
        let m = a.relate(b);
        match self {
            Relation::Intersects => m.is_intersects(),
            Relation::Within => m.is_within(),
            Relation::Contains => m.is_contains(),
            Relation::Touches => m.is_touches(),
            Relation::Crosses => m.is_crosses(),
            Relation::Disjoint => m.is_disjoint(),
        }
    }
}

/// The features of the other layer that meet `a`'s extent, per feature of `a`, with the
/// geometries of both worked out once.
struct Pairs {
    ga: Vec<Option<Geometry<f64>>>,
    gb: Vec<Option<Geometry<f64>>>,
    near: Vec<Vec<usize>>,
}

fn pairs(a: &VectorData, ka: GeomKind, b: &VectorData, kb: GeomKind) -> Pairs {
    let boxes: Vec<[f64; 4]> = b.features.iter().map(|f| bbox(&f.parts)).collect();
    let tree = crate::rtree::RTree::new(&boxes);
    let ga = a.features.iter().map(|f| geometry_of(&f.parts, ka)).collect();
    let gb = b.features.iter().map(|f| geometry_of(&f.parts, kb)).collect();
    let near = a
        .features
        .iter()
        .map(|f| {
            let bx = bbox(&f.parts);
            if bx.iter().any(|v| v.is_nan()) {
                return Vec::new();
            }
            let mut hits = tree.query(bx);
            hits.sort_unstable();
            hits
        })
        .collect();
    Pairs { ga, gb, near }
}

/// The features of `a` that stand in the relation to some feature of `b` (to none, for
/// [`Relation::Disjoint`]), in layer order.
pub fn select_by_location(a: &VectorData, ka: GeomKind, b: &VectorData, kb: GeomKind, rel: Relation) -> Vec<usize> {
    let p = pairs(a, ka, b, kb);
    (0..a.features.len())
        .filter(|&i| {
            let Some(g) = &p.ga[i] else { return false };
            let hit = |j: &usize| p.gb[*j].as_ref().is_some_and(|h| {
                if rel == Relation::Disjoint { Relation::Intersects.holds(g, h) } else { rel.holds(g, h) }
            });
            let any = p.near[i].iter().any(hit);
            if rel == Relation::Disjoint { !any } else { any }
        })
        .collect()
}

/// What a spatial join copies from the matching features.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinMode {
    /// The fields of the first matching feature.
    First,
    /// The sum and the mean of each numeric field of all the matching features.
    Summary,
}

fn numeric_column(d: &VectorData, c: usize) -> bool {
    let vals: Vec<&str> = d.text.iter().filter_map(|r| r.get(c)).map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    !vals.is_empty() && vals.iter().all(|t| t.parse::<f64>().is_ok_and(f64::is_finite))
}

pub(crate) fn nice(v: f64) -> String {
    if !v.is_finite() {
        String::new()
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.10}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The layer `a` with fields taken from the features of `b` that stand in the relation to each
/// of its features (`n_coinc` counts them). Names of new fields start with `prefix`.
pub fn spatial_join(
    a: &VectorData,
    ka: GeomKind,
    b: &VectorData,
    kb: GeomKind,
    rel: Relation,
    mode: JoinMode,
    prefix: &str,
) -> Result<VectorData> {
    if rel == Relation::Disjoint {
        return Err(Error::Param("«No toca a» no sirve para unir atributos: no hay con qué emparejar".into()));
    }
    let p = pairs(a, ka, b, kb);
    let matches: Vec<Vec<usize>> = (0..a.features.len())
        .map(|i| {
            let Some(g) = &p.ga[i] else { return Vec::new() };
            p.near[i].iter().copied().filter(|j| p.gb[*j].as_ref().is_some_and(|h| rel.holds(g, h))).collect()
        })
        .collect();
    let mut out = a.clone();
    let taken = |name: &str, out: &VectorData| out.fields.iter().any(|f| f.eq_ignore_ascii_case(name));
    let unique = |base: String, out: &VectorData| {
        let mut n = base.clone();
        let mut k = 2;
        while taken(&n, out) {
            n = format!("{base}_{k}");
            k += 1;
        }
        n
    };
    let add = |out: &mut VectorData, name: String, col: Vec<String>| {
        let name = unique(name, out);
        out.fields.push(name);
        for (i, v) in col.into_iter().enumerate() {
            out.num[i].push(number(&v));
            out.text[i].push(v);
        }
    };
    add(&mut out, format!("{prefix}n_coinc"), matches.iter().map(|m| m.len().to_string()).collect());
    for c in 0..b.fields.len() {
        let col_of = |f: &dyn Fn(&[usize]) -> String| -> Vec<String> { matches.iter().map(|m| f(m)).collect() };
        match mode {
            JoinMode::First => {
                let col = col_of(&|m: &[usize]| m.first().and_then(|j| b.text[*j].get(c)).cloned().unwrap_or_default());
                add(&mut out, format!("{prefix}{}", b.fields[c]), col);
            }
            JoinMode::Summary => {
                if !numeric_column(b, c) {
                    continue;
                }
                let vals = |m: &[usize]| -> Vec<f64> { m.iter().filter_map(|j| b.num[*j].get(c).copied()).filter(|v| v.is_finite()).collect() };
                let sum = col_of(&|m: &[usize]| {
                    let v = vals(m);
                    if v.is_empty() { String::new() } else { nice(v.iter().sum()) }
                });
                let mean = col_of(&|m: &[usize]| {
                    let v = vals(m);
                    if v.is_empty() { String::new() } else { nice(v.iter().sum::<f64>() / v.len() as f64) }
                });
                add(&mut out, format!("{prefix}{}_sum", b.fields[c]), sum);
                add(&mut out, format!("{prefix}{}_med", b.fields[c]), mean);
            }
        }
    }
    Ok(out)
}

/// One point per feature of the layer, with its attributes: the centroid, or a point that lies
/// on the feature itself (inside a polygon even when it is concave or has holes). Features with
/// no geometry are left out.
pub fn representative_points(d: &VectorData, kind: GeomKind, on_surface: bool) -> VectorData {
    let mut out = VectorData { points: true, fields: d.fields.clone(), ..VectorData::default() };
    for (i, f) in d.features.iter().enumerate() {
        let Some(g) = geometry_of(&f.parts, kind) else { continue };
        let pt = if on_surface { g.interior_point() } else { g.centroid() };
        let Some(pt) = pt else { continue };
        out.features.push(Feature { parts: vec![vec![[pt.x(), pt.y()]]] });
        out.text.push(d.text[i].clone());
        out.num.push(d.num[i].clone());
    }
    bounds_of(&mut out);
    out
}

/// The parts of a feature, one feature each: the polygons (with their holes) of a polygon
/// feature, the lines of a line feature. A point feature stays as it is.
pub fn split_parts(parts: &[Vec<[f64; 2]>], kind: GeomKind) -> Vec<Vec<Vec<[f64; 2]>>> {
    match kind {
        GeomKind::Point => vec![parts.to_vec()],
        GeomKind::Line => parts.iter().map(|p| vec![p.clone()]).collect(),
        GeomKind::Polygon => {
            let m = multipolygon_of(parts);
            if m.0.len() <= 1 {
                return vec![parts.to_vec()];
            }
            m.0.iter()
                .map(|p| crate::geomops::parts_of(&geo::MultiPolygon::new(vec![p.clone()])))
                .collect()
        }
    }
}

/// Splits every selected feature that has several parts into one feature per part; the pieces
/// keep the attributes. Returns the indices of the features that were split and how many
/// features there are now. Selected features come out selected.
pub fn explode(d: &mut VectorData, kind: GeomKind, which: &HashSet<usize>) -> (usize, HashSet<usize>) {
    let mut features = Vec::with_capacity(d.features.len());
    let (mut text, mut num) = (Vec::new(), Vec::new());
    let (mut split, mut selected) = (0, HashSet::new());
    for (i, f) in std::mem::take(&mut d.features).into_iter().enumerate() {
        let pieces = if which.contains(&i) { split_parts(&f.parts, kind) } else { vec![f.parts.clone()] };
        if pieces.len() > 1 {
            split += 1;
        }
        for parts in pieces {
            if which.contains(&i) {
                selected.insert(features.len());
            }
            features.push(Feature { parts });
            text.push(d.text[i].clone());
            num.push(d.num[i].clone());
        }
    }
    d.features = features;
    d.text = text;
    d.num = num;
    bounds_of(d);
    (split, selected)
}

/// Gathers the selected features into one that has all their parts, without joining the
/// geometry (a polygon feature keeps its polygons separate, even if they overlap). The new
/// feature takes the place and the attributes of the first selected one.
pub fn gather(d: &mut VectorData, which: &HashSet<usize>) -> Option<usize> {
    let mut idx: Vec<usize> = which.iter().copied().filter(|i| *i < d.features.len()).collect();
    idx.sort_unstable();
    let first = *idx.first()?;
    if idx.len() < 2 {
        return None;
    }
    let mut parts = std::mem::take(&mut d.features[first].parts);
    for &i in &idx[1..] {
        parts.extend(std::mem::take(&mut d.features[i].parts));
    }
    d.features[first].parts = parts;
    for &i in idx[1..].iter().rev() {
        d.features.remove(i);
        d.text.remove(i);
        d.num.remove(i);
    }
    bounds_of(d);
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(x: f64, y: f64, s: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y], [x, y]]
    }

    fn layer(kind: GeomKind, feats: Vec<Vec<Vec<[f64; 2]>>>, vals: &[&str]) -> VectorData {
        let mut d = VectorData {
            polygons: kind == GeomKind::Polygon,
            points: kind == GeomKind::Point,
            fields: vec!["v".into()],
            ..VectorData::default()
        };
        for (parts, v) in feats.into_iter().zip(vals) {
            d.features.push(Feature { parts });
            d.text.push(vec![(*v).to_string()]);
            d.num.push(vec![number(v)]);
        }
        bounds_of(&mut d);
        d
    }

    fn polys() -> VectorData {
        layer(GeomKind::Polygon, vec![vec![sq(0.0, 0.0, 10.0)], vec![sq(20.0, 0.0, 10.0)]], &["a", "b"])
    }

    fn pts() -> VectorData {
        layer(GeomKind::Point, vec![vec![vec![[5.0, 5.0]]], vec![vec![[6.0, 6.0]]], vec![vec![[50.0, 50.0]]]], &["2", "4", "9"])
    }

    #[test]
    fn points_are_selected_by_the_polygons_they_lie_in() {
        let (p, q) = (pts(), polys());
        let (g, k) = (GeomKind::Point, GeomKind::Polygon);
        assert_eq!(select_by_location(&p, g, &q, k, Relation::Within), [0, 1]);
        assert_eq!(select_by_location(&p, g, &q, k, Relation::Disjoint), [2]);
        assert_eq!(select_by_location(&q, k, &p, g, Relation::Contains), [0]);
    }

    #[test]
    fn a_join_copies_the_first_match_or_sums_them() -> Result<()> {
        let (p, q) = (pts(), polys());
        let (g, k) = (GeomKind::Point, GeomKind::Polygon);
        let first = spatial_join(&q, k, &p, g, Relation::Contains, JoinMode::First, "p_")?;
        assert_eq!(first.fields, ["v", "p_n_coinc", "p_v"]);
        assert_eq!(first.text[0], ["a", "2", "2"]);
        assert_eq!(first.text[1], ["b", "0", ""]);
        let sum = spatial_join(&q, k, &p, g, Relation::Contains, JoinMode::Summary, "")?;
        assert_eq!(sum.fields, ["v", "n_coinc", "v_sum", "v_med"]);
        assert_eq!(sum.text[0], ["a", "2", "6", "3"]);
        Ok(())
    }

    #[test]
    fn a_point_on_the_surface_lies_in_a_c_shaped_polygon_where_the_centroid_does_not() {
        let c = vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 9.0], [1.0, 9.0], [1.0, 1.0], [10.0, 1.0], [10.0, 0.0], [0.0, 0.0]];
        let d = layer(GeomKind::Polygon, vec![vec![c]], &["c"]);
        let (cen, pos) = (representative_points(&d, GeomKind::Polygon, false), representative_points(&d, GeomKind::Polygon, true));
        assert_eq!(select_by_location(&cen, GeomKind::Point, &d, GeomKind::Polygon, Relation::Within), Vec::<usize>::new());
        assert_eq!(select_by_location(&pos, GeomKind::Point, &d, GeomKind::Polygon, Relation::Within), [0]);
        assert_eq!(pos.text[0], ["c"]);
    }

    #[test]
    fn a_multipart_polygon_is_split_and_gathered_back() {
        let mut d = layer(GeomKind::Polygon, vec![vec![sq(0.0, 0.0, 5.0), sq(20.0, 0.0, 5.0)], vec![sq(50.0, 0.0, 5.0)]], &["x", "y"]);
        let all: HashSet<usize> = [0, 1].into_iter().collect();
        let (split, sel) = explode(&mut d, GeomKind::Polygon, &all);
        assert_eq!((split, d.features.len(), sel.len()), (1, 3, 3));
        assert_eq!(d.text[1], ["x"]);
        let two: HashSet<usize> = [0, 1].into_iter().collect();
        assert_eq!(gather(&mut d, &two), Some(0));
        assert_eq!((d.features.len(), d.features[0].parts.len()), (2, 2));
    }
}
