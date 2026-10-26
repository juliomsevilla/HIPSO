//! Tier-1 decoding of JPEG 2000 (T.800 Annex C and D): the MQ arithmetic decoder and the three
//! coding passes (significance propagation, magnitude refinement, cleanup) that turn the bytes of
//! a code-block into its wavelet coefficients.
//!
//! Coefficients come out as signed magnitudes in units of half the last decoded bit, which is how
//! the reconstruction at the midpoint of the interval left by the missing bit-planes is kept exact
//! for integers: a reversible band divides by two, an irreversible one multiplies by half its step.

/// Code-block style bits (the `SPcod` byte).
pub const BYPASS: u8 = 0x01;
pub const RESET: u8 = 0x02;
pub const TERMALL: u8 = 0x04;
pub const CAUSAL: u8 = 0x08;
pub const SEGSYM: u8 = 0x20;

/// One codeword segment of a code-block and the passes it holds.
#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub data: Vec<u8>,
    pub passes: u32,
    pub max_passes: u32,
}

// ------------------------------------------------------------------------------------- MQ coder

/// (Qe, next state after a more probable symbol, after a less probable one, switch of the MPS).
const QE: [(u16, u8, u8, u8); 47] = [
    (0x5601, 1, 1, 1),
    (0x3401, 2, 6, 0),
    (0x1801, 3, 9, 0),
    (0x0AC1, 4, 12, 0),
    (0x0521, 5, 29, 0),
    (0x0221, 38, 33, 0),
    (0x5601, 7, 6, 1),
    (0x5401, 8, 14, 0),
    (0x4801, 9, 14, 0),
    (0x3801, 10, 14, 0),
    (0x3001, 11, 17, 0),
    (0x2401, 12, 18, 0),
    (0x1C01, 13, 20, 0),
    (0x1601, 29, 21, 0),
    (0x5601, 15, 14, 1),
    (0x5401, 16, 14, 0),
    (0x5101, 17, 15, 0),
    (0x4801, 18, 16, 0),
    (0x3801, 19, 17, 0),
    (0x3401, 20, 18, 0),
    (0x3001, 21, 19, 0),
    (0x2801, 22, 19, 0),
    (0x2401, 23, 20, 0),
    (0x2201, 24, 21, 0),
    (0x1C01, 25, 22, 0),
    (0x1801, 26, 23, 0),
    (0x1601, 27, 24, 0),
    (0x1401, 28, 25, 0),
    (0x1201, 29, 26, 0),
    (0x1101, 30, 27, 0),
    (0x0AC1, 31, 28, 0),
    (0x09C1, 32, 29, 0),
    (0x08A1, 33, 30, 0),
    (0x0521, 34, 31, 0),
    (0x0441, 35, 32, 0),
    (0x02A1, 36, 33, 0),
    (0x0221, 37, 34, 0),
    (0x0141, 38, 35, 0),
    (0x0111, 39, 36, 0),
    (0x0085, 40, 37, 0),
    (0x0049, 41, 38, 0),
    (0x0025, 42, 39, 0),
    (0x0015, 43, 40, 0),
    (0x0009, 44, 41, 0),
    (0x0005, 45, 42, 0),
    (0x0001, 45, 43, 0),
    (0x5601, 46, 46, 0),
];

const CTX_RUN: usize = 17;
const CTX_UNI: usize = 18;

/// The context states: bit 0 the more probable symbol, the rest the index into [`QE`].
type Contexts = [u8; 19];

fn reset_contexts(cx: &mut Contexts) {
    *cx = [0; 19];
    cx[0] = 4 << 1;
    cx[CTX_RUN] = 3 << 1;
    cx[CTX_UNI] = 46 << 1;
}

struct Mq<'a> {
    data: &'a [u8],
    bp: usize,
    c: u32,
    a: u32,
    ct: u32,
}

