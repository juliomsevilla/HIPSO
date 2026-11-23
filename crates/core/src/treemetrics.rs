//! Métricas de cada árbol a partir de los puntos que ya llevan su número de árbol (`treeID`), el
//! que escriben `trees --cloud` y `trees3d`: altura, base de la copa, longitud y proporción de
//! copa, superficie y radio de la copa, y el volumen de la copa como envolvente convexa y como forma
//! alfa tridimensional (véase [`crate::alpha3d`]).
//!
//! La base de la copa es una definición sencilla y propia: al bajar desde la cima, la copa acaba
//! donde el salto vertical entre dos puntos consecutivos del árbol supera `gap`; los puntos por
//! debajo de `crown_min` (el tronco bajo, el suelo) no cuentan. Con nubes dispersas, un `gap` mayor
//! evita cortar la copa en un hueco entre ramas. La copa son los puntos desde esa base hasta la
//! cima.
//!
//! Los puntos de todos los árboles se guardan en memoria mientras se leen (24 bytes por punto de
//! árbol): para una zona grande, divídela en hojas.

use std::collections::HashMap;
use std::path::PathBuf;

use rayon::prelude::*;

use crate::alpha3d;
use crate::filter::Filter;
use crate::forest::{convex_hull, polygon_area};
use crate::grid::Grid;
use crate::las::Reader;
use crate::{Error, Result, paths};

#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// Salto vertical (m) que cierra la copa por abajo.
    pub gap: f64,
    /// Los puntos por debajo de esta altura (m) no cuentan para la copa.
    pub crown_min: f64,
    /// Radio alfa (m) de la forma alfa.
    pub alpha: f64,
    /// Los árboles con menos puntos no se miden.
    pub min_points: usize,
    /// Puntos de copa como mucho que se triangulan por árbol.
    pub max_triangulated: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            gap: 2.0,
            crown_min: 1.3,
            alpha: 2.0,
            min_points: 20,
            max_triangulated: 2500,
        }
    }
}

/// Lo que se mide de un árbol.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeMetrics {
    pub id: u32,
    /// Posición y elevación de la cima.
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Altura de la cima sobre el suelo.
    pub height: f64,
    pub points: usize,
    pub crown_points: usize,
    /// Altura de la base de la copa.
    pub cbh: f64,
    pub crown_length: f64,
    /// Longitud de la copa / altura.
    pub crown_ratio: f64,
    /// Superficie (m²) de la envolvente convexa de la copa en planta.
    pub crown_area: f64,
    /// Radio (m) del círculo de esa superficie.
    pub crown_radius: f64,
    pub volume_hull: f64,
    pub volume_alpha: f64,
    pub area_alpha: f64,
}

/// Mide un árbol: cada punto es `[x, y, z, altura sobre el suelo]`.
pub fn measure(id: u32, pts: &[[f64; 4]], p: &Params) -> Option<TreeMetrics> {
    let top = pts
        .iter()
        .max_by(|a, b| a[3].total_cmp(&b[3]).then(b[0].total_cmp(&a[0])))?;
    let height = top[3];
    if !(height > 0.0) {
        return None;
    }
    // alturas de la cima hacia abajo; los puntos bajos no cuentan
    let mut hs: Vec<f64> = pts
        .iter()
        .map(|q| q[3])
        .filter(|h| *h >= p.crown_min)
        .collect();
    hs.sort_by(|a, b| b.total_cmp(a));
    let mut cbh = hs.last().copied().unwrap_or(height);
    for w in hs.windows(2) {
        if w[0] - w[1] > p.gap {
            cbh = w[0];
            break;
        }
    }
    let crown: Vec<&[f64; 4]> = pts.iter().filter(|q| q[3] >= cbh).collect();
    let flat: Vec<[f64; 2]> = crown
        .iter()
        .map(|q| [q[0] - top[0], q[1] - top[1]])
        .collect();
    let area = polygon_area(&convex_hull(flat));
    let rel: Vec<[f64; 3]> = crown
        .iter()
        .map(|q| [q[0] - top[0], q[1] - top[1], q[3]])
        .collect();
    let s = alpha3d::shape(&rel, p.alpha, p.max_triangulated);
    Some(TreeMetrics {
        id,
        x: top[0],
        y: top[1],
        z: top[2],
        height,
        points: pts.len(),
        crown_points: crown.len(),
        cbh,
        crown_length: height - cbh,
        crown_ratio: (height - cbh) / height,
        crown_area: area,
        crown_radius: (area / std::f64::consts::PI).sqrt(),
        volume_hull: s.hull_volume,
        volume_alpha: s.alpha_volume,
        area_alpha: s.alpha_area,
    })
}

