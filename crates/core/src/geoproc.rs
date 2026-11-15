//! Geoprocesos de capas vectoriales y de tablas que se usan solos o como nodos de un flujo:
//! extraer valores de un ráster a una capa, unir una tabla por un campo, combinar y dividir capas,
//! recortar y borrar con polígonos para cualquier tipo de geometría, distancia al elemento más
//! cercano y estadísticas de resumen por grupos.
//!
//! Todas leen y escriben capas con los lectores y escritores del visor (shapefile, GeoPackage,
//! GeoJSON, KML), de modo que conservan los tipos de campo, y comprueban que las capas y los
//! rásteres estén en el mismo sistema de coordenadas antes de mezclarlos.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use geo::{BooleanOps, Contains, Coord, InteriorPoint, LineString, MultiLineString, MultiPolygon};

use crate::fieldtype::FieldType;
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result};

/// Una capa leída del archivo: los datos, el tipo de geometría y el texto `.prj` que se escribe de
/// vuelta.
type Layer = (VectorData, GeomKind, Option<String>);

fn read(path: &Path) -> Result<Layer> {
    crate::vecops::read_layer(path)
}

fn write(path: &Path, d: &VectorData, kind: GeomKind, prj: Option<&str>) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    crate::vector::write_layer(path, d, kind, prj)
}

/// Falla cuando dos archivos declaran sistemas de coordenadas distintos.
fn same_system(a: &Path, b: &Path) -> Result<()> {
    match (crate::crs::epsg_of(a), crate::crs::epsg_of(b)) {
        (Some(x), Some(y)) if x != y => Err(Error::Param(format!(
            "{} está en EPSG:{x} y {} en EPSG:{y}: llévese una al sistema de la otra antes de combinarlas",
            a.display(),
            b.display()
        ))),
        _ => Ok(()),
    }
}

/// Recalcula la caja de una capa.
fn refit(d: &mut VectorData) {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in d.features.iter().flat_map(|f| f.parts.iter().flatten()) {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    d.bounds = b;
}

/// Una capa vacía con los campos y tipos de `like`.
fn empty_like(like: &VectorData) -> VectorData {
    let mut out = VectorData {
        polygons: like.polygons,
        points: like.points,
        fields: like.fields.clone(),
        ..VectorData::default()
    };
    out.types = (0..like.fields.len()).map(|c| like.field_type(c)).collect();
    out
}

/// Añade a `out` un elemento con la fila `row` de `src` (o una fila vacía si es `None`).
fn push_from(out: &mut VectorData, f: Feature, src: &VectorData, row: Option<usize>) {
    out.features.push(f);
    match row {
        Some(r) => {
            out.text.push(src.text.get(r).cloned().unwrap_or_default());
            out.num.push(src.num.get(r).cloned().unwrap_or_default());
        }
        None => {
            out.text.push(vec![String::new(); out.fields.len()]);
            out.num.push(vec![f64::NAN; out.fields.len()]);
        }
    }
}

/// Un nombre de campo seguro para un shapefile (letras, cifras y guion bajo, hasta 10 bytes).
fn short_name(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .take(10)
        .collect();
    if t.is_empty() { "campo".into() } else { t }
}

/// Un nombre que no está en `fields` (sin distinguir mayúsculas): añade `_2`, `_3`…
fn free_field(fields: &[String], wanted: &str) -> String {
    let taken = |n: &str| fields.iter().any(|f| f.eq_ignore_ascii_case(n));
    if !taken(wanted) {
        return wanted.to_string();
    }
    let base: String = wanted.chars().take(8).collect();
    (2..).map(|k| format!("{base}_{k}")).find(|n| !taken(n)).unwrap_or_default()
}

/// El punto con el que se representa un elemento al muestrear o medir distancias: el propio punto,
/// o un punto que cae dentro de la línea o del polígono.
fn representative(f: &Feature, kind: GeomKind) -> Option<[f64; 2]> {
    match kind {
        GeomKind::Point => f.parts.first().and_then(|p| p.first()).copied(),
        _ => {
            let g = crate::vecops::geometry_of(&f.parts, kind)?;
            let p = g.interior_point()?;
            Some([p.x(), p.y()])
        }
    }
}

// ---------------------------------------------------------------------------------------------
// extraer valores de un ráster

/// Qué se hace con los valores del ráster que caen bajo cada elemento.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampling {
    /// El valor de la celda que contiene el punto.
    Nearest,
    /// La interpolación bilineal entre los cuatro centros de celda más cercanos.
    Bilinear,
}

