//! Estadística zonal: los valores de un ráster dentro de cada polígono (un rodal, una unidad de
//! gestión, una parcela), como recuento, mínimo, máximo, media, suma, desviación típica y mediana.
//!
//! Una celda pertenece a una región cuando su centro está dentro de ella (como es habitual en las
//! estadísticas ráster); las celdas vacías se omiten. Las regiones son independientes, así que se
//! ejecutan en paralelo. La desviación típica es la de población (dividida entre el recuento), la
//! mediana de un recuento par es la media de los dos valores centrales. El resultado es una capa:
//! los polígonos con sus atributos y un campo nuevo por estadístico ([`zonal_layer`]).

use rayon::prelude::*;

use crate::grid::Grid;
use crate::vector::Region;
use crate::{Error, Result};

/// Los estadísticos de una región.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub name: String,
    pub count: u64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub sum: f64,
    pub stddev: f64,
    pub median: f64,
}

impl Zone {
    fn empty(name: &str) -> Self {
        Self {
            name: name.to_string(),
            count: 0,
            min: f64::NAN,
            max: f64::NAN,
            mean: f64::NAN,
            sum: 0.0,
            stddev: f64::NAN,
            median: f64::NAN,
        }
    }
}

/// Los estadísticos de `g` dentro de cada región, en el orden de `regions`. Las regiones y el
/// ráster deben estar en el mismo sistema de coordenadas.
pub fn zonal_stats(g: &Grid, regions: &[Region]) -> Result<Vec<Zone>> {
    if !(g.cell > 0.0) {
        return Err(Error::Param("la celda del ráster no es válida".into()));
    }
    Ok(regions.par_iter().map(|r| one(g, r)).collect())
}

fn one(g: &Grid, r: &Region) -> Zone {
    let b = r.bbox();
    let top = g.y0 + g.rows as f64 * g.cell;
    // celdas cuyos centros pueden caer en la caja
    let c0 = (((b[0] - g.x0) / g.cell - 0.5).ceil().max(0.0)) as usize;
    let c1 = ((((b[2] - g.x0) / g.cell - 0.5).floor()).max(-1.0) + 1.0) as usize;
    let r0 = (((top - b[3]) / g.cell - 0.5).ceil().max(0.0)) as usize;
    let r1 = ((((top - b[1]) / g.cell - 0.5).floor()).max(-1.0) + 1.0) as usize;
    let (c1, r1) = (c1.min(g.cols), r1.min(g.rows));
    let mut values: Vec<f64> = Vec::new();
    for row in r0..r1 {
        for col in c0..c1 {
            let v = g.data[row * g.cols + col];
            if v.is_nan() {
                continue;
            }
            let (x, y) = g.center(col, row);
            if r.contains(x, y) {
                values.push(f64::from(v));
            }
        }
    }
    if values.is_empty() {
        return Zone::empty(&r.name);
    }
    let n = values.len() as f64;
    let sum: f64 = values.iter().sum();
    let mean = sum / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in &values {
        min = min.min(*v);
        max = max.max(*v);
    }
    let mid = values.len() / 2;
    values.select_nth_unstable_by(mid, f64::total_cmp);
    let median = if values.len() % 2 == 1 {
        values[mid]
    } else {
        let lo = values[..mid].iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (lo + values[mid]) / 2.0
    };
    Zone {
        name: r.name.clone(),
        count: values.len() as u64,
        min,
        max,
        mean,
        sum,
        stddev: var.sqrt(),
        median,
    }
}

/// Los nombres de los campos de estadísticos, en orden, con un prefijo opcional (`chm_` da
/// `chm_mean`). Un nombre de campo de shapefile tiene como mucho 10 caracteres.
pub const FIELDS: [&str; 7] = ["count", "min", "max", "mean", "sum", "stddev", "median"];

