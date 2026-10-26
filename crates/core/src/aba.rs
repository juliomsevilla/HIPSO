//! Area-based model: a linear regression of a field variable (volume, basal area, biomass…)
//! on the LiDAR metrics of the sample plots, and the map of that variable from the metrics of
//! every cell.
//!
//! Plain least squares with a forward selection of the metrics (BIC or AICc, a cap on the number
//! of variables and on their collinearity). What matters is how the result is judged:
//!
//! - **Cross-validation leaving one plot out, repeating the selection in every round.** Choosing
//!   the metrics with the same plots that are then used to judge the model flatters it
//!   (Ambroise and McLachlan, 2002); here each plot is predicted by a model that never saw it
//!   or took part in picking its variables. The share of rounds each metric was picked in, and
//!   how often the final model came out, tell how stable the choice is.
//! - The leave-one-out error of the final model with its variables fixed, from the hat matrix
//!   (Allen, 1974): the gap to the honest one is the cost of the selection.
//! - Before the fit, the normality of the response (Shapiro–Wilk) and a screening of the
//!   candidates that puts aside the metrics that repeat another (collinearity).
//! - Standard errors, t and F tests, variance inflation factors, leverage, studentized residuals
//!   and Cook's distance, and the three assumptions of the linear model: normal residuals
//!   (Shapiro–Wilk), linearity (Ramsey's RESET) and constant variance (Breusch–Pagan).
//! - On the map, cells whose metrics fall outside the range of the plots are left empty (the
//!   model has seen nothing like them), and the half-width of the 95 % prediction interval can
//!   be written next to the estimate.
//!
//! A response in logarithms is brought back with the correction `exp(s²/2)` (Baskerville, 1972).

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::grid::Grid;
use crate::{Error, Result};

// ---------------------------------------------------------------------------------------------
// distributions
// ---------------------------------------------------------------------------------------------

fn ln_gamma(x: f64) -> f64 {
    const COF: [f64; 6] = [
        76.180_091_729_471_46,
        -86.505_320_329_416_77,
        24.014_098_240_830_91,
        -1.231_739_572_450_155,
        0.120_865_097_386_617_9e-2,
        -0.539_523_938_495_3e-5,
    ];
    let mut y = x;
    let mut tmp = x + 5.5;
    tmp -= (x + 0.5) * tmp.ln();
    let mut ser = 1.000_000_000_190_015;
    for c in COF {
        y += 1.0;
        ser += c / y;
    }
    -tmp + (2.506_628_274_631_000_5 * ser / x).ln()
}

/// Continued fraction of the incomplete beta function (modified Lentz).
fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=300 {
        let m = f64::from(m);
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-14 {
            break;
        }
    }
    h
}

/// The regularized incomplete beta function `I_x(a, b)`.
fn beta_inc(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_cf(a, b, x) / a
    } else {
        1.0 - bt * beta_cf(b, a, 1.0 - x) / b
    }
}

/// Two-sided p-value of a t statistic.
fn t_pvalue(t: f64, df: f64) -> f64 {
    if !(df > 0.0) || !t.is_finite() {
        return f64::NAN;
    }
    beta_inc(df / 2.0, 0.5, df / (df + t * t))
}

/// p-value of an F statistic with `d1` and `d2` degrees of freedom.
fn f_pvalue(f: f64, d1: f64, d2: f64) -> f64 {
    if !(d1 > 0.0) || !(d2 > 0.0) || !f.is_finite() || f < 0.0 {
        return f64::NAN;
    }
    beta_inc(d2 / 2.0, d1 / 2.0, d2 / (d2 + d1 * f))
}

/// The t value leaving 2.5 % in each tail.
fn t_crit95(df: f64) -> f64 {
    if !(df > 0.0) {
        return f64::NAN;
    }
    let (mut lo, mut hi) = (0.0, 1e4);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if t_pvalue(mid, df) > 0.05 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Complementary error function (Numerical Recipes' Chebyshev fit, error below 1.2e-7).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let ans = t * poly.exp();
    if x >= 0.0 { ans } else { 2.0 - ans }
}

/// The standard normal distribution function.
fn norm_cdf(z: f64) -> f64 {
    0.5 * erfc(-z / std::f64::consts::SQRT_2)
}

/// Inverse of the standard normal distribution function (Acklam's rational approximation,
/// relative error below 1.2e-9).
fn norm_inv(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    const LOW: f64 = 0.024_25;
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    if p < LOW {
        tail((-2.0 * p.ln()).sqrt())
    } else if p <= 1.0 - LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    }
}

/// The value of a standard normal variable below which a share `p` (0 to 1) of it lies.
pub fn normal_quantile(p: f64) -> f64 {
    norm_inv(p.clamp(1e-12, 1.0 - 1e-12))
}

/// The Shapiro–Wilk statistic W and its p-value (Royston's 1995 approximation), for 3 to 5000
/// values that are not all equal.
pub fn shapiro_wilk(x: &[f64]) -> Option<(f64, f64)> {
    let n = x.len();
    if !(3..=5000).contains(&n) || x.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let mut v = x.to_vec();
    v.sort_by(f64::total_cmp);
    if !(v[n - 1] > v[0]) {
        return None;
    }
    let nf = n as f64;
    let m: Vec<f64> = (1..=n)
        .map(|i| norm_inv((i as f64 - 0.375) / (nf + 0.25)))
        .collect();
    let ssm: f64 = m.iter().map(|v| v * v).sum();
    let mut a = vec![0.0; n];
    if n == 3 {
        a[2] = 0.5f64.sqrt();
        a[0] = -a[2];
    } else {
        let u = 1.0 / nf.sqrt();
        let poly =
            |c: [f64; 6]| ((((c[5] * u + c[4]) * u + c[3]) * u + c[2]) * u + c[1]) * u + c[0];
        let an = m[n - 1] / ssm.sqrt()
            + poly([0.0, 0.221_157, -0.147_981, -2.071_19, 4.434_685, -2.706_056]);
        let lo;
        if n > 5 {
            let an1 = m[n - 2] / ssm.sqrt()
                + poly([
                    0.0, 0.042_981, -0.293_762, -1.752_461, 5.682_633, -3.582_633,
                ]);
            let phi = (ssm - 2.0 * m[n - 1].powi(2) - 2.0 * m[n - 2].powi(2))
                / (1.0 - 2.0 * an * an - 2.0 * an1 * an1);
            a[n - 2] = an1;
            a[1] = -an1;
            lo = 2;
            for i in lo..n - lo {
                a[i] = m[i] / phi.sqrt();
            }
        } else {
            let phi = (ssm - 2.0 * m[n - 1].powi(2)) / (1.0 - 2.0 * an * an);
            lo = 1;
            for i in lo..n - lo {
                a[i] = m[i] / phi.sqrt();
            }
        }
        a[n - 1] = an;
        a[0] = -an;
    }
    let mean = v.iter().sum::<f64>() / nf;
    let ss: f64 = v.iter().map(|x| (x - mean).powi(2)).sum();
    let num: f64 = a.iter().zip(&v).map(|(a, x)| a * x).sum();
    let w = (num * num / ss).min(1.0);
    let p = if n == 3 {
        let lower = 0.75f64.sqrt().asin();
        (6.0 / std::f64::consts::PI * (w.sqrt().asin() - lower)).clamp(0.0, 1.0)
    } else if n <= 11 {
        let g = -2.273 + 0.459 * nf;
        let arg = g - (1.0 - w).ln();
        if !(arg > 0.0) {
            0.0
        } else {
            let y = -arg.ln();
            let mu = 0.5440 - 0.39978 * nf + 0.025054 * nf * nf - 0.0006714 * nf.powi(3);
            let sg = (1.3822 - 0.77857 * nf + 0.062767 * nf * nf - 0.0020322 * nf.powi(3)).exp();
            1.0 - norm_cdf((y - mu) / sg)
        }
    } else {
        let l = nf.ln();
        let y = (1.0 - w).ln();
        let mu = 0.0038915 * l.powi(3) - 0.083751 * l * l - 0.31082 * l - 1.5861;
        let sg = (0.0030302 * l * l - 0.082676 * l - 0.4803).exp();
        1.0 - norm_cdf((y - mu) / sg)
    };
    Some((w, p.clamp(0.0, 1.0)))
}

/// Mean, standard deviation, skewness and excess kurtosis.
fn moments(x: &[f64]) -> (f64, f64, f64, f64) {
    let n = x.len() as f64;
    if x.len() < 2 {
        return (
            x.first().copied().unwrap_or(f64::NAN),
            f64::NAN,
            f64::NAN,
            f64::NAN,
        );
    }
    let mean = x.iter().sum::<f64>() / n;
    let m2 = x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    let m3 = x.iter().map(|v| (v - mean).powi(3)).sum::<f64>() / n;
    let m4 = x.iter().map(|v| (v - mean).powi(4)).sum::<f64>() / n;
    let sd = (m2 * n / (n - 1.0)).sqrt();
    if m2 > 0.0 {
        (mean, sd, m3 / m2.powf(1.5), m4 / (m2 * m2) - 3.0)
    } else {
        (mean, sd, f64::NAN, f64::NAN)
    }
}

/// Pearson's correlation coefficient (NaN when either has no variation).
fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx).powi(2);
        syy += (b - my).powi(2);
    }
    if sxx > 0.0 && syy > 0.0 {
        sxy / (sxx * syy).sqrt()
    } else {
        f64::NAN
    }
}

/// Ranks, ties sharing their mean rank.
fn ranks(x: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..x.len()).collect();
    order.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let mut r = vec![0.0; x.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && x[order[j + 1]] == x[order[i]] {
            j += 1;
        }
        let mean = (i + j) as f64 / 2.0 + 1.0;
        for &k in &order[i..=j] {
            r[k] = mean;
        }
        i = j + 1;
    }
    r
}

// ---------------------------------------------------------------------------------------------
// tables
// ---------------------------------------------------------------------------------------------

/// A table of text cells with a header row.
pub struct Table {
    pub names: Vec<String>,
    rows: Vec<Vec<String>>,
}

fn delimiter(head: &str) -> char {
    [('\t', 0), (';', 0), (',', 0)]
        .iter()
        .map(|(d, _)| (*d, head.matches(*d).count()))
        .max_by_key(|(_, n)| *n)
        .map_or('\t', |(d, _)| d)
}

fn clean(s: &str) -> String {
    s.trim().trim_matches('"').trim().to_string()
}

fn lookup(names: &[String], name: &str) -> Option<usize> {
    let want = name.trim().to_lowercase();
    names.iter().position(|c| c.trim().to_lowercase() == want)
}

/// A metric by its name or by the short name its column has in a shapefile (`EP90` for
/// `Elev P90`): the shortening is deterministic, so the short names are worked out again.
fn metric_position(names: &[String], name: &str) -> Option<usize> {
    lookup(names, name).or_else(|| {
        let want = name.trim();
        crate::vector::dbf_names(names)
            .iter()
            .position(|(short, _)| short.eq_ignore_ascii_case(want))
    })
}

/// The error for a metric that is not in the table, with the names that look most like it.
fn missing_metric(names: &[String], name: &str, why: &str) -> Error {
    let want = name.trim().to_lowercase();
    let shorts = crate::vector::dbf_names(names);
    let mut close: Vec<String> = names
        .iter()
        .zip(&shorts)
        .filter(|(long, (short, _))| {
            let (l, s) = (long.to_lowercase(), short.to_lowercase());
            !want.is_empty() && (l.contains(&want) || want.contains(&s) || s.contains(&want))
        })
        .map(|(long, (short, _))| format!("«{long}» ({short})"))
        .take(6)
        .collect();
    close.dedup();
    let hint = if close.is_empty() {
        String::from(
            " Usa el nombre largo de la métrica (como «Elev P90») o el corto del shapefile (EP90).",
        )
    } else {
        format!(" ¿Quizá {}?", close.join(", "))
    };
    Error::Param(format!(
        "la métrica '{name}' no está en la tabla de parcelas{why}.{hint}"
    ))
}

/// A number from text: decimal commas allowed; empty, `NA` and the -9999 of an empty cell are
/// missing (NaN).
fn number(s: &str) -> f64 {
    let t = s.trim().trim_matches('"').replace(',', ".");
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() && (v - -9999.0).abs() > 1e-9 => v,
        _ => f64::NAN,
    }
}

/// A plot name as compared between tables: `7`, `7.0` and ` 7 ` are the same plot.
fn plot_key(s: &str) -> String {
    let t = clean(s).to_lowercase();
    match t.replace(',', ".").parse::<f64>() {
        Ok(v) if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e12 => format!("{}", v as i64),
        _ => t,
    }
}