/// El valor de la banda `band` de `img` en `(x, y)`; NaN fuera del ráster o sobre una celda vacía.
fn sample_band(img: &crate::raster::Image, band: usize, x: f64, y: f64, how: Sampling) -> f64 {
    let data = &img.bands[band];
    let top = img.y0 + img.rows as f64 * img.cell;
    let fx = (x - img.x0) / img.cell;
    let fy = (top - y) / img.cell;
    let at = |c: i64, r: i64| -> f64 {
        if c < 0 || r < 0 || c >= img.cols as i64 || r >= img.rows as i64 {
            f64::NAN
        } else {
            f64::from(data[r as usize * img.cols + c as usize])
        }
    };
    match how {
        Sampling::Nearest => {
            if fx < 0.0 || fy < 0.0 {
                return f64::NAN;
            }
            at(fx.floor() as i64, fy.floor() as i64)
        }
        Sampling::Bilinear => {
            let (gx, gy) = (fx - 0.5, fy - 0.5);
            let (c0, r0) = (gx.floor() as i64, gy.floor() as i64);
            let (tx, ty) = (gx - c0 as f64, gy - r0 as f64);
            let v = [at(c0, r0), at(c0 + 1, r0), at(c0, r0 + 1), at(c0 + 1, r0 + 1)];
            let w = [(1.0 - tx) * (1.0 - ty), tx * (1.0 - ty), (1.0 - tx) * ty, tx * ty];
            let (mut sum, mut wsum) = (0.0, 0.0);
            for (a, b) in v.iter().zip(w) {
                if a.is_finite() {
                    sum += a * b;
                    wsum += b;
                }
            }
            // si falta alguna celda vecina se cae a la que contiene el punto
            if wsum > 0.999_999 {
                sum / wsum
            } else {
                at(fx.floor() as i64, fy.floor() as i64)
            }
        }
    }
}

/// Resultado de una extracción.
pub struct Extracted {
    pub layer: VectorData,
    pub kind: GeomKind,
    pub prj: Option<String>,
    /// Elementos sin valor en algún ráster (fuera de él o sobre una celda vacía).
    pub without: usize,
    pub fields: Vec<String>,
}

/// Copia de la capa con un campo por banda de cada ráster, con el valor del ráster bajo cada
/// elemento (en el punto, o en un punto representativo de la línea o del polígono).
pub fn extract(layer: &Path, rasters: &[PathBuf], bands: Option<&[usize]>, how: Sampling, prefix: &str) -> Result<Extracted> {
    if rasters.is_empty() {
        return Err(Error::Param("hace falta al menos un ráster".into()));
    }
    let (mut d, kind, prj) = read(layer)?;
    let sites: Vec<Option<[f64; 2]>> = d.features.iter().map(|f| representative(f, kind)).collect();
    let mut without = vec![false; d.features.len()];
    let mut names = Vec::new();
    for r in rasters {
        same_system(layer, r)?;
        let img = crate::raster::read_image(r)?;
        let stem = short_name(&crate::paths::stem(r));
        let chosen: Vec<usize> = match bands {
            Some(b) => b.iter().map(|k| k.saturating_sub(1)).collect(),
            None => (0..img.bands.len()).collect(),
        };
        for k in chosen {
            if k >= img.bands.len() {
                return Err(Error::Param(format!("{} no tiene la banda {}", r.display(), k + 1)));
            }
            let base = if img.bands.len() == 1 { format!("{prefix}{stem}") } else { format!("{prefix}{stem}{}", k + 1) };
            let name = free_field(&d.fields, &short_name(&base));
            let col = d.add_field(&name, FieldType::Real);
            for (i, s) in sites.iter().enumerate() {
                let v = s.map_or(f64::NAN, |p| sample_band(&img, k, p[0], p[1], how));
                if v.is_finite() {
                    d.set_value(i, col, &format!("{v}")).map_err(Error::Param)?;
                } else {
                    without[i] = true;
                }
            }
            names.push(name);
        }
    }
    let without = without.iter().filter(|w| **w).count();
    Ok(Extracted { layer: d, kind, prj, without, fields: names })
}

