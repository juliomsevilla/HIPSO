//! Layers on the map: vector layers (their geometry, drawing, symbology and labels), the layers
//! pane, identifying what lies under a click, and the link between the plan and the 3D view.
//! Editing tools, the attribute table and the contextual ribbon tabs are in `vecedit.rs` and
//! `layer_tabs.rs`.

use super::*;
use crate::symbology::{Labels, SymKind, Symbology};

/// The triangles of one polygon (outer ring and its holes), for filling it.
pub struct Tri {
    pub verts: Vec<[f64; 2]>,
    pub idx: Vec<usize>,
}

/// What is worked out from the geometry once and reused every frame.
#[derive(Default)]
pub struct Geom {
    pub bbox: Vec<[f64; 4]>,
    /// Where the label of each feature goes.
    pub anchor: Vec<[f64; 2]>,
    pub tris: Vec<Vec<Tri>>,
}

/// A vector layer: its features and attributes, how they are drawn, and the editing state.
pub struct VecLayer {
    pub data: view::VectorData,
    pub kind: view::GeomKind,
    /// Outline colour of lines and areas, colour of points.
    pub color: [u8; 3],
    pub fill: [u8; 3],
    pub fill_alpha: f32,
    pub width: f32,
    pub point_size: f32,
    pub sym: Symbology,
    pub labels: Labels,
    pub selected: HashSet<usize>,
    /// Changed since it was read or saved.
    pub dirty: bool,
    pub undo: Vec<view::VectorData>,
    pub redo: Vec<view::VectorData>,
    pub geom: Geom,
}

fn ring_area_centroid(r: &[[f64; 2]]) -> ([f64; 2], f64) {
    let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        let c = p[0] * q[1] - q[0] * p[1];
        a += c;
        cx += (p[0] + q[0]) * c;
        cy += (p[1] + q[1]) * c;
    }
    if a.abs() < 1e-12 {
        let n = r.len().max(1) as f64;
        return (
            [
                r.iter().map(|p| p[0]).sum::<f64>() / n,
                r.iter().map(|p| p[1]).sum::<f64>() / n,
            ],
            0.0,
        );
    }
    ([cx / (3.0 * a), cy / (3.0 * a)], a.abs() / 2.0)
}

/// Triangles for the polygon parts of a feature: rings inside an odd number of others are holes.
fn triangulate(parts: &[Vec<[f64; 2]>]) -> Vec<Tri> {
    let rings: Vec<Vec<[f64; 2]>> = parts
        .iter()
        .map(|p| {
            let mut r = p.clone();
            if r.len() > 1 && r.first() == r.last() {
                r.pop();
            }
            r
        })
        .filter(|r| r.len() >= 3)
        .collect();
    let depth: Vec<usize> = if rings.len() > 40 {
        vec![0; rings.len()]
    } else {
        (0..rings.len())
            .map(|i| {
                (0..rings.len())
                    .filter(|&j| j != i && in_ring(&rings[j], rings[i][0][0], rings[i][0][1]))
                    .count()
            })
            .collect()
    };
    let mut out = Vec::new();
    for o in (0..rings.len()).filter(|&i| depth[i].is_multiple_of(2)) {
        let mut flat: Vec<f64> = rings[o].iter().flat_map(|p| *p).collect();
        let mut verts = rings[o].clone();
        let mut holes = Vec::new();
        for h in (0..rings.len()).filter(|&j| depth[j] % 2 == 1) {
            // the innermost outer ring that contains the hole owns it
            let owner = (0..rings.len())
                .filter(|&i| {
                    depth[i].is_multiple_of(2) && in_ring(&rings[i], rings[h][0][0], rings[h][0][1])
                })
                .max_by_key(|&i| depth[i]);
            if owner == Some(o) {
                holes.push(verts.len());
                flat.extend(rings[h].iter().flat_map(|p| *p));
                verts.extend(rings[h].iter().copied());
            }
        }
        if let Ok(idx) = earcutr::earcut(&flat, &holes, 2) {
            out.push(Tri { verts, idx });
        }
    }
    out
}

impl VecLayer {
    pub fn new(data: view::VectorData, kind: view::GeomKind) -> Self {
        let color = match kind {
            view::GeomKind::Point => [255, 200, 0],
            view::GeomKind::Line => [255, 160, 70],
            view::GeomKind::Polygon => [60, 200, 255],
        };
        let mut l = Self {
            data,
            kind,
            color,
            fill: color,
            fill_alpha: 0.25,
            width: 1.5,
            point_size: 4.0,
            sym: Symbology::default(),
            labels: Labels::default(),
            selected: HashSet::new(),
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            geom: Geom::default(),
        };
        l.rebuild();
        l
    }

