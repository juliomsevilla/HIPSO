//! Herramientas entre nubes y sobre puntos repetidos: puntos duplicados, detección de cambios entre
//! dos nubes, alineación de una nube con otra (punto más cercano iterativo) y diferencia de altura
//! entre pasadas. Todas leen las nubes como flujos; lo que guardan en memoria es una copia
//! adelgazada de la referencia, los puntos cambiados o un número por celda.

// El código de matrices pequeñas de tamaño fijo se lee mejor con índices.
#![allow(clippy::needless_range_loop)]

use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::{Path, PathBuf};

use crate::filter::ReturnKind;
use crate::grid::Grid;
use crate::las::{Reader, Writer, set_xyz};
use crate::{Error, Result};

#[derive(Default)]
struct Fx(u64);

impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(*b)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
        self.0 ^= self.0 >> 29;
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

fn pack(ix: i64, iy: i64, iz: i64) -> u64 {
    const M: u64 = 0x1F_FFFF;
    (((ix + (1 << 20)) as u64 & M) << 42)
        | (((iy + (1 << 20)) as u64 & M) << 21)
        | ((iz + (1 << 20)) as u64 & M)
}

// ----------------------------------------------------------------------------- duplicates

/// Qué atributos deben coincidir también para que dos puntos en el mismo lugar sean duplicados.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DupFields {
    pub class: bool,
    pub gps_time: bool,
    pub intensity: bool,
    pub return_number: bool,
    pub return_count: bool,
    pub source_id: bool,
}

fn dup_key(r: &Reader, rec: &[u8], f: DupFields) -> u64 {
    let mut h = Fx::default();
    h.write(rec.get(..12).unwrap_or(rec));
    let (p, e) = (r.decode(rec), r.decode_extra(rec));
    if let Some(p) = p {
        if f.class {
            h.write_u64(u64::from(p.class) | 1 << 40);
        }
        if f.intensity {
            h.write_u64(u64::from(p.intensity) | 2 << 40);
        }
        if f.return_number {
            h.write_u64(u64::from(p.return_number) | 3 << 40);
        }
        if f.return_count {
            h.write_u64(u64::from(p.number_of_returns) | 4 << 40);
        }
    }
    if f.gps_time {
        h.write_u64(e.gps_time.to_bits());
    }
    if f.source_id {
        h.write_u64(u64::from(e.source_id) | 5 << 40);
    }
    h.finish()
}

/// Cuántas pasadas hace [`remove_duplicates`] sobre las claves de `points` puntos: la tabla de
/// puntos distintos ocupa unos 40 bytes por punto (con el espacio que guarda una tabla hash) y
/// puede ocupar un tercio de la memoria del equipo. `HIPSO_DEDUP_PARTS` fuerza un número (para
/// probar).
fn dup_parts(points: u64) -> u64 {
    if let Some(k) = std::env::var("HIPSO_DEDUP_PARTS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return k.max(1);
    }
    let mem = crate::copc_cache::total_memory().unwrap_or(8 << 30);
    ((points as f64 * 40.0) / (mem as f64 / 3.0)).ceil().max(1.0) as u64
}

/// La parte de las claves (de `parts`) a la que pertenece una clave.
#[inline]
fn dup_part(key: u64, parts: u64) -> u64 {
    (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) % parts
}

