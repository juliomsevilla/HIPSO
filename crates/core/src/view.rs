//! What a point-cloud viewer needs, with no window in it: a compact in-memory store of
//! (decimated) points, colouring and filters, and CPU renderers for the plan, 3D and profile
//! views. The GUI paints the images these make; `easylidar render` writes them as PNG.

use std::path::{Path, PathBuf};

use crate::classes::ClassSet;
use crate::grid::Grid;
use crate::las::Reader;
use crate::{Error, Result};

/// Points kept in memory, 21 bytes each, with coordinates relative to `origin` in f32.
#[derive(Clone, Debug, Default)]
pub struct PointStore {
    pub origin: [f64; 3],
    /// min x, min y, min z, max x, max y, max z of the kept points.
    pub bounds: [f64; 6],
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub z: Vec<f32>,
    pub class: Vec<u8>,
    pub intensity: Vec<u16>,
    /// Return number in the low nibble, number of returns in the high one.
    pub ret: Vec<u8>,
    pub src: Vec<u16>,
    /// Scan angle in tenths of a degree (0 when the file has none).
    pub scan: Vec<i16>,
    pub file: Vec<u8>,
    /// Empty when no file carries colour.
    pub rgb: Vec<[u8; 3]>,
    pub files: Vec<String>,
    /// Selected points (empty: none selected) and points removed by edits (empty: none).
    pub selected: Vec<bool>,
    pub deleted: Vec<bool>,
    /// Points in the files, and one kept in every `stride`.
    pub total: u64,
    pub stride: u64,
    /// The coordinate system of the first cloud as `.prj` text (for files made from it).
    pub prj: Option<String>,
}

impl PointStore {
    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// Absolute coordinates of point `i`.
    pub fn xyz(&self, i: usize) -> [f64; 3] {
        [
            self.origin[0] + f64::from(self.x[i]),
            self.origin[1] + f64::from(self.y[i]),
            self.origin[2] + f64::from(self.z[i]),
        ]
    }

    pub fn intensity_max(&self) -> u16 {
        let mut v: Vec<u16> = self
            .intensity
            .iter()
            .step_by((self.len() / 50_000).max(1))
            .copied()
            .collect();
        v.sort_unstable();
        v.get(v.len() * 99 / 100).copied().unwrap_or(1).max(1)
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

/// x, y, z, intensity, return byte, class, source id, file id, colour.
type Staged = (f64, f64, f64, u16, u8, u8, u16, u8, Option<[u8; 3]>, i16);

/// Reads LAS/LAZ (and ASCII `x y z [intensity [class]]`) files keeping about `max_points` of
/// them: one point in every `stride`, so large clouds fit in memory.
pub fn load(paths: &[PathBuf], max_points: usize) -> Result<PointStore> {
    let mut readers = Vec::new();
    let mut ascii = Vec::new();
    let mut total = 0u64;
    for p in paths {
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if ext == "las" || ext == "laz" {
            let r = Reader::open(p)?;
            total += r.header.count;
            readers.push(r);
        } else {
            let size = std::fs::metadata(p)?.len();
            total += size / 28;
            ascii.push(p.clone());
        }
    }
    if total == 0 {
        return Err(Error::Param("no points to show".into()));
    }
    let stride = total.div_ceil(max_points.max(1) as u64).max(1);
    let mut s = PointStore {
        stride,
        total,
        ..PointStore::default()
    };
    s.prj = readers.first().and_then(|r| {
        r.header
            .epsg
            .and_then(crate::vector::prj_for_epsg)
            .or_else(|| {
                r.meta
                    .wkt
                    .clone()
                    .filter(|w| w.starts_with("PROJCS") || w.starts_with("GEOGCS"))
            })
    });
    let first = readers.first().map(|r| r.header.bounds);
    s.origin = first.map_or([0.0; 3], |b| {
        [(b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0, b[2]]
    });
    let mut n = 0u64;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let has_rgb = readers
        .iter()
        .any(|r| matches!(r.header.format, 2 | 3 | 5 | 7 | 8 | 10));
    let mut push = |s: &mut PointStore, p: Staged| {
        if lo[0].is_infinite() && s.origin == [0.0; 3] {
            s.origin = [p.0, p.1, p.2];
        }
        s.x.push((p.0 - s.origin[0]) as f32);
        s.y.push((p.1 - s.origin[1]) as f32);
        s.z.push((p.2 - s.origin[2]) as f32);
        s.intensity.push(p.3);
        s.ret.push(p.4);
        s.class.push(p.5);
        s.src.push(p.6);
        s.scan.push(p.9);
        s.file.push(p.7);
        if has_rgb {
            s.rgb.push(p.8.unwrap_or([150, 150, 150]));
        }
        for k in 0..3 {
            let v = [p.0, p.1, p.2][k];
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    };
    for (fi, r) in readers.iter().enumerate() {
        let h = &r.header;
        let rec_len = usize::from(h.record_len);
        let (src_at, rgb_at) = (
            if h.format >= 6 { 20 } else { 18 },
            match h.format {
                2 => Some(20),
                3 | 5 => Some(28),
                7 | 8 | 10 => Some(30),
                _ => None,
            },
        );
        s.files.push(
            r.path()
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned()),
        );
        let file_id = fi.min(255) as u8;
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                n += 1;
                if !(n - 1).is_multiple_of(stride) {
                    continue;
                }
                let Some(p) = r.decode(rec) else { continue };
                let rgb = rgb_at.map(|a| {
                    let c = |k: usize| (u16_at(rec, a + 2 * k) >> 8) as u8;
                    [c(0), c(1), c(2)]
                });
                push(
                    &mut s,
                    (
                        p.x,
                        p.y,
                        p.z,
                        p.intensity,
                        p.return_number | (p.number_of_returns << 4),
                        p.class,
                        u16_at(rec, src_at),
                        file_id,
                        rgb,
                        (r.decode_extra(rec).scan_angle * 10.0).round() as i16,
                    ),
                );
            }
            Ok(())
        })?;
    }
    for (fi, p) in ascii.iter().enumerate() {
        let id = (readers.len() + fi).min(255) as u8;
        s.files.push(
            p.file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned()),
        );
        let text = std::fs::read_to_string(p)?;
        for line in text.lines() {
            let f: Vec<f64> = line
                .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                .filter(|t| !t.is_empty())
                .map_while(|t| t.parse().ok())
                .collect();
            if f.len() < 3 {
                continue;
            }
            n += 1;
            if !(n - 1).is_multiple_of(stride) {
                continue;
            }
            push(
                &mut s,
                (
                    f[0],
                    f[1],
                    f[2],
                    f.get(3).map_or(0, |v| *v as u16),
                    0x11,
                    f.get(4).map_or(0, |v| *v as u8),
                    0,
                    id,
                    None,
                    0,
                ),
            );
        }
    }
    if s.is_empty() {
        return Err(Error::Param("no points to show".into()));
    }
    s.bounds = [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]];
    Ok(s)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorBy {
    Elevation,
    ElevationEarth,
    Class,
    Intensity,
    Return,
    Source,
    File,
    Rgb,
}