impl<'a> Mq<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut m = Mq {
            data,
            bp: 0,
            c: 0,
            a: 0,
            ct: 0,
        };
        m.c = u32::from(m.byte(0)) << 16;
        m.bytein();
        m.c <<= 7;
        m.ct -= 7;
        m.a = 0x8000;
        m
    }

    /// The byte at `i`; past the end the coder sees the 0xFF 0xFF of a marker, as the standard asks.
    #[inline]
    fn byte(&self, i: usize) -> u8 {
        self.data.get(i).copied().unwrap_or(0xFF)
    }

    #[inline]
    fn bytein(&mut self) {
        if self.byte(self.bp) == 0xFF {
            if self.byte(self.bp + 1) > 0x8F {
                self.c += 0xFF00;
                self.ct = 8;
            } else {
                self.bp += 1;
                self.c += u32::from(self.byte(self.bp)) << 9;
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            self.c += u32::from(self.byte(self.bp)) << 8;
            self.ct = 8;
        }
    }

    #[inline]
    fn decode(&mut self, cx: &mut u8) -> u32 {
        let idx = (*cx >> 1) as usize;
        let mut mps = u32::from(*cx & 1);
        let (qe, nmps, nlps, sw) = QE[idx];
        let qe = u32::from(qe);
        let next;
        let d;
        self.a -= qe;
        if (self.c >> 16) < qe {
            // the less probable sub-interval is at the bottom
            if self.a < qe {
                d = mps;
                next = nmps;
            } else {
                d = 1 ^ mps;
                if sw == 1 {
                    mps = d;
                }
                next = nlps;
            }
            self.a = qe;
        } else {
            self.c -= qe << 16;
            if self.a & 0x8000 != 0 {
                return mps;
            }
            if self.a < qe {
                d = 1 ^ mps;
                if sw == 1 {
                    mps = d;
                }
                next = nlps;
            } else {
                d = mps;
                next = nmps;
            }
        }
        // renormalisation
        loop {
            if self.ct == 0 {
                self.bytein();
            }
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.a & 0x8000 != 0 {
                break;
            }
        }
        *cx = (next << 1) | mps as u8;
        d
    }
}

/// Raw (bypass) bits, most significant first, with the stuffed bit after a 0xFF byte.
struct Raw<'a> {
    data: &'a [u8],
    pos: usize,
    cur: u32,
    left: u32,
}

impl<'a> Raw<'a> {
    fn new(data: &'a [u8]) -> Self {
        Raw {
            data,
            pos: 0,
            cur: 0,
            left: 0,
        }
    }

    fn bit(&mut self) -> u32 {
        if self.left == 0 {
            let prev_ff = self.cur == 0xFF;
            self.cur = u32::from(self.data.get(self.pos).copied().unwrap_or(0xFF));
            self.pos += 1;
            self.left = if prev_ff { 7 } else { 8 };
        }
        self.left -= 1;
        (self.cur >> self.left) & 1
    }
}

// ------------------------------------------------------------------------------- context tables

const SIG: u16 = 1 << 8;
const VISIT: u16 = 1 << 9;
const REFINED: u16 = 1 << 10;
const NEG: u16 = 1 << 11;
// neighbour significance bits 0..8
const N: u16 = 1;
const S: u16 = 2;
const W: u16 = 4;
const E: u16 = 8;
const NW: u16 = 16;
const NE: u16 = 32;
const SW: u16 = 64;
const SE: u16 = 128;
// signs of the four direct neighbours, kept beside the significance bits
const SN: u16 = 1 << 12;
const SS: u16 = 1 << 13;
const SW_: u16 = 1 << 14;
const SE_: u16 = 1 << 15;