/// Escribe `input` sin sus puntos duplicados: se conserva un punto de cada conjunto de duplicados,
/// o ninguno con `delete_all`. Devuelve (puntos leídos, puntos eliminados).
///
/// Memoria: una tabla de los puntos distintos de una parte de las claves y un bit por cada punto de
/// la nube. Cuando la tabla de todos los puntos no cabría en memoria, las claves se dividen en
/// partes, con una pasada sobre la nube por cada una, de modo que una nube de miles de millones de
/// puntos se limpia con la memoria de unos pocos millones.
pub fn remove_duplicates(
    input: &Path,
    output: &Path,
    fields: DupFields,
    delete_all: bool,
) -> Result<(u64, u64)> {
    if input == output {
        return Err(Error::Param(
            "el archivo nuevo no debe sobrescribir el original".into(),
        ));
    }
    let r = Reader::open(input)?.for_passes()?;
    let rec_len = usize::from(r.header.record_len);
    let parts = dup_part_count(&r);
    // un bit por cada punto: activado cuando el punto se va
    let mut gone: Vec<u64> = if parts > 1 {
        vec![0; (r.header.count as usize).div_ceil(64)]
    } else {
        Vec::new()
    };
    for part in 0..if parts > 1 { parts } else { 0 } {
        crate::check_cancel()?;
        let mut count: FxMap<u64, u8> = FxMap::default();
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let k = dup_key(&r, rec, fields);
                if dup_part(k, parts) == part {
                    let c = count.entry(k).or_insert(0);
                    *c = c.saturating_add(1);
                }
            }
            Ok(())
        })?;
        let mut i = 0usize;
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let k = dup_key(&r, rec, fields);
                if dup_part(k, parts) == part {
                    let keep = match count.get_mut(&k) {
                        Some(c) if delete_all => *c <= 1,
                        Some(c) if *c > 0 => {
                            *c = 0;
                            true
                        }
                        _ => false,
                    };
                    if !keep {
                        gone[i / 64] |= 1 << (i % 64);
                    }
                }
                i += 1;
            }
            Ok(())
        })?;
    }
    // el caso de una sola parte conserva su tabla para la pasada de escritura
    let mut count: FxMap<u64, u8> = FxMap::default();
    if parts == 1 {
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let c = count.entry(dup_key(&r, rec, fields)).or_insert(0);
                *c = c.saturating_add(1);
            }
            Ok(())
        })?;
    }
    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create(output, &r)?;
    let (mut read, mut removed) = (0u64, 0u64);
    let mut i = 0usize;
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            read += 1;
            let keep = if parts > 1 {
                gone[i / 64] & (1 << (i % 64)) == 0
            } else {
                match count.get_mut(&dup_key(&r, rec, fields)) {
                    Some(c) if delete_all => *c <= 1,
                    Some(c) if *c > 0 => {
                        *c = 0;
                        true
                    }
                    _ => false,
                }
            };
            i += 1;
            if keep {
                w.write(rec)?;
            } else {
                removed += 1;
            }
        }
        Ok(())
    })?;
    w.finish()?;
    Ok((read, removed))
}

fn dup_part_count(r: &Reader) -> u64 {
    dup_parts(r.header.count)
}

// ----------------------------------------------------------------------------- comparing

/// Puntos de referencia en una rejilla de celda `cell`, adelgazados a un tercio de ella, para
/// pruebas de punto más cercano.
struct RefGrid {
    origin: [f64; 3],
    cell: f64,
    cells: FxMap<u64, Vec<[f32; 3]>>,
    points: usize,
}

impl RefGrid {
    fn build(paths: &[PathBuf], cell: f64, voxel: f64) -> Result<Self> {
        let mut origin = [f64::MAX; 3];
        for p in paths {
            let b = Reader::open(p)?.header.bounds;
            for k in 0..3 {
                origin[k] = origin[k].min(b[k]);
            }
        }
        let v = voxel;
        let mut seen: FxMap<u64, ()> = FxMap::default();
        let mut cells: FxMap<u64, Vec<[f32; 3]>> = FxMap::default();
        let mut points = 0;
        for p in paths {
            let r = Reader::open(p)?;
            r.scan(|q| {
                if q.is_noise() {
                    return;
                }
                let (x, y, z) = (q.x - origin[0], q.y - origin[1], q.z - origin[2]);
                let key = pack(
                    (x / v).floor() as i64,
                    (y / v).floor() as i64,
                    (z / v).floor() as i64,
                );
                if seen.insert(key, ()).is_none() {
                    cells
                        .entry(pack(
                            (x / cell).floor() as i64,
                            (y / cell).floor() as i64,
                            (z / cell).floor() as i64,
                        ))
                        .or_default()
                        .push([x as f32, y as f32, z as f32]);
                    points += 1;
                }
            })?;
        }
        Ok(Self {
            origin,
            cell,
            cells,
            points,
        })
    }

    /// Distancia al punto de referencia más cercano si es menor que `reach` (como mucho la celda).
    fn nearest(&self, x: f64, y: f64, z: f64, reach: f64) -> Option<f64> {
        let (x, y, z) = (x - self.origin[0], y - self.origin[1], z - self.origin[2]);
        let (cx, cy, cz) = (
            (x / self.cell).floor() as i64,
            (y / self.cell).floor() as i64,
            (z / self.cell).floor() as i64,
        );
        let mut best = reach * reach;
        let mut found = false;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = self.cells.get(&pack(cx + dx, cy + dy, cz + dz)) {
                        for q in list {
                            let d = (f64::from(q[0]) - x).powi(2)
                                + (f64::from(q[1]) - y).powi(2)
                                + (f64::from(q[2]) - z).powi(2);
                            if d < best {
                                best = d;
                                found = true;
                            }
                        }
                    }
                }
            }
        }
        found.then(|| best.sqrt())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompareParams {
    /// Distancia 3D a partir de la cual un punto ha cambiado.
    pub min_dist: f64,
    /// Un punto cambiado necesita tantos otros puntos cambiados dentro del doble de la distancia.
    pub min_neighbors: usize,
}

