//! Inverse discrete wavelet transform of JPEG 2000 (T.800 Annex F): the reversible 5/3 filter on
//! integers and the irreversible 9/7 filter on floats, one level at a time.
//!
//! A level takes the low-pass image of the resolution below and the three detail bands, and gives
//! the image of the resolution: rows and columns are interleaved (low-pass samples at even
//! coordinates of the reference grid, high-pass at odd ones) and the lifting steps undone, with
//! the signal mirrored at the borders.

/// Dimensions and origin of a region on the grid of its resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
}

impl Rect {
    pub fn w(&self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }
    pub fn h(&self) -> usize {
        (self.y1 - self.y0).max(0) as usize
    }
}

/// Mirrors an index into `0..n` (whole-sample symmetric extension).
#[inline]
fn mirror(i: isize, n: isize) -> usize {
    if n == 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let mut k = i.rem_euclid(period);
    if k >= n {
        k = period - k;
    }
    k as usize
}

/// Undoes the reversible lifting on an interleaved line whose first sample sits at an index of
/// parity `p0` (0: low-pass first).
fn synth_53(line: &mut [i32], p0: usize) {
    let n = line.len() as isize;
    if n == 1 {
        if p0 == 1 {
            line[0] >>= 1;
        }
        return;
    }
    // even coordinates: low-pass
    for k in (p0..line.len()).step_by(2) {
        let (a, b) = (mirror(k as isize - 1, n), mirror(k as isize + 1, n));
        line[k] -= (line[a] + line[b] + 2) >> 2;
    }
    for k in ((1 - p0)..line.len()).step_by(2) {
        let (a, b) = (mirror(k as isize - 1, n), mirror(k as isize + 1, n));
        line[k] += (line[a] + line[b]) >> 1;
    }
}

const ALPHA: f32 = -1.586_134_3;
const BETA: f32 = -0.052_980_12;
const GAMMA: f32 = 0.882_911_1;
const DELTA: f32 = 0.443_506_85;
const K: f32 = 1.230_174_1;

fn synth_97(line: &mut [f32], p0: usize) {
    let n = line.len() as isize;
    if n == 1 {
        if p0 == 1 {
            line[0] *= 0.5;
        }
        return;
    }
    for k in (p0..line.len()).step_by(2) {
        line[k] *= K;
    }
    for k in ((1 - p0)..line.len()).step_by(2) {
        line[k] *= 1.0 / K;
    }
    let step = |line: &mut [f32], first: usize, c: f32| {
        for k in (first..line.len()).step_by(2) {
            let (a, b) = (mirror(k as isize - 1, n), mirror(k as isize + 1, n));
            line[k] -= c * (line[a] + line[b]);
        }
    };
    step(line, p0, DELTA);
    step(line, 1 - p0, GAMMA);
    step(line, p0, BETA);
    step(line, 1 - p0, ALPHA);
}

macro_rules! level {
    ($name:ident, $t:ty, $synth:ident, $zero:expr) => {
        /// One level of the inverse transform: `ll` is the image of the resolution below
        /// (`ll_rect`), `hl`, `lh`, `hh` the details; returns the image of `rect`.
        pub fn $name(rect: Rect, ll: &[$t], hl: &[$t], lh: &[$t], hh: &[$t]) -> Vec<$t> {
            let (w, h) = (rect.w(), rect.h());
            let mut out: Vec<$t> = vec![$zero; w * h];
            if w == 0 || h == 0 {
                return out;
            }
            // counts of low-pass and high-pass samples along each axis
            let lw = ((rect.x1 + 1) / 2 - (rect.x0 + 1) / 2) as usize;
            let hw = (rect.x1 / 2 - rect.x0 / 2) as usize;
            let lh_n = ((rect.y1 + 1) / 2 - (rect.y0 + 1) / 2) as usize;
            let hh_n = (rect.y1 / 2 - rect.y0 / 2) as usize;
            let (px, py) = ((rect.x0 & 1) as usize, (rect.y0 & 1) as usize);
            // interleave: rows of the low-pass half come from LL and HL, the others from LH and HH
            for y in 0..h {
                let gy = rect.y0 + y as i64;
                let (low_rows, row_idx) = if gy & 1 == 0 {
                    (true, (gy / 2 - (rect.y0 + 1) / 2) as usize)
                } else {
                    (false, ((gy - 1) / 2 - rect.y0 / 2) as usize)
                };
                for x in 0..w {
                    let gx = rect.x0 + x as i64;
                    out[y * w + x] = if gx & 1 == 0 {
                        let cx = (gx / 2 - (rect.x0 + 1) / 2) as usize;
                        if low_rows {
                            ll[row_idx * lw + cx]
                        } else {
                            lh[row_idx * lw + cx]
                        }
                    } else {
                        let cx = ((gx - 1) / 2 - rect.x0 / 2) as usize;
                        if low_rows {
                            hl[row_idx * hw + cx]
                        } else {
                            hh[row_idx * hw + cx]
                        }
                    };
                }
            }
            let _ = (lh_n, hh_n);
            for y in 0..h {
                $synth(&mut out[y * w..(y + 1) * w], px);
            }
            let mut col: Vec<$t> = vec![$zero; h];
            for x in 0..w {
                for y in 0..h {
                    col[y] = out[y * w + x];
                }
                $synth(&mut col, py);
                for y in 0..h {
                    out[y * w + x] = col[y];
                }
            }
            out
        }
    };
}

level!(inverse_53, i32, synth_53, 0);
level!(inverse_97, f32, synth_97, 0.0);

#[cfg(test)]
mod tests {
    use super::*;

    /// The forward 5/3 transform of a line, to check that the inverse undoes it exactly.
    fn forward_53(x: &mut [i32], p0: usize) {
        let n = x.len() as isize;
        if n == 1 {
            if p0 == 1 {
                x[0] *= 2;
            }
            return;
        }
        for k in ((1 - p0)..x.len()).step_by(2) {
            let (a, b) = (mirror(k as isize - 1, n), mirror(k as isize + 1, n));
            x[k] -= (x[a] + x[b]) >> 1;
        }
        for k in (p0..x.len()).step_by(2) {
            let (a, b) = (mirror(k as isize - 1, n), mirror(k as isize + 1, n));
            x[k] += (x[a] + x[b] + 2) >> 2;
        }
    }

    #[test]
    fn the_53_lifting_round_trips_for_every_length_and_parity() {
        for p0 in 0..2 {
            for n in 1..40usize {
                let src: Vec<i32> = (0..n)
                    .map(|i| ((i * 7919 + 13) % 251) as i32 - 100)
                    .collect();
                let mut t = src.clone();
                forward_53(&mut t, p0);
                synth_53(&mut t, p0);
                assert_eq!(t, src, "n={n} p0={p0}");
            }
        }
    }

    #[test]
    fn a_level_of_a_flat_image_stays_flat() {
        // an 8 x 6 image of 100s: low-pass bands hold the value, detail bands are zero
        let rect = Rect {
            x0: 0,
            y0: 0,
            x1: 8,
            y1: 6,
        };
        let ll = vec![100i32; 4 * 3];
        let z = vec![0i32; 4 * 3];
        let out = inverse_53(rect, &ll, &z, &z, &z);
        assert!(out.iter().all(|&v| v == 100));
        let llf = vec![100f32; 12];
        let zf = vec![0f32; 12];
        let outf = inverse_97(rect, &llf, &zf, &zf, &zf);
        assert!(outf.iter().all(|&v| (v - 100.0).abs() < 1e-3), "{outf:?}");
    }
}
