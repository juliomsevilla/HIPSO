//! Corredores de líneas eléctricas: los conductores reconstruidos a partir de los puntos
//! clasificados como cables, los postes a partir de los clasificados como postes, y la vegetación
//! que se acerca demasiado o que podría alcanzar la línea si cayera.
//!
//! 1. Los puntos de cable que se tocan se unen en un conductor cada uno; el trazado del conductor
//!    es la altura y el desplazamiento medianos de sus puntos a lo largo de su dirección principal,
//!    cada `step` metros, y los huecos (unos metros donde el láser no dio en el cable) se rellenan
//!    con una parábola ajustada a los puntos contiguos, que sigue la flecha en lugar de cortarla.
//! 2. Los postes son los grupos de puntos de poste; los vanos de un conductor son los tramos entre
//!    los postes que están a su lado.
//! 3. La vegetación se lee una vez más; cada punto a menos de `half_width` de un conductor se mide
//!    contra él, y los puntos se agrupan en celdas de `cell` metros, cada una recordando su punto
//!    más alto y su mayor aproximación. Una celda es un peligro cuando se acerca más que
//!    `min_distance` («invasión») o cuando su altura más un margen alcanza la posición horizontal
//!    del conductor («caída»).
//!
//! Lo que no puede hacer: los puntos de cable de los vuelos aéreos son pocos y a veces faltan, así
//! que un conductor con huecos largos se comunica en trozos y su distancia de seguridad allí no se
//! mide; y la distancia de seguridad depende de la tensión, que es el usuario quien debe fijar.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cluster::{Dsu, median};
use crate::filter::ClassSel;
use crate::grid::Grid;
use crate::las::Reader;
use crate::paths::{self, Formats};
use crate::vector::{ColKind, Table, Val, prj_for_epsg, write_points, write_polylines};
use crate::{Error, Result, check_cancel};

pub const WIRE: u8 = 14;
pub const POLE: u8 = 15;

#[derive(Clone, Debug, PartialEq)]
pub struct CorridorParams {
    /// Clase de los puntos de cable (ASPRS 14, conductor); otra solo para nubes que usan sus
    /// propios códigos.
    pub wire_class: u8,
    /// Clase de los puntos de poste (ASPRS 15, torre de transmisión).
    pub pole_class: u8,
    /// Los puntos de cable más cercanos que esto son el mismo conductor.
    pub link: f64,
    /// Espaciado de los vértices del trazado de un conductor.
    pub step: f64,
    /// Los conductores que van uno al lado de otro se distinguen cuando sus puntos están más lejos
    /// que esto entre sí a través de la línea (lado y altura, m).
    pub lane: f64,
    /// Un conductor hecho de menos puntos se descarta.
    pub min_points: usize,
    /// Un conductor más corto que esto se descarta: un vano mide decenas de metros, mientras que
    /// las «líneas» cortas que un clasificador encuentra en las copas de los árboles son unos
    /// pocos.
    pub min_length: f64,
    /// Un poste más bajo que esto, o más ancho que `pole_max_width`, no es un poste.
    pub min_pole_height: f64,
    pub pole_max_width: f64,
    /// Los huecos más largos que esto no se rellenan: el conductor se corta allí.
    pub gap_max: f64,
    /// La vegetación más lejos que esto de todo conductor no se mira.
    pub half_width: f64,
    /// A menos de esta distancia de un conductor (en el espacio) hay invasión.
    pub min_distance: f64,
    /// Un árbol cuya altura más este margen alcanza la posición horizontal del conductor podría
    /// golpearlo al caer.
    pub fall_margin: f64,
    /// Lado de las celdas en las que se agrupa la vegetación.
    pub cell: f64,
    /// La vegetación por debajo de esta altura sobre el suelo se ignora.
    pub min_height: f64,
    /// Los puntos de poste más cercanos que esto son el mismo poste.
    pub pole_link: f64,
    /// Un poste más lejos que esto de un conductor no es uno de sus apoyos.
    pub pole_reach: f64,
    pub vegetation: ClassSel,
}

