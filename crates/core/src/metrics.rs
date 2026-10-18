//! Grid metrics, as FUSION's GridMetrics `*_all_returns_elevation_stats.csv`: descriptive
//! statistics of the point heights in each cell.
//!
//! Memory stays under a budget whatever the cloud's size: a first pass counts the points of
//! each cell, then the rows are processed in bands that fit the budget, each band reading the
//! cloud once and placing its heights straight into per-cell slices (a counting sort).

use std::io::Write;

use crate::grid::Grid;
use crate::las::Reader;
use crate::{Error, Result};

/// The output columns, in FUSION's order; `{hb}` is the height break.
const COLUMNS: [&str; 71] = [
    "row",
    "col",
    "center X",
    "center Y",
    "Total return count",
    "Elev minimum",
    "Elev maximum",
    "Elev mean",
    "Elev mode",
    "Elev stddev",
    "Elev variance",
    "Elev CV",
    "Elev IQ",
    "Elev skewness",
    "Elev kurtosis",
    "Elev AAD",
    "Elev L1",
    "Elev L2",
    "Elev L3",
    "Elev L4",
    "Elev L CV",
    "Elev L skewness",
    "Elev L kurtosis",
    "Elev P01",
    "Elev P05",
    "Elev P10",
    "Elev P20",
    "Elev P25",
    "Elev P30",
    "Elev P40",
    "Elev P50",
    "Elev P60",
    "Elev P70",
    "Elev P75",
    "Elev P80",
    "Elev P90",
    "Elev P95",
    "Elev P99",
    "Return 1 count",
    "Return 2 count",
    "Return 3 count",
    "Return 4 count",
    "Return 5 count",
    "Return 6 count",
    "Return 7 count",
    "Return 8 count",
    "Return 9 count",
    "Other return count",
    "Percentage first returns above {hb}",
    "Percentage all returns above {hb}",
    "(All returns above {hb}) / (Total first returns) * 100",
    "First returns above {hb}",
    "All returns above {hb}",
    "Percentage first returns above mean",
    "Percentage first returns above mode",
    "Percentage all returns above mean",
    "Percentage all returns above mode",
    "(All returns above mean) / (Total first returns) * 100",
    "(All returns above mode) / (Total first returns) * 100",
    "First returns above mean",
    "First returns above mode",
    "All returns above mean",
    "All returns above mode",
    "Total first returns",
    "Total all returns",
    "Elev MAD median",
    "Elev MAD mode",
    "Canopy relief ratio",
    "Elev quadratic mean",
    "Elev cubic mean",
    "Profile area",
];

const PERCENTILES: [f64; 15] = [
    0.01, 0.05, 0.10, 0.20, 0.25, 0.30, 0.40, 0.50, 0.60, 0.70, 0.75, 0.80, 0.90, 0.95, 0.99,
];

/// Bytes held per point while its band is processed: its height, its return number, and its
/// intensity when intensity or strata metrics are asked for.
const BYTES_PER_POINT: u64 = 5;
const BYTES_PER_POINT_INT: u64 = 7;

/// Where the metrics go: the elevation metrics always, the others when given.
pub struct Outputs<'a> {
    pub elevation: &'a mut dyn Write,
    pub intensity: Option<&'a mut dyn Write>,
    pub strata: Option<&'a mut dyn Write>,
}

/// FUSION's intensity metrics: the distribution statistics of the elevation file (its
/// columns 5 to 38), of the intensities.
const INT_STATS: usize = 34;

fn int_header() -> String {
    COLUMNS[..4 + INT_STATS]
        .iter()
        .map(|c| c.replacen("Elev ", "Int ", 1))
        .collect::<Vec<_>>()
        .join(",")
}

/// The strata of `limits`: (name, lower, upper).
fn strata_names(limits: &[f64]) -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    let mut low = f64::NEG_INFINITY;
    for &up in limits {
        let name = if low == f64::NEG_INFINITY {
            format!("below {up:.2}")
        } else {
            format!("{low:.2} to {up:.2}")
        };
        out.push((name, low, up));
        low = up;
    }
    if let Some(&last) = limits.last() {
        out.push((format!("above {last:.2}"), last, f64::INFINITY));
    }
    out
}

const STRATA_STATS: [&str; 11] = [
    "total return count",
    "return proportion",
    "min",
    "max",
    "mean",
    "mode",
    "median",
    "stddev",
    "CV",
    "skewness",
    "kurtosis",
];