impl Default for CompareParams {
    fn default() -> Self {
        Self {
            min_dist: 0.5,
            min_neighbors: 3,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareReport {
    pub points: u64,
    pub changed: u64,
    pub kept: u64,
    /// Distancia media y cuadrática de los puntos que no cambiaron.
    pub mean: f64,
    pub rms: f64,
    /// Proporción de los puntos con una contrapartida dentro de la distancia.
    pub completeness: f64,
}

/// Escribe en `output` los puntos de `source` que están más lejos que `min_dist` de todo punto de
/// `reference`: lo que cambió entre dos vuelos (o lo que quitó una edición).
pub fn compare_clouds(
    reference: &[PathBuf],
    source: &Path,
    output: &Path,
    p: &CompareParams,
) -> Result<CompareReport> {
    if !(p.min_dist > 0.0) {
        return Err(Error::Param("la distancia debe ser positiva".into()));
    }
    if reference.iter().any(|r| r == source) || source == output {
        return Err(Error::Param(
            "la nube de referencia y la que se comprueba deben ser distintas".into(),
        ));
    }
    let mut same_system: Vec<PathBuf> = reference.to_vec();
    same_system.push(source.to_path_buf());
    let unified = crate::crs::unify(&same_system, false)?;
    let reference = unified.all(reference);
    let reference = reference.as_slice();
    let source = &unified.path(source);
    let grid = RefGrid::build(reference, p.min_dist, p.min_dist / 3.0)?;
    if grid.points == 0 {
        return Err(Error::Param("la nube de referencia no tiene puntos".into()));
    }
    let r = Reader::open(source)?;
    let rec_len = usize::from(r.header.record_len);
    let mut rep = CompareReport::default();
    let (mut sum, mut sum2, mut near) = (0.0, 0.0, 0u64);
    let mut changed: Vec<([f64; 3], Vec<u8>)> = Vec::new();
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let Some(q) = r.decode(rec) else { continue };
            if q.is_noise() {
                continue;
            }
            rep.points += 1;
            match grid.nearest(q.x, q.y, q.z, p.min_dist) {
                Some(d) => {
                    near += 1;
                    sum += d;
                    sum2 += d * d;
                }
                None => changed.push(([q.x, q.y, q.z], rec.to_vec())),
            }
        }
        Ok(())
    })?;
    rep.changed = changed.len() as u64;
    if near > 0 {
        rep.mean = sum / near as f64;
        rep.rms = (sum2 / near as f64).sqrt();
    }
    rep.completeness = near as f64 / rep.points.max(1) as f64 * 100.0;
    // un punto cambiado solo es ruido: se conservan los que tienen compañía
    let reach = 2.0 * p.min_dist;
    let mut cells: FxMap<u64, Vec<u32>> = FxMap::default();
    for (i, (c, _)) in changed.iter().enumerate() {
        cells
            .entry(pack(
                (c[0] / reach).floor() as i64,
                (c[1] / reach).floor() as i64,
                (c[2] / reach).floor() as i64,
            ))
            .or_default()
            .push(i as u32);
    }
    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create(output, &r)?;
    for (i, (c, rec)) in changed.iter().enumerate() {
        let (cx, cy, cz) = (
            (c[0] / reach).floor() as i64,
            (c[1] / reach).floor() as i64,
            (c[2] / reach).floor() as i64,
        );
        let mut n = 0usize;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) {
                        for &o in list {
                            let q = &changed[o as usize].0;
                            if o as usize != i
                                && (q[0] - c[0]).powi(2)
                                    + (q[1] - c[1]).powi(2)
                                    + (q[2] - c[2]).powi(2)
                                    <= reach * reach
                            {
                                n += 1;
                                if n >= p.min_neighbors {
                                    break 'search;
                                }
                            }
                        }
                    }
                }
            }
        }
        if n >= p.min_neighbors {
            w.write(rec)?;
            rep.kept += 1;
        }
    }
    w.finish()?;
    Ok(rep)
}

pub fn compare_text(r: &CompareReport, out: &Path) -> String {
    format!(
        "{} puntos comparados: {} cambiados, {} conservados tras quitar los aislados; distancia media de los que coinciden {:.3} m (RMS {:.3} m); compleción {:.1} %: {}",
        r.points,
        r.changed,
        r.kept,
        r.mean,
        r.rms,
        r.completeness,
        out.display()
    )
}

// ----------------------------------------------------------------------------- aligning