/// The colour of selected points.
pub const HIGHLIGHT: [u8; 3] = [0, 255, 255];

/// Which return filter bit a return byte (number low, count high) falls in: 1 first,
/// 2 intermediate, 4 last, 8 single.
pub fn return_bit(ret: u8) -> u8 {
    let (n, t) = (ret & 15, ret >> 4);
    if t <= 1 && n <= 1 {
        8
    } else if n <= 1 {
        1
    } else if n >= t {
        4
    } else {
        2
    }
}

fn ramp(t: f32, earth: bool) -> [u8; 3] {
    const BLUE_RED: [[f32; 3]; 5] = [
        [40.0, 60.0, 220.0],
        [30.0, 190.0, 230.0],
        [70.0, 210.0, 70.0],
        [250.0, 220.0, 40.0],
        [225.0, 40.0, 30.0],
    ];
    const EARTH: [[f32; 3]; 5] = [
        [40.0, 90.0, 60.0],
        [120.0, 150.0, 70.0],
        [200.0, 180.0, 100.0],
        [160.0, 110.0, 70.0],
        [250.0, 250.0, 250.0],
    ];
    let st = if earth { &EARTH } else { &BLUE_RED };
    let t = t.clamp(0.0, 1.0) * 4.0;
    let i = (t.floor() as usize).min(3);
    let f = t - i as f32;
    let c = |k: usize| (st[i][k] + (st[i + 1][k] - st[i][k]) * f) as u8;
    [c(0), c(1), c(2)]
}

/// How points are coloured and which are drawn.
#[derive(Clone, Debug)]
pub struct Style {
    pub color_by: ColorBy,
    /// Elevation range of the colour ramp.
    pub zmin: f64,
    pub zmax: f64,
    pub intensity_max: u16,
    pub point_size: usize,
    /// Profile dots as a share of the spacing between points on screen (1: touching).
    pub profile_scale: f64,
    /// The classes that are drawn.
    pub classes: ClassSet,
    pub class_colors: [[u8; 3]; 256],
    /// Bits: 1 first, 2 intermediate, 4 last, 8 single returns are drawn.
    pub returns: u8,
    /// Bit k set: the points of file k are drawn (files from the 65th on always are).
    pub files: u64,
    pub background: [u8; 3],
    /// Draw only every `thin`-th point (fast, while the view moves).
    pub thin: usize,
    /// Leave the background transparent (to paint the cloud over reference layers).
    pub transparent: bool,
    /// Typical distance between displayed points, in map units: points are drawn as big as
    /// this when zoomed in, so the surface never turns into scattered specks.
    pub spacing: f64,
    /// Draw only points inside this rectangle `[xmin, ymin, xmax, ymax]` (3D view).
    pub window: Option<[f64; 4]>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color_by: ColorBy::Elevation,
            zmin: 0.0,
            zmax: 1.0,
            intensity_max: 255,
            point_size: 1,
            profile_scale: 0.5,
            classes: ClassSet::all(),
            class_colors: crate::classes::default_colors(),
            returns: 15,
            files: u64::MAX,
            background: [18, 21, 26],
            thin: 1,
            transparent: false,
            window: None,
            spacing: 0.0,
        }
    }
}

impl Style {
    /// A style whose ramps span the store.
    pub fn fit(s: &PointStore) -> Self {
        let area = ((s.bounds[3] - s.bounds[0]) * (s.bounds[4] - s.bounds[1])).max(1.0) * 0.5;
        Self {
            spacing: (area / s.len().max(1) as f64).sqrt(),
            zmin: s.bounds[2],
            zmax: s.bounds[5],
            intensity_max: s.intensity_max(),
            color_by: if s.rgb.is_empty() {
                ColorBy::Elevation
            } else {
                ColorBy::Rgb
            },
            ..Self::default()
        }
    }

    pub fn visible(&self, s: &PointStore, i: usize) -> bool {
        if s.deleted.get(i).copied().unwrap_or(false) {
            return false;
        }
        if s.file[i] < 64 && self.files & (1u64 << s.file[i]) == 0 {
            return false;
        }
        if !self.classes.contains(s.class[i]) {
            return false;
        }
        self.returns & return_bit(s.ret[i]) != 0
    }