impl Default for CorridorParams {
    fn default() -> Self {
        Self {
            wire_class: WIRE,
            pole_class: POLE,
            link: 2.5,
            step: 2.0,
            lane: 1.0,
            min_points: 30,
            min_length: 25.0,
            min_pole_height: 6.0,
            pole_max_width: 8.0,
            gap_max: 30.0,
            half_width: 30.0,
            min_distance: 3.0,
            fall_margin: 1.0,
            cell: 3.0,
            min_height: 2.0,
            pole_link: 1.5,
            pole_reach: 8.0,
            vegetation: ClassSel::parse("3,4,5").unwrap_or_else(|_| ClassSel::all()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CorridorJob {
    pub inputs: Vec<PathBuf>,
    /// Modelo del suelo, para saber cuán alta es la vegetación.
    pub dtm: PathBuf,
    /// Ruta base de los resultados.
    pub output: PathBuf,
    pub params: CorridorParams,
    pub formats: Formats,
}

pub struct Outputs {
    pub conductors: Option<PathBuf>,
    pub poles: Option<PathBuf>,
    pub hazards: Option<PathBuf>,
    pub spans: PathBuf,
    pub hazards_table: PathBuf,
}

impl CorridorJob {
    pub fn outputs(&self) -> Outputs {
        let f = self.formats.fixed();
        let named = |tag: &str, ext: &str| {
            let b = paths::swap_ext(&self.output, ext);
            let stem = b
                .file_stem()
                .map_or(String::new(), |s| s.to_string_lossy().into_owned());
            b.with_file_name(format!("{stem}_{tag}.{ext}"))
        };
        Outputs {
            conductors: f.vector.then(|| named("conductores", "shp")),
            poles: f.vector.then(|| named("postes", "shp")),
            hazards: f.vector.then(|| named("riesgo", "shp")),
            spans: named("vanos", "csv"),
            hazards_table: named("riesgo", "csv"),
        }
    }
}

// ----------------------------------------------------------------------------- clustering

/// Grupos de puntos más cercanos que `link` (encadenados), en 3D cuando `three_d`, si no en planta.
fn cluster(pts: &[[f64; 3]], link: f64, three_d: bool) -> Vec<Vec<usize>> {
    let labels = crate::cluster::link_labels(
        pts.len(),
        |i| pts[i],
        link,
        if three_d { 1.0 } else { 0.0 },
    );
    crate::cluster::groups(&labels)
}

/// Parábola de mínimos cuadrados `z = a + b t + c t²` por `(t, z)`; `None` cuando es degenerada.
fn parabola(samples: &[(f64, f64)]) -> Option<[f64; 3]> {
    let n = samples.len() as f64;
    let (mut s1, mut s2, mut s3, mut s4) = (0.0, 0.0, 0.0, 0.0);
    let (mut z0, mut z1, mut z2) = (0.0, 0.0, 0.0);
    for &(t, z) in samples {
        s1 += t;
        s2 += t * t;
        s3 += t * t * t;
        s4 += t * t * t * t;
        z0 += z;
        z1 += t * z;
        z2 += t * t * z;
    }
    // ecuaciones normales por la regla de Cramer
    let m = [[n, s1, s2], [s1, s2, s3], [s2, s3, s4]];
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    if d.abs() < 1e-12 {
        return None;
    }
    let rhs = [z0, z1, z2];
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut mk = m;
        for r in 0..3 {
            mk[r][k] = rhs[r];
        }
        *o = det(&mk) / d;
    }
    Some(out)
}

/// Une grupos de puntos de cable que son trozos de un mismo conductor partido por un hueco en los
/// retornos del láser: apuntan en la misma dirección, el hueco es más corto que `gap_max` y el
/// extremo de uno está en la línea del otro.
fn join_pieces(pts: &[[f64; 3]], groups: Vec<Vec<usize>>, p: &CorridorParams) -> Vec<Vec<usize>> {
    if groups.len() < 2 || groups.len() > 5000 {
        return groups;
    }
    struct Piece {
        end: [[f64; 3]; 2],
        axis: [f64; 2],
    }
    let pieces: Vec<Piece> = groups
        .iter()
        .map(|g| {
            let n = g.len() as f64;
            let (mx, my) = (
                g.iter().map(|&i| pts[i][0]).sum::<f64>() / n,
                g.iter().map(|&i| pts[i][1]).sum::<f64>() / n,
            );
            let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
            for &i in g {
                let (dx, dy) = (pts[i][0] - mx, pts[i][1] - my);
                sxx += dx * dx;
                syy += dy * dy;
                sxy += dx * dy;
            }
            let a = 0.5 * (2.0 * sxy).atan2(sxx - syy);
            let (ux, uy) = (a.cos(), a.sin());
            let s = |i: usize| (pts[i][0] - mx) * ux + (pts[i][1] - my) * uy;
            let lo = g
                .iter()
                .copied()
                .min_by(|&a, &b| s(a).total_cmp(&s(b)))
                .unwrap_or(g[0]);
            let hi = g
                .iter()
                .copied()
                .max_by(|&a, &b| s(a).total_cmp(&s(b)))
                .unwrap_or(g[0]);
            Piece {
                end: [pts[lo], pts[hi]],
                axis: [ux, uy],
            }
        })
        .collect();
    let mut dsu = Dsu::new(groups.len());
    for a in 0..groups.len() {
        for b in a + 1..groups.len() {
            let (pa, pb) = (&pieces[a], &pieces[b]);
            // el par de extremos que se enfrentan
            let mut best: Option<(f64, [f64; 3], [f64; 3])> = None;
            for ea in &pa.end {
                for eb in &pb.end {
                    let d = (ea[0] - eb[0]).hypot(ea[1] - eb[1]);
                    if best.is_none_or(|x| d < x.0) {
                        best = Some((d, *ea, *eb));
                    }
                }
            }
            let Some((d, ea, eb)) = best else { continue };
            if d > p.gap_max || d <= p.link {
                continue;
            }
            // misma dirección (en cualquier sentido), y el otro extremo está en la línea de este
            let cos = (pa.axis[0] * pb.axis[0] + pa.axis[1] * pb.axis[1]).abs();
            let v = [(eb[0] - ea[0]) / d, (eb[1] - ea[1]) / d];
            let along_a = (v[0] * pa.axis[0] + v[1] * pa.axis[1]).abs();
            let along_b = (v[0] * pb.axis[0] + v[1] * pb.axis[1]).abs();
            let slope_ok = (eb[2] - ea[2]).abs() <= 0.3 * d + 1.0;
            if cos > 0.985 && along_a > 0.97 && along_b > 0.97 && slope_ok {
                dsu.union(a, b);
            }
        }
    }
    let mut merged: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, g) in groups.into_iter().enumerate() {
        merged.entry(dsu.find(k)).or_default().extend(g);
    }
    let mut out: Vec<Vec<usize>> = merged.into_values().collect();
    out.sort_by_key(|g| g[0]);
    // los trozos de un metro o dos, demasiado cortos para saber su dirección, se unen por estar en
    // fila: la misma prueba que usa la clasificación para los trozos de un conductor
    let centres: Vec<[f64; 3]> = out
        .iter()
        .map(|g| {
            let n = g.len() as f64;
            let mut c = [0.0; 3];
            for &i in g {
                for (k, v) in c.iter_mut().enumerate() {
                    *v += pts[i][k] / n;
                }
            }
            c
        })
        .collect();
    let mut row = crate::cluster::in_a_row(
        &centres,
        2.0 * p.link,
        p.gap_max,
        0.5,
        crate::cluster::STRAIGHT,
    );
    let mut rows: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, g) in out.into_iter().enumerate() {
        rows.entry(row.find(k)).or_default().extend(g);
    }
    let mut out: Vec<Vec<usize>> = rows.into_values().collect();
    out.sort_by_key(|g| g[0]);
    out
}

/// Un conductor: un trazado de vértices (x, y, z) cada `step` metros, en orden.
#[derive(Clone, Debug)]
pub struct Conductor {
    pub verts: Vec<[f64; 3]>,
    /// Distancia a lo largo del trazado de cada vértice.
    pub cum: Vec<f64>,
    pub points: usize,
    /// Tramos que se rellenaron.
    pub gaps: usize,
    /// Partes del grupo descartadas por estar cortadas por un hueco demasiado largo para rellenar.
    pub breaks: usize,
}

impl Conductor {
    fn from_verts(verts: Vec<[f64; 3]>, points: usize, gaps: usize, breaks: usize) -> Self {
        let mut cum = vec![0.0];
        for w in verts.windows(2) {
            let d = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            cum.push(cum[cum.len() - 1] + d);
        }
        Self {
            verts,
            cum,
            points,
            gaps,
            breaks,
        }
    }

