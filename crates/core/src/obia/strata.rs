//! Estratos: objetos clasificados unidos en áreas homogéneas, y cuántas parcelas de muestra merece
//! cada área.

use std::collections::HashMap;

use super::segment::label_components;
use super::{Labels, Lattice};
use crate::{Result, check_cancel};

/// Áreas unidas: una etiqueta por celda y la clase de cada área (las celdas de clase 0 conservan la
/// etiqueta 0).
pub struct Strata {
    pub labels: Labels,
    /// La clase del área `i + 1`.
    pub class: Vec<u16>,
    /// La clase de cada celda, después de que las áreas pequeñas tomaran la clase de sus vecinas.
    pub cell_class: Vec<u16>,
}

/// Une los objetos de la misma clase que se tocan en un área. Las áreas de menos de `min_cells`
/// toman la clase con la que más limitan entre las áreas suficientemente grandes y se unen a ellas;
/// un área que no tiene vecina grande se queda como está.
pub fn dissolve(labels: &Labels, class: &[u16], min_cells: u32) -> Result<Strata> {
    let Lattice { cols, rows, .. } = labels.lat;
    let mut cell_class: Vec<u16> = labels
        .ids
        .iter()
        .map(|&l| if l == 0 { 0 } else { class[l as usize - 1] })
        .collect();
    let areas =
        |cls: &[u16]| label_components(cols, rows, false, |i| cls[i] > 0, |i, j| cls[i] == cls[j]);
    // cada ronda reasigna las áreas pequeñas a la clase de una vecina mayor; unas pocas rondas lo
    // asientan
    for _ in 0..12 {
        check_cancel()?;
        let (lab, n) = areas(&cell_class)?;
        if min_cells <= 1 || n == 0 {
            break;
        }
        let mut size = vec![0u32; n as usize + 1];
        for &l in &lab {
            size[l as usize] += 1;
        }
        let mut votes: HashMap<(u32, u16), u32> = HashMap::new();
        let mut vote = |a: u32, b: u32, class_b: u16| {
            if size[a as usize] < min_cells && size[b as usize] >= min_cells {
                *votes.entry((a, class_b)).or_insert(0) += 1;
            }
        };
        for y in 0..rows {
            for x in 0..cols {
                let i = y * cols + x;
                if lab[i] == 0 {
                    continue;
                }
                for j in [
                    (x + 1 < cols).then_some(i + 1),
                    (y + 1 < rows).then_some(i + cols),
                ]
                .into_iter()
                .flatten()
                {
                    if lab[j] != 0 && lab[j] != lab[i] {
                        vote(lab[i], lab[j], cell_class[j]);
                        vote(lab[j], lab[i], cell_class[i]);
                    }
                }
            }
        }
        if votes.is_empty() {
            break;
        }
        // la clase con más votos para cada área pequeña (el número de clase menor en un empate)
        let mut best: HashMap<u32, (u16, u32)> = HashMap::new();
        for (&(a, c), &v) in &votes {
            let e = best.entry(a).or_insert((c, v));
            if v > e.1 || (v == e.1 && c < e.0) {
                *e = (c, v);
            }
        }
        for (i, l) in lab.iter().enumerate() {
            if let Some(&(c, _)) = best.get(l) {
                cell_class[i] = c;
            }
        }
    }
    let (ids, count) = areas(&cell_class)?;
    let mut class_of = vec![0u16; count as usize];
    for (i, &l) in ids.iter().enumerate() {
        if l != 0 {
            class_of[l as usize - 1] = cell_class[i];
        }
    }
    Ok(Strata {
        labels: Labels {
            lat: labels.lat,
            ids,
            count,
        },
        class: class_of,
        cell_class,
    })
}

/// Qué aspecto tiene un estrato en una capa.
#[derive(Clone, Debug, PartialEq)]
pub struct StratumStat {
    pub class: u16,
    pub polygons: usize,
    pub cells: u64,
    pub mean: f64,
    pub sd: f64,
}

impl StratumStat {
    /// Coeficiente de variación en porcentaje: cuán heterogéneo es el estrato.
    pub fn cv(&self) -> f64 {
        if self.mean.abs() > 1e-12 {
            100.0 * self.sd / self.mean.abs()
        } else {
            f64::NAN
        }
    }
}

/// Área, media y desviación típica de `layer` en cada clase (`n_classes` de ellas, códigos desde 1)
/// sobre las celdas que tienen una clase y un valor, y cuántas áreas separadas tiene cada clase.
pub fn summarize(
    cell_class: &[u16],
    layer: &[f32],
    area_classes: &[u16],
    n_classes: usize,
) -> Vec<StratumStat> {
    let mut cells = vec![0u64; n_classes];
    let mut sum = vec![0f64; n_classes];
    let mut sq = vec![0f64; n_classes];
    let mut polygons = vec![0usize; n_classes];
    for &c in area_classes {
        if c > 0 {
            polygons[c as usize - 1] += 1;
        }
    }
    // centrado en el primer valor visto, por precisión
    let shift = layer
        .iter()
        .find(|v| v.is_finite())
        .copied()
        .map_or(0.0, f64::from);
    for (&c, &v) in cell_class.iter().zip(layer) {
        if c > 0 && v.is_finite() {
            let z = f64::from(v) - shift;
            let k = c as usize - 1;
            cells[k] += 1;
            sum[k] += z;
            sq[k] += z * z;
        }
    }
    (0..n_classes)
        .map(|k| {
            let n = cells[k] as f64;
            let (mean, sd) = if cells[k] > 0 {
                let m = sum[k] / n;
                (shift + m, (sq[k] / n - m * m).max(0.0).sqrt())
            } else {
                (f64::NAN, f64::NAN)
            };
            StratumStat {
                class: k as u16 + 1,
                polygons: polygons[k],
                cells: cells[k],
                mean,
                sd,
            }
        })
        .collect()
}