/// Escribe el resultado de [`extract`].
pub fn write_extracted(e: &Extracted, out: &Path) -> Result<()> {
    write(out, &e.layer, e.kind, e.prj.as_deref())
}

// ---------------------------------------------------------------------------------------------
// unir una tabla a una capa

/// Una clave comparable entre tablas: `7`, `7.0` y ` 7 ` son la misma; el texto no distingue
/// mayúsculas.
fn key_of(s: &str) -> String {
    let t = s.trim();
    match t.replace(',', ".").parse::<f64>() {
        Ok(n) if n.is_finite() => format!("{n}"),
        _ => t.to_lowercase(),
    }
}

/// Une a una capa las columnas de una tabla (csv, texto o xlsx) por un campo clave.
pub struct Joined {
    pub layer: VectorData,
    pub kind: GeomKind,
    pub prj: Option<String>,
    /// Elementos de la capa con fila en la tabla.
    pub matched: usize,
    /// Filas de la tabla que no encontraron elemento.
    pub unused: usize,
    pub fields: Vec<String>,
}

pub fn join_table(layer: &Path, table: &Path, layer_key: &str, table_key: &str, prefix: &str) -> Result<Joined> {
    let (mut d, kind, prj) = read(layer)?;
    let t = crate::aba::Table::read(table)?;
    let lk = d
        .fields
        .iter()
        .position(|f| f.eq_ignore_ascii_case(layer_key))
        .ok_or_else(|| Error::Param(format!("la capa no tiene el campo «{layer_key}»; tiene: {}", d.fields.join(", "))))?;
    let tk = t
        .index(table_key)
        .ok_or_else(|| Error::Param(format!("la tabla no tiene la columna «{table_key}»; tiene: {}", t.names.join(", "))))?;
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for r in 0..t.len() {
        by_key.entry(key_of(t.text(r, tk))).or_insert(r);
    }
    let rows: Vec<Option<usize>> = (0..d.features.len())
        .map(|i| by_key.get(&key_of(d.text[i].get(lk).map_or("", String::as_str))).copied())
        .collect();
    let matched = rows.iter().flatten().count();
    let used: std::collections::HashSet<usize> = rows.iter().flatten().copied().collect();
    let mut names = Vec::new();
    for (c, col) in t.names.iter().enumerate() {
        if c == tk {
            continue;
        }
        // una columna de números con coma decimal se guarda con punto
        let raw: Vec<&str> = (0..t.len()).map(|r| t.text(r, c).trim()).collect();
        let comma = raw.iter().any(|v| v.contains(','))
            && raw.iter().all(|v| v.is_empty() || v.replace(',', ".").parse::<f64>().is_ok());
        let values: Vec<String> = raw.iter().map(|v| if comma { v.replace(',', ".") } else { (*v).to_string() }).collect();
        let ty = FieldType::infer(values.iter().map(String::as_str));
        let name = free_field(&d.fields, &short_name(&format!("{prefix}{col}")));
        let idx = d.add_field(&name, ty);
        for (i, row) in rows.iter().enumerate() {
            if let Some(r) = row {
                let v = values[*r].as_str();
                if !v.is_empty() {
                    d.set_value(i, idx, v).map_err(Error::Param)?;
                }
            }
        }
        names.push(name);
    }
    Ok(Joined { layer: d, kind, prj, matched, unused: t.len() - used.len(), fields: names })
}

// ---------------------------------------------------------------------------------------------
// combinar y dividir