    pub fn length(&self) -> f64 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// El punto del trazado más cercano a `(x, y)` en planta: la distancia a lo largo del trazado y
    /// la distancia en planta hasta él.
    pub fn locate(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let mut best: Option<(f64, f64)> = None;
        for k in 1..self.verts.len() {
            let (d, t) = segment(&self.verts[k - 1], &self.verts[k], x, y);
            if best.is_none_or(|b| d < b.1) {
                best = Some((self.cum[k - 1] + t * (self.cum[k] - self.cum[k - 1]), d));
            }
        }
        best
    }
}

/// Las muestras de un tramo de un grupo: sus puntos en (lado, altura) agrupados a menos de `lane`
/// unos de otros, como (lado mediano, altura mediana, puntos). Los conductores paralelos, a uno o
/// tres metros, son grupos separados; un conductor es uno.
fn lane_samples(side: &[f64], z: &[f64], lane: f64) -> Vec<(f64, f64, usize)> {
    let labels = crate::cluster::link_labels(side.len(), |i| [side[i], z[i], 0.0], lane, 0.0);
    crate::cluster::groups(&labels)
        .into_iter()
        .map(|g| {
            let mut d: Vec<f64> = g.iter().map(|&i| side[i]).collect();
            let mut h: Vec<f64> = g.iter().map(|&i| z[i]).collect();
            (median(&mut d), median(&mut h), g.len())
        })
        .collect()
}

/// Un conductor seguido a lo largo del grupo: el vértice de cada tramo donde tiene puntos,
/// (distancia a lo largo, lado, altura), y cuántos puntos tiene.
struct Lane {
    vs: Vec<Option<(f64, f64, f64)>>,
    last: (f64, f64),
    last_bin: usize,
    points: usize,
}

/// Un trazado por los tramos de un carril, con los vacíos entre los llenos rellenados por una
/// parábola de sus vecinos (salvo que el hueco sea más largo que `gap_max`): las partes que quedan,
/// cuántos huecos se rellenaron y cuántos eran demasiado largos.
fn trace(
    mut vs: Vec<Option<(f64, f64, f64)>>,
    smin: f64,
    p: &CorridorParams,
) -> (Vec<Vec<(f64, f64, f64)>>, usize, usize) {
    let nbins = vs.len();
    let mut pieces: Vec<Vec<(f64, f64, f64)>> = Vec::new();
    let mut cur: Vec<(f64, f64, f64)> = Vec::new();
    let mut gaps = 0usize;
    let mut breaks = 0usize;
    let mut b = 0;
    while b < nbins {
        if let Some(v) = vs[b] {
            cur.push(v);
            b += 1;
            continue;
        }
        let start = b;
        while b < nbins && vs[b].is_none() {
            b += 1;
        }
        let len_m = (b - start) as f64 * p.step;
        if cur.is_empty() || b >= nbins {
            // los huecos en los extremos son solo conductores más cortos
            continue;
        }
        if len_m > p.gap_max {
            pieces.push(std::mem::take(&mut cur));
            breaks += 1;
            continue;
        }
        // parábola por hasta cuatro vértices a cada lado
        let before: Vec<(f64, f64, f64)> = cur.iter().rev().take(4).copied().collect();
        let after: Vec<(f64, f64, f64)> = (b..nbins.min(b + 4)).filter_map(|k| vs[k]).collect();
        let mid = smin + (start as f64 + (b - start) as f64 / 2.0) * p.step;
        let samples: Vec<(f64, f64)> = before
            .iter()
            .chain(&after)
            .map(|v| (v.0 - mid, v.2))
            .collect();
        let fitted = (samples.len() >= 4).then(|| parabola(&samples)).flatten();
        let (first, last) = (before[0], after[0]);
        for (k, slot) in vs.iter_mut().enumerate().take(b).skip(start) {
            let s = smin + (k as f64 + 0.5) * p.step;
            let f = (s - first.0) / (last.0 - first.0);
            let side_v = first.1 + f * (last.1 - first.1);
            let z = match fitted {
                Some(c) => {
                    let t = s - mid;
                    c[0] + c[1] * t + c[2] * t * t
                }
                None => first.2 + f * (last.2 - first.2),
            };
            *slot = Some((s, side_v, z));
            cur.push((s, side_v, z));
        }
        gaps += 1;
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    (pieces, gaps, breaks)
}

/// Los trazados de conductor de un grupo de puntos de cable: uno por cada conductor que hay en él
/// (los carriles de puntos que van uno al lado de otro a lo largo de su dirección principal), y más
/// de uno de ellos cuando un hueco largo lo corta.
fn fit(points: &[[f64; 3]], p: &CorridorParams) -> Vec<Conductor> {
    let n = points.len() as f64;
    let (mx, my) = (
        points.iter().map(|q| q[0]).sum::<f64>() / n,
        points.iter().map(|q| q[1]).sum::<f64>() / n,
    );
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for q in points {
        let (dx, dy) = (q[0] - mx, q[1] - my);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let angle = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (ux, uy) = (angle.cos(), angle.sin());
    let along = |q: &[f64; 3]| (q[0] - mx) * ux + (q[1] - my) * uy;
    let side = |q: &[f64; 3]| -(q[0] - mx) * uy + (q[1] - my) * ux;
    let smin = points.iter().map(&along).fold(f64::INFINITY, f64::min);
    let smax = points.iter().map(&along).fold(f64::NEG_INFINITY, f64::max);
    let nbins = ((smax - smin) / p.step).floor() as usize + 1;
    let mut bins: Vec<(Vec<f64>, Vec<f64>)> = vec![(Vec::new(), Vec::new()); nbins];
    for q in points {
        let b = (((along(q) - smin) / p.step).floor() as usize).min(nbins - 1);
        bins[b].0.push(side(q));
        bins[b].1.push(q[2]);
    }
    // se siguen los conductores de tramo en tramo: una muestra va al carril cuyo último vértice
    // está más cerca en (lado, altura), si está a menos de dos anchuras de carril; si no, empieza
    // un carril nuevo
    let max_jump = 2.0 * p.lane;
    let max_idle = (p.gap_max / p.step).ceil() as usize + 1;
    let mut lanes: Vec<Lane> = Vec::new();
    for (b, (d, z)) in bins.iter().enumerate() {
        if z.is_empty() {
            continue;
        }
        let s = smin + (b as f64 + 0.5) * p.step;
        let mut taken = vec![false; lanes.len()];
        let mut samples = lane_samples(d, z, p.lane);
        samples.sort_by_key(|v| std::cmp::Reverse(v.2));
        for (sd, sz, cnt) in samples {
            let near = lanes
                .iter()
                .enumerate()
                .filter(|(k, l)| !taken[*k] && b - l.last_bin <= max_idle)
                .map(|(k, l)| (k, (l.last.0 - sd).hypot(l.last.1 - sz)))
                .filter(|(_, dist)| *dist <= max_jump)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let k = match near {
                Some((k, _)) => k,
                None => {
                    lanes.push(Lane {
                        vs: vec![None; nbins],
                        last: (sd, sz),
                        last_bin: b,
                        points: 0,
                    });
                    taken.push(false);
                    lanes.len() - 1
                }
            };
            taken[k] = true;
            let l = &mut lanes[k];
            l.vs[b] = Some((s, sd, sz));
            l.last = (sd, sz);
            l.last_bin = b;
            l.points += cnt;
        }
    }
    let mut out = Vec::new();
    for l in lanes {
        if l.vs.iter().flatten().count() < 3 {
            continue;
        }
        let (pieces, gaps, breaks) = trace(l.vs, smin, p);
        for v in pieces.into_iter().filter(|v| v.len() >= 2) {
            let verts = v
                .iter()
                .map(|&(s, d, z)| [mx + s * ux - d * uy, my + s * uy + d * ux, z])
                .collect();
            out.push(Conductor::from_verts(verts, l.points, gaps, breaks));
        }
    }
    out
}

// ----------------------------------------------------------------------------- la escena

/// A pole.
#[derive(Clone, Debug)]
pub struct Pole {
    pub x: f64,
    pub y: f64,
    pub base_z: f64,
    pub top_z: f64,
    pub points: usize,
    /// El lado mayor de su extensión en planta.
    pub width: f64,
}

/// El tramo de un conductor entre dos postes.
#[derive(Clone, Debug)]
pub struct Span {
    pub conductor: usize,
    pub index: usize,
    pub pole_a: Option<usize>,
    pub pole_b: Option<usize>,
    /// Distancia a lo largo del conductor donde empieza y termina.
    pub from: f64,
    pub to: f64,
    pub sag: f64,
}

#[derive(Default)]
struct SpanStats {
    invasion: u32,
    fall: u32,
    min_d3: f64,
    tallest: f64,
}

/// Un grupo de puntos de vegetación.
#[derive(Clone, Copy)]
struct Cell {
    max_h: f64,
    top: [f64; 3],
    d_h: f64,
    wire_z: f64,
    d3_min: f64,
    conductor: usize,
    along: f64,
}

/// Un peligro encontrado.
#[derive(Clone, Debug)]
pub struct Hazard {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub height: f64,
    pub horizontal: f64,
    pub distance: f64,
    pub clearance: f64,
    pub invasion: bool,
    pub fall: bool,
    pub conductor: usize,
    pub span: usize,
}

/// Lo que halló una ejecución.
pub struct Scene {
    pub conductors: Vec<Conductor>,
    pub poles: Vec<Pole>,
    pub spans: Vec<Span>,
    pub hazards: Vec<Hazard>,
    pub span_stats: Vec<(f64, u32, u32, f64)>,
    pub vegetation_points: u64,
    /// Trozos de conductor dejados fuera por ser más cortos que `min_length`.
    pub discarded: usize,
}

/// El punto del segmento `a`–`b` más cercano a `(x, y)` en planta: `(distancia, t)`.
fn segment(a: &[f64; 3], b: &[f64; 3], x: f64, y: f64) -> (f64, f64) {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = vx * vx + vy * vy;
    let t = if l2 > 0.0 {
        (((x - a[0]) * vx + (y - a[1]) * vy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((x - a[0] - t * vx).hypot(y - a[1] - t * vy), t)
}

/// Índice en planta de los vértices de los conductores.
struct Index {
    cell: f64,
    grid: HashMap<(i64, i64), Vec<(u32, u32)>>,
}

impl Index {
    fn new(conductors: &[Conductor], cell: f64) -> Self {
        let mut grid: HashMap<(i64, i64), Vec<(u32, u32)>> = HashMap::new();
        for (c, cd) in conductors.iter().enumerate() {
            for (i, v) in cd.verts.iter().enumerate() {
                grid.entry(((v[0] / cell).floor() as i64, (v[1] / cell).floor() as i64))
                    .or_default()
                    .push((c as u32, i as u32));
            }
        }
        Self { cell, grid }
    }

    /// El conductor más cercano a `(x, y, z)` en planta a menos de `reach`: su número, la distancia
    /// a lo largo de él, la distancia en planta y la altura del conductor allí.
    fn nearest(
        &self,
        conductors: &[Conductor],
        x: f64,
        y: f64,
        reach: f64,
    ) -> Option<(usize, f64, f64, f64)> {
        let (kx, ky) = (
            (x / self.cell).floor() as i64,
            (y / self.cell).floor() as i64,
        );
        let mut best: Option<(f64, u32, u32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &(c, i) in self.grid.get(&(kx + dx, ky + dy)).into_iter().flatten() {
                    let v = &conductors[c as usize].verts[i as usize];
                    let d = (x - v[0]).hypot(y - v[1]);
                    if d <= reach && best.is_none_or(|b| d < b.0) {
                        best = Some((d, c, i));
                    }
                }
            }
        }
        let (_, c, i) = best?;
        let cd = &conductors[c as usize];
        let i = i as usize;
        // se refina sobre los dos segmentos junto al vértice
        let mut out: Option<(f64, f64, f64)> = None;
        for (a, b) in [(i.wrapping_sub(1), i), (i, i + 1)] {
            if a < cd.verts.len() && b < cd.verts.len() {
                let (d, t) = segment(&cd.verts[a], &cd.verts[b], x, y);
                if out.is_none_or(|o| d < o.0) {
                    let z = cd.verts[a][2] + t * (cd.verts[b][2] - cd.verts[a][2]);
                    let s = cd.cum[a] + t * (cd.cum[b] - cd.cum[a]);
                    out = Some((d, s, z));
                }
            }
        }
        let (d, s, z) = out?;
        Some((c as usize, s, d, z))
    }
}

/// Postes a partir de los puntos de poste.
fn find_poles(pts: &[[f64; 3]], p: &CorridorParams) -> Vec<Pole> {
    cluster(pts, p.pole_link, false)
        .into_iter()
        .filter(|g| g.len() >= 5)
        .map(|g| {
            let n = g.len() as f64;
            let (w0, w1) = (
                g.iter().map(|&i| pts[i][0]).fold(f64::INFINITY, f64::min),
                g.iter()
                    .map(|&i| pts[i][0])
                    .fold(f64::NEG_INFINITY, f64::max),
            );
            let (v0, v1) = (
                g.iter().map(|&i| pts[i][1]).fold(f64::INFINITY, f64::min),
                g.iter()
                    .map(|&i| pts[i][1])
                    .fold(f64::NEG_INFINITY, f64::max),
            );
            Pole {
                width: crate::cluster::core_spread(&g.iter().map(|&i| pts[i][0]).collect::<Vec<_>>())
                    .max(crate::cluster::core_spread(
                        &g.iter().map(|&i| pts[i][1]).collect::<Vec<_>>(),
                    ))
                    .min((w1 - w0).max(v1 - v0)),
                x: g.iter().map(|&i| pts[i][0]).sum::<f64>() / n,
                y: g.iter().map(|&i| pts[i][1]).sum::<f64>() / n,
                base_z: g.iter().map(|&i| pts[i][2]).fold(f64::INFINITY, f64::min),
                top_z: g
                    .iter()
                    .map(|&i| pts[i][2])
                    .fold(f64::NEG_INFINITY, f64::max),
                points: g.len(),
            }
        })
        .collect()
}

/// Los vanos de cada conductor: entre postes consecutivos que están a su lado.
fn find_spans(conductors: &[Conductor], poles: &[Pole], p: &CorridorParams) -> Vec<Span> {
    let mut spans = Vec::new();
    for (c, cd) in conductors.iter().enumerate() {
        // postes junto a este conductor (todo conductor junto al que está un poste lo tiene como
        // apoyo) y dónde están a lo largo de él
        let mut mine: Vec<(f64, usize)> = poles
            .iter()
            .enumerate()
            .filter_map(|(k, pole)| {
                let (s, d) = cd.locate(pole.x, pole.y)?;
                (d <= p.pole_reach).then_some((s, k))
            })
            .collect();
        mine.sort_by(|a, b| a.0.total_cmp(&b.0));
        let z_at = |s: f64| -> f64 {
            let k = cd
                .cum
                .partition_point(|&v| v < s)
                .clamp(1, cd.verts.len() - 1);
            let (a, b) = (k - 1, k);
            let f = if cd.cum[b] > cd.cum[a] {
                (s - cd.cum[a]) / (cd.cum[b] - cd.cum[a])
            } else {
                0.0
            };
            cd.verts[a][2] + f.clamp(0.0, 1.0) * (cd.verts[b][2] - cd.verts[a][2])
        };
        let mut bounds: Vec<(f64, Option<usize>)> = vec![(0.0, None)];
        bounds.extend(mine.iter().map(|&(s, k)| (s, Some(k))));
        bounds.push((cd.length(), None));
        for (idx, w) in bounds.windows(2).enumerate() {
            let (from, to) = (w[0].0, w[1].0);
            if to - from < p.step {
                continue;
            }
            // flecha: cuánto cuelga el conductor por debajo de la cuerda entre los extremos del
            // vano
            let (za, zb) = (z_at(from), z_at(to));
            let sag = cd
                .verts
                .iter()
                .zip(&cd.cum)
                .filter(|(_, s)| **s >= from && **s <= to)
                .map(|(v, s)| za + (zb - za) * (s - from) / (to - from).max(1e-9) - v[2])
                .fold(0.0, f64::max);
            spans.push(Span {
                conductor: c,
                index: idx,
                pole_a: w[0].1,
                pole_b: w[1].1,
                from,
                to,
                sag,
            });
        }
    }
    spans
}

fn span_of(spans: &[Span], conductor: usize, s: f64) -> usize {
    spans
        .iter()
        .position(|sp| sp.conductor == conductor && s >= sp.from && s <= sp.to)
        .or_else(|| spans.iter().position(|sp| sp.conductor == conductor))
        .unwrap_or(0)
}

/// Lee las nubes y encuentra los conductores, los postes y los peligros.
pub fn analyse(j: &CorridorJob) -> Result<Scene> {
    let p = &j.params;
    if !(p.link > 0.0 && p.step > 0.0 && p.cell > 0.0 && p.half_width > 0.0) {
        return Err(Error::Param("las distancias deben ser positivas".into()));
    }
    let dtm = Grid::read(&j.dtm)?;
    // pasada 1: cables y postes
    let (mut wires, mut pole_pts): (Vec<[f64; 3]>, Vec<[f64; 3]>) = (Vec::new(), Vec::new());
    for input in &j.inputs {
        Reader::open(input)?.scan(|pt| {
            if pt.withheld {
                return;
            }
            if pt.class == p.wire_class {
                wires.push([pt.x, pt.y, pt.z]);
            } else if pt.class == p.pole_class {
                pole_pts.push([pt.x, pt.y, pt.z]);
            }
        })?;
        check_cancel()?;
    }
    if wires.is_empty() {
        return Err(Error::Param(format!(
            "la nube no tiene puntos de cables (clase {}): clasifica antes «Cables y postes» en Clasificar",
            p.wire_class
        )));
    }
    let mut conductors: Vec<Conductor> = Vec::new();
    for g in join_pieces(&wires, cluster(&wires, p.link, true), p) {
        if g.len() < p.min_points {
            continue;
        }
        let pts: Vec<[f64; 3]> = g.iter().map(|&i| wires[i]).collect();
        conductors.extend(fit(&pts, p));
    }
    drop(wires);
    if conductors.is_empty() {
        return Err(Error::Param(format!(
            "ningún grupo de puntos de cable llega a {} puntos: baja el mínimo o sube el radio de unión",
            p.min_points
        )));
    }
    // lo que el clasificador tomó por líneas en las copas es corto, y lo que tomó por postes es
    // bajo, ancho o está lejos de todo conductor
    let before = conductors.len();
    conductors.retain(|c| c.length() >= p.min_length);
    if conductors.is_empty() {
        return Err(Error::Param(format!(
            "ninguno de los {before} grupos de puntos de cable mide {} m o más: parecen ruido de la clasificación (copas de árboles); baja la longitud mínima si de verdad hay vanos más cortos",
            p.min_length
        )));
    }
    let discarded = before - conductors.len();
    let poles: Vec<Pole> = find_poles(&pole_pts, p)
        .into_iter()
        .filter(|q| q.top_z - q.base_z >= p.min_pole_height && q.width <= p.pole_max_width)
        .filter(|q| {
            conductors
                .iter()
                .any(|c| c.locate(q.x, q.y).is_some_and(|(_, d)| d <= p.pole_reach))
        })
        .collect();
    let spans = find_spans(&conductors, &poles, p);
    check_cancel()?;
    // pasada 2: la vegetación cerca de los conductores
    let index = Index::new(&conductors, p.half_width.max(5.0));
    let mut cells: HashMap<(i64, i64), Cell> = HashMap::new();
    let mut seen = 0u64;
    for input in &j.inputs {
        Reader::open(input)?.scan(|pt| {
            if pt.withheld || !p.vegetation.allows(pt.class) {
                return;
            }
            let Some(g) = dtm.sample(pt.x, pt.y) else {
                return;
            };
            let h = pt.z - f64::from(g);
            if h < p.min_height {
                return;
            }
            let Some((c, s, d_h, zc)) = index.nearest(&conductors, pt.x, pt.y, p.half_width) else {
                return;
            };
            seen += 1;
            let d3 = d_h.hypot(zc - pt.z);
            let key = (
                (pt.x / p.cell).floor() as i64,
                (pt.y / p.cell).floor() as i64,
            );
            let cell = cells.entry(key).or_insert(Cell {
                max_h: f64::NEG_INFINITY,
                top: [pt.x, pt.y, pt.z],
                d_h,
                wire_z: zc,
                d3_min: f64::INFINITY,
                conductor: c,
                along: s,
            });
            cell.d3_min = cell.d3_min.min(d3);
            if h > cell.max_h {
                *cell = Cell {
                    max_h: h,
                    top: [pt.x, pt.y, pt.z],
                    d_h,
                    wire_z: zc,
                    d3_min: cell.d3_min,
                    conductor: c,
                    along: s,
                };
            }
        })?;
        check_cancel()?;
    }
    let mut hazards = Vec::new();
    let mut keys: Vec<_> = cells.keys().copied().collect();
    keys.sort_unstable();
    let mut stats: Vec<SpanStats> = spans
        .iter()
        .map(|_| SpanStats {
            min_d3: f64::INFINITY,
            ..SpanStats::default()
        })
        .collect();
    for k in keys {
        let c = cells[&k];
        let invasion = c.d3_min < p.min_distance;
        let fall = c.max_h + p.fall_margin >= c.d_h;
        let sp = span_of(&spans, c.conductor, c.along);
        if let Some(st) = stats.get_mut(sp) {
            st.min_d3 = st.min_d3.min(c.d3_min);
            if invasion || fall {
                st.invasion += u32::from(invasion);
                st.fall += u32::from(fall);
                st.tallest = st.tallest.max(c.max_h);
            }
        }
        if invasion || fall {
            hazards.push(Hazard {
                x: c.top[0],
                y: c.top[1],
                z: c.top[2],
                height: c.max_h,
                horizontal: c.d_h,
                distance: c.d3_min,
                clearance: c.wire_z - c.top[2],
                invasion,
                fall,
                conductor: c.conductor,
                span: sp,
            });
        }
    }
    let span_stats = stats
        .iter()
        .map(|s| (s.min_d3, s.invasion, s.fall, s.tallest))
        .collect();
    Ok(Scene {
        conductors,
        poles,
        spans,
        hazards,
        span_stats,
        vegetation_points: seen,
        discarded,
    })
}

fn kind(h: &Hazard) -> &'static str {
    match (h.invasion, h.fall) {
        (true, true) => "invasion+caida",
        (true, false) => "invasion",
        _ => "caida",
    }
}

fn name(c: usize, s: &Span) -> String {
    format!("C{}-V{}", c + 1, s.index + 1)
}

/// Ejecuta todo el análisis y escribe los resultados.
pub fn run(j: &CorridorJob) -> Result<String> {
    let mut inputs: Vec<&PathBuf> = j.inputs.iter().collect();
    inputs.push(&j.dtm);
    let o = j.outputs();
    for out in [&o.conductors, &o.poles, &o.hazards]
        .map(Option::as_ref)
        .into_iter()
        .flatten()
        .chain([&o.spans, &o.hazards_table])
    {
        crate::paths::guard_output(&inputs, out)?;
    }
    let mut files = j.inputs.clone();
    files.push(j.dtm.clone());
    let unified = crate::crs::unify(&files, false)?;
    let notes = unified.warnings.clone();
    let moved = CorridorJob {
        inputs: unified.all(&j.inputs),
        dtm: unified.path(&j.dtm),
        ..j.clone()
    };
    let sc = analyse(&moved)?;
    let out = j.outputs();
    paths::ensure_parent(&j.output)?;
    let epsg = moved
        .inputs
        .first()
        .and_then(|p| Reader::open(p).ok())
        .and_then(|r| r.header.epsg);
    let prj = epsg.and_then(prj_for_epsg);
    let mut lines = vec![format!(
        "{} conductor(es), {} poste(s), {} vano(s); {} puntos de vegetación a menos de {} m de un conductor",
        sc.conductors.len(),
        sc.poles.len(),
        sc.spans.len(),
        sc.vegetation_points,
        j.params.half_width
    )];
    // spans
    let mut csv = std::io::BufWriter::new(std::fs::File::create(&out.spans)?);
    writeln!(
        csv,
        "vano,conductor,poste_a,poste_b,longitud_m,flecha_m,celdas_invasion,celdas_caida,distancia_min_m,altura_max_riesgo_m"
    )?;
    for (sp, st) in sc.spans.iter().zip(&sc.span_stats) {
        let pole = |k: Option<usize>| k.map_or(String::new(), |k| format!("P{}", k + 1));
        writeln!(
            csv,
            "{},C{},{},{},{:.1},{:.2},{},{},{},{}",
            name(sp.conductor, sp),
            sp.conductor + 1,
            pole(sp.pole_a),
            pole(sp.pole_b),
            sp.to - sp.from,
            sp.sag,
            st.1,
            st.2,
            if st.0.is_finite() {
                format!("{:.2}", st.0)
            } else {
                String::new()
            },
            if st.3 > 0.0 {
                format!("{:.1}", st.3)
            } else {
                String::new()
            },
        )?;
    }
    csv.flush()?;
    lines.push(format!("  vanos: {}", out.spans.display()));
    // hazards
    let mut csv = std::io::BufWriter::new(std::fs::File::create(&out.hazards_table)?);
    writeln!(
        csv,
        "id,riesgo,vano,x,y,z,altura_m,distancia_horizontal_m,distancia_min_m,holgura_vertical_m"
    )?;
    for (k, h) in sc.hazards.iter().enumerate() {
        writeln!(
            csv,
            "{},{},{},{:.2},{:.2},{:.2},{:.1},{:.1},{:.2},{:.2}",
            k + 1,
            kind(h),
            sc.spans
                .get(h.span)
                .map_or(String::new(), |s| name(h.conductor, s)),
            h.x,
            h.y,
            h.z,
            h.height,
            h.horizontal,
            h.distance,
            h.clearance
        )?;
    }
    csv.flush()?;
    let (inv, fall) = (
        sc.hazards.iter().filter(|h| h.invasion).count(),
        sc.hazards.iter().filter(|h| h.fall).count(),
    );
    lines.push(format!(
        "  puntos de riesgo: {} ({inv} por invasión a menos de {} m, {fall} por posible caída): {}",
        sc.hazards.len(),
        j.params.min_distance,
        out.hazards_table.display()
    ));
    // shapefiles
    if let Some(path) = &out.conductors {
        let table = Table {
            columns: vec![
                ("id".into(), ColKind::Num(8, 0)),
                ("puntos".into(), ColKind::Num(10, 0)),
                ("longitud".into(), ColKind::Num(12, 1)),
                ("huecos".into(), ColKind::Num(6, 0)),
                ("cortes".into(), ColKind::Num(6, 0)),
            ],
            rows: sc
                .conductors
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    vec![
                        Val::Int(k as i64 + 1),
                        Val::Int(c.points as i64),
                        Val::Num(c.length()),
                        Val::Int(c.gaps as i64),
                        Val::Int(c.breaks as i64),
                    ]
                })
                .collect(),
        };
        let lines3: Vec<Vec<[f64; 3]>> = sc.conductors.iter().map(|c| c.verts.clone()).collect();
        write_polylines(path, &lines3, true, &table, prj.as_deref())?;
        lines.push(format!("  conductores: {}", path.display()));
    }
    if let Some(path) = &out.poles {
        let table = Table {
            columns: vec![
                ("id".into(), ColKind::Num(8, 0)),
                ("altura".into(), ColKind::Num(8, 1)),
                ("puntos".into(), ColKind::Num(10, 0)),
            ],
            rows: sc
                .poles
                .iter()
                .enumerate()
                .map(|(k, p)| {
                    vec![
                        Val::Int(k as i64 + 1),
                        Val::Num(p.top_z - p.base_z),
                        Val::Int(p.points as i64),
                    ]
                })
                .collect(),
        };
        let pts: Vec<[f64; 3]> = sc.poles.iter().map(|p| [p.x, p.y, p.top_z]).collect();
        write_points(path, &pts, true, &table, prj.as_deref())?;
        lines.push(format!("  postes: {}", path.display()));
    }
    if let Some(path) = &out.hazards {
        let table = Table {
            columns: vec![
                ("id".into(), ColKind::Num(8, 0)),
                ("riesgo".into(), ColKind::Text(16)),
                ("vano".into(), ColKind::Text(12)),
                ("altura".into(), ColKind::Num(8, 1)),
                ("dist_h".into(), ColKind::Num(8, 1)),
                ("dist_min".into(), ColKind::Num(8, 2)),
                ("holgura".into(), ColKind::Num(8, 2)),
            ],
            rows: sc
                .hazards
                .iter()
                .enumerate()
                .map(|(k, h)| {
                    vec![
                        Val::Int(k as i64 + 1),
                        Val::Text(kind(h).to_string()),
                        Val::Text(
                            sc.spans
                                .get(h.span)
                                .map_or(String::new(), |s| name(h.conductor, s)),
                        ),
                        Val::Num(h.height),
                        Val::Num(h.horizontal),
                        Val::Num(h.distance),
                        Val::Num(h.clearance),
                    ]
                })
                .collect(),
        };
        let pts: Vec<[f64; 3]> = sc.hazards.iter().map(|h| [h.x, h.y, h.z]).collect();
        write_points(path, &pts, true, &table, prj.as_deref())?;
        lines.push(format!("  riesgo (puntos): {}", path.display()));
    }
    if sc.discarded > 0 {
        lines.push(format!(
            "  {} grupo(s) de puntos de cable descartados por medir menos de {} m",
            sc.discarded, j.params.min_length
        ));
    }
    let gaps: usize = sc.conductors.iter().map(|c| c.breaks).sum();
    if gaps > 0 {
        lines.push(format!(
            "  Aviso: {gaps} corte(s) de conductor por huecos de más de {} m sin puntos de cable; ahí no se ha medido",
            j.params.gap_max
        ));
    }
    Ok(lines.join("\n") + &crate::crs::notes(&notes))
}

/// Las rutas de las entradas que lee el trabajo.
pub fn inputs(j: &CorridorJob) -> Vec<&Path> {
    let mut v: Vec<&Path> = j.inputs.iter().map(PathBuf::as_path).collect();
    v.push(j.dtm.as_path());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::las::{Writer, WriterOptions};

    /// Un registro de formato de punto 0: posición (escalada por 0,001 desde el desplazamiento),
    /// clase.
    fn record(x: f64, y: f64, z: f64, class: u8) -> Vec<u8> {
        let mut r = vec![0u8; 20];
        for (k, v) in [x - 500_000.0, y - 4_000_000.0, z].iter().enumerate() {
            r[4 * k..4 * k + 4].copy_from_slice(&((v / 0.001).round() as i32).to_le_bytes());
        }
        r[14] = 0x09;
        r[15] = class;
        r
    }

    #[test]
    fn conductors_poles_and_hazards_are_found_in_a_synthetic_line() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_corridor_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let (x0, y0) = (500_000.0, 4_000_000.0);
        let mut recs: Vec<Vec<u8>> = Vec::new();
        // dos conductores a 4 m uno de otro, de x = 0 a 100 m, colgando 4 m a mitad de vano desde
        // 12 m
        let sag = |s: f64| 12.0 - 4.0 * (1.0 - ((s - 50.0) / 50.0).powi(2));
        for (k, dy) in [0.0, 4.0].iter().enumerate() {
            let mut s = 0.0;
            while s <= 100.0 {
                // un tramo ausente en medio del primer conductor
                if !(k == 0 && (40.0..46.0).contains(&s)) {
                    recs.push(record(
                        x0 + s,
                        y0 + dy,
                        sag(s) + 0.05 * ((s * 7.0).sin()),
                        WIRE,
                    ));
                }
                s += 0.5;
            }
        }
        // dos postes, entre los conductores, de 12 m de altura
        for px in [0.0, 100.0] {
            let mut z = 0.0;
            while z <= 12.0 {
                recs.push(record(x0 + px, y0 + 2.0, z, POLE));
                z += 0.5;
            }
        }
        // árboles: A bajo la línea (invasión), B alto y a 15 m al lado (caída), C bajo y lejos
        for (tx, ty, h) in [(50.0, 2.0, 6.5), (20.0, 15.0, 20.0), (80.0, 25.0, 10.0)] {
            let mut z = 2.0;
            while z <= h {
                recs.push(record(x0 + tx, y0 + ty, z, 5));
                z += 0.5;
            }
        }
        // una plantilla solo con cabecera y luego el archivo
        let tpl = crate::ply::template(&dir, 0, 0.001, [x0, y0, 0.0], Some(25830))?;
        let cloud = dir.join("linea.las");
        let mut w = Writer::create_with(&cloud, &tpl, &WriterOptions::default())?;
        for r in &recs {
            w.write(r)?;
        }
        w.finish()?;
        let mut dtm = Grid::new(x0 - 50.0, y0 - 50.0, 1.0, 250, 150)?;
        dtm.data.fill(0.0);
        dtm.write_tiff(&dir.join("mdt.tif"), Some(25830))?;
        let job = CorridorJob {
            inputs: vec![cloud],
            dtm: dir.join("mdt.tif"),
            output: dir.join("linea.shp"),
            params: CorridorParams::default(),
            formats: Formats {
                vector: true,
                csv: true,
            },
        };
        let sc = analyse(&job)?;
        // el tramo ausente se rellena, así que ambos son conductores enteros, de 100 m de largo
        assert_eq!(
            sc.conductors.len(),
            2,
            "{:?}",
            sc.conductors.iter().map(|c| c.length()).collect::<Vec<_>>()
        );
        for c in &sc.conductors {
            assert!((c.length() - 100.0).abs() < 4.0, "{}", c.length());
        }
        assert!(sc.conductors.iter().any(|c| c.gaps > 0));
        // el hueco rellenado sigue la flecha: en s = 43 el cable cuelga unos 12 - 4(1-(7/50)^2) =
        // 8,08
        let first = sc
            .conductors
            .iter()
            .find(|c| (c.verts[0][1] - y0).abs() < 1.0)
            .ok_or(Error::Cancelled)?;
        let v = first
            .verts
            .iter()
            .min_by(|a, b| {
                (a[0] - (x0 + 43.0))
                    .abs()
                    .total_cmp(&(b[0] - (x0 + 43.0)).abs())
            })
            .ok_or(Error::Cancelled)?;
        assert!((v[2] - 8.08).abs() < 0.4, "{}", v[2]);
        assert_eq!(sc.poles.len(), 2);
        assert!((sc.poles[0].top_z - sc.poles[0].base_z - 12.0).abs() < 0.6);
        assert_eq!(sc.spans.len(), 2);
        assert!(
            sc.spans
                .iter()
                .all(|s| s.pole_a.is_some() && s.pole_b.is_some()),
            "{:?}",
            sc.spans
        );
        assert!((sc.spans[0].sag - 4.0).abs() < 0.8, "{}", sc.spans[0].sag);
        // A es una invasión, B un peligro de caída, C nada
        assert_eq!(sc.hazards.len(), 2, "{:?}", sc.hazards);
        let a = sc
            .hazards
            .iter()
            .find(|h| h.invasion)
            .ok_or(Error::Cancelled)?;
        assert!((a.x - (x0 + 50.0)).abs() < 3.0 && a.distance < 3.0);
        let b = sc
            .hazards
            .iter()
            .find(|h| !h.invasion)
            .ok_or(Error::Cancelled)?;
        assert!(b.fall && (b.x - (x0 + 20.0)).abs() < 3.0);
        // el informe y los archivos
        let text = run(&job)?;
        assert!(
            text.contains("2 conductor(es), 2 poste(s), 2 vano(s)"),
            "{text}"
        );
        let out = job.outputs();
        let spans = std::fs::read_to_string(&out.spans)?;
        assert_eq!(spans.lines().count(), 3, "{spans}");
        assert!(out.conductors.as_ref().is_some_and(|p| p.exists()));
        assert!(out.hazards.as_ref().is_some_and(|p| p.exists()));
        // una nube sin cables lo dice
        let none = dir.join("sin.las");
        let mut w = Writer::create_with(&none, &tpl, &WriterOptions::default())?;
        w.write(&record(x0, y0, 1.0, 2))?;
        w.finish()?;
        let bad = CorridorJob {
            inputs: vec![none],
            ..job
        };
        assert!(analyse(&bad).is_err());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Los conductores más cercanos que la distancia de enlace acaban en un mismo grupo de puntos;
    /// aun así deben salir como un trazado cada uno, no como un trazado por el medio de ambos.
    #[test]
    fn parallel_conductors_in_one_cluster_are_told_apart() {
        let p = CorridorParams::default();
        let mut pts = Vec::new();
        let mut s = 0.0_f64;
        while s <= 60.0 {
            // a 2 m uno de otro a través y a 1 m de altura
            for (side, z) in [(0.0, 20.0), (2.0, 21.0)] {
                pts.push([s, side, z - 0.002 * (s - 30.0).powi(2)]);
            }
            s += 0.5;
        }
        let paths = fit(&pts, &p);
        assert_eq!(paths.len(), 2, "{paths:?}");
        let mut zs: Vec<f64> = paths
            .iter()
            .map(|c| c.verts.iter().map(|v| v[2]).sum::<f64>() / c.verts.len() as f64)
            .collect();
        zs.sort_by(f64::total_cmp);
        assert!((zs[1] - zs[0] - 1.0).abs() < 0.1, "{zs:?}");
        assert!(paths.iter().all(|c| c.length() > 50.0));
    }

    /// Un vuelo con pocas pasadas ve un conductor en trozos de un metro, a diez metros unos de
    /// otros: demasiado cortos para saber su dirección, son un conductor por estar en fila.
    #[test]
    fn pieces_of_a_metre_in_a_row_are_one_conductor() {
        let p = CorridorParams::default();
        let mut pts = Vec::new();
        for k in 0..12 {
            for i in 0..20 {
                pts.push([f64::from(k) * 10.0 + f64::from(i) * 0.05, f64::from(k) * 0.1, 30.0]);
            }
        }
        let groups = join_pieces(&pts, cluster(&pts, p.link, true), &p);
        assert_eq!(groups.len(), 1, "{}", groups.len());
        assert_eq!(groups[0].len(), pts.len());
        // un trozo fuera de la línea se queda solo
        pts.push([55.0, 40.0, 30.0]);
        pts.push([55.5, 40.0, 30.0]);
        let groups = join_pieces(&pts, cluster(&pts, p.link, true), &p);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn a_parabola_is_recovered_from_samples() {
        let samples: Vec<(f64, f64)> = [-6.0, -4.0, 4.0, 6.0]
            .iter()
            .map(|&t| (t, 3.0 - 0.1 * t + 0.02 * t * t))
            .collect();
        let c = parabola(&samples).unwrap_or_default();
        assert!(
            (c[0] - 3.0).abs() < 1e-9 && (c[1] + 0.1).abs() < 1e-9 && (c[2] - 0.02).abs() < 1e-9
        );
        // tres muestras en un punto no pueden decir nada
        assert!(parabola(&[(1.0, 1.0), (1.0, 1.0), (1.0, 1.0), (1.0, 1.0)]).is_none());
    }
}
