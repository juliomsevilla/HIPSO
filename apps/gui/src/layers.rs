//! Capas en el mapa: capas vectoriales (su geometría, dibujo, simbología y etiquetas), el panel de
//! capas, la identificación de lo que hay bajo un clic y el vínculo entre la planta y la vista 3D.
//! Las herramientas de edición, la tabla de atributos y las pestañas contextuales de la cinta están
//! en `vecedit.rs` y `layer_tabs.rs`.

use super::*;
use crate::symbology::{Labels, SymKind, Symbology};

/// Los triángulos de un polígono (anillo exterior y sus huecos), para rellenarlo.
pub struct Tri {
    pub verts: Vec<[f64; 2]>,
    pub idx: Vec<u32>,
}

/// Lo que se calcula una vez a partir de la geometría y se reutiliza en cada fotograma.
#[derive(Default)]
pub struct Geom {
    pub bbox: Vec<[f64; 4]>,
    /// Dónde va la etiqueta de cada elemento.
    pub anchor: Vec<[f64; 2]>,
    /// Triángulos de relleno de cada elemento, calculados la primera vez que se dibuja: una capa de
    /// millones de vértices tardaría si no minutos y gigabytes en abrirse.
    pub tris: Vec<std::sync::OnceLock<Vec<Tri>>>,
    /// Qué elementos caen en una ventana, calculado la primera vez que se pide.
    pub index: std::sync::OnceLock<hipso_core::rtree::RTree>,
    /// Vértices de todos los elementos.
    pub nverts: usize,
}