impl Table {
    /// Reads a delimited text table (tab, `;` or `,`) or the first sheet of an `.xlsx`.
    pub fn read(path: &Path) -> Result<Self> {
        let is_xlsx = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"));
        let is_shp = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("shp") || e.eq_ignore_ascii_case("dbf"));
        let grid: Vec<Vec<String>> = if is_xlsx {
            crate::xlsx::read(path)?
        } else if is_shp {
            // the table of a layer: the plots with their metrics, as the plots tool writes it
            crate::check_vector_format(path)?;
            let (names, rows) = crate::vector::attribute_table(path)?;
            std::iter::once(names).chain(rows).collect()
        } else {
            let bytes = std::fs::read(path).map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "{}: {e}{}",
                        path.display(),
                        crate::cloud_hint(&e.to_string())
                    ),
                ))
            })?;
            let text = String::from_utf8(bytes.clone())
                .unwrap_or_else(|_| bytes.iter().map(|&b| char::from(b)).collect());
            let text = text.trim_start_matches('\u{feff}');
            let d = delimiter(text.lines().next().unwrap_or(""));
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.split(d).map(clean).collect())
                .collect()
        };
        let mut it = grid.into_iter();
        let head = it
            .next()
            .ok_or_else(|| Error::Format(format!("{} está vacío", path.display())))?;
        let mut names: Vec<String> = Vec::new();
        for h in head {
            let mut n = clean(&h);
            let base = n.clone();
            let mut k = 2;
            while names.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
                n = format!("{base}#{k}");
                k += 1;
            }
            names.push(n);
        }
        let rows: Vec<Vec<String>> = it.filter(|r| r.iter().any(|c| !c.is_empty())).collect();
        Ok(Self { names, rows })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        lookup(&self.names, name)
    }

    fn text(&self, row: usize, col: usize) -> &str {
        self.rows
            .get(row)
            .and_then(|r| r.get(col))
            .map_or("", String::as_str)
    }
}

/// The header of a delimited text file, without reading the rest.
fn header_of(path: &Path) -> Result<Vec<String>> {
    let f = std::fs::File::open(path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!(
                "{}: {e}{}",
                path.display(),
                crate::cloud_hint(&e.to_string())
            ),
        ))
    })?;
    let mut head = String::new();
    BufReader::new(f).read_line(&mut head)?;
    let head = head.trim_end().trim_start_matches('\u{feff}');
    let d = delimiter(head);
    Ok(head.split(d).map(clean).collect())
}

/// The column names of a table, for the user to choose from.
pub fn columns(path: &Path) -> Result<Vec<String>> {
    Ok(Table::read(path)?.names)
}

// ---------------------------------------------------------------------------------------------
// least squares
// ---------------------------------------------------------------------------------------------

/// Whether a column of the plot table can be a predictor on its own account: not an identifier or
/// a position, and not an absolute count (a count depends on the size of the plot, so a model
/// fitted on plots would not carry over to cells of another size).
fn is_candidate_name(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    let n = n.split('#').next().unwrap_or("");
    const SKIP: [&str; 11] = [
        "parcela", "area", "row", "col", "fila", "columna", "center x", "center y", "date",
        "captura", "id",
    ];
    if SKIP.contains(&n) {
        return false;
    }
    !(n.contains("return count")
        || n.contains("returns count")
        || n.starts_with("first returns above")
        || n.starts_with("all returns above")
        || n.starts_with("total first returns")
        || n.starts_with("total all returns")
        || n.ends_with(" count"))
}

/// A least-squares fit of `y` on `k` predictors, worked in the standardized predictors: the
/// correlation matrix of the predictors is inverted, so its diagonal is the variance inflation
/// factors, and the leverage and the prediction variance follow from the same inverse.
#[derive(Clone)]
struct Ols {
    n: usize,
    means: Vec<f64>,
    /// Square roots of the sums of squares about the mean.
    norms: Vec<f64>,
    ybar: f64,
    /// Slopes of the standardized predictors.
    b: Vec<f64>,
    /// Inverse of the correlation matrix, row-major `k × k`.
    rinv: Vec<f64>,
    sse: f64,
    sst: f64,
}

/// Inverse of a small symmetric matrix by Gauss–Jordan; None if (near) singular.
fn invert(mut a: Vec<f64>, k: usize) -> Option<Vec<f64>> {
    let mut inv = vec![0.0; k * k];
    for i in 0..k {
        inv[i * k + i] = 1.0;
    }
    for c in 0..k {
        let p = (c..k).max_by(|&i, &j| a[i * k + c].abs().total_cmp(&a[j * k + c].abs()))?;
        if a[p * k + c].abs() < 1e-9 {
            return None;
        }
        if p != c {
            for j in 0..k {
                a.swap(p * k + j, c * k + j);
                inv.swap(p * k + j, c * k + j);
            }
        }
        let d = a[c * k + c];
        for j in 0..k {
            a[c * k + j] /= d;
            inv[c * k + j] /= d;
        }
        for r in 0..k {
            if r != c {
                let f = a[r * k + c];
                if f != 0.0 {
                    for j in 0..k {
                        a[r * k + j] -= f * a[c * k + j];
                        inv[r * k + j] -= f * inv[c * k + j];
                    }
                }
            }
        }
    }
    Some(inv)
}

fn ols(cols: &[&[f64]], y: &[f64]) -> Option<Ols> {
    let n = y.len();
    let k = cols.len();
    if n < k + 2 {
        return None;
    }
    let nf = n as f64;
    let ybar = y.iter().sum::<f64>() / nf;
    let sst: f64 = y.iter().map(|v| (v - ybar).powi(2)).sum();
    let mut means = Vec::with_capacity(k);
    let mut norms = Vec::with_capacity(k);
    let mut z: Vec<Vec<f64>> = Vec::with_capacity(k);
    for c in cols {
        if c.len() != n {
            return None;
        }
        let m = c.iter().sum::<f64>() / nf;
        let s = c.iter().map(|v| (v - m).powi(2)).sum::<f64>().sqrt();
        if !(s > 1e-12 * (m.abs() + 1.0)) {
            return None;
        }
        z.push(c.iter().map(|v| (v - m) / s).collect());
        means.push(m);
        norms.push(s);
    }
    let mut g = vec![0.0; k * k];
    for i in 0..k {
        for j in i..k {
            let v: f64 = z[i].iter().zip(&z[j]).map(|(a, b)| a * b).sum();
            g[i * k + j] = v;
            g[j * k + i] = v;
        }
    }
    let rinv = invert(g, k)?;
    let zy: Vec<f64> = z
        .iter()
        .map(|c| c.iter().zip(y).map(|(a, b)| a * (b - ybar)).sum())
        .collect();
    let b: Vec<f64> = (0..k)
        .map(|i| (0..k).map(|j| rinv[i * k + j] * zy[j]).sum())
        .collect();
    let mut sse = 0.0;
    for r in 0..n {
        let fit = ybar + (0..k).map(|j| b[j] * z[j][r]).sum::<f64>();
        sse += (y[r] - fit).powi(2);
    }
    Some(Ols {
        n,
        means,
        norms,
        ybar,
        b,
        rinv,
        sse,
        sst,
    })
}

impl Ols {
    fn k(&self) -> usize {
        self.b.len()
    }

    fn df(&self) -> f64 {
        self.n as f64 - self.k() as f64 - 1.0
    }

    /// Residual variance.
    fn s2(&self) -> f64 {
        if self.df() > 0.0 {
            self.sse / self.df()
        } else {
            f64::NAN
        }
    }

    fn z(&self, x: &[f64]) -> Vec<f64> {
        (0..self.k())
            .map(|j| (x[j] - self.means[j]) / self.norms[j])
            .collect()
    }

    fn predict(&self, x: &[f64]) -> f64 {
        let z = self.z(x);
        self.ybar + self.b.iter().zip(&z).map(|(b, z)| b * z).sum::<f64>()
    }

    /// Leverage of a point: 1/n plus its distance in the standardized predictors.
    fn leverage(&self, x: &[f64]) -> f64 {
        let k = self.k();
        let z = self.z(x);
        let mut q = 0.0;
        for i in 0..k {
            for j in 0..k {
                q += z[i] * self.rinv[i * k + j] * z[j];
            }
        }
        1.0 / self.n as f64 + q
    }

    fn vif(&self, j: usize) -> f64 {
        self.rinv[j * self.k() + j]
    }

    fn max_vif(&self) -> f64 {
        (0..self.k()).map(|j| self.vif(j)).fold(1.0, f64::max)
    }

    /// Slopes in the units of the predictors, and the intercept.
    fn slopes(&self) -> (f64, Vec<f64>) {
        let s: Vec<f64> = self.b.iter().zip(&self.norms).map(|(b, n)| b / n).collect();
        let a = self.ybar - s.iter().zip(&self.means).map(|(s, m)| s * m).sum::<f64>();
        (a, s)
    }

    /// Standard errors of the intercept and of the slopes.
    fn std_errors(&self) -> Vec<f64> {
        let k = self.k();
        let s2 = self.s2();
        let u: Vec<f64> = (0..k).map(|j| self.means[j] / self.norms[j]).collect();
        let mut var0 = 1.0 / self.n as f64;
        for i in 0..k {
            for j in 0..k {
                var0 += u[i] * u[j] * self.rinv[i * k + j];
            }
        }
        let mut se = vec![(s2 * var0).max(0.0).sqrt()];
        se.extend((0..k).map(|j| (s2 * self.rinv[j * k + j]).max(0.0).sqrt() / self.norms[j]));
        se
    }

    fn score(&self, crit: Criterion) -> f64 {
        let n = self.n as f64;
        let p = self.k() as f64 + 1.0;
        let base = n * (self.sse.max(1e-300) / n).ln();
        match crit {
            Criterion::Bic => base + n.ln() * p,
            Criterion::Aicc => {
                if n - p - 1.0 > 0.0 {
                    base + 2.0 * p + 2.0 * p * (p + 1.0) / (n - p - 1.0)
                } else {
                    f64::INFINITY
                }
            }
        }
    }
}

/// How the metrics are chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Criterion {
    /// Bayesian information criterion: the strictest, the best against over-fitting.
    Bic,
    /// Akaike's criterion with the correction for few plots.
    Aicc,
}

impl Criterion {
    pub fn name(self) -> &'static str {
        match self {
            Criterion::Bic => "BIC",
            Criterion::Aicc => "AICc",
        }
    }
}

/// A transformation of the variables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    None,
    /// The logarithm of the response.
    LogResponse,
    /// The logarithm of the response and of the metrics (a power law).
    LogBoth,
}

impl Transform {
    fn log_y(self) -> bool {
        self != Transform::None
    }

    fn log_x(self) -> bool {
        self == Transform::LogBoth
    }
}

/// Forward selection with a backward pass: the indices (into `cands`) of the chosen columns.
fn select(
    cands: &[Vec<f64>],
    pool: &[usize],
    y: &[f64],
    crit: Criterion,
    max_vars: usize,
    max_vif: f64,
) -> Vec<usize> {
    let mut chosen: Vec<usize> = Vec::new();
    let Some(null) = ols(&[], y) else {
        return chosen;
    };
    let mut best = null.score(crit);
    let fit = |set: &[usize]| {
        let cols: Vec<&[f64]> = set.iter().map(|&j| cands[j].as_slice()).collect();
        ols(&cols, y)
    };
    while chosen.len() < max_vars {
        let mut pick: Option<(usize, f64)> = None;
        for &c in pool {
            if chosen.contains(&c) {
                continue;
            }
            let mut set = chosen.clone();
            set.push(c);
            let Some(m) = fit(&set) else { continue };
            if m.max_vif() > max_vif {
                continue;
            }
            let s = m.score(crit);
            if pick.is_none_or(|(_, b)| s < b) {
                pick = Some((c, s));
            }
        }
        match pick {
            Some((c, s)) if s < best - 1e-9 => {
                chosen.push(c);
                best = s;
            }
            _ => break,
        }
    }
    loop {
        let mut dropped = None;
        if chosen.len() > 1 {
            for pos in 0..chosen.len() {
                let mut set = chosen.clone();
                set.remove(pos);
                if let Some(m) = fit(&set) {
                    let s = m.score(crit);
                    if s < best - 1e-9 {
                        best = s;
                        dropped = Some(pos);
                        break;
                    }
                }
            }
        }
        match dropped {
            Some(pos) => {
                chosen.remove(pos);
            }
            None => break,
        }
    }
    chosen
}

/// Takes the candidates in order of their correlation with the response and keeps each one only
/// if its correlation with every one already kept is at most `max_corr`: of a group of metrics
/// that say the same, the one most related to the response stays. Returns the kept indices (most
/// related first) and, for each dropped one, the kept one it repeats and their correlation.
fn prune(cands: &[Vec<f64>], y: &[f64], max_corr: f64) -> (Vec<usize>, Vec<(usize, usize, f64)>) {
    let mut order: Vec<(usize, f64)> = cands
        .iter()
        .enumerate()
        .map(|(j, c)| (j, pearson(c, y)))
        .filter(|(_, r)| r.is_finite())
        .collect();
    order.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    if !(max_corr < 1.0) {
        return (order.into_iter().map(|(j, _)| j).collect(), Vec::new());
    }
    let mut kept: Vec<usize> = Vec::new();
    let mut dropped = Vec::new();
    for (j, _) in order {
        let twin = kept
            .iter()
            .map(|&k| (k, pearson(&cands[j], &cands[k])))
            .find(|(_, r)| r.abs() > max_corr);
        match twin {
            Some((k, r)) => dropped.push((j, k, r)),
            None => kept.push(j),
        }
    }
    (kept, dropped)
}

