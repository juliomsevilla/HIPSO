//! What comes out of a tree segmentation besides the raster: crown polygons, shapefiles, the
//! tree id written into the points, and a check against a field inventory.

use std::path::{Path, PathBuf};

use crate::grid::Grid;
use crate::las::{ExtraAttr, ExtraKind, Reader, Writer, WriterOptions};
use crate::trees::Tree;
use crate::{Error, Result};

fn fmt_err(e: impl std::fmt::Display) -> Error {
    Error::Format(e.to_string())
}

/// Convex hull (Andrew's monotone chain), counter-clockwise, without the closing vertex.
pub fn convex_hull(mut p: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut h: Vec<[f64; 2]> = Vec::new();
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &[f64; 2]>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for &q in it {
            while h.len() >= start + 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

pub fn polygon_area(p: &[[f64; 2]]) -> f64 {
    (p.iter()
        .zip(p.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.0)
        .abs()
}

/// The crown outline of every tree: the convex hull of its cells' corners, by tree id.
pub fn crown_hulls(g: &Grid, ids: &[u32]) -> std::collections::BTreeMap<u32, Vec<[f64; 2]>> {
    let mut corners: std::collections::BTreeMap<u32, Vec<[f64; 2]>> =
        std::collections::BTreeMap::new();
    for (i, &id) in ids.iter().enumerate() {
        if id == 0 {
            continue;
        }
        let (c, r) = (i % g.cols, i / g.cols);
        let (x, y) = g.center(c, r);
        let h = g.cell / 2.0;
        let e = corners.entry(id).or_default();
        e.extend([
            [x - h, y - h],
            [x + h, y - h],
            [x + h, y + h],
            [x - h, y + h],
        ]);
    }
    corners
        .into_iter()
        .map(|(k, v)| (k, convex_hull(v)))
        .collect()
}

pub fn write_tops_shp(
    path: &Path,
    trees: &[Tree],
    hulls: &std::collections::BTreeMap<u32, Vec<[f64; 2]>>,
    epsg: Option<u16>,
) -> Result<()> {
    use shapefile::dbase::{FieldName, FieldValue, Record, TableWriterBuilder};
    let n = |s: &str| FieldName::try_from(s).map_err(|e| Error::Param(format!("{e:?}")));
    let table = TableWriterBuilder::new()
        .add_numeric_field(n("TREEID")?, 9, 0)
        .add_numeric_field(n("Z")?, 10, 2)
        .add_numeric_field(n("AREA")?, 10, 2)
        .add_numeric_field(n("HULLAREA")?, 10, 2)
        .add_numeric_field(n("CROWNDIAM")?, 8, 2);
    let mut w = shapefile::Writer::from_path(path, table).map_err(fmt_err)?;
    for t in trees {
        let mut r = Record::default();
        let hull = hulls.get(&t.id).map_or(0.0, |h| polygon_area(h));
        r.insert("TREEID".into(), FieldValue::Numeric(Some(f64::from(t.id))));
        r.insert("Z".into(), FieldValue::Numeric(Some(f64::from(t.height))));
        r.insert("AREA".into(), FieldValue::Numeric(Some(t.area)));
        r.insert("HULLAREA".into(), FieldValue::Numeric(Some(hull)));
        r.insert(
            "CROWNDIAM".into(),
            FieldValue::Numeric(Some(2.0 * (t.area / std::f64::consts::PI).sqrt())),
        );
        w.write_shape_and_record(
            &shapefile::PointZ::new(t.x, t.y, f64::from(t.height), 0.0),
            &r,
        )
        .map_err(fmt_err)?;
    }
    drop(w);
    if let Some(prj) = epsg.and_then(crate::vector::prj_for_epsg) {
        std::fs::write(path.with_extension("prj"), prj)?;
    }
    Ok(())
}

pub fn write_crowns_shp(
    path: &Path,
    trees: &[Tree],
    hulls: &std::collections::BTreeMap<u32, Vec<[f64; 2]>>,
) -> Result<()> {
    use shapefile::dbase::{FieldName, FieldValue, Record, TableWriterBuilder};
    use shapefile::{Point, Polygon, PolygonRing};
    let n = |s: &str| FieldName::try_from(s).map_err(|e| Error::Param(format!("{e:?}")));
    let table = TableWriterBuilder::new()
        .add_numeric_field(n("TREEID")?, 9, 0)
        .add_numeric_field(n("Z")?, 10, 2)
        .add_numeric_field(n("AREA")?, 10, 2);
    let mut w = shapefile::Writer::from_path(path, table).map_err(fmt_err)?;
    for t in trees {
        let Some(h) = hulls.get(&t.id).filter(|h| h.len() >= 3) else {
            continue;
        };
        // shapefile outer rings run clockwise: the hull is counter-clockwise
        let mut pts: Vec<Point> = h.iter().rev().map(|p| Point::new(p[0], p[1])).collect();
        pts.push(pts[0]);
        let mut r = Record::default();
        r.insert("TREEID".into(), FieldValue::Numeric(Some(f64::from(t.id))));
        r.insert("Z".into(), FieldValue::Numeric(Some(f64::from(t.height))));
        r.insert("AREA".into(), FieldValue::Numeric(Some(polygon_area(h))));
        w.write_shape_and_record(&Polygon::new(PolygonRing::Outer(pts)), &r)
            .map_err(fmt_err)?;
    }
    Ok(())
}

/// Copies `input` to `output` with a `treeID` extra attribute: the id of the crown under each
/// point (0 outside every crown); how many points fell in a crown.
pub fn write_tree_ids(input: &Path, output: &Path, chm: &Grid, ids: &[u32]) -> Result<u64> {
    let r = Reader::open(input)?;
    let opts = WriterOptions {
        extra: vec![ExtraAttr {
            name: "treeID".into(),
            description: "tree id".into(),
            kind: ExtraKind::U32,
        }],
        ..WriterOptions::default()
    };
    let mut w = Writer::create_with(output, &r, &opts)?;
    let rec_len = usize::from(r.header.record_len);
    let mut inside = 0u64;
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let id = r
                .decode(rec)
                .and_then(|p| chm.index(p.x, p.y))
                .and_then(|i| ids.get(i).copied())
                .unwrap_or(0);
            inside += u64::from(id != 0);
            w.write_with_extra(rec, &id.to_le_bytes())?;
        }
        Ok(())
    })?;
    w.finish()?;
    Ok(inside)
}

