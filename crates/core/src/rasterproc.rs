//! Operaciones ráster que faltaban entre las de uso general: rasterizar una capa vectorial,
//! filtros focales (media, mediana, mínimo, máximo, desviación, suma, rango, mayoría y gaussiano)
//! y estadísticas de un ráster (globales, con histograma, y tabuladas por clases de otro ráster).
//!
//! Trabajan sobre rejillas en memoria ([`Grid`]), que ya se niegan a crecer más de lo que cabe en
//! el equipo; las celdas vacías son NaN y nunca cuentan.

use std::path::Path;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::view::{GeomKind, VectorData};
use crate::{Error, Result};

// ---------------------------------------------------------------------------------------------
// rasterizar

/// De dónde sale el marco de la rejilla que se rasteriza.
pub enum Frame<'a> {
    /// Las celdas de un ráster de referencia.
    Reference(&'a Path),
    /// La caja de la capa, con celdas de este tamaño.
    Cell(f64),
}

/// Rasteriza una capa: cada celda recibe el valor del campo `field` del último elemento que la
/// cubre (o 1 si no hay campo). Los polígonos cubren las celdas cuyo centro está dentro (regla
/// par-impar con sus huecos), las líneas las celdas que atraviesan y los puntos la celda que los
/// contiene. Las celdas que ningún elemento cubre quedan vacías.
pub fn rasterize(layer: &VectorData, kind: GeomKind, frame: Frame, field: Option<&str>) -> Result<Grid> {
    let col = match field {
        Some(f) => Some(
            layer
                .fields
                .iter()
                .position(|g| g.eq_ignore_ascii_case(f))
                .ok_or_else(|| Error::Param(format!("la capa no tiene el campo «{f}»; tiene: {}", layer.fields.join(", "))))?,
        ),
        None => None,
    };
    let mut g = match frame {
        Frame::Reference(p) => {
            let r = Grid::read(p)?;
            r.like()
        }
        Frame::Cell(cell) => {
            if !(cell > 0.0) || !layer.bounds.iter().all(|v| v.is_finite()) {
                return Err(Error::Param("hace falta una celda positiva y una capa con elementos".into()));
            }
            let b = layer.bounds;
            Grid::covering([b[0], b[1], b[2] + cell, b[3] + cell], cell)?
        }
    };
    let top = g.y0 + g.rows as f64 * g.cell;
    for (i, f) in layer.features.iter().enumerate() {
        let v = match col {
            Some(c) => {
                let n = layer.num.get(i).and_then(|r| r.get(c)).copied().unwrap_or(f64::NAN);
                if !n.is_finite() {
                    continue;
                }
                n as f32
            }
            None => 1.0,
        };
        match kind {
            GeomKind::Point => {
                for p in f.parts.iter().filter_map(|p| p.first()) {
                    let (cx, cy) = ((p[0] - g.x0) / g.cell, (top - p[1]) / g.cell);
                    if cx >= 0.0 && cy >= 0.0 && (cx as usize) < g.cols && (cy as usize) < g.rows {
                        g.data[cy as usize * g.cols + cx as usize] = v;
                    }
                }
            }
            GeomKind::Line => {
                for part in &f.parts {
                    for w in part.windows(2) {
                        burn_segment(&mut g, top, w[0], w[1], v);
                    }
                }
            }
            GeomKind::Polygon => burn_polygon(&mut g, top, &f.parts, v),
        }
    }
    Ok(g)
}

/// Pinta las celdas que atraviesa un segmento (recorrido por las rejillas, de celda en celda).
fn burn_segment(g: &mut Grid, top: f64, a: [f64; 2], b: [f64; 2], v: f32) {
    let (ax, ay) = ((a[0] - g.x0) / g.cell, (top - a[1]) / g.cell);
    let (bx, by) = ((b[0] - g.x0) / g.cell, (top - b[1]) / g.cell);
    let steps = ((bx - ax).abs().max((by - ay).abs()) * 2.0).ceil().max(1.0) as usize;
    for k in 0..=steps {
        let t = k as f64 / steps as f64;
        let (x, y) = (ax + (bx - ax) * t, ay + (by - ay) * t);
        if x >= 0.0 && y >= 0.0 && (x as usize) < g.cols && (y as usize) < g.rows {
            g.data[y as usize * g.cols + x as usize] = v;
        }
    }
}