/// Zero-coding context for the neighbour mask, by orientation class (0: LL and LH, 1: HL, 2: HH).
fn zc_table() -> [[u8; 256]; 3] {
    let mut t = [[0u8; 256]; 3];
    #[allow(clippy::needless_range_loop)]
    for m in 0..256usize {
        let m16 = m as u16;
        let h = u32::from(m16 & W != 0) + u32::from(m16 & E != 0);
        let v = u32::from(m16 & N != 0) + u32::from(m16 & S != 0);
        let d = u32::from(m16 & NW != 0)
            + u32::from(m16 & NE != 0)
            + u32::from(m16 & SW != 0)
            + u32::from(m16 & SE != 0);
        let ll = |h: u32, v: u32, d: u32| -> u8 {
            match (h, v, d) {
                (2, _, _) => 8,
                (1, v, _) if v >= 1 => 7,
                (1, 0, d) if d >= 1 => 6,
                (1, 0, _) => 5,
                (0, 2, _) => 4,
                (0, 1, _) => 3,
                (0, 0, d) if d >= 2 => 2,
                (0, 0, 1) => 1,
                _ => 0,
            }
        };
        t[0][m] = ll(h, v, d);
        t[1][m] = ll(v, h, d);
        let hv = h + v;
        t[2][m] = match (d, hv) {
            (d, _) if d >= 3 => 8,
            (2, hv) if hv >= 1 => 7,
            (2, _) => 6,
            (1, hv) if hv >= 2 => 5,
            (1, 1) => 4,
            (1, _) => 3,
            (_, hv) if hv >= 2 => 2,
            (_, 1) => 1,
            _ => 0,
        };
    }
    t
}

/// Sign-coding context and the bit to XOR, from the significance and sign of the four direct
/// neighbours, packed as `N S W E` significance (bits 0..4) and `N S W E` signs (bits 4..8).
fn sc_table() -> [(u8, u8); 256] {
    let mut t = [(9u8, 0u8); 256];
    for (m, slot) in t.iter_mut().enumerate() {
        let contrib = |sig: usize, sign: usize| -> i32 {
            if m & (1 << sig) == 0 {
                0
            } else if m & (1 << (4 + sign)) != 0 {
                -1
            } else {
                1
            }
        };
        let h = (contrib(2, 2) + contrib(3, 3)).clamp(-1, 1);
        let v = (contrib(0, 0) + contrib(1, 1)).clamp(-1, 1);
        *slot = match (h, v) {
            (1, 1) => (13, 0),
            (1, 0) => (12, 0),
            (1, -1) => (11, 0),
            (0, 1) => (10, 0),
            (0, 0) => (9, 0),
            (0, -1) => (10, 1),
            (-1, 1) => (11, 1),
            (-1, 0) => (12, 1),
            _ => (13, 1),
        };
    }
    t
}

struct Tables {
    zc: [[u8; 256]; 3],
    sc: [(u8, u8); 256],
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| Tables {
        zc: zc_table(),
        sc: sc_table(),
    })
}

// -------------------------------------------------------------------------------------- the block

struct Block {
    w: usize,
    h: usize,
    /// Padded by one on every side: `(w + 2) * (h + 2)`.
    flags: Vec<u16>,
    /// Magnitudes in half units.
    mag: Vec<u32>,
    zc: &'static [u8; 256],
    sc: &'static [(u8, u8); 256],
    causal: bool,
}

impl Block {
    #[inline]
    fn fi(&self, x: usize, y: usize) -> usize {
        (y + 1) * (self.w + 2) + x + 1
    }

    /// The neighbour significance mask of a coefficient, with the next stripe left out when the
    /// block asks for vertically causal contexts.
    #[inline]
    fn neighbours(&self, x: usize, y: usize) -> u16 {
        let f = self.flags[self.fi(x, y)];
        if self.causal && y % 4 == 3 {
            f & !(S | SW | SE | SS)
        } else {
            f
        }
    }

    #[inline]
    fn zc_ctx(&self, f: u16) -> usize {
        usize::from(self.zc[(f & 0xFF) as usize])
    }

    #[inline]
    fn sc_ctx(&self, f: u16) -> (usize, u32) {
        let m = (f & (N | S | W | E)) as usize | ((f >> 8) & 0xF0) as usize;
        let (c, x) = self.sc[m];
        (usize::from(c), u32::from(x))
    }

