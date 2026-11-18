//! Varita mágica (prueba de viabilidad): crece una región desde un punto semilla hasta los vecinos
//! conectados que se le parecen.
//!
//! Hay dos modos, elegidos por la forma del entorno de la semilla (planaridad, Weinmann et al. 2015):
//!
//! * superficie (semilla en un plano: suelo, tejado, calzada): un vecino entra si su plano local
//!   se desvía poco del plano del punto del que se llega, de modo que la región sigue una superficie
//!   lisa o curva y se detiene en un escalón o en una arista (cumbrera, borde de acera);
//! * volumen (semilla en vegetación o cables): entra todo vecino de la misma clase a distancia de
//!   enlace.
//!
//! En ambos se pueden exigir la misma clase y una intensidad parecida a la de la semilla.

use crate::features::smallest_direction;
use crate::las::Point;
use crate::tls::eigen;
use crate::view::PointStore;
use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

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

/// Cómo se elige entre seguir una superficie y llenar un volumen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WandMode {
    /// Por la planaridad del entorno de la semilla.
    #[default]
    Auto,
    Surface,
    Volume,
}

/// Qué tiene que cumplir un vecino para entrar en la región.
#[derive(Clone, Copy, Debug)]
pub struct WandParams {
    pub mode: WandMode,
    /// Distancia máxima (m) entre dos puntos enlazados.
    pub link: f64,
    /// Radio (m) del entorno con el que se estima el plano local de un punto.
    pub normal_radius: f64,
    /// Mayor ángulo (grados) entre el plano de un punto y el del vecino que entra (modo superficie).
    pub angle_tol: f64,
    /// Mayor ángulo (grados) entre el plano de un punto y el de la semilla: sin él, una curva suave
    /// o un cambio de plano repartido en varios pasos pequeños se cuela.
    pub drift_tol: f64,
    /// Planaridad desde la que un entorno se considera un plano (decide el modo y filtra vecinos).
    pub plane_min: f64,
    /// Salto máximo de altura (m) entre dos puntos enlazados en modo superficie.
    pub max_step: f64,
    /// Diferencia máxima de intensidad con la semilla (`u16::MAX` la ignora).
    pub intensity_tol: u16,
    /// Exige la misma clase que la semilla.
    pub same_class: bool,
    /// Diferencia máxima de altura (m) entre cualquier punto de la región y la semilla.
    pub max_dz: f64,
    /// Radio máximo (m) en planta alrededor de la semilla: acota el coste en nubes enormes.
    pub max_radius: f64,
}

impl Default for WandParams {
    fn default() -> Self {
        Self {
            mode: WandMode::Auto,
            link: 1.0,
            normal_radius: 1.0,
            angle_tol: 15.0,
            drift_tol: 25.0,
            plane_min: 0.5,
            max_step: 0.3,
            intensity_tol: u16::MAX,
            same_class: true,
            max_dz: 1000.0,
            max_radius: 100.0,
        }
    }
}

/// Rejilla 3D de celdas de lado `cell` con los puntos ordenados por celda: los de una celda están
/// contiguos en `order`, de modo que recorrerla es leer memoria seguida.
pub struct Index {
    cell: f64,
    order: Vec<u32>,
    cells: FxMap<u64, (u32, u32)>,
}

fn cell_of(cell: f64, x: f64, y: f64, z: f64) -> (i64, i64, i64) {
    (
        (x / cell).floor() as i64,
        (y / cell).floor() as i64,
        (z / cell).floor() as i64,
    )
}

fn pack((x, y, z): (i64, i64, i64)) -> u64 {
    const M: u64 = 0x1F_FFFF;
    ((x as u64 & M) << 42) | ((y as u64 & M) << 21) | (z as u64 & M)
}

