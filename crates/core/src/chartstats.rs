//! Estadísticos para los gráficos de exploración de datos: la caja de un conjunto de valores
//! (cuartiles y bigotes de Tukey), el resumen de una columna, la rejilla de recuentos de una
//! dispersión de muchos puntos y el ajuste lineal entre dos columnas.
//!
//! El histograma de una sola variable está en [`crate::histogram`]; esto lo completa. Todo se
//! calcula de una pasada (o con una ordenación, donde hacen falta los cuartiles exactos) y los
//! valores no finitos se ignoran.

use rayon::prelude::*;

/// La caja de un conjunto de valores.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxStats {
    pub n: usize,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
    pub mean: f64,
    /// El menor y el mayor de los valores que no pasan de 1,5 rangos intercuartílicos de la caja
    /// (los bigotes de Tukey).
    pub whisker_lo: f64,
    pub whisker_hi: f64,
    /// Cuántos valores quedan más allá de los bigotes.
    pub outliers: usize,
}

/// El cuantil `p` (de 0 a 1) de valores ya ordenados, por interpolación lineal entre los dos
/// vecinos (el método 7 de Hyndman y Fan 1996, el de R y NumPy).
pub fn quantile_sorted(sorted: &[f64], p: f64) -> f64 {
    match sorted.len() {
        0 => f64::NAN,
        1 => sorted[0],
        n => {
            let h = p.clamp(0.0, 1.0) * (n - 1) as f64;
            let (lo, frac) = (h.floor() as usize, h - h.floor());
            let hi = (lo + 1).min(n - 1);
            sorted[lo] + (sorted[hi] - sorted[lo]) * frac
        }
    }
}

/// Los valores finitos de `values`, ordenados.
fn finite_sorted(values: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    v.par_sort_unstable_by(f64::total_cmp);
    v
}

/// La caja de `values`; `None` si no hay ningún valor finito.
pub fn box_stats(values: &[f64]) -> Option<BoxStats> {
    let v = finite_sorted(values);
    box_of_sorted(&v)
}

/// La caja de valores finitos ya ordenados.
pub fn box_of_sorted(v: &[f64]) -> Option<BoxStats> {
    let (&min, &max) = (v.first()?, v.last()?);
    let (q1, median, q3) = (quantile_sorted(v, 0.25), quantile_sorted(v, 0.5), quantile_sorted(v, 0.75));
    let iqr = q3 - q1;
    let (lo_fence, hi_fence) = (q1 - 1.5 * iqr, q3 + 1.5 * iqr);
    // el primero y el último dentro de las vallas: la ordenación permite buscarlos por bisección
    let first = v.partition_point(|x| *x < lo_fence);
    let last = v.partition_point(|x| *x <= hi_fence);
    let (whisker_lo, whisker_hi) = (v.get(first).copied().unwrap_or(min), v.get(last.saturating_sub(1)).copied().unwrap_or(max));
    Some(BoxStats {
        n: v.len(),
        min,
        q1,
        median,
        q3,
        max,
        mean: v.iter().sum::<f64>() / v.len() as f64,
        whisker_lo,
        whisker_hi,
        outliers: first + (v.len() - last),
    })
}

/// El resumen de una columna numérica.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnStats {
    /// Valores finitos y valores que faltan (vacíos o no numéricos).
    pub n: usize,
    pub missing: usize,
    pub sum: f64,
    pub mean: f64,
    /// Desviación típica de la muestra (dividida entre n − 1; 0 con un solo valor).
    pub std: f64,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
    /// Desviación absoluta mediana, sin escalar.
    pub mad: f64,
}

/// El resumen de `values`, contando aparte los que faltan; `None` si no hay ninguno finito.
pub fn column_stats(values: &[f64]) -> Option<ColumnStats> {
    let v = finite_sorted(values);
    let n = v.len();
    if n == 0 {
        return None;
    }
    let sum: f64 = v.iter().sum();
    let mean = sum / n as f64;
    let var = if n > 1 { v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64 } else { 0.0 };
    let median = quantile_sorted(&v, 0.5);
    let mut dev: Vec<f64> = v.iter().map(|x| (x - median).abs()).collect();
    dev.par_sort_unstable_by(f64::total_cmp);
    Some(ColumnStats {
        n,
        missing: values.len() - n,
        sum,
        mean,
        std: var.sqrt(),
        min: v[0],
        q1: quantile_sorted(&v, 0.25),
        median,
        q3: quantile_sorted(&v, 0.75),
        max: v[n - 1],
        mad: quantile_sorted(&dev, 0.5),
    })
}