fn strata_header(p: &MetricParams) -> String {
    let mut cols = vec!["row".to_string(), "col".to_string()];
    for (kind, limits) in [("Elev", &p.strata), ("Int", &p.int_strata)] {
        for (name, _, _) in strata_names(limits) {
            cols.extend(
                STRATA_STATS
                    .iter()
                    .map(|s| format!("{kind} strata ({name}) {s}")),
            );
        }
    }
    cols.join(",")
}

/// One stratum's statistics, formatted as FUSION writes them.
fn stratum(out: &mut dyn Write, v: &mut [f32], total: usize) -> std::io::Result<()> {
    let n = v.len();
    if n == 0 {
        out.write_all(b",0,0.0")?;
        for _ in 0..9 {
            out.write_all(b",-9999.0")?;
        }
        return Ok(());
    }
    let nf = n as f64;
    if n < 3 {
        // FUSION gives strata of fewer than 3 points their mean only.
        let mean = v.iter().map(|&x| f64::from(x)).sum::<f64>() / nf;
        write!(
            out,
            ",{n},{:.6},-9999.0,-9999.0,{mean:.6}",
            nf / total.max(1) as f64
        )?;
        for _ in 0..6 {
            out.write_all(b",-9999.0")?;
        }
        return Ok(());
    }
    let (mut min, mut max, mut sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
    for &x in v.iter() {
        let x = f64::from(x);
        min = min.min(x);
        max = max.max(x);
        sum += x;
    }
    let mean = sum / nf;
    let (mut m2, mut m3, mut m4) = (0.0, 0.0, 0.0);
    for &x in v.iter() {
        let d = f64::from(x) - mean;
        m2 += d * d;
        m3 += d * d * d;
        m4 += d * d * d * d;
    }
    let var = if n > 1 { m2 / (nf - 1.0) } else { 0.0 };
    let sd = var.sqrt();
    let skew = if sd > 0.0 {
        m3 / ((nf - 1.0) * sd.powi(3))
    } else {
        0.0
    };
    let kurt = if sd > 0.0 {
        m4 / ((nf - 1.0) * sd.powi(4))
    } else {
        0.0
    };
    let mode_v = mode(v, min, max);
    v.sort_unstable_by(f32::total_cmp);
    let median = percentile(v, 0.5);
    let cv = if mean != 0.0 { sd / mean } else { 0.0 };
    write!(out, ",{n},{:.6}", nf / total.max(1) as f64)?;
    for x in [min, max, mean, mode_v, median, sd, cv, skew, kurt] {
        write!(out, ",{x:.6}")?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricParams {
    pub cell: f64,
    /// Height break for the cover metrics.
    pub heightbreak: f64,
    /// Cells with fewer points get no metrics.
    pub min_points: u32,
    /// Most bytes of point heights held at once.
    pub memory: u64,
    /// Upper limits of the height strata (FUSION /strata) and of the intensity strata
    /// (FUSION /intstrata).
    pub strata: Vec<f64>,
    pub int_strata: Vec<f64>,
}

impl Default for MetricParams {
    fn default() -> Self {
        Self {
            cell: 15.0,
            heightbreak: 2.0,
            min_points: 4,
            memory: 512 << 20,
            strata: vec![0.15, 1.37, 5.0, 10.0, 20.0, 30.0],
            int_strata: vec![0.15, 1.37],
        }
    }
}

/// FUSION's percentile (Resampling Stats "method 5"): linear between order statistics, at rank
/// `p (n - 1)`. `v` is sorted and not empty.
fn percentile(v: &[f32], p: f64) -> f64 {
    // FUSION computes the rank in single precision; doing the same picks the same order
    // statistics and fraction, which matters where neighbouring values are far apart.
    let idx32 = p as f32 * (v.len() - 1) as f32;
    let i = idx32.floor() as usize;
    let idx = i as f64 + f64::from(idx32 - idx32.floor());
    let lo = f64::from(v.get(i).copied().unwrap_or(0.0));
    let hi = f64::from(
        v.get(i + 1)
            .copied()
            .unwrap_or(v.get(i).copied().unwrap_or(0.0)),
    );
    lo + (idx - i as f64) * (hi - lo)
}

/// The mode as FUSION computes it: 64 bins `(max - min) / 63` wide (the maximum alone opens
/// the last), and the lower edge of the fullest, the lowest on a tie.
fn mode(v: &[f32], min: f64, max: f64) -> f64 {
    const BINS: usize = 64;
    let width = (max - min) / (BINS - 1) as f64;
    if !(width > 0.0) {
        return min;
    }
    let mut hist = [0u32; BINS];
    for &x in v {
        // Multiplying first keeps whole-number data (intensities) on exact bin edges.
        let b = (((f64::from(x) - min) * (BINS - 1) as f64 / (max - min)) as usize).min(BINS - 1);
        if let Some(h) = hist.get_mut(b) {
            *h += 1;
        }
    }
    let mut best = 0;
    for (i, &h) in hist.iter().enumerate() {
        if h > hist.get(best).copied().unwrap_or(0) {
            best = i;
        }
    }
    min + best as f64 * width
}

/// The metrics of one cell: `h` its heights (sorted here), `ret` their return numbers.
fn cell_metrics(h: &mut [f32], ret: &[u8], hb: f64) -> Vec<f64> {
    let n = h.len() as f64;
    let (mut sum, mut sum2, mut sum3) = (0.0, 0.0, 0.0);
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for &x in h.iter() {
        let x = f64::from(x);
        sum += x;
        sum2 += x * x;
        // FUSION's cubic mean is of absolute values.
        sum3 += x.abs().powi(3);
        min = min.min(x);
        max = max.max(x);
    }
    let mean = sum / n;
    let (mut m2, mut m3, mut m4, mut aad) = (0.0, 0.0, 0.0, 0.0);
    for &x in h.iter() {
        let d = f64::from(x) - mean;
        m2 += d * d;
        m3 += d * d * d;
        m4 += d * d * d * d;
        aad += d.abs();
    }
    let variance = if n > 1.0 { m2 / (n - 1.0) } else { 0.0 };
    let sd = variance.sqrt();
    let skew = if sd > 0.0 {
        m3 / ((n - 1.0) * sd.powi(3))
    } else {
        0.0
    };
    let kurt = if sd > 0.0 {
        m4 / ((n - 1.0) * sd.powi(4))
    } else {
        0.0
    };
    let mode_v = mode(h, min, max);

    // Counts that pair a height with its return number, before the heights are sorted.
    let mut by_ret = [0u32; 10];
    let (
        mut first,
        mut first_hb,
        mut all_hb,
        mut first_mean,
        mut first_mode,
        mut all_mean,
        mut all_mode,
    ) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
    for (&x, &r) in h.iter().zip(ret) {
        let x = f64::from(x);
        let slot = if (1..=9).contains(&r) {
            usize::from(r) - 1
        } else {
            9
        };
        if let Some(c) = by_ret.get_mut(slot) {
            *c += 1;
        }
        let is_first = r == 1;
        first += u32::from(is_first);
        all_hb += u32::from(x > hb);
        all_mean += u32::from(x > mean);
        all_mode += u32::from(x > mode_v);
        if is_first {
            first_hb += u32::from(x > hb);
            first_mean += u32::from(x > mean);
            first_mode += u32::from(x > mode_v);
        }
    }

    h.sort_unstable_by(f32::total_cmp);
    let pct: Vec<f64> = PERCENTILES.iter().map(|&p| percentile(h, p)).collect();
    let p = |q: f64| percentile(h, q);
    let median = p(0.5);

    // L-moments from probability-weighted moments (unbiased, Wang 1996).
    let (mut b0, mut b1, mut b2, mut b3) = (0.0, 0.0, 0.0, 0.0);
    for (i, &x) in h.iter().enumerate() {
        let (x, i) = (f64::from(x), i as f64);
        b0 += x;
        if n > 1.0 {
            b1 += x * i / (n - 1.0);
        }
        if n > 2.0 {
            b2 += x * i * (i - 1.0) / ((n - 1.0) * (n - 2.0));
        }
        if n > 3.0 {
            b3 += x * i * (i - 1.0) * (i - 2.0) / ((n - 1.0) * (n - 2.0) * (n - 3.0));
        }
    }
    let (b0, b1, b2, b3) = (b0 / n, b1 / n, b2 / n, b3 / n);
    let l1 = b0;
    let l2 = 2.0 * b1 - b0;
    let l3 = 6.0 * b2 - 6.0 * b1 + b0;
    let l4 = 20.0 * b3 - 30.0 * b2 + 12.0 * b1 - b0;
    let ratio = |a: f64, b: f64| if b != 0.0 { a / b } else { 0.0 };

    // Median absolute deviations, from the median and from the mode.
    let mut dev: Vec<f32> = h
        .iter()
        .map(|&x| (f64::from(x) - median).abs() as f32)
        .collect();
    dev.sort_unstable_by(f32::total_cmp);
    let mad_median = percentile(&dev, 0.5);
    for (d, &x) in dev.iter_mut().zip(h.iter()) {
        *d = (f64::from(x) - mode_v).abs() as f32;
    }
    dev.sort_unstable_by(f32::total_cmp);
    let mad_mode = percentile(&dev, 0.5);

    // Profile area: the area under the 1-99 percentile curve of the heights (at least 0),
    // normalised by the 99th percentile.
    let mut clamped: Vec<f32> = h.iter().map(|&x| x.max(0.0)).collect();
    clamped.sort_unstable_by(f32::total_cmp);
    let p99 = percentile(&clamped, 0.99);
    let profile = if p99 > 0.0 {
        let v: Vec<f64> = (1..=99)
            .map(|k| percentile(&clamped, f64::from(k) / 100.0) / p99)
            .collect();
        v.windows(2).map(|w| (w[0] + w[1]) / 2.0).sum::<f64>()
    } else {
        -9999.0
    };

    let (f, a) = (f64::from(first), n);
    let pc = |c: u32, of: f64| {
        if of > 0.0 {
            f64::from(c) / of * 100.0
        } else {
            0.0
        }
    };
    let mut out = vec![
        n,
        min,
        max,
        mean,
        mode_v,
        sd,
        variance,
        ratio(sd, mean),
        p(0.75) - p(0.25),
        skew,
        kurt,
        aad / n,
        l1,
        l2,
        l3,
        l4,
        ratio(l2, l1),
        ratio(l3, l2),
        ratio(l4, l2),
    ];
    out.extend(pct);
    out.extend(by_ret.iter().map(|&c| f64::from(c)));
    out.extend([
        pc(first_hb, f),
        pc(all_hb, a),
        pc(all_hb, f),
        f64::from(first_hb),
        f64::from(all_hb),
        pc(first_mean, f),
        pc(first_mode, f),
        pc(all_mean, a),
        pc(all_mode, a),
        pc(all_mean, f),
        pc(all_mode, f),
        f64::from(first_mean),
        f64::from(first_mode),
        f64::from(all_mean),
        f64::from(all_mode),
        f,
        a,
        mad_median,
        mad_mode,
        ratio(mean - min, max - min),
        (sum2 / n).sqrt(),
        (sum3 / n).cbrt(),
        profile,
    ]);
    out
}

/// Column indexes (in the 67 values after row, col and centre) written as whole numbers.
fn is_count(k: usize) -> bool {
    // Total, the 10 return counts, and the 4 + 4 + 2 cover counts.
    k == 0 || (34..44).contains(&k) || (47..49).contains(&k) || (55..61).contains(&k)
}

/// The header of a per-sample metrics row (FUSION CloudMetrics): the elevation metrics, then
/// the intensity ones.
pub fn sample_header(heightbreak: f64) -> Vec<String> {
    // (the capture date column is added by the caller, which knows the files)
    let mut cols: Vec<String> = COLUMNS[4..]
        .iter()
        .map(|c| c.replace("{hb}", &format!("{heightbreak:.2}")))
        .collect();
    cols.extend(
        COLUMNS[4..4 + INT_STATS]
            .iter()
            .map(|c| c.replacen("Elev ", "Int ", 1)),
    );
    cols
}

/// One sample's metrics: `heights` and `intensity` of its points, `ret` their return numbers.
/// Empty samples give -9999 throughout, as FUSION writes them.
pub fn sample_row(
    heights: &mut [f32],
    ret: &[u8],
    intensity: &mut [f32],
    heightbreak: f64,
) -> Vec<String> {
    let total = 67 + INT_STATS;
    if heights.is_empty() {
        return (0..total)
            .map(|k| {
                if is_count(k % 67) {
                    "-9999".to_string()
                } else {
                    "-9999.0000".to_string()
                }
            })
            .collect();
    }
    let fmt = |k: usize, v: f64| {
        if is_count(k) {
            format!("{}", v as i64)
        } else {
            format!("{v:.4}")
        }
    };
    let mut row: Vec<String> = cell_metrics(heights, ret, heightbreak)
        .into_iter()
        .enumerate()
        .map(|(k, v)| fmt(k, v))
        .collect();
    row.extend(
        cell_metrics(intensity, ret, heightbreak)
            .into_iter()
            .take(INT_STATS)
            .enumerate()
            .map(|(k, v)| fmt(k, v)),
    );
    row
}

/// What the run did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricReport {
    pub cells: usize,
    pub cells_with_metrics: usize,
    pub bands: usize,
    pub points: u64,
}

/// Writes the metrics of every cell as CSV, northernmost row first.
pub fn grid_metrics(
    reader: &Reader,
    dtm: &Grid,
    p: &MetricParams,
    outs: Outputs,
) -> Result<MetricReport> {
    let Outputs {
        elevation: out,
        mut intensity,
        mut strata,
    } = outs;
    let with_int = intensity.is_some() || strata.is_some();
    let per_point = if with_int {
        BYTES_PER_POINT_INT
    } else {
        BYTES_PER_POINT
    };
    if !(p.cell > 0.0) || p.memory < per_point {
        return Err(Error::Param(
            "metrics need a positive cell size and a memory budget".into(),
        ));
    }
    let h = &reader.header;
    // FUSION's alignment: cell centres on multiples of the cell size.
    let (c0, r0) = (
        (h.bounds[0] / p.cell).floor(),
        (h.bounds[1] / p.cell).floor(),
    );
    let cols = ((h.bounds[3] / p.cell).ceil() - c0) as usize + 1;
    let rows = ((h.bounds[4] / p.cell).ceil() - r0) as usize + 1;
    let grid = Grid::new(
        c0 * p.cell - p.cell / 2.0,
        r0 * p.cell - p.cell / 2.0,
        p.cell,
        cols,
        rows,
    )?;

    let height = |pt: &crate::las::Point| -> Option<(usize, f32)> {
        if pt.is_noise() {
            return None;
        }
        let i = grid.index(pt.x, pt.y)?;
        Some((i, pt.z as f32 - dtm.sample(pt.x, pt.y)?))
    };

    // Pass 1: points per cell.
    let mut count = vec![0u32; grid.len()];
    let mut report = MetricReport {
        cells: grid.len(),
        ..Default::default()
    };
    reader.scan(|pt| {
        if let Some((i, _)) = height(pt)
            && let Some(c) = count.get_mut(i)
        {
            *c += 1;
            report.points += 1;
        }
    })?;

    // The day the data were captured is one more metric of every cell.
    let date = reader.capture_date().unwrap_or_else(|| "NA".into());
    let header: Vec<String> = COLUMNS
        .iter()
        .map(|c| c.replace("{hb}", &format!("{:.2}", p.heightbreak)))
        .collect();
    writeln!(out, "{},Date", header.join(","))?;
    if let Some(o) = intensity.as_mut() {
        writeln!(o, "{},Date", int_header())?;
    }
    if let Some(o) = strata.as_mut() {
        writeln!(o, "{},Date", strata_header(p))?;
    }
    let (elev_strata, int_strata) = (strata_names(&p.strata), strata_names(&p.int_strata));

    let budget = (p.memory / per_point).max(1);
    let mut row = 0usize;
    while row < rows {
        // The band: as many whole rows as fit the budget (at least one).
        let mut end = row;
        let mut pts = 0u64;
        while end < rows {
            let in_row: u64 = count
                .get(end * cols..(end + 1) * cols)
                .map_or(0, |s| s.iter().map(|&c| u64::from(c)).sum());
            if end > row && pts + in_row > budget {
                break;
            }
            pts += in_row;
            end += 1;
        }
        let (first_cell, last_cell) = (row * cols, end * cols);
        let mut start = Vec::with_capacity(last_cell - first_cell + 1);
        let mut acc = 0usize;
        start.push(0);
        for c in count.get(first_cell..last_cell).unwrap_or(&[]) {
            acc += *c as usize;
            start.push(acc);
        }
        let mut heights = vec![0f32; acc];
        let mut rets = vec![0u8; acc];
        let mut ints = vec![0u16; if with_int { acc } else { 0 }];
        let mut fill = vec![0u32; last_cell - first_cell];
        if acc > 0 {
            reader.scan(|pt| {
                let Some((i, ht)) = height(pt) else { return };
                if i < first_cell || i >= last_cell {
                    return;
                }
                let k = i - first_cell;
                if let (Some(s), Some(f)) = (start.get(k), fill.get_mut(k)) {
                    let at = s + *f as usize;
                    if let (Some(hv), Some(rv)) = (heights.get_mut(at), rets.get_mut(at)) {
                        *hv = ht;
                        *rv = pt.return_number;
                        *f += 1;
                    }
                    if let Some(iv) = ints.get_mut(at) {
                        *iv = pt.intensity;
                    }
                }
            })?;
        }
        report.bands += 1;
        for r in row..end {
            for c in 0..cols {
                let k = r * cols + c - first_cell;
                let (Some(&s), Some(&e)) = (start.get(k), start.get(k + 1)) else {
                    continue;
                };
                let (cx, cy) = grid.center(c, r);
                write!(out, "{},{},{:.4},{:.4}", rows - 1 - r, c, cx, cy)?;
                let n = e - s;
                let enough = n >= p.min_points as usize && n > 0;
                // Intensity and strata read the heights unsorted, so they come first.
                if let (Some(o), Some(is), Some(rs)) =
                    (intensity.as_mut(), ints.get(s..e), rets.get(s..e))
                {
                    write!(o, "{},{},{:.4},{:.4}", rows - 1 - r, c, cx, cy)?;
                    if enough {
                        let mut iv: Vec<f32> = is.iter().map(|&v| f32::from(v)).collect();
                        let stats = cell_metrics(&mut iv, rs, p.heightbreak);
                        for (k, v) in stats.into_iter().take(INT_STATS).enumerate() {
                            if k == 0 {
                                write!(o, ",{}", v as i64)?;
                            } else {
                                write!(o, ",{v:.4}")?;
                            }
                        }
                    } else {
                        for k in 0..INT_STATS {
                            o.write_all(if k == 0 { b",-9999" } else { b",-9999.0000" })?;
                        }
                    }
                    writeln!(o, ",{date}")?;
                }
                if let (Some(o), Some(hs), Some(is)) =
                    (strata.as_mut(), heights.get(s..e), ints.get(s..e))
                {
                    write!(o, "{},{}", rows - 1 - r, c)?;
                    for (by_height, limits) in [(true, &elev_strata), (false, &int_strata)] {
                        for (_, low, up) in limits {
                            if !enough {
                                o.write_all(b",-9999,-9999.0")?;
                                for _ in 0..9 {
                                    o.write_all(b",-9999.0")?;
                                }
                                continue;
                            }
                            let mut v: Vec<f32> = hs
                                .iter()
                                .zip(is)
                                .filter(|(h, _)| f64::from(**h) >= *low && f64::from(**h) < *up)
                                .map(|(h, i)| if by_height { *h } else { f32::from(*i) })
                                .collect();
                            stratum(&mut **o, &mut v, n)?;
                        }
                    }
                    writeln!(o, ",{date}")?;
                }
                if enough {
                    let (Some(hs), Some(rs)) = (heights.get_mut(s..e), rets.get(s..e)) else {
                        continue;
                    };
                    for (k, v) in cell_metrics(hs, rs, p.heightbreak).into_iter().enumerate() {
                        if is_count(k) {
                            write!(out, ",{}", v as i64)?;
                        } else {
                            write!(out, ",{v:.4}")?;
                        }
                    }
                    report.cells_with_metrics += 1;
                } else {
                    for k in 0..67 {
                        out.write_all(if is_count(k) {
                            b",-9999"
                        } else {
                            b",-9999.0000"
                        })?;
                    }
                }
                writeln!(out, ",{date}")?;
            }
        }
        row = end;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_method_5() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 0.5), 2.5);
        assert_eq!(percentile(&v, 1.0), 4.0);
    }

    #[test]
    fn counts_and_cover() {
        let mut h = [0.1f32, 0.2, 5.0, 10.0];
        let ret = [1u8, 1, 1, 2];
        let m = cell_metrics(&mut h, &ret, 2.0);
        assert_eq!(m.len(), 67);
        assert_eq!(m[0], 4.0);
        // 3 first returns, 1 above 2 m; 2 of all 4 above.
        assert!((m[44] - 100.0 / 3.0).abs() < 1e-9);
        assert_eq!(m[45], 50.0);
        assert_eq!(m[59], 3.0);
    }
}