// ---------------------------------------------------------------------------------------------
// the model
// ---------------------------------------------------------------------------------------------

/// What to fit.
#[derive(Clone, Debug)]
pub struct Spec {
    pub transform: Transform,
    pub criterion: Criterion,
    /// Metrics to use; empty: chosen automatically among all the candidates.
    pub predictors: Vec<String>,
    /// Most metrics the automatic choice may keep (0: one per ten plots, from two to five).
    pub max_vars: usize,
    /// Largest variance inflation factor the automatic choice accepts.
    pub max_vif: f64,
    /// Metrics the automatic choice may use; empty: all of the table.
    pub candidates: Vec<String>,
    /// Two candidates correlated more than this (in absolute value) say the same thing: only the
    /// one more related to the response is kept (1 or more: none is dropped).
    pub max_corr: f64,
    /// Offer the intensity metrics to the automatic choice (the intensity of most surveys is
    /// not calibrated between flight lines, so they are left out unless asked for).
    pub intensity: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            transform: Transform::None,
            criterion: Criterion::Bic,
            predictors: Vec::new(),
            max_vars: 0,
            max_vif: 5.0,
            candidates: Vec::new(),
            max_corr: 0.8,
            intensity: false,
        }
    }
}

/// One coefficient of the fitted model.
#[derive(Clone, Debug)]
pub struct Coef {
    pub name: String,
    pub value: f64,
    pub se: f64,
    pub t: f64,
    pub p: f64,
    /// NaN for the intercept.
    pub vif: f64,
}

/// The fitted model, with what is needed to predict.
#[derive(Clone)]
pub struct Model {
    pub response: String,
    pub transform: Transform,
    /// Names of the metrics, as in the tables.
    pub vars: Vec<String>,
    pub coefs: Vec<Coef>,
    /// Range of each metric among the plots.
    pub ranges: Vec<(f64, f64)>,
    nonneg: bool,
    fit: Ols,
}

impl Model {
    /// The estimate and the half-width of its 95 % prediction interval, for the metrics `x` (in
    /// the order of `vars`, untransformed). None when a metric cannot be used (a log of zero).
    pub fn predict(&self, x: &[f64]) -> Option<(f64, f64)> {
        let mut v = Vec::with_capacity(x.len());
        for &a in x {
            if !a.is_finite() || (self.transform.log_x() && a <= 0.0) {
                return None;
            }
            v.push(if self.transform.log_x() { a.ln() } else { a });
        }
        let yt = self.fit.predict(&v);
        let sp = (self.fit.s2() * (1.0 + self.fit.leverage(&v))).sqrt();
        let t = t_crit95(self.fit.df());
        if self.transform.log_y() {
            let est = (yt + self.fit.s2() / 2.0).exp();
            let (lo, hi) = ((yt - t * sp).exp(), (yt + t * sp).exp());
            Some((est, (hi - lo) / 2.0))
        } else {
            let est = if self.nonneg { yt.max(0.0) } else { yt };
            Some((est, t * sp))
        }
    }

    /// Whether `x` lies inside the range of the plots in every metric.
    pub fn inside(&self, x: &[f64]) -> bool {
        x.iter()
            .zip(&self.ranges)
            .all(|(a, (lo, hi))| *a >= *lo && *a <= *hi)
    }

    /// The equation as text.
    pub fn equation(&self) -> String {
        let mut s = format!("{} = {}", self.response_label(), fmt_g(self.coefs[0].value));
        for c in &self.coefs[1..] {
            let _ = write!(
                s,
                " {} {}·{}",
                if c.value < 0.0 { '−' } else { '+' },
                fmt_g(c.value.abs()),
                c.name
            );
        }
        s
    }

    fn response_label(&self) -> String {
        if self.transform.log_y() {
            format!("ln({})", self.response)
        } else {
            self.response.clone()
        }
    }
}

fn fmt_g(v: f64) -> String {
    if v.abs() < 1e-9 {
        return "0".into();
    }
    let a = v.abs();
    if (0.001..100000.0).contains(&a) {
        let digits = (3 - a.log10().floor() as i32).clamp(0, 8) as usize;
        format!("{v:.digits$}")
    } else {
        format!("{v:.3e}")
    }
}

/// Statistics of the errors of a set of predictions, in the units of the response.
#[derive(Clone, Copy, Debug, Default)]
pub struct Errors {
    pub rmse: f64,
    /// RMSE as a percentage of the mean of the observations.
    pub rrmse: f64,
    pub bias: f64,
    pub mae: f64,
    /// 1 − Σ errors² / Σ (observation − mean)².
    pub r2: f64,
}

fn errors(obs: &[f64], pred: &[f64]) -> Errors {
    let n = obs.len() as f64;
    if obs.is_empty() {
        return Errors::default();
    }
    let mean = obs.iter().sum::<f64>() / n;
    let sse: f64 = obs.iter().zip(pred).map(|(o, p)| (p - o).powi(2)).sum();
    let sst: f64 = obs.iter().map(|o| (o - mean).powi(2)).sum();
    let rmse = (sse / n).sqrt();
    Errors {
        rmse,
        rrmse: if mean != 0.0 {
            100.0 * rmse / mean.abs()
        } else {
            f64::NAN
        },
        bias: obs.iter().zip(pred).map(|(o, p)| p - o).sum::<f64>() / n,
        mae: obs
            .iter()
            .zip(pred)
            .map(|(o, p)| (p - o).abs())
            .sum::<f64>()
            / n,
        r2: if sst > 0.0 { 1.0 - sse / sst } else { f64::NAN },
    }
}

/// Everything known about one plot after the fit.
#[derive(Clone, Debug)]
pub struct PlotRow {
    pub key: String,
    pub observed: f64,
    pub fitted: f64,
    /// Predicted by the cross-validation: the plot, and its variable choice, left out.
    pub cv: f64,
    pub leverage: f64,
    /// Residual over its own standard error (internally studentized).
    pub student: f64,
    pub cook: f64,
}

/// The result of a fit.
pub struct Fitted {
    pub model: Model,
    pub plots: Vec<PlotRow>,
    pub fit_errors: Errors,
    pub cv_errors: Errors,
    /// Leave-one-out with the variables of the final model kept fixed.
    pub loo_fixed: Errors,
    pub r2_model: f64,
    pub r2_adj: f64,
    pub f_p: f64,
    pub bp_p: f64,
    /// Ramsey's RESET test of the linearity: p-value.
    pub reset_p: f64,
    /// Shapiro–Wilk W and p-value of the residuals.
    pub resid_normal: (f64, f64),
    /// The analysis before the fit.
    pub screening: Screening,
    /// For each candidate metric, the share of cross-validation rounds that kept it.
    pub frequency: Vec<(String, f64)>,
    /// Share of rounds that ended with exactly the final set of metrics.
    pub same_model: f64,
    pub n_candidates: usize,
    pub automatic: bool,
    pub max_vars: usize,
    pub spec: Spec,
    pub notes: Vec<String>,
}

/// The data of a fit: plot names, response, and the candidate metrics.
pub struct Data {
    pub response: String,
    pub keys: Vec<String>,
    pub y: Vec<f64>,
    pub names: Vec<String>,
    pub x: Vec<Vec<f64>>,
}

fn back(yt: f64, s2: f64, spec: &Spec) -> f64 {
    if spec.transform.log_y() {
        (yt + s2 / 2.0).exp()
    } else {
        yt
    }
}

/// The columns of a fit as the model sees them: the metrics (with their logarithm when asked
/// for) and the response.
struct Prepared {
    names: Vec<String>,
    /// The metrics as they are in the table.
    raw: Vec<Vec<f64>>,
    cands: Vec<Vec<f64>>,
    yt: Vec<f64>,
    notes: Vec<String>,
}

fn prepare(data: &Data, spec: &Spec) -> Prepared {
    let mut notes: Vec<String> = Vec::new();
    let mut names = Vec::new();
    let mut raw = Vec::new();
    let mut cands = Vec::new();
    let mut dropped_log = 0usize;
    for (name, col) in data.names.iter().zip(&data.x) {
        if spec.transform.log_x() {
            if col.iter().any(|v| *v <= 0.0) {
                dropped_log += 1;
                continue;
            }
            cands.push(col.iter().map(|v| v.ln()).collect());
        } else {
            cands.push(col.clone());
        }
        names.push(name.clone());
        raw.push(col.clone());
    }
    if dropped_log > 0 {
        notes.push(format!(
            "{dropped_log} métricas con valores cero o negativos se apartaron porque se pidió el logaritmo de las métricas."
        ));
    }
    let yt = if spec.transform.log_y() {
        data.y.iter().map(|v| v.ln()).collect()
    } else {
        data.y.clone()
    };
    Prepared {
        names,
        raw,
        cands,
        yt,
        notes,
    }
}

/// How normal a variable is: its moments and the Shapiro–Wilk test.
#[derive(Clone, Debug)]
pub struct Normality {
    pub n: usize,
    pub mean: f64,
    pub sd: f64,
    pub skew: f64,
    pub kurt: f64,
    pub w: f64,
    /// p-value of the test; below 0.05 the variable is not normal.
    pub p: f64,
}

fn normality(x: &[f64]) -> Normality {
    let (mean, sd, skew, kurt) = moments(x);
    let (w, p) = shapiro_wilk(x).unwrap_or((f64::NAN, f64::NAN));
    Normality {
        n: x.len(),
        mean,
        sd,
        skew,
        kurt,
        w,
        p,
    }
}

/// What the screening says about one candidate metric.
#[derive(Clone, Debug)]
pub struct VarInfo {
    pub name: String,
    /// Pearson and Spearman correlation with the response.
    pub r: f64,
    pub rho: f64,
    /// Shapiro–Wilk p-value and skewness of the metric.
    pub normal_p: f64,
    pub skew: f64,
    /// When dropped: the metric it repeats and their correlation.
    pub dropped_by: Option<(String, f64)>,
}

/// The analysis before the fit: is the response normal, and which candidates are worth offering.
#[derive(Clone, Debug)]
pub struct Screening {
    pub response: Normality,
    /// The logarithm of the response, when it is positive.
    pub response_ln: Option<Normality>,
    /// Candidates, the ones kept first, each group by correlation with the response.
    pub vars: Vec<VarInfo>,
    pub kept: usize,
    pub max_corr: f64,
    pub advice: Vec<String>,
}

fn screen_prepared(data: &Data, spec: &Spec, p: &Prepared) -> Screening {
    let (kept, dropped) = prune(&p.cands, &p.yt, spec.max_corr);
    let ry = ranks(&p.yt);
    let mut vars: Vec<VarInfo> = Vec::new();
    let info = |j: usize, by: Option<(usize, f64)>| {
        let c = &p.cands[j];
        let n = normality(c);
        VarInfo {
            name: p.names[j].clone(),
            r: pearson(c, &p.yt),
            rho: pearson(&ranks(c), &ry),
            normal_p: n.p,
            skew: n.skew,
            dropped_by: by.map(|(k, r)| (p.names[k].clone(), r)),
        }
    };
    for &j in &kept {
        vars.push(info(j, None));
    }
    let n_kept = vars.len();
    let mut rest: Vec<VarInfo> = dropped
        .iter()
        .map(|&(j, k, r)| info(j, Some((k, r))))
        .collect();
    rest.sort_by(|a, b| b.r.abs().total_cmp(&a.r.abs()));
    vars.extend(rest);
    let response = normality(&data.y);
    let positive = data.y.iter().all(|v| *v > 0.0);
    let response_ln = positive.then(|| {
        let l: Vec<f64> = data.y.iter().map(|v| v.ln()).collect();
        normality(&l)
    });
    let mut advice = Vec::new();
    let tested = |n: &Normality| n.p.is_finite();
    if tested(&response) && !spec.transform.log_y() && response.p < 0.05 {
        match &response_ln {
            Some(l) if tested(l) && l.p >= 0.05 => advice.push(format!(
                "La variable a estimar no es normal (Shapiro–Wilk p = {:.3}) pero su logaritmo sí (p = {:.3}): usa la transformación logarítmica.",
                response.p, l.p
            )),
            Some(l) if tested(l) && l.p > response.p => advice.push(format!(
                "La variable a estimar no es normal (p = {:.3}); con el logaritmo mejora (p = {:.3}) sin llegar a serlo. Prueba la transformación logarítmica y comprueba los residuos.",
                response.p, l.p
            )),
            _ => advice.push(format!(
                "La variable a estimar no es normal (Shapiro–Wilk p = {:.3}) y el logaritmo no lo arregla. Una regresión lineal pide residuos normales: comprueba los del modelo y ten cuidado con las parcelas extremas.",
                response.p
            )),
        }
    }
    if spec.transform.log_y()
        && let Some(l) = response_ln.as_ref().filter(|l| tested(l) && l.p < 0.05)
    {
        advice.push(format!(
            "Ni siquiera el logaritmo de la variable es normal (p = {:.3}): comprueba los residuos del modelo.",
            l.p
        ));
    }
    Screening {
        response,
        response_ln,
        vars,
        kept: n_kept,
        max_corr: spec.max_corr,
        advice,
    }
}