impl Index {
    /// `cell` debe ser al menos el mayor de `link` y `normal_radius`.
    pub fn new(pts: &[Point], cell: f64) -> Self {
        let keys: Vec<u64> = pts
            .par_iter()
            .map(|p| pack(cell_of(cell, p.x, p.y, p.z)))
            .collect();
        let mut order: Vec<u32> = (0..pts.len() as u32).collect();
        order.par_sort_unstable_by_key(|&i| keys[i as usize]);
        let mut cells: FxMap<u64, (u32, u32)> = FxMap::default();
        let mut start = 0;
        for k in 1..=order.len() {
            if k == order.len() || keys[order[k] as usize] != keys[order[start] as usize] {
                cells.insert(keys[order[start] as usize], (start as u32, (k - start) as u32));
                start = k;
            }
        }
        Self { cell, order, cells }
    }

    /// Llama a `f` con cada punto a distancia de `q` como mucho `r` (`r` ≤ lado de celda).
    fn around(&self, pts: &[Point], q: &Point, r: f64, mut f: impl FnMut(usize, f64)) {
        let (cx, cy, cz) = cell_of(self.cell, q.x, q.y, q.z);
        let r2 = r * r;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(&(s, n)) = self.cells.get(&pack((cx + dx, cy + dy, cz + dz))) else {
                        continue;
                    };
                    for &j in &self.order[s as usize..(s + n) as usize] {
                        let o = &pts[j as usize];
                        let d2 = (o.x - q.x).powi(2) + (o.y - q.y).powi(2) + (o.z - q.z).powi(2);
                        if d2 <= r2 {
                            f(j as usize, d2);
                        }
                    }
                }
            }
        }
    }

    /// Plano local de un punto: normal (sentido arriba) y planaridad; `None` si hay muy pocos vecinos.
    fn local(&self, pts: &[Point], i: usize, r: f64) -> Option<([f64; 3], f64)> {
        let q = &pts[i];
        let mut nb: Vec<[f64; 3]> = Vec::with_capacity(64);
        self.around(pts, q, r, |j, _| {
            let o = &pts[j];
            nb.push([o.x - q.x, o.y - q.y, o.z - q.z]);
        });
        let n = nb.len();
        if n < 4 {
            return None;
        }
        let inv = 1.0 / n as f64;
        let mut mean = [0.0; 3];
        for p in &nb {
            for k in 0..3 {
                mean[k] += p[k] * inv;
            }
        }
        let mut c = [[0.0; 3]; 3];
        for p in &nb {
            let d = [p[0] - mean[0], p[1] - mean[1], p[2] - mean[2]];
            for a in 0..3 {
                for b in a..3 {
                    c[a][b] += d[a] * d[b] * inv;
                }
            }
        }
        c[1][0] = c[0][1];
        c[2][0] = c[0][2];
        c[2][1] = c[1][2];
        let (vals, _) = eigen(c);
        let [l1, l2, l3] = [vals[0].max(0.0), vals[1].max(0.0), vals[2].max(0.0)];
        if l1 <= 1e-12 {
            return None;
        }
        let mut nrm = smallest_direction(c, l3);
        if nrm[2] < 0.0 {
            nrm = [-nrm[0], -nrm[1], -nrm[2]];
        }
        Some((nrm, (l2 - l3) / l1))
    }
}

/// Plano local ya calculado de un punto: normal cuantizada y planaridad (0-255); `[0; 4]` sin
/// calcular, planaridad 0 sin entorno suficiente.
type Cached = [i8; 4];

fn quant(l: Option<([f64; 3], f64)>) -> Cached {
    match l {
        Some((n, pl)) => [
            (n[0] * 127.0).round() as i8,
            (n[1] * 127.0).round() as i8,
            (n[2] * 127.0).round() as i8,
            (pl * 100.0).round().clamp(1.0, 100.0) as i8,
        ],
        None => [0, 0, 0, -1],
    }
}