    pub fn color(&self, s: &PointStore, i: usize) -> [u8; 3] {
        if s.selected.get(i).copied().unwrap_or(false) {
            return HIGHLIGHT;
        }
        match self.color_by {
            ColorBy::Elevation | ColorBy::ElevationEarth => {
                let z = s.origin[2] + f64::from(s.z[i]);
                ramp(
                    ((z - self.zmin) / (self.zmax - self.zmin).max(1e-9)) as f32,
                    self.color_by == ColorBy::ElevationEarth,
                )
            }
            ColorBy::Class => self.class_colors[usize::from(s.class[i])],
            ColorBy::Intensity => {
                let g = (f32::from(s.intensity[i]) / f32::from(self.intensity_max.max(1)))
                    .clamp(0.0, 1.0);
                let v = (g * 255.0) as u8;
                [v, v, v]
            }
            ColorBy::Return => match (s.ret[i] & 15, s.ret[i] >> 4) {
                (1, 1) => [240, 240, 240],
                (1, _) => [60, 200, 60],
                (n, t) if n >= t => [230, 60, 60],
                _ => [240, 200, 40],
            },
            ColorBy::Source => hash_color(u32::from(s.src[i])),
            ColorBy::File => hash_color(u32::from(s.file[i]) + 7),
            ColorBy::Rgb => s.rgb.get(i).copied().unwrap_or([200, 200, 200]),
        }
    }
}

fn hash_color(k: u32) -> [u8; 3] {
    let h = (k.wrapping_mul(2654435761) >> 8) as f32 / 16777216.0;
    let (r, g, b) = hsv(h, 0.65, 0.95);
    [r, g, b]
}

fn hsv(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// A rendered image, RGBA8, row 0 at the top.
#[derive(Clone, Debug)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn filled(w: usize, h: usize, c: [u8; 3]) -> Self {
        let mut rgba = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        Self { w, h, rgba }
    }

    pub fn set(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            let i = (y as usize * self.w + x as usize) * 4;
            self.rgba[i..i + 3].copy_from_slice(&c);
        }
    }

    pub fn write_png(&self, path: &Path) -> Result<()> {
        let f = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(f, self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc
            .write_header()
            .map_err(|e| Error::Format(e.to_string()))?;
        w.write_image_data(&self.rgba)
            .map_err(|e| Error::Format(e.to_string()))
    }
}

/// The plan view: map centre and pixels per map unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub cx: f64,
    pub cy: f64,
    pub scale: f64,
}

impl Plan {
    /// The view that shows `b` (min x, min y, ..., max x, max y) in a `w` x `h` window.
    pub fn fit(b: [f64; 6], w: usize, h: usize) -> Self {
        let (dx, dy) = ((b[3] - b[0]).max(1e-6), (b[4] - b[1]).max(1e-6));
        Self {
            cx: (b[0] + b[3]) / 2.0,
            cy: (b[1] + b[4]) / 2.0,
            scale: (w as f64 / dx).min(h as f64 / dy) * 0.96,
        }
    }

    pub fn to_screen(&self, x: f64, y: f64, w: usize, h: usize) -> (f64, f64) {
        (
            (x - self.cx) * self.scale + w as f64 / 2.0,
            (self.cy - y) * self.scale + h as f64 / 2.0,
        )
    }

    pub fn to_map(&self, px: f64, py: f64, w: usize, h: usize) -> (f64, f64) {
        (
            self.cx + (px - w as f64 / 2.0) / self.scale,
            self.cy - (py - h as f64 / 2.0) / self.scale,
        )
    }

    /// Zooms by `k` keeping the map point under pixel `(px, py)` fixed.
    pub fn zoom_at(&mut self, k: f64, px: f64, py: f64, w: usize, h: usize) {
        let (mx, my) = self.to_map(px, py, w, h);
        self.scale *= k;
        let (nx, ny) = self.to_map(px, py, w, h);
        self.cx += mx - nx;
        self.cy += my - ny;
    }
}

/// One thread's share of a render: the nearest point per pixel and its colour.
struct Layer {
    w: usize,
    h: usize,
    depth: Vec<f32>,
    rgb: Vec<[u8; 3]>,
}

impl Layer {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            depth: vec![f32::INFINITY; w * h],
            rgb: vec![[0; 3]; w * h],
        }
    }

    fn splat(&mut self, x: i64, y: i64, d: f32, size: i64, c: [u8; 3]) {
        let r0 = (size - 1) / 2;
        // dots from 3 pixels up are round, not square
        let (mid, rad2) = ((size as f64 - 1.0) / 2.0, (size as f64 / 2.0).powi(2));
        for yy in (y - r0).max(0)..(y - r0 + size).min(self.h as i64) {
            for xx in (x - r0).max(0)..(x - r0 + size).min(self.w as i64) {
                if size >= 3 {
                    let (dx, dy) = ((xx - (x - r0)) as f64 - mid, (yy - (y - r0)) as f64 - mid);
                    if dx * dx + dy * dy > rad2 {
                        continue;
                    }
                }
                let k = yy as usize * self.w + xx as usize;
                if d < self.depth[k] {
                    self.depth[k] = d;
                    self.rgb[k] = c;
                }
            }
        }
    }

    fn merge(mut self, o: Layer) -> Layer {
        for k in 0..self.depth.len() {
            if o.depth[k] < self.depth[k] {
                self.depth[k] = o.depth[k];
                self.rgb[k] = o.rgb[k];
            }
        }
        self
    }
}

/// Runs `f(layer, point index)` over every `step`-th point on all cores and paints the nearest
/// points over `img`.
fn par_points(img: &mut Image, n: usize, step: usize, f: impl Fn(&mut Layer, usize) + Sync + Send) {
    use rayon::prelude::*;
    let (w, h) = (img.w, img.h);
    let count = n / step.max(1);
    let min = (count / (rayon::current_num_threads() * 2)).max(20_000);
    let layer = (0..count)
        .into_par_iter()
        .with_min_len(min)
        .fold(
            || Layer::new(w, h),
            |mut l, k| {
                f(&mut l, k * step.max(1));
                l
            },
        )
        .reduce_with(Layer::merge);
    if let Some(l) = layer {
        for k in 0..w * h {
            if l.depth[k].is_finite() {
                img.rgba[k * 4..k * 4 + 3].copy_from_slice(&l.rgb[k]);
                img.rgba[k * 4 + 3] = 255;
            }
        }
    }
}