/// Los polígonos como una capa que conserva sus atributos y gana un campo por estadístico de `g`
/// dentro de cada uno. Las regiones sin celdas reciben valores vacíos y un recuento de 0.
pub fn zonal_layer(g: &Grid, regions: &[Region], prefix: &str) -> Result<crate::geomops::PolyLayer> {
    use crate::geomops::{Feature, PolyLayer, multipolygon_of, rings_of};
    let zones = zonal_stats(g, regions)?;
    let num = |v: f64| if v.is_nan() { String::new() } else { format!("{v}") };
    let features = regions
        .iter()
        .zip(&zones)
        .map(|(r, z)| {
            let mut attrs = r.attrs.clone();
            let vals = [z.count.to_string(), num(z.min), num(z.max), num(z.mean), num(z.sum), num(z.stddev), num(z.median)];
            for (f, v) in FIELDS.iter().zip(vals) {
                let mut name = format!("{prefix}{f}");
                // nunca sobrescribir un campo que ya tiene la capa
                while attrs.iter().any(|(k, _)| k.eq_ignore_ascii_case(&name)) {
                    name = format!("z_{name}");
                }
                attrs.push((name, v));
            }
            Feature { geom: multipolygon_of(&rings_of(r)), attrs }
        })
        .collect();
    Ok(PolyLayer { features, prj: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::{Polygon, Shape2};

    fn poly(name: &str, x0: f64, y0: f64, x1: f64, y1: f64) -> Region {
        Region {
            name: name.into(),
            shape: Shape2::Polygon(Polygon::new(vec![vec![[x0, y0], [x0, y1], [x1, y1], [x1, y0]]])),
            attrs: vec![],
        }
    }

    #[test]
    fn a_constant_raster_gives_exact_statistics() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 20, 20)?;
        g.data.fill(7.5);
        let z = zonal_stats(&g, &[poly("a", 2.0, 3.0, 12.0, 9.0)])?;
        let z = &z[0];
        // 10 x 6 celdas, con los centros dentro
        assert_eq!(z.count, 60);
        assert_eq!((z.min, z.max, z.mean, z.median), (7.5, 7.5, 7.5, 7.5));
        assert_eq!(z.sum, 450.0);
        assert_eq!(z.stddev, 0.0);
        Ok(())
    }

    #[test]
    fn statistics_of_known_values_and_empty_cells_are_skipped() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 4, 1)?;
        g.data.copy_from_slice(&[1.0, 2.0, f32::NAN, 9.0]);
        let z = &zonal_stats(&g, &[poly("r", 0.0, 0.0, 4.0, 1.0)])?[0];
        assert_eq!(z.count, 3);
        assert_eq!((z.min, z.max, z.sum, z.median), (1.0, 9.0, 12.0, 2.0));
        assert_eq!(z.mean, 4.0);
        assert!((z.stddev - ((9.0 + 4.0 + 25.0) / 3.0f64).sqrt()).abs() < 1e-12);
        // un recuento par: la mediana es la media de los dos valores centrales
        g.data.copy_from_slice(&[1.0, 2.0, 4.0, 9.0]);
        let z = &zonal_stats(&g, &[poly("r", 0.0, 0.0, 4.0, 1.0)])?[0];
        assert_eq!(z.median, 3.0);
        // una región fuera del ráster
        let z = &zonal_stats(&g, &[poly("fuera", 50.0, 50.0, 60.0, 60.0)])?[0];
        assert_eq!(z.count, 0);
        assert!(z.mean.is_nan());
        Ok(())
    }
}

#[cfg(test)]
mod layer_tests {
    use super::*;
    use crate::vector::{Polygon, Shape2};

    #[test]
    fn the_layer_keeps_attributes_and_adds_the_statistics() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 10, 10)?;
        g.data.fill(4.0);
        let sq = |x: f64, name: &str| Region {
            name: name.into(),
            shape: Shape2::Polygon(Polygon::new(vec![vec![[x, 0.0], [x, 5.0], [x + 5.0, 5.0], [x + 5.0, 0.0]]])),
            attrs: vec![("nombre".into(), name.into()), ("mean".into(), "viejo".into())],
        };
        let l = zonal_layer(&g, &[sq(0.0, "A"), sq(50.0, "B")], "")?;
        let a = &l.features[0].attrs;
        assert_eq!(a[0], ("nombre".to_string(), "A".to_string()));
        // un campo existente con el nombre de un estadístico no se sobrescribe
        assert_eq!(a[1], ("mean".to_string(), "viejo".to_string()));
        let get = |k: &str| a.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("count").unwrap(), "25");
        assert_eq!(get("z_mean").unwrap(), "4");
        assert_eq!(get("sum").unwrap(), "100");
        // fuera del ráster: recuento 0 y valores vacíos
        let b = &l.features[1].attrs;
        assert_eq!(b.iter().find(|(n, _)| n == "count").unwrap().1, "0");
        assert_eq!(b.iter().find(|(n, _)| n == "max").unwrap().1, "");
        let g = crate::geomops::table_of(&l.features);
        assert!(g.columns.iter().any(|(n, _)| n == "stddev"));
        Ok(())
    }
}