/// The analysis before the fit, for the user to choose the candidates with.
pub fn screen(data: &Data, spec: &Spec) -> Screening {
    screen_prepared(data, spec, &prepare(data, spec))
}

/// Fits the model to `data` and judges it.
pub fn fit(data: &Data, spec: &Spec) -> Result<Fitted> {
    let n = data.y.len();
    if n < 6 {
        return Err(Error::Param(format!(
            "hay {n} parcelas con dato de campo y métricas: hacen falta al menos 6 para ajustar un modelo (y bastantes más para fiarse de él)"
        )));
    }
    if spec.transform.log_y() && data.y.iter().any(|v| *v <= 0.0) {
        return Err(Error::Param(
            "la variable a estimar tiene valores cero o negativos: no se puede usar el logaritmo"
                .into(),
        ));
    }
    let Prepared {
        names,
        raw,
        cands,
        yt,
        notes,
    } = prepare(data, spec);
    let automatic = spec.predictors.is_empty();
    let max_vars = if spec.max_vars > 0 {
        spec.max_vars
    } else {
        (n / 10).clamp(2, 5)
    }
    .min(n.saturating_sub(3));
    let fixed: Vec<usize> = if automatic {
        Vec::new()
    } else {
        spec.predictors
            .iter()
            .map(|p| {
                metric_position(&names, p)
                    .ok_or_else(|| missing_metric(&names, p, " (o tiene valores no válidos)"))
            })
            .collect::<Result<_>>()?
    };
    if !automatic && fixed.len() + 3 > n {
        return Err(Error::Param(format!(
            "{} métricas son demasiadas para {n} parcelas",
            fixed.len()
        )));
    }
    let choose = |c: &[Vec<f64>], y: &[f64]| -> Vec<usize> {
        if automatic {
            // the metrics that repeat another are put aside, in every round of the validation
            let (pool, _) = prune(c, y, spec.max_corr);
            select(c, &pool, y, spec.criterion, max_vars, spec.max_vif)
        } else {
            fixed.clone()
        }
    };
    let chosen = choose(&cands, &yt);
    if chosen.is_empty() {
        return Err(Error::Param(
            "ninguna métrica mejora a un modelo sin variables (la media): con estas parcelas no se ve relación entre el LiDAR y la variable de campo".into(),
        ));
    }
    let cols: Vec<&[f64]> = chosen.iter().map(|&j| cands[j].as_slice()).collect();
    let m = ols(&cols, &yt).ok_or_else(|| {
        let worst = (0..chosen.len())
            .flat_map(|a| (a + 1..chosen.len()).map(move |b| (a, b)))
            .map(|(a, b)| (a, b, pearson(&cands[chosen[a]], &cands[chosen[b]])))
            .filter(|x| x.2.is_finite())
            .max_by(|x, y| x.2.abs().total_cmp(&y.2.abs()));
        Error::Param(match worst {
            Some((a, b, r)) => format!(
                "las métricas elegidas son casi idénticas entre sí (colinealidad): «{}» y «{}» tienen una correlación de {r:.3}. Deja solo una de cada grupo, o deja que la herramienta elija entre las candidatas",
                names[chosen[a]], names[chosen[b]]
            ),
            None => "las métricas elegidas son casi idénticas entre sí (colinealidad): elige otras".into(),
        })
    })?;
    let k = m.k();
    if m.df() < 1.0 {
        return Err(Error::Param(
            "no quedan grados de libertad: menos métricas o más parcelas".into(),
        ));
    }
    let s2 = m.s2();
    // coefficients
    let (a, slopes) = m.slopes();
    let se = m.std_errors();
    let df = m.df();
    let label = |j: usize| {
        if spec.transform.log_x() {
            format!("ln({})", names[j])
        } else {
            names[j].clone()
        }
    };
    let mut coefs = vec![Coef {
        name: "(constante)".into(),
        value: a,
        se: se[0],
        t: a / se[0],
        p: t_pvalue(a / se[0], df),
        vif: f64::NAN,
    }];
    for (i, &j) in chosen.iter().enumerate() {
        coefs.push(Coef {
            name: label(j),
            value: slopes[i],
            se: se[i + 1],
            t: slopes[i] / se[i + 1],
            p: t_pvalue(slopes[i] / se[i + 1], df),
            vif: m.vif(i),
        });
    }
    let model = Model {
        response: data.response.clone(),
        transform: spec.transform,
        vars: chosen.iter().map(|&j| names[j].clone()).collect(),
        coefs,
        ranges: chosen
            .iter()
            .map(|&j| {
                let c = &raw[j];
                (
                    c.iter().copied().fold(f64::INFINITY, f64::min),
                    c.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                )
            })
            .collect(),
        nonneg: data.y.iter().all(|v| *v >= 0.0),
        fit: m.clone(),
    };
    // fit, residuals and influence
    let row = |r: usize| -> Vec<f64> { chosen.iter().map(|&j| cands[j][r]).collect() };
    let fitted_t: Vec<f64> = (0..n).map(|r| m.predict(&row(r))).collect();
    let fitted: Vec<f64> = fitted_t.iter().map(|v| back(*v, s2, spec)).collect();
    let resid: Vec<f64> = (0..n).map(|r| yt[r] - fitted_t[r]).collect();
    let s = s2.sqrt();
    let kk = k as f64 + 1.0;
    let lever: Vec<f64> = (0..n).map(|r| m.leverage(&row(r))).collect();
    let student: Vec<f64> = (0..n)
        .map(|r| resid[r] / (s * (1.0 - lever[r]).max(1e-12).sqrt()))
        .collect();
    let cook: Vec<f64> = (0..n)
        .map(|r| student[r].powi(2) * lever[r] / (kk * (1.0 - lever[r]).max(1e-12)))
        .collect();
    // fixed-variable leave-one-out from the hat matrix
    let loo_fixed_pred: Vec<f64> = (0..n)
        .map(|r| {
            let e = resid[r] / (1.0 - lever[r]).max(1e-12);
            back(yt[r] - e, s2, spec)
        })
        .collect();
    // honest cross-validation: the choice of metrics is repeated without each plot
    let folds: Vec<(f64, Vec<usize>)> = (0..n)
        .into_par_iter()
        .map(|i| {
            let tc: Vec<Vec<f64>> = cands
                .iter()
                .map(|c| {
                    c.iter()
                        .enumerate()
                        .filter(|(r, _)| *r != i)
                        .map(|(_, v)| *v)
                        .collect()
                })
                .collect();
            let ty: Vec<f64> = yt
                .iter()
                .enumerate()
                .filter(|(r, _)| *r != i)
                .map(|(_, v)| *v)
                .collect();
            let set = choose(&tc, &ty);
            let cols: Vec<&[f64]> = set.iter().map(|&j| tc[j].as_slice()).collect();
            let model = ols(&cols, &ty).or_else(|| ols(&[], &ty));
            match model {
                Some(f) => {
                    let x: Vec<f64> = set.iter().map(|&j| cands[j][i]).collect();
                    let s2f = if f.df() > 0.0 { f.s2() } else { 0.0 };
                    (back(f.predict(&x), s2f, spec), set)
                }
                None => (f64::NAN, set),
            }
        })
        .collect();
    // a response that cannot be negative is not estimated negative (as on the map)
    let floor = data.y.iter().all(|v| *v >= 0.0) && !spec.transform.log_y();
    let cv_pred: Vec<f64> = folds
        .iter()
        .map(|f| if floor { f.0.max(0.0) } else { f.0 })
        .collect();
    let mut frequency: Vec<(String, f64)> = (0..names.len())
        .map(|j| {
            let c = folds.iter().filter(|f| f.1.contains(&j)).count();
            (names[j].clone(), c as f64 / n as f64)
        })
        .filter(|(_, f)| *f > 0.0)
        .collect();
    frequency.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut sorted_final = chosen.clone();
    sorted_final.sort_unstable();
    let same = folds
        .iter()
        .filter(|f| {
            let mut s = f.1.clone();
            s.sort_unstable();
            s == sorted_final
        })
        .count() as f64
        / n as f64;
    // tests of the residuals
    let nf = n as f64;
    let r2_model = if m.sst > 0.0 {
        1.0 - m.sse / m.sst
    } else {
        f64::NAN
    };
    let r2_adj = 1.0 - (1.0 - r2_model) * (nf - 1.0) / df;
    let f_stat = (r2_model / k as f64) / ((1.0 - r2_model) / df);
    let f_p = f_pvalue(f_stat, k as f64, df);
    let bp_p = {
        // Koenker's form: n·R² of the squared residuals on the fitted values
        let e2: Vec<f64> = resid.iter().map(|e| e * e).collect();
        let (me, mf) = (
            e2.iter().sum::<f64>() / nf,
            fitted_t.iter().sum::<f64>() / nf,
        );
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        for r in 0..n {
            sxy += (e2[r] - me) * (fitted_t[r] - mf);
            sxx += (fitted_t[r] - mf).powi(2);
            syy += (e2[r] - me).powi(2);
        }
        let r2 = if sxx > 0.0 && syy > 0.0 {
            sxy * sxy / (sxx * syy)
        } else {
            0.0
        };
        erfc((nf * r2).sqrt() / std::f64::consts::SQRT_2)
    };
    // Ramsey's RESET: the squares and cubes of the fitted values added to the model must not
    // improve it if the relation is linear
    let reset_p = {
        let (mf, sf, _, _) = moments(&fitted_t);
        let z: Vec<f64> = fitted_t.iter().map(|v| (v - mf) / sf).collect();
        let z2: Vec<f64> = z.iter().map(|v| v * v).collect();
        let z3: Vec<f64> = z.iter().map(|v| v * v * v).collect();
        let mut cols: Vec<&[f64]> = chosen.iter().map(|&j| cands[j].as_slice()).collect();
        cols.push(&z2);
        cols.push(&z3);
        match ols(&cols, &yt) {
            Some(u) if u.df() > 0.0 && u.sse > 0.0 => {
                f_pvalue(((m.sse - u.sse) / 2.0) / (u.sse / u.df()), 2.0, u.df())
            }
            _ => f64::NAN,
        }
    };
    let resid_normal = shapiro_wilk(&resid).unwrap_or((f64::NAN, f64::NAN));
    let screening = screen_prepared(
        data,
        spec,
        &Prepared {
            names: names.clone(),
            raw: Vec::new(),
            cands: cands.clone(),
            yt: yt.clone(),
            notes: Vec::new(),
        },
    );
    let plots: Vec<PlotRow> = (0..n)
        .map(|r| PlotRow {
            key: data.keys[r].clone(),
            observed: data.y[r],
            fitted: fitted[r],
            cv: cv_pred[r],
            leverage: lever[r],
            student: student[r],
            cook: cook[r],
        })
        .collect();
    let valid: Vec<usize> = (0..n).filter(|&r| cv_pred[r].is_finite()).collect();
    let cv_errors = errors(
        &valid.iter().map(|&r| data.y[r]).collect::<Vec<_>>(),
        &valid.iter().map(|&r| cv_pred[r]).collect::<Vec<_>>(),
    );
    let result = Fitted {
        fit_errors: errors(&data.y, &fitted),
        cv_errors,
        loo_fixed: errors(&data.y, &loo_fixed_pred),
        model,
        plots,
        r2_model,
        r2_adj,
        f_p,
        bp_p,
        reset_p,
        resid_normal,
        screening,
        frequency,
        same_model: same,
        n_candidates: names.len(),
        automatic,
        max_vars,
        spec: spec.clone(),
        notes,
    };
    Ok(result)
}

