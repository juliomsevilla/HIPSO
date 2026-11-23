//! Exactitud vertical de un modelo digital del terreno frente a puntos de control del suelo.
//!
//! El error de cada punto es `MDT - referencia`. Además del sesgo, el error absoluto medio (MAE) y
//! el error cuadrático medio (RMSE) se dan los estadísticos robustos que recomiendan Höhle y Höhle
//! (2009) para modelos de elevaciones, cuyos errores rara vez son normales: la mediana, la
//! desviación absoluta mediana normalizada (NMAD = 1,4826 · mediana(|e − mediana(e)|)) y el
//! percentil 95 del error absoluto (LE95, el error lineal al 95 %).
//!
//! Los puntos de referencia pueden ser los de suelo de una nube LAS/LAZ (clase 2 por defecto), una
//! capa de puntos con un campo de cota o una tabla con columnas X, Y y Z.

use std::path::PathBuf;

use crate::grid::Grid;
use crate::las::Reader;
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result, paths};

/// Cómo se toma el valor del MDT bajo cada punto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampling {
    /// Interpolación bilineal entre los cuatro centros de celda más cercanos.
    Bilinear,
    /// El valor de la celda que contiene el punto.
    Nearest,
}

pub struct DtmCheckJob {
    pub dtm: PathBuf,
    /// Nube LAS/LAZ, capa de puntos o tabla con las cotas de referencia.
    pub reference: PathBuf,
    pub sampling: Sampling,
    /// Clases de la nube que sirven de referencia.
    pub class: u8,
    /// Campo con la cota (capas y tablas).
    pub z_field: Option<String>,
    /// Columnas de coordenadas de una tabla sin geometría.
    pub x_field: Option<String>,
    pub y_field: Option<String>,
    /// Se descartan los errores mayores que esto en valor absoluto (puntos de referencia mal
    /// puestos, vegetación en la clase 2…); `None`: se conservan todos.
    pub max_error: Option<f64>,
    /// Ruta base de los resultados.
    pub output: PathBuf,
    /// Escribe los errores punto a punto como tabla CSV y como capa de puntos.
    pub csv: bool,
    pub shp: bool,
}

impl DtmCheckJob {
    pub fn new(dtm: PathBuf, reference: PathBuf, output: PathBuf) -> Self {
        Self {
            dtm,
            reference,
            sampling: Sampling::Bilinear,
            class: 2,
            z_field: None,
            x_field: None,
            y_field: None,
            max_error: None,
            output,
            csv: true,
            shp: false,
        }
    }
}

