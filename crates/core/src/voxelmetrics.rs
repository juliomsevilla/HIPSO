//! Métricas de vóxeles: la nube se reparte en columnas de celdas de `cell` metros y, en cada una, en
//! capas de `dz` metros de altura sobre el suelo. Del recuento de puntos de cada vóxel salen
//! métricas de la estructura vertical del dosel, y cada métrica es un ráster con las celdas de la
//! columna.
//!
//! - `puntos`: puntos de la columna.
//! - `voxeles`: vóxeles con al menos `min_points` puntos.
//! - `volumen`: volumen (m³) de esos vóxeles.
//! - `relleno`: proporción de las capas, hasta la más alta ocupada, que están ocupadas.
//! - `hmax`: altura (m) del borde superior del vóxel ocupado más alto.
//! - `hmedia`: altura media (m) de los vóxeles ocupados, en el centro de cada uno.
//! - `entropia`: entropía de Shannon, −Σ p ln p, de la proporción de puntos de cada capa, la
//!   diversidad de alturas del follaje de MacArthur y MacArthur (1961).
//! - `hueco`: proporción de los puntos a menos de `ground_h` metros del suelo, contados con su
//!   altura exacta: la fracción de huecos del dosel.
//! - `lai`: índice de área foliar efectivo del método de MacArthur y Horn (1969) con retornos
//!   discretos: −ln(`hueco`) / k, con k el coeficiente de extinción (véase también Richardson et
//!   al. 2009 sobre los modelos de LAI efectivo con retornos discretos). Las tres últimas coinciden
//!   con las de estructura de las métricas por celda, con la misma altura de suelo por defecto (1 m).
//!
//! La memoria es la de una tesela (columnas × capas), no la de la nube.

use std::path::PathBuf;

use crate::filter::Filter;
use crate::grid::Grid;
use crate::las::Reader;
use crate::pipeline::Tiling;
use crate::tile::{Layout, Tile, Tiled};
use crate::{Error, Result, paths};

/// Los nombres de las métricas, en el orden en que se calculan.
pub const METRICS: [&str; 9] = [
    "puntos", "voxeles", "volumen", "relleno", "hmax", "hmedia", "entropia", "hueco", "lai",
];

pub struct VoxelJob {
    pub inputs: Vec<PathBuf>,
    /// MDT con el que se normalizan las alturas; sin él, la nube ya tiene alturas sobre el suelo.
    pub dtm: Option<PathBuf>,
    /// Lado de la columna (m).
    pub cell: f64,
    /// Grosor de la capa (m).
    pub dz: f64,
    /// Altura (m) por encima de la cual los puntos no cuentan.
    pub max_height: f64,
    /// Un vóxel está ocupado con al menos tantos puntos.
    pub min_points: u32,
    /// Los puntos a menos de esta altura (m) son «suelo» para la fracción de huecos y el LAI.
    pub ground_h: f64,
    /// Coeficiente de extinción de la luz del LAI.
    pub k: f64,
    /// Las métricas que se escriben (nombres de [`METRICS`]); vacío: todas.
    pub metrics: Vec<String>,
    pub filter: Filter,
    pub tiling: Tiling,
    /// Ruta base: cada métrica es `<salida>_<métrica>.tif`.
    pub output: PathBuf,
}

impl VoxelJob {
    pub fn new(inputs: Vec<PathBuf>, output: PathBuf) -> Self {
        Self {
            inputs,
            dtm: None,
            cell: 2.0,
            dz: 1.0,
            max_height: 60.0,
            min_points: 1,
            ground_h: 1.0,
            k: 0.5,
            metrics: Vec::new(),
            filter: Filter {
                skip_noise: true,
                ..Filter::default()
            },
            tiling: Tiling {
                size: 250.0,
                buffer: 0.0,
                threads: 0,
            },
            output,
        }
    }
}