/// Una rejilla de recuentos de pares (x, y).
#[derive(Clone, Debug)]
pub struct Hist2 {
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub dy: f64,
    pub nx: usize,
    pub ny: usize,
    /// `counts[iy * nx + ix]`.
    pub counts: Vec<u32>,
    pub max: u32,
    /// Pares que cayeron dentro del rango.
    pub inside: u64,
}

/// Cuenta los pares que da `xy(i)` para `i` en `0..n` en una rejilla de `nx` por `ny` celdas sobre
/// `range` (x mínima, y mínima, x máxima, y máxima). En paralelo y sin guardar los puntos, de modo
/// que cuesta lo mismo con un millón de puntos que con cien millones.
pub fn hist2d(n: usize, xy: impl Fn(usize) -> Option<(f64, f64)> + Sync, range: [f64; 4], nx: usize, ny: usize) -> Hist2 {
    let (nx, ny) = (nx.clamp(1, 2048), ny.clamp(1, 2048));
    let dx = ((range[2] - range[0]) / nx as f64).max(f64::MIN_POSITIVE);
    let dy = ((range[3] - range[1]) / ny as f64).max(f64::MIN_POSITIVE);
    let size = nx * ny;
    let counts = (0..n)
        .into_par_iter()
        .fold(
            || vec![0u32; size],
            |mut acc, i| {
                if let Some((x, y)) = xy(i)
                    && x.is_finite()
                    && y.is_finite()
                    && x >= range[0]
                    && y >= range[1]
                    && x <= range[2]
                    && y <= range[3]
                {
                    let ix = (((x - range[0]) / dx) as usize).min(nx - 1);
                    let iy = (((y - range[1]) / dy) as usize).min(ny - 1);
                    acc[iy * nx + ix] += 1;
                }
                acc
            },
        )
        .reduce(
            || vec![0u32; size],
            |mut a, b| {
                for (p, q) in a.iter_mut().zip(b) {
                    *p += q;
                }
                a
            },
        );
    let max = counts.iter().copied().max().unwrap_or(0);
    let inside = counts.iter().map(|c| u64::from(*c)).sum();
    Hist2 { x0: range[0], y0: range[1], dx, dy, nx, ny, counts, max, inside }
}

/// El ajuste por mínimos cuadrados de y sobre x y su correlación.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub n: usize,
    pub slope: f64,
    pub intercept: f64,
    /// Correlación de Pearson.
    pub r: f64,
}

impl Fit {
    pub fn r2(&self) -> f64 {
        self.r * self.r
    }
}