/// The analysis before the fit as text.
pub fn screening_text(
    sc: &Screening,
    response: &str,
    automatic: bool,
    n_indicated: usize,
) -> String {
    let mut s = String::new();
    let norm_line = |n: &Normality| {
        if n.p.is_finite() {
            format!(
                "Shapiro–Wilk W = {:.3}, p = {:.3} → {}",
                n.w,
                n.p,
                if n.p < 0.05 {
                    "no es normal"
                } else {
                    "compatible con una distribución normal"
                }
            )
        } else {
            "Shapiro–Wilk no calculable (valores iguales o menos de 3)".to_string()
        }
    };
    let _ = writeln!(s, "\nANÁLISIS PREVIO DE LAS VARIABLES");
    let _ = writeln!(
        s,
        "  Variable a estimar «{}»: n = {} · media {} · desviación típica {} · asimetría {:.2} · curtosis {:.2}",
        response,
        sc.response.n,
        fmt_g(sc.response.mean),
        fmt_g(sc.response.sd),
        sc.response.skew,
        sc.response.kurt
    );
    let _ = writeln!(s, "    {}", norm_line(&sc.response));
    if let Some(l) = &sc.response_ln {
        let _ = writeln!(s, "    Con el logaritmo: {}", norm_line(l));
    }
    for a in &sc.advice {
        let _ = writeln!(s, "  ▸ {a}");
    }
    let total = sc.vars.len();
    if automatic {
        let _ = writeln!(
            s,
            "  Candidatas: {total} métricas numéricas · {} tras dejar de lado las que repiten a otra (correlación > {:.2})",
            sc.kept, sc.max_corr
        );
    } else {
        let _ = writeln!(s, "  Métricas indicadas: {}", n_indicated);
    }
    let _ = writeln!(
        s,
        "    {:<40} {:>7} {:>7} {:>9} {:>9}",
        "más relacionadas con la variable", "r", "ρ", "normal p", "asimetría"
    );
    for v in sc.vars.iter().take(sc.kept.min(8)) {
        let _ = writeln!(
            s,
            "    {:<40} {:>7.2} {:>7.2} {:>9.3} {:>9.2}",
            v.name, v.r, v.rho, v.normal_p, v.skew
        );
    }
    let dropped: Vec<String> = sc
        .vars
        .iter()
        .filter_map(|v| {
            v.dropped_by
                .as_ref()
                .map(|(by, r)| format!("{} (r = {r:.2} con {by})", v.name))
        })
        .take(5)
        .collect();
    if !dropped.is_empty() {
        let _ = writeln!(
            s,
            "    Dejadas de lado por colinealidad, las primeras: {}",
            dropped.join(" · ")
        );
    }
    let _ = writeln!(
        s,
        "  La regresión lineal exige residuos normales y no la normalidad de cada variable; se comprueban tras el ajuste. La normalidad de la variable a estimar es el primer aviso."
    );
    s
}

impl Fitted {
    /// Warnings the numbers call for.
    pub fn warnings(&self) -> Vec<String> {
        let mut w = self.notes.clone();
        let n = self.plots.len();
        let k = self.model.vars.len();
        if n < 15 {
            w.push(format!(
                "Solo {n} parcelas: con tan pocas, las cifras de ajuste y de validación cambian mucho al añadir o quitar una parcela."
            ));
        }
        if n < 10 * k {
            w.push(format!(
                "Menos de 10 parcelas por variable ({n} parcelas, {k} variables): riesgo de sobreajuste."
            ));
        }
        if self.automatic && self.n_candidates > n {
            w.push(format!(
                "Se probaron {} métricas con {n} parcelas: el ajuste y los valores p son optimistas. Fíate de la validación cruzada.",
                self.n_candidates
            ));
        } else if self.automatic {
            w.push("Los valores p de las variables elegidas por selección automática son optimistas; la validación cruzada es la medida honesta.".into());
        }
        let (r2, r2cv) = (self.fit_errors.r2, self.cv_errors.r2);
        if r2 - r2cv > 0.15 {
            w.push(format!(
                "La validación cruzada (R² {r2cv:.2}) queda muy por debajo del ajuste (R² {r2:.2}): señal de sobreajuste. {}",
                if self.automatic {
                    "Suele pasar al elegir entre muchas métricas con pocas parcelas: limita el número de variables o elige a mano las que tengan sentido forestal."
                } else {
                    "Prueba con menos variables."
                }
            ));
        }
        if r2cv < 0.3 {
            w.push("R² de validación bajo: el modelo explica poco de la variable de campo; los mapas serán orientativos como mucho.".into());
        }
        let mean = self.plots.iter().map(|p| p.observed).sum::<f64>() / n as f64;
        if mean != 0.0 && self.cv_errors.bias.abs() / mean.abs() > 0.05 {
            w.push(format!(
                "Sesgo de validación de {:+.1} % de la media: el modelo tiende a {}.",
                100.0 * self.cv_errors.bias / mean,
                if self.cv_errors.bias > 0.0 {
                    "sobrestimar"
                } else {
                    "subestimar"
                }
            ));
        }
        for c in self.model.coefs.iter().skip(1) {
            if c.vif > 5.0 {
                w.push(format!(
                    "'{}' tiene VIF {:.1}: está muy relacionada con otra variable del modelo.",
                    c.name, c.vif
                ));
            }
            if !self.automatic && c.p > 0.05 {
                w.push(format!(
                    "'{}' no es significativa (p = {:.3}).",
                    c.name, c.p
                ));
            }
        }
        if self.bp_p < 0.05 {
            w.push(format!(
                "Varianza no constante (Breusch–Pagan p = {:.3}): el error cambia con el tamaño de la parcela. {}",
                self.bp_p,
                if self.spec.transform == Transform::None { "Prueba el logaritmo de la variable." } else { "" }
            ));
        }
        if self.reset_p < 0.05 {
            w.push(format!(
                "La relación no parece lineal (RESET de Ramsey p = {:.3}): el modelo deja estructura sin explicar. {}",
                self.reset_p,
                if self.spec.transform == Transform::None {
                    "Prueba la transformación logarítmica de la variable y de las métricas."
                } else {
                    "Prueba otras métricas."
                }
            ));
        }
        if self.resid_normal.1 < 0.05 {
            w.push(format!(
                "Los residuos no son normales (Shapiro–Wilk p = {:.3}): los errores típicos, los valores p y los intervalos de predicción no son fiables. {}",
                self.resid_normal.1,
                if self.spec.transform == Transform::None {
                    "Prueba la transformación logarítmica."
                } else {
                    "Revisa las parcelas extremas."
                }
            ));
        }
        // a validation estimate far beyond what the field data ever were: the choice of metrics
        // changed with that plot, or the plot is unlike the rest
        let (_, sd, _, _) = moments(&self.plots.iter().map(|p| p.observed).collect::<Vec<_>>());
        let wild: Vec<String> = self
            .plots
            .iter()
            .filter(|p| p.cv.is_finite() && (p.cv - p.observed).abs() > 4.0 * sd)
            .map(|p| format!("{} ({})", p.key, fmt_g(p.cv)))
            .take(5)
            .collect();
        if !wild.is_empty() {
            w.push(format!(
                "La validación cruzada estima de forma disparatada {}: sin esas parcelas el modelo elige otras métricas, o sus valores se salen de lo que ve el resto. Las cifras de error de la validación están dominadas por ellas.",
                wild.join(", ")
            ));
        }
        let hot: Vec<&str> = self
            .plots
            .iter()
            .filter(|p| p.cook > 4.0 / n as f64 && p.cook > 0.5 || p.cook > 1.0)
            .map(|p| p.key.as_str())
            .collect();
        if !hot.is_empty() {
            w.push(format!(
                "Parcelas que mandan sobre el modelo (distancia de Cook alta): {}. Comprueba su dato de campo o su posición.",
                hot.join(", ")
            ));
        }
        if self.automatic && self.same_model < 0.5 {
            w.push(format!(
                "La elección de variables es inestable: el mismo modelo salió solo en el {:.0} % de las pruebas de validación.",
                100.0 * self.same_model
            ));
        }
        w
    }

    /// The plot-by-plot table as CSV.
    pub fn plots_csv(&self) -> String {
        let n = self.plots.len() as f64;
        let mut s = String::from(
            "Parcela,Observado,Ajustado,Residuo,Validacion_cruzada,Error_validacion,Palanca,Residuo_estudentizado,Cook,Aviso\n",
        );
        let k = self.model.vars.len() as f64 + 1.0;
        for p in &self.plots {
            let mut flags: Vec<&str> = Vec::new();
            if p.student.abs() > 2.5 {
                flags.push("atipica");
            }
            if p.leverage > 2.0 * k / n {
                flags.push("palanca");
            }
            if p.cook > 4.0 / n && p.cook > 0.5 || p.cook > 1.0 {
                flags.push("influyente");
            }
            let _ = writeln!(
                s,
                "{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.3},{:.4},{}",
                p.key.replace(',', " "),
                p.observed,
                p.fitted,
                p.observed - p.fitted,
                p.cv,
                p.observed - p.cv,
                p.leverage,
                p.student,
                p.cook,
                flags.join(" ")
            );
        }
        s
    }

    /// The report of the fit.
    pub fn report(&self) -> String {
        let m = &self.model;
        let n = self.plots.len();
        let mut s = String::new();
        let _ = writeln!(s, "MODELO DE TRANSICIÓN (regresión lineal múltiple)");
        let _ = writeln!(s, "Variable a estimar: {} · {n} parcelas", m.response);
        let _ = writeln!(
            s,
            "Transformación: {}",
            match self.spec.transform {
                Transform::None => "ninguna",
                Transform::LogResponse =>
                    "logaritmo de la variable (con corrección de sesgo exp(s²/2))",
                Transform::LogBoth =>
                    "logaritmo de la variable y de las métricas (corrección exp(s²/2))",
            }
        );
        if self.automatic {
            let _ = writeln!(
                s,
                "Elección de métricas: automática, {} entre {} candidatas (hasta {} variables, VIF ≤ {})",
                self.spec.criterion.name(),
                self.n_candidates,
                self.max_vars,
                self.spec.max_vif
            );
        } else {
            let _ = writeln!(
                s,
                "Elección de métricas: la indicada ({} variables)",
                m.vars.len()
            );
        }
        s.push_str(&screening_text(
            &self.screening,
            &m.response,
            self.automatic,
            m.vars.len(),
        ));
        let _ = writeln!(s, "\nMODELO\n  {}\n", m.equation());
        let _ = writeln!(
            s,
            "  {:<44} {:>11} {:>10} {:>8} {:>8} {:>6}",
            "variable", "coeficiente", "error típ.", "t", "p", "VIF"
        );
        for c in &m.coefs {
            let _ = writeln!(
                s,
                "  {:<44} {:>11} {:>10} {:>8.2} {:>8.4} {:>6}",
                c.name,
                fmt_g(c.value),
                fmt_g(c.se),
                c.t,
                c.p,
                if c.vif.is_nan() {
                    "-".into()
                } else {
                    format!("{:.1}", c.vif)
                }
            );
        }
        let e = &self.fit_errors;
        let _ = writeln!(
            s,
            "\nAJUSTE (con las mismas parcelas que lo ajustaron: optimista)"
        );
        let _ = writeln!(
            s,
            "  R² {:.3} · R² ajustado {:.3}{} · test F p = {:.4}",
            e.r2,
            self.r2_adj,
            if m.transform.log_y() {
                format!(" (del modelo en logaritmos: {:.3})", self.r2_model)
            } else {
                String::new()
            },
            self.f_p
        );
        let _ = writeln!(
            s,
            "  RMSE {} ({:.1} %) · sesgo {} · error absoluto medio {}",
            fmt_g(e.rmse),
            e.rrmse,
            fmt_g(e.bias),
            fmt_g(e.mae)
        );
        let c = &self.cv_errors;
        let _ = writeln!(
            s,
            "\nVALIDACIÓN CRUZADA, una parcela fuera cada vez{}",
            if self.automatic {
                " y repitiendo la elección de métricas"
            } else {
                ""
            }
        );
        let _ = writeln!(
            s,
            "  Cada parcela se estima con un modelo que no la vio: es la medida de error que cabe esperar en parcelas nuevas."
        );
        let _ = writeln!(
            s,
            "  RMSE {} ({:.1} %) · R² {:.3} · sesgo {} · error absoluto medio {}",
            fmt_g(c.rmse),
            c.rrmse,
            c.r2,
            fmt_g(c.bias),
            fmt_g(c.mae)
        );
        if self.automatic {
            let l = &self.loo_fixed;
            let _ = writeln!(
                s,
                "  Con las variables del modelo final ya fijadas (más optimista; la diferencia es el coste de elegirlas con los mismos datos): RMSE {} ({:.1} %) · R² {:.3}",
                fmt_g(l.rmse),
                l.rrmse,
                l.r2
            );
        }
        if self.automatic {
            let top: Vec<String> = self
                .frequency
                .iter()
                .take(8)
                .map(|(n, f)| format!("{n} {:.0} %", 100.0 * f))
                .collect();
            let _ = writeln!(
                s,
                "  Estabilidad: el modelo final salió entero en el {:.0} % de las pruebas. Métricas elegidas: {}",
                100.0 * self.same_model,
                top.join(" · ")
            );
        }
        let k = m.vars.len() as f64 + 1.0;
        let nf = n as f64;
        let hot: Vec<String> = self
            .plots
            .iter()
            .filter(|p| p.cook > 4.0 / nf && p.cook > 0.5 || p.cook > 1.0)
            .map(|p| format!("{} (Cook {:.2})", p.key, p.cook))
            .collect();
        let far: Vec<String> = self
            .plots
            .iter()
            .filter(|p| p.student.abs() > 2.5)
            .map(|p| format!("{} ({:+.1})", p.key, p.student))
            .collect();
        let lev: Vec<String> = self
            .plots
            .iter()
            .filter(|p| p.leverage > 2.0 * k / nf)
            .map(|p| format!("{} ({:.2})", p.key, p.leverage))
            .collect();
        let _ = writeln!(s, "\nDIAGNÓSTICO DE LOS RESIDUOS");
        let ok = |p: f64| p.is_nan() || p >= 0.05;
        let _ = writeln!(
            s,
            "  Supuestos de la regresión lineal: {}",
            if ok(self.resid_normal.1) && ok(self.reset_p) && ok(self.bp_p) {
                "se cumplen los tres contrastes".to_string()
            } else {
                let mut fail = Vec::new();
                if !ok(self.resid_normal.1) {
                    fail.push("residuos normales");
                }
                if !ok(self.reset_p) {
                    fail.push("linealidad");
                }
                if !ok(self.bp_p) {
                    fail.push("varianza constante");
                }
                format!("NO se cumple: {}", fail.join(", "))
            }
        );
        let _ = writeln!(
            s,
            "  Linealidad (RESET de Ramsey): p = {:.3}{}",
            self.reset_p,
            if self.reset_p < 0.05 {
                "  ← no se cumple"
            } else {
                ""
            }
        );
        let _ = writeln!(
            s,
            "  Varianza constante (Breusch–Pagan): p = {:.3}{}",
            self.bp_p,
            if self.bp_p < 0.05 {
                "  ← no se cumple"
            } else {
                ""
            }
        );
        let _ = writeln!(
            s,
            "  Residuos normales (Shapiro–Wilk): W = {:.3}, p = {:.3}{}",
            self.resid_normal.0,
            self.resid_normal.1,
            if self.resid_normal.1 < 0.05 {
                "  ← no se cumple"
            } else {
                ""
            }
        );
        let none = |v: &Vec<String>| {
            if v.is_empty() {
                "ninguna".to_string()
            } else {
                v.join(", ")
            }
        };
        let _ = writeln!(s, "  Parcelas influyentes (Cook): {}", none(&hot));
        let _ = writeln!(
            s,
            "  Parcelas atípicas (residuo estudentizado > 2,5): {}",
            none(&far)
        );
        let _ = writeln!(s, "  Parcelas de palanca alta (> 2k/n): {}", none(&lev));
        let w = self.warnings();
        if !w.is_empty() {
            let _ = writeln!(s, "\nAVISOS");
            for x in &w {
                let _ = writeln!(s, "  - {x}");
            }
        }
        s
    }
}