#[derive(Clone, Debug, PartialEq)]
pub struct FitParams {
    /// Adelgazamiento 3D (m) de los puntos usados para hallar la transformación.
    pub thin: f64,
    /// Lo más lejos que puede estar un punto de referencia, en horizontal y en vertical, para
    /// emparejarse con un punto.
    pub max_xy: f64,
    pub max_z: f64,
    pub max_iter: usize,
    /// Se detiene cuando la distancia media es menor que esto.
    pub stop_dist: f64,
    /// Solo traslada (sin rotación).
    pub shift_only: bool,
    /// Minimiza la distancia al plano tangente del punto de referencia (punto a plano) en lugar de
    /// la distancia al propio punto: converge en menos iteraciones sobre superficies y no se atasca
    /// con una referencia dispersa.
    pub plane: bool,
    /// Proporción de los pares, los más cercanos, usados para hallar el movimiento (1 los conserva
    /// todos). Por debajo de 1 aparta los pares de suelo cambiado, árboles talados u objetos en
    /// movimiento.
    pub trim: f64,
}

impl Default for FitParams {
    fn default() -> Self {
        Self {
            thin: 1.0,
            max_xy: 5.0,
            max_z: 2.0,
            max_iter: 30,
            stop_dist: 0.01,
            shift_only: false,
            plane: false,
            trim: 1.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FitReport {
    pub iterations: usize,
    pub pairs: usize,
    pub before: f64,
    pub after: f64,
    pub shift: [f64; 3],
    /// Rotación alrededor de x, y y z, en grados.
    pub rotation: [f64; 3],
}

/// Resuelve el sistema simétrico `a x = b` (Gauss con pivoteo); `None` cuando es singular.
fn solve<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for c in 0..N {
        let p = (c..N).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..N {
            let f = a[r][c] / a[c][c];
            for k in c..N {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = [0.0; N];
    for c in (0..N).rev() {
        let s: f64 = (c + 1..N).map(|k| a[c][k] * x[k]).sum();
        x[c] = (b[c] - s) / a[c][c];
    }
    Some(x)
}

/// A rigid motion: p' = r p + t.
#[derive(Clone, Copy, Debug)]
struct Rigid {
    r: [[f64; 3]; 3],
    t: [f64; 3],
}

impl Rigid {
    const ID: Rigid = Rigid {
        r: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };

    fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let mut o = self.t;
        for i in 0..3 {
            for j in 0..3 {
                o[i] += self.r[i][j] * p[j];
            }
        }
        o
    }

    /// `self` después de `first`.
    fn after(&self, first: &Rigid) -> Rigid {
        let mut r = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                for k in 0..3 {
                    r[i][j] += self.r[i][k] * first.r[k][j];
                }
            }
        }
        let t = self.apply(first.t);
        Rigid { r, t }
    }

    /// La rotación para un giro pequeño `w` (radianes alrededor de x, y, z), por la fórmula de
    /// Rodrigues.
    fn small(w: [f64; 3], t: [f64; 3]) -> Rigid {
        let a = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        let mut r = Self::ID.r;
        if a > 1e-15 {
            let k = [w[0] / a, w[1] / a, w[2] / a];
            let (s, c) = a.sin_cos();
            let kx = [[0.0, -k[2], k[1]], [k[2], 0.0, -k[0]], [-k[1], k[0], 0.0]];
            for i in 0..3 {
                for j in 0..3 {
                    let id = if i == j { 1.0 } else { 0.0 };
                    let kk: f64 = (0..3).map(|m| kx[i][m] * kx[m][j]).sum();
                    r[i][j] = id + s * kx[i][j] + (1.0 - c) * kk;
                }
            }
        }
        Rigid { r, t }
    }
}

/// Normal unitaria del mejor plano por los vecinos de cada punto dentro de `radius`, o ceros donde
/// hay muy pocos vecinos o no son suficientemente planos para definir uno.
fn plane_normals(pts: &[[f64; 3]], radius: f64) -> Vec<[f64; 3]> {
    let mut cells: FxMap<u64, Vec<u32>> = FxMap::default();
    for (i, q) in pts.iter().enumerate() {
        cells
            .entry(pack(
                (q[0] / radius).floor() as i64,
                (q[1] / radius).floor() as i64,
                (q[2] / radius).floor() as i64,
            ))
            .or_default()
            .push(i as u32);
    }
    let r2 = radius * radius;
    pts.par_iter()
        .map(|q| {
            let (cx, cy, cz) = (
                (q[0] / radius).floor() as i64,
                (q[1] / radius).floor() as i64,
                (q[2] / radius).floor() as i64,
            );
            let mut near: Vec<[f64; 3]> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(list) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) else {
                            continue;
                        };
                        for &o in list {
                            let s = pts[o as usize];
                            let d = [s[0] - q[0], s[1] - q[1], s[2] - q[2]];
                            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= r2 {
                                near.push(d);
                            }
                        }
                    }
                }
            }
            plane_through(&near).unwrap_or([0.0; 3])
        })
        .collect()
}