fn canvas(w: usize, h: usize, st: &Style) -> Image {
    if st.transparent {
        Image {
            w,
            h,
            rgba: vec![0; w * h * 4],
        }
    } else {
        Image::filled(w, h, st.background)
    }
}

/// Dot size in pixels: at least the chosen one, and as big as the spacing between points
/// looks on screen (`px_per_unit` pixels per map unit).
fn dot(st: &Style, px_per_unit: f64) -> i64 {
    ((st.spacing * px_per_unit * 0.9).ceil() as i64).clamp(st.point_size.max(1) as i64, 24)
}

/// The plan view, highest point on top.
pub fn render_plan(s: &PointStore, v: &Plan, w: usize, h: usize, st: &Style) -> Image {
    let mut img = canvas(w, h, st);
    let size = dot(st, v.scale);
    par_points(&mut img, s.len(), st.thin.max(1), |l, i| {
        let px = ((f64::from(s.x[i]) + s.origin[0] - v.cx) * v.scale + w as f64 / 2.0).floor();
        let py = ((v.cy - (f64::from(s.y[i]) + s.origin[1])) * v.scale + h as f64 / 2.0).floor();
        if px < -size as f64
            || py < -size as f64
            || px >= (w as i64 + size) as f64
            || py >= (h as i64 + size) as f64
            || !st.visible(s, i)
        {
            return;
        }
        l.splat(px as i64, py as i64, -s.z[i], size, st.color(s, i));
    });
    img
}

/// A perspective camera orbiting a target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub target: [f64; 3],
    /// Rotation about the vertical axis, radians (0: looking north).
    pub yaw: f64,
    /// Elevation above the horizon, radians.
    pub pitch: f64,
    pub dist: f64,
    pub exaggeration: f64,
}

impl Orbit {
    pub fn fit(b: [f64; 6]) -> Self {
        let d = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2))
            .sqrt()
            .max(1.0);
        Self {
            target: [
                (b[0] + b[3]) / 2.0,
                (b[1] + b[4]) / 2.0,
                (b[2] + b[5]) / 2.0,
            ],
            yaw: 0.0,
            pitch: 0.6,
            dist: d * 1.1,
            exaggeration: 1.0,
        }
    }
}

impl Orbit {
    /// Focal length in pixels of a view `w`×`h`.
    fn focal(w: usize, h: usize) -> f64 {
        h.min(w) as f64 * 1.1
    }

    /// Where a map point lands on a view `w`×`h`, or `None` behind the camera.
    pub fn project(&self, p: [f64; 3], w: usize, h: usize) -> Option<(f64, f64)> {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (dx, dy, dz) = (
            p[0] - self.target[0],
            p[1] - self.target[1],
            (p[2] - self.target[2]) * self.exaggeration,
        );
        let (x1, y1) = (dx * cy - dy * sy, dx * sy + dy * cy);
        let depth = y1 * cp - dz * sp + self.dist;
        if depth < 1e-3 {
            return None;
        }
        let f = Self::focal(w, h);
        Some((
            x1 * f / depth + w as f64 / 2.0,
            -(y1 * sp + dz * cp) * f / depth + h as f64 / 2.0,
        ))
    }

    /// Drags the scene by `(dx, dy)` pixels in the plane of the screen: what lies at the
    /// distance of the target follows the cursor exactly.
    pub fn pan_px(&mut self, dx: f64, dy: f64, w: usize, h: usize) {
        let k = self.dist / Self::focal(w, h);
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let right = [cy, -sy, 0.0];
        let up = [sy * sp, cy * sp, cp / self.exaggeration.max(1e-6)];
        for i in 0..3 {
            self.target[i] += (-dx * right[i] + dy * up[i]) * k;
        }
    }
}

pub fn render_3d(s: &PointStore, o: &Orbit, w: usize, h: usize, st: &Style) -> Image {
    let mut img = canvas(w, h, st);
    let (sy, cy) = o.yaw.sin_cos();
    let (sp, cp) = o.pitch.sin_cos();
    let f = Orbit::focal(w, h);
    par_points(&mut img, s.len(), st.thin.max(1), |l, i| {
        if !st.visible(s, i) {
            return;
        }
        let p = s.xyz(i);
        if let Some(w3) = st.window
            && (p[0] < w3[0] || p[0] > w3[2] || p[1] < w3[1] || p[1] > w3[3])
        {
            return;
        }
        let (dx, dy, dz) = (
            p[0] - o.target[0],
            p[1] - o.target[1],
            (p[2] - o.target[2]) * o.exaggeration,
        );
        // yaw about z, then pitch about x; the camera sits o.dist away behind +y rotated
        let (x1, y1) = (dx * cy - dy * sy, dx * sy + dy * cy);
        let depth_axis = y1 * cp - dz * sp + o.dist;
        let up = y1 * sp + dz * cp;
        if depth_axis < 1e-3 {
            return;
        }
        let px = (x1 * f / depth_axis + w as f64 / 2.0).floor();
        let py = (-up * f / depth_axis + h as f64 / 2.0).floor();
        if px < 0.0 || py < 0.0 || px >= w as f64 || py >= h as f64 {
            return;
        }
        // nearer points look bigger: the size follows the perspective
        l.splat(
            px as i64,
            py as i64,
            depth_axis as f32,
            dot(st, f / depth_axis),
            st.color(s, i),
        );
    });
    img
}