// ---------------------------------------------------------------------------------------------
// data
// ---------------------------------------------------------------------------------------------

/// Joins the metrics of the plots with the field table into the data of a fit; the second value
/// is a note on plots that did not match.
/// Which columns of the two tables to join, and which metrics may be candidates.
pub struct JoinSpec<'a> {
    pub plot_key: &'a str,
    pub field_key: &'a str,
    pub response: &'a str,
    /// Metrics asked for; empty: every candidate column.
    pub wanted: &'a [String],
    /// Columns of the cell table: with it, only the metrics it also has are candidates.
    pub available: Option<&'a [String]>,
    /// Metrics the automatic choice may use; empty: all.
    pub candidates: &'a [String],
    pub intensity: bool,
}

pub fn join(plots: &Table, field: &Table, j: &JoinSpec) -> Result<(Data, Vec<String>)> {
    let JoinSpec {
        plot_key: plot_key_name,
        field_key: field_key_name,
        response,
        wanted,
        available,
        candidates,
        intensity,
    } = *j;
    let kp = plots.index(plot_key_name).ok_or_else(|| {
        Error::Param(format!(
            "la tabla de métricas no tiene la columna '{plot_key_name}' que identifica la parcela (columnas: {})",
            plots.names.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
        ))
    })?;
    let kf = field.index(field_key_name).ok_or_else(|| {
        Error::Param(format!(
            "la tabla de campo no tiene la columna '{field_key_name}' que identifica la parcela (columnas: {})",
            field.names.join(", ")
        ))
    })?;
    let ky = field.index(response).ok_or_else(|| {
        Error::Param(format!(
            "la tabla de campo no tiene la columna '{response}' (columnas: {})",
            field.names.join(", ")
        ))
    })?;
    let mut by_key: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in 0..field.len() {
        let k = plot_key(field.text(r, kf));
        if k.is_empty() {
            continue;
        }
        if by_key.insert(k.clone(), r).is_some() {
            return Err(Error::Param(format!(
                "la parcela '{k}' sale en más de una fila de la tabla de campo: aquí hace falta una fila por parcela (el total o la media de la parcela, no la lista de árboles)"
            )));
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<(usize, usize)> = Vec::new();
    let mut no_field: Vec<String> = Vec::new();
    let mut no_value: Vec<String> = Vec::new();
    for r in 0..plots.len() {
        let name = clean(plots.text(r, kp));
        let k = plot_key(&name);
        if !seen.insert(k.clone()) {
            return Err(Error::Param(format!(
                "la parcela '{name}' sale dos veces en la tabla de métricas"
            )));
        }
        match by_key.get(&k) {
            None => no_field.push(name),
            Some(&fr) => {
                if number(field.text(fr, ky)).is_nan() {
                    no_value.push(name);
                } else {
                    rows.push((r, fr));
                }
            }
        }
    }
    let used: std::collections::HashSet<String> = rows
        .iter()
        .map(|&(r, _)| plot_key(plots.text(r, kp)))
        .collect();
    let unmatched_field: Vec<&String> = by_key.keys().filter(|k| !used.contains(*k)).collect();
    let mut notes = Vec::new();
    let list = |v: &[String]| {
        let mut s = v.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
        if v.len() > 6 {
            s.push_str(", …");
        }
        s
    };
    if !no_field.is_empty() {
        notes.push(format!(
            "{} parcelas de la tabla de métricas no tienen fila en la de campo y no se usan: {}.",
            no_field.len(),
            list(&no_field)
        ));
    }
    if !no_value.is_empty() {
        notes.push(format!(
            "{} parcelas no tienen valor de '{response}' en el campo y no se usan: {}.",
            no_value.len(),
            list(&no_value)
        ));
    }
    if !unmatched_field.is_empty() && no_field.is_empty() && no_value.is_empty() {
        notes.push(format!(
            "{} filas de la tabla de campo no tienen parcela con métricas (¿nombres distintos?).",
            unmatched_field.len()
        ));
    } else if !unmatched_field.is_empty() {
        notes.push(format!(
            "{} filas de la tabla de campo no encontraron su parcela en la de métricas.",
            unmatched_field.len()
        ));
    }
    if rows.is_empty() {
        return Err(Error::Param(format!(
            "ninguna parcela de la tabla de métricas coincide con la de campo (columna '{plot_key_name}' frente a '{field_key_name}'): comprueba que los nombres son los mismos"
        )));
    }
    let y: Vec<f64> = rows
        .iter()
        .map(|&(_, fr)| number(field.text(fr, ky)))
        .collect();
    let keys: Vec<String> = rows
        .iter()
        .map(|&(r, _)| clean(plots.text(r, kp)))
        .collect();
    let mut names = Vec::new();
    let mut x = Vec::new();
    let mut gone_missing = 0usize;
    let mut gone_const = 0usize;
    let mut not_in_cells = 0usize;
    let mut gone_intensity = 0usize;
    let take = |c: usize| -> Vec<f64> {
        rows.iter()
            .map(|&(r, _)| number(plots.text(r, c)))
            .collect()
    };
    if wanted.is_empty() {
        for (c, name) in plots.names.iter().enumerate() {
            if c == kp || !is_candidate_name(name) {
                continue;
            }
            if !candidates.is_empty() && lookup(candidates, name).is_none() {
                continue;
            }
            if !intensity && candidates.is_empty() && name.trim().to_lowercase().starts_with("int ")
            {
                gone_intensity += 1;
                continue;
            }
            if available.is_some_and(|a| lookup(a, name).is_none()) {
                not_in_cells += 1;
                continue;
            }
            let col = take(c);
            if col.iter().any(|v| v.is_nan()) {
                // a column of text, or with empty plots
                if col.iter().any(|v| !v.is_nan()) {
                    gone_missing += 1;
                }
                continue;
            }
            let (lo, hi) = (
                col.iter().copied().fold(f64::INFINITY, f64::min),
                col.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            );
            if !(hi - lo > 1e-9 * (lo.abs() + hi.abs() + 1e-12)) {
                gone_const += 1;
                continue;
            }
            names.push(name.clone());
            x.push(col);
        }
    } else {
        for w in wanted {
            let c = metric_position(&plots.names, w)
                .ok_or_else(|| missing_metric(&plots.names, w, ""))?;
            let col = take(c);
            if col.iter().any(|v| v.is_nan()) {
                return Err(Error::Param(format!(
                    "la métrica '{w}' tiene valores vacíos o no numéricos en alguna parcela"
                )));
            }
            names.push(plots.names[c].clone());
            x.push(col);
        }
    }
    if gone_missing > 0 {
        notes.push(format!(
            "{gone_missing} columnas con valores vacíos en alguna parcela (parcelas sin puntos) no se usan como candidatas."
        ));
    }
    if gone_intensity > 0 {
        notes.push(format!(
            "{gone_intensity} métricas de intensidad no se ofrecen como candidatas: la intensidad de casi todos los vuelos no está calibrada entre pasadas. Se pueden usar indicándolas en la lista de métricas."
        ));
    }
    if not_in_cells > 0 {
        notes.push(format!(
            "{not_in_cells} métricas de las parcelas (p. ej. las de intensidad) no están en la tabla de celdas y no se ofrecen como candidatas, para poder hacer el mapa."
        ));
    }
    if gone_const > 0 {
        notes.push(format!(
            "{gone_const} columnas constantes no se usan como candidatas."
        ));
    }
    let missing: Vec<&String> = candidates
        .iter()
        .filter(|c| lookup(&names, c).is_none())
        .collect();
    if wanted.is_empty() && !missing.is_empty() {
        return Err(Error::Param(format!(
            "las candidatas {} no están en la tabla de parcelas, o tienen valores vacíos, o no están en la tabla de celdas",
            missing
                .iter()
                .map(|s| format!("«{s}»"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if names.is_empty() {
        return Err(Error::Param(
            "la tabla de parcelas no tiene métricas numéricas que usar".into(),
        ));
    }
    Ok((
        Data {
            response: field.names[ky].clone(),
            keys,
            y,
            names,
            x,
        },
        notes,
    ))
}

// ---------------------------------------------------------------------------------------------
// the map
// ---------------------------------------------------------------------------------------------

/// What `predict_map` found.
#[derive(Clone, Debug, Default)]
pub struct MapStats {
    pub cells: usize,
    pub estimated: usize,
    pub outside_range: usize,
    /// Cells outside the range of the plots, by metric (of the cells with every metric).
    pub outside_by_metric: Vec<(String, usize)>,
    pub incomplete: usize,
    pub clamped: usize,
    pub mean: f64,
    pub total: f64,
    pub cell: f64,
}

/// Applies the model to the cell metrics table (the text output of the stand metrics tool) and
/// writes the estimate as a GeoTIFF, and optionally the half-width of the 95 % prediction
/// interval. Cells with a metric outside the range of the plots are left empty when
/// `mask_extrapolation` is on.
pub fn predict_map(
    model: &Model,
    cells_csv: &[PathBuf],
    output: &Path,
    interval: Option<&Path>,
    mask_extrapolation: bool,
    epsg: Option<u16>,
) -> Result<MapStats> {
    let mut pts: Vec<(f64, f64, Vec<f64>)> = Vec::new();
    for cells_csv in cells_csv {
        crate::check_cancel()?;
        let f = std::fs::File::open(cells_csv).map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "{}: {e}{}",
                    cells_csv.display(),
                    crate::cloud_hint(&e.to_string())
                ),
            ))
        })?;
        let mut lines = BufReader::new(f).lines();
        let Some(head) = lines.next() else { continue };
        let head = head?.trim_start_matches('\u{feff}').to_string();
        let d = delimiter(&head);
        let names: Vec<String> = head.split(d).map(clean).collect();
        let find = |n: &str| lookup(&names, n);
        let (Some(cx), Some(cy)) = (find("center X"), find("center Y")) else {
            return Err(Error::Param(format!(
                "{} no tiene las columnas 'center X' y 'center Y': usa la tabla de texto (.csv) que escribe Métricas de rodal",
                cells_csv.display()
            )));
        };
        let mut cols = Vec::new();
        let mut missing = Vec::new();
        for v in &model.vars {
            match find(v) {
                Some(c) => cols.push(c),
                None => missing.push(v.clone()),
            }
        }
        if !missing.is_empty() {
            return Err(Error::Param(format!(
                "{} no tiene las métricas {}: las parcelas y las celdas deben calcularse con el mismo corte de altura y las mismas opciones",
                cells_csv.display(),
                missing.join(", ")
            )));
        }
        for line in lines {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(d).collect();
            let g = |c: usize| f.get(c).map_or(f64::NAN, |s| number(s));
            let (x, y) = (g(cx), g(cy));
            if x.is_finite() && y.is_finite() {
                pts.push((x, y, cols.iter().map(|&c| g(c)).collect()));
            }
        }
    }
    if pts.is_empty() {
        return Err(Error::Param("la tabla de celdas no tiene filas".into()));
    }
    let gap = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        v.windows(2)
            .map(|w| w[1] - w[0])
            .fold(f64::INFINITY, f64::min)
    };
    let cell = gap(pts.iter().map(|p| p.0).collect()).min(gap(pts.iter().map(|p| p.1).collect()));
    if !cell.is_finite() || cell <= 0.0 {
        return Err(Error::Param(
            "no se pudo deducir el tamaño de celda de la tabla".into(),
        ));
    }
    let (minx, maxx) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| {
        (a.0.min(p.0), a.1.max(p.0))
    });
    let (miny, maxy) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| {
        (a.0.min(p.1), a.1.max(p.1))
    });
    let cols_n = ((maxx - minx) / cell).round() as usize + 1;
    let rows_n = ((maxy - miny) / cell).round() as usize + 1;
    let mut est = Grid::new(minx - cell / 2.0, miny - cell / 2.0, cell, cols_n, rows_n)?;
    let mut err = if interval.is_some() {
        Some(est.like())
    } else {
        None
    };
    let mut st = MapStats {
        cells: pts.len(),
        cell,
        ..MapStats::default()
    };
    let mut sum = 0.0;
    let mut by_metric = vec![0usize; model.vars.len()];
    for (x, y, v) in &pts {
        let c = ((x - minx) / cell).round() as usize;
        let r = rows_n - 1 - ((y - miny) / cell).round() as usize;
        let i = r * cols_n + c;
        if v.iter().any(|a| !a.is_finite()) {
            st.incomplete += 1;
            continue;
        }
        if !model.inside(v) {
            for (j, a) in v.iter().enumerate() {
                if *a < model.ranges[j].0 || *a > model.ranges[j].1 {
                    by_metric[j] += 1;
                }
            }
            if mask_extrapolation {
                st.outside_range += 1;
                continue;
            }
        }
        let Some((p, hw)) = model.predict(v) else {
            st.incomplete += 1;
            continue;
        };
        if model.nonneg && !model.transform.log_y() && p <= 0.0 {
            st.clamped += 1;
        }
        est.data[i] = p as f32;
        if let Some(e) = err.as_mut() {
            e.data[i] = hw as f32;
        }
        st.estimated += 1;
        sum += p;
    }
    st.outside_by_metric = model.vars.iter().cloned().zip(by_metric).collect();
    if st.estimated == 0 {
        let detail: Vec<String> = st
            .outside_by_metric
            .iter()
            .map(|(n, c)| format!("{n}: {c} celdas"))
            .collect();
        return Err(Error::Param(format!(
            "ninguna celda tiene todas sus métricas dentro del rango de las parcelas, así que el modelo no puede estimar nada ({}). Las parcelas no representan esta zona, o las métricas de las celdas y de las parcelas no se calcularon igual (mismo corte de altura, mismas clases y retornos)",
            detail.join("; ")
        )));
    }
    st.mean = sum / st.estimated as f64;
    st.total = sum;
    crate::paths::ensure_parent(output)?;
    est.write_tiff(output, epsg)?;
    if let (Some(e), Some(path)) = (err.as_ref(), interval) {
        e.write_tiff(path, epsg)?;
    }
    Ok(st)
}

