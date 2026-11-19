//! Operaciones sobre capas de polígonos: comprobación de validez y reparación, búfer, disolver,
//! unión e intersección, y las ediciones de cortar y adosar de polígonos sueltos.
//!
//! La geometría sale de los algoritmos del crate `geo` (reglas de validez de OGC; reparación según
//! Ledoux, Arroyo Ohori y Meijers 2014; operaciones booleanas; búfer). Una capa conserva su tabla
//! de atributos: las operaciones que fusionan elementos dicen cuáles son los atributos del
//! resultado.

use std::collections::BTreeMap;
use std::path::Path;

use geo::{Area, BooleanOps, BoundingRect, Buffer, Coord, LineString, MultiPolygon, Polygon as GPoly};
use geo::algorithm::repair_polygon::MakeValid;
use geo::algorithm::validation::Validation;

use crate::vector::{self, ColKind, Region, Shape2, Table, Val};
use crate::{Error, Result};

/// Un elemento: su geometría y sus valores de atributo (campo, texto).
#[derive(Clone, Debug)]
pub struct Feature {
    pub geom: MultiPolygon<f64>,
    pub attrs: Vec<(String, String)>,
}

/// Una capa de polígonos.
#[derive(Clone, Debug, Default)]
pub struct PolyLayer {
    pub features: Vec<Feature>,
    /// El texto `.prj` del archivo leído, que se vuelve a escribir con el resultado.
    pub prj: Option<String>,
}

/// Los polígonos (con huecos y partes) de un conjunto de anillos, cuya orientación no se fía: un
/// anillo es un hueco cuando un número impar de otros anillos lo contienen, y pertenece al anillo
/// más interno que lo contiene.
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

pub fn rings_of(region: &Region) -> Vec<Vec<[f64; 2]>> {
    match &region.shape {
        Shape2::Polygon(p) => p.rings.clone(),
        other => crate::plots::rings(other),
    }
}

/// Lee los polígonos de una capa de shapefile, GeoJSON o GeoPackage con sus atributos.
pub fn read_polygons(path: &Path) -> Result<PolyLayer> {
    read_polygons_with(path, &vector::ReadOptions::default()).map_err(|e| {
        // el lector de regiones habla de recortar; aquí lo que pasa es que la herramienta pide polígonos
        let m = e.to_string();
        if m.contains("no sirven para recortar puntos") {
            let kind = m.split("formas de tipo ").nth(1).and_then(|t| t.split(' ').next()).unwrap_or("otro tipo");
            Error::Param(format!(
                "{}: esta herramienta solo trabaja con capas de polígonos y la capa tiene formas de tipo {kind}",
                path.display()
            ))
        } else {
            e
        }
    })
}