/// A field tree.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldTree {
    pub x: f64,
    pub y: f64,
    pub height: f64,
    pub plot: String,
}

/// Where the columns of a field inventory are.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldColumns {
    pub x: String,
    pub y: String,
    pub height: String,
    pub plot: String,
}

impl Default for FieldColumns {
    fn default() -> Self {
        Self {
            x: "X_arbol".into(),
            y: "Y_arbol".into(),
            height: "Ht".into(),
            plot: "Parcela".into(),
        }
    }
}

/// Reads a delimited text file (tab, `;` or `,`; decimal commas allowed when the delimiter is
/// not a comma) of field trees; rows without coordinates are skipped.
pub fn read_field(path: &Path, cols: &FieldColumns) -> Result<Vec<FieldTree>> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8(bytes.clone())
        .unwrap_or_else(|_| bytes.iter().map(|&b| char::from(b)).collect());
    let mut lines = text.lines();
    let head = lines
        .next()
        .ok_or_else(|| Error::Format("empty field file".into()))?;
    let delim = [('\t', 0), (';', 0), (',', 0)]
        .iter()
        .map(|(d, _)| (*d, head.matches(*d).count()))
        .max_by_key(|(_, n)| *n)
        .map_or('\t', |(d, _)| d);
    let names: Vec<String> = head
        .split(delim)
        .map(|s| s.trim().trim_start_matches('\u{feff}').to_lowercase())
        .collect();
    let at = |n: &str| names.iter().position(|c| *c == n.to_lowercase());
    let (Some(ix), Some(iy)) = (at(&cols.x), at(&cols.y)) else {
        return Err(Error::Param(format!(
            "columns '{}' and '{}' not found in {}",
            cols.x,
            cols.y,
            path.display()
        )));
    };
    let (ih, ip) = (at(&cols.height), at(&cols.plot));
    let num = |s: &str| s.trim().replace(',', ".").parse::<f64>().ok();
    let mut out = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split(delim).collect();
        let (Some(x), Some(y)) = (
            f.get(ix).and_then(|s| num(s)),
            f.get(iy).and_then(|s| num(s)),
        ) else {
            continue;
        };
        out.push(FieldTree {
            x,
            y,
            height: ih
                .and_then(|i| f.get(i))
                .and_then(|s| num(s))
                .unwrap_or(f64::NAN),
            plot: ip
                .and_then(|i| f.get(i))
                .map_or(String::new(), |s| s.trim().to_string()),
        });
    }
    Ok(out)
}

/// Detections against field trees.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Evaluation {
    pub field: usize,
    pub detected: usize,
    pub matched: usize,
    pub false_positives: usize,
    pub missed: usize,
    pub recall: f64,
    pub precision: f64,
    pub f_score: f64,
    pub height_rmse: f64,
    pub height_bias: f64,
}