/// A profile slice: the strip of `width` map units along a→b.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileBox {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub width: f64,
}

impl ProfileBox {
    pub fn length(&self) -> f64 {
        ((self.b[0] - self.a[0]).powi(2) + (self.b[1] - self.a[1]).powi(2)).sqrt()
    }

    /// (distance along, distance across) of a map point.
    pub fn local(&self, x: f64, y: f64) -> (f64, f64) {
        let l = self.length().max(1e-9);
        let (ux, uy) = ((self.b[0] - self.a[0]) / l, (self.b[1] - self.a[1]) / l);
        let (dx, dy) = (x - self.a[0], y - self.a[1]);
        (dx * ux + dy * uy, -dx * uy + dy * ux)
    }

    /// Moves the box across itself (positive: to the left of a→b).
    pub fn shifted(&self, d: f64) -> Self {
        let l = self.length().max(1e-9);
        let (nx, ny) = (
            -(self.b[1] - self.a[1]) / l * d,
            (self.b[0] - self.a[0]) / l * d,
        );
        Self {
            a: [self.a[0] + nx, self.a[1] + ny],
            b: [self.b[0] + nx, self.b[1] + ny],
            width: self.width,
        }
    }
}

/// The profile: distance along on the horizontal axis, elevation on the vertical one. Returns
/// the image and the elevation range shown.
pub fn render_profile(
    s: &PointStore,
    bx: &ProfileBox,
    w: usize,
    h: usize,
    st: &Style,
) -> (Image, f64, f64) {
    use rayon::prelude::*;
    let mut img = Image::filled(w, h, st.background);
    let len = bx.length().max(1e-9);
    let half = bx.width / 2.0;
    let inside: Vec<(usize, f64, f64, f64)> = (0..s.len())
        .into_par_iter()
        .filter_map(|i| {
            let p = s.xyz(i);
            let (u, t) = bx.local(p[0], p[1]);
            (u >= 0.0 && u <= len && t.abs() <= half && st.visible(s, i)).then_some((
                i,
                u,
                p[2],
                t.abs(),
            ))
        })
        .collect();
    if inside.is_empty() {
        return (img, 0.0, 1.0);
    }
    let (zlo, zhi) = inside
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| {
            (a.0.min(p.2), a.1.max(p.2))
        });
    let pad = (zhi - zlo).max(1.0) * 0.05;
    let (zlo, zhi) = (zlo - pad, zhi + pad);
    // equal scale on both axes, centred
    let k = (w as f64 / len).min(h as f64 / (zhi - zlo));
    let (ox, oy) = (
        (w as f64 - len * k) / 2.0,
        (h as f64 - (zhi - zlo) * k) / 2.0,
    );
    // the spacing of points in this slice, so zooming in grows the dots instead of leaving specks
    let slice_spacing = (len * (zhi - zlo) / inside.len() as f64).sqrt();
    let size = ((slice_spacing * k * st.profile_scale).round() as i64).clamp(1, 24);
    let mut layer = Layer::new(w, h);
    for (i, u, z, t) in inside {
        let px = (ox + u * k).floor() as i64;
        let py = (h as f64 - oy - (z - zlo) * k).floor() as i64;
        layer.splat(px, py, t as f32, size, st.color(s, i));
    }
    for q in 0..w * h {
        if layer.depth[q].is_finite() {
            img.rgba[q * 4..q * 4 + 3].copy_from_slice(&layer.rgb[q]);
            img.rgba[q * 4 + 3] = 255;
        }
    }
    (img, zlo, zhi)
}

/// Index of the point nearest to `(x, y)` in plan, within `tol` map units, highest first.
pub fn nearest_point(s: &PointStore, st: &Style, x: f64, y: f64, tol: f64) -> Option<usize> {
    let mut best: Option<(usize, f64, f32)> = None;
    for i in 0..s.len() {
        let p = s.xyz(i);
        let d = (p[0] - x).hypot(p[1] - y);
        if d <= tol
            && st.visible(s, i)
            && best.is_none_or(|b| d < b.1 - tol * 0.2 || (d <= b.1 + tol * 0.2 && s.z[i] > b.2))
        {
            best = Some((i, d, s.z[i]));
        }
    }
    best.map(|b| b.0)
}

/// Points per square map unit inside the rectangle `[x0, y0, x1, y1]`: (points, area, density)
/// as the store counts them, scaled back by the decimation.
pub fn density(s: &PointStore, st: &Style, r: [f64; 4]) -> (u64, f64, f64) {
    let n = (0..s.len())
        .filter(|&i| {
            let p = s.xyz(i);
            p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3] && st.visible(s, i)
        })
        .count() as u64
        * s.stride;
    let area = (r[2] - r[0]).abs() * (r[3] - r[1]).abs();
    (n, area, if area > 0.0 { n as f64 / area } else { 0.0 })
}

/// A surface grid from the points whose class is in `classes`: the TIN of those
/// points, rasterised at about `cells` cells across the longer side.
pub fn surface(s: &PointStore, classes: ClassSet, cells: usize) -> Result<Grid> {
    let b = s.bounds;
    let cell = ((b[3] - b[0]).max(b[4] - b[1]) / cells.max(16) as f64).max(1e-6);
    let mut g = Grid::covering([b[0], b[1], b[3], b[4]], cell)?;
    let pts: Vec<[f64; 3]> = (0..s.len())
        .filter(|&i| classes.contains(s.class[i]))
        .map(|i| s.xyz(i))
        .collect();
    crate::tin::rasterize_with(&pts, &mut g, cell * 60.0, crate::tin::Thin::Nearest);
    Ok(g)
}

