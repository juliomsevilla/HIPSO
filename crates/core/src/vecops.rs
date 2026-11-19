//! Operaciones sobre capas vectoriales de cualquier tipo (puntos, líneas, polígonos) que trabajan
//! sobre los datos que tiene el visor: selección por localización, unión espacial de atributos,
//! centroides y puntos en la superficie, y la división y reunión de las partes de los elementos.
//!
//! La geometría viene del crate `geo` (relaciones según el modelo DE-9IM de OGC).

use std::collections::HashSet;
use std::path::Path;

use geo::{Centroid, Coord, Geometry, InteriorPoint, LineString, MultiLineString, MultiPoint, Point, Relate};

use crate::geomops::multipolygon_of;
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result};

/// El tipo de geometría de una capa leída para mostrar.
pub fn kind_of(d: &VectorData) -> GeomKind {
    if d.points {
        GeomKind::Point
    } else if d.polygons {
        GeomKind::Polygon
    } else {
        GeomKind::Line
    }
}

/// Un campo de la tabla de una capa o de una tabla de texto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldInfo {
    pub name: String,
    /// Todos sus valores son números (el campo sirve para estadísticas, gráficos o modelos).
    pub numeric: bool,
}

/// Los campos de una capa vectorial (shapefile, GeoJSON, GeoPackage, KML) o las columnas de una tabla
/// (CSV, TSV, texto o XLSX), en su orden, con lo que hace falta para ofrecerlos en una lista
/// desplegable en lugar de pedir que se escriban. De un shapefile solo se lee la cabecera de su
/// tabla; de los demás, el archivo.
pub fn field_list(path: &Path) -> Result<Vec<FieldInfo>> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "shp" | "dbf" => {
            use shapefile::dbase::FieldType as Dbf;
            let r = shapefile::dbase::Reader::from_path(path.with_extension("dbf"))
                .map_err(|e| Error::Format(format!("{}: {e}", path.display())))?;
            Ok(r.fields()
                .iter()
                .map(|f| FieldInfo {
                    name: f.name().to_string(),
                    numeric: matches!(f.field_type(), Dbf::Numeric | Dbf::Float | Dbf::Double | Dbf::Currency | Dbf::Integer),
                })
                .collect())
        }
        "csv" | "tsv" | "txt" | "xlsx" => {
            let t = crate::aba::Table::read(path)?;
            Ok(t.names
                .iter()
                .enumerate()
                .map(|(c, name)| {
                    let mut any = false;
                    let numeric = (0..t.len()).all(|r| {
                        let v = t.text(r, c).trim();
                        if v.is_empty() {
                            return true;
                        }
                        any = true;
                        v.replace(',', ".").parse::<f64>().is_ok_and(f64::is_finite)
                    }) && any;
                    FieldInfo { name: name.clone(), numeric }
                })
                .collect())
        }
        _ => {
            let d = crate::view::read_vector(path)?;
            Ok((0..d.fields.len())
                .map(|c| FieldInfo { name: d.fields[c].clone(), numeric: d.field_type(c).is_numeric() })
                .collect())
        }
    }
}

/// Una capa de un archivo tal como la tiene el visor, con su tipo y el texto `.prj` que escribir de
/// vuelta (el del propio archivo o el de su código EPSG).
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

/// La geometría de un elemento para las relaciones; `None` cuando no la tiene.
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

/// Cómo debe estar un elemento de una capa respecto a un elemento de otra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Intersects,
    /// El elemento está dentro del otro.
    Within,
    /// El elemento contiene al otro.
    Contains,
    Touches,
    Crosses,
    /// El elemento no toca a ninguno de los otros.
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

/// Los elementos de la otra capa que tocan la extensión de `a`, por elemento de `a`, con las
/// geometrías de ambos calculadas una vez.
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

/// Los elementos de `a` que están en la relación con algún elemento de `b` (con ninguno, para
/// [`Relation::Disjoint`]), en el orden de la capa.
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

/// Lo que copia una unión espacial de los elementos coincidentes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinMode {
    /// Los campos del primer elemento coincidente.
    First,
    /// La suma y la media de cada campo numérico de todos los elementos coincidentes.
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

/// La capa `a` con campos tomados de los elementos de `b` que están en la relación con cada uno de
/// sus elementos (`n_coinc` los cuenta). Los nombres de los campos nuevos empiezan por `prefix`.
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

/// Un punto por elemento de la capa, con sus atributos: el centroide o un punto que está en el
/// propio elemento (dentro de un polígono aunque sea cóncavo o tenga huecos). Los elementos sin
/// geometría se dejan fuera.
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

/// Las partes de un elemento, un elemento cada una: los polígonos (con sus huecos) de un elemento
/// poligonal, las líneas de un elemento lineal. Un elemento puntual se queda como está.
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

/// Divide cada elemento seleccionado que tiene varias partes en un elemento por parte; las piezas
/// conservan los atributos. Devuelve los índices de los elementos que se dividieron y cuántos
/// elementos hay ahora. Los elementos seleccionados salen seleccionados.
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

/// Reúne los elementos seleccionados en uno que tiene todas sus partes, sin unir la geometría (un
/// elemento poligonal mantiene sus polígonos separados, aunque se solapen). El elemento nuevo toma
/// el lugar y los atributos del primer seleccionado.
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

#[cfg(test)]
mod field_list_tests {
    use super::*;

    #[test]
    fn the_fields_of_a_layer_and_of_a_table_come_with_their_numeric_flag() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_fieldlist_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // un GeoJSON
        let g = dir.join("a.geojson");
        std::fs::write(
            &g,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"especie":"pino","altura":12.5},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#,
        )?;
        let f = field_list(&g)?;
        assert_eq!(f.iter().map(|x| (x.name.as_str(), x.numeric)).collect::<Vec<_>>(), vec![("especie", false), ("altura", true)]);
        // el mismo, escrito como shapefile: solo se lee la cabecera
        let (d, kind, prj) = read_layer(&g)?;
        let shp = dir.join("a.shp");
        crate::vector::write_layer(&shp, &d, kind, prj.as_deref())?;
        let f = field_list(&shp)?;
        assert_eq!(f.iter().map(|x| (x.name.as_str(), x.numeric)).collect::<Vec<_>>(), vec![("especie", false), ("altura", true)]);
        // una tabla con coma decimal y una columna vacía
        let t = dir.join("t.csv");
        std::fs::write(&t, "id;peso;nota;vacia\n1;2,5;buena;\n2;3;mala;\n")?;
        let f = field_list(&t)?;
        let flags: Vec<(&str, bool)> = f.iter().map(|x| (x.name.as_str(), x.numeric)).collect();
        assert_eq!(flags, vec![("id", true), ("peso", true), ("nota", false), ("vacia", false)]);
        assert!(field_list(&dir.join("no_existe.shp")).is_err());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