/// Ajusta y = a + b·x a los pares que da `xy(i)`, de una pasada y en paralelo; `None` si hay
/// menos de dos pares o x no varía.
pub fn linear_fit(n: usize, xy: impl Fn(usize) -> Option<(f64, f64)> + Sync) -> Option<Fit> {
    // sumas centradas en el primer par válido, para no perder precisión con coordenadas grandes
    let first = (0..n).find_map(|i| xy(i).filter(|p| p.0.is_finite() && p.1.is_finite()))?;
    let (cnt, sx, sy, sxx, syy, sxy) = (0..n)
        .into_par_iter()
        .fold(
            || (0usize, 0.0, 0.0, 0.0, 0.0, 0.0),
            |mut a, i| {
                if let Some((x, y)) = xy(i)
                    && x.is_finite()
                    && y.is_finite()
                {
                    let (x, y) = (x - first.0, y - first.1);
                    a.0 += 1;
                    a.1 += x;
                    a.2 += y;
                    a.3 += x * x;
                    a.4 += y * y;
                    a.5 += x * y;
                }
                a
            },
        )
        .reduce(
            || (0usize, 0.0, 0.0, 0.0, 0.0, 0.0),
            |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3 + b.3, a.4 + b.4, a.5 + b.5),
        );
    if cnt < 2 {
        return None;
    }
    let c = cnt as f64;
    let (vx, vy, cxy) = (sxx - sx * sx / c, syy - sy * sy / c, sxy - sx * sy / c);
    if vx <= 0.0 {
        return None;
    }
    let slope = cxy / vx;
    let intercept = (first.1 + sy / c) - slope * (first.0 + sx / c);
    let r = if vy > 0.0 { cxy / (vx * vy).sqrt() } else { 0.0 };
    Some(Fit { n: cnt, slope, intercept, r })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quartiles_follow_the_usual_interpolation() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert!((quantile_sorted(&v, 0.25) - 3.25).abs() < 1e-12);
        assert!((quantile_sorted(&v, 0.5) - 5.5).abs() < 1e-12);
        assert!((quantile_sorted(&v, 0.75) - 7.75).abs() < 1e-12);
        assert_eq!(quantile_sorted(&v, 0.0), 1.0);
        assert_eq!(quantile_sorted(&v, 1.0), 10.0);
        assert!(quantile_sorted(&[], 0.5).is_nan());
        assert_eq!(quantile_sorted(&[7.0], 0.9), 7.0);
    }

    #[test]
    fn a_box_puts_its_whiskers_at_the_last_values_inside_the_fences() {
        // 1..=10 y un valor lejano: queda fuera de la valla y el bigote se detiene en 10
        let mut v: Vec<f64> = (1..=10).map(f64::from).collect();
        v.push(100.0);
        v.push(f64::NAN);
        let b = box_stats(&v).unwrap();
        assert_eq!(b.n, 11);
        assert_eq!((b.min, b.max), (1.0, 100.0));
        assert_eq!(b.whisker_hi, 10.0);
        assert_eq!(b.whisker_lo, 1.0);
        assert_eq!(b.outliers, 1);
        assert!((b.median - 6.0).abs() < 1e-12);
        assert!(box_stats(&[f64::NAN]).is_none());
    }

    #[test]
    fn a_column_summary_counts_missing_values_and_uses_the_sample_deviation() {
        let c = column_stats(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0, f64::NAN, f64::INFINITY]).unwrap();
        assert_eq!((c.n, c.missing), (8, 2));
        assert!((c.mean - 5.0).abs() < 1e-12);
        assert!((c.std - (32.0f64 / 7.0).sqrt()).abs() < 1e-12);
        assert_eq!((c.min, c.max, c.sum), (2.0, 9.0, 40.0));
        assert!((c.median - 4.5).abs() < 1e-12);
        // desviaciones respecto de 4,5: 2,5 0,5 0,5 0,5 0,5 0,5 2,5 4,5 → mediana 0,5
        assert!((c.mad - 0.5).abs() < 1e-12);
        let one = column_stats(&[3.0]).unwrap();
        assert_eq!(one.std, 0.0);
    }

    #[test]
    fn the_grid_counts_every_pair_once_and_ignores_those_outside() {
        let pts = [(0.5, 0.5), (0.5, 0.6), (1.5, 1.5), (9.0, 9.0), (f64::NAN, 1.0), (2.0, 2.0)];
        let h = hist2d(pts.len(), |i| Some(pts[i]), [0.0, 0.0, 2.0, 2.0], 2, 2);
        assert_eq!(h.counts, vec![2, 0, 0, 2]);
        assert_eq!((h.max, h.inside), (2, 4));
        // el borde superior cae en la última celda
        let edge = hist2d(1, |_| Some((2.0, 2.0)), [0.0, 0.0, 2.0, 2.0], 2, 2);
        assert_eq!(edge.counts[3], 1);
    }

    #[test]
    fn the_fit_recovers_a_line_even_with_large_coordinates() {
        let n = 1000;
        let f = linear_fit(n, |i| {
            let x = 440_000.0 + i as f64;
            Some((x, 3.0 + 0.5 * (x - 440_000.0)))
        })
        .unwrap();
        assert!((f.slope - 0.5).abs() < 1e-9);
        assert!((f.intercept - (3.0 - 0.5 * 440_000.0)).abs() < 1e-3);
        assert!((f.r - 1.0).abs() < 1e-12);
        assert_eq!(f.n, n);
        // x constante: sin ajuste; un solo par: sin ajuste
        assert!(linear_fit(5, |i| Some((1.0, i as f64))).is_none());
        assert!(linear_fit(1, |_| Some((1.0, 2.0))).is_none());
        // y constante: pendiente cero y correlación cero
        let flat = linear_fit(5, |i| Some((i as f64, 4.0))).unwrap();
        assert_eq!((flat.slope, flat.r), (0.0, 0.0));
        // anticorrelación
        let down = linear_fit(10, |i| Some((i as f64, -2.0 * i as f64))).unwrap();
        assert!((down.r + 1.0).abs() < 1e-12);
    }
}