/// Como [`read_polygons`], con opciones: una capa de puntos se convierte en círculos de
/// `opts.radius`.
pub fn read_polygons_with(path: &Path, opts: &vector::ReadOptions) -> Result<PolyLayer> {
    let regions = vector::read_regions(path, opts)?;
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

/// La tabla de atributos de una capa: una columna es numérica cuando todos sus valores son números;
/// si no, texto. Los nombres se cortan a los 10 caracteres del formato y se mantienen distintos.
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

/// Escribe una capa como shapefile (con su `.prj` cuando lo tiene).
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

/// Los elementos que no son válidos, con lo que les pasa: `(índice del elemento, motivos)`.
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

/// La capa con cada elemento inválido reparado (regla par-impar): los cruces consigo mismo pasan a
/// ser partes, los bordes duplicados se cancelan. Los elementos válidos no se tocan. Se devuelve el
/// número de elementos cambiados.
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

/// Cada elemento ampliado `dist` unidades del mapa (reducido si es negativa), con esquinas
/// redondeadas. Los elementos que desaparecen al reducirse conservan su fila sin geometría.
pub fn buffer(layer: &PolyLayer, dist: f64) -> Result<PolyLayer> {
    require_valid(layer, "hacer el buffer")?;
    let mut out = layer.clone();
    for f in &mut out.features {
        f.geom = f.geom.buffer(dist);
    }
    Ok(out)
}

/// Fusiona elementos. Con `field`, los que comparten su valor pasan a ser un elemento (se conserva
/// ese valor, más `n` el número de fusionados); sin él, toda la capa pasa a ser un elemento con
/// `n`.
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

/// Las extensiones de los elementos de una capa (no finita para un elemento sin geometría) y un
/// índice sobre ellas, de modo que los elementos cercanos a una ventana se encuentran sin mirarlos
/// todos.
fn index_of(layer: &PolyLayer) -> (Vec<[f64; 4]>, crate::rtree::RTree) {
    let boxes: Vec<[f64; 4]> = layer
        .features
        .iter()
        .map(|f| f.geom.bounding_rect().map_or([f64::NAN; 4], |r| [r.min().x, r.min().y, r.max().x, r.max().y]))
        .collect();
    let tree = crate::rtree::RTree::new(&boxes);
    (boxes, tree)
}

/// Los elementos de la capa indexada cuya extensión corta la de `g`, en el orden de la capa.
fn near(tree: &crate::rtree::RTree, g: &MultiPolygon<f64>) -> Vec<usize> {
    let Some(r) = g.bounding_rect() else { return Vec::new() };
    let mut hits = tree.query([r.min().x, r.min().y, r.max().x, r.max().y]);
    hits.sort_unstable();
    hits
}

/// La unión de dos capas como superposición: piezas en ambas (con los atributos de ambas, los de
/// `b` tras un prefijo `b_`), solo en `a` y solo en `b`. Las piezas sin área se descartan.
pub fn union(a: &PolyLayer, b: &PolyLayer) -> Result<PolyLayer> {
    require_valid(a, "la unión")?;
    require_valid(b, "la unión")?;
    let ((_, tree_a), (_, tree_b)) = (index_of(a), index_of(b));
    let eps = 1e-9;
    let mut features = Vec::new();
    let prefixed = |f: &Feature| -> Vec<(String, String)> { f.attrs.iter().map(|(k, v)| (format!("b_{k}"), v.clone())).collect() };
    for fa in &a.features {
        let mut rest = fa.geom.clone();
        for fb in near(&tree_b, &fa.geom).into_iter().map(|j| &b.features[j]) {
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
        for fa in near(&tree_a, &fb.geom).into_iter().map(|i| &a.features[i]) {
            rest = rest.difference(&fa.geom);
        }
        if rest.unsigned_area() > eps {
            features.push(Feature { geom: rest, attrs: prefixed(fb) });
        }
    }
    Ok(PolyLayer { features, prj: a.prj.clone().or_else(|| b.prj.clone()) })
}

/// La intersección de dos capas: solo las piezas que están en ambas, con los atributos de ambas
/// (los de `b` tras un prefijo `b_`). Las piezas sin área se descartan.
pub fn intersect(a: &PolyLayer, b: &PolyLayer) -> Result<PolyLayer> {
    require_valid(a, "la intersección")?;
    require_valid(b, "la intersección")?;
    let (_, tree_b) = index_of(b);
    let mut features = Vec::new();
    for fa in &a.features {
        for fb in near(&tree_b, &fa.geom).into_iter().map(|j| &b.features[j]) {
            let both = fa.geom.intersection(&fb.geom);
            if both.unsigned_area() > 1e-9 {
                let mut attrs = fa.attrs.clone();
                attrs.extend(fb.attrs.iter().map(|(k, v)| (format!("b_{k}"), v.clone())));
                features.push(Feature { geom: both, attrs });
            }
        }
    }
    Ok(PolyLayer { features, prj: a.prj.clone().or_else(|| b.prj.clone()) })
}

/// Los elementos de `a` sin lo que cubre `b`; las piezas conservan los atributos de `a`. Los
/// elementos de los que no queda nada se descartan.
pub fn difference(a: &PolyLayer, b: &PolyLayer) -> Result<PolyLayer> {
    require_valid(a, "la diferencia")?;
    require_valid(b, "la diferencia")?;
    let (_, tree_b) = index_of(b);
    let mut features = Vec::new();
    for fa in &a.features {
        let mut rest = fa.geom.clone();
        for fb in near(&tree_b, &fa.geom).into_iter().map(|j| &b.features[j]) {
            rest = rest.difference(&fb.geom);
        }
        if rest.unsigned_area() > 1e-9 {
            features.push(Feature { geom: rest, attrs: fa.attrs.clone() });
        }
    }
    Ok(PolyLayer { features, prj: a.prj.clone().or_else(|| b.prj.clone()) })
}

/// Lo que está en una capa o en la otra pero no en ambas: lo que solo cubre `a` (con sus atributos)
/// y lo que solo cubre `b` (con sus atributos tras un prefijo `b_`).
pub fn symmetric_difference(a: &PolyLayer, b: &PolyLayer) -> Result<PolyLayer> {
    let mut out = difference(a, b)?;
    let only_b = difference(b, a)?;
    out.features.extend(only_b.features.into_iter().map(|mut f| {
        f.attrs = f.attrs.into_iter().map(|(k, v)| (format!("b_{k}"), v)).collect();
        f
    }));
    Ok(out)
}

/// La unión de los polígonos de varios elementos como partes (anillos de polígonos con sus huecos).
/// Las astillas y los huecos más estrechos que una millonésima de la extensión, que dejan los
/// bordes que deberían coincidir cuando no lo hacen del todo (tras un corte o dibujados a mano), se
/// cierran, de modo que el límite entre vecinos desaparece.
pub fn merge_parts(sets: &[&[Vec<[f64; 2]>]]) -> Vec<Vec<[f64; 2]>> {
    let polys: Vec<MultiPolygon<f64>> = sets.iter().map(|s| multipolygon_of(s)).collect();
    let all = geo::algorithm::bool_ops::unary_union(polys.iter());
    let tol = tolerance(&all);
    // las esquinas redondeadas que deja el búfer se vuelven a enderezar
    parts_of(&geo::Simplify::simplify(&all.buffer(tol).buffer(-tol), 2.0 * tol))
}

/// Los anillos de un multipolígono como partes cerradas: para cada polígono su anillo exterior y
/// luego sus huecos.
pub fn parts_of(m: &MultiPolygon<f64>) -> Vec<Vec<[f64; 2]>> {
    // los exteriores van en el sentido de las agujas del reloj y los huecos en sentido contrario,
    // como los guarda el resto del programa
    let ring = |l: &LineString<f64>, exterior: bool| {
        let mut r: Vec<[f64; 2]> = l.0.iter().map(|c| [c.x, c.y]).collect();
        if (crate::geojson::signed_area(&r) > 0.0) == exterior {
            r.reverse();
        }
        r
    };
    m.0.iter()
        .flat_map(|p| std::iter::once(ring(p.exterior(), true)).chain(p.interiors().iter().map(|h| ring(h, false))))
        .collect()
}

/// Corta un elemento poligonal (sus partes) con una línea que empieza y termina fuera de él: las
/// piezas a cada lado de la línea, cada una como partes. Una sola pieza cuando la línea no lo
/// divide.
pub fn cut_by_line(parts: &[Vec<[f64; 2]>], line: &[[f64; 2]]) -> Vec<Vec<Vec<[f64; 2]>>> {
    let poly = multipolygon_of(parts);
    let whole = || vec![parts.to_vec()];
    let Some(r) = poly.bounding_rect() else { return whole() };
    if line.len() < 2 {
        return whole();
    }
    // la línea se engrosa hasta una astilla que divide el polígono, y cada pieza vuelve a crecer
    // hasta la línea en la misma cantidad
    let eps = (r.width().hypot(r.height()) * 1e-7).max(f64::MIN_POSITIVE);
    let ls = LineString::new(line.iter().map(|p| Coord { x: p[0], y: p[1] }).collect());
    let cut = ls.buffer(eps);
    let rest = poly.difference(&cut);
    let pieces: Vec<GPoly<f64>> = rest.0.into_iter().filter(|p| p.unsigned_area() > eps * eps).collect();
    if pieces.len() < 2 {
        return whole();
    }
    pieces
        .into_iter()
        .map(|p| parts_of(&MultiPolygon::new(vec![p]).buffer(eps).intersection(&poly)))
        .collect()
}

/// Un polígono nuevo dibujado sobre otros existentes, sin lo que ya está cubierto: las partes de
/// `ring` menos `others`. Vacío cuando no queda nada.
///
/// El terreno que el anillo y los polígonos encierran entre sí (una bolsa que deja atrás el lado de
/// cierre del anillo) también se toma, de modo que el polígono nuevo encuentra a los demás sin
/// hueco.
pub fn attach(ring: &[[f64; 2]], others: &[&[Vec<[f64; 2]>]]) -> Vec<Vec<[f64; 2]>> {
    let new = multipolygon_of(&[ring.to_vec()]);
    let polys: Vec<MultiPolygon<f64>> = others.iter().map(|o| multipolygon_of(o)).collect();
    let covered = geo::algorithm::bool_ops::unary_union(polys.iter());
    let tol = tolerance(&new);
    // las astillas entre polígonos que no se tocan del todo (tras un corte) forman parte de lo que
    // está cubierto, o el polígono nuevo las rellenaría como finas púas a lo largo de los bordes
    // antiguos
    let slivers = covered.buffer(tol).buffer(-tol).difference(&covered);
    let covered = covered.union(&MultiPolygon::new(slivers.0.into_iter().filter(|p| p.unsigned_area() > 100.0 * tol * tol).collect()));
    let pockets = holes_of(&new.union(&covered)).difference(&holes_of(&covered));
    let pockets = MultiPolygon::new(pockets.0.into_iter().filter(|p| p.unsigned_area() > tol * tol).collect());
    // lo que queda de las astillas (muescas demasiado pequeñas para verse) se endereza
    let m = geo::Simplify::simplify(&new.union(&pockets).difference(&covered), tol);
    if m.unsigned_area() <= 1e-9 { Vec::new() } else { parts_of(&m) }
}

/// Los huecos de un multipolígono como polígonos.
fn holes_of(m: &MultiPolygon<f64>) -> MultiPolygon<f64> {
    MultiPolygon::new(m.0.iter().flat_map(|p| p.interiors().iter().map(|h| GPoly::new(h.clone(), Vec::new()))).collect())
}

/// Una distancia demasiado pequeña para importar en el mapa (una millonésima de su extensión), por
/// debajo de la cual dos bordes que deberían coincidir se toman como uno.
fn tolerance(m: &MultiPolygon<f64>) -> f64 {
    m.bounding_rect().map_or(1e-9, |r| (r.width().hypot(r.height()) * 1e-6).max(1e-9))
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
        // el otro no se toca y la tabla lo sigue
        assert_eq!(fixed.features[1].attrs[0].1, "2");
        // las operaciones rechazan una entrada inválida y dicen qué hacer
        let e = buffer(&l, 1.0).unwrap_err().to_string();
        assert!(e.contains("make-valid"), "{e}");
    }

    #[test]
    fn difference_cuts_away_what_the_other_layer_covers() -> Result<()> {
        let a = layer(vec![feat(vec![sq(0.0, 0.0, 10.0)], &[("id", "1")]), feat(vec![sq(50.0, 0.0, 5.0)], &[("id", "2")])]);
        let b = layer(vec![feat(vec![sq(5.0, 0.0, 10.0)], &[("k", "x")]), feat(vec![sq(48.0, -1.0, 20.0)], &[("k", "y")])]);
        let d = difference(&a, &b)?;
        assert_eq!(d.features.len(), 1, "the second polygon is covered whole");
        assert!((d.features[0].geom.unsigned_area() - 50.0).abs() < 1e-6);
        assert_eq!(d.features[0].attrs, [("id".to_string(), "1".to_string())]);
        let s = symmetric_difference(&a, &b)?;
        let area: f64 = s.features.iter().map(|f| f.geom.unsigned_area()).sum();
        // solo a: 50; solo b: 100 - 50 (solape con el primero) + (400 - 25) = 425
        assert!((area - 475.0).abs() < 1e-6, "{area}");
        assert!(s.features.iter().any(|f| f.attrs.iter().any(|(k, _)| k == "b_k")));
        Ok(())
    }

    #[test]
    fn neighbours_that_do_not_quite_touch_merge_with_no_border_left() {
        // dos cuadrados con un hueco de 1e-5 entre ellos, y uno con un solape en astilla
        let (a, b, c) = (vec![sq(0.0, 0.0, 10.0)], vec![sq(10.00001, 0.0, 10.0)], vec![sq(19.99999, 0.0, 10.0)]);
        let parts = merge_parts(&[&a, &b, &c]);
        assert_eq!(parts.len(), 1, "one ring: no hole and no second part: {parts:?}");
        assert!(parts[0].len() <= 12, "no stray vertices: {}", parts[0].len());
        assert!((multipolygon_of(&parts).unsigned_area() - 300.0).abs() < 1e-3);
    }

    #[test]
    fn an_attached_polygon_fills_the_pocket_the_closing_side_leaves() {
        // una U de tres polígonos y un anillo desde dentro de un brazo, por arriba, hasta el otro
        // brazo
        let (left, right, floor) = (vec![sq(0.0, 0.0, 10.0)], vec![sq(20.0, 0.0, 10.0)], vec![vec![[0.0, 0.0], [0.0, 2.0], [30.0, 2.0], [30.0, 0.0]]]);
        let ring = vec![[5.0, 5.0], [5.0, 20.0], [25.0, 20.0], [25.0, 5.0]];
        let parts = attach(&ring, &[&left, &right, &floor]);
        let area = multipolygon_of(&parts).unsigned_area();
        // el hueco entre los brazos hasta y = 20 (10 x 18) y el anillo sobre cada brazo (2 x 50)
        assert!((area - 280.0).abs() < 1e-6, "{area}");
    }

    #[test]
    fn an_attached_polygon_does_not_fill_the_sliver_between_neighbours() {
        let (a, b) = (vec![sq(0.0, 0.0, 10.0)], vec![sq(10.00001, 0.0, 10.0)]);
        let ring = vec![[5.0, 5.0], [5.0, 20.0], [15.0, 20.0], [15.0, 5.0]];
        let parts = attach(&ring, &[&a, &b]);
        assert_eq!(parts.len(), 1);
        assert!(parts[0].len() <= 6, "no spike down the sliver: {:?}", parts[0]);
        assert!((multipolygon_of(&parts).unsigned_area() - 100.0).abs() < 1e-2);
    }

    #[test]
    fn merged_polygons_are_one_and_their_rings_are_oriented() {
        let (a, b, far) = (vec![sq(0.0, 0.0, 10.0)], vec![sq(5.0, 0.0, 10.0)], vec![sq(100.0, 0.0, 4.0)]);
        let parts = merge_parts(&[&a, &b, &far]);
        assert_eq!(parts.len(), 2);
        assert!((multipolygon_of(&parts).unsigned_area() - (150.0 + 16.0)).abs() < 1e-2);
        // exteriores en el sentido de las agujas del reloj, como los guarda el programa
        assert!(parts.iter().all(|r| crate::geojson::signed_area(r) < 0.0));
    }

    #[test]
    fn buffer_grows_by_the_distance() {
        let l = layer(vec![feat(vec![sq(0.0, 0.0, 100.0)], &[])]);
        let g = &buffer(&l, 10.0).unwrap().features[0].geom;
        // un cuadrado ampliado 10 con esquinas redondeadas: 100² + 4·100·10 + π·10²
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
    fn intersect_keeps_only_the_common_pieces() {
        let a = layer(vec![feat(vec![sq(0.0, 0.0, 10.0)], &[("a", "1")])]);
        let b = layer(vec![feat(vec![sq(5.0, 0.0, 10.0)], &[("k", "x")]), feat(vec![sq(50.0, 0.0, 5.0)], &[("k", "y")])]);
        let r = intersect(&a, &b).unwrap();
        assert_eq!(r.features.len(), 1);
        assert!((r.features[0].geom.unsigned_area() - 50.0).abs() < 1e-6);
        assert_eq!(r.features[0].attrs[1].0, "b_k");
    }

    #[test]
    fn a_line_cuts_a_polygon_in_two_that_add_up() {
        let square = sq(0.0, 0.0, 10.0);
        let pieces = cut_by_line(&[square.clone()], &[[5.0, -3.0], [5.0, 13.0]]);
        assert_eq!(pieces.len(), 2);
        let areas: Vec<f64> = pieces.iter().map(|p| multipolygon_of(p).unsigned_area()).collect();
        assert!((areas[0] + areas[1] - 100.0).abs() < 1e-3, "{areas:?}");
        assert!((areas[0] - 50.0).abs() < 1e-3);
        // una línea que se queda fuera deja el polígono entero
        assert_eq!(cut_by_line(&[square], &[[20.0, 0.0], [20.0, 10.0]]).len(), 1);
    }

    #[test]
    fn a_polygon_drawn_over_another_keeps_only_the_new_ground() {
        let old = vec![sq(0.0, 0.0, 10.0)];
        let drawn = vec![[5.0, 5.0], [5.0, 15.0], [15.0, 15.0], [15.0, 5.0]];
        let parts = attach(&drawn, &[&old]);
        assert!((multipolygon_of(&parts).unsigned_area() - 75.0).abs() < 1e-6);
        // dibujado dentro del antiguo no queda nada
        assert!(attach(&sq(2.0, 2.0, 3.0), &[&old]).is_empty());
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
    #[test]
    fn a_layer_of_lines_is_refused_as_polygons_with_a_clear_message() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_lines_as_polys_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let p = dir.join("l.shp");
        let mut d = crate::view::VectorData { polygons: false, points: false, ..crate::view::VectorData::default() };
        d.features = vec![crate::view::Feature { parts: vec![vec![[0.0, 0.0], [1.0, 1.0]]] }];
        d.text = vec![Vec::new()];
        d.num = vec![Vec::new()];
        crate::vector::write_layer(&p, &d, crate::view::GeomKind::Line, None)?;
        let e = read_polygons(&p).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("solo trabaja con capas de polígonos") && !e.contains("recortar"), "{e}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