/// The surface shaded from one direction and coloured by elevation, grey or intensity-free
/// grey: RGBA pixels at the grid's resolution plus a flag per colour mode.
pub fn shade(g: &Grid, st: &Style, azimuth_deg: f64, altitude_deg: f64, tint: bool) -> Image {
    let mut img = Image {
        w: g.cols,
        h: g.rows,
        rgba: vec![0; g.cols * g.rows * 4],
    };
    let (az, alt) = (azimuth_deg.to_radians(), altitude_deg.to_radians());
    let l = [az.sin() * alt.cos(), az.cos() * alt.cos(), alt.sin()];
    let at = |c: isize, r: isize| -> f32 {
        let (c, r) = (
            c.clamp(0, g.cols as isize - 1) as usize,
            r.clamp(0, g.rows as isize - 1) as usize,
        );
        g.data[r * g.cols + c]
    };
    for r in 0..g.rows {
        for c in 0..g.cols {
            let z = g.data[r * g.cols + c];
            if z.is_nan() {
                continue;
            }
            let (ci, ri) = (c as isize, r as isize);
            let (dzdx, dzdy) = (
                f64::from(at(ci + 1, ri) - at(ci - 1, ri)) / (2.0 * g.cell),
                f64::from(at(ci, ri - 1) - at(ci, ri + 1)) / (2.0 * g.cell),
            );
            let n = {
                let m = (dzdx * dzdx + dzdy * dzdy + 1.0).sqrt();
                [-dzdx / m, -dzdy / m, 1.0 / m]
            };
            let lam = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).clamp(0.0, 1.0);
            let base = if tint {
                ramp(
                    ((f64::from(z) - st.zmin) / (st.zmax - st.zmin).max(1e-9)) as f32,
                    st.color_by == ColorBy::ElevationEarth,
                )
            } else {
                [255, 255, 255]
            };
            let k = (0.25 + 0.75 * lam) as f32;
            let i = (r * g.cols + c) * 4;
            for (q, b) in base.iter().enumerate() {
                img.rgba[i + q] = (f32::from(*b) * k) as u8;
            }
            img.rgba[i + 3] = 255;
        }
    }
    img
}

/// Contour segments `(x1, y1, x2, y2, level)` of the grid every `interval`.
pub fn contours(g: &Grid, interval: f64) -> Vec<[f64; 5]> {
    let mut out = Vec::new();
    if !(interval > 0.0) || g.cols < 2 || g.rows < 2 {
        return out;
    }
    // A tiny interval over a tall surface would produce millions of segments: coarsen it.
    let (zlo, zhi) = g
        .data
        .iter()
        .filter(|v| !v.is_nan())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    if !zlo.is_finite() {
        return out;
    }
    let interval = interval.max(f64::from(zhi - zlo) / 1000.0);
    if interval <= 0.0 {
        return out;
    }
    for r in 0..g.rows - 1 {
        for c in 0..g.cols - 1 {
            let q = [
                g.data[r * g.cols + c],
                g.data[r * g.cols + c + 1],
                g.data[(r + 1) * g.cols + c + 1],
                g.data[(r + 1) * g.cols + c],
            ];
            if q.iter().any(|v| v.is_nan()) {
                continue;
            }
            let (lo, hi) = (
                q.iter().copied().fold(f32::INFINITY, f32::min),
                q.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            );
            let first = (f64::from(lo) / interval).ceil() as i64;
            let last = (f64::from(hi) / interval).floor() as i64;
            for k in first..=last {
                let level = k as f64 * interval;
                let (x0, y0) = g.center(c, r);
                // corners: 0 (x0,y0) 1 (x0+cell,y0) 2 (x0+cell,y0-cell) 3 (x0,y0-cell)
                let pos = [
                    (x0, y0),
                    (x0 + g.cell, y0),
                    (x0 + g.cell, y0 - g.cell),
                    (x0, y0 - g.cell),
                ];
                let mut pts = Vec::new();
                for e in 0..4 {
                    let (a, b) = (e, (e + 1) % 4);
                    let (va, vb) = (f64::from(q[a]), f64::from(q[b]));
                    if (va < level) != (vb < level) {
                        let t = (level - va) / (vb - va);
                        pts.push((
                            pos[a].0 + (pos[b].0 - pos[a].0) * t,
                            pos[a].1 + (pos[b].1 - pos[a].1) * t,
                        ));
                    }
                }
                if pts.len() >= 2 {
                    out.push([pts[0].0, pts[0].1, pts[1].0, pts[1].1, level]);
                    if pts.len() == 4 {
                        out.push([pts[2].0, pts[2].1, pts[3].0, pts[3].1, level]);
                    }
                }
            }
        }
    }
    out
}

/// Draws a grid (a DTM, CHM or any raster) as an image with the elevation ramp.
pub fn raster_image(g: &Grid, st: &Style) -> Image {
    let mut img = Image {
        w: g.cols,
        h: g.rows,
        rgba: vec![0; g.cols * g.rows * 4],
    };
    for (i, v) in g.data.iter().enumerate() {
        if !v.is_nan() {
            let t = ((f64::from(*v) - st.zmin) / (st.zmax - st.zmin).max(1e-9)) as f32;
            let c = if st.color_by == ColorBy::Intensity {
                let g = (t.clamp(0.0, 1.0) * 255.0) as u8;
                [g, g, g]
            } else {
                ramp(t, st.color_by == ColorBy::ElevationEarth)
            };
            img.rgba[i * 4..i * 4 + 3].copy_from_slice(&c);
            img.rgba[i * 4 + 3] = 255;
        }
    }
    img
}

/// What a vector layer holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GeomKind {
    #[default]
    Point,
    Line,
    Polygon,
}

/// One feature of a vector layer: its parts (lines, rings of polygons, or a single point).
#[derive(Clone, Debug, Default)]
pub struct Feature {
    pub parts: Vec<Vec<[f64; 2]>>,
}