/// Normal del plano de mínimos cuadrados por `d` (desplazamientos desde cualquier origen), si los
/// puntos son suficientemente planos: el menor autovalor de su covarianza como mucho una cuarta
/// parte del intermedio.
fn plane_through(d: &[[f64; 3]]) -> Option<[f64; 3]> {
    if d.len() < 6 {
        return None;
    }
    let n = d.len() as f64;
    let m = (0..3)
        .map(|k| d.iter().map(|v| v[k]).sum::<f64>() / n)
        .collect::<Vec<_>>();
    let mut c = [[0.0f64; 3]; 3];
    for v in d {
        let u = [v[0] - m[0], v[1] - m[1], v[2] - m[2]];
        for i in 0..3 {
            for j in 0..3 {
                c[i][j] += u[i] * u[j];
            }
        }
    }
    let (vals, vecs) = jacobi3(c);
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| vals[a].total_cmp(&vals[b]));
    let (lo, mid) = (vals[order[0]], vals[order[1]]);
    if mid <= 0.0 || lo > 0.25 * mid {
        return None;
    }
    let k = order[0];
    Some([vecs[0][k], vecs[1][k], vecs[2][k]])
}

/// Autovalores y autovectores (como columnas) de una matriz simétrica de 3×3, por rotaciones de
/// Jacobi.
fn jacobi3(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..32 {
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off < 1e-18 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}

/// Mueve `source` para que quede sobre `reference` (punto más cercano iterativo) y lo escribe en
/// `output`.
pub fn fit_cloud(
    reference: &[PathBuf],
    source: &Path,
    output: &Path,
    p: &FitParams,
) -> Result<FitReport> {
    if reference.iter().any(|r| r == source) || source == output {
        return Err(Error::Param(
            "la nube de referencia y la que se mueve deben ser distintas".into(),
        ));
    }
    let mut same_system: Vec<PathBuf> = reference.to_vec();
    same_system.push(source.to_path_buf());
    let unified = crate::crs::unify(&same_system, false)?;
    let reference = unified.all(reference);
    let reference = reference.as_slice();
    let source = &unified.path(source);
    if !(p.trim > 0.0 && p.trim <= 1.0) {
        return Err(Error::Param(
            "la parte de pares que se usa va de 0 (excluido) a 1".into(),
        ));
    }
    if !(p.thin > 0.0 && p.max_xy > 0.0 && p.max_z > 0.0) {
        return Err(Error::Param(
            "el aclarado y los límites deben ser positivos".into(),
        ));
    }
    let limit = p.max_xy.max(p.max_z);
    let grid = RefGrid::build(reference, limit, p.thin / 2.0)?;
    if grid.points == 0 {
        return Err(Error::Param("la nube de referencia no tiene puntos".into()));
    }
    let flat: Vec<[f64; 3]> = grid
        .cells
        .values()
        .flatten()
        .map(|s| {
            [
                f64::from(s[0]) + grid.origin[0],
                f64::from(s[1]) + grid.origin[1],
                f64::from(s[2]) + grid.origin[2],
            ]
        })
        .collect();
    drop(grid);
    let normals = if p.plane && !p.shift_only {
        plane_normals(&flat, 2.0 * p.thin)
    } else {
        Vec::new()
    };
    let r = Reader::open(source)?;
    // los puntos que buscan su pareja: últimos retornos y retornos únicos, uno por celda 3D
    let mut seen: FxMap<u64, ()> = FxMap::default();
    let mut sample: Vec<[f64; 3]> = Vec::new();
    r.scan(|q| {
        if q.is_noise() || !matches!(ReturnKind::of(q), ReturnKind::Last | ReturnKind::Single) {
            return;
        }
        let k = pack(
            (q.x / p.thin).floor() as i64,
            (q.y / p.thin).floor() as i64,
            (q.z / p.thin).floor() as i64,
        );
        if seen.insert(k, ()).is_none() {
            sample.push([q.x, q.y, q.z]);
        }
    })?;
    drop(seen);
    if sample.len() < 10 {
        return Err(Error::Param("hay muy pocos puntos para ajustar".into()));
    }
    let c0 = sample
        .iter()
        .fold([0.0; 3], |a, q| [a[0] + q[0], a[1] + q[1], a[2] + q[2]])
        .map(|v| v / sample.len() as f64);
    let mut m = Rigid::ID;
    let mut rep = FitReport::default();
    let mut reach = limit;
    for it in 0..p.max_iter.max(1) {
        crate::check_cancel()?;
        // la búsqueda se estrecha a medida que las nubes se juntan: una rejilla de celdas tan
        // anchas como el alcance
        let mut cells: FxMap<u64, Vec<u32>> = FxMap::default();
        for (i, q) in flat.iter().enumerate() {
            cells
                .entry(pack(
                    (q[0] / reach).floor() as i64,
                    (q[1] / reach).floor() as i64,
                    (q[2] / reach).floor() as i64,
                ))
                .or_default()
                .push(i as u32);
        }
        let (lim_xy, lim_z) = (p.max_xy.min(reach), p.max_z.min(reach));
        let moved: Vec<[f64; 3]> = sample.iter().map(|q| m.apply(*q)).collect();
        let mut pairs: Vec<([f64; 3], [f64; 3], u32)> = moved
            .par_iter()
            .filter_map(|q| {
                let (cx, cy, cz) = (
                    (q[0] / reach).floor() as i64,
                    (q[1] / reach).floor() as i64,
                    (q[2] / reach).floor() as i64,
                );
                let mut best: Option<(f64, [f64; 3], u32)> = None;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let Some(list) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) else {
                                continue;
                            };
                            for &o in list {
                                let s = flat[o as usize];
                                let (ex, ey, ez) = (s[0] - q[0], s[1] - q[1], s[2] - q[2]);
                                if ex.hypot(ey) > lim_xy || ez.abs() > lim_z {
                                    continue;
                                }
                                let d = ex * ex + ey * ey + ez * ez;
                                if best.is_none_or(|b| d < b.0) {
                                    best = Some((d, s, o));
                                }
                            }
                        }
                    }
                }
                best.map(|b| (*q, b.1, b.2))
            })
            .collect();
        if pairs.len() < 10 {
            return Err(Error::Param(
                "las nubes no se solapan dentro de los límites: amplía los límites horizontal y vertical".into(),
            ));
        }
        if p.trim < 1.0 {
            let dist = |(a, b, _): &([f64; 3], [f64; 3], u32)| {
                (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
            };
            pairs.sort_by(|x, y| dist(x).total_cmp(&dist(y)));
            let keep = ((pairs.len() as f64 * p.trim).ceil() as usize).clamp(10, pairs.len());
            pairs.truncate(keep);
        }
        let mean = pairs
            .iter()
            .map(|(a, b, _)| {
                ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
            })
            .sum::<f64>()
            / pairs.len() as f64;
        if it == 0 {
            rep.before = mean;
        }
        rep.after = mean;
        rep.pairs = pairs.len();
        rep.iterations = it;
        if mean <= p.stop_dist {
            break;
        }
        reach = (3.0 * mean).clamp(p.thin * 1.5, limit);
        // mínimos cuadrados para el pequeño movimiento que lleva cada punto a su pareja
        let step = if p.shift_only {
            let n = pairs.len() as f64;
            let t = (0..3)
                .map(|k| pairs.iter().map(|(a, b, _)| b[k] - a[k]).sum::<f64>() / n)
                .collect::<Vec<_>>();
            Rigid::small([0.0; 3], [t[0], t[1], t[2]])
        } else {
            let mut ata = [[0.0; 6]; 6];
            let mut atb = [0.0; 6];
            for (a, b, o) in &pairs {
                let (x, y, z) = (a[0] - c0[0], a[1] - c0[1], a[2] - c0[2]);
                let rows: Vec<([f64; 6], f64)> = if p.plane {
                    // distancia solo a lo largo de la normal del punto de referencia
                    let nv = normals[*o as usize];
                    if nv == [0.0; 3] {
                        continue;
                    }
                    let cross = [
                        y * nv[2] - z * nv[1],
                        z * nv[0] - x * nv[2],
                        x * nv[1] - y * nv[0],
                    ];
                    let rhs = nv[0] * (b[0] - a[0]) + nv[1] * (b[1] - a[1]) + nv[2] * (b[2] - a[2]);
                    vec![([cross[0], cross[1], cross[2], nv[0], nv[1], nv[2]], rhs)]
                } else {
                    vec![
                        ([0.0, z, -y, 1.0, 0.0, 0.0], b[0] - a[0]),
                        ([-z, 0.0, x, 0.0, 1.0, 0.0], b[1] - a[1]),
                        ([y, -x, 0.0, 0.0, 0.0, 1.0], b[2] - a[2]),
                    ]
                };
                for (row, rhs) in rows {
                    for i in 0..6 {
                        atb[i] += row[i] * rhs;
                        for j in 0..6 {
                            ata[i][j] += row[i] * row[j];
                        }
                    }
                }
            }
            if p.plane {
                // una superficie plana deja libres algunos movimientos: una ligera cresta los
                // mantiene en reposo
                let ridge = 1e-9 * (0..6).map(|i| ata[i][i]).sum::<f64>().max(1e-12);
                for (i, row) in ata.iter_mut().enumerate() {
                    row[i] += ridge;
                }
            }
            let Some(x) = solve(ata, atb) else { break };
            // giro alrededor del centro de la muestra y luego traslación
            let rot = Rigid::small([x[0], x[1], x[2]], [0.0; 3]);
            let back = Rigid {
                r: Rigid::ID.r,
                t: c0,
            };
            let centre_out = Rigid {
                r: Rigid::ID.r,
                t: [-c0[0], -c0[1], -c0[2]],
            };
            let turn = back.after(&rot).after(&centre_out);
            Rigid {
                r: turn.r,
                t: [turn.t[0] + x[3], turn.t[1] + x[4], turn.t[2] + x[5]],
            }
        };
        let shift = step.apply(c0);
        let small = (0..3).all(|k| (shift[k] - c0[k]).abs() < 1e-4);
        m = step.after(&m);
        if small {
            break;
        }
    }
    // toda la nube sigue el movimiento
    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create(output, &r)?;
    let rec_len = usize::from(r.header.record_len);
    let header = r.header.clone();
    let mut buf = vec![0u8; rec_len];
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            buf.copy_from_slice(rec);
            if let Some(q) = r.decode(rec) {
                let o = m.apply([q.x, q.y, q.z]);
                if !set_xyz(&header, &mut buf, o[0], o[1], o[2]) {
                    return Err(Error::Param(
                        "un punto movido no cabe en la escala de coordenadas del archivo".into(),
                    ));
                }
            }
            w.write(&buf)?;
        }
        Ok(())
    })?;
    w.finish()?;
    // el desplazamiento del centro de la nube
    let mid = m.apply(c0);
    rep.shift = [mid[0] - c0[0], mid[1] - c0[1], mid[2] - c0[2]];
    // ángulos de la rotación final (orden x-y-z)
    let rr = m.r;
    rep.rotation = [
        rr[2][1].atan2(rr[2][2]).to_degrees(),
        (-rr[2][0]).asin().to_degrees(),
        rr[1][0].atan2(rr[0][0]).to_degrees(),
    ];
    Ok(rep)
}