/// Lo que se mide del error.
#[derive(Clone, Debug, PartialEq)]
pub struct Accuracy {
    pub n: usize,
    /// Puntos fuera del MDT o sobre celdas sin dato.
    pub outside: usize,
    /// Puntos descartados por `max_error`.
    pub rejected: usize,
    pub bias: f64,
    pub median: f64,
    pub mae: f64,
    pub rmse: f64,
    pub sd: f64,
    pub nmad: f64,
    pub le95: f64,
    pub min: f64,
    pub max: f64,
    /// Cuadrado del coeficiente de correlación entre la referencia y el MDT.
    pub r2: f64,
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

/// Los estadísticos de los errores `e` (MDT - referencia); `refs` son las cotas de referencia, con
/// el mismo orden, para el R².
pub fn accuracy(e: &[f64], refs: &[f64]) -> Accuracy {
    let n = e.len();
    let nan = f64::NAN;
    if n == 0 {
        return Accuracy {
            n: 0,
            outside: 0,
            rejected: 0,
            bias: nan,
            median: nan,
            mae: nan,
            rmse: nan,
            sd: nan,
            nmad: nan,
            le95: nan,
            min: nan,
            max: nan,
            r2: nan,
        };
    }
    let nf = n as f64;
    let bias = e.iter().sum::<f64>() / nf;
    let mut sorted = e.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = percentile(&sorted, 0.5);
    let mut dev: Vec<f64> = e.iter().map(|v| (v - median).abs()).collect();
    dev.sort_by(f64::total_cmp);
    let nmad = 1.4826 * percentile(&dev, 0.5);
    let mut abs: Vec<f64> = e.iter().map(|v| v.abs()).collect();
    abs.sort_by(f64::total_cmp);
    let mae = abs.iter().sum::<f64>() / nf;
    let rmse = (e.iter().map(|v| v * v).sum::<f64>() / nf).sqrt();
    let sd = if n > 1 {
        (e.iter().map(|v| (v - bias).powi(2)).sum::<f64>() / (nf - 1.0)).sqrt()
    } else {
        0.0
    };
    // R²: correlación de Pearson entre la referencia y el MDT (= referencia + error)
    let mean_ref = refs.iter().sum::<f64>() / nf;
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for (r, err) in refs.iter().zip(e) {
        let (dx, dy) = (r - mean_ref, r + err - (mean_ref + bias));
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let r2 = if sxx > 0.0 && syy > 0.0 {
        sxy * sxy / (sxx * syy)
    } else {
        nan
    };
    Accuracy {
        n,
        outside: 0,
        rejected: 0,
        bias,
        median,
        mae,
        rmse,
        sd,
        nmad,
        le95: percentile(&abs, 0.95),
        min: sorted[0],
        max: sorted[n - 1],
        r2,
    }
}

/// Los puntos de referencia `(x, y, z)` de `path`.
fn read_reference(job: &DtmCheckJob) -> Result<Vec<[f64; 3]>> {
    let ext = job
        .reference
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut out = Vec::new();
    if ext == "las" || ext == "laz" {
        let r = Reader::open(&job.reference)?;
        r.scan(|p| {
            if p.class == job.class && !p.is_noise() {
                out.push([p.x, p.y, p.z]);
            }
        })?;
        if out.is_empty() {
            return Err(Error::Param(format!(
                "la nube no tiene puntos de la clase {}",
                job.class
            )));
        }
        return Ok(out);
    }
    let d = if matches!(ext.as_str(), "csv" | "txt" | "tsv" | "xlsx") {
        crate::view::read_table(&job.reference)?
    } else {
        crate::view::read_vector(&job.reference)?
    };
    let find = |name: &str| -> Result<usize> {
        d.fields
            .iter()
            .position(|f| f.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                Error::Param(format!(
                    "no hay un campo «{name}» (hay: {})",
                    d.fields.join(", ")
                ))
            })
    };
    let z = match &job.z_field {
        Some(n) => find(n)?,
        None => ["z", "cota", "elev", "elevacion", "elevación", "altura"]
            .iter()
            .find_map(|n| find(n).ok())
            .ok_or_else(|| Error::Param("indica el campo de la cota con --z-field".into()))?,
    };
    let from_table = !d.points
        || d.features
            .iter()
            .all(|f| f.parts.iter().flatten().all(|p| *p == [0.0, 0.0]));
    let (xc, yc) = if from_table {
        let pick = |given: &Option<String>, names: &[&str]| -> Result<usize> {
            match given {
                Some(n) => find(n),
                None => names.iter().find_map(|n| find(n).ok()).ok_or_else(|| {
                    Error::Param(
                        "indica las columnas de las coordenadas con --x-field y --y-field".into(),
                    )
                }),
            }
        };
        (
            pick(&job.x_field, &["x", "este", "easting"])?,
            pick(&job.y_field, &["y", "norte", "northing"])?,
        )
    } else {
        (usize::MAX, usize::MAX)
    };
    for (k, f) in d.features.iter().enumerate() {
        let zv = d
            .num
            .get(k)
            .and_then(|r| r.get(z))
            .copied()
            .unwrap_or(f64::NAN);
        let (x, y) = if from_table {
            (
                d.num
                    .get(k)
                    .and_then(|r| r.get(xc))
                    .copied()
                    .unwrap_or(f64::NAN),
                d.num
                    .get(k)
                    .and_then(|r| r.get(yc))
                    .copied()
                    .unwrap_or(f64::NAN),
            )
        } else {
            match f.parts.first().and_then(|p| p.first()) {
                Some(p) => (p[0], p[1]),
                None => continue,
            }
        };
        if x.is_finite() && y.is_finite() && zv.is_finite() {
            out.push([x, y, zv]);
        }
    }
    if out.is_empty() {
        return Err(Error::Param(
            "no hay puntos de referencia con coordenadas y cota".into(),
        ));
    }
    Ok(out)
}

/// Los errores de cada punto de referencia sobre `dtm`: `(x, y, referencia, MDT)`.
pub fn residuals(dtm: &Grid, pts: &[[f64; 3]], s: Sampling) -> (Vec<[f64; 4]>, usize) {
    let mut out = Vec::with_capacity(pts.len());
    let mut outside = 0;
    for p in pts {
        let v = match s {
            Sampling::Bilinear => dtm.sample(p[0], p[1]),
            Sampling::Nearest => dtm
                .index(p[0], p[1])
                .and_then(|i| dtm.data.get(i).copied())
                .filter(|v| !v.is_nan()),
        };
        match v {
            Some(v) => out.push([p[0], p[1], p[2], f64::from(v)]),
            None => outside += 1,
        }
    }
    (out, outside)
}

fn report_text(a: &Accuracy) -> String {
    format!(
        "{} puntos de control ({} fuera del MDT o sin dato, {} descartados por error excesivo): sesgo {:.3}, mediana {:.3}, MAE {:.3}, RMSE {:.3}, desviación {:.3}, NMAD {:.3}, LE95 {:.3}, mínimo {:.3}, máximo {:.3}, R² {:.4}",
        a.n,
        a.outside,
        a.rejected,
        a.bias,
        a.median,
        a.mae,
        a.rmse,
        a.sd,
        a.nmad,
        a.le95,
        a.min,
        a.max,
        a.r2
    )
}

/// Mide el MDT frente a la referencia y escribe lo pedido; devuelve el resumen.
pub fn run(job: &DtmCheckJob) -> Result<String> {
    let dtm = Grid::read(&job.dtm)?;
    let epsg = Grid::read_epsg(&job.dtm);
    let pts = read_reference(job)?;
    let (mut res, outside) = residuals(&dtm, &pts, job.sampling);
    let before = res.len();
    if let Some(m) = job.max_error {
        res.retain(|r| (r[3] - r[2]).abs() <= m);
    }
    let rejected = before - res.len();
    let e: Vec<f64> = res.iter().map(|r| r[3] - r[2]).collect();
    let refs: Vec<f64> = res.iter().map(|r| r[2]).collect();
    let mut a = accuracy(&e, &refs);
    a.outside = outside;
    a.rejected = rejected;
    if a.n == 0 {
        return Err(Error::Param(
            "ningún punto de control cae sobre el MDT: comprueba que comparten sistema de coordenadas".into(),
        ));
    }
    let mut lines = vec![report_text(&a)];
    paths::ensure_parent(&job.output)?;
    if job.csv {
        let p = paths::swap_ext(&job.output, "csv");
        let mut text = String::from("X,Y,Z_referencia,Z_MDT,Error\n");
        for r in &res {
            text.push_str(&format!(
                "{:.3},{:.3},{:.3},{:.3},{:.3}\n",
                r[0],
                r[1],
                r[2],
                r[3],
                r[3] - r[2]
            ));
        }
        std::fs::write(&p, text)?;
        let s = p.with_file_name(format!("{}_resumen.csv", paths::stem(&p)));
        std::fs::write(
            &s,
            format!(
                "N,Fuera,Descartados,Sesgo,Mediana,MAE,RMSE,Desviacion,NMAD,LE95,Minimo,Maximo,R2\n{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.5}\n",
                a.n,
                a.outside,
                a.rejected,
                a.bias,
                a.median,
                a.mae,
                a.rmse,
                a.sd,
                a.nmad,
                a.le95,
                a.min,
                a.max,
                a.r2
            ),
        )?;
        lines.push(format!("  errores punto a punto: {}", p.display()));
        lines.push(format!("  resumen: {}", s.display()));
    }
    if job.shp {
        let mut d = VectorData {
            points: true,
            fields: ["z_ref", "z_mdt", "error"].map(String::from).to_vec(),
            bounds: [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            ..Default::default()
        };
        for r in &res {
            d.bounds = [
                d.bounds[0].min(r[0]),
                d.bounds[1].min(r[1]),
                d.bounds[2].max(r[0]),
                d.bounds[3].max(r[1]),
            ];
            let v = vec![
                format!("{:.3}", r[2]),
                format!("{:.3}", r[3]),
                format!("{:.3}", r[3] - r[2]),
            ];
            d.num.push(vec![r[2], r[3], r[3] - r[2]]);
            d.text.push(v);
            d.features.push(Feature {
                parts: vec![vec![[r[0], r[1]]]],
            });
        }
        let p = paths::swap_ext(&job.output, "shp");
        let prj = epsg.and_then(crate::vector::prj_for_epsg);
        crate::vector::write_layer(&p, &d, GeomKind::Point, prj.as_deref())?;
        lines.push(format!("  errores (puntos): {}", p.display()));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_of_a_known_error_set() {
        let e = [-1.0, 0.0, 1.0, 2.0, -2.0];
        let r = [10.0, 11.0, 12.0, 13.0, 14.0];
        let a = accuracy(&e, &r);
        assert_eq!(a.n, 5);
        assert!(a.bias.abs() < 1e-12);
        assert!((a.mae - 1.2).abs() < 1e-12);
        assert!((a.rmse - 2.0f64.sqrt()).abs() < 1e-12);
        assert!(a.median.abs() < 1e-12);
        // MAD de |e - 0| = [1,0,1,2,2] -> mediana 1
        assert!((a.nmad - 1.4826).abs() < 1e-12);
        assert!((a.le95 - 1.9 - 0.0).abs() < 0.11);
    }

    #[test]
    fn perfect_surface_has_r2_one_and_zero_error() {
        let r = [1.0, 2.0, 3.0, 4.0];
        let a = accuracy(&[0.0; 4], &r);
        assert_eq!(a.rmse, 0.0);
        assert!((a.r2 - 1.0).abs() < 1e-12);
    }

    #[test]
    fn nearest_and_bilinear_read_the_surface() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 2, 2)?;
        g.data = vec![0.0, 2.0, 0.0, 2.0];
        let pts = [[1.0, 1.0, 1.0]];
        let (b, _) = residuals(&g, &pts, Sampling::Bilinear);
        assert!((b[0][3] - 1.0).abs() < 1e-6);
        let (n, o) = residuals(&g, &[[5.0, 5.0, 0.0]], Sampling::Nearest);
        assert!(n.is_empty() && o == 1);
        Ok(())
    }
}
