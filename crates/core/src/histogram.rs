//! Histograms and statistics of a point attribute, and the limits that tell the odd values from
//! the rest (Tukey's fences, the robust z-score of the median absolute deviation, standard
//! deviations, percentiles).
//!
//! Everything is computed in a few parallel passes over the values, never by sorting them:
//! quantiles come from a fine histogram (4096 bins over the range), so they are exact to a
//! 4096th of the range, which is far below what a histogram bar shows.

use rayon::prelude::*;

/// Bins of the fine histogram the quantiles are read from.
const FINE: usize = 4096;

#[derive(Clone, Debug)]
pub struct Histogram {
    /// Lower edge of the first bar and the width of every bar.
    pub lo: f64,
    pub width: f64,
    pub counts: Vec<u64>,
    pub n: u64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub std: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    /// Median absolute deviation from the median.
    pub mad: f64,
    fine: Vec<u64>,
}

/// How odd values are told from the rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Method {
    /// Outside `Q1 - k·IQR … Q3 + k·IQR` (k = 1.5 is Tukey's usual fence, 3 the far one).
    Tukey(f64),
    /// Robust z-score above `k`: `|x - median| > k · 1.4826 · MAD` (3.5 is the usual limit).
    Mad(f64),
    /// More than `k` standard deviations from the mean.
    Sigma(f64),
    /// In the lowest or highest `p` percent.
    Percentile(f64),
}

impl Histogram {
    /// The value below which a share `p` (0–1) of the values lie.
    pub fn quantile(&self, p: f64) -> f64 {
        quantile_of(
            &self.fine,
            self.n,
            self.min,
            fine_width(self.min, self.max),
            p,
        )
    }

    /// The limits `(low, high)`: values below the first or above the second are odd. None when
    /// the method cannot tell (most values are equal, so the spread is zero).
    pub fn limits(&self, m: Method) -> Option<(f64, f64)> {
        let ok = |(lo, hi): (f64, f64)| (lo.is_finite() && hi.is_finite()).then_some((lo, hi));
        match m {
            Method::Tukey(k) => {
                let iqr = self.q3 - self.q1;
                (iqr > 0.0).then_some((self.q1 - k * iqr, self.q3 + k * iqr))
            }
            Method::Mad(k) => {
                let spread = 1.4826 * self.mad;
                (spread > 0.0).then_some((self.median - k * spread, self.median + k * spread))
            }
            Method::Sigma(k) => {
                (self.std > 0.0).then_some((self.mean - k * self.std, self.mean + k * self.std))
            }
            Method::Percentile(p) => {
                let p = (p / 100.0).clamp(0.0, 0.5);
                Some((self.quantile(p), self.quantile(1.0 - p)))
            }
        }
        .and_then(ok)
    }

    /// The intervals of the odd values: `[-inf, low)` and `(high, inf)`. Percentiles keep the
    /// values equal to the limit inside, so a tail never takes more than its share.
    pub fn tails(&self, m: Method) -> Option<Vec<(f64, f64)>> {
        let (lo, hi) = self.limits(m)?;
        Some(vec![(f64::NEG_INFINITY, lo), (hi.next_up(), f64::INFINITY)])
    }

    /// Edges `[lo, hi)` of bar `i`.
    pub fn bar(&self, i: usize) -> (f64, f64) {
        (
            self.lo + i as f64 * self.width,
            self.lo + (i + 1) as f64 * self.width,
        )
    }

    /// Bar of a value.
    pub fn bar_of(&self, v: f64) -> usize {
        (((v - self.lo) / self.width).max(0.0) as usize).min(self.counts.len() - 1)
    }

    /// The intervals the picked bars cover, adjacent ones merged. The last bar takes in its
    /// upper edge, since the largest value belongs to it.
    pub fn ranges(&self, picked: &[bool]) -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = Vec::new();
        for (i, on) in picked.iter().enumerate().take(self.counts.len()) {
            if !*on {
                continue;
            }
            let (lo, mut hi) = self.bar(i);
            if i + 1 == self.counts.len() {
                hi = hi.next_up();
            }
            match out.last_mut() {
                Some(last) if last.1 == lo => last.1 = hi,
                _ => out.push((lo, hi)),
            }
        }
        out
    }

    /// The bars that hold a value of the intervals (those of [`Histogram::tails`], say).
    pub fn bars_in(&self, ranges: &[(f64, f64)]) -> Vec<bool> {
        (0..self.counts.len())
            .map(|i| {
                let (lo, hi) = self.bar(i);
                ranges.iter().any(|r| lo < r.1 && hi > r.0)
            })
            .collect()
    }

    /// How many values the picked bars hold.
    pub fn picked_count(&self, picked: &[bool]) -> u64 {
        self.counts
            .iter()
            .zip(picked)
            .filter(|(_, on)| **on)
            .map(|(c, _)| *c)
            .sum()
    }
}