pub fn fit_text(r: &FitReport, out: &Path) -> String {
    format!(
        "{} iteraciones, {} pares; distancia media {:.3} m → {:.3} m; desplazamiento ({:.3}, {:.3}, {:.3}) m, giro ({:.4}°, {:.4}°, {:.4}°): {}",
        r.iterations,
        r.pairs,
        r.before,
        r.after,
        r.shift[0],
        r.shift[1],
        r.shift[2],
        r.rotation[0],
        r.rotation[1],
        r.rotation[2],
        out.display()
    )
}

// ----------------------------------------------------------------------------- swaths

/// Qué retornos cuentan en la separación entre pasadas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwathReturns {
    All,
    Last,
    Single,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwathReport {
    pub cells: usize,
    pub overlap_cells: usize,
    /// Diferencia absoluta media y cuadrática de las diferencias entre pasadas, en m.
    pub mean_abs: f64,
    pub rms: f64,
    pub max: f64,
    /// Proporción de las celdas de solape cuya diferencia supera 8 cm y 16 cm.
    pub over_8: f64,
    pub over_16: f64,
}

/// Diferencia de altura entre pasadas: por celda, la mayor diferencia entre las elevaciones medias
/// de las pasadas (identificadores de fuente) que tienen puntos en ella. Escribe un GeoTIFF de la
/// diferencia y devuelve las cifras del solape.
pub fn swath_separation(
    inputs: &[PathBuf],
    output: &Path,
    cell: f64,
    returns: SwathReturns,
    each_cloud_a_swath: bool,
) -> Result<SwathReport> {
    let by = if each_cloud_a_swath {
        SwathBy::Cloud
    } else {
        SwathBy::SourceId
    };
    swath_separation_by(inputs, output, cell, returns, by)
}