    /// Makes (x, y) significant, updating the neighbour bits of the eight around it.
    fn set_significant(&mut self, x: usize, y: usize, negative: bool) {
        let stride = self.w + 2;
        let i = self.fi(x, y);
        self.flags[i] |= SIG | if negative { NEG } else { 0 };
        let sgn = |bit: u16| if negative { bit } else { 0 };
        self.flags[i - stride] |= S | sgn(SS);
        self.flags[i + stride] |= N | sgn(SN);
        self.flags[i - 1] |= E | sgn(SE_);
        self.flags[i + 1] |= W | sgn(SW_);
        self.flags[i - stride - 1] |= SE;
        self.flags[i - stride + 1] |= SW;
        self.flags[i + stride - 1] |= NE;
        self.flags[i + stride + 1] |= NW;
    }
}

/// What the coding passes read the bits from.
enum Source<'a> {
    Mq(Mq<'a>),
    Raw(Raw<'a>),
}

/// Decodes a code-block. `numbps` is the number of magnitude bit-planes that were coded (the
/// band's `Mb` less the zero bit-planes signalled); `orient` is 0 for LL, 1 HL, 2 LH, 3 HH.
/// Returns `w * h` signed values in half units (see the module description).
pub fn decode_block(
    w: usize,
    h: usize,
    orient: u8,
    style: u8,
    numbps: u32,
    roishift: u32,
    segments: &[Segment],
) -> Vec<i32> {
    let t = tables();
    let class = match orient {
        1 => 1,
        3 => 2,
        _ => 0,
    };
    let mut b = Block {
        w,
        h,
        flags: vec![0; (w + 2) * (h + 2)],
        mag: vec![0; w * h],
        zc: &t.zc[class],
        sc: &t.sc,
        causal: style & CAUSAL != 0,
    };
    let total_planes = numbps + roishift;
    if total_planes == 0 || total_planes > 31 || segments.is_empty() {
        return vec![0; w * h];
    }
    let mut cx: Contexts = [0; 19];
    reset_contexts(&mut cx);
    let mut bpno = total_planes as i32 - 1;
    let mut pass = 2u32;
    'segments: for seg in segments {
        if seg.passes == 0 {
            continue;
        }
        let raw = style & BYPASS != 0 && pass < 2 && bpno <= total_planes as i32 - 5;
        let mut src = if raw {
            Source::Raw(Raw::new(&seg.data))
        } else {
            Source::Mq(Mq::new(&seg.data))
        };
        for _ in 0..seg.passes {
            if bpno < 0 {
                break 'segments;
            }
            match pass {
                0 => sig_pass(&mut b, &mut cx, &mut src, bpno as u32),
                1 => ref_pass(&mut b, &mut cx, &mut src, bpno as u32),
                _ => {
                    clean_pass(&mut b, &mut cx, &mut src, bpno as u32);
                    if style & SEGSYM != 0
                        && let Source::Mq(m) = &mut src
                    {
                        for _ in 0..4 {
                            m.decode(&mut cx[CTX_UNI]);
                        }
                    }
                }
            }
            if style & RESET != 0 {
                reset_contexts(&mut cx);
            }
            pass += 1;
            if pass == 3 {
                pass = 0;
                bpno -= 1;
            }
        }
    }
    // region of interest: magnitudes at or above the shift are scaled down
    let thresh = if roishift > 0 {
        1u32 << (roishift + 1)
    } else {
        0
    };
    let mut out = vec![0i32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut m = b.mag[y * w + x];
            if roishift > 0 && m >= thresh {
                m >>= roishift;
            }
            let v = m as i32;
            out[y * w + x] = if b.flags[b.fi(x, y)] & NEG != 0 {
                -v
            } else {
                v
            };
        }
    }
    out
}

#[inline]
fn bit(src: &mut Source, cx: &mut u8) -> u32 {
    match src {
        Source::Mq(m) => m.decode(cx),
        Source::Raw(r) => r.bit(),
    }
}