/// Reparte `plots` parcelas de muestra entre los estratos en proporción al área por la desviación
/// típica (asignación de Neyman: los estratos grandes y variables reciben más), al menos dos cada
/// uno cuando hay suficientes parcelas. Devuelve un recuento por entrada de `stats`.
pub fn allocate(stats: &[StratumStat], plots: usize) -> Vec<usize> {
    let h = stats.iter().filter(|s| s.cells > 0).count();
    let mut out = vec![0usize; stats.len()];
    if h == 0 || plots == 0 {
        return out;
    }
    let floor = if plots >= 2 * h { 2 } else { 0 };
    for (o, s) in out.iter_mut().zip(stats) {
        if s.cells > 0 {
            *o = floor;
        }
    }
    let rest = plots - floor * h;
    let weight: Vec<f64> = stats
        .iter()
        .map(|s| {
            if s.cells > 0 {
                s.cells as f64 * s.sd.max(1e-9)
            } else {
                0.0
            }
        })
        .collect();
    let total: f64 = weight.iter().sum();
    let exact: Vec<f64> = weight.iter().map(|w| rest as f64 * w / total).collect();
    for (o, e) in out.iter_mut().zip(&exact) {
        *o += e.floor() as usize;
    }
    // las parcelas que sobran van a las mayores fracciones, la clase menor primero en un empate
    let mut left = plots - out.iter().sum::<usize>();
    let mut order: Vec<usize> = (0..stats.len()).filter(|&k| stats[k].cells > 0).collect();
    order.sort_by(|&a, &b| {
        (exact[b] - exact[b].floor())
            .total_cmp(&(exact[a] - exact[a].floor()))
            .then(a.cmp(&b))
    });
    for k in order.into_iter().cycle().take(left.min(1_000_000)) {
        out[k] += 1;
        left -= 1;
        if left == 0 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(cols: usize, rows: usize, ids: Vec<u32>) -> Labels {
        let count = ids.iter().copied().max().unwrap_or(0);
        Labels {
            lat: Lattice {
                x0: 0.0,
                y0: 0.0,
                cell: 1.0,
                cols,
                rows,
            },
            ids,
            count,
        }
    }

    #[test]
    fn neighbours_of_one_class_become_one_area() -> Result<()> {
        // objetos 1, 2 y 3 lado a lado; 1 y 2 son clase 1, 3 es clase 2
        let l = labels(6, 2, vec![1, 1, 2, 2, 3, 3, 1, 1, 2, 2, 3, 3]);
        let s = dissolve(&l, &[1, 1, 2], 1)?;
        assert_eq!(s.labels.count, 2);
        assert_eq!(s.class, vec![1, 2]);
        assert_eq!(s.labels.ids[0], s.labels.ids[3]);
        Ok(())
    }

    #[test]
    fn small_areas_join_their_biggest_neighbour() -> Result<()> {
        // una isla de 2 celdas de clase 2 dentro de la clase 1, y un área grande de clase 2 en otro
        // sitio
        let mut ids = vec![1u32; 40];
        ids[11] = 2;
        ids[12] = 2;
        ids[30..40].fill(3);
        let l = labels(10, 4, ids);
        let s = dissolve(&l, &[1, 2, 2], 5)?;
        // la isla tomó la clase del área que la rodea, así que hay dos áreas
        assert_eq!(s.labels.count, 2);
        assert_eq!(s.cell_class[11], 1);
        assert_eq!(s.cell_class[35], 2);
        let kept = dissolve(&l, &[1, 2, 2], 1)?;
        assert_eq!(kept.labels.count, 3);
        Ok(())
    }

    #[test]
    fn neyman_gives_more_to_big_and_variable_strata() {
        let stat = |class, cells, sd| StratumStat {
            class,
            polygons: 1,
            cells,
            mean: 10.0,
            sd,
        };
        let stats = [stat(1, 1000, 1.0), stat(2, 1000, 3.0), stat(3, 0, 0.0)];
        let a = allocate(&stats, 20);
        assert_eq!(a.iter().sum::<usize>(), 20);
        assert_eq!(a[2], 0);
        assert!(a[1] > a[0] && a[0] >= 2);
        assert_eq!(allocate(&stats, 3).iter().sum::<usize>(), 3);
    }

    #[test]
    fn summary_and_variation() {
        let st = summarize(&[1, 1, 2, 0], &[2.0, 4.0, 10.0, 99.0], &[1, 2], 2);
        assert_eq!((st[0].cells, st[0].mean), (2, 3.0));
        assert!((st[0].sd - 1.0).abs() < 1e-9);
        assert!((st[0].cv() - 100.0 / 3.0).abs() < 1e-9);
        assert_eq!(st[1].polygons, 1);
    }
}