/// Una capa vectorial: sus elementos y atributos, cómo se dibujan y el estado de edición.
pub struct VecLayer {
    pub data: view::VectorData,
    pub kind: view::GeomKind,
    /// Color del contorno de líneas y polígonos, color de los puntos.
    pub color: [u8; 3],
    pub fill: [u8; 3],
    pub fill_alpha: f32,
    pub width: f32,
    pub point_size: f32,
    pub sym: Symbology,
    pub labels: Labels,
    pub selected: HashSet<usize>,
    /// Cambiada desde que se leyó o se guardó.
    pub dirty: bool,
    pub undo: Vec<view::VectorData>,
    pub redo: Vec<view::VectorData>,
    /// Los datos tal como estaban cuando se activó una herramienta de edición, y si `dirty` estaba
    /// fijado entonces: a lo que vuelve el descarte de los cambios de la sesión.
    pub edit_base: Option<(view::VectorData, bool)>,
    /// Se hizo un cambio desde que empezó la sesión de edición.
    pub edit_touched: bool,
    /// La tabla del GeoPackage de la que se leyó la capa: al guardar se escribe sobre esa tabla.
    pub table: Option<String>,
    /// El sistema en que estaba el archivo antes de llevar la capa al del trabajo, que es en el que
    /// se vuelve a escribir cuando se guarda sobre su propia tabla.
    pub file_epsg: Option<u16>,
    pub geom: Geom,
    /// Cambia con la geometría: aquello para lo que se hace una imagen de la capa.
    pub rev: u64,
    /// La imagen de una capa grande.
    pub raster: std::sync::Mutex<crate::vecraster::Cache>,
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

/// Triángulos de las partes poligonales de un elemento: los anillos dentro de un número impar de
/// otros son huecos.
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
            // el anillo exterior más interno que contiene el hueco es su dueño
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
            out.push(Tri {
                verts,
                idx: idx.into_iter().map(|i| i as u32).collect(),
            });
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
            edit_base: None,
            edit_touched: false,
            table: None,
            file_epsg: None,
            geom: Geom::default(),
            rev: 0,
            raster: Default::default(),
        };
        l.rebuild();
        l
    }

    /// La extensión, el lugar de la etiqueta y los triángulos de relleno de un elemento.
    fn geom_of(&self, f: &view::Feature) -> ([f64; 4], [f64; 2]) {
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
        (b, anchor)
    }

    /// Los elementos cuya extensión toca la ventana, en orden.
    pub fn in_window(&self, w: [f64; 4]) -> Vec<usize> {
        self.geom
            .index
            .get_or_init(|| hipso_core::rtree::RTree::new(&self.geom.bbox))
            .query(w)
    }

    /// Los triángulos de relleno del elemento `i`, calculados la primera vez que se piden.
    pub fn tris(&self, i: usize) -> &[Tri] {
        if self.kind != view::GeomKind::Polygon {
            return &[];
        }
        self.geom.tris[i].get_or_init(|| triangulate(&self.data.features[i].parts))
    }

    /// Recalcula lo que depende de la geometría (extensiones, lugares de etiqueta; los triángulos
    /// de relleno esperan a que se dibuje el elemento).
    pub fn rebuild(&mut self) {
        use rayon::prelude::*;
        let per: Vec<([f64; 4], [f64; 2])> = self
            .data
            .features
            .par_iter()
            .map(|f| self.geom_of(f))
            .collect();
        let mut g = Geom::default();
        for (b, a) in per {
            g.bbox.push(b);
            g.anchor.push(a);
            g.tris.push(Default::default());
        }
        self.geom = g;
        self.refresh_bounds();
        self.selected.retain(|i| *i < self.data.features.len());
    }

    /// La extensión de la capa, a partir de las extensiones de sus elementos.
    pub fn refresh_bounds(&mut self) {
        self.rev += 1;
        self.geom.nverts = self
            .data
            .features
            .iter()
            .map(|f| f.parts.iter().map(Vec::len).sum::<usize>())
            .sum();
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

    /// El elemento `i` cambió su geometría.
    pub fn update_feature(&mut self, i: usize) {
        let Some(f) = self.data.features.get(i) else {
            return;
        };
        let (b, a) = self.geom_of(f);
        self.geom.bbox[i] = b;
        self.geom.anchor[i] = a;
        self.geom.tris[i] = Default::default();
        self.geom.index = Default::default();
        self.refresh_bounds();
        self.dirty = true;
        self.reclassify_if_data();
    }

    /// Añade un elemento cuyo primer campo recibe un número correlativo; devuelve su índice.
    pub fn push_feature(&mut self, parts: Vec<Vec<[f64; 2]>>) -> usize {
        let n = self.data.features.len() + 1;
        self.data.features.push(view::Feature { parts });
        // el primer campo recibe el número correlativo cuando puede contenerlo
        let first_takes = !self.data.fields.is_empty()
            && matches!(
                self.data.field_type(0),
                hipso_core::fieldtype::FieldType::Text
                    | hipso_core::fieldtype::FieldType::Integer
                    | hipso_core::fieldtype::FieldType::Real
            );
        // se fija antes de que exista la fila: el tipo de una columna sin tipo sigue a sus valores
        self.data.fix_types();
        let row: Vec<String> = (0..self.data.fields.len())
            .map(|c| if c == 0 && first_takes { n.to_string() } else { String::new() })
            .collect();
        self.data
            .num
            .push(row.iter().map(|t| t.parse().unwrap_or(f64::NAN)).collect());
        self.data.text.push(row);
        let (b, a) = self.geom_of(&self.data.features[n - 1]);
        self.geom.bbox.push(b);
        self.geom.anchor.push(a);
        self.geom.tris.push(Default::default());
        self.geom.index = Default::default();
        self.refresh_bounds();
        self.dirty = true;
        self.reclassify_if_data();
        n - 1
    }

    /// Quita los elementos de los índices dados.
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

    /// Las clases que siguen a los datos se vuelven a calcular después de que estos cambien.
    pub fn reclassify_if_data(&mut self) {
        if self.sym.kind != SymKind::Single {
            self.reclassify();
        }
    }

    /// Pone al día la simbología y las etiquetas, que guardan su campo como índice, tras quitar el
    /// campo `removed` (los siguientes se corren una posición y el que lo usaba se queda sin
    /// campo) o, con `None`, tras deshacer o rehacer, que pueden dejar un índice fuera de la tabla.
    pub fn fix_field_refs(&mut self, removed: Option<usize>) {
        let n = self.data.fields.len();
        let shift = |f: &mut Option<usize>| match (*f, removed) {
            (Some(c), Some(r)) if c == r => *f = None,
            (Some(c), Some(r)) if c > r => *f = Some(c - 1),
            (Some(c), _) if c >= n => *f = None,
            _ => {}
        };
        shift(&mut self.sym.field);
        shift(&mut self.labels.field);
        if self.sym.field.is_none() {
            self.sym.breaks.clear();
            self.sym.colors.clear();
            self.sym.categories.clear();
        }
    }

    /// Vuelve a clasificar después de que cambie la simbología o los datos.
    pub fn reclassify(&mut self) {
        self.sym.classify(&self.data);
    }

    /// Recuerda los datos antes de un cambio, para poder deshacerlo.
    pub fn push_undo(&mut self) {
        self.edit_touched = true;
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
        self.fix_field_refs(None);
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
        self.fix_field_refs(None);
        self.rebuild();
        self.reclassify();
        true
    }

    /// El primer elemento en un punto del mapa (a menos de `tol` para puntos y líneas).
    pub fn feature_at(&self, x: f64, y: f64, tol: f64) -> Option<usize> {
        let near =
            |b: &[f64; 4]| x >= b[0] - tol && x <= b[2] + tol && y >= b[1] - tol && y <= b[3] + tol;
        // el último dibujado es el que está encima
        self.in_window([x - tol, y - tol, x + tol, y + tol])
            .into_iter()
            .rev()
            .find(|&i| {
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

/// El diálogo que hace una capa vectorial nueva.
#[derive(Default)]
pub struct NewLayerDlg {
    pub name: String,
    pub kind: view::GeomKind,
    pub fields: Vec<(String, hipso_core::fieldtype::FieldType)>,
    pub new_field: String,
    pub new_type: hipso_core::fieldtype::FieldType,
}

fn alpha(c: [u8; 3], a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Del mapa a la pantalla para un fotograma.
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

    /// La polilínea en pantalla, omitiendo los vértices a menos de un píxel del anterior.
    fn path(&self, part: &[[f64; 2]], min_d2: f32, heavy: bool) -> Vec<Pos2> {
        let mut out = Vec::with_capacity(part.len().min(1024));
        let mut last = Pos2::new(f32::NAN, f32::NAN);
        for (i, p) in part.iter().enumerate() {
            let s = self.s(p);
            if i == 0 || i + 1 == part.len() || (s - last).length_sq() >= min_d2 {
                out.push(s);
                last = s;
            }
        }
        if heavy && out.len() > 4 {
            // un contorno pequeño necesita pocos puntos: cada triángulo enviado a la pantalla
            // cuesta memoria
            let (mut lo, mut hi) = (out[0], out[0]);
            for p in &out {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            if (hi - lo).max_elem() < 1.5 {
                return Vec::new();
            }
            let keep = (((hi - lo).max_elem()) as usize).clamp(4, 64);
            if out.len() > keep {
                let n = out.len();
                out = (0..keep).map(|k| out[k * (n - 1) / (keep - 1)]).collect();
            }
        }
        out
    }
}

/// Una polilínea como un cuadrilátero por segmento en una malla (sin uniones ni suavizado: para los
/// muchos contornos finos de una capa grande, donde una forma por parte costaría gigabytes).
fn line_quads(mesh: &mut egui::Mesh, pts: &[Pos2], width: f32, col: Color32) {
    for s in pts.windows(2) {
        let d = s[1] - s[0];
        let len = d.length();
        if len < 1e-3 {
            continue;
        }
        let n = Vec2::new(-d.y, d.x) / len * (width / 2.0);
        let base = mesh.vertices.len() as u32;
        for p in [s[0] + n, s[0] - n, s[1] - n, s[1] + n] {
            mesh.colored_vertex(p, col);
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
}

fn flush(mesh: &mut egui::Mesh, clip: &egui::Painter) {
    if !mesh.is_empty() {
        clip.add(egui::Shape::mesh(std::mem::take(mesh)));
    }
}

impl App {
    /// El sistema del que y al que hay que mover una capa, cuando no es el del trabajo: `(desde,
    /// hasta)`. Una capa abierta antes que ninguna nube espera en `pending_conform` hasta que la
    /// primera nube fija el sistema del trabajo.
    pub fn crs_move(&mut self, p: &Path) -> Option<(u16, u16)> {
        let from = self.crs_of(p);
        self.crs_move_from(p, from)
    }

    /// [`Self::crs_move`] para una capa cuyo sistema ya se conoce (una tabla de un GeoPackage).
    pub fn crs_move_from(&mut self, _p: &Path, from: Option<u16>) -> Option<(u16, u16)> {
        let from = from?;
        match self.project_crs() {
            Some(to)
                if hipso_core::crs::relation(to, from) == hipso_core::crs::Relation::Different =>
            {
                Some((from, to))
            }
            Some(_) => None,
            None => {
                // la primera capa fija el sistema del trabajo hasta que una nube diga otra cosa
                self.work_crs = Some(from);
                None
            }
        }
    }

    /// Los sistemas que se prueban para una nube que no declara ninguno: las zonas UTM de ETRS89
    /// (Europa) y luego las de WGS 84, norte y sur.
    fn guess_list() -> Vec<u16> {
        (25828..=25838).chain(32601..=32660).chain(32701..=32760).collect()
    }

    /// Una nube que no declara sistema y una capa que sí lo hace y que no cae donde está la nube:
    /// el sistema en el que probablemente está la nube, juzgado por dónde caería la capa.
    pub fn assume_move(&mut self, from: Option<u16>, bounds: [f64; 4], cloud: [f64; 6]) -> Option<(u16, u16)> {
        let from = from?;
        if self.clouds.is_empty() || self.project_crs().is_some() {
            return None;
        }
        let b = cloud;
        let to = hipso_core::reproject::guess_system(bounds, from, [b[0], b[1], b[3], b[4]], &Self::guess_list())?;
        (hipso_core::crs::relation(from, to) == hipso_core::crs::Relation::Different)
            .then_some((from, to))
    }

    /// Las capas abiertas antes que una nube que no declara sistema siguen la misma conjetura.
    pub fn conform_to_undeclared_cloud(&mut self, cloud: [f64; 6]) {
        if self.project_crs().is_some() {
            return;
        }
        for k in 0..self.refs.len() {
            let path = self.refs[k].path.clone();
            let RefKind::Vector(v) = &self.refs[k].kind else { continue };
            let bounds = v.data.bounds;
            let from = self.crs_of(&path);
            if let Some((from, to)) = self.assume_move(from, bounds, cloud) {
                let name = self.refs[k].name.clone();
                let RefKind::Vector(v) = &mut self.refs[k].kind else { continue };
                if hipso_core::reproject::vector_data(&mut v.data, from, to).is_ok() {
                    v.rebuild();
                    self.plan_fit = true;
                    self.log(format!(
                        "La nube no declara su sistema de coordenadas: «{name}» (EPSG:{from}) se ha pasado a EPSG:{to}, que es donde cae sobre la nube. Si no es el de la nube, asígnaselo en LiDAR > Asignar proyección."
                    ));
                }
            }
        }
    }

    pub fn add_vector(&mut self, p: &Path) {
        if p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"))
            && let Ok(t) = hipso_core::gpkg::layers(p)
            && t.len() > 1
        {
            let first = t.first().map(|l| l.table.clone());
            self.gpkg_pick = Some((
                p.to_path_buf(),
                t.into_iter()
                    .map(|l| (l.table.clone(), Some(&l.table) == first.as_ref()))
                    .collect(),
            ));
            return;
        }
        self.add_vector_table(p, None);
    }

    /// Pregunta qué tablas de un GeoPackage abrir como capas.
    pub fn gpkg_window(&mut self, ctx: &egui::Context) {
        let Some((path, mut tables)) = self.gpkg_pick.take() else {
            return;
        };
        let (mut open, mut load) = (true, false);
        egui::Window::new("Tablas del GeoPackage")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} tiene varias tablas. ¿Cuáles se abren?",
                    path.display()
                ));
                for (name, on) in &mut tables {
                    ui.checkbox(on, name.as_str());
                }
                load = ui.button("Abrir").clicked();
            });
        if load {
            for (name, _) in tables.iter().filter(|t| t.1) {
                self.add_vector_table(&path, Some(name));
            }
        } else if open {
            self.gpkg_pick = Some((path, tables));
        }
    }

    pub fn add_vector_table(&mut self, p: &Path, table: Option<&str>) {
        let read = match (table, p.extension()) {
            (Some(t), _) => hipso_core::gpkg::read_vector(p, Some(t)),
            _ => view::read_vector(p),
        };
        match read {
            Ok(mut data) => {
                let own = table.and_then(|t| hipso_core::gpkg::epsg_of_table(p, t));
                let layer_crs = own.or_else(|| self.crs_of(p));
                let gpkg_path = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gpkg"));
                let mut moved = if own.is_some() {
                    self.crs_move_from(p, own)
                } else {
                    self.crs_move(p)
                };
                if moved.is_none() {
                    let cloud = self.store.as_ref().map(|s| s.bounds);
                    moved = cloud.and_then(|c| self.assume_move(layer_crs, data.bounds, c));
                    if let Some((from, to)) = moved {
                        self.log(format!(
                            "La nube no declara su sistema de coordenadas: «{}» está en EPSG:{from} y no cae sobre ella; se pasa a EPSG:{to}, donde sí cae. Si no es el de la nube, asígnaselo en LiDAR > Asignar proyección.",
                            p.display()
                        ));
                    }
                }
                if let Some((from, to)) = moved {
                    let started = std::time::Instant::now();
                    self.log(format!(
                        "«{}» está en {}, distinto del de la nube ({}): se pasa a él al cargar.",
                        p.display(),
                        hipso_core::crs::name(from),
                        hipso_core::crs::name(to)
                    ));
                    match hipso_core::reproject::vector_data(&mut data, from, to) {
                        Ok(()) => self.log(format!(
                            "  convertida en {:.1} s",
                            started.elapsed().as_secs_f64()
                        )),
                        Err(e) => {
                            self.log(format!(
                                "No se pudo pasar {} al sistema de la nube: {e}",
                                p.display()
                            ));
                            return;
                        }
                    }
                }
                if table.is_none() {
                    self.remove_paths(p);
                }
                let n = data.features.len();
                let kind = if data.points {
                    view::GeomKind::Point
                } else if data.polygons {
                    view::GeomKind::Polygon
                } else {
                    view::GeomKind::Line
                };
                self.add_ref(
                    RefLayer {
                        name: {
                            let f = p
                                .file_name()
                                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                            table.map_or(f.clone(), |t| format!("{f} · {t}"))
                        },
                        path: p.to_path_buf(),
                        kind: RefKind::Vector(Box::new({
                            let mut vl = VecLayer::new(data, kind);
                            vl.table = table.map(str::to_string).or_else(|| {
                                gpkg_path
                                    .then(|| hipso_core::gpkg::layers(p).ok().and_then(|l| l.into_iter().next()))
                                    .flatten()
                                    .map(|l| l.table)
                            });
                            vl.file_epsg = layer_crs;
                            vl
                        })),
                        visible: true,
                        opacity: 1.0,
                    },
                    false,
                );
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

    /// Abre una tabla sin geometría (CSV, TSV o XLSX) como una capa que no se dibuja: se ve en la
    /// tabla de atributos, se consulta y se grafica como cualquier capa.
    pub fn add_table(&mut self, p: &Path) {
        match view::read_table(p) {
            Ok(data) => {
                let n = data.features.len();
                let fields = data.fields.len();
                self.add_ref(
                    RefLayer {
                        name: p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
                        path: p.to_path_buf(),
                        kind: RefKind::Vector(Box::new(VecLayer::new(data, view::GeomKind::Point))),
                        visible: false,
                        opacity: 1.0,
                    },
                    false,
                );
                let k = self.refs.len() - 1;
                self.sel_layer = Some(k);
                self.active = Some(k);
                self.table_layer = Some(k);
                self.log(format!("Tabla cargada: {} ({n} filas, {fields} columnas). Tiene la tabla de atributos y los gráficos, pero no se dibuja en el mapa.", p.display()));
            }
            Err(e) => self.log(format!("No se pudo abrir {}: {e}", p.display())),
        }
    }

    /// La extensión de una capa: x mínima, y mínima, x máxima, y máxima.
    pub fn layer_extent(&self, k: usize) -> Option<[f64; 4]> {
        let b = match &self.refs.get(k)?.kind {
            RefKind::Raster { grid, .. } => [
                grid.x0,
                grid.y0,
                grid.x0 + grid.cols as f64 * grid.cell,
                grid.y0 + grid.rows as f64 * grid.cell,
            ],
            RefKind::Vector(v) => v.data.bounds,
            RefKind::Image(il) => il.display_extent(),
        };
        b.iter().all(|v| v.is_finite()).then_some(b)
    }

    /// La extensión de las capas visibles.
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

    /// Hace zoom de la planta a una extensión (un punto recibe una ventana de 40 m).
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
        // la vista 3D va donde fue la planta: con nubes muy separadas no mostraría ninguna
        if let Some(s) = self.store.clone() {
            self.orbit = self
                .orbit
                .fit_area(&s, [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad]);
            self.touch();
        }
    }

    /// Hace zoom a los puntos seleccionados o, si no hay, a los elementos seleccionados de la capa
    /// vectorial activa.
    pub fn zoom_selection(&mut self) {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        if let Some(s) = &self.store {
            for (i, on) in s.selected.iter().enumerate() {
                if *on {
                    let (x, y) = (
                        s.origin[0] + f64::from(s.x[i]),
                        s.origin[1] + f64::from(s.y[i]),
                    );
                    b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                }
            }
        }
        if !b[0].is_finite()
            && let Some(k) = self.active
            && let Some(RefKind::Vector(v)) = self.refs.get(k).map(|r| &r.kind)
        {
            for &i in &v.selected {
                if let Some(e) = v.geom.bbox.get(i) {
                    b = [
                        b[0].min(e[0]),
                        b[1].min(e[1]),
                        b[2].max(e[2]),
                        b[3].max(e[3]),
                    ];
                }
            }
        }
        if b.iter().all(|v| v.is_finite()) {
            self.zoom_to_bounds(b);
        } else {
            self.log(
                "No hay nada seleccionado que encuadrar (puntos o elementos de la capa activa).",
            );
        }
    }

    /// Encuadra la vista 3D en lo que muestra ahora la planta, conservando su dirección.
    pub fn relink_3d(&mut self) {
        let Some(s) = self.store.clone() else { return };
        let (w, h) = (self.plan_px.0.max(8), self.plan_px.1.max(8));
        let (lo, hi) = (
            self.plan.to_map(0.0, h as f64, w, h),
            self.plan.to_map(w as f64, 0.0, w, h),
        );
        self.orbit = self.orbit.fit_area(&s, [lo.0, lo.1, hi.0, hi.1]);
        self.touch();
    }

    pub fn zoom_to_layer(&mut self, k: usize) {
        if let Some(b) = self.layer_extent(k) {
            self.zoom_to_bounds(b);
        }
    }

    /// Dibuja las capas vectoriales en el orden de la lista.
    pub fn paint_vectors(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        w: usize,
        h: usize,
        range: std::ops::Range<usize>,
    ) {
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
        for (_, r) in self
            .refs
            .iter()
            .enumerate()
            .filter(|(k, r)| r.visible && range.contains(k))
        {
            let RefKind::Vector(v) = &r.kind else {
                continue;
            };
            let op = r.opacity.clamp(0.0, 1.0);
            if v.is_heavy() {
                self.paint_raster(&clip, v, rect, w, h, op);
                continue;
            }
            // un elemento bajo un píxel es un punto: su contorno y su relleno solo costarían tiempo
            let tiny = |i: usize| {
                let b = &v.geom.bbox[i];
                (b[2] - b[0]).max(b[3] - b[1]) * self.plan.scale < 1.0
            };
            let visible: Vec<usize> = v.in_window(view);
            // cuantos más vértices hay a la vista, más gruesos se dibujan y menos áreas pequeñas se
            // rellenan: millones de ellos teselados en cada fotograma tardan segundos y gigabytes
            let nv: usize = visible
                .iter()
                .map(|&i| v.data.features[i].parts.iter().map(Vec::len).sum::<usize>())
                .sum();
            let min_d2 = match nv {
                0..=200_000 => 0.7,
                200_001..=1_000_000 => 2.5,
                _ => 9.0,
            };
            let scale = self.plan.scale;
            // los rellenos comparten un presupuesto de vértices (sus triángulos cuestan memoria en
            // el renderizador): las áreas más pequeñas pierden el suyo primero
            let fill_min = if v.kind == view::GeomKind::Polygon && nv > 150_000 {
                const STEPS: [f64; 6] = [1.0, 4.0, 8.0, 16.0, 32.0, 64.0];
                let mut cost = [0usize; 6];
                for &i in &visible {
                    let b = &v.geom.bbox[i];
                    let px = (b[2] - b[0]).max(b[3] - b[1]) * scale;
                    let n: usize = v.data.features[i].parts.iter().map(Vec::len).sum();
                    for (k, s) in STEPS.iter().enumerate() {
                        if px >= *s {
                            cost[k] += n;
                        }
                    }
                }
                let k = cost.iter().position(|c| *c <= 150_000).unwrap_or(5);
                STEPS[k]
            } else {
                1.0
            };
            let tiny_fill = |i: usize| {
                let b = &v.geom.bbox[i];
                (b[2] - b[0]).max(b[3] - b[1]) * scale < fill_min
            };
            // lo mismo para los contornos: lo que es demasiado pequeño para su parte de segmentos
            // se dibuja como un punto
            let line_min = if v.kind != view::GeomKind::Point && nv > 150_000 {
                const STEPS: [f64; 7] = [1.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];
                let mut cost = [0usize; 7];
                for &i in &visible {
                    let b = &v.geom.bbox[i];
                    let px = (b[2] - b[0]).max(b[3] - b[1]) * scale;
                    let n: usize = v.data.features[i].parts.iter().map(Vec::len).sum();
                    let segs = n.min(px as usize + 1);
                    for (k, s) in STEPS.iter().enumerate() {
                        if px >= *s {
                            cost[k] += segs;
                        }
                    }
                }
                let k = cost.iter().position(|c| *c <= 150_000).unwrap_or(6);
                STEPS[k]
            } else {
                1.0
            };
            if v.kind == view::GeomKind::Polygon {
                use rayon::prelude::*;
                visible
                    .par_iter()
                    .filter(|&&i| !tiny_fill(i))
                    .for_each(|&i| {
                        v.tris(i);
                    });
            }
            // áreas: primero los rellenos, como una sola malla
            if v.kind == view::GeomKind::Polygon {
                let mut mesh = egui::Mesh::default();
                for &i in &visible {
                    if tiny_fill(i) {
                        continue;
                    }
                    let c = v.sym.color_of(&v.data, i, v.fill);
                    let col = alpha(c, v.fill_alpha * op);
                    for t in v.tris(i) {
                        let base = mesh.vertices.len() as u32;
                        for p in &t.verts {
                            mesh.colored_vertex(xf.s(p), col);
                        }
                        for tri in t.idx.as_chunks::<3>().0 {
                            mesh.add_triangle(base + tri[0], base + tri[1], base + tri[2]);
                        }
                        if mesh.vertices.len() > 50_000 {
                            flush(&mut mesh, &clip);
                        }
                    }
                }
                flush(&mut mesh, &clip);
            }
            let mut lines = egui::Mesh::default();
            // un punto por cuadrado de 3 px: los puntos que caen unos sobre otros cuestan
            // triángulos para nada
            let mut seen: HashSet<(i32, i32)> = HashSet::new();
            let mut fresh = |p: Pos2| seen.insert(((p.x / 3.0) as i32, (p.y / 3.0) as i32));
            for &i in &visible {
                let small = |i: usize| {
                    let b = &v.geom.bbox[i];
                    (b[2] - b[0]).max(b[3] - b[1]) * scale < line_min
                };
                if v.kind != view::GeomKind::Point && (tiny(i) || small(i)) {
                    let c = v.sym.color_of(&v.data, i, v.color);
                    let p = xf.s(&v.geom.anchor[i]);
                    if fresh(p) {
                        disc(&mut lines, p, 0.8, 4, alpha(c, op));
                        if lines.vertices.len() > 50_000 {
                            flush(&mut lines, &clip);
                        }
                    }
                    continue;
                }
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
                        if visible.len() > 2000 {
                            // miles de círculos como formas cuestan gigabytes: una malla en su
                            // lugar
                            let p = xf.s(&part[0]);
                            if fresh(p) {
                                disc(&mut lines, p, size, 6, col);
                                if lines.vertices.len() > 50_000 {
                                    flush(&mut lines, &clip);
                                }
                            }
                            continue;
                        }
                        clip.circle(
                            xf.s(&part[0]),
                            size,
                            col,
                            Stroke::new(1.0, Color32::from_black_alpha(150)),
                        );
                        continue;
                    }
                    let mut pts = xf.path(part, min_d2, nv > 200_000);
                    if v.kind == view::GeomKind::Polygon
                        && part.first() != part.last()
                        && let Some(f0) = pts.first().copied()
                    {
                        pts.push(f0);
                    }
                    if v.kind != view::GeomKind::Polygon || v.width > 0.0 {
                        if nv > 200_000 {
                            // una forma por parte cuesta unos 5 KB en egui: 200 000 partes son un
                            // gigabyte
                            line_quads(&mut lines, &pts, v.width.max(1.0), outline);
                            if lines.vertices.len() > 50_000 {
                                flush(&mut lines, &clip);
                            }
                        } else {
                            clip.add(egui::Shape::line(pts, Stroke::new(v.width, outline)));
                        }
                    }
                }
            }
            flush(&mut lines, &clip);
        }
    }

    /// Dibuja las etiquetas de las capas vectoriales de `range`; se hace tras cada capa para que
    /// ninguna capa de encima las oculte.
    pub fn paint_vector_labels(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        w: usize,
        h: usize,
        range: std::ops::Range<usize>,
    ) {
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
        for r in self
            .refs
            .iter()
            .enumerate()
            .filter(|(k, r)| r.visible && range.contains(k))
            .map(|(_, r)| r)
        {
            let RefKind::Vector(v) = &r.kind else {
                continue;
            };
            if v.is_heavy() || v.labels.field.is_none() {
                continue;
            }
            let visible: Vec<usize> = v.in_window(view);
            self.paint_labels(&clip, v, &visible, &xf, rect);
        }
    }

    /// Etiquetas sin solapes: una etiqueta que cubriría otra ya colocada se deja fuera.
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

    /// Lo que hay en un punto del mapa: los atributos de los elementos vectoriales y el valor de
    /// los rásteres, como líneas de texto.
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

    /// Lo que se sabe de cada capa que no es una nube: tipo, tamaño, sistema y extensión.
    pub fn layers_report(&mut self) -> String {
        let mut out = Vec::new();
        for k in 0..self.refs.len() {
            let path = self.refs[k].path.clone();
            let system = self
                .crs_of(&path)
                .map_or("sin declarar".to_string(), hipso_core::crs::name);
            let r = &self.refs[k];
            let mut l = vec![r.name.to_string()];
            let ext = |b: [f64; 4]| format!("X {:.2} a {:.2} · Y {:.2} a {:.2}", b[0], b[2], b[1], b[3]);
            match &r.kind {
                RefKind::Vector(v) => {
                    let kind = match v.kind {
                        view::GeomKind::Point => "puntos",
                        view::GeomKind::Line => "líneas",
                        view::GeomKind::Polygon => "polígonos",
                    };
                    l.push(format!("  Capa vectorial de {kind} · {} elementos · {} vértices", v.data.features.len(), v.geom.nverts));
                    l.push(format!("  Campos: {}", if v.data.fields.is_empty() { "ninguno".to_string() } else { v.data.fields.join(", ") }));
                    l.push(format!("  Extensión: {}", ext(v.data.bounds)));
                }
                RefKind::Raster { grid, .. } => {
                    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                    for v in grid.data.iter().filter(|v| v.is_finite()) {
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                    l.push(format!("  Ráster de una banda · {} × {} celdas de {} m", grid.cols, grid.rows, grid.cell));
                    l.push(format!("  Valores: {lo:.3} a {hi:.3}"));
                    l.push(format!(
                        "  Extensión: {}",
                        ext([grid.x0, grid.y0, grid.x0 + grid.cols as f64 * grid.cell, grid.y0 + grid.rows as f64 * grid.cell])
                    ));
                }
                RefKind::Image(il) => {
                    if let Some(f) = il.pyr.levels.first() {
                        l.push(format!("  Imagen de {} bandas · {} × {} píxeles de {} m", il.pyr.bands, f.cols, f.rows, il.pyr.cell));
                        l.push(format!(
                            "  Extensión: {}",
                            ext([il.pyr.x0, il.pyr.y_top - f64::from(f.rows) * il.pyr.cell, il.pyr.x0 + f64::from(f.cols) * il.pyr.cell, il.pyr.y_top])
                        ));
                    }
                }
            }
            l.push(format!("  Sistema: {system}"));
            if !r.path.as_os_str().is_empty() {
                l.push(format!("  Archivo: {}", r.path.display()));
            }
            out.push(l.join("\n"));
        }
        out.join("\n\n")
    }

    /// Dibuja la ficha de lo que encontró la herramienta de consulta en las capas; devuelve su
    /// lugar.
    pub fn paint_ident(&self, painter: &egui::Painter, rect: Rect) -> Option<Rect> {
        if self.ident.is_empty() {
            return None;
        }
        let wrap = (rect.width() - 40.0).clamp(120.0, 520.0);
        let galleys: Vec<_> = self
            .ident
            .iter()
            .map(|l| {
                painter.layout(l.clone(), FontId::monospace(11.0), Color32::WHITE, wrap)
            })
            .collect();
        let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 16.0;
        let h = 10.0 + galleys.iter().map(|g| g.size().y + 2.0).sum::<f32>();
        let b = Rect::from_min_size(
            rect.left_bottom() + Vec2::new(10.0, -h - 34.0),
            Vec2::new(w, h),
        );
        painter.rect_filled(b, 4.0, Color32::from_black_alpha(200));
        let mut y = 5.0;
        for g in galleys {
            let dy = g.size().y + 2.0;
            painter.galley(b.min + Vec2::new(8.0, y), g, Color32::WHITE);
            y += dy;
        }
        Some(b)
    }

    /// Los polígonos de las capas de polígonos visibles, para las herramientas que cortan con
    /// polígonos.
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

    // ------------------------------------------------------------------------ panel de capas

    /// El orden de dibujo, desde abajo: las capas bajo las nubes de puntos, las nubes como un solo
    /// elemento y las capas encima.
    pub fn order_items(&self) -> Vec<Item> {
        let n = self.refs.len();
        let slot = self.cloud_slot.min(n);
        let mut v: Vec<Item> = (0..slot).map(Item::Layer).collect();
        v.push(Item::Clouds);
        v.extend((slot..n).map(Item::Layer));
        v
    }

    /// Pone las capas (y las nubes) en el orden dado, desde abajo; la selección y los demás índices
    /// de capas siguen a su capa.
    pub fn apply_order(&mut self, items: &[Item]) {
        let n = self.refs.len();
        let layers: Vec<usize> = items
            .iter()
            .filter_map(|i| match i {
                Item::Layer(k) if *k < n => Some(*k),
                _ => None,
            })
            .collect();
        if layers.len() != n {
            return;
        }
        let mut to_new = vec![0usize; n];
        for (new, old) in layers.iter().enumerate() {
            to_new[*old] = new;
        }
        let mut taken: Vec<Option<RefLayer>> = std::mem::take(&mut self.refs)
            .into_iter()
            .map(Some)
            .collect();
        self.refs = layers.iter().filter_map(|k| taken[*k].take()).collect();
        self.cloud_slot =
            items
                .iter()
                .position(|i| *i == Item::Clouds)
                .map_or(self.cloud_slot, |p| {
                    items[..p]
                        .iter()
                        .filter(|i| matches!(i, Item::Layer(_)))
                        .count()
                });
        let fix = |o: &mut Option<usize>| {
            if let Some(i) = *o {
                *o = to_new.get(i).copied();
            }
        };
        fix(&mut self.sel_layer);
        fix(&mut self.active);
        fix(&mut self.table_layer);
        self.sel_layers = self
            .sel_layers
            .iter()
            .filter_map(|i| to_new.get(*i).copied())
            .collect();
        if let Some(i) = to_new.get(self.rs.layer) {
            self.rs.layer = *i;
        }
    }

    /// Añade una capa: un vector va sobre todo, un ráster o una imagen bajo las nubes de puntos
    /// (una imagen del terreno las ocultaría si no). Devuelve su índice.
    pub fn add_ref(&mut self, layer: RefLayer, under_clouds: bool) -> usize {
        if !under_clouds {
            self.refs.push(layer);
            return self.refs.len() - 1;
        }
        self.refs.insert(0, layer);
        self.cloud_slot += 1;
        for i in [&mut self.sel_layer, &mut self.active, &mut self.table_layer]
            .into_iter()
            .flatten()
        {
            *i += 1;
        }
        self.rs.layer += 1;
        self.sel_layers = self.sel_layers.iter().map(|i| i + 1).collect();
        0
    }

    /// Quita una capa y mantiene correctos los índices que apuntan a capas.
    pub fn remove_ref(&mut self, k: usize) {
        if k >= self.refs.len() {
            return;
        }
        self.refs.remove(k);
        if k < self.cloud_slot {
            self.cloud_slot -= 1;
        }
        let fix = |o: &mut Option<usize>| match *o {
            Some(i) if i == k => *o = None,
            Some(i) if i > k => *o = Some(i - 1),
            _ => {}
        };
        fix(&mut self.sel_layer);
        fix(&mut self.active);
        fix(&mut self.table_layer);
        self.sel_layers = self
            .sel_layers
            .iter()
            .filter(|i| **i != k)
            .map(|i| if *i > k { i - 1 } else { *i })
            .collect();
        if self.rs.layer > k {
            self.rs.layer -= 1;
        }
        self.dig_draft.clear();
    }

    /// Quita las capas hechas a partir de un archivo (se está cargando de nuevo).
    pub fn remove_paths(&mut self, p: &Path) {
        while let Some(k) = self.refs.iter().position(|r| r.path == p) {
            self.remove_ref(k);
        }
    }

    /// Mueve una capa, o las nubes, un lugar arriba o abajo en el orden de dibujo.
    pub fn step_item(&mut self, item: Item, up: bool) {
        let mut v = self.order_items();
        let Some(at) = v.iter().position(|i| *i == item) else {
            return;
        };
        let to = if up { at + 1 } else { at.wrapping_sub(1) };
        if to < v.len() {
            v.swap(at, to);
            self.apply_order(&v);
        }
    }

    /// Lo que pueden hacer como grupo las capas seleccionadas a la vez.
    pub fn group_command(&mut self, c: GroupCmd) {
        let sel: Vec<usize> = self
            .sel_layers
            .iter()
            .copied()
            .filter(|k| *k < self.refs.len())
            .collect();
        if sel.is_empty() {
            return;
        }
        match c {
            GroupCmd::Show | GroupCmd::Hide => {
                for k in sel {
                    self.refs[k].visible = c == GroupCmd::Show;
                }
            }
            GroupCmd::Zoom => {
                let b = sel
                    .iter()
                    .filter_map(|k| self.layer_extent(*k))
                    .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
                if let Some(b) = b {
                    self.zoom_to_bounds(b);
                }
            }
            GroupCmd::Up | GroupCmd::Down => {
                // el bloque se mueve un lugar, conservando las capas su orden entre sí; una capa
                // que tiene el borde u otra capa seleccionada por encima se queda
                let mut v = self.order_items();
                let chosen = |i: &Item| matches!(i, Item::Layer(k) if sel.contains(k));
                if c == GroupCmd::Up {
                    for at in (0..v.len().saturating_sub(1)).rev() {
                        if chosen(&v[at]) && !chosen(&v[at + 1]) {
                            v.swap(at, at + 1);
                        }
                    }
                } else {
                    for at in 1..v.len() {
                        if chosen(&v[at]) && !chosen(&v[at - 1]) {
                            v.swap(at - 1, at);
                        }
                    }
                }
                self.apply_order(&v);
            }
            GroupCmd::Remove => {
                for k in sel.into_iter().rev() {
                    self.remove_ref(k);
                }
                self.sel_layers.clear();
            }
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
                self.side = Side::Edit;
            }
            LayerCmd::Save => self.save_layer(k, false),
            LayerCmd::SaveAs => self.save_layer(k, true),
            LayerCmd::Bands => {
                self.rs.open = true;
                self.rs.layer = k;
            }
            LayerCmd::Up | LayerCmd::Down => self.step_item(Item::Layer(k), c == LayerCmd::Up),
            LayerCmd::Remove => self.remove_ref(k),
        }
        let _ = ctx;
    }

    /// Si una herramienta de edición vectorial es la actual.
    pub fn edit_layer_on(&self) -> bool {
        matches!(
            self.nav,
            Nav::Digitize | Nav::VecSelect | Nav::VecVertex | Nav::VecMove
        )
    }

    /// Mantiene la planta y la vista 3D sobre el mismo terreno: la que se haya ampliado o
    /// desplazado en este fotograma mueve a la otra.
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

    // ------------------------------------------------------------------------ capa nueva, guardar

    pub fn new_layer_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.new_layer.take() else {
            return;
        };
        if d.fields.is_empty() && d.name.is_empty() {
            d.name = format!("Capa {}", self.refs.len() + 1);
            d.fields = vec![("Nombre".to_string(), hipso_core::fieldtype::FieldType::Text)];
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
                    ui.radio_value(&mut d.kind, view::GeomKind::Polygon, "Polígonos");
                });
                ui.label("Campos de la tabla");
                let mut remove = None;
                for (k, (f, ty)) in d.fields.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(f).desired_width(140.0));
                        egui::ComboBox::from_id_salt(("nl_type", k))
                            .width(120.0)
                            .selected_text(ty.name())
                            .show_ui(ui, |ui| {
                                for t in hipso_core::fieldtype::FieldType::ALL {
                                    ui.selectable_value(ty, t, t.name()).on_hover_text(t.help());
                                }
                            });
                        if ui
                            .small_button("✖")
                            .on_hover_text("Quitar este campo de la tabla")
                            .clicked()
                        {
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
                            .desired_width(140.0),
                    );
                    egui::ComboBox::from_id_salt("nl_new_type")
                        .width(120.0)
                        .selected_text(d.new_type.name())
                        .show_ui(ui, |ui| {
                            for t in hipso_core::fieldtype::FieldType::ALL {
                                ui.selectable_value(&mut d.new_type, t, t.name()).on_hover_text(t.help());
                            }
                        });
                    let dup = d.fields.iter().any(|(f, _)| f.trim().eq_ignore_ascii_case(d.new_field.trim()));
                    if ui.add_enabled(!dup, egui::Button::new("Añadir campo")).clicked() && !d.new_field.trim().is_empty() {
                        let f = d.new_field.trim().to_string();
                        let t = d.new_type;
                        d.fields.push((f, t));
                        d.new_field.clear();
                    }
                });
                ui.label(egui::RichText::new(format!("{}: {}", d.new_type.name(), d.new_type.help())).size(11.0).weak());
                if ui.button("Crear").clicked() {
                    create = true;
                }
            });
        if create {
            let data = view::VectorData {
                fields: d.fields.iter().map(|f| f.0.clone()).collect(),
                types: d.fields.iter().map(|f| f.1).collect(),
                polygons: d.kind == view::GeomKind::Polygon,
                points: d.kind == view::GeomKind::Point,
                ..view::VectorData::default()
            };
            let mut layer = VecLayer::new(data, d.kind);
            layer.dirty = true;
            self.add_ref(
                RefLayer {
                    name: d.name.trim().to_string(),
                    path: PathBuf::new(),
                    kind: RefKind::Vector(Box::new(layer)),
                    visible: true,
                    opacity: 1.0,
                },
                false,
            );
            let k = self.refs.len() - 1;
            self.active = Some(k);
            self.sel_layer = Some(k);
            // una capa vacía no tiene extensión que ajustar: la vista se queda donde está
            self.plan_fit = self.plan_fit && self.refs.len() > 1;
            self.nav = Nav::Digitize;
            self.side = Side::Edit;
            self.log(format!(
                "Capa «{}» creada: digitaliza sobre la planta.",
                d.name
            ));
        } else if open {
            self.new_layer = Some(d);
        }
    }

    /// La proyección de la nube o, si no, la de una imagen del mapa.
    fn work_prj(&self) -> Option<String> {
        self.store.as_ref().and_then(|s| s.prj.clone()).or_else(|| {
            self.refs.iter().find_map(|r| match &r.kind {
                RefKind::Image(il) => il.pyr.epsg.and_then(hipso_core::vector::prj_for_epsg),
                _ => None,
            })
        })
    }

    /// Escribe una capa sobre su propia tabla de su GeoPackage, en el sistema en que estaba la
    /// tabla.
    pub fn save_gpkg_over(&mut self, k: usize) {
        let (work, prj) = (self.project_crs(), self.work_prj());
        let work = work.or_else(|| prj.as_deref().and_then(hipso_core::vector::epsg_from_prj));
        let Some(r) = self.refs.get_mut(k) else { return };
        let RefKind::Vector(v) = &mut r.kind else { return };
        let Some(table) = v.table.clone() else { return };
        let target = v.file_epsg.or(work);
        let moved = match (work, target) {
            (Some(w), Some(t)) if hipso_core::crs::relation(w, t) == hipso_core::crs::Relation::Different => {
                let mut d = v.data.clone();
                match hipso_core::reproject::vector_data(&mut d, w, t) {
                    Ok(()) => Some(d),
                    Err(e) => {
                        let msg = format!("No se pudo devolver la capa al sistema EPSG:{t} de la tabla: {e}");
                        self.log(msg);
                        return;
                    }
                }
            }
            _ => None,
        };
        let res = hipso_core::gpkg_write::replace_table(&r.path, &table, moved.as_ref().unwrap_or(&v.data), v.kind, target);
        let msg = match res {
            Ok(()) => {
                v.dirty = false;
                format!("Capa guardada en la tabla «{table}» de {}", r.path.display())
            }
            Err(e) => format!("No se pudo guardar sobre la tabla «{table}»: {e}"),
        };
        self.log(msg);
    }

    /// Guarda una capa vectorial como shapefile con la proyección de la nube: en su sitio cuando ya
    /// tiene un archivo y `ask` es falso.
    pub fn save_layer(&mut self, k: usize, ask: bool) {
        // una capa leída de un GeoPackage se guarda sobre su propia tabla, conservando las otras
        // tablas
        if !ask
            && let Some(RefLayer { path, kind: RefKind::Vector(v), .. }) = self.refs.get(k)
            && v.table.is_some()
            && path.is_file()
            && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gpkg"))
        {
            let path = path.clone();
            match hipso_core::gpkg_write::lost_on_rewrite(&path) {
                Ok(lost) if lost.is_empty() => self.save_gpkg_over(k),
                Ok(lost) => self.vm.gpkg_loss = Some((k, lost)),
                Err(e) => self.log(format!("No se puede leer el GeoPackage para guardar sobre él: {e}")),
            }
            return;
        }
        let prj = self.work_prj();
        let dir = self
            .clouds
            .first()
            .map_or_else(|| PathBuf::from("."), |c| hipso_core::paths::results_dir(c));
        let Some(r) = self.refs.get_mut(k) else {
            return;
        };
        let RefKind::Vector(v) = &mut r.kind else {
            return;
        };
        let path = if !ask
            && r.path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("shp") || e.eq_ignore_ascii_case("geojson"))
        {
            r.path.clone()
        } else {
            let stem = v.table.clone().unwrap_or_else(|| hipso_core::paths::stem(Path::new(&r.name)));
            let Some(p) = crate::prefs::dialog()
                .add_filter("Shapefile", &["shp"])
                .add_filter("GeoJSON", &["geojson"])
                .add_filter("GeoPackage", &["gpkg"])
                .add_filter("KML", &["kml"])
                .add_filter("KMZ", &["kmz"])
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
        let res = hipso_core::vector::write_layer(&path, &v.data, v.kind, prj.as_deref());
        match res {
            Ok(()) => {
                r.path = path.clone();
                r.name = path
                    .file_name()
                    .map_or(r.name.clone(), |n| n.to_string_lossy().into_owned());
                v.dirty = false;
                // el archivo escrito es ahora el de la propia capa: un GeoPackage tiene la tabla
                // con ese nombre
                v.table = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"))
                    .then(|| {
                        hipso_core::paths::stem(&path)
                            .chars()
                            .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
                            .collect()
                    });
                v.file_epsg = prj.as_deref().and_then(hipso_core::vector::epsg_from_prj);
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

/// Un elemento del orden de dibujo: una capa o las nubes de puntos juntas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Layer(usize),
    Clouds,
}

/// Órdenes para las capas seleccionadas a la vez.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupCmd {
    Show,
    Hide,
    Zoom,
    Up,
    Down,
    Remove,
}

/// Lo que pide el panel de capas a una capa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Un polígono regular relleno (un punto) como abanico de triángulos en una malla.
fn disc(mesh: &mut egui::Mesh, c: Pos2, r: f32, sides: u32, col: Color32) {
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(c, col);
    for k in 0..sides {
        let a = std::f32::consts::TAU * k as f32 / sides as f32;
        mesh.colored_vertex(c + Vec2::new(a.cos(), a.sin()) * r, col);
    }
    for k in 0..sides {
        mesh.add_triangle(base, base + 1 + k, base + 1 + (k + 1) % sides);
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    fn layer(name: &str) -> RefLayer {
        RefLayer {
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}.shp")),
            kind: RefKind::Vector(Box::new(VecLayer::new(
                view::VectorData::default(),
                view::GeomKind::Polygon,
            ))),
            visible: true,
            opacity: 1.0,
        }
    }

    fn names(a: &App) -> Vec<String> {
        a.order_items()
            .iter()
            .map(|i| match i {
                Item::Layer(k) => a.refs[*k].name.clone(),
                Item::Clouds => "NUBES".to_string(),
            })
            .collect()
    }

    #[test]
    fn the_clouds_move_through_the_layers_as_one_item() {
        let mut a = App::default();
        for n in ["a", "b", "c"] {
            a.add_ref(layer(n), false);
        }
        // nubes bajo todo, por defecto (posición 0), capas a b c desde abajo
        assert_eq!(names(&a), ["NUBES", "a", "b", "c"]);
        a.step_item(Item::Clouds, true);
        a.step_item(Item::Clouds, true);
        assert_eq!(names(&a), ["a", "b", "NUBES", "c"]);
        assert_eq!(a.cloud_slot, 2);
        // una capa cruza las nubes sin mover las demás
        a.sel_layer = Some(1);
        a.step_item(Item::Layer(1), true);
        assert_eq!(names(&a), ["a", "NUBES", "b", "c"]);
        // la selección sigue a su capa
        assert_eq!(a.refs[a.sel_layer.unwrap_or(99)].name, "b");
        // arriba del todo y abajo del todo
        a.step_item(Item::Clouds, true);
        a.step_item(Item::Clouds, true);
        assert_eq!(names(&a), ["a", "b", "c", "NUBES"]);
        a.step_item(Item::Clouds, true);
        assert_eq!(names(&a), ["a", "b", "c", "NUBES"]);
    }

    #[test]
    fn layers_selected_together_hide_move_and_go_as_a_group() {
        let mut a = App::default();
        for n in ["a", "b", "c", "d", "e"] {
            a.add_ref(layer(n), false);
        }
        a.sel_layer = Some(3);
        a.sel_layers = [1, 3].into_iter().collect();
        a.group_command(GroupCmd::Hide);
        let vis: Vec<bool> = a.refs.iter().map(|r| r.visible).collect();
        assert_eq!(vis, [true, false, true, false, true]);
        a.group_command(GroupCmd::Show);
        assert!(a.refs.iter().all(|r| r.visible));
        // subir: el bloque sube un lugar, conservando su orden; la selección sigue a sus capas
        a.group_command(GroupCmd::Up);
        assert_eq!(names(&a), ["NUBES", "a", "c", "b", "e", "d"]);
        assert_eq!(a.sel_layers.len(), 2);
        assert!(a.sel_layers.iter().all(|k| ["b", "d"].contains(&a.refs[*k].name.as_str())));
        // arriba del todo una capa se queda mientras las otras todavía suben
        a.group_command(GroupCmd::Up);
        assert_eq!(names(&a), ["NUBES", "a", "c", "e", "b", "d"]);
        a.group_command(GroupCmd::Down);
        assert_eq!(names(&a), ["NUBES", "a", "c", "b", "d", "e"]);
        // remove both
        a.group_command(GroupCmd::Remove);
        assert_eq!(names(&a), ["NUBES", "a", "c", "e"]);
        assert!(a.sel_layers.is_empty());
    }

    #[test]
    fn a_new_raster_goes_under_the_clouds_and_a_vector_over_them() {
        let mut a = App::default();
        a.add_ref(layer("v1"), false);
        a.sel_layer = Some(0);
        let k = a.add_ref(layer("ortofoto"), true);
        assert_eq!(k, 0);
        // la capa vectorial seleccionada sigue siendo la misma capa
        assert_eq!(a.refs[a.sel_layer.unwrap_or(99)].name, "v1");
        a.add_ref(layer("v2"), false);
        assert_eq!(names(&a), ["ortofoto", "NUBES", "v1", "v2"]);
    }

    #[test]
    fn removing_and_dragging_keep_the_slot_right() {
        let mut a = App::default();
        a.add_ref(layer("r1"), true);
        a.add_ref(layer("r2"), true);
        a.add_ref(layer("v1"), false);
        assert_eq!(names(&a), ["r2", "r1", "NUBES", "v1"]);
        a.remove_ref(0);
        assert_eq!(names(&a), ["r1", "NUBES", "v1"]);
        // todo el orden de una vez, como hace un arrastre: v1 al fondo
        let items = vec![Item::Layer(1), Item::Layer(0), Item::Clouds];
        a.apply_order(&items);
        assert_eq!(names(&a), ["v1", "r1", "NUBES"]);
        assert_eq!(a.cloud_slot, 2);
    }
}