pub struct TreeMetricsJob {
    /// Nube con el atributo `treeID`.
    pub input: PathBuf,
    /// MDT para las alturas; sin él se busca el más reciente hecho con la nube y, si no hay, se
    /// exige `normalized`.
    pub dtm: Option<PathBuf>,
    /// La nube ya tiene alturas sobre el suelo.
    pub normalized: bool,
    pub params: Params,
    pub filter: Filter,
    pub output: PathBuf,
    pub shp: bool,
}

impl TreeMetricsJob {
    pub fn new(input: PathBuf, output: PathBuf) -> Self {
        Self {
            input,
            dtm: None,
            normalized: false,
            params: Params::default(),
            filter: Filter {
                skip_noise: true,
                ..Filter::default()
            },
            output,
            shp: true,
        }
    }
}

/// Mide todos los árboles de la nube.
pub fn measure_cloud(job: &TreeMetricsJob) -> Result<Vec<TreeMetrics>> {
    let p = &job.params;
    if !(p.gap > 0.0) || !(p.alpha > 0.0) || !(p.crown_min >= 0.0) {
        return Err(Error::Param(
            "el salto, el radio alfa y la altura mínima de copa deben ser positivos".into(),
        ));
    }
    let r = Reader::open(&job.input)?;
    let field = r.extra_field("treeID").ok_or_else(|| {
        Error::Param("la nube no tiene el atributo treeID: segméntala antes con `trees --cloud` o con `trees3d`".into())
    })?;
    let dtm = if job.normalized {
        None
    } else {
        let path = job.dtm.clone().or_else(|| paths::find_dtm(&job.input)).ok_or_else(|| {
            Error::Param("no hay MDT: indica --dtm, haz antes `dtm` o, si las alturas ya son sobre el suelo, --normalized".into())
        })?;
        Some(Grid::read(&path)?)
    };
    let rec_len = usize::from(r.header.record_len);
    let mut by_tree: HashMap<u32, Vec<[f64; 4]>> = HashMap::new();
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let id = field.id(rec);
            if id == 0 {
                continue;
            }
            let Some(q) = r.decode(rec) else { continue };
            if !job.filter.keeps(&q) {
                continue;
            }
            let h = match &dtm {
                Some(d) => match d.sample(q.x, q.y) {
                    Some(g) => q.z - f64::from(g),
                    None => continue,
                },
                None => q.z,
            };
            if h.is_finite() && h >= 0.0 && job.filter.z_ok(h) {
                by_tree.entry(id).or_default().push([q.x, q.y, q.z, h]);
            }
        }
        Ok(())
    })?;
    let mut ids: Vec<u32> = by_tree.keys().copied().collect();
    ids.sort_unstable();
    let out: Vec<TreeMetrics> = ids
        .par_iter()
        .filter_map(|id| {
            let pts = by_tree.get(id)?;
            if pts.len() < p.min_points {
                None
            } else {
                measure(*id, pts, p)
            }
        })
        .collect();
    Ok(out)
}