/// Matches each field tree to the nearest unused detection within `max_dist` (horizontal),
/// best pairs first; detections count only inside the circles `(x, y, radius)` of `plots`
/// when any are given. Height error is detected minus field.
pub fn evaluate(
    trees: &[Tree],
    field: &[FieldTree],
    plots: &[(f64, f64, f64)],
    max_dist: f64,
) -> Evaluation {
    let inside =
        |x: f64, y: f64| plots.is_empty() || plots.iter().any(|p| (x - p.0).hypot(y - p.1) <= p.2);
    let det: Vec<&Tree> = trees.iter().filter(|t| inside(t.x, t.y)).collect();
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (fi, f) in field.iter().enumerate() {
        for (di, d) in det.iter().enumerate() {
            let dist = (d.x - f.x).hypot(d.y - f.y);
            if dist <= max_dist {
                pairs.push((dist, fi, di));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut used_f, mut used_d) = (vec![false; field.len()], vec![false; det.len()]);
    let (mut n, mut sse, mut bias) = (0usize, 0.0f64, 0.0f64);
    for (_, fi, di) in pairs {
        if used_f[fi] || used_d[di] {
            continue;
        }
        used_f[fi] = true;
        used_d[di] = true;
        n += 1;
        let dh = f64::from(det[di].height) - field[fi].height;
        if dh.is_finite() {
            sse += dh * dh;
            bias += dh;
        }
    }
    let (fp, fnn) = (det.len() - n, field.len() - n);
    let div = |a: f64, b: f64| if b > 0.0 { a / b } else { 0.0 };
    let (recall, precision) = (
        div(n as f64, field.len() as f64),
        div(n as f64, det.len() as f64),
    );
    Evaluation {
        field: field.len(),
        detected: det.len(),
        matched: n,
        false_positives: fp,
        missed: fnn,
        recall,
        precision,
        f_score: div(2.0 * recall * precision, recall + precision),
        height_rmse: div(sse, n as f64).sqrt(),
        height_bias: div(bias, n as f64),
    }
}

/// Plot circles from the field file: one per plot, centred on the columns `px`, `py`.
pub fn plot_circles(
    path: &Path,
    plot: &str,
    px: &str,
    py: &str,
    radius: f64,
) -> Result<Vec<(f64, f64, f64)>> {
    let f = read_field(
        path,
        &FieldColumns {
            x: px.into(),
            y: py.into(),
            height: String::new(),
            plot: plot.into(),
        },
    )?;
    let mut seen: Vec<(String, f64, f64)> = Vec::new();
    for t in f {
        if !seen.iter().any(|s| s.0 == t.plot) {
            seen.push((t.plot, t.x, t.y));
        }
    }
    Ok(seen.into_iter().map(|s| (s.1, s.2, radius)).collect())
}

pub fn evaluation_text(e: &Evaluation) -> String {
    format!(
        "Evaluación contra el inventario: {} árboles de campo, {} detectados en las parcelas\n  \
         acertados {}  falsos positivos {}  omitidos {}\n  \
         exhaustividad {:.1} %  precisión {:.1} %  F {:.3}\n  \
         altura: sesgo {:+.2} m  RMSE {:.2} m",
        e.field,
        e.detected,
        e.matched,
        e.false_positives,
        e.missed,
        e.recall * 100.0,
        e.precision * 100.0,
        e.f_score,
        e.height_bias,
        e.height_rmse
    )
}

/// Output path next to `base` with `suffix` (`a.csv` + `crowns.shp` → `a_crowns.shp`).
pub fn sibling(base: &Path, suffix: &str) -> PathBuf {
    let stem = base
        .file_stem()
        .map_or(String::new(), |s| s.to_string_lossy().into_owned());
    base.with_file_name(format!("{stem}_{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_of_a_square_with_inner_points() {
        let h = convex_hull(vec![
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 4.0],
            [0.0, 4.0],
            [2.0, 2.0],
            [1.0, 3.0],
        ]);
        assert_eq!(h.len(), 4);
        assert!((polygon_area(&h) - 16.0).abs() < 1e-9);
    }

    #[test]
    fn evaluation_counts_matches_and_misses() {
        let t = |id, x, y, h| Tree {
            id,
            x,
            y,
            height: h,
            area: 0.0,
        };
        let trees = [
            t(1, 0.0, 0.0, 10.0),
            t(2, 10.0, 0.0, 8.0),
            t(3, 50.0, 50.0, 5.0),
        ];
        let field = [
            FieldTree {
                x: 0.5,
                y: 0.0,
                height: 9.0,
                plot: "1".into(),
            },
            FieldTree {
                x: 30.0,
                y: 0.0,
                height: 7.0,
                plot: "1".into(),
            },
        ];
        let e = evaluate(&trees, &field, &[(0.0, 0.0, 15.0)], 3.0);
        assert_eq!(
            (e.detected, e.matched, e.false_positives, e.missed),
            (2, 1, 1, 1)
        );
        assert!((e.height_bias - 1.0).abs() < 1e-9);
    }
}