// ---------------------------------------------------------------------------------------------
// the job
// ---------------------------------------------------------------------------------------------

/// A model from files.
#[derive(Clone, Debug)]
pub struct AbaJob {
    /// Metrics of the plots: the layer (`.shp`) or the text table of the plots tool.
    pub plots: PathBuf,
    /// Field table: `.csv`, `.txt` or `.xlsx`, one row per plot.
    pub field: PathBuf,
    pub response: String,
    pub plot_key: String,
    pub field_key: String,
    pub spec: Spec,
    /// Metrics of every cell (text output of the stand metrics tool): to write the map.
    pub cells: Vec<PathBuf>,
    /// The map (GeoTIFF); the report and the plot table are written next to it.
    pub output: PathBuf,
    pub interval_map: bool,
    pub mask_extrapolation: bool,
    pub epsg: Option<u16>,
}

/// The files a model writes next to its map, as `(suffix, extension)`.
pub const COMPANIONS: &[(&str, &str)] = &[
    ("modelo", "txt"),
    ("parcelas", "csv"),
    ("incertidumbre", "tif"),
];

impl AbaJob {
    pub fn report_path(&self) -> PathBuf {
        self.sibling("modelo", "txt")
    }

    pub fn plots_path(&self) -> PathBuf {
        self.sibling("parcelas", "csv")
    }

    pub fn interval_path(&self) -> PathBuf {
        self.sibling("incertidumbre", "tif")
    }

    fn sibling(&self, suffix: &str, ext: &str) -> PathBuf {
        let stem = self
            .output
            .file_stem()
            .map_or("modelo".into(), |s| s.to_string_lossy().into_owned());
        self.output.with_file_name(format!("{stem}_{suffix}.{ext}"))
    }
}

/// The analysis before the fit (normality of the response, which metrics to offer) for the files
/// of a job, with the notes about what could not be used.
pub fn analyze(job: &AbaJob) -> Result<(Screening, Vec<String>)> {
    let plots = Table::read(&job.plots)?;
    let field = Table::read(&job.field)?;
    let available = match job.cells.first() {
        Some(c) => Some(header_of(c)?),
        None => None,
    };
    let (data, notes) = join(
        &plots,
        &field,
        &JoinSpec {
            plot_key: &job.plot_key,
            field_key: &job.field_key,
            response: &job.response,
            wanted: &job.spec.predictors,
            available: available.as_deref(),
            candidates: &job.spec.candidates,
            intensity: job.spec.intensity,
        },
    )?;
    if data.y.len() < 3 {
        return Err(Error::Param("hacen falta al menos 3 parcelas".into()));
    }
    Ok((screen(&data, &job.spec), notes))
}