/// Decodes the sign of a newly significant coefficient.
#[inline]
fn sign(b: &Block, src: &mut Source, cx: &mut Contexts, x: usize, y: usize) -> bool {
    match src {
        Source::Raw(r) => r.bit() == 1,
        Source::Mq(m) => {
            let f = b.neighbours(x, y);
            let (c, xor) = b.sc_ctx(f);
            (m.decode(&mut cx[c]) ^ xor) == 1
        }
    }
}

fn sig_pass(b: &mut Block, cx: &mut Contexts, src: &mut Source, bpno: u32) {
    let (w, h) = (b.w, b.h);
    let one = 3u32 << bpno;
    for y0 in (0..h).step_by(4) {
        for x in 0..w {
            for y in y0..(y0 + 4).min(h) {
                let i = b.fi(x, y);
                let f = b.neighbours(x, y);
                if b.flags[i] & (SIG | VISIT) != 0 || f & 0xFF == 0 {
                    continue;
                }
                let c = b.zc_ctx(f);
                let v = bit(src, &mut cx[c]);
                if v == 1 {
                    let neg = sign(b, src, cx, x, y);
                    b.mag[y * w + x] = one;
                    b.set_significant(x, y, neg);
                }
                b.flags[i] |= VISIT;
            }
        }
    }
}

fn ref_pass(b: &mut Block, cx: &mut Contexts, src: &mut Source, bpno: u32) {
    let (w, h) = (b.w, b.h);
    let delta = 1u32 << bpno;
    for y0 in (0..h).step_by(4) {
        for x in 0..w {
            for y in y0..(y0 + 4).min(h) {
                let i = b.fi(x, y);
                let f0 = b.flags[i];
                if f0 & SIG == 0 || f0 & VISIT != 0 {
                    continue;
                }
                let c = if f0 & REFINED != 0 {
                    16
                } else if b.neighbours(x, y) & 0xFF != 0 {
                    15
                } else {
                    14
                };
                let v = bit(src, &mut cx[c]);
                let m = &mut b.mag[y * w + x];
                if v == 1 {
                    *m += delta;
                } else {
                    *m -= delta;
                }
                b.flags[i] |= REFINED;
            }
        }
    }
}

fn clean_pass(b: &mut Block, cx: &mut Contexts, src: &mut Source, bpno: u32) {
    let (w, h) = (b.w, b.h);
    let one = 3u32 << bpno;
    for y0 in (0..h).step_by(4) {
        for x in 0..w {
            let mut y = y0;
            let full = y0 + 4 <= h;
            // run-length mode: four rows, none significant, visited or with a significant neighbour
            if full {
                let clear = (0..4).all(|k| {
                    let f = b.neighbours(x, y0 + k);
                    b.flags[b.fi(x, y0 + k)] & (SIG | VISIT) == 0 && f & 0xFF == 0
                });
                if clear {
                    let Source::Mq(m) = src else {
                        // cleanup passes are always arithmetic coded
                        return;
                    };
                    if m.decode(&mut cx[CTX_RUN]) == 0 {
                        continue;
                    }
                    let first = (m.decode(&mut cx[CTX_UNI]) << 1) | m.decode(&mut cx[CTX_UNI]);
                    y = y0 + first as usize;
                    let neg = sign(b, src, cx, x, y);
                    b.mag[y * w + x] = one;
                    b.set_significant(x, y, neg);
                    y += 1;
                }
            }
            for yy in y..(y0 + 4).min(h) {
                let i = b.fi(x, yy);
                if b.flags[i] & (SIG | VISIT) != 0 {
                    continue;
                }
                let f = b.neighbours(x, yy);
                let c = b.zc_ctx(f);
                if bit(src, &mut cx[c]) == 1 {
                    let neg = sign(b, src, cx, x, yy);
                    b.mag[yy * w + x] = one;
                    b.set_significant(x, yy, neg);
                }
            }
        }
    }
    // the visit marks only last for one bit-plane
    for f in &mut b.flags {
        *f &= !VISIT;
    }
}