/// A shapefile read for display: geometry plus the attribute table.
#[derive(Clone, Debug, Default)]
pub struct VectorData {
    pub features: Vec<Feature>,
    pub polygons: bool,
    pub points: bool,
    pub fields: Vec<String>,
    /// Per feature, per field: the value as text and as a number (NaN when it is not one).
    pub text: Vec<Vec<String>>,
    pub num: Vec<Vec<f64>>,
    /// min x, min y, max x, max y.
    pub bounds: [f64; 4],
}

/// Reads a shapefile with its attributes.
pub fn read_vector(path: &Path) -> Result<VectorData> {
    use shapefile::Shape;
    use shapefile::dbase::FieldValue;
    let mut rd =
        shapefile::ShapeReader::from_path(path).map_err(|e| Error::Format(e.to_string()))?;
    let mut v = VectorData {
        bounds: [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        ..VectorData::default()
    };
    for shape in rd.iter_shapes() {
        let shape = shape.map_err(|e| Error::Format(e.to_string()))?;
        let parts: Vec<Vec<[f64; 2]>> = match shape {
            Shape::Point(p) => vec![vec![[p.x, p.y]]],
            Shape::PointZ(p) => vec![vec![[p.x, p.y]]],
            Shape::PointM(p) => vec![vec![[p.x, p.y]]],
            Shape::Polyline(p) => p
                .parts()
                .iter()
                .map(|l| l.iter().map(|q| [q.x, q.y]).collect())
                .collect(),
            Shape::PolylineZ(p) => p
                .parts()
                .iter()
                .map(|l| l.iter().map(|q| [q.x, q.y]).collect())
                .collect(),
            Shape::PolylineM(p) => p
                .parts()
                .iter()
                .map(|l| l.iter().map(|q| [q.x, q.y]).collect())
                .collect(),
            Shape::Polygon(p) => p
                .rings()
                .iter()
                .map(|l| l.points().iter().map(|q| [q.x, q.y]).collect())
                .collect(),
            Shape::PolygonZ(p) => p
                .rings()
                .iter()
                .map(|l| l.points().iter().map(|q| [q.x, q.y]).collect())
                .collect(),
            Shape::PolygonM(p) => p
                .rings()
                .iter()
                .map(|l| l.points().iter().map(|q| [q.x, q.y]).collect())
                .collect(),
            Shape::NullShape => Vec::new(),
            _ => Vec::new(),
        };
        v.points |= parts.len() == 1 && parts[0].len() == 1;
        v.polygons |= parts
            .first()
            .is_some_and(|r| r.len() >= 3 && r.first() == r.last());
        for q in parts.iter().flatten() {
            v.bounds = [
                v.bounds[0].min(q[0]),
                v.bounds[1].min(q[1]),
                v.bounds[2].max(q[0]),
                v.bounds[3].max(q[1]),
            ];
        }
        v.features.push(Feature { parts });
    }
    // the attribute table, when there is one
    if let Ok(mut dbf) = shapefile::dbase::Reader::from_path(path.with_extension("dbf")) {
        v.fields = dbf.fields().iter().map(|f| f.name().to_string()).collect();
        if let Ok(records) = dbf.read() {
            for rec in records {
                let mut text = Vec::new();
                let mut num = Vec::new();
                for f in &v.fields {
                    let (t, n) = match rec.get(f) {
                        Some(FieldValue::Numeric(Some(x))) => (format!("{x}"), *x),
                        Some(FieldValue::Float(Some(x))) => (format!("{x}"), f64::from(*x)),
                        Some(FieldValue::Double(x)) => (format!("{x}"), *x),
                        Some(FieldValue::Integer(x)) => (format!("{x}"), f64::from(*x)),
                        Some(FieldValue::Character(Some(t))) => {
                            (t.trim().to_string(), t.trim().parse().unwrap_or(f64::NAN))
                        }
                        Some(FieldValue::Date(Some(d))) => (format!("{d:?}"), f64::NAN),
                        _ => (String::new(), f64::NAN),
                    };
                    text.push(t);
                    num.push(n);
                }
                v.text.push(text);
                v.num.push(num);
            }
        }
    }
    Ok(v)
}

/// Vertices of the shapes of a shapefile (lines, polygon rings and points), for drawing over
/// the cloud.
pub fn read_overlay(path: &Path) -> Result<Vec<Vec<[f64; 2]>>> {
    use shapefile::Shape;
    let mut rd =
        shapefile::ShapeReader::from_path(path).map_err(|e| Error::Format(e.to_string()))?;
    let mut out = Vec::new();
    for shape in rd.iter_shapes() {
        let shape = shape.map_err(|e| Error::Format(e.to_string()))?;
        match shape {
            Shape::Point(p) => out.push(vec![[p.x, p.y]]),
            Shape::PointZ(p) => out.push(vec![[p.x, p.y]]),
            Shape::PointM(p) => out.push(vec![[p.x, p.y]]),
            Shape::Polyline(p) => out.extend(
                p.parts()
                    .iter()
                    .map(|l| l.iter().map(|q| [q.x, q.y]).collect()),
            ),
            Shape::PolylineZ(p) => out.extend(
                p.parts()
                    .iter()
                    .map(|l| l.iter().map(|q| [q.x, q.y]).collect()),
            ),
            Shape::PolylineM(p) => out.extend(
                p.parts()
                    .iter()
                    .map(|l| l.iter().map(|q| [q.x, q.y]).collect()),
            ),
            Shape::Polygon(p) => out.extend(
                p.rings()
                    .iter()
                    .map(|l| l.points().iter().map(|q| [q.x, q.y]).collect()),
            ),
            Shape::PolygonZ(p) => out.extend(
                p.rings()
                    .iter()
                    .map(|l| l.points().iter().map(|q| [q.x, q.y]).collect()),
            ),
            Shape::PolygonM(p) => out.extend(
                p.rings()
                    .iter()
                    .map(|l| l.points().iter().map(|q| [q.x, q.y]).collect()),
            ),
            _ => {}
        }
    }
    Ok(out)
}

/// Paints a shaded surface (from [`shade`]) into a plan view window, under whatever is drawn
/// next; pixels outside the surface keep `img`.
pub fn paint_surface(img: &mut Image, g: &Grid, shaded: &Image, v: &Plan) {
    let (w, h) = (img.w, img.h);
    for py in 0..h {
        for px in 0..w {
            let (x, y) = v.to_map(px as f64 + 0.5, py as f64 + 0.5, w, h);
            let (c, r) = (((x - g.x0) / g.cell).floor(), ((y - g.y0) / g.cell).floor());
            if c < 0.0 || r < 0.0 || c >= g.cols as f64 || r >= g.rows as f64 {
                continue;
            }
            let k = (g.rows - 1 - r as usize) * g.cols + c as usize;
            if shaded.rgba[k * 4 + 3] != 0 {
                let o = (py * w + px) * 4;
                img.rgba[o..o + 3].copy_from_slice(&shaded.rgba[k * 4..k * 4 + 3]);
            }
        }
    }
}

/// Draws contour segments from [`contours`] over a plan view; every `index`-th level thicker.
pub fn paint_contours(img: &mut Image, segs: &[[f64; 5]], v: &Plan, color: [u8; 3]) {
    let (w, h) = (img.w, img.h);
    for s in segs {
        let a = v.to_screen(s[0], s[1], w, h);
        let b = v.to_screen(s[2], s[3], w, h);
        draw_line(img, a, b, color);
    }
}

/// Bresenham line into an image.
pub fn draw_line(img: &mut Image, a: (f64, f64), b: (f64, f64), c: [u8; 3]) {
    let (mut x0, mut y0, x1, y1) = (
        a.0.round() as i64,
        a.1.round() as i64,
        b.0.round() as i64,
        b.1.round() as i64,
    );
    if (x0 - x1).abs() + (y0 - y1).abs() > 20_000 {
        return;
    }
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        img.set(x0, y0, c);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> PointStore {
        let mut s = PointStore {
            stride: 1,
            total: 4,
            origin: [100.0, 200.0, 0.0],
            ..PointStore::default()
        };
        for (x, y, z, c) in [
            (0.0f32, 0.0f32, 1.0f32, 2u8),
            (10.0, 0.0, 5.0, 5),
            (10.0, 10.0, 9.0, 5),
            (0.0, 10.0, 3.0, 2),
        ] {
            s.x.push(x);
            s.y.push(y);
            s.z.push(z);
            s.class.push(c);
            s.intensity.push(100);
            s.ret.push(0x11);
            s.src.push(0);
            s.file.push(0);
        }
        s.bounds = [100.0, 200.0, 1.0, 110.0, 210.0, 9.0];
        s
    }

    #[test]
    fn plan_puts_points_where_the_map_says() {
        let s = store();
        let v = Plan {
            cx: 105.0,
            cy: 205.0,
            scale: 8.0,
        };
        let img = render_plan(&s, &v, 100, 100, &Style::fit(&s));
        // (100, 200) is the bottom-left corner of the 100x100 view centred on (105, 205)
        let at = |x: usize, y: usize| img.rgba[(y * 100 + x) * 4..(y * 100 + x) * 4 + 3].to_vec();
        assert_ne!(at(10, 90), vec![18, 21, 26]);
        assert_eq!(at(50, 50), vec![18, 21, 26]);
        let mut hidden = Style::fit(&s);
        hidden.classes = ClassSet::from_bits(1 << 5);
        let img = render_plan(&s, &v, 100, 100, &hidden);
        assert_eq!(
            &img.rgba[(90 * 100 + 10) * 4..(90 * 100 + 10) * 4 + 3],
            &[18, 21, 26]
        );
    }

    #[test]
    fn profile_keeps_only_the_strip() {
        let s = store();
        let bx = ProfileBox {
            a: [100.0, 200.0],
            b: [110.0, 200.0],
            width: 2.0,
        };
        let (_, lo, hi) = render_profile(&s, &bx, 80, 40, &Style::fit(&s));
        assert!(lo < 1.0 && hi > 5.0 && hi < 9.0, "{lo} {hi}");
    }

    #[test]
    fn contour_crosses_a_ramp_once_per_row() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 10, 4)?;
        for r in 0..4 {
            for c in 0..10 {
                g.data[r * 10 + c] = c as f32;
            }
        }
        assert_eq!(contours(&g, 5.0 / 2.0).len(), 9);
        Ok(())
    }

    #[test]
    fn pan_moves_the_scene_with_the_cursor() {
        // looking down, tilted, turned and with exaggeration: a point at the target's depth
        // must move exactly as far as the cursor
        for (yaw, pitch, ex) in [(0.0, 0.6, 1.0), (2.1, 0.2, 3.0), (-1.0, -0.8, 1.5)] {
            let mut o = Orbit {
                target: [10.0, 20.0, 5.0],
                yaw,
                pitch,
                dist: 80.0,
                exaggeration: ex,
            };
            let q = [10.0, 20.0, 5.0];
            let (x0, y0) = o.project(q, 800, 600).unwrap_or_default();
            o.pan_px(40.0, -25.0, 800, 600);
            let (x1, y1) = o.project(q, 800, 600).unwrap_or_default();
            assert!((x1 - x0 - 40.0).abs() < 1e-6, "{yaw} {x1} {x0}");
            assert!((y1 - y0 + 25.0).abs() < 1e-6, "{yaw} {y1} {y0}");
        }
    }
}