/// Qué distingue una pasada de otra.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwathBy {
    /// El identificador de fuente del punto de los puntos.
    SourceId,
    /// Cada nube es una pasada.
    Cloud,
    /// Las pasadas halladas a partir de los saltos del tiempo GPS (segundos), cuando las pasadas no
    /// se han escrito en el identificador de fuente.
    GpsGap(f64),
}

/// [`swath_separation`] con la manera de distinguir las pasadas ya elegida.
pub fn swath_separation_by(
    inputs: &[PathBuf],
    output: &Path,
    cell: f64,
    returns: SwathReturns,
    by: SwathBy,
) -> Result<SwathReport> {
    if !(cell > 0.0) {
        return Err(Error::Param("la celda debe ser positiva".into()));
    }
    let lines = match by {
        SwathBy::GpsGap(gap) => Some(
            crate::flightlines::detect(
                inputs,
                &crate::flightlines::FlightParams {
                    gap,
                    min_points: 1,
                    ..Default::default()
                },
            )?
            .index,
        ),
        _ => None,
    };
    let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let readers: Vec<Reader> = inputs
        .iter()
        .map(|p| Reader::open(p))
        .collect::<Result<_>>()?;
    for r in &readers {
        let b = r.header.bounds;
        bounds = [
            bounds[0].min(b[0]),
            bounds[1].min(b[1]),
            bounds[2].max(b[3]),
            bounds[3].max(b[4]),
        ];
    }
    let mut g = Grid::covering(bounds, cell)?;
    let mut acc: FxMap<u32, Vec<(u32, f64, u32)>> = FxMap::default();
    for (k, r) in readers.iter().enumerate() {
        let rec_len = usize::from(r.header.record_len);
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let Some(q) = r.decode(rec) else { continue };
                let kind = ReturnKind::of(&q);
                let ok = match returns {
                    SwathReturns::All => true,
                    SwathReturns::Last => matches!(kind, ReturnKind::Last | ReturnKind::Single),
                    SwathReturns::Single => kind == ReturnKind::Single,
                };
                if q.is_noise() || !ok {
                    continue;
                }
                let Some(i) = g.index(q.x, q.y) else { continue };
                let id = match (&lines, by) {
                    (Some(idx), _) => idx.line_at(r.decode_extra(rec).gps_time),
                    (None, SwathBy::Cloud) => k as u32,
                    _ => u32::from(r.decode_extra(rec).source_id),
                };
                let list = acc.entry(i as u32).or_default();
                match list.iter_mut().find(|e| e.0 == id) {
                    Some(e) => {
                        e.1 += q.z;
                        e.2 += 1;
                    }
                    None => list.push((id, q.z, 1)),
                }
            }
            Ok(())
        })?;
    }
    let mut rep = SwathReport::default();
    let (mut sum, mut sum2) = (0.0, 0.0);
    let (mut o8, mut o16) = (0usize, 0usize);
    g.data.fill(f32::NAN);
    rep.cells = acc.len();
    for (i, list) in &acc {
        if list.len() < 2 {
            continue;
        }
        let means: Vec<f64> = list.iter().map(|e| e.1 / f64::from(e.2)).collect();
        let (lo, hi) = means
            .iter()
            .fold((f64::MAX, f64::MIN), |a, m| (a.0.min(*m), a.1.max(*m)));
        let d = hi - lo;
        g.data[*i as usize] = d as f32;
        rep.overlap_cells += 1;
        sum += d;
        sum2 += d * d;
        rep.max = rep.max.max(d);
        o8 += usize::from(d > 0.08);
        o16 += usize::from(d > 0.16);
    }
    if rep.overlap_cells > 0 {
        let n = rep.overlap_cells as f64;
        rep.mean_abs = sum / n;
        rep.rms = (sum2 / n).sqrt();
        rep.over_8 = o8 as f64 / n * 100.0;
        rep.over_16 = o16 as f64 / n * 100.0;
    }
    crate::paths::ensure_parent(output)?;
    g.write_tiff(output, readers.first().and_then(|r| r.header.epsg))?;
    Ok(rep)
}

pub fn swath_text(r: &SwathReport, out: &Path) -> String {
    if r.overlap_cells == 0 {
        return format!(
            "sin solapes entre pasadas en {} celdas: {}",
            r.cells,
            out.display()
        );
    }
    format!(
        "{} celdas de solape: diferencia media {:.3} m, RMS {:.3} m, máxima {:.3} m; {:.1} % pasa de 8 cm y {:.1} % de 16 cm: {}",
        r.overlap_cells,
        r.mean_abs,
        r.rms,
        r.max,
        r.over_8,
        r.over_16,
        out.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_small_turn_matches_rotation() {
        let m = Rigid::small([0.0, 0.0, std::f64::consts::FRAC_PI_2], [0.0; 3]);
        let p = m.apply([1.0, 0.0, 0.0]);
        assert!((p[0]).abs() < 1e-12 && (p[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn linear_system_solves() {
        let x = solve([[2.0, 1.0], [1.0, 3.0]], [5.0, 10.0]);
        assert!(x.is_some_and(|x| (x[0] - 1.0).abs() < 1e-12 && (x[1] - 3.0).abs() < 1e-12));
    }
}