    /// The extent, label place and fill triangles of one feature.
    fn geom_of(&self, f: &view::Feature) -> ([f64; 4], [f64; 2], Vec<Tri>) {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for q in f.parts.iter().flatten() {
            b = [
                b[0].min(q[0]),
                b[1].min(q[1]),
                b[2].max(q[0]),
                b[3].max(q[1]),
            ];
        }
        let anchor = match (self.kind, f.parts.first()) {
            (view::GeomKind::Polygon, Some(_)) => {
                let best = f
                    .parts
                    .iter()
                    .filter(|p| p.len() >= 3)
                    .map(|p| ring_area_centroid(p))
                    .max_by(|a, b| a.1.total_cmp(&b.1));
                best.map_or([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0], |r| r.0)
            }
            (view::GeomKind::Line, Some(p)) if p.len() >= 2 => {
                let total: f64 = p
                    .windows(2)
                    .map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]))
                    .sum();
                let (mut run, mut at) = (0.0, p[0]);
                for s in p.windows(2) {
                    let l = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
                    if run + l >= total / 2.0 && l > 0.0 {
                        let t = (total / 2.0 - run) / l;
                        at = [
                            s[0][0] + (s[1][0] - s[0][0]) * t,
                            s[0][1] + (s[1][1] - s[0][1]) * t,
                        ];
                        break;
                    }
                    run += l;
                }
                at
            }
            (_, Some(p)) if !p.is_empty() => p[0],
            _ => [0.0, 0.0],
        };
        let tris = if self.kind == view::GeomKind::Polygon {
            triangulate(&f.parts)
        } else {
            Vec::new()
        };
        (b, anchor, tris)
    }

    /// Recomputes what depends on the geometry (extents, label places, fill triangles).
    pub fn rebuild(&mut self) {
        let mut g = Geom::default();
        for f in &self.data.features {
            let (b, a, t) = self.geom_of(f);
            g.bbox.push(b);
            g.anchor.push(a);
            g.tris.push(t);
        }
        self.geom = g;
        self.refresh_bounds();
        self.selected.retain(|i| *i < self.data.features.len());
    }

    /// The extent of the layer, from the extents of its features.
    pub fn refresh_bounds(&mut self) {
        self.data.bounds = self.geom.bbox.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            },
        );
    }

    /// Feature `i` changed its geometry.
    pub fn update_feature(&mut self, i: usize) {
        let Some(f) = self.data.features.get(i) else {
            return;
        };
        let (b, a, t) = self.geom_of(f);
        self.geom.bbox[i] = b;
        self.geom.anchor[i] = a;
        self.geom.tris[i] = t;
        self.refresh_bounds();
        self.dirty = true;
        self.reclassify_if_data();
    }

    /// Adds a feature whose first field gets a running number; returns its index.
    pub fn push_feature(&mut self, parts: Vec<Vec<[f64; 2]>>) -> usize {
        let n = self.data.features.len() + 1;
        self.data.features.push(view::Feature { parts });
        let row: Vec<String> = (0..self.data.fields.len())
            .map(|c| if c == 0 { n.to_string() } else { String::new() })
            .collect();
        self.data
            .num
            .push(row.iter().map(|t| t.parse().unwrap_or(f64::NAN)).collect());
        self.data.text.push(row);
        let (b, a, t) = self.geom_of(&self.data.features[n - 1]);
        self.geom.bbox.push(b);
        self.geom.anchor.push(a);
        self.geom.tris.push(t);
        self.refresh_bounds();
        self.dirty = true;
        self.reclassify_if_data();
        n - 1
    }

    /// Removes the features at the given indices.
    pub fn remove_features(&mut self, which: &HashSet<usize>) {
        let mut k = 0;
        self.data.features.retain(|_| {
            k += 1;
            !which.contains(&(k - 1))
        });
        let mut k = 0;
        self.data.text.retain(|_| {
            k += 1;
            !which.contains(&(k - 1))
        });
        let mut k = 0;
        self.data.num.retain(|_| {
            k += 1;
            !which.contains(&(k - 1))
        });
        self.selected.clear();
        self.dirty = true;
        self.rebuild();
        self.reclassify_if_data();
    }

    /// Classes that follow the data are worked out again after it changes.
    fn reclassify_if_data(&mut self) {
        if self.sym.kind != SymKind::Single {
            self.reclassify();
        }
    }

    /// Re-classifies after the symbology or the data changed.
    pub fn reclassify(&mut self) {
        self.sym.classify(&self.data);
    }

    /// Remembers the data before a change, so that it can be undone.
    pub fn push_undo(&mut self) {
        let keep = if self.data.features.len() > 50_000 {
            3
        } else {
            40
        };
        self.undo.push(self.data.clone());
        if self.undo.len() > keep {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.data, prev));
        self.dirty = true;
        self.rebuild();
        self.reclassify();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.data, next));
        self.dirty = true;
        self.rebuild();
        self.reclassify();
        true
    }

    /// The first feature at a map point (within `tol` for points and lines).
    pub fn feature_at(&self, x: f64, y: f64, tol: f64) -> Option<usize> {
        let near =
            |b: &[f64; 4]| x >= b[0] - tol && x <= b[2] + tol && y >= b[1] - tol && y <= b[3] + tol;
        // the last drawn is the one on top
        (0..self.data.features.len()).rev().find(|&i| {
            near(&self.geom.bbox[i])
                && self.data.features[i].parts.iter().any(|p| {
                    if p.len() == 1 {
                        (p[0][0] - x).hypot(p[0][1] - y) <= tol
                    } else if self.kind == view::GeomKind::Polygon {
                        p.len() >= 4 && in_ring(p, x, y)
                            || p.windows(2)
                                .any(|s| dist_to_segment([x, y], s[0], s[1]) <= tol)
                    } else {
                        p.windows(2)
                            .any(|s| dist_to_segment([x, y], s[0], s[1]) <= tol)
                    }
                })
        })
    }

    pub fn selection_bounds(&self) -> Option<[f64; 4]> {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for &i in &self.selected {
            if let Some(f) = self.geom.bbox.get(i) {
                b = [
                    b[0].min(f[0]),
                    b[1].min(f[1]),
                    b[2].max(f[2]),
                    b[3].max(f[3]),
                ];
            }
        }
        b.iter().all(|v| v.is_finite()).then_some(b)
    }
}