/// Combina capas del mismo tipo de geometría en una: los campos son la unión de los de todas (por
/// nombre, sin distinguir mayúsculas) y, con `origin`, un campo más dice de qué archivo vino cada
/// elemento.
pub fn merge(inputs: &[PathBuf], origin: Option<&str>) -> Result<(VectorData, GeomKind, Option<String>)> {
    let first = inputs.first().ok_or_else(|| Error::Param("hacen falta capas que combinar".into()))?;
    let (d0, kind, prj) = read(first)?;
    let mut out = empty_like(&d0);
    let layers: Vec<Layer> = std::iter::once(Ok((d0, kind, prj.clone())))
        .chain(inputs[1..].iter().map(|p| {
            same_system(first, p)?;
            read(p)
        }))
        .collect::<Result<_>>()?;
    for (p, (d, k, _)) in inputs.iter().zip(&layers) {
        if *k != kind {
            return Err(Error::Param(format!(
                "{} es de otro tipo de geometría que {}: solo se combinan capas del mismo tipo",
                p.display(),
                first.display()
            )));
        }
        for (c, f) in d.fields.iter().enumerate() {
            if !out.fields.iter().any(|g| g.eq_ignore_ascii_case(f)) {
                out.add_field(f, d.field_type(c));
            }
        }
    }
    let ofield = origin.map(|n| {
        let name = free_field(&out.fields, &short_name(n));
        out.add_field(&name, FieldType::Text);
        out.fields.len() - 1
    });
    for (p, (d, _, _)) in inputs.iter().zip(&layers) {
        let map: Vec<Option<usize>> = d
            .fields
            .iter()
            .map(|f| out.fields.iter().position(|g| g.eq_ignore_ascii_case(f)))
            .collect();
        let stem = crate::paths::stem(p);
        for (i, f) in d.features.iter().enumerate() {
            push_from(&mut out, f.clone(), d, None);
            let row = out.features.len() - 1;
            for (c, m) in map.iter().enumerate() {
                if let (Some(m), Some(v)) = (m, d.text.get(i).and_then(|r| r.get(c))) {
                    if !v.is_empty() {
                        out.set_value(row, *m, v).map_err(Error::Param)?;
                    }
                }
            }
            if let Some(o) = ofield {
                out.set_value(row, o, &stem).map_err(Error::Param)?;
            }
        }
    }
    refit(&mut out);
    Ok((out, kind, prj))
}