/// Rellena un polígono (todas sus partes juntas, regla par-impar) fila a fila.
fn burn_polygon(g: &mut Grid, top: f64, rings: &[Vec<[f64; 2]>], v: f32) {
    let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in rings.iter().flatten() {
        ymin = ymin.min(p[1]);
        ymax = ymax.max(p[1]);
    }
    if !ymin.is_finite() {
        return;
    }
    let r0 = (((top - ymax) / g.cell).floor().max(0.0)) as usize;
    let r1 = ((((top - ymin) / g.cell).ceil()).max(0.0) as usize).min(g.rows);
    let mut xs: Vec<f64> = Vec::new();
    for r in r0..r1 {
        let y = top - (r as f64 + 0.5) * g.cell;
        xs.clear();
        for ring in rings {
            for w in ring.windows(2) {
                let (a, b) = (w[0], w[1]);
                if (a[1] > y) != (b[1] > y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            // un anillo abierto se cierra solo
            if let (Some(a), Some(b)) = (ring.last(), ring.first())
                && a != b
                && (a[1] > y) != (b[1] > y)
            {
                xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        xs.sort_by(|a, b| a.total_cmp(b));
        for pair in xs.as_chunks::<2>().0 {
            // cells cuyo centro cae entre dos cruces
            let c0 = ((pair[0] - g.x0) / g.cell - 0.5).ceil().max(0.0) as usize;
            let c1v = ((pair[1] - g.x0) / g.cell - 0.5).floor();
            if c1v < 0.0 {
                continue;
            }
            let c1 = (c1v as usize).min(g.cols.saturating_sub(1));
            for c in c0..=c1 {
                g.data[r * g.cols + c] = v;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// filtros focales

/// Qué calcula un filtro focal sobre su ventana.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Focal {
    Mean,
    Median,
    Min,
    Max,
    Sum,
    Range,
    Std,
    /// El valor más frecuente (para clases).
    Majority,
    /// Media ponderada por una gaussiana de esta desviación típica, en celdas.
    Gaussian(f32),
}

impl Focal {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_lowercase().as_str() {
            "mean" | "media" => Focal::Mean,
            "median" | "mediana" => Focal::Median,
            "min" => Focal::Min,
            "max" => Focal::Max,
            "sum" | "suma" => Focal::Sum,
            "range" | "rango" => Focal::Range,
            "std" | "desv" => Focal::Std,
            "majority" | "mayoria" | "mayoría" => Focal::Majority,
            other => return Err(Error::Param(format!("filtro desconocido «{other}»: mean, median, min, max, sum, range, std, majority"))),
        })
    }
}

/// Aplica un filtro focal con una ventana de radio `radius` celdas (cuadrada o circular). Una
/// celda vacía sigue vacía; una celda con menos de `min_valid` vecinas con valor (la propia
/// incluida) queda vacía.
pub fn focal(g: &Grid, stat: Focal, radius: usize, circle: bool, min_valid: usize) -> Result<Grid> {
    if radius == 0 {
        return Err(Error::Param("el radio de la ventana debe ser de al menos una celda".into()));
    }
    let r = radius as isize;
    let offsets: Vec<(isize, isize, f32)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| !circle || dx * dx + dy * dy <= r * r)
        .map(|(dx, dy)| {
            let w = match stat {
                Focal::Gaussian(s) if s > 0.0 => (-((dx * dx + dy * dy) as f32) / (2.0 * s * s)).exp(),
                _ => 1.0,
            };
            (dx, dy, w)
        })
        .collect();
    let mut out = Grid::new(g.x0, g.y0, g.cell, g.cols, g.rows)?;
    let (cols, rows) = (g.cols as isize, g.rows as isize);
    out.data.par_chunks_mut(g.cols).enumerate().for_each(|(row, line)| {
        let mut vals: Vec<f32> = Vec::with_capacity(offsets.len());
        let mut wts: Vec<f32> = Vec::with_capacity(offsets.len());
        for (col, cell) in line.iter_mut().enumerate() {
            if !g.data[row * g.cols + col].is_finite() {
                continue;
            }
            vals.clear();
            wts.clear();
            for &(dx, dy, w) in &offsets {
                let (c, r2) = (col as isize + dx, row as isize + dy);
                if c < 0 || r2 < 0 || c >= cols || r2 >= rows {
                    continue;
                }
                let v = g.data[r2 as usize * g.cols + c as usize];
                if v.is_finite() {
                    vals.push(v);
                    wts.push(w);
                }
            }
            if vals.is_empty() || vals.len() < min_valid.max(1) {
                continue;
            }
            *cell = reduce(stat, &mut vals, &wts);
        }
    });
    Ok(out)
}

fn reduce(stat: Focal, v: &mut [f32], w: &[f32]) -> f32 {
    let n = v.len() as f32;
    match stat {
        Focal::Mean => v.iter().sum::<f32>() / n,
        Focal::Sum => v.iter().sum(),
        Focal::Min => v.iter().copied().fold(f32::INFINITY, f32::min),
        Focal::Max => v.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        Focal::Range => {
            v.iter().copied().fold(f32::NEG_INFINITY, f32::max) - v.iter().copied().fold(f32::INFINITY, f32::min)
        }
        Focal::Std => {
            let m = v.iter().sum::<f32>() / n;
            (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / n).sqrt()
        }
        Focal::Median => {
            v.sort_by(|a, b| a.total_cmp(b));
            let k = v.len() / 2;
            if v.len() % 2 == 1 { v[k] } else { (v[k - 1] + v[k]) / 2.0 }
        }
        Focal::Majority => {
            v.sort_by(|a, b| a.total_cmp(b));
            let (mut best, mut best_n, mut run, mut cur) = (v[0], 0usize, 0usize, v[0]);
            for &x in v.iter() {
                if x == cur {
                    run += 1;
                } else {
                    cur = x;
                    run = 1;
                }
                if run > best_n {
                    best_n = run;
                    best = cur;
                }
            }
            best
        }
        Focal::Gaussian(_) => {
            let (s, ws) = v.iter().zip(w).fold((0.0, 0.0), |(s, ws), (x, w)| (s + x * w, ws + w));
            s / ws
        }
    }
}

// ---------------------------------------------------------------------------------------------
// estadísticas

/// Estadísticos de un ráster y su histograma.
pub struct RasterStats {
    pub count: usize,
    pub empty: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub std: f64,
    pub median: f64,
    /// Percentiles 5, 25, 75 y 95.
    pub percentiles: [f64; 4],
    /// Límite inferior de cada intervalo, su anchura y los recuentos.
    pub histogram: (f64, f64, Vec<usize>),
    /// Área de cada celda.
    pub cell_area: f64,
}

/// Estadísticos de las celdas con valor de `g`, con un histograma de `bins` intervalos.
pub fn stats(g: &Grid, bins: usize) -> Result<RasterStats> {
    let mut v: Vec<f32> = g.data.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return Err(Error::Param("el ráster no tiene ninguna celda con valor".into()));
    }
    let empty = g.data.len() - v.len();
    v.par_sort_by(|a, b| a.total_cmp(b));
    let n = v.len() as f64;
    let mean = v.iter().map(|x| f64::from(*x)).sum::<f64>() / n;
    let var = v.iter().map(|x| (f64::from(*x) - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    let q = |p: f64| {
        let r = p * (n - 1.0);
        let (lo, hi) = (r.floor() as usize, r.ceil() as usize);
        f64::from(v[lo]) + (f64::from(v[hi]) - f64::from(v[lo])) * (r - lo as f64)
    };
    let (min, max) = (f64::from(v[0]), f64::from(v[v.len() - 1]));
    let bins = bins.max(1);
    let width = if max > min { (max - min) / bins as f64 } else { 1.0 };
    let mut hist = vec![0usize; bins];
    for x in &v {
        let k = (((f64::from(*x) - min) / width) as usize).min(bins - 1);
        hist[k] += 1;
    }
    Ok(RasterStats {
        count: v.len(),
        empty,
        min,
        max,
        mean,
        std: var.sqrt(),
        median: q(0.5),
        percentiles: [q(0.05), q(0.25), q(0.75), q(0.95)],
        histogram: (min, width, hist),
        cell_area: g.cell * g.cell,
    })
}

/// Las estadísticas como texto (informe) y como CSV del histograma.
pub fn stats_report(s: &RasterStats) -> String {
    format!(
        "celdas con valor: {} (vacías: {}), área {:.3} m²\nmínimo {:.4}, máximo {:.4}, media {:.4}, desviación {:.4}, mediana {:.4}\npercentiles 5/25/75/95: {:.4} / {:.4} / {:.4} / {:.4}",
        s.count,
        s.empty,
        s.count as f64 * s.cell_area,
        s.min,
        s.max,
        s.mean,
        s.std,
        s.median,
        s.percentiles[0],
        s.percentiles[1],
        s.percentiles[2],
        s.percentiles[3]
    )
}

pub fn histogram_csv(s: &RasterStats) -> String {
    let (lo, w, h) = &s.histogram;
    let mut out = String::from("desde,hasta,celdas,area\n");
    for (k, c) in h.iter().enumerate() {
        out.push_str(&format!(
            "{},{},{},{}\n",
            lo + w * k as f64,
            lo + w * (k + 1) as f64,
            c,
            *c as f64 * s.cell_area
        ));
    }
    out
}

/// El valor de la celda de `g` que contiene `(x, y)` (la más cercana), o NaN fuera de ella.
fn nearest(g: &Grid, x: f64, y: f64) -> f32 {
    let top = g.y0 + g.rows as f64 * g.cell;
    let (cx, cy) = ((x - g.x0) / g.cell, (top - y) / g.cell);
    if cx < 0.0 || cy < 0.0 || cx as usize >= g.cols || cy as usize >= g.rows {
        f32::NAN
    } else {
        g.data[cy as usize * g.cols + cx as usize]
    }
}

/// Área (m²) de cada combinación de la clase de `zones` y la de `values` (tabular área): una fila
/// por zona, una columna por clase de `values`, sobre las celdas de `zones` (los valores se toman
/// de la celda de `values` que cae bajo el centro de cada una). Los valores se redondean a enteros.
/// Devuelve el CSV y cuántas celdas no tenían valor en alguno de los dos.
pub fn tabulate(zones: &Grid, values: &Grid) -> Result<(String, usize)> {
    use std::collections::BTreeMap;
    let top = zones.y0 + zones.rows as f64 * zones.cell;
    let mut table: BTreeMap<i64, BTreeMap<i64, usize>> = BTreeMap::new();
    let mut classes = std::collections::BTreeSet::new();
    let mut skipped = 0usize;
    for r in 0..zones.rows {
        let y = top - (r as f64 + 0.5) * zones.cell;
        for c in 0..zones.cols {
            let z = zones.data[r * zones.cols + c];
            let x = zones.x0 + (c as f64 + 0.5) * zones.cell;
            let v = nearest(values, x, y);
            if !z.is_finite() || !v.is_finite() {
                skipped += 1;
                continue;
            }
            let (z, v) = (z.round() as i64, v.round() as i64);
            *table.entry(z).or_default().entry(v).or_insert(0) += 1;
            classes.insert(v);
        }
    }
    if table.is_empty() {
        return Err(Error::Param("los dos rásteres no se solapan en ninguna celda con valor".into()));
    }
    let area = zones.cell * zones.cell;
    let mut out = String::from("zona");
    for c in &classes {
        out.push_str(&format!(",clase_{c}"));
    }
    out.push_str(",total\n");
    for (z, row) in &table {
        out.push_str(&z.to_string());
        let mut total = 0.0;
        for c in &classes {
            let a = row.get(c).copied().unwrap_or(0) as f64 * area;
            total += a;
            out.push_str(&format!(",{a}"));
        }
        out.push_str(&format!(",{total}\n"));
    }
    Ok((out, skipped))
}

/// Escribe una rejilla como GeoTIFF con el sistema de otro archivo.
pub fn write_like(g: &Grid, out: &Path, like: &Path) -> Result<()> {
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    g.write_tiff(out, crate::crs::epsg_of(like))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Feature;

    fn grid(cols: usize, rows: usize, f: impl Fn(usize, usize) -> f32) -> Grid {
        let mut g = Grid::new(0.0, 0.0, 1.0, cols, rows).expect("rejilla");
        for r in 0..rows {
            for c in 0..cols {
                g.data[r * cols + c] = f(c, r);
            }
        }
        g
    }

    #[test]
    fn a_polygon_with_a_hole_burns_the_cells_whose_centre_is_inside() -> Result<()> {
        let outer = vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0], [0.0, 0.0]];
        let hole = vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0], [4.0, 4.0]];
        let mut d = VectorData { polygons: true, ..VectorData::default() };
        d.features.push(Feature { parts: vec![outer, hole] });
        d.fields = vec!["clase".into()];
        d.text = vec![vec!["7".into()]];
        d.num = vec![vec![7.0]];
        d.bounds = [0.0, 0.0, 10.0, 10.0];
        let g = rasterize(&d, GeomKind::Polygon, Frame::Cell(1.0), Some("clase"))?;
        let inside = g.data.iter().filter(|v| **v == 7.0).count();
        assert_eq!(inside, 100 - 4);
        // sin campo, valor 1
        let g1 = rasterize(&d, GeomKind::Polygon, Frame::Cell(1.0), None)?;
        assert!(g1.data.iter().all(|v| v.is_nan() || *v == 1.0));
        Ok(())
    }

    #[test]
    fn focal_filters_agree_with_a_hand_calculation() -> Result<()> {
        // una rampa en x: la media de una ventana 3x3 es el valor central salvo en los bordes
        let g = grid(5, 5, |c, _| c as f32);
        let m = focal(&g, Focal::Mean, 1, false, 1)?;
        assert!((m.data[2 * 5 + 2] - 2.0).abs() < 1e-6);
        assert!((m.data[2 * 5] - 0.5).abs() < 1e-6, "borde: media de 0 y 1");
        let mx = focal(&g, Focal::Max, 1, false, 1)?;
        assert_eq!(mx.data[2 * 5 + 2], 3.0);
        let rg = focal(&g, Focal::Range, 1, false, 1)?;
        assert_eq!(rg.data[2 * 5 + 2], 2.0);
        // la mayoría de una ventana con cinco 1 y cuatro 2
        let c = grid(3, 3, |x, y| if (x + y) % 2 == 0 { 1.0 } else { 2.0 });
        let mj = focal(&c, Focal::Majority, 1, false, 1)?;
        assert_eq!(mj.data[4], 1.0);
        // una celda vacía sigue vacía, y con min_valid exigente los bordes quedan vacíos
        let mut h = grid(3, 3, |_, _| 1.0);
        h.data[0] = f32::NAN;
        let m2 = focal(&h, Focal::Mean, 1, false, 9)?;
        assert!(m2.data[0].is_nan() && m2.data[1].is_nan() && m2.data[4].is_nan());
        let m3 = focal(&grid(3, 3, |_, _| 1.0), Focal::Median, 1, true, 1)?;
        assert_eq!(m3.data[4], 1.0);
        Ok(())
    }

    #[test]
    fn statistics_and_the_tabulation_of_areas() -> Result<()> {
        let g = grid(4, 1, |c, _| c as f32 + 1.0);
        let s = stats(&g, 3)?;
        assert_eq!((s.min, s.max, s.mean), (1.0, 4.0, 2.5));
        assert_eq!(s.median, 2.5);
        assert_eq!(s.histogram.2.iter().sum::<usize>(), 4);
        assert!(histogram_csv(&s).starts_with("desde,hasta"));
        let zones = grid(4, 1, |c, _| if c < 2 { 1.0 } else { 2.0 });
        let classes = grid(4, 1, |c, _| if c % 2 == 0 { 10.0 } else { 20.0 });
        let (csv, skipped) = tabulate(&zones, &classes)?;
        assert_eq!(skipped, 0);
        assert!(csv.contains("1,1,1,2"), "{csv}");
        Ok(())
    }
}