/// Las métricas de una columna a partir de los puntos de cada capa y de cuántos están por debajo
/// de la altura de suelo (contados con su altura exacta); `NaN` cuando no hay puntos.
pub fn column_metrics(
    counts: &[u32],
    low: u32,
    cell: f64,
    dz: f64,
    min_points: u32,
    k: f64,
) -> [f32; 9] {
    let total: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    if total == 0 {
        let mut m = [f32::NAN; 9];
        m[0] = 0.0;
        return m;
    }
    let occupied: Vec<usize> = counts
        .iter()
        .enumerate()
        .filter(|(_, c)| **c >= min_points.max(1))
        .map(|(i, _)| i)
        .collect();
    let nvox = occupied.len();
    let top = occupied.last().copied();
    let fill = top.map_or(f64::NAN, |t| nvox as f64 / (t + 1) as f64);
    let hmax = top.map_or(f64::NAN, |t| (t + 1) as f64 * dz);
    let hmean = if nvox == 0 {
        f64::NAN
    } else {
        occupied.iter().map(|&i| (i as f64 + 0.5) * dz).sum::<f64>() / nvox as f64
    };
    let tf = total as f64;
    let entropy: f64 = counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| c as f64 / tf)
        .map(|p| -p * p.ln())
        .sum();
    let low = f64::from(low);
    let gap = low / tf;
    let lai = if gap > 0.0 { -gap.ln() / k } else { f64::NAN };
    [
        total as f32,
        nvox as f32,
        (nvox as f64 * cell * cell * dz) as f32,
        fill as f32,
        hmax as f32,
        hmean as f32,
        entropy as f32,
        gap as f32,
        lai as f32,
    ]
}

/// Las métricas calculadas: nombre y ráster.
pub type Rasters = Vec<(&'static str, Grid)>;

/// Las métricas pedidas, cada una como ráster, con el sistema de coordenadas de la nube.
pub fn compute(job: &VoxelJob) -> Result<(Rasters, Option<u16>, u64)> {
    if !(job.cell > 0.0)
        || !(job.dz > 0.0)
        || !(job.max_height > 0.0)
        || !(job.k > 0.0)
        || !(job.ground_h >= 0.0)
    {
        return Err(Error::Param(
            "la celda, el grosor de capa, la altura máxima y el coeficiente de extinción deben ser positivos".into(),
        ));
    }
    if job.inputs.is_empty() {
        return Err(Error::Param("no hay nubes de entrada".into()));
    }
    let want: Vec<usize> = if job.metrics.is_empty() {
        (0..METRICS.len()).collect()
    } else {
        job.metrics
            .iter()
            .map(|n| {
                METRICS
                    .iter()
                    .position(|m| m.eq_ignore_ascii_case(n))
                    .ok_or_else(|| {
                        Error::Param(format!(
                            "métrica «{n}» desconocida (hay: {})",
                            METRICS.join(", ")
                        ))
                    })
            })
            .collect::<Result<_>>()?
    };
    let nz = (job.max_height / job.dz).ceil() as usize;
    if nz == 0 || nz > 4096 {
        return Err(Error::Param(
            "la altura máxima y el grosor de capa dan un número de capas inválido".into(),
        ));
    }
    let dtm = job.dtm.as_ref().map(|p| Grid::read(p)).transpose()?;
    let readers: Vec<Reader> = job
        .inputs
        .iter()
        .map(|p| Reader::open(p))
        .collect::<Result<_>>()?;
    let epsg = readers.first().and_then(|r| r.header.epsg);
    let b = readers.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, r| {
            let h = r.header.bounds;
            [
                b[0].min(h[0]),
                b[1].min(h[1]),
                b[2].max(h[3]),
                b[3].max(h[4]),
            ]
        },
    );
    let layout = Layout::new(b, job.tiling.size, 0.0, job.cell)?;
    let mut out: Vec<Grid> = (0..METRICS.len())
        .map(|_| {
            let mut g = Grid::covering(b, job.cell)?;
            g.data.fill(f32::NAN);
            Ok(g)
        })
        .collect::<Result<_>>()?;
    let mut tiled = Tiled::spool(&readers, layout)?;
    tiled.keep_records = false;
    let mut dropped = 0u64;
    let cell = job.cell;
    tiled.process(
        job.tiling.threads,
        |tile: Tile| {
            let e = tile.core;
            let (cols, rows) = (
                ((e[2] - e[0]) / cell).round() as usize,
                ((e[3] - e[1]) / cell).round() as usize,
            );
            let base = Grid::new(e[0], e[1], cell, cols, rows)?;
            let mut counts = vec![0u32; cols * rows * nz];
            let mut lows = vec![0u32; cols * rows];
            let mut lost = 0u64;
            for (q, own) in tile.points.pts.iter().zip(&tile.own) {
                if !*own || !job.filter.keeps(q) {
                    continue;
                }
                let h = match &dtm {
                    Some(d) => match d.sample(q.x, q.y) {
                        Some(g) => q.z - f64::from(g),
                        None => {
                            lost += 1;
                            continue;
                        }
                    },
                    None => q.z,
                };
                if !h.is_finite() || h < -1.0 || h >= job.max_height || !job.filter.z_ok(h) {
                    lost += 1;
                    continue;
                }
                // las filas de una rejilla van de norte a sur
                let Some((c, r)) = base.col_row(q.x, q.y) else {
                    lost += 1;
                    continue;
                };
                let k = ((h.max(0.0) / job.dz) as usize).min(nz - 1);
                counts[(r * cols + c) * nz + k] += 1;
                lows[r * cols + c] += u32::from(h < job.ground_h);
            }
            let mut g: Vec<Grid> = (0..METRICS.len())
                .map(|_| {
                    let mut g = Grid::new(e[0], e[1], cell, cols, rows)?;
                    g.data.fill(f32::NAN);
                    Ok(g)
                })
                .collect::<Result<_>>()?;
            for i in 0..cols * rows {
                let m = column_metrics(
                    &counts[i * nz..(i + 1) * nz],
                    lows[i],
                    cell,
                    job.dz,
                    job.min_points,
                    job.k,
                );
                for (grid, v) in g.iter_mut().zip(m) {
                    grid.data[i] = v;
                }
            }
            Ok((g, lost))
        },
        |(g, lost)| {
            dropped += lost;
            for (dst, src) in out.iter_mut().zip(&g) {
                for row in 0..src.rows {
                    for col in 0..src.cols {
                        let (x, y) = src.center(col, row);
                        if let Some(i) = dst.index(x, y) {
                            dst.data[i] = src.data[row * src.cols + col];
                        }
                    }
                }
            }
            Ok(())
        },
    )?;
    let mut slots: Vec<Option<Grid>> = out.into_iter().map(Some).collect();
    let result = want
        .iter()
        .filter_map(|&i| slots[i].take().map(|g| (METRICS[i], g)))
        .collect();
    Ok((result, epsg, dropped))
}