/// Índices de los puntos de la región que crece desde `seed`, en orden de hallazgo, y si trabajó
/// en modo superficie.
pub fn grow(pts: &[Point], idx: &Index, seed: usize, prm: &WandParams) -> (Vec<u32>, bool) {
    let Some(s) = pts.get(seed) else {
        return (Vec::new(), false);
    };
    let mut cache: Vec<Cached> = vec![[0; 4]; pts.len()];
    let get = |i: usize, cache: &mut Vec<Cached>| -> Option<([f64; 3], f64)> {
        if cache[i][3] == 0 {
            cache[i] = quant(idx.local(pts, i, prm.normal_radius));
        }
        let c = cache[i];
        (c[3] > 0).then(|| {
            (
                [f64::from(c[0]) / 127.0, f64::from(c[1]) / 127.0, f64::from(c[2]) / 127.0],
                f64::from(c[3]) / 100.0,
            )
        })
    };
    let surface = match prm.mode {
        WandMode::Auto => get(seed, &mut cache).is_some_and(|(_, pl)| pl >= prm.plane_min),
        WandMode::Surface => true,
        WandMode::Volume => false,
    };
    let cos_tol = prm.angle_tol.to_radians().cos();
    let cos_drift = prm.drift_tol.to_radians().cos();
    let n_seed = get(seed, &mut cache).map(|(n, _)| n).unwrap_or([0.0, 0.0, 1.0]);
    let mut seen = vec![false; pts.len()];
    seen[seed] = true;
    let mut out = vec![seed as u32];
    let mut head = 0;
    let mut batch: Vec<usize> = Vec::new();
    while head < out.len() {
        // Un nivel de la búsqueda: sus candidatos se recogen primero y los planos locales que
        // falten se calculan en paralelo, que es lo que más cuesta.
        let level = out.len();
        let mut found: Vec<(usize, usize)> = Vec::new();
        for &qi in &out[head..level] {
            let qi = qi as usize;
            let q = &pts[qi];
            idx.around(pts, q, prm.link, |o, _| {
                if !seen[o] {
                    let w = &pts[o];
                    if (!prm.same_class || w.class == s.class)
                        && w.intensity.abs_diff(s.intensity) <= prm.intensity_tol
                        && (w.x - s.x).hypot(w.y - s.y) <= prm.max_radius
                        && (w.z - s.z).abs() <= prm.max_dz
                        && (!surface || (w.z - q.z).abs() <= prm.max_step)
                    {
                        found.push((qi, o));
                    }
                }
            });
        }
        if surface {
            batch.clear();
            for &(qi, o) in &found {
                for k in [qi, o] {
                    if cache[k][3] == 0 {
                        batch.push(k);
                    }
                }
            }
            batch.sort_unstable();
            batch.dedup();
            let r = prm.normal_radius;
            let calc: Vec<Cached> = if batch.len() > 256 {
                batch.par_iter().map(|&k| quant(idx.local(pts, k, r))).collect()
            } else {
                batch.iter().map(|&k| quant(idx.local(pts, k, r))).collect()
            };
            for (&k, c) in batch.iter().zip(calc) {
                cache[k] = c;
            }
        }
        for (qi, o) in found {
            if seen[o] {
                continue;
            }
            let ok = if surface {
                match (get(qi, &mut cache), get(o, &mut cache)) {
                    (Some((a, _)), Some((b, pl))) => {
                        pl >= prm.plane_min * 0.6
                            && (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).abs() >= cos_tol
                            && (n_seed[0] * b[0] + n_seed[1] * b[1] + n_seed[2] * b[2]).abs()
                                >= cos_drift
                    }
                    _ => false,
                }
            } else {
                true
            };
            if ok {
                seen[o] = true;
                out.push(o as u32);
            }
        }
        head = level;
    }
    (out, surface)
}