/// Escribe en `dir` una capa por cada valor distinto del campo `field` (`<nombre>_<valor>.<ext>`).
/// Devuelve los archivos escritos.
pub fn split_by(layer: &Path, field: &str, dir: &Path, ext: &str) -> Result<Vec<PathBuf>> {
    let (d, kind, prj) = read(layer)?;
    let c = d
        .fields
        .iter()
        .position(|f| f.eq_ignore_ascii_case(field))
        .ok_or_else(|| Error::Param(format!("la capa no tiene el campo «{field}»; tiene: {}", d.fields.join(", "))))?;
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for i in 0..d.features.len() {
        let v = d.text[i].get(c).cloned().unwrap_or_default();
        match groups.iter_mut().find(|(k, _)| *k == v) {
            Some((_, rows)) => rows.push(i),
            None => groups.push((v, vec![i])),
        }
    }
    if groups.len() > 500 {
        return Err(Error::Param(format!(
            "el campo «{field}» tiene {} valores distintos: dividir daría demasiados archivos",
            groups.len()
        )));
    }
    let stem = crate::paths::stem(layer);
    let mut files = Vec::new();
    for (value, rows) in groups {
        let mut out = empty_like(&d);
        for i in rows {
            push_from(&mut out, d.features[i].clone(), &d, Some(i));
        }
        refit(&mut out);
        let name: String = value
            .chars()
            .map(|ch| if ch.is_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' })
            .collect();
        let name = if name.is_empty() { "vacio".to_string() } else { name };
        let path = crate::paths::unique(&dir.join(format!("{stem}_{name}.{ext}")));
        write(&path, &out, kind, prj.as_deref())?;
        files.push(path);
    }
    Ok(files)
}

// ---------------------------------------------------------------------------------------------
// recortar y borrar con polígonos

/// El conjunto de los polígonos de una capa como un solo multipolígono.
fn mask_of(path: &Path) -> Result<MultiPolygon<f64>> {
    let layer = crate::geomops::read_polygons(path)?;
    let mut mask = MultiPolygon::<f64>::new(vec![]);
    for f in &layer.features {
        mask = mask.union(&f.geom);
    }
    if mask.0.is_empty() {
        return Err(Error::Param(format!("{} no tiene polígonos", path.display())));
    }
    Ok(mask)
}

fn line_parts(m: &MultiLineString<f64>) -> Vec<Vec<[f64; 2]>> {
    m.0.iter()
        .filter(|l| l.0.len() >= 2)
        .map(|l| l.0.iter().map(|c| [c.x, c.y]).collect())
        .collect()
}

/// Recorta `layer` con los polígonos de `mask` (o, con `erase`, borra de ella lo que cubren). Vale
/// para puntos (se conservan los que quedan dentro, o fuera), líneas (se cortan por el contorno) y
/// polígonos (intersección o diferencia); cada pieza conserva los atributos de su elemento y el
/// que no deja nada se descarta. Devuelve la capa y cuántos elementos se descartaron.
pub fn clip_layer(layer: &Path, mask: &Path, erase: bool) -> Result<(VectorData, GeomKind, Option<String>, usize)> {
    same_system(layer, mask)?;
    let (d, kind, prj) = read(layer)?;
    let m = mask_of(mask)?;
    let mut out = empty_like(&d);
    let mut dropped = 0;
    for (i, f) in d.features.iter().enumerate() {
        let parts: Vec<Vec<[f64; 2]>> = match kind {
            GeomKind::Point => f
                .parts
                .iter()
                .filter_map(|p| p.first())
                .filter(|p| m.contains(&geo::Point::new(p[0], p[1])) != erase)
                .map(|p| vec![*p])
                .collect(),
            GeomKind::Line => {
                let ml = MultiLineString::new(
                    f.parts
                        .iter()
                        .map(|p| LineString::new(p.iter().map(|q| Coord { x: q[0], y: q[1] }).collect()))
                        .collect(),
                );
                line_parts(&m.clip(&ml, erase))
            }
            GeomKind::Polygon => {
                let g = crate::geomops::multipolygon_of(&f.parts);
                let r = if erase { g.difference(&m) } else { g.intersection(&m) };
                crate::geomops::parts_of(&r)
            }
        };
        if parts.is_empty() {
            dropped += 1;
            continue;
        }
        let piece = Feature { parts };
        push_from(&mut out, piece, &d, Some(i));
    }
    refit(&mut out);
    Ok((out, kind, prj, dropped))
}

// ---------------------------------------------------------------------------------------------
// cercanía

/// Copia de la capa con la distancia al elemento más cercano de otra capa (campo `dist`), su
/// número de orden (`near_id`) y, si se pide, un campo suyo (`near_<campo>`). La distancia es la
/// mínima entre geometrías, en las unidades de la capa.
pub fn near(layer: &Path, to: &Path, carry: Option<&str>, max: Option<f64>) -> Result<(VectorData, GeomKind, Option<String>, usize)> {
    use geo::Distance;
    same_system(layer, to)?;
    let (mut d, kind, prj) = read(layer)?;
    let (t, tk, _) = read(to)?;
    let carry_col = match carry {
        Some(c) => Some(
            t.fields
                .iter()
                .position(|f| f.eq_ignore_ascii_case(c))
                .ok_or_else(|| Error::Param(format!("«{to:?}» no tiene el campo «{c}»")))?,
        ),
        None => None,
    };
    let targets: Vec<Option<geo::Geometry<f64>>> = t.features.iter().map(|f| crate::vecops::geometry_of(&f.parts, tk)).collect();
    // cajas para descartar los que no pueden ser los más cercanos
    let boxes: Vec<[f64; 4]> = t
        .features
        .iter()
        .map(|f| {
            let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for p in f.parts.iter().flatten() {
                b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
            }
            b
        })
        .collect();
    let box_dist = |b: &[f64; 4], p: [f64; 2]| -> f64 {
        let dx = (b[0] - p[0]).max(0.0).max(p[0] - b[2]);
        let dy = (b[1] - p[1]).max(0.0).max(p[1] - b[3]);
        dx.hypot(dy)
    };
    let dist_col = d.add_field(&free_field(&d.fields, "dist"), FieldType::Real);
    let id_col = d.add_field(&free_field(&d.fields, "near_id"), FieldType::Integer);
    let name = carry.map(|c| free_field(&d.fields, &short_name(&format!("near_{c}"))));
    let carry_new = carry_col.and_then(|c| name.as_ref().map(|n| (c, d.add_field(n, t.field_type(c)))));
    let mut without = 0;
    for i in 0..d.features.len() {
        let g = crate::vecops::geometry_of(&d.features[i].parts, kind);
        let c = representative(&d.features[i], kind);
        let (Some(g), Some(c)) = (g, c) else {
            without += 1;
            continue;
        };
        // las geometrías se miran por orden de la distancia de sus cajas al punto representativo
        let mut order: Vec<(f64, usize)> = boxes.iter().enumerate().map(|(k, b)| (box_dist(b, c), k)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best: Option<(f64, usize)> = None;
        for (bd, k) in order {
            if best.is_some_and(|(b, _)| bd > b + extent_slack(&g)) {
                break;
            }
            let Some(tg) = &targets[k] else { continue };
            let dd = geo::Euclidean.distance(&g, tg);
            if best.is_none_or(|(b, _)| dd < b) {
                best = Some((dd, k));
            }
        }
        match best {
            Some((dd, k)) if max.is_none_or(|m| dd <= m) => {
                d.set_value(i, dist_col, &format!("{dd}")).map_err(Error::Param)?;
                d.set_value(i, id_col, &format!("{}", k + 1)).map_err(Error::Param)?;
                if let Some((c, n)) = carry_new {
                    let v = t.text[k].get(c).cloned().unwrap_or_default();
                    if !v.is_empty() {
                        d.set_value(i, n, &v).map_err(Error::Param)?;
                    }
                }
            }
            _ => without += 1,
        }
    }
    Ok((d, kind, prj, without))
}

/// Cuánto puede alejarse un punto de la geometría de un elemento de su punto representativo:
/// el mayor lado de su caja (las distancias se miden desde el punto representativo solo para
/// ordenar candidatos).
fn extent_slack(g: &geo::Geometry<f64>) -> f64 {
    use geo::BoundingRect;
    g.bounding_rect().map_or(0.0, |r| r.width().hypot(r.height()))
}

// ---------------------------------------------------------------------------------------------
// estadísticas de resumen

/// Qué se calcula de cada campo numérico.
pub const STATS: [&str; 6] = ["count", "sum", "mean", "min", "max", "std"];

/// Una fila de resumen: el valor del grupo y sus estadísticos por campo.
pub struct SummaryRow {
    pub group: String,
    pub count: usize,
    /// Por campo: (suma, media, mínimo, máximo, desviación típica de población).
    pub stats: Vec<(f64, f64, f64, f64, f64)>,
}

/// Estadísticas de resumen de los campos numéricos `fields` de una capa o tabla agrupadas por el
/// campo `by` (sin él, todo es un solo grupo). Devuelve las filas y los nombres de los campos
/// usados.
pub fn summary(input: &Path, by: Option<&str>, fields: &[String]) -> Result<(Vec<SummaryRow>, Vec<String>)> {
    let (names, text): (Vec<String>, Vec<Vec<String>>) = match crate::vecops::read_layer(input) {
        Ok((d, _, _)) => (d.fields.clone(), d.text.clone()),
        Err(_) => {
            let t = crate::aba::Table::read(input)?;
            let rows = (0..t.len()).map(|r| (0..t.names.len()).map(|c| t.text(r, c).to_string()).collect()).collect();
            (t.names.clone(), rows)
        }
    };
    let find = |n: &str| {
        names
            .iter()
            .position(|f| f.eq_ignore_ascii_case(n))
            .ok_or_else(|| Error::Param(format!("no existe el campo «{n}»; hay: {}", names.join(", "))))
    };
    let by_col = by.map(find).transpose()?;
    let num = |s: &str| s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite());
    let cols: Vec<usize> = if fields.is_empty() {
        (0..names.len())
            .filter(|c| Some(*c) != by_col)
            .filter(|c| {
                let v: Vec<Option<f64>> = text.iter().map(|r| r.get(*c).and_then(|s| num(s))).collect();
                v.iter().any(Option::is_some) && v.iter().zip(&text).all(|(n, r)| n.is_some() || r.get(*c).is_none_or(|s| s.trim().is_empty()))
            })
            .collect()
    } else {
        fields.iter().map(|f| find(f)).collect::<Result<_>>()?
    };
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, r) in text.iter().enumerate() {
        let g = by_col.map_or(String::new(), |c| r.get(c).cloned().unwrap_or_default());
        match groups.iter_mut().find(|(k, _)| *k == g) {
            Some((_, v)) => v.push(i),
            None => groups.push((g, vec![i])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    let rows = groups
        .into_iter()
        .map(|(group, idx)| {
            let stats = cols
                .iter()
                .map(|c| {
                    let v: Vec<f64> = idx.iter().filter_map(|i| text[*i].get(*c).and_then(|s| num(s))).collect();
                    if v.is_empty() {
                        return (0.0, f64::NAN, f64::NAN, f64::NAN, f64::NAN);
                    }
                    let n = v.len() as f64;
                    let sum: f64 = v.iter().sum();
                    let mean = sum / n;
                    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
                    let min = v.iter().copied().fold(f64::INFINITY, f64::min);
                    let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    (sum, mean, min, max, var.sqrt())
                })
                .collect();
            SummaryRow { group, count: idx.len(), stats }
        })
        .collect();
    Ok((rows, cols.iter().map(|c| names[*c].clone()).collect()))
}

/// La tabla de [`summary`] como CSV (un campo de grupo, el recuento y cinco columnas por campo).
pub fn summary_csv(rows: &[SummaryRow], fields: &[String], by: Option<&str>) -> String {
    let mut s = format!("{},n", by.unwrap_or("grupo"));
    for f in fields {
        for st in ["suma", "media", "min", "max", "desv"] {
            s.push_str(&format!(",{f}_{st}"));
        }
    }
    s.push('\n');
    for r in rows {
        s.push_str(&format!("{},{}", r.group.replace(',', ";"), r.count));
        for (sum, mean, min, max, sd) in &r.stats {
            for v in [sum, mean, min, max, sd] {
                s.push_str(&if v.is_nan() { ",".to_string() } else { format!(",{v}") });
            }
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hipso_geoproc_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).ok();
        d
    }

    /// Una capa de puntos con un campo `id` y otro `sp`.
    fn points(path: &Path, pts: &[(f64, f64, &str, &str)]) -> Result<()> {
        let mut d = VectorData { points: true, ..VectorData::default() };
        d.add_field("id", FieldType::Text);
        d.add_field("sp", FieldType::Text);
        for (i, (x, y, id, sp)) in pts.iter().enumerate() {
            push_from(&mut d, Feature { parts: vec![vec![[*x, *y]]] }, &VectorData::default(), None);
            d.set_value(i, 0, id).map_err(Error::Param)?;
            d.set_value(i, 1, sp).map_err(Error::Param)?;
        }
        refit(&mut d);
        write(path, &d, GeomKind::Point, None)
    }

    fn square(path: &Path, x0: f64, y0: f64, x1: f64, y1: f64) -> Result<()> {
        let mut d = VectorData { polygons: true, ..VectorData::default() };
        d.add_field("n", FieldType::Integer);
        push_from(&mut d, Feature { parts: vec![vec![[x0, y0], [x0, y1], [x1, y1], [x1, y0], [x0, y0]]] }, &VectorData::default(), None);
        d.set_value(0, 0, "1").map_err(Error::Param)?;
        refit(&mut d);
        write(path, &d, GeomKind::Polygon, None)
    }

    #[test]
    fn a_table_joins_by_key_and_reports_what_did_not_match() -> Result<()> {
        let d = dir("join");
        points(&d.join("p.shp"), &[(1.0, 1.0, "7", "pino"), (2.0, 2.0, "8", "haya"), (3.0, 3.0, "9", "pino")])?;
        std::fs::write(d.join("t.csv"), "parcela;vol;nota\n7.0;120,5;a\n8;80;b\n99;1;c\n")?;
        let j = join_table(&d.join("p.shp"), &d.join("t.csv"), "id", "parcela", "")?;
        assert_eq!((j.matched, j.unused), (2, 1));
        assert_eq!(j.fields.len(), 2);
        let vol = j.layer.fields.iter().position(|f| f == "vol").ok_or_else(|| Error::Param("sin vol".into()))?;
        assert_eq!(j.layer.num[0][vol], 120.5);
        assert!(j.layer.text[2][vol].is_empty());
        Ok(())
    }

    #[test]
    fn layers_merge_with_the_union_of_fields_and_split_back_by_value() -> Result<()> {
        let d = dir("merge");
        points(&d.join("a.shp"), &[(0.0, 0.0, "1", "pino")])?;
        points(&d.join("b.shp"), &[(5.0, 5.0, "2", "haya"), (6.0, 6.0, "3", "pino")])?;
        let (m, kind, prj) = merge(&[d.join("a.shp"), d.join("b.shp")], Some("origen"))?;
        assert_eq!(m.features.len(), 3);
        assert!(m.fields.iter().any(|f| f == "origen"));
        let out = d.join("todo.shp");
        write(&out, &m, kind, prj.as_deref())?;
        let files = split_by(&out, "sp", &d.join("por_especie"), "shp")?;
        assert_eq!(files.len(), 2);
        let (pino, _, _) = read(files.iter().find(|f| f.to_string_lossy().contains("pino")).ok_or_else(|| Error::Param("sin pino".into()))?)?;
        assert_eq!(pino.features.len(), 2);
        // capas de otro tipo no se combinan
        square(&d.join("s.shp"), 0.0, 0.0, 1.0, 1.0)?;
        assert!(merge(&[d.join("a.shp"), d.join("s.shp")], None).is_err());
        Ok(())
    }

    #[test]
    fn clipping_works_for_points_lines_and_polygons_and_erase_is_the_complement() -> Result<()> {
        let d = dir("clip");
        square(&d.join("mask.shp"), 0.0, 0.0, 10.0, 10.0)?;
        points(&d.join("p.shp"), &[(5.0, 5.0, "d", ""), (15.0, 5.0, "f", "")])?;
        let (inside, _, _, dropped) = clip_layer(&d.join("p.shp"), &d.join("mask.shp"), false)?;
        assert_eq!((inside.features.len(), dropped), (1, 1));
        let (outside, _, _, _) = clip_layer(&d.join("p.shp"), &d.join("mask.shp"), true)?;
        assert_eq!(outside.text[0][0], "f");
        // una línea de x = -5 a 15 por y = 5 queda de 0 a 10 dentro y en dos trozos fuera
        let mut l = VectorData::default();
        l.add_field("k", FieldType::Text);
        push_from(&mut l, Feature { parts: vec![vec![[-5.0, 5.0], [15.0, 5.0]]] }, &VectorData::default(), None);
        refit(&mut l);
        write(&d.join("l.shp"), &l, GeomKind::Line, None)?;
        let (li, _, _, _) = clip_layer(&d.join("l.shp"), &d.join("mask.shp"), false)?;
        let len: f64 = li.features[0].parts.iter().map(|p| p.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f64>()).sum();
        assert!((len - 10.0).abs() < 1e-9, "{len}");
        let (lo, _, _, _) = clip_layer(&d.join("l.shp"), &d.join("mask.shp"), true)?;
        assert_eq!(lo.features[0].parts.len(), 2);
        // un polígono medio dentro: intersección de 5 x 10
        square(&d.join("q.shp"), 5.0, 0.0, 15.0, 10.0)?;
        let (pi, _, _, _) = clip_layer(&d.join("q.shp"), &d.join("mask.shp"), false)?;
        let area = crate::geomops::multipolygon_of(&pi.features[0].parts);
        use geo::Area;
        assert!((area.unsigned_area() - 50.0).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn near_gives_distance_id_and_a_carried_field() -> Result<()> {
        let d = dir("near");
        points(&d.join("a.shp"), &[(0.0, 0.0, "a", ""), (10.0, 0.0, "b", "")])?;
        points(&d.join("t.shp"), &[(3.0, 4.0, "x", "uno"), (10.0, 2.0, "y", "dos")])?;
        let (r, _, _, without) = near(&d.join("a.shp"), &d.join("t.shp"), Some("sp"), None)?;
        assert_eq!(without, 0);
        let dc = r.fields.iter().position(|f| f == "dist").ok_or_else(|| Error::Param("sin dist".into()))?;
        assert!((r.num[0][dc] - 5.0).abs() < 1e-9 && (r.num[1][dc] - 2.0).abs() < 1e-9);
        let nc = r.fields.iter().position(|f| f == "near_sp").ok_or_else(|| Error::Param("sin near_sp".into()))?;
        assert_eq!(r.text[1][nc], "dos");
        // con un máximo, el lejano queda sin valor
        let (_, _, _, w) = near(&d.join("a.shp"), &d.join("t.shp"), None, Some(3.0))?;
        assert_eq!(w, 1);
        Ok(())
    }

    #[test]
    fn summary_groups_and_uses_numeric_columns_only() -> Result<()> {
        let d = dir("sum");
        std::fs::write(d.join("t.csv"), "rodal;vol;sp\nA;10;pino\nA;20;pino\nB;5;haya\n")?;
        let (rows, fields) = summary(&d.join("t.csv"), Some("rodal"), &[])?;
        assert_eq!(fields, vec!["vol".to_string()]);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].count, rows[0].stats[0].1), (2, 15.0));
        assert!(summary_csv(&rows, &fields, Some("rodal")).starts_with("rodal,n,vol_suma"));
        Ok(())
    }
}