/// Calcula las métricas y las escribe, una por ráster; devuelve el resumen.
pub fn run(job: &VoxelJob) -> Result<String> {
    let (grids, epsg, dropped) = compute(job)?;
    paths::ensure_parent(&job.output)?;
    let mut lines = vec![format!(
        "vóxeles de {} x {} m, capas de {} m hasta {} m: {} métricas{}",
        job.cell,
        job.cell,
        job.dz,
        job.max_height,
        grids.len(),
        if dropped > 0 {
            format!(" ({dropped} puntos fuera de las alturas o del MDT no cuentan)")
        } else {
            String::new()
        }
    )];
    let base = paths::stem(&job.output);
    for (name, g) in &grids {
        let p = job.output.with_file_name(format!("{base}_{name}.tif"));
        g.write_tiff(&p, epsg)?;
        let vals: Vec<f64> = g
            .data
            .iter()
            .filter(|v| v.is_finite())
            .map(|v| f64::from(*v))
            .collect();
        let mean = vals.iter().sum::<f64>() / vals.len().max(1) as f64;
        lines.push(format!(
            "  {name}: media {mean:.3} en {} columnas -> {}",
            vals.len(),
            p.display()
        ));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_column_has_no_metrics_and_zero_points() {
        let m = column_metrics(&[0, 0, 0], 0, 2.0, 1.0, 1, 0.5);
        assert_eq!(m[0], 0.0);
        assert!(m[1..].iter().all(|v| v.is_nan()));
    }

    #[test]
    fn two_layers_with_equal_counts() {
        // capas 0 y 3 con 10 puntos cada una, celdas de 2 m y capas de 1 m
        let m = column_metrics(&[10, 0, 0, 10, 0], 10, 2.0, 1.0, 1, 0.5);
        assert_eq!(m[0], 20.0);
        assert_eq!(m[1], 2.0);
        assert_eq!(m[2], 8.0); // 2 vóxeles de 2 x 2 x 1
        assert!((m[3] - 0.5).abs() < 1e-6); // 2 de las 4 capas hasta la más alta
        assert_eq!(m[4], 4.0);
        assert!((m[5] - 2.0).abs() < 1e-6); // centros a 0,5 y 3,5
        assert!((m[6] - 2.0f32.ln()).abs() < 1e-6);
        assert!((m[7] - 0.5).abs() < 1e-6); // la mitad de los puntos bajo 1 m
        assert!((m[8] - 2.0 * 2.0f32.ln()).abs() < 1e-5); // -ln(0,5)/0,5
    }

    #[test]
    fn min_points_decides_occupancy() {
        let m = column_metrics(&[1, 5, 1], 0, 1.0, 1.0, 2, 0.5);
        assert_eq!(m[1], 1.0);
        assert_eq!(m[4], 2.0);
    }
}