/// Fits, judges and (when cell metrics are given) maps. Returns the report.
pub fn run(job: &AbaJob) -> Result<String> {
    crate::check_cancel()?;
    let plots = Table::read(&job.plots)?;
    let field = Table::read(&job.field)?;
    // with a map to make, only the metrics the cell table also has can be used
    let available = match job.cells.first() {
        Some(c) => Some(header_of(c)?),
        None => None,
    };
    let (data, notes) = join(
        &plots,
        &field,
        &JoinSpec {
            plot_key: &job.plot_key,
            field_key: &job.field_key,
            response: &job.response,
            wanted: &job.spec.predictors,
            available: available.as_deref(),
            candidates: &job.spec.candidates,
            intensity: job.spec.intensity,
        },
    )?;
    let mut fitted = fit(&data, &job.spec)?;
    // notes about the join come first
    let mut all = notes;
    all.append(&mut fitted.notes);
    fitted.notes = all;
    crate::paths::ensure_parent(&job.output)?;
    let mut text = fitted.report();
    std::fs::write(job.plots_path(), fitted.plots_csv())?;
    let mut files = vec![job.report_path(), job.plots_path()];
    if !job.cells.is_empty() {
        let cells = &job.cells;
        crate::check_cancel()?;
        let epsg = job
            .epsg
            .or_else(|| cells.first().and_then(|c| crate::crs::epsg_of(c)));
        let interval = job.interval_map.then(|| job.interval_path());
        let st = predict_map(
            &fitted.model,
            cells,
            &job.output,
            interval.as_deref(),
            job.mask_extrapolation,
            epsg,
        );
        let st = match st {
            Ok(s) => s,
            Err(e @ Error::Cancelled) => return Err(e),
            Err(e) => {
                // the fit is still worth keeping: the report goes out with the reason
                let _ = writeln!(text, "\nMAPA\n  No se pudo hacer: {e}");
                std::fs::write(job.report_path(), &text)?;
                return Err(Error::Param(text));
            }
        };
        let _ = writeln!(text, "\nMAPA");
        let _ = writeln!(
            text,
            "  {} celdas de {} m · estimadas {} · media {} · suma {}",
            st.cells,
            fmt_g(st.cell),
            st.estimated,
            fmt_g(st.mean),
            fmt_g(st.total)
        );
        if st.outside_range > 0 {
            let worst: Vec<String> = st
                .outside_by_metric
                .iter()
                .map(|(n, c)| format!("{n} {:.0} %", 100.0 * *c as f64 / st.cells as f64))
                .collect();
            let _ = writeln!(
                text,
                "  Celdas fuera del rango de las parcelas, por métrica: {}.",
                worst.join(" · ")
            );
            let _ = writeln!(
                text,
                "  {} celdas ({:.1} %) quedan vacías por tener alguna métrica fuera del rango de las parcelas: el modelo no ha visto nada parecido.",
                st.outside_range,
                100.0 * st.outside_range as f64 / st.cells as f64
            );
        }
        if st.incomplete > 0 {
            let _ = writeln!(
                text,
                "  {} celdas sin estimar por falta de datos en alguna métrica.",
                st.incomplete
            );
        }
        if st.clamped > 0 {
            let _ = writeln!(
                text,
                "  {} celdas salían negativas y se dejaron en 0.",
                st.clamped
            );
        }
        let _ = writeln!(
            text,
            "  Nota: la suma de la variable en el mapa es una estimación con el error de arriba, no un inventario."
        );
        files.push(job.output.clone());
        if job.interval_map {
            files.push(job.interval_path());
        }
    }
    let mut out = std::fs::File::create(job.report_path())?;
    out.write_all(text.as_bytes())?;
    let _ = writeln!(
        text,
        "\nArchivos: {}",
        files
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" · ")
    );
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Error {
        Error::Param("sin resultado".into())
    }

    /// A reproducible stream of numbers in [0, 1).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }

        fn normal(&mut self) -> f64 {
            let (a, b) = (self.next().max(1e-12), self.next());
            (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
        }
    }

    #[test]
    fn distributions_match_tables() -> Result<()> {
        assert!((t_pvalue(2.228_138_85, 10.0) - 0.05).abs() < 1e-4);
        assert!((t_pvalue(2.0, 1e6) - 0.0455).abs() < 1e-3);
        assert!((t_crit95(10.0) - 2.228_14).abs() < 1e-3);
        assert!((t_crit95(30.0) - 2.042_27).abs() < 1e-3);
        assert!((erfc(1.0) - 0.157_299).abs() < 1e-5);
        // F(2, 10) = 4.10 leaves 5 %
        assert!((f_pvalue(4.102_8, 2.0, 10.0) - 0.05).abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn recovers_exact_coefficients() -> Result<()> {
        let mut g = Lcg(1);
        let x1: Vec<f64> = (0..30).map(|_| 10.0 * g.next()).collect();
        let x2: Vec<f64> = (0..30).map(|_| 50.0 * g.next()).collect();
        let y: Vec<f64> = (0..30).map(|i| 3.0 + 2.0 * x1[i] - 0.5 * x2[i]).collect();
        let m = ols(&[&x1, &x2], &y).ok_or_else(none)?;
        let (a, s) = m.slopes();
        assert!((a - 3.0).abs() < 1e-8 && (s[0] - 2.0).abs() < 1e-8 && (s[1] + 0.5).abs() < 1e-8);
        assert!(m.sse < 1e-12);
        Ok(())
    }

    #[test]
    fn collinear_columns_are_refused() -> Result<()> {
        let x1: Vec<f64> = (0..20).map(f64::from).collect();
        let x2: Vec<f64> = x1.iter().map(|v| 2.0 * v + 1.0).collect();
        let y: Vec<f64> = x1.iter().map(|v| v * v).collect();
        assert!(ols(&[&x1, &x2], &y).is_none());
        Ok(())
    }

    #[test]
    fn hat_matrix_loo_equals_refitting() -> Result<()> {
        let mut g = Lcg(7);
        let n = 25;
        let x1: Vec<f64> = (0..n).map(|_| 10.0 * g.next()).collect();
        let x2: Vec<f64> = (0..n).map(|_| 5.0 * g.next()).collect();
        let y: Vec<f64> = (0..n)
            .map(|i| 1.0 + x1[i] + 2.0 * x2[i] + g.normal())
            .collect();
        let m = ols(&[&x1, &x2], &y).ok_or_else(none)?;
        for i in 0..n {
            let drop = |c: &Vec<f64>| -> Vec<f64> {
                c.iter()
                    .enumerate()
                    .filter(|(r, _)| *r != i)
                    .map(|(_, v)| *v)
                    .collect()
            };
            let (a, b, yy) = (drop(&x1), drop(&x2), drop(&y));
            let f = ols(&[&a, &b], &yy).ok_or_else(none)?;
            let explicit = y[i] - f.predict(&[x1[i], x2[i]]);
            let h = m.leverage(&[x1[i], x2[i]]);
            let fitted = m.predict(&[x1[i], x2[i]]);
            let hat = (y[i] - fitted) / (1.0 - h);
            assert!((explicit - hat).abs() < 1e-9, "{explicit} {hat}");
        }
        Ok(())
    }

    fn synthetic(n: usize, noise: f64, p: usize, seed: u64) -> Data {
        let mut g = Lcg(seed);
        let x: Vec<Vec<f64>> = (0..p)
            .map(|_| (0..n).map(|_| 20.0 * g.next() + 1.0).collect())
            .collect();
        let y: Vec<f64> = (0..n)
            .map(|i| 20.0 + 6.0 * x[0][i] + 3.0 * x[1][i] + noise * g.normal())
            .collect();
        Data {
            response: "Vol".into(),
            keys: (0..n).map(|i| format!("P{i}")).collect(),
            y,
            names: (0..p).map(|j| format!("m{j}")).collect(),
            x,
        }
    }

    #[test]
    fn selection_finds_the_true_metrics_and_cv_is_honest() -> Result<()> {
        let d = synthetic(150, 5.0, 12, 11);
        let f = fit(&d, &Spec::default())?;
        let mut v = f.model.vars.clone();
        v.sort();
        assert_eq!(v, vec!["m0".to_string(), "m1".to_string()]);
        assert!(f.fit_errors.r2 > 0.95);
        // the honest error is not below the fitted one, and is close to the noise
        assert!(f.cv_errors.rmse >= f.fit_errors.rmse * 0.99);
        assert!(f.cv_errors.rmse < 8.0);
        assert!(f.same_model > 0.9);
        Ok(())
    }

    #[test]
    fn pure_noise_gives_no_model_or_a_poor_validation() -> Result<()> {
        let mut g = Lcg(5);
        let n = 20;
        let x: Vec<Vec<f64>> = (0..40)
            .map(|_| (0..n).map(|_| g.next()).collect())
            .collect();
        let d = Data {
            response: "Vol".into(),
            keys: (0..n).map(|i| format!("P{i}")).collect(),
            y: (0..n).map(|_| 100.0 + 10.0 * g.normal()).collect(),
            names: (0..40).map(|j| format!("m{j}")).collect(),
            x,
        };
        match fit(&d, &Spec::default()) {
            Err(_) => {}
            Ok(f) => assert!(f.cv_errors.r2 < 0.2, "R2cv {}", f.cv_errors.r2),
        }
        Ok(())
    }

    #[test]
    fn log_model_predicts_in_original_units() -> Result<()> {
        let mut g = Lcg(3);
        let n = 50;
        let h: Vec<f64> = (0..n).map(|_| 5.0 + 25.0 * g.next()).collect();
        let y: Vec<f64> = h
            .iter()
            .map(|h| 0.8 * h.powf(1.7) * (0.05 * g.normal()).exp())
            .collect();
        let d = Data {
            response: "Vol".into(),
            keys: (0..n).map(|i| format!("P{i}")).collect(),
            y,
            names: vec!["h".into()],
            x: vec![h],
        };
        let spec = Spec {
            transform: Transform::LogBoth,
            predictors: vec!["h".into()],
            ..Spec::default()
        };
        let f = fit(&d, &spec)?;
        assert!((f.model.coefs[1].value - 1.7).abs() < 0.1);
        let (est, hw) = f.model.predict(&[20.0]).ok_or_else(none)?;
        assert!((est / (0.8 * 20f64.powf(1.7)) - 1.0).abs() < 0.1);
        assert!(hw > 0.0);
        assert!(f.model.inside(&[20.0]) && !f.model.inside(&[100.0]));
        Ok(())
    }

    #[test]
    fn influential_plot_is_flagged() -> Result<()> {
        let mut d = synthetic(30, 2.0, 3, 21);
        d.y[4] += 400.0;
        let f = fit(
            &d,
            &Spec {
                predictors: vec!["m0".into(), "m1".into()],
                ..Spec::default()
            },
        )?;
        assert!(
            f.warnings().iter().any(|w| w.contains("P4")),
            "{:?}",
            f.warnings()
        );
        assert!(
            f.plots_csv()
                .lines()
                .any(|l| l.starts_with("P4,") && l.contains("atipica"))
        );
        Ok(())
    }

    #[test]
    fn join_matches_names_and_reports_the_rest() -> Result<()> {
        let dir = std::env::temp_dir().join("easylidar_aba_join");
        let _ = std::fs::create_dir_all(&dir);
        let (p, f) = (dir.join("p.csv"), dir.join("f.csv"));
        let mut metrics = String::from("Parcela,Area,Elev P95,Elev mean,Total return count\n");
        let mut field = String::from("Parcela;Vol\n");
        for i in 1..=8 {
            let h = f64::from(i) * 2.0 + f64::from(i % 3);
            metrics.push_str(&format!("{i},500,{h},{},{}\n", h / 2.0, 100 + i));
            field.push_str(&format!(
                "{i}.0;{}\n",
                (3.0 * h + 5.0).to_string().replace('.', ",")
            ));
        }
        metrics.push_str("99,500,5,3,100\n");
        assert!(std::fs::write(&p, metrics).is_ok() && std::fs::write(&f, field).is_ok());
        let (t, u) = (Table::read(&p)?, Table::read(&f)?);
        let (data, notes) = join(
            &t,
            &u,
            &JoinSpec {
                plot_key: "Parcela",
                field_key: "Parcela",
                response: "Vol",
                wanted: &[],
                available: None,
                candidates: &[],
                intensity: false,
            },
        )?;
        assert_eq!(data.y.len(), 8);
        // the count and the area are not candidates
        assert_eq!(
            data.names,
            vec!["Elev P95".to_string(), "Elev mean".to_string()]
        );
        assert!(notes.iter().any(|n| n.contains("99")));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn map_follows_the_model_and_masks_extrapolation() -> Result<()> {
        let dir = std::env::temp_dir().join("easylidar_aba_map");
        let _ = std::fs::create_dir_all(&dir);
        let cells = dir.join("c.csv");
        let mut t = String::from("row,col,center X,center Y,Elev P95\n");
        for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            t.push_str(&format!(
                "{r},{c},{},{},{}\n",
                5.0 + 10.0 * f64::from(c),
                25.0 - 10.0 * f64::from(r),
                10.0 + 10.0 * f64::from(c + r)
            ));
        }
        assert!(std::fs::write(&cells, t).is_ok());
        let mut g = Lcg(9);
        let h: Vec<f64> = (0..20).map(|_| 5.0 + 20.0 * g.next()).collect();
        let d = Data {
            response: "Vol".into(),
            keys: (0..20).map(|i| i.to_string()).collect(),
            y: h.iter().map(|h| 4.0 * h + 1.0).collect(),
            names: vec!["Elev P95".into()],
            x: vec![h],
        };
        let f = fit(
            &d,
            &Spec {
                predictors: vec!["Elev P95".into()],
                ..Spec::default()
            },
        )?;
        let out = dir.join("v.tif");
        let st = predict_map(&f.model, &[cells], &out, None, true, Some(25830))?;
        // 10, 20, 20, 30: 30 is above the largest height of the plots (at most 25)
        assert_eq!(st.cell, 10.0);
        assert!(st.estimated >= 3);
        let g = Grid::read(&out)?;
        assert_eq!((g.cols, g.rows), (2, 2));
        // the north-west cell has 10 m: 4·10 + 1
        assert!((f64::from(g.data[0]) - 41.0).abs() < 1e-3, "{}", g.data[0]);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
    #[test]
    fn normal_quantiles_and_shapiro_wilk_match_known_values() -> Result<()> {
        assert!((norm_inv(0.975) - 1.959_964).abs() < 1e-5);
        assert!((norm_inv(0.5)).abs() < 1e-9 && (norm_inv(0.001) + 3.090_232).abs() < 1e-4);
        // the example of the original paper (11 weights): W = 0.79, p < 0.01
        let x = [
            148.0, 154.0, 158.0, 160.0, 161.0, 162.0, 166.0, 170.0, 182.0, 195.0, 236.0,
        ];
        let (w, p) = shapiro_wilk(&x).ok_or_else(none)?;
        assert!((w - 0.79).abs() < 0.01 && p > 0.002 && p < 0.015, "{w} {p}");
        // values at the normal quantiles are as normal as it gets
        let q: Vec<f64> = (1..=30)
            .map(|i| norm_inv((f64::from(i) - 0.5) / 30.0))
            .collect();
        let (w, p) = shapiro_wilk(&q).ok_or_else(none)?;
        assert!(w > 0.98 && p > 0.5, "{w} {p}");
        // a strongly skewed sample is rejected, at any size
        let mut g = Lcg(4);
        for n in [8usize, 25, 200] {
            let s: Vec<f64> = (0..n).map(|_| (3.0 * g.normal()).exp()).collect();
            let (_, p) = shapiro_wilk(&s).ok_or_else(none)?;
            assert!(p < 0.05, "n {n} p {p}");
        }
        assert!(shapiro_wilk(&[1.0, 1.0, 1.0, 1.0]).is_none());
        Ok(())
    }

    #[test]
    fn a_group_of_repeated_metrics_leaves_one() -> Result<()> {
        let mut g = Lcg(2);
        let n = 40;
        let base: Vec<f64> = (0..n).map(|_| 10.0 * g.next()).collect();
        let y: Vec<f64> = base.iter().map(|b| 3.0 * b + g.normal()).collect();
        // five near copies of `base`, and one unrelated metric
        let mut cands: Vec<Vec<f64>> = (0..5)
            .map(|k| {
                base.iter()
                    .map(|b| b * (1.0 + 0.1 * f64::from(k)) + 0.05 * g.normal())
                    .collect()
            })
            .collect();
        cands.push((0..n).map(|_| g.next()).collect());
        let (kept, dropped) = prune(&cands, &y, 0.8);
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert_eq!(dropped.len(), 4);
        assert!(
            dropped
                .iter()
                .all(|d| kept.contains(&d.1) && d.2.abs() > 0.8)
        );
        // with no limit nothing is dropped
        assert_eq!(prune(&cands, &y, 1.0).0.len(), 6);
        Ok(())
    }

    #[test]
    fn many_copies_of_the_same_metric_do_not_break_the_fit() -> Result<()> {
        let mut d = synthetic(60, 5.0, 3, 17);
        // forty almost identical copies of m0 and m1: the collinearity that made fitting all fail
        for k in 0..40 {
            let src = d.x[k % 2].clone();
            let mut g = Lcg(100 + k as u64);
            d.x.push(src.iter().map(|v| v * 1.01 + 0.01 * g.normal()).collect());
            d.names.push(format!("copy{k}"));
        }
        let f = fit(&d, &Spec::default())?;
        assert_eq!(f.model.vars.len(), 2);
        assert!(f.screening.kept <= 5, "{}", f.screening.kept);
        assert!(f.fit_errors.r2 > 0.9);
        Ok(())
    }

    #[test]
    fn a_skewed_response_is_flagged_and_the_log_recommended() -> Result<()> {
        let mut g = Lcg(8);
        let n = 60;
        let h: Vec<f64> = (0..n).map(|_| 5.0 + 20.0 * g.next()).collect();
        // a log-normal response: skewed as it is, normal in logarithms
        let d = Data {
            response: "Vol".into(),
            keys: (0..n).map(|i| i.to_string()).collect(),
            y: (0..n).map(|_| (4.0 + 0.8 * g.normal()).exp()).collect(),
            names: vec!["h".into()],
            x: vec![h],
        };
        let s = screen(&d, &Spec::default());
        assert!(s.response.p < 0.05 && s.response_ln.as_ref().is_some_and(|l| l.p > 0.05));
        assert!(
            s.advice.iter().any(|a| a.contains("logarítmica")),
            "{:?}",
            s.advice
        );
        Ok(())
    }
    #[test]
    fn reset_detects_a_curved_relation_and_accepts_a_straight_one() -> Result<()> {
        let mut g = Lcg(12);
        let n = 60;
        let x: Vec<f64> = (0..n).map(|_| 1.0 + 9.0 * g.next()).collect();
        let mk = |f: &dyn Fn(f64) -> f64, g: &mut Lcg| Data {
            response: "y".into(),
            keys: (0..n).map(|i| i.to_string()).collect(),
            y: x.iter().map(|v| f(*v) + 0.5 * g.normal()).collect(),
            names: vec!["x".into()],
            x: vec![x.clone()],
        };
        let spec = Spec {
            predictors: vec!["x".into()],
            ..Spec::default()
        };
        let curved = fit(&mk(&|v| 2.0 + 0.8 * v * v, &mut g), &spec)?;
        assert!(curved.reset_p < 0.01, "{}", curved.reset_p);
        assert!(
            curved
                .warnings()
                .iter()
                .any(|w| w.contains("no parece lineal"))
        );
        let r = curved.report();
        assert!(
            r.contains("NO se cumple:") && r.contains("linealidad"),
            "{r}"
        );
        let straight = fit(&mk(&|v| 2.0 + 3.0 * v, &mut g), &spec)?;
        assert!(straight.reset_p > 0.05, "{}", straight.reset_p);
        Ok(())
    }
}
