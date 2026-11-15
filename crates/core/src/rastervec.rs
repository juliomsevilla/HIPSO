//! Ráster a polígonos: las celdas que comparten una clase, caen en un rango o en la misma banda
//! entre cortes pasan a ser polígonos cuyos límites siguen exactamente los bordes de las celdas (o
//! simplificados), con el valor como atributo.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::obia::segment::label_components;
use crate::obia::vectorize::Tracer;
use crate::obia::{Labels, Lattice};
use crate::paths;
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result};

/// Qué hace que las celdas vayan juntas.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// Valores iguales (redondeados a números enteros): una clasificación.
    Classes,
    /// Celdas con un valor de `lo` a `hi`, ambos incluidos; el resto se deja fuera.
    Range { lo: f64, hi: f64 },
    /// La banda entre cortes consecutivos (los de por debajo del primero y por encima del último
    /// cuentan también).
    Breaks(Vec<f64>),
}

pub struct VecJob {
    pub input: PathBuf,
    pub mode: Mode,
    /// Las partes de menos celdas que esto se descartan.
    pub min_cells: u32,
    /// Tolerancia de Douglas-Peucker en unidades del mapa; 0 conserva los escalones de las celdas.
    pub simplify: f64,
    /// Las celdas que se tocan por una esquina son una sola parte.
    pub diagonal: bool,
    /// La capa que escribir: `.shp` o `.gpkg`.
    pub output: PathBuf,
}

impl VecJob {
    pub fn new(input: PathBuf, output: PathBuf) -> Self {
        Self {
            input,
            mode: Mode::Classes,
            min_cells: 1,
            simplify: 0.0,
            diagonal: false,
            output,
        }
    }
}

/// El cubo de cada celda: `None` donde se deja fuera (vacía o fuera del rango).
fn buckets(g: &Grid, mode: &Mode) -> Vec<Option<i64>> {
    g.data
        .par_iter()
        .map(|&v| {
            if !v.is_finite() {
                return None;
            }
            let v = f64::from(v);
            match mode {
                Mode::Classes => Some(v.round() as i64),
                Mode::Range { lo, hi } => (v >= *lo && v <= *hi).then_some(1),
                Mode::Breaks(cuts) => Some(cuts.partition_point(|c| *c <= v) as i64),
            }
        })
        .collect()
}

/// La rejilla de etiquetas de las partes y, para cada parte, su cubo y su número de celdas.
fn parts(g: &Grid, job: &VecJob) -> Result<(Labels, Vec<(i64, u32)>)> {
    let bucket = buckets(g, &job.mode);
    let (mut ids, count) = label_components(
        g.cols,
        g.rows,
        job.diagonal,
        |i| bucket[i].is_some(),
        |i, j| bucket[i] == bucket[j],
    )?;
    let mut info = vec![(0i64, 0u32); count as usize + 1];
    for (i, &l) in ids.iter().enumerate() {
        if l != 0 {
            let e = &mut info[l as usize];
            e.0 = bucket[i].unwrap_or(0);
            e.1 += 1;
        }
    }
    // renumerar lo que se conserva, desde 1
    let mut new = vec![0u32; count as usize + 1];
    let mut kept = Vec::new();
    for l in 1..=count as usize {
        if info[l].1 >= job.min_cells.max(1) {
            kept.push(info[l]);
            new[l] = kept.len() as u32;
        }
    }
    for l in &mut ids {
        *l = new[*l as usize];
    }
    let labels = Labels {
        lat: Lattice::of(g),
        count: kept.len() as u32,
        ids,
    };
    Ok((labels, kept))
}

/// Los límites de la banda `k` entre `cuts`: NaN donde no tiene límite.
fn limits(cuts: &[f64], k: i64) -> (f64, f64) {
    let k = k.max(0) as usize;
    (
        if k == 0 { f64::NAN } else { cuts[k - 1] },
        cuts.get(k).copied().unwrap_or(f64::NAN),
    )
}

