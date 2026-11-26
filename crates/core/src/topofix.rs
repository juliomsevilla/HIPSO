//! Reparación de la topología de una capa vectorial: lo que `topology` solo detecta, aquí se
//! corrige directamente.
//!
//! * **Polígonos.** Primero se ajustan los vértices a una distancia (los cercanos pasan a ser uno
//!   y los que quedan cerca del borde de otro polígono se apoyan en él, con un vértice común en
//!   los dos), de modo que los bordes que debían coincidir coincidan. Después cada polígono se
//!   hace válido, los solapes se quitan (la superficie común se queda con un solo polígono, según
//!   la regla elegida) y los huecos entre polígonos de superficie pequeña se rellenan con el
//!   vecino con el que comparten más borde.
//! * **Líneas.** Los extremos que quedan a menos de la distancia de otra línea se llevan a ella:
//!   al vértice más cercano o a un vértice nuevo sobre el segmento, que se inserta también en la
//!   otra línea, de modo que las dos quedan unidas.
//!
//! Cada fase tiene un coste lineal en el número de vértices y segmentos (índices de rejilla para
//! buscar vecinos), de modo que una capa de cientos de miles de elementos no es un problema. Los
//! atributos de cada elemento se conservan; un elemento que se queda sin geometría se quita.

use std::collections::HashMap;

use geo::algorithm::repair_polygon::MakeValid;
use geo::algorithm::validation::Validation;
use geo::{Area, BooleanOps, BoundingRect, Coord, Distance, Euclidean, LineString, MultiPolygon, Polygon};

use crate::cluster::Dsu;
use crate::geomops::{multipolygon_of, parts_of};
use crate::view::{GeomKind, VectorData};
use crate::{Error, Result};

/// A quién se queda la superficie que comparten dos polígonos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlapRule {
    /// El polígono de mayor superficie.
    Larger,
    /// El polígono de menor superficie.
    Smaller,
    /// El que está antes en la tabla.
    First,
}

/// Qué se repara y con qué tolerancias.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Distancia de ajuste, en unidades de la capa: los vértices (o, en líneas, los extremos) más
    /// cercanos se unen. 0 no ajusta nada.
    pub snap: f64,
    /// Superficie máxima de un hueco entre polígonos que se rellena. 0 no rellena ninguno.
    pub max_gap: f64,
    /// Quién se queda un solape; `None` los deja como están.
    pub overlaps: Option<OverlapRule>,
}

impl Default for Options {
    fn default() -> Self {
        Self { snap: 0.0, max_gap: 0.0, overlaps: Some(OverlapRule::Larger) }
    }
}

/// Lo que se hizo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Report {
    /// Vértices o extremos que cambiaron de sitio al ajustarlos.
    pub moved: usize,
    /// Vértices nuevos puestos sobre un borde para unirlo con el de otro elemento.
    pub inserted: usize,
    /// Elementos que no eran válidos y se repararon.
    pub made_valid: usize,
    /// Elementos de los que se quitó superficie compartida con otro.
    pub overlaps_fixed: usize,
    /// Superficie quitada en total por los solapes.
    pub overlap_area: f64,
    /// Huecos rellenados y superficie que sumaban.
    pub gaps_filled: usize,
    pub gap_area: f64,
    /// Huecos de más superficie que `max_gap`, que se dejan.
    pub gaps_left: usize,
    /// Elementos que se quedaron sin geometría y se quitaron.
    pub dropped: usize,
}

/// Repara la topología de `d` según `opts`. Solo hay reparación para polígonos y para líneas.
pub fn repair(d: &mut VectorData, kind: GeomKind, opts: &Options) -> Result<Report> {
    if !(opts.snap >= 0.0 && opts.max_gap >= 0.0) {
        return Err(Error::Param("el ajuste y la superficie de los huecos no pueden ser negativos".into()));
    }
    match kind {
        GeomKind::Polygon => Ok(repair_polygons(d, opts)),
        GeomKind::Line => Ok(repair_lines(d, opts)),
        GeomKind::Point => Err(Error::Param("la topología se repara en capas de polígonos o de líneas, no de puntos".into())),
    }
}

// ---------------------------------------------------------------------------------------------
// índices de rejilla

/// Una rejilla de cajas: los elementos que pueden estar cerca de una caja dada.
pub(crate) struct BoxIndex {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<u32>>,
}

impl BoxIndex {
    pub(crate) fn new(cell: f64) -> Self {
        Self { cell: cell.max(1e-12), cells: HashMap::new() }
    }

    fn range(&self, b: [f64; 4]) -> (i64, i64, i64, i64) {
        let f = |v: f64| (v / self.cell).floor() as i64;
        (f(b[0]), f(b[1]), f(b[2]), f(b[3]))
    }