/// La varita sobre un almacén de puntos: toma solo los puntos que `keep` deja pasar y caen a menos de
/// `max_radius` (más el margen del plano local) de la semilla en planta, de modo que el coste no
/// depende del tamaño de la nube. Da los índices del almacén y si trabajó en modo superficie.
pub fn grow_in_store(
    s: &PointStore,
    keep: &(dyn Fn(usize) -> bool + Sync),
    seed: usize,
    prm: &WandParams,
) -> (Vec<usize>, bool) {
    let c = s.xyz(seed);
    let reach = prm.max_radius + prm.link + prm.normal_radius;
    let ids: Vec<usize> = (0..s.len())
        .into_par_iter()
        .filter(|&i| {
            let p = s.xyz(i);
            (p[0] - c[0]).hypot(p[1] - c[1]) <= reach && (i == seed || keep(i))
        })
        .collect();
    let Some(local_seed) = ids.iter().position(|&i| i == seed) else {
        return (Vec::new(), false);
    };
    let pts: Vec<Point> = ids
        .iter()
        .map(|&i| {
            let p = s.xyz(i);
            Point {
                x: p[0],
                y: p[1],
                z: p[2],
                intensity: s.intensity[i],
                return_number: s.ret[i] & 0x0F,
                number_of_returns: s.ret[i] >> 4,
                class: s.class[i],
                withheld: false,
                scan_angle: f32::from(s.scan[i]) / 10.0,
                source_id: s.src[i],
            }
        })
        .collect();
    let idx = Index::new(&pts, prm.link.max(prm.normal_radius));
    let (found, surface) = grow(&pts, &idx, local_seed, prm);
    (found.into_iter().map(|j| ids[j as usize]).collect(), surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64, class: u8) -> Point {
        Point {
            x,
            y,
            z,
            intensity: 100,
            return_number: 1,
            number_of_returns: 1,
            class,
            withheld: false,
            scan_angle: 0.0,
            source_id: 0,
        }
    }

    fn near(pts: &[Point], x: f64, y: f64) -> usize {
        pts.iter()
            .enumerate()
            .min_by(|a, b| {
                let da = (a.1.x - x).hypot(a.1.y - y);
                let db = (b.1.x - x).hypot(b.1.y - y);
                da.total_cmp(&db).then(b.1.z.total_cmp(&a.1.z))
            })
            .map_or(0, |(i, _)| i)
    }

    /// Suelo plano con un edificio de 10x10 m a z=6: clic en el tejado selecciona el tejado y solo
    /// el tejado; clic en el suelo, el suelo.
    #[test]
    fn roof_stays_apart_from_ground() {
        let mut pts = Vec::new();
        for i in 0..60 {
            for j in 0..60 {
                let (x, y) = (f64::from(i) * 0.5, f64::from(j) * 0.5);
                let roof = (10.0..20.0).contains(&x) && (10.0..20.0).contains(&y);
                pts.push(pt(x, y, if roof { 6.0 } else { 0.0 }, 2));
            }
        }
        let idx = Index::new(&pts, 1.0);
        let prm = WandParams::default();
        let (roof, _) = grow(&pts, &idx, near(&pts, 15.2, 15.2), &prm);
        assert!(roof.iter().all(|&i| pts[i as usize].z > 5.0));
        assert!(roof.len() > 300, "{}", roof.len());
        let (ground, _) = grow(&pts, &idx, 0, &prm);
        assert!(ground.iter().all(|&i| pts[i as usize].z < 1.0));
        assert!(ground.len() > 3000, "{}", ground.len());
    }

    /// Suelo llano que sube a una rampa de 30° sin ningún escalón de altura: solo el cambio de
    /// plano separa la rampa del suelo.
    #[test]
    fn crease_separates_ramp_from_flat() {
        let mut pts = Vec::new();
        for i in 0..80 {
            for j in 0..40 {
                let (x, y) = (f64::from(i) * 0.25, f64::from(j) * 0.25);
                let z = if x > 10.0 { (x - 10.0) * 30f64.to_radians().tan() } else { 0.0 };
                pts.push(pt(x, y, z, 2));
            }
        }
        let idx = Index::new(&pts, 1.0);
        let prm = WandParams { max_step: 1.0, ..WandParams::default() };
        let (flat, _) = grow(&pts, &idx, near(&pts, 3.0, 5.0), &prm);
        assert!(flat.iter().all(|&i| pts[i as usize].x < 11.3), "se coló en la rampa");
        assert!(flat.len() > 400);
        let (ramp, _) = grow(&pts, &idx, near(&pts, 15.0, 5.0), &prm);
        assert!(ramp.iter().all(|&i| pts[i as usize].x > 8.7), "se coló en el llano");
        assert!(ramp.len() > 400);
    }
}