/// Los polígonos de `g` como capa.
pub fn vectorize(g: &Grid, job: &VecJob) -> Result<VectorData> {
    if let Mode::Breaks(c) = &job.mode
        && (c.is_empty() || c.windows(2).any(|w| !(w[0] < w[1])))
    {
        return Err(Error::Param(
            "los cortes deben ser números distintos, de menor a mayor".into(),
        ));
    }
    if let Mode::Range { lo, hi } = &job.mode
        && !(lo <= hi)
    {
        return Err(Error::Param(
            "el rango va de un mínimo a un máximo mayor o igual".into(),
        ));
    }
    let (labels, info) = parts(g, job)?;
    let tracer = Tracer::new(&labels)?;
    let outlines: Vec<_> = (1..=labels.count)
        .into_par_iter()
        .map(|id| tracer.outline(id, job.simplify))
        .collect();
    let breaks = matches!(job.mode, Mode::Breaks(_));
    let mut fields: Vec<String> = ["valor", "area_m2", "celdas"].map(String::from).to_vec();
    if breaks {
        fields.extend(["lim_inf", "lim_sup"].map(String::from));
    }
    let mut d = VectorData {
        fields,
        polygons: true,
        bounds: [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        ..Default::default()
    };
    let area = g.cell * g.cell;
    for ((bucket, cells), o) in info.iter().zip(outlines) {
        let mut parts = o.outers;
        parts.extend(o.holes);
        for q in parts.iter().flatten() {
            d.bounds = [
                d.bounds[0].min(q[0]),
                d.bounds[1].min(q[1]),
                d.bounds[2].max(q[0]),
                d.bounds[3].max(q[1]),
            ];
        }
        let mut values = vec![
            bucket.to_string(),
            format!("{:.4}", f64::from(*cells) * area),
            cells.to_string(),
        ];
        if let Mode::Breaks(c) = &job.mode {
            let (lo, hi) = limits(c, *bucket);
            let text = |v: f64| {
                if v.is_nan() {
                    String::new()
                } else {
                    v.to_string()
                }
            };
            values.push(text(lo));
            values.push(text(hi));
        }
        d.num.push(
            values
                .iter()
                .map(|t| t.parse().unwrap_or(f64::NAN))
                .collect(),
        );
        d.text.push(values);
        d.features.push(Feature { parts });
    }
    Ok(d)
}

/// Vectoriza `job.input` en `job.output`; devuelve el resumen.
pub fn run(job: &VecJob) -> Result<String> {
    if let Ok(pyr) = crate::pyramid::Pyramid::open(&job.input)
        && let Some(mem) = crate::copc_cache::total_memory()
    {
        let e = pyr.extent();
        let cells = ((e[2] - e[0]) / pyr.cell) * ((e[3] - e[1]) / pyr.cell);
        // el ráster, los cubos (16 bytes con la opción) y las etiquetas
        let need = cells * 28.0;
        if need > mem as f64 * 0.7 {
            return Err(Error::Param(format!(
                "vectorizar {:.0} millones de celdas necesita unos {:.0} GB de memoria (este equipo tiene {:.0} GB): recorta el ráster con un polígono o tamízalo antes",
                cells / 1e6,
                need / 1e9,
                mem as f64 / 1e9
            )));
        }
    }
    let g = Grid::read(&job.input)?;
    let epsg = Grid::read_epsg(&job.input);
    let data = vectorize(&g, job)?;
    paths::ensure_parent(&job.output)?;
    let prj = epsg.and_then(crate::vector::prj_for_epsg);
    let gpkg = job
        .output
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"));
    if gpkg {
        crate::gpkg_write::write(
            &job.output,
            &[crate::gpkg_write::Table {
                name: "poligonos",
                data: &data,
                kind: GeomKind::Polygon,
                epsg,
            }],
        )?;
    } else {
        crate::vector::write_layer(&job.output, &data, GeomKind::Polygon, prj.as_deref())?;
    }
    let area: f64 = data
        .num
        .iter()
        .map(|r| r.get(1).copied().unwrap_or(0.0))
        .sum();
    Ok(format!(
        "{} polígonos, {:.1} m² -> {}",
        data.features.len(),
        area,
        job.output.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cols: usize, rows: usize, v: &[f32]) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 1.0, cols, rows)?;
        g.data = v.to_vec();
        Ok(g)
    }

    fn job(mode: Mode) -> VecJob {
        let mut j = VecJob::new(PathBuf::new(), PathBuf::new());
        j.mode = mode;
        j
    }

    #[test]
    fn a_checkerboard_is_four_parts_or_two_with_corners() -> Result<()> {
        let g = grid(2, 2, &[1.0, 2.0, 2.0, 1.0])?;
        let d = vectorize(&g, &job(Mode::Classes))?;
        assert_eq!(d.features.len(), 4);
        let mut j = job(Mode::Classes);
        j.diagonal = true;
        assert_eq!(vectorize(&g, &j)?.features.len(), 2);
        Ok(())
    }

    #[test]
    fn an_l_with_a_hole_has_an_inner_ring_and_the_area_is_kept() -> Result<()> {
        // un bloque de 3x3 con la celda del medio vacía
        let mut v = vec![1.0f32; 9];
        v[4] = f32::NAN;
        let g = grid(3, 3, &v)?;
        let d = vectorize(&g, &job(Mode::Classes))?;
        assert_eq!(d.features.len(), 1);
        assert_eq!(d.features[0].parts.len(), 2);
        assert_eq!(d.num[0][1], 8.0);
        assert_eq!(d.num[0][2], 8.0);
        Ok(())
    }

    #[test]
    fn the_area_of_the_polygons_is_the_area_of_the_cells() -> Result<()> {
        let v: Vec<f32> = (0..30).map(|i| ((i * 7) % 3) as f32).collect();
        let g = grid(6, 5, &v)?;
        let d = vectorize(&g, &job(Mode::Classes))?;
        let total: f64 = d.num.iter().map(|r| r[1]).sum();
        assert_eq!(total, 30.0);
        Ok(())
    }

    #[test]
    fn small_parts_are_dropped() -> Result<()> {
        let g = grid(4, 1, &[1.0, 1.0, 2.0, 1.0])?;
        let mut j = job(Mode::Classes);
        j.min_cells = 2;
        let d = vectorize(&g, &j)?;
        assert_eq!(d.features.len(), 1);
        assert_eq!(d.num[0][2], 2.0);
        Ok(())
    }

    #[test]
    fn breaks_make_bands_with_their_limits() -> Result<()> {
        let v: Vec<f32> = (0..12).map(|i| i as f32).collect();
        let g = grid(12, 1, &v)?;
        let d = vectorize(&g, &job(Mode::Breaks(vec![3.0, 6.0, 9.0])))?;
        assert_eq!(d.features.len(), 4);
        // la banda 1 es [3, 6)
        assert_eq!((d.num[1][3], d.num[1][4]), (3.0, 6.0));
        assert!(d.num[0][3].is_nan() && d.num[0][4] == 3.0);
        assert!(d.num[3][4].is_nan());
        assert_eq!(d.num[1][2], 3.0);
        Ok(())
    }

    #[test]
    fn a_range_keeps_only_what_falls_in_it() -> Result<()> {
        let g = grid(4, 1, &[1.0, 5.0, 6.0, 9.0])?;
        let d = vectorize(&g, &job(Mode::Range { lo: 5.0, hi: 6.0 }))?;
        assert_eq!(d.features.len(), 1);
        assert_eq!(d.num[0][2], 2.0);
        Ok(())
    }

    #[test]
    fn bad_cuts_and_ranges_are_refused() -> Result<()> {
        let g = grid(2, 1, &[1.0, 2.0])?;
        assert!(vectorize(&g, &job(Mode::Breaks(vec![5.0, 2.0]))).is_err());
        assert!(vectorize(&g, &job(Mode::Range { lo: 3.0, hi: 1.0 })).is_err());
        Ok(())
    }

    #[test]
    fn the_layer_is_written() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_rastervec_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let g = grid(3, 1, &[1.0, 1.0, 2.0])?;
        g.write_tiff(&dir.join("c.tif"), Some(25829))?;
        for ext in ["shp", "gpkg"] {
            let j = VecJob::new(dir.join("c.tif"), dir.join(format!("p.{ext}")));
            run(&j)?;
            assert_eq!(
                crate::view::read_vector(&dir.join(format!("p.{ext}")))?
                    .features
                    .len(),
                2
            );
        }
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