    pub(crate) fn insert(&mut self, id: u32, b: [f64; 4]) {
        let (x0, y0, x1, y1) = self.range(b);
        for x in x0..=x1 {
            for y in y0..=y1 {
                self.cells.entry((x, y)).or_default().push(id);
            }
        }
    }

    /// Los elementos de las celdas que toca la caja, sin repetir y en orden de índice.
    pub(crate) fn query(&self, b: [f64; 4]) -> Vec<u32> {
        let (x0, y0, x1, y1) = self.range(b);
        let mut out = Vec::new();
        for x in x0..=x1 {
            for y in y0..=y1 {
                if let Some(v) = self.cells.get(&(x, y)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

pub(crate) fn expand(b: [f64; 4], by: f64) -> [f64; 4] {
    [b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

pub(crate) fn box_of(pts: &[[f64; 2]]) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

pub(crate) fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

// ---------------------------------------------------------------------------------------------
// ajuste de vértices

/// Una línea poligonal (cerrada o no) cuyos vértices se ajustan; `owner` es su índice.
struct Path {
    pts: Vec<[f64; 2]>,
    closed: bool,
}

impl Path {
    fn seg_count(&self) -> usize {
        if self.closed { self.pts.len() } else { self.pts.len().saturating_sub(1) }
    }

    fn seg(&self, i: usize) -> ([f64; 2], [f64; 2]) {
        (self.pts[i], self.pts[(i + 1) % self.pts.len()])
    }
}

/// El punto de un segmento más cercano a `p`: su parámetro (0 a 1), el punto y la distancia.
pub(crate) fn project(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, [f64; 2], f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    let q = [a[0] + t * dx, a[1] + t * dy];
    (t, q, dist(p, q))
}

/// Una rejilla con los segmentos de todos los caminos.
struct SegIndex {
    grid: BoxIndex,
    /// Por identificador de segmento: camino y número de segmento dentro de él.
    segs: Vec<(u32, u32)>,
}

impl SegIndex {
    fn new(paths: &[Path], snap: f64) -> Self {
        let mut lens: Vec<f64> = paths
            .iter()
            .flat_map(|p| (0..p.seg_count()).map(|i| {
                let (a, b) = p.seg(i);
                dist(a, b)
            }))
            .filter(|l| *l > 0.0)
            .collect();
        lens.sort_by(f64::total_cmp);
        let median = lens.get(lens.len() / 2).copied().unwrap_or(snap);
        // una celda no menor que la milésima parte de la extensión: con segmentos diminutos (rendijas) la
        // mediana sería minúscula y cada segmento largo ocuparía millones de celdas
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in paths.iter().flat_map(|p| p.pts.iter()) {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
        let floor = if b[0].is_finite() { (b[2] - b[0]).hypot(b[3] - b[1]) / 1024.0 } else { 0.0 };
        let mut grid = BoxIndex::new((2.0 * snap).max(median).max(floor));
        let mut segs = Vec::new();
        for (o, p) in paths.iter().enumerate() {
            for i in 0..p.seg_count() {
                let (a, b) = p.seg(i);
                grid.insert(segs.len() as u32, expand(box_of(&[a, b]), snap));
                segs.push((o as u32, i as u32));
            }
        }
        Self { grid, segs }
    }

    /// El segmento más cercano a `p` a menos de `snap` entre los que no son de caminos excluidos:
    /// (camino, segmento, parámetro, punto, distancia).
    fn nearest(
        &self,
        paths: &[Path],
        p: [f64; 2],
        snap: f64,
        skip: impl Fn(usize) -> bool,
    ) -> Option<(usize, usize, f64, [f64; 2], f64)> {
        let mut best: Option<(usize, usize, f64, [f64; 2], f64)> = None;
        for id in self.grid.query([p[0], p[1], p[0], p[1]]) {
            let (o, i) = self.segs[id as usize];
            let (o, i) = (o as usize, i as usize);
            if skip(o) {
                continue;
            }
            let (a, b) = paths[o].seg(i);
            let (t, q, d) = project(p, a, b);
            if d <= snap && best.is_none_or(|x| d < x.4) {
                best = Some((o, i, t, q, d));
            }
        }
        best
    }
}

/// Une los vértices que `which` marca (o todos, si es `None`) que están a menos de `snap`: cada
/// grupo pasa al primer vértice que tiene. Devuelve cuántos cambiaron de sitio.
fn cluster_vertices(paths: &mut [Path], snap: f64, which: impl Fn(&Path, usize) -> bool) -> usize {
    let mut ids: Vec<(usize, usize)> = Vec::new();
    for (o, p) in paths.iter().enumerate() {
        for i in 0..p.pts.len() {
            if which(p, i) {
                ids.push((o, i));
            }
        }
    }
    let at = |k: usize, paths: &[Path]| paths[ids[k].0].pts[ids[k].1];
    let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    let cell = |p: [f64; 2]| ((p[0] / snap).floor() as i64, (p[1] / snap).floor() as i64);
    let mut dsu = Dsu::new(ids.len());
    for k in 0..ids.len() {
        let p = at(k, paths);
        let (cx, cy) = cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(list) = grid.get(&(cx + dx, cy + dy)) {
                    for &j in list {
                        if dist(p, at(j as usize, paths)) <= snap {
                            dsu.union(k, j as usize);
                        }
                    }
                }
            }
        }
        grid.entry((cx, cy)).or_default().push(k as u32);
    }
    let mut moved = 0;
    let targets: Vec<[f64; 2]> = (0..ids.len()).map(|k| at(dsu.find(k), paths)).collect();
    for (k, &(o, i)) in ids.iter().enumerate() {
        let t = targets[k];
        if paths[o].pts[i] != t {
            paths[o].pts[i] = t;
            moved += 1;
        }
    }
    moved
}

/// Apoya en los bordes de otros caminos los vértices que `which` marca y que están a menos de
/// `snap` de uno: el vértice pasa al punto del borde y, si cae dentro de un segmento, se inserta
/// también en él. Dos vértices que caen cerca uno de otro sobre el mismo segmento comparten el
/// punto. `same_owner` dice si un camino es del mismo elemento que otro (no se ajusta a sí mismo).
/// Devuelve los vértices movidos y los insertados.
fn snap_to_edges(
    paths: &mut [Path],
    snap: f64,
    same_owner: impl Fn(usize, usize) -> bool,
    which: impl Fn(&Path, usize) -> bool,
) -> (usize, usize) {
    let index = SegIndex::new(paths, snap);
    let eps = snap * 1e-3;
    // puntos nuevos por segmento: (parámetro, punto)
    let mut inserts: HashMap<(usize, usize), Vec<(f64, [f64; 2])>> = HashMap::new();
    let mut moves: Vec<(usize, usize, [f64; 2])> = Vec::new();
    for o in 0..paths.len() {
        for i in 0..paths[o].pts.len() {
            if !which(&paths[o], i) {
                continue;
            }
            let p = paths[o].pts[i];
            let Some((o2, s, t, q, d)) = index.nearest(paths, p, snap, |x| same_owner(x, o)) else {
                continue;
            };
            if d <= eps {
                continue; // ya está sobre el borde (o sobre uno de sus vértices)
            }
            let (a, b) = paths[o2].seg(s);
            let target = if t * dist(a, b) <= eps {
                a
            } else if (1.0 - t) * dist(a, b) <= eps {
                b
            } else {
                // dentro del segmento: se reutiliza un punto ya insertado cerca
                let list = inserts.entry((o2, s)).or_default();
                match list.iter().find(|(_, r)| dist(*r, q) <= snap) {
                    Some((_, r)) => *r,
                    None => {
                        list.push((t, q));
                        q
                    }
                }
            };
            moves.push((o, i, target));
        }
    }
    let mut moved = 0;
    for (o, i, t) in moves {
        if paths[o].pts[i] != t {
            paths[o].pts[i] = t;
            moved += 1;
        }
    }
    // los vértices nuevos se insertan de atrás hacia delante dentro de cada camino, para que los
    // números de segmento siguientes sigan valiendo
    let mut by_path: HashMap<usize, Vec<(usize, Vec<(f64, [f64; 2])>)>> = HashMap::new();
    let mut inserted = 0;
    for ((o, s), mut list) in inserts {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        inserted += list.len();
        by_path.entry(o).or_default().push((s, list));
    }
    for (o, mut segs) in by_path {
        segs.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (s, list) in segs {
            let at = s + 1;
            let new: Vec<[f64; 2]> = list.into_iter().map(|x| x.1).collect();
            paths[o].pts.splice(at..at, new);
        }
    }
    (moved, inserted)
}

/// Quita los vértices repetidos seguidos (y el de cierre, si lo trae).
fn dedupe(pts: &mut Vec<[f64; 2]>, closed: bool) {
    pts.dedup();
    if closed && pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
}

// ---------------------------------------------------------------------------------------------
// polígonos

fn repair_polygons(d: &mut VectorData, opts: &Options) -> Report {
    let mut rep = Report::default();
    let n = d.features.len();
    // los anillos de cada elemento como caminos cerrados sin el vértice repetido del cierre
    let mut paths: Vec<Path> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for (row, f) in d.features.iter().enumerate() {
        for part in &f.parts {
            let mut pts = part.clone();
            dedupe(&mut pts, true);
            paths.push(Path { pts, closed: true });
            owner.push(row);
        }
    }
    if opts.snap > 0.0 {
        rep.moved += cluster_vertices(&mut paths, opts.snap, |_, _| true);
        let (m, i) = snap_to_edges(&mut paths, opts.snap, |a, b| owner[a] == owner[b], |_, _| true);
        rep.moved += m;
        rep.inserted += i;
    }
    // de vuelta a elementos: el ajuste puede dejar anillos de menos de tres vértices distintos
    let mut rings: Vec<Vec<Vec<[f64; 2]>>> = vec![Vec::new(); n];
    for (k, p) in paths.iter_mut().enumerate() {
        dedupe(&mut p.pts, true);
        if p.pts.len() >= 3 {
            let mut r = std::mem::take(&mut p.pts);
            r.push(r[0]);
            rings[owner[k]].push(r);
        }
    }
    let mut polys: Vec<MultiPolygon<f64>> = rings.iter().map(|r| multipolygon_of(r)).collect();
    // cada elemento, válido
    for m in &mut polys {
        if !m.0.is_empty() && !m.is_valid()
            && let Ok(v) = m.make_valid() {
                *m = v;
                rep.made_valid += 1;
            }
    }
    let scale = {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for m in &polys {
            if let Some(r) = m.bounding_rect() {
                b = [b[0].min(r.min().x), b[1].min(r.min().y), b[2].max(r.max().x), b[3].max(r.max().y)];
            }
        }
        if b[0].is_finite() { (b[2] - b[0]).hypot(b[3] - b[1]) } else { 0.0 }
    };
    // las operaciones booleanas dejan los bordes de los vecinos separados por rendijas de micras:
    // tras cada una, los vértices se llevan a un mismo sitio con una tolerancia mínima
    let tol = scale * 1e-6;
    if let Some(rule) = opts.overlaps {
        remove_overlaps(&mut polys, rule, scale, &mut rep);
        harmonise(&mut polys, tol);
    }
    if opts.max_gap > 0.0 {
        // un hueco puede ser una bahía abierta al exterior por una rendija; al cerrar las rendijas y
        // los huecos que ya lo son aparece como hueco: se repite hasta que no se rellena nada
        // solo cuenta como hueco real el de más de (una diezmilésima de la extensión)²: los
        // demás son ruido de las operaciones y no justifican otra pasada
        let noise = (scale * 1e-4).powi(2);
        for _ in 0..4 {
            let before = rep.gaps_filled;
            let area_before = rep.gap_area;
            let left_before = rep.gaps_left;
            fill_gaps(&mut polys, opts, scale, &mut rep);
            harmonise(&mut polys, tol);
            if rep.gaps_filled == before || rep.gap_area - area_before <= noise {
                break;
            }
            // los que se dejan por grandes se cuentan en la última pasada, no en cada una
            rep.gaps_left = left_before;
        }
        // la cuenta final de los que se dejan, en el estado en que quedó
        let mut probe = Report::default();
        let keep_polys = polys.clone();
        let mut ro = *opts;
        ro.max_gap = 0.0;
        count_big_gaps(&keep_polys, opts, &mut probe);
        rep.gaps_left = probe.gaps_left;
        let _ = ro;
        // rellenar un hueco con un vecino puede dejarle superficie encima de otro: se repite la regla
        // de los solapes sobre el resultado
        if let Some(rule) = opts.overlaps {
            let mut again = Report::default();
            remove_overlaps(&mut polys, rule, scale, &mut again);
            if again.overlaps_fixed > 0 {
                harmonise(&mut polys, tol);
            }
            rep.overlap_area += again.overlap_area;
        }
    }
    // de vuelta a la capa, quitando lo que no tiene geometría
    let keep: Vec<bool> = polys.iter().map(|m| !m.0.is_empty() && m.unsigned_area() > 0.0).collect();
    let mut k = 0;
    let mut features = Vec::with_capacity(n);
    let mut text = Vec::with_capacity(n);
    let mut num = Vec::with_capacity(n);
    for (row, m) in polys.iter().enumerate() {
        if !keep[row] {
            rep.dropped += 1;
            continue;
        }
        features.push(crate::view::Feature { parts: parts_of(m) });
        text.push(d.text.get(row).cloned().unwrap_or_default());
        num.push(d.num.get(row).cloned().unwrap_or_default());
        k += 1;
    }
    let _ = k;
    d.features = features;
    d.text = text;
    d.num = num;
    refit(d);
    rep
}

/// Cuenta los huecos de la unión mayores que `max_gap`, que no se rellenan.
fn count_big_gaps(polys: &[MultiPolygon<f64>], opts: &Options, rep: &mut Report) {
    let all: Vec<Polygon<f64>> = polys.iter().flat_map(|m| m.0.iter().cloned()).collect();
    if all.len() < 2 {
        return;
    }
    for p in &geo::unary_union(&all).0 {
        for h in p.interiors() {
            if Polygon::new(h.clone(), vec![]).unsigned_area() > opts.max_gap {
                rep.gaps_left += 1;
            }
        }
    }
}

/// Une los vértices de los polígonos que quedan a menos de `tol` unos de otros y de los bordes de
/// los demás, de modo que los bordes que deben coincidir coincidan exactamente.
fn harmonise(polys: &mut [MultiPolygon<f64>], tol: f64) {
    if !(tol > 0.0) {
        return;
    }
    let mut paths: Vec<Path> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for (row, m) in polys.iter().enumerate() {
        for ring in parts_of(m) {
            let mut pts = ring;
            dedupe(&mut pts, true);
            paths.push(Path { pts, closed: true });
            owner.push(row);
        }
    }
    cluster_vertices(&mut paths, tol, |_, _| true);
    snap_to_edges(&mut paths, tol, |a, b| owner[a] == owner[b], |_, _| true);
    let mut rings: Vec<Vec<Vec<[f64; 2]>>> = vec![Vec::new(); polys.len()];
    for (k, p) in paths.iter_mut().enumerate() {
        dedupe(&mut p.pts, true);
        if p.pts.len() >= 3 {
            let mut r = std::mem::take(&mut p.pts);
            r.push(r[0]);
            rings[owner[k]].push(r);
        }
    }
    for (m, r) in polys.iter_mut().zip(rings) {
        if r.is_empty() {
            continue;
        }
        let mut g = multipolygon_of(&r);
        if !g.is_valid()
            && let Ok(v) = g.make_valid()
        {
            g = v;
        }
        *m = g;
    }
}

/// Recalcula la caja de una capa.
fn refit(d: &mut VectorData) {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in d.features.iter().flat_map(|f| f.parts.iter().flatten()) {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    d.bounds = b;
}

fn box_of_multi(m: &MultiPolygon<f64>) -> Option<[f64; 4]> {
    m.bounding_rect().map(|r| [r.min().x, r.min().y, r.max().x, r.max().y])
}

/// Quita a cada polígono lo que cubre otro que tiene prioridad sobre él.
fn remove_overlaps(polys: &mut [MultiPolygon<f64>], rule: OverlapRule, scale: f64, rep: &mut Report) {
    let n = polys.len();
    let areas: Vec<f64> = polys.iter().map(|m| m.unsigned_area()).collect();
    let mut order: Vec<usize> = (0..n).collect();
    match rule {
        OverlapRule::Larger => order.sort_by(|&a, &b| areas[b].total_cmp(&areas[a]).then(a.cmp(&b))),
        OverlapRule::Smaller => order.sort_by(|&a, &b| areas[a].total_cmp(&areas[b]).then(a.cmp(&b))),
        OverlapRule::First => {}
    }
    let boxes: Vec<Option<[f64; 4]>> = polys.iter().map(box_of_multi).collect();
    // una celda del tamaño medio de las cajas
    let mut sides: Vec<f64> = boxes.iter().flatten().map(|b| (b[2] - b[0]).max(b[3] - b[1])).filter(|s| *s > 0.0).collect();
    sides.sort_by(f64::total_cmp);
    let cell = sides.get(sides.len() / 2).copied().unwrap_or(1.0).max(scale / 1024.0);
    let mut done = BoxIndex::new(cell);
    let min_area = scale * scale * 1e-18;
    for &i in &order {
        let Some(bi) = boxes[i] else { continue };
        let mut g = polys[i].clone();
        let before = areas[i];
        for j in done.query(bi) {
            let j = j as usize;
            if boxes[j].is_some_and(|bj| overlap(bi, bj)) && !g.0.is_empty() {
                g = g.difference(&polys[j]);
            }
        }
        let removed = before - g.unsigned_area();
        if removed > min_area {
            rep.overlaps_fixed += 1;
            rep.overlap_area += removed;
            polys[i] = g;
        }
        done.insert(i as u32, bi);
    }
}

/// Rellena con el vecino de más borde común los huecos de la unión que no pasan de `max_gap`.
fn fill_gaps(polys: &mut [MultiPolygon<f64>], opts: &Options, scale: f64, rep: &mut Report) {
    let all_polys: Vec<Polygon<f64>> = polys.iter().flat_map(|m| m.0.iter().cloned()).collect();
    if all_polys.len() < 2 {
        return;
    }
    let union = geo::unary_union(&all_polys);
    let eps = (opts.snap * 1e-3).max(scale * 1e-9);
    let boxes: Vec<Option<[f64; 4]>> = polys.iter().map(box_of_multi).collect();
    let mut sides: Vec<f64> = boxes.iter().flatten().map(|b| (b[2] - b[0]).max(b[3] - b[1])).filter(|s| *s > 0.0).collect();
    sides.sort_by(f64::total_cmp);
    let mut grid = BoxIndex::new(sides.get(sides.len() / 2).copied().unwrap_or(1.0).max(scale / 1024.0));
    for (i, b) in boxes.iter().enumerate() {
        if let Some(b) = b {
            grid.insert(i as u32, *b);
        }
    }
    for p in &union.0 {
        for h in p.interiors() {
            let hole = Polygon::new(h.clone(), vec![]);
            let area = hole.unsigned_area();
            if area > opts.max_gap {
                rep.gaps_left += 1;
                continue;
            }
            let hb = [
                h.0.iter().map(|c| c.x).fold(f64::INFINITY, f64::min),
                h.0.iter().map(|c| c.y).fold(f64::INFINITY, f64::min),
                h.0.iter().map(|c| c.x).fold(f64::NEG_INFINITY, f64::max),
                h.0.iter().map(|c| c.y).fold(f64::NEG_INFINITY, f64::max),
            ];
            // el borde del hueco que toca cada vecino
            let mut best: Option<(usize, f64)> = None;
            for c in grid.query(expand(hb, eps)) {
                let c = c as usize;
                let mut shared = 0.0;
                for w in h.0.windows(2) {
                    let mid = geo::Point::new((w[0].x + w[1].x) / 2.0, (w[0].y + w[1].y) / 2.0);
                    let near = polys[c].0.iter().any(|q| {
                        std::iter::once(q.exterior()).chain(q.interiors()).any(|r| Euclidean.distance(&mid, r) <= eps)
                    });
                    if near {
                        shared += (w[0].x - w[1].x).hypot(w[0].y - w[1].y);
                    }
                }
                if shared > 0.0 && best.is_none_or(|b| shared > b.1) {
                    best = Some((c, shared));
                }
            }
            if let Some((c, _)) = best {
                polys[c] = polys[c].union(&MultiPolygon::new(vec![hole]));
                rep.gaps_filled += 1;
                rep.gap_area += area;
            } else {
                rep.gaps_left += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// líneas

fn repair_lines(d: &mut VectorData, opts: &Options) -> Report {
    let mut rep = Report::default();
    if opts.snap <= 0.0 {
        return rep;
    }
    let mut paths: Vec<Path> = Vec::new();
    let mut owner: Vec<(usize, usize)> = Vec::new();
    for (row, f) in d.features.iter().enumerate() {
        for (k, part) in f.parts.iter().enumerate() {
            let mut pts = part.clone();
            pts.dedup();
            let closed = pts.len() > 2 && pts.first() == pts.last();
            if closed {
                pts.pop();
            }
            paths.push(Path { pts, closed });
            owner.push((row, k));
        }
    }
    // solo se ajustan los extremos de las líneas abiertas
    let is_end = |p: &Path, i: usize| !p.closed && (i == 0 || i + 1 == p.pts.len());
    rep.moved += cluster_vertices(&mut paths, opts.snap, is_end);
    let (m, i) = snap_to_edges(&mut paths, opts.snap, |a, b| a == b, is_end);
    rep.moved += m;
    rep.inserted += i;
    for (k, p) in paths.iter().enumerate() {
        let (row, part) = owner[k];
        let mut pts = p.pts.clone();
        if p.closed {
            pts.push(pts[0]);
        }
        d.features[row].parts[part] = pts;
    }
    refit(d);
    rep
}

/// Una línea poligonal de `geo` a partir de puntos.
#[allow(dead_code)]
fn line_of(pts: &[[f64; 2]]) -> LineString<f64> {
    LineString::new(pts.iter().map(|p| Coord { x: p[0], y: p[1] }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Feature;

    fn sq(x: f64, y: f64, w: f64, h: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x, y + h], [x + w, y + h], [x + w, y], [x, y]]
    }

    fn layer(kind: GeomKind, feats: Vec<Vec<Vec<[f64; 2]>>>) -> VectorData {
        let mut d = VectorData { polygons: kind == GeomKind::Polygon, points: kind == GeomKind::Point, ..VectorData::default() };
        d.fields = vec!["id".into()];
        for (k, parts) in feats.into_iter().enumerate() {
            d.features.push(Feature { parts });
            d.text.push(vec![k.to_string()]);
            d.num.push(vec![k as f64]);
        }
        d
    }

    fn area(d: &VectorData, row: usize) -> f64 {
        multipolygon_of(&d.features[row].parts).unsigned_area()
    }

    #[test]
    fn overlap_goes_to_the_larger_polygon() {
        // 10 x 10 y 6 x 6 que lo pisa en 4 x 4 = 16
        let mut d = layer(GeomKind::Polygon, vec![vec![sq(0.0, 0.0, 10.0, 10.0)], vec![sq(6.0, 6.0, 6.0, 6.0)]]);
        let r = repair(&mut d, GeomKind::Polygon, &Options::default()).unwrap();
        assert_eq!(r.overlaps_fixed, 1);
        assert!((r.overlap_area - 16.0).abs() < 1e-9);
        assert!((area(&d, 0) - 100.0).abs() < 1e-9);
        assert!((area(&d, 1) - 20.0).abs() < 1e-9);
        assert!(crate::topology::overlaps(&d, 1e-9).is_empty());
        // y al más pequeño si se pide
        let mut d = layer(GeomKind::Polygon, vec![vec![sq(0.0, 0.0, 10.0, 10.0)], vec![sq(6.0, 6.0, 6.0, 6.0)]]);
        let o = Options { overlaps: Some(OverlapRule::Smaller), ..Options::default() };
        repair(&mut d, GeomKind::Polygon, &o).unwrap();
        assert!((area(&d, 0) - 84.0).abs() < 1e-9);
        assert!((area(&d, 1) - 36.0).abs() < 1e-9);
    }

    #[test]
    fn snapping_closes_a_sliver_between_neighbours_and_keeps_their_attributes() {
        // dos cuadrados con una ranura de 0,05 entre ellos
        let mut d = layer(GeomKind::Polygon, vec![vec![sq(0.0, 0.0, 10.0, 10.0)], vec![sq(10.05, 0.0, 10.0, 10.0)]]);
        assert!(!crate::topology::gaps(&d, 1e-9).is_empty() || crate::topology::overlaps(&d, 1e-9).is_empty());
        let o = Options { snap: 0.1, max_gap: 1.0, overlaps: Some(OverlapRule::Larger) };
        let r = repair(&mut d, GeomKind::Polygon, &o).unwrap();
        assert!(r.moved >= 2, "{r:?}");
        assert_eq!(d.features.len(), 2);
        assert_eq!(d.text[1][0], "1");
        // el borde común es el mismo: no quedan huecos ni solapes
        assert!(crate::topology::gaps(&d, 1e-9).is_empty());
        assert!(crate::topology::overlaps(&d, 1e-9).is_empty());
    }

    #[test]
    fn a_small_hole_between_four_polygons_is_filled_and_a_big_one_is_left() {
        let mut rings = Vec::new();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (0.0, 1.0), (2.0, 1.0), (0.0, 2.0), (1.0, 2.0), (2.0, 2.0)] {
            rings.push(vec![sq(x, y, 1.0, 1.0)]);
        }
        let mut small = layer(GeomKind::Polygon, rings.clone());
        let o = Options { max_gap: 1.5, ..Options::default() };
        let r = repair(&mut small, GeomKind::Polygon, &o).unwrap();
        assert_eq!(r.gaps_filled, 1);
        assert!((r.gap_area - 1.0).abs() < 1e-9);
        let total: f64 = (0..small.features.len()).map(|k| area(&small, k)).sum();
        assert!((total - 9.0).abs() < 1e-9);
        assert!(crate::topology::gaps(&small, 1e-9).is_empty());
        let mut big = layer(GeomKind::Polygon, rings);
        let o = Options { max_gap: 0.5, ..Options::default() };
        let r = repair(&mut big, GeomKind::Polygon, &o).unwrap();
        assert_eq!((r.gaps_filled, r.gaps_left), (0, 1));
    }

    #[test]
    fn a_vertex_near_the_edge_of_another_polygon_shares_a_new_vertex_with_it() {
        // la esquina del segundo cae a 0,02 del lado derecho del primero
        let mut d = layer(GeomKind::Polygon, vec![vec![sq(0.0, 0.0, 10.0, 10.0)], vec![vec![[10.02, 4.0], [20.0, 4.0], [20.0, 6.0], [10.02, 6.0], [10.02, 4.0]]]]);
        let o = Options { snap: 0.05, max_gap: 0.0, overlaps: None };
        let r = repair(&mut d, GeomKind::Polygon, &o).unwrap();
        assert!(r.inserted >= 1, "{r:?}");
        assert!(d.features[0].parts[0].iter().any(|p| (p[0] - 10.0).abs() < 1e-12 && (p[1] - 4.0).abs() < 1e-9));
        assert!(d.features[0].parts[0].iter().any(|p| (p[0] - 10.0).abs() < 1e-12 && (p[1] - 6.0).abs() < 1e-9));
    }

    #[test]
    fn a_bow_tie_becomes_valid() {
        let bow = vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0], [0.0, 0.0]];
        let mut d = layer(GeomKind::Polygon, vec![vec![bow], vec![sq(30.0, 0.0, 5.0, 5.0)]]);
        let r = repair(&mut d, GeomKind::Polygon, &Options::default()).unwrap();
        assert_eq!(r.made_valid, 1);
        assert!(crate::topology::self_intersections(&d).is_empty());
    }

    #[test]
    fn an_undershooting_line_is_joined_to_the_one_it_should_meet() {
        // una línea horizontal y otra que termina 0,3 antes de ella
        let mut d = layer(GeomKind::Line, vec![vec![vec![[0.0, 0.0], [10.0, 0.0]]], vec![vec![[5.0, 5.0], [5.0, 0.3]]]]);
        assert_eq!(crate::topology::dangles(&d, 0.01).len(), 4);
        let o = Options { snap: 0.5, ..Options::default() };
        let r = repair(&mut d, GeomKind::Line, &o).unwrap();
        assert_eq!((r.moved, r.inserted), (1, 1), "{r:?}");
        assert_eq!(d.features[1].parts[0].last(), Some(&[5.0, 0.0]));
        // la horizontal tiene ahora un vértice en el punto de unión
        assert_eq!(d.features[0].parts[0], vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]]);
        // quedan los dos extremos libres de verdad, no el que se unió
        assert_eq!(crate::topology::dangles(&d, 0.01).len(), 3);
    }

    #[test]
    fn endpoints_close_to_each_other_become_one() {
        let mut d = layer(GeomKind::Line, vec![vec![vec![[0.0, 0.0], [10.0, 0.0]]], vec![vec![[10.1, 0.1], [20.0, 0.0]]]]);
        let o = Options { snap: 0.5, ..Options::default() };
        let r = repair(&mut d, GeomKind::Line, &o).unwrap();
        assert_eq!(r.moved, 1);
        assert_eq!(d.features[1].parts[0][0], [10.0, 0.0]);
    }

    #[test]
    fn rotated_tiles_that_overlap_come_out_without_slivers_for_the_checker() {
        // una cuadrícula de 4 x 4 cuadrados girados 17° y con coordenadas de UTM, que se pisan 0,4 m: las
        // operaciones booleanas dejan rendijas de micras que la comprobación de topología vería
        let (cos, sin) = (17f64.to_radians().cos(), 17f64.to_radians().sin());
        let rot = |x: f64, y: f64| [440_000.0 + x * cos - y * sin, 4_470_000.0 + x * sin + y * cos];
        let mut feats = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let (x0, y0) = (f64::from(i) * 50.0, f64::from(j) * 50.0);
                let (x1, y1) = (x0 + 50.4, y0 + 50.4);
                feats.push(vec![vec![rot(x0, y0), rot(x0, y1), rot(x1, y1), rot(x1, y0), rot(x0, y0)]]);
            }
        }
        let mut d = layer(GeomKind::Polygon, feats);
        assert!(!crate::topology::overlaps(&d, 1e-6).is_empty());
        let o = Options { snap: 0.0, max_gap: 10.0, overlaps: Some(OverlapRule::Larger) };
        repair(&mut d, GeomKind::Polygon, &o).unwrap();
        let left_overlaps = crate::topology::overlaps(&d, 1e-6);
        let left_gaps = crate::topology::gaps(&d, 1e-6);
        assert!(left_overlaps.is_empty(), "quedan solapes: {:?}", left_overlaps.iter().map(|i| i.area).collect::<Vec<_>>());
        assert!(left_gaps.is_empty(), "quedan huecos: {:?}", left_gaps.iter().map(|i| i.area).collect::<Vec<_>>());
        // y la superficie total es la de la unión: no se ha perdido nada
        let total: f64 = (0..d.features.len()).map(|k| area(&d, k)).sum();
        assert!((total - 200.4 * 200.4).abs() < 0.01, "{total}");
    }

    #[test]
    fn points_are_refused_and_negative_tolerances_too() {
        let mut d = layer(GeomKind::Point, vec![vec![vec![[0.0, 0.0]]]]);
        assert!(repair(&mut d, GeomKind::Point, &Options::default()).is_err());
        let mut p = layer(GeomKind::Polygon, vec![vec![sq(0.0, 0.0, 1.0, 1.0)]]);
        assert!(repair(&mut p, GeomKind::Polygon, &Options { snap: -1.0, ..Options::default() }).is_err());
    }
}