fn fine_width(min: f64, max: f64) -> f64 {
    if max > min {
        (max - min) / FINE as f64
    } else {
        1.0
    }
}

fn quantile_of(fine: &[u64], n: u64, min: f64, width: f64, p: f64) -> f64 {
    let target = p.clamp(0.0, 1.0) * n as f64;
    let mut before = 0.0;
    for (i, c) in fine.iter().enumerate() {
        let after = before + *c as f64;
        if after >= target && *c > 0 {
            let frac = ((target - before) / *c as f64).clamp(0.0, 1.0);
            return min + (i as f64 + frac) * width;
        }
        before = after;
    }
    min + fine.len() as f64 * width
}

struct Acc {
    counts: Vec<u64>,
    fine: Vec<u64>,
    ssd: f64,
}

fn add(a: &mut [u64], b: &[u64]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x += *y;
    }
}

/// The histogram of the `n` values `value(i)` (None: the point has none), with `bins` bars, or
/// one bar per integer when `integer` and the values span less than 256.
pub fn histogram(
    n: usize,
    value: impl Fn(usize) -> Option<f64> + Sync,
    bins: usize,
    integer: bool,
) -> Option<Histogram> {
    let vals = || {
        (0..n)
            .into_par_iter()
            .filter_map(|i| value(i).filter(|v| v.is_finite()))
    };
    let (count, sum, min, max) = vals()
        .fold(
            || (0u64, 0.0, f64::INFINITY, f64::NEG_INFINITY),
            |a, v| (a.0 + 1, a.1 + v, a.2.min(v), a.3.max(v)),
        )
        .reduce(
            || (0u64, 0.0, f64::INFINITY, f64::NEG_INFINITY),
            |a, b| (a.0 + b.0, a.1 + b.1, a.2.min(b.2), a.3.max(b.3)),
        );
    if count == 0 {
        return None;
    }
    let mean = sum / count as f64;
    let (lo, width, nb) = if integer && max - min < 256.0 {
        (min.floor(), 1.0, (max.floor() - min.floor()) as usize + 1)
    } else if max > min {
        let nb = bins.clamp(2, 1000);
        (min, (max - min) / nb as f64, nb)
    } else {
        (min, 1.0, 1)
    };
    let fw = fine_width(min, max);
    let empty = || Acc {
        counts: vec![0; nb],
        fine: vec![0; FINE],
        ssd: 0.0,
    };
    let acc = vals()
        .fold(empty, |mut a, v| {
            let bar = (((v - lo) / width) as usize).min(nb - 1);
            let f = (((v - min) / fw) as usize).min(FINE - 1);
            a.counts[bar] += 1;
            a.fine[f] += 1;
            a.ssd += (v - mean) * (v - mean);
            a
        })
        .reduce(empty, |mut a, b| {
            add(&mut a.counts, &b.counts);
            add(&mut a.fine, &b.fine);
            a.ssd += b.ssd;
            a
        });
    let spread = max > min;
    let q = |p| quantile_of(&acc.fine, count, min, fw, p);
    let median = q(0.5);
    // the median of the distances to the median, from a second fine histogram
    let reach = (median - min).max(max - median);
    let dw = fine_width(0.0, reach);
    let devs = vals()
        .fold(
            || vec![0u64; FINE],
            |mut a, v| {
                a[((((v - median).abs()) / dw) as usize).min(FINE - 1)] += 1;
                a
            },
        )
        .reduce(
            || vec![0u64; FINE],
            |mut a, b| {
                add(&mut a, &b);
                a
            },
        );
    Some(Histogram {
        lo,
        width,
        counts: acc.counts,
        n: count,
        min,
        max,
        mean,
        std: (acc.ssd / count as f64).sqrt(),
        // equal values have no spread, whatever the fine bins interpolate
        q1: if spread { q(0.25) } else { min },
        median: if spread { median } else { min },
        q3: if spread { q(0.75) } else { min },
        mad: if spread {
            quantile_of(&devs, count, 0.0, dw, 0.5)
        } else {
            0.0
        },
        fine: acc.fine,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, Result};

    fn ramp_with_outlier() -> Result<Histogram> {
        // 0, 1, …, 999 and one far value
        let mut v: Vec<f64> = (0..1000).map(f64::from).collect();
        v.push(10_000.0);
        histogram(v.len(), |i| v.get(i).copied(), 50, false)
            .ok_or_else(|| Error::Param("no histogram".into()))
    }

    #[test]
    fn counts_and_statistics() -> Result<()> {
        let h = ramp_with_outlier()?;
        assert_eq!(h.n, 1001);
        assert_eq!(h.counts.iter().sum::<u64>(), 1001);
        assert_eq!((h.min, h.max), (0.0, 10_000.0));
        // the median of the ramp, to a 4096th of the range
        assert!(
            (h.median - 500.0).abs() < 10_000.0 / 4096.0 * 2.0,
            "{}",
            h.median
        );
        assert!((h.q1 - 250.0).abs() < 10.0 && (h.q3 - 750.0).abs() < 10.0);
        Ok(())
    }

    #[test]
    fn every_method_flags_the_far_value_and_only_it() -> Result<()> {
        let h = ramp_with_outlier()?;
        for m in [Method::Tukey(1.5), Method::Mad(3.5), Method::Sigma(3.0)] {
            let (lo, hi) = h
                .limits(m)
                .ok_or_else(|| Error::Param("no limits".into()))?;
            assert!(
                lo <= 0.0 && (999.0..10_000.0).contains(&hi),
                "{m:?} {lo} {hi}"
            );
        }
        // the lowest and highest 1 %
        let (lo, hi) = h
            .limits(Method::Percentile(1.0))
            .ok_or_else(|| Error::Param("no limits".into()))?;
        assert!(lo > 0.0 && lo < 50.0 && hi > 900.0, "{lo} {hi}");
        Ok(())
    }

    #[test]
    fn no_limits_when_the_values_do_not_vary() -> Result<()> {
        let h = histogram(10, |_| Some(3.0), 20, false)
            .ok_or_else(|| Error::Param("no histogram".into()))?;
        assert_eq!(h.counts, [10]);
        assert!(h.limits(Method::Tukey(1.5)).is_none());
        assert!(h.limits(Method::Sigma(3.0)).is_none());
        assert!(h.limits(Method::Mad(3.5)).is_none());
        Ok(())
    }

    #[test]
    fn integers_get_one_bar_each_and_bars_merge_into_ranges() -> Result<()> {
        let classes = [2u8, 2, 2, 5, 5, 6, 7];
        let h = histogram(
            classes.len(),
            |i| classes.get(i).map(|c| f64::from(*c)),
            50,
            true,
        )
        .ok_or_else(|| Error::Param("no histogram".into()))?;
        assert_eq!((h.lo, h.width, h.counts.len()), (2.0, 1.0, 6));
        assert_eq!(h.counts, [3, 0, 0, 2, 1, 1]);
        // bars for 5, 6 and 7 are adjacent: one interval, and the top edge takes in 7
        let picked = [false, false, false, true, true, true];
        let r = h.ranges(&picked);
        assert_eq!(r.len(), 1);
        assert!(r[0].0 == 5.0 && r[0].1 > 8.0 - 1e-9 && r[0].1 < 8.0 + 1e-9);
        assert_eq!(h.picked_count(&picked), 4);
        assert_eq!(h.bar_of(6.0), 4);
        Ok(())
    }

    #[test]
    fn tails_cover_the_bars_beyond_the_limits() -> Result<()> {
        let h = ramp_with_outlier()?;
        let tails = h
            .tails(Method::Tukey(1.5))
            .ok_or_else(|| Error::Param("no tails".into()))?;
        let bars = h.bars_in(&tails);
        // the bars of the ramp (0–1000 m) are not odd, the last one holds the far value
        assert!(bars.last().copied().unwrap_or(false));
        assert!(bars.iter().take(5).all(|b| !*b));
        Ok(())
    }
}