pub fn in_ring(ring: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn dist_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

/// The dialog that makes a new vector layer.
#[derive(Default)]
pub struct NewLayerDlg {
    pub name: String,
    pub kind: view::GeomKind,
    pub fields: Vec<String>,
    pub new_field: String,
}

fn alpha(c: [u8; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Map to screen for one frame.
struct Xf {
    plan: Plan,
    w: usize,
    h: usize,
    origin: Vec2,
}

impl Xf {
    fn s(&self, p: &[f64; 2]) -> Pos2 {
        let s = self.plan.to_screen(p[0], p[1], self.w, self.h);
        Pos2::new(s.0 as f32, s.1 as f32) + self.origin
    }

    /// The polyline on screen, leaving out vertices less than a pixel from the previous one.
    fn path(&self, part: &[[f64; 2]]) -> Vec<Pos2> {
        let mut out = Vec::with_capacity(part.len().min(1024));
        let mut last = Pos2::new(f32::NAN, f32::NAN);
        for (i, p) in part.iter().enumerate() {
            let s = self.s(p);
            if i == 0 || i + 1 == part.len() || (s - last).length_sq() >= 0.7 {
                out.push(s);
                last = s;
            }
        }
        out
    }
}

fn flush(mesh: &mut egui::Mesh, clip: &egui::Painter) {
    if !mesh.is_empty() {
        clip.add(egui::Shape::mesh(std::mem::take(mesh)));
    }
}

impl App {
    pub fn add_vector(&mut self, p: &Path) {
        match view::read_vector(p) {
            Ok(data) => {
                self.refs.retain(|r| r.path != p);
                let n = data.features.len();
                let kind = if data.points {
                    view::GeomKind::Point
                } else if data.polygons {
                    view::GeomKind::Polygon
                } else {
                    view::GeomKind::Line
                };
                self.refs.push(RefLayer {
                    name: p
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned()),
                    path: p.to_path_buf(),
                    kind: RefKind::Vector(Box::new(VecLayer::new(data, kind))),
                    visible: true,
                    opacity: 1.0,
                });
                let k = self.refs.len() - 1;
                self.sel_layer = Some(k);
                self.active = Some(k);
                self.log(format!("Capa cargada: {} ({n} elementos)", p.display()));
                if self.store.is_none() {
                    self.plan_fit = true;
                }
            }
            Err(e) => self.log(format!("No se pudo abrir {}: {e}", p.display())),
        }
    }

    /// The extent of a layer: min x, min y, max x, max y.
    pub fn layer_extent(&self, k: usize) -> Option<[f64; 4]> {
        let b = match &self.refs.get(k)?.kind {
            RefKind::Raster { grid, .. } => [
                grid.x0,
                grid.y0,
                grid.x0 + grid.cols as f64 * grid.cell,
                grid.y0 + grid.rows as f64 * grid.cell,
            ],
            RefKind::Vector(v) => v.data.bounds,
            RefKind::Image(il) => il.pyr.extent(),
        };
        b.iter().all(|v| v.is_finite()).then_some(b)
    }

    /// The extent of the visible layers.
    pub fn refs_bounds(&self) -> Option<[f64; 4]> {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for k in (0..self.refs.len()).filter(|&k| self.refs[k].visible) {
            if let Some(e) = self.layer_extent(k) {
                b = [
                    b[0].min(e[0]),
                    b[1].min(e[1]),
                    b[2].max(e[2]),
                    b[3].max(e[3]),
                ];
            }
        }
        b.iter().all(|v| v.is_finite()).then_some(b)
    }

    /// Zooms the plan to an extent (a point gets a window of 40 m).
    pub fn zoom_to_bounds(&mut self, b: [f64; 4]) {
        let pad = if (b[2] - b[0]).max(b[3] - b[1]) < 1e-6 {
            20.0
        } else {
            0.0
        };
        let (w, h) = self.plan_px;
        let p = Plan::fit(
            [b[0] - pad, b[1] - pad, 0.0, b[2] + pad, b[3] + pad, 0.0],
            w.max(8),
            h.max(8),
        );
        self.zoom_to(p);
    }

    pub fn zoom_to_layer(&mut self, k: usize) {
        if let Some(b) = self.layer_extent(k) {
            self.zoom_to_bounds(b);
        }
    }

    /// Draws the vector layers in the order of the list.
    pub fn paint_vectors(&self, painter: &egui::Painter, rect: Rect, w: usize, h: usize) {
        let clip = painter.with_clip_rect(rect);
        let xf = Xf {
            plan: self.plan,
            w,
            h,
            origin: rect.min.to_vec2(),
        };
        let (lo, hi) = (
            self.plan.to_map(0.0, h as f64, w, h),
            self.plan.to_map(w as f64, 0.0, w, h),
        );
        let view = [lo.0, lo.1, hi.0, hi.1];
        for r in self.refs.iter().filter(|r| r.visible) {
            let RefKind::Vector(v) = &r.kind else {
                continue;
            };
            let op = r.opacity.clamp(0.0, 1.0);
            let visible: Vec<usize> = (0..v.data.features.len())
                .filter(|&i| {
                    let b = &v.geom.bbox[i];
                    b[2] >= view[0] && b[0] <= view[2] && b[3] >= view[1] && b[1] <= view[3]
                })
                .collect();
            // areas: fills first, as one mesh
            if v.kind == view::GeomKind::Polygon {
                let mut mesh = egui::Mesh::default();
                for &i in &visible {
                    let c = v.sym.color_of(&v.data, i, v.fill);
                    let col = alpha(c, v.fill_alpha * op);
                    for t in &v.geom.tris[i] {
                        let base = mesh.vertices.len() as u32;
                        for p in &t.verts {
                            mesh.colored_vertex(xf.s(p), col);
                        }
                        for tri in t.idx.as_chunks::<3>().0 {
                            mesh.add_triangle(
                                base + tri[0] as u32,
                                base + tri[1] as u32,
                                base + tri[2] as u32,
                            );
                        }
                        if mesh.vertices.len() > 50_000 {
                            flush(&mut mesh, &clip);
                        }
                    }
                }
                flush(&mut mesh, &clip);
            }
            for &i in &visible {
                let f = &v.data.features[i];
                let c = v.sym.color_of(&v.data, i, v.color);
                let col = alpha(c, op);
                let outline = if v.kind == view::GeomKind::Polygon && v.sym.kind != SymKind::Single
                {
                    alpha([c[0] / 2, c[1] / 2, c[2] / 2], op.max(0.6))
                } else {
                    col
                };
                for part in &f.parts {
                    if v.kind == view::GeomKind::Point || part.len() == 1 {
                        let size = v.sym.size_of(&v.data, i, v.point_size);
                        clip.circle(
                            xf.s(&part[0]),
                            size,
                            col,
                            Stroke::new(1.0, Color32::from_black_alpha(150)),
                        );
                        continue;
                    }
                    let mut pts = xf.path(part);
                    if v.kind == view::GeomKind::Polygon
                        && part.first() != part.last()
                        && let Some(f0) = pts.first().copied()
                    {
                        pts.push(f0);
                    }
                    clip.add(egui::Shape::line(pts, Stroke::new(v.width, outline)));
                }
            }
            self.paint_labels(&clip, v, &visible, &xf, rect);
        }
    }

    /// Labels without overlaps: a label that would cover one already placed is left out.
    fn paint_labels(
        &self,
        clip: &egui::Painter,
        v: &VecLayer,
        visible: &[usize],
        xf: &Xf,
        rect: Rect,
    ) {
        let Some(c) = v.labels.field else {
            return;
        };
        let font = FontId::proportional(v.labels.size);
        let mut placed: Vec<Rect> = Vec::new();
        for &i in visible.iter().take(4000) {
            let Some(text) = v
                .data
                .text
                .get(i)
                .and_then(|r| r.get(c))
                .filter(|t| !t.is_empty())
            else {
                continue;
            };
            let mut at = xf.s(&v.geom.anchor[i]);
            if !rect.contains(at) {
                continue;
            }
            let galley = clip.layout_no_wrap(text.clone(), font.clone(), color32(v.labels.color));
            let size = galley.size();
            if v.kind == view::GeomKind::Point {
                at.x += v.point_size + 3.0 + size.x / 2.0;
            }
            let r = Rect::from_center_size(at, size).expand(2.0);
            if placed.iter().any(|p| p.intersects(r)) {
                continue;
            }
            placed.push(r);
            if v.labels.halo {
                let dark =
                    clip.layout_no_wrap(text.clone(), font.clone(), Color32::from_black_alpha(220));
                for d in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                    clip.galley(
                        r.min + Vec2::new(2.0 + d.0, 2.0 + d.1),
                        dark.clone(),
                        Color32::BLACK,
                    );
                }
            }
            clip.galley(r.min + Vec2::new(2.0, 2.0), galley, Color32::WHITE);
        }
    }

    /// What lies at a map point: the attributes of the vector features and the value of the
    /// rasters, as lines of text.
    pub fn identify(&self, x: f64, y: f64, tol: f64) -> Vec<String> {
        let mut out = Vec::new();
        for r in self.refs.iter().filter(|r| r.visible) {
            match &r.kind {
                RefKind::Raster { grid, .. } => {
                    if let Some(v) = grid
                        .index(x, y)
                        .and_then(|i| grid.data.get(i))
                        .filter(|v| !v.is_nan())
                    {
                        out.push(format!("{}: {v:.3}", r.name));
                    }
                }
                RefKind::Image(il) => {
                    if let Some(vals) = il.values_at(x, y) {
                        out.push(format!("{}: {}", r.name, vals));
                    }
                }
                RefKind::Vector(v) => {
                    if let Some(k) = v.feature_at(x, y, tol) {
                        out.push(format!("{} · elemento {}", r.name, k + 1));
                        if let Some(row) = v.data.text.get(k) {
                            for (name, val) in v.data.fields.iter().zip(row).take(14) {
                                out.push(format!("   {name} = {val}"));
                            }
                            if v.data.fields.len() > 14 {
                                out.push(format!("   … {} campos más", v.data.fields.len() - 14));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn paint_ident(&self, painter: &egui::Painter, rect: Rect) {
        if self.ident.is_empty() {
            return;
        }
        let h = 8.0 + self.ident.len() as f32 * 15.0;
        let b = Rect::from_min_size(
            rect.left_bottom() + Vec2::new(10.0, -h - 34.0),
            Vec2::new(340.0, h),
        );
        painter.rect_filled(b, 4.0, Color32::from_black_alpha(200));
        for (k, l) in self.ident.iter().enumerate() {
            painter.text(
                b.min + Vec2::new(8.0, 5.0 + k as f32 * 15.0),
                Align2::LEFT_TOP,
                l,
                FontId::monospace(11.0),
                Color32::WHITE,
            );
        }
    }

    /// The areas of the visible polygon layers, for the tools that cut with polygons.
    pub fn polygon_aois(&self) -> Vec<Aoi> {
        let mut out = Vec::new();
        for r in self.refs.iter().filter(|r| r.visible) {
            let RefKind::Vector(v) = &r.kind else {
                continue;
            };
            if v.kind != view::GeomKind::Polygon {
                continue;
            }
            for (i, f) in v.data.features.iter().enumerate() {
                if let Some(ring) = f.parts.first() {
                    let mut points = ring.clone();
                    if points.len() > 1 && points.first() == points.last() {
                        points.pop();
                    }
                    out.push(Aoi {
                        name: format!("{} {}", r.name, i + 1),
                        color: v.color,
                        comment: String::new(),
                        points,
                    });
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------------ layers pane

    /// The layers pane, in the manner of a GIS: the last layer is on top, each row has its
    /// visibility box, its symbol and its name (cut to fit); the context menu holds the actions.
    pub fn layers_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = self.tokens();
        let mut act: Vec<(usize, LayerCmd)> = Vec::new();
        for k in (0..self.refs.len()).rev() {
            let sel = self.sel_layer == Some(k);
            let (icon, tint, legend) = {
                let r = &self.refs[k];
                match &r.kind {
                    RefKind::Vector(v) => (
                        match v.kind {
                            view::GeomKind::Point => "@lpoint",
                            view::GeomKind::Line => "@lline",
                            view::GeomKind::Polygon => "@lpoly",
                        },
                        color32(if v.kind == view::GeomKind::Polygon {
                            v.fill
                        } else {
                            v.color
                        }),
                        if v.sym.kind == SymKind::Single {
                            Vec::new()
                        } else {
                            v.sym.legend(v.color)
                        },
                    ),
                    RefKind::Raster { .. } => ("@lraster", t.text, Vec::new()),
                    RefKind::Image(_) => ("@lraster", t.text, Vec::new()),
                }
            };
            let editing = self.active == Some(k) && self.edit_layer_on();
            let r = &mut self.refs[k];
            let dirty = matches!(&r.kind, RefKind::Vector(v) if v.dirty);
            let name = format!("{}{}", r.name, if dirty { " *" } else { "" });
            let tip = if r.path.as_os_str().is_empty() {
                r.name.clone()
            } else {
                r.path.display().to_string()
            };
            let resp = ui
                .horizontal(|ui| {
                    ui.checkbox(&mut r.visible, "");
                    let (ir, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    ribbon::draw_icon(ui.painter(), ir.center(), 15.0, icon, tint);
                    let w = (ui.available_width() - if editing { 30.0 } else { 0.0 }).max(30.0);
                    let (rr, b) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::click());
                    if sel {
                        ui.painter().rect_filled(rr, 3.0, t.selection);
                    } else if b.hovered() {
                        ui.painter().rect_filled(rr, 3.0, t.hover);
                    }
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        name,
                        FontId::proportional(13.0),
                        t.text,
                    );
                    job.wrap = egui::text::TextWrapping::truncate_at_width((w - 8.0).max(10.0));
                    let galley = ui.painter().layout_job(job);
                    ui.painter().galley(
                        Pos2::new(rr.left() + 4.0, rr.center().y - galley.size().y / 2.0),
                        galley,
                        t.text,
                    );
                    if editing {
                        let (ir, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                        ribbon::draw_icon(ui.painter(), ir.center(), 14.0, "@pencil", t.accent);
                    }
                    b
                })
                .inner;
            let resp = resp.on_hover_text(tip);
            if resp.clicked() {
                act.push((k, LayerCmd::Select));
            }
            if resp.double_clicked() {
                act.push((k, LayerCmd::Zoom));
            }
            resp.context_menu(|ui| {
                let (is_vec, is_img) = (
                    matches!(r.kind, RefKind::Vector(_)),
                    matches!(r.kind, RefKind::Image(_)),
                );
                let mut item = |ui: &mut egui::Ui, text: &str, c: LayerCmd| {
                    if ui.button(text).clicked() {
                        act.push((k, c));
                        ui.close();
                    }
                };
                item(ui, "Zoom a la capa", LayerCmd::Zoom);
                if is_vec {
                    item(ui, "Tabla de atributos", LayerCmd::Table);
                    item(ui, "Editar esta capa", LayerCmd::Edit);
                    item(ui, "Guardar", LayerCmd::Save);
                    item(ui, "Guardar como…", LayerCmd::SaveAs);
                }
                if is_img {
                    item(ui, "Bandas e índices…", LayerCmd::Bands);
                }
                ui.separator();
                item(ui, "Subir", LayerCmd::Up);
                item(ui, "Bajar", LayerCmd::Down);
                item(ui, "Quitar", LayerCmd::Remove);
            });
            for (text, c) in legend.iter().take(14) {
                ui.horizontal(|ui| {
                    ui.add_space(46.0);
                    let (sr, _) = ui.allocate_exact_size(Vec2::splat(11.0), Sense::hover());
                    ui.painter().rect_filled(sr, 2.0, color32(*c));
                    ui.add(egui::Label::new(egui::RichText::new(text).size(11.0)).truncate());
                });
            }
            if legend.len() > 14 {
                ui.label(
                    egui::RichText::new(format!("      … {} más", legend.len() - 14))
                        .size(11.0)
                        .color(t.weak),
                );
            }
        }
        if let Some(k) = self.sel_layer.filter(|k| *k < self.refs.len()) {
            ui.separator();
            self.layer_quick(ui, ctx, k);
        }
        for (k, c) in act {
            self.layer_command(ctx, k, c);
        }
    }

    /// Opacity and colours of the selected layer, close at hand; the rest is in its ribbon tab.
    fn layer_quick(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, k: usize) {
        let r = &mut self.refs[k];
        ui.add(egui::Label::new(egui::RichText::new(&r.name).strong().size(12.0)).truncate());
        ui.horizontal(|ui| {
            ui.label("Opacidad");
            ui.add(egui::Slider::new(&mut r.opacity, 0.0..=1.0).show_value(false));
            ui.label(format!("{:.0} %", r.opacity * 100.0));
        });
        let open_tab = match &mut r.kind {
            RefKind::Vector(v) => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(if v.kind == view::GeomKind::Point {
                        "Color"
                    } else {
                        "Trazo"
                    });
                    egui::color_picker::color_edit_button_srgb(ui, &mut v.color);
                    if v.kind == view::GeomKind::Polygon {
                        ui.label("Relleno");
                        egui::color_picker::color_edit_button_srgb(ui, &mut v.fill);
                    }
                });
                // the classes of graduated and unique-value symbology, each colour editable
                if v.sym.kind != SymKind::Single {
                    egui::CollapsingHeader::new("Clases")
                        .default_open(true)
                        .show(ui, |ui| {
                            let labels = v.sym.legend(v.color);
                            for (c, (text, _)) in v.sym.colors.iter_mut().zip(labels).take(30) {
                                ui.horizontal(|ui| {
                                    egui::color_picker::color_edit_button_srgb(ui, c);
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(text).size(11.5))
                                            .truncate(),
                                    );
                                });
                            }
                        });
                }
                ui.button("Simbología y etiquetas…").clicked()
            }
            RefKind::Raster { .. } | RefKind::Image(_) => {
                ui.button("Simbología del ráster…").clicked()
            }
        };
        let _ = ctx;
        if open_tab {
            self.tab = 5;
        }
    }

    pub fn layer_command(&mut self, ctx: &egui::Context, k: usize, c: LayerCmd) {
        if k >= self.refs.len() {
            return;
        }
        match c {
            LayerCmd::Select => {
                self.sel_layer = Some(k);
                if matches!(self.refs[k].kind, RefKind::Vector(_)) {
                    self.active = Some(k);
                }
            }
            LayerCmd::Zoom => self.zoom_to_layer(k),
            LayerCmd::Table => {
                self.sel_layer = Some(k);
                self.active = Some(k);
                self.table_layer = Some(k);
            }
            LayerCmd::Edit => {
                self.sel_layer = Some(k);
                self.active = Some(k);
                self.tab = 6;
            }
            LayerCmd::Save => self.save_layer(k, false),
            LayerCmd::SaveAs => self.save_layer(k, true),
            LayerCmd::Bands => {
                self.rs.open = true;
                self.rs.layer = k;
            }
            LayerCmd::Up | LayerCmd::Down => {
                let to = if c == LayerCmd::Up {
                    k + 1
                } else {
                    k.saturating_sub(1)
                };
                if to < self.refs.len() && to != k {
                    self.refs.swap(k, to);
                    let fix = |o: &mut Option<usize>| {
                        if *o == Some(k) {
                            *o = Some(to);
                        } else if *o == Some(to) {
                            *o = Some(k);
                        }
                    };
                    fix(&mut self.sel_layer);
                    fix(&mut self.active);
                    fix(&mut self.table_layer);
                }
            }
            LayerCmd::Remove => {
                self.refs.remove(k);
                let fix = |o: &mut Option<usize>| match *o {
                    Some(i) if i == k => *o = None,
                    Some(i) if i > k => *o = Some(i - 1),
                    _ => {}
                };
                fix(&mut self.sel_layer);
                fix(&mut self.active);
                fix(&mut self.table_layer);
                self.dig_draft.clear();
            }
        }
        let _ = ctx;
    }

    /// Whether a vector editing tool is the current one.
    pub fn edit_layer_on(&self) -> bool {
        matches!(
            self.nav,
            Nav::Digitize | Nav::VecSelect | Nav::VecVertex | Nav::VecMove
        )
    }

    /// Keeps the plan and the 3D view on the same ground: whichever one was zoomed or panned
    /// this frame moves the other.
    pub fn sync_views(&mut self) {
        let has3d = self.layout == Layout::All;
        if !self.follow_plan || !has3d || self.store.is_none() {
            self.snap_plan = None;
            self.snap_orbit = None;
            return;
        }
        let (w, h) = self.plan_px;
        let diag_px = (w as f64).hypot(h as f64).max(1.0);
        let cur_plan = (self.plan.cx, self.plan.cy, self.plan.scale);
        let cur_orb = (self.orbit.target[0], self.orbit.target[1], self.orbit.dist);
        let plan_changed = self.snap_plan.is_none_or(|s| s != cur_plan);
        let orbit_changed = self.snap_orbit.is_some_and(|s| s != cur_orb);
        if orbit_changed && !plan_changed {
            self.plan.cx = self.orbit.target[0];
            self.plan.cy = self.orbit.target[1];
            self.plan.scale = diag_px / (self.orbit.dist / 1.2).max(1e-6);
        } else {
            self.orbit.target[0] = self.plan.cx;
            self.orbit.target[1] = self.plan.cy;
            self.orbit.dist = (1.2 * diag_px / self.plan.scale.max(1e-9)).max(1.0);
        }
        self.snap_plan = Some((self.plan.cx, self.plan.cy, self.plan.scale));
        self.snap_orbit = Some((self.orbit.target[0], self.orbit.target[1], self.orbit.dist));
        let (hw, hh) = (
            w as f64 / 2.0 / self.plan.scale,
            h as f64 / 2.0 / self.plan.scale,
        );
        self.style.window = Some([
            self.plan.cx - hw,
            self.plan.cy - hh,
            self.plan.cx + hw,
            self.plan.cy + hh,
        ]);
    }

    // ------------------------------------------------------------------------ new layer, save

    pub fn new_layer_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.new_layer.take() else {
            return;
        };
        if d.fields.is_empty() && d.name.is_empty() {
            d.name = format!("Capa {}", self.refs.len() + 1);
            d.fields = vec!["Nombre".to_string()];
        }
        let (mut open, mut create) = (true, false);
        egui::Window::new("Nueva capa vectorial")
            .open(&mut open)
            .collapsible(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Nombre");
                    ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(200.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Tipo");
                    ui.radio_value(&mut d.kind, view::GeomKind::Point, "Puntos");
                    ui.radio_value(&mut d.kind, view::GeomKind::Line, "Líneas");
                    ui.radio_value(&mut d.kind, view::GeomKind::Polygon, "Áreas");
                });
                ui.label("Campos de la tabla");
                let mut remove = None;
                for (k, f) in d.fields.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(f).desired_width(160.0));
                        if ui.small_button("✖").clicked() {
                            remove = Some(k);
                        }
                    });
                }
                if let Some(k) = remove {
                    d.fields.remove(k);
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut d.new_field)
                            .hint_text("nuevo campo")
                            .desired_width(160.0),
                    );
                    if ui.button("Añadir campo").clicked() && !d.new_field.trim().is_empty() {
                        let f = d.new_field.trim().to_string();
                        d.fields.push(f);
                        d.new_field.clear();
                    }
                });
                if ui.button("Crear").clicked() {
                    create = true;
                }
            });
        if create {
            let data = view::VectorData {
                fields: d.fields.clone(),
                polygons: d.kind == view::GeomKind::Polygon,
                points: d.kind == view::GeomKind::Point,
                ..view::VectorData::default()
            };
            let mut layer = VecLayer::new(data, d.kind);
            layer.dirty = true;
            self.refs.push(RefLayer {
                name: d.name.trim().to_string(),
                path: PathBuf::new(),
                kind: RefKind::Vector(Box::new(layer)),
                visible: true,
                opacity: 1.0,
            });
            let k = self.refs.len() - 1;
            self.active = Some(k);
            self.sel_layer = Some(k);
            // an empty layer has no extent to fit: the view stays where it is
            self.plan_fit = self.plan_fit && self.refs.len() > 1;
            self.nav = Nav::Digitize;
            self.tab = 6;
            self.log(format!(
                "Capa «{}» creada: digitaliza sobre la planta.",
                d.name
            ));
        } else if open {
            self.new_layer = Some(d);
        }
    }

    /// Saves a vector layer as a shapefile with the projection of the cloud: in place when it
    /// already has a file and `ask` is false.
    pub fn save_layer(&mut self, k: usize, ask: bool) {
        // the projection of the cloud, else that of an image on the map
        let prj = self.store.as_ref().and_then(|s| s.prj.clone()).or_else(|| {
            self.refs.iter().find_map(|r| match &r.kind {
                RefKind::Image(il) => il.pyr.epsg.and_then(rustlidar_core::vector::prj_for_epsg),
                _ => None,
            })
        });
        let dir = self.clouds.first().map_or_else(
            || PathBuf::from("."),
            |c| rustlidar_core::paths::results_dir(c),
        );
        let Some(r) = self.refs.get_mut(k) else {
            return;
        };
        let RefKind::Vector(v) = &mut r.kind else {
            return;
        };
        let path = if !ask
            && r.path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("shp"))
        {
            r.path.clone()
        } else {
            let stem = rustlidar_core::paths::stem(Path::new(&r.name));
            let Some(p) = rfd::FileDialog::new()
                .add_filter("Shapefile", &["shp"])
                .set_directory(if r.path.parent().is_some_and(|p| p.is_dir()) {
                    r.path.parent().map_or(dir.clone(), Path::to_path_buf)
                } else {
                    dir
                })
                .set_file_name(format!("{stem}.shp"))
                .save_file()
            else {
                return;
            };
            p
        };
        let res = rustlidar_core::vector::write_layer(&path, &v.data, v.kind, prj.as_deref());
        match res {
            Ok(()) => {
                r.path = path.clone();
                r.name = path
                    .file_name()
                    .map_or(r.name.clone(), |n| n.to_string_lossy().into_owned());
                v.dirty = false;
                let msg = if prj.is_some() {
                    format!("Capa guardada: {}", path.display())
                } else {
                    format!(
                        "Capa guardada sin .prj (la nube no declara su sistema de coordenadas): {}",
                        path.display()
                    )
                };
                self.log(msg);
            }
            Err(e) => self.log(format!("No se pudo guardar la capa: {e}")),
        }
    }
}

/// What the layers pane asks of a layer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LayerCmd {
    Select,
    Zoom,
    Table,
    Edit,
    Save,
    SaveAs,
    Bands,
    Up,
    Down,
    Remove,
}