/// Mide los árboles y escribe la tabla y la capa de cimas; devuelve el resumen.
pub fn run(job: &TreeMetricsJob) -> Result<String> {
    let epsg = Reader::open(&job.input)?.header.epsg;
    let trees = measure_cloud(job)?;
    paths::ensure_parent(&job.output)?;
    let csv = paths::swap_ext(&job.output, "csv");
    let mut text = String::from(
        "treeID,X,Y,Z,H,Puntos,Puntos_copa,CBH,Long_copa,Prop_copa,Area_copa,Radio_copa,Vol_convexo,Vol_alfa,Sup_alfa\n",
    );
    for t in &trees {
        text.push_str(&format!(
            "{},{:.3},{:.3},{:.3},{:.3},{},{},{:.3},{:.3},{:.4},{:.3},{:.3},{:.3},{:.3},{:.3}\n",
            t.id,
            t.x,
            t.y,
            t.z,
            t.height,
            t.points,
            t.crown_points,
            t.cbh,
            t.crown_length,
            t.crown_ratio,
            t.crown_area,
            t.crown_radius,
            t.volume_hull,
            t.volume_alpha,
            t.area_alpha
        ));
    }
    std::fs::write(&csv, text)?;
    let n = trees.len().max(1) as f64;
    let mut lines = vec![format!(
        "{} árboles medidos (al menos {} puntos): altura media {:.2} m, base de copa media {:.2} m, proporción de copa media {:.2}, volumen de copa medio {:.1} m³ (alfa {} m) y {:.1} m³ (envolvente convexa)",
        trees.len(),
        job.params.min_points,
        trees.iter().map(|t| t.height).sum::<f64>() / n,
        trees.iter().map(|t| t.cbh).sum::<f64>() / n,
        trees.iter().map(|t| t.crown_ratio).sum::<f64>() / n,
        trees.iter().map(|t| t.volume_alpha).sum::<f64>() / n,
        job.params.alpha,
        trees.iter().map(|t| t.volume_hull).sum::<f64>() / n,
    )];
    lines.push(format!("  tabla: {}", csv.display()));
    if job.shp {
        use crate::view::{Feature, GeomKind, VectorData};
        let names = [
            "treeid",
            "z",
            "h",
            "puntos",
            "cbh",
            "long_copa",
            "prop_copa",
            "area_copa",
            "radio",
            "vol_conv",
            "vol_alfa",
            "sup_alfa",
        ];
        let mut d = VectorData {
            points: true,
            fields: names.map(String::from).to_vec(),
            bounds: [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            ..Default::default()
        };
        for t in &trees {
            d.bounds = [
                d.bounds[0].min(t.x),
                d.bounds[1].min(t.y),
                d.bounds[2].max(t.x),
                d.bounds[3].max(t.y),
            ];
            let v = [
                f64::from(t.id),
                t.z,
                t.height,
                t.points as f64,
                t.cbh,
                t.crown_length,
                t.crown_ratio,
                t.crown_area,
                t.crown_radius,
                t.volume_hull,
                t.volume_alpha,
                t.area_alpha,
            ];
            d.text.push(v.iter().map(|x| format!("{x:.3}")).collect());
            d.num.push(v.to_vec());
            d.features.push(Feature {
                parts: vec![vec![[t.x, t.y]]],
            });
        }
        let p = paths::swap_ext(&job.output, "shp");
        let prj = epsg.and_then(crate::vector::prj_for_epsg);
        crate::vector::write_layer(&p, &d, GeomKind::Point, prj.as_deref())?;
        lines.push(format!("  árboles (puntos): {}", p.display()));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un tronco de `trunk` m con puntos cada 0,5 m y, encima, una copa esférica de radio `r`.
    fn tree(trunk: f64, r: f64) -> Vec<[f64; 4]> {
        let mut v = Vec::new();
        let mut h = 0.5;
        while h < trunk {
            v.push([0.0, 0.0, 100.0 + h, h]);
            h += 0.5;
        }
        let golden = std::f64::consts::PI * (3.0 - 5.0f64.sqrt());
        let n = 400;
        for i in 0..n {
            let y = 1.0 - 2.0 * (f64::from(i) + 0.5) / f64::from(n);
            let rr = (1.0 - y * y).sqrt();
            let t = golden * f64::from(i);
            let (x, yy, z) = (r * rr * t.cos(), r * rr * t.sin(), trunk + r + r * y);
            v.push([x, yy, 100.0 + z, z]);
        }
        v
    }

    #[test]
    fn a_sphere_crown_has_its_volume_and_base() -> Result<()> {
        let pts = tree(6.0, 3.0);
        let m = measure(
            7,
            &pts,
            &Params {
                alpha: 100.0,
                ..Params::default()
            },
        );
        let m = m.ok_or_else(|| Error::Param("sin árbol".into()))?;
        assert_eq!(m.id, 7);
        assert!((m.height - 12.0).abs() < 0.1, "{}", m.height);
        // el tronco acaba en 5,5 y la copa empieza en 6: el salto de 0,5 no la corta, el de la
        // base de la esfera (6 frente a 5,5) tampoco; con el umbral por defecto la copa son todos
        assert!(m.cbh <= 6.0, "{}", m.cbh);
        let exact = 4.0 / 3.0 * std::f64::consts::PI * 27.0;
        assert!(
            m.volume_hull > 0.9 * exact * 0.9 && m.volume_hull < exact * 1.25,
            "{} {}",
            m.volume_hull,
            exact
        );
        assert!((m.crown_radius - 3.0).abs() < 0.4, "{}", m.crown_radius);
        Ok(())
    }

    #[test]
    fn a_vertical_gap_closes_the_crown() -> Result<()> {
        // tronco hasta 4 m, nada entre 4 y 8, copa de 8 a 14
        let mut pts: Vec<[f64; 4]> = (1..8)
            .map(|k| [0.0, 0.0, 100.0 + f64::from(k) * 0.5, f64::from(k) * 0.5])
            .collect();
        for k in 0..60 {
            let h = 8.0 + f64::from(k) * 0.1;
            let a = f64::from(k);
            pts.push([2.0 * a.cos(), 2.0 * a.sin(), 100.0 + h, h]);
        }
        let m = measure(1, &pts, &Params::default());
        let m = m.ok_or_else(|| Error::Param("sin árbol".into()))?;
        assert!((m.cbh - 8.0).abs() < 1e-9, "{}", m.cbh);
        assert!((m.crown_ratio - (m.height - 8.0) / m.height).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn no_height_no_tree() {
        assert!(measure(1, &[[0.0, 0.0, 5.0, 0.0]], &Params::default()).is_none());
    }
}
