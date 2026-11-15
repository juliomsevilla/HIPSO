//! Medición de troncos y cubicación a partir de nubes de puntos terrestres (y móviles o
//! fotogramétricas) de una parcela forestal. Los pasos:
//!
//! 1. La nube se lee en flujo y se reduce a un punto por vóxel, de modo que la memoria sigue a los
//!    vóxeles ocupados y no al número de puntos.
//! 2. Alturas sobre el suelo: los puntos ya clasificados como suelo, una simulación de tela sobre
//!    los puntos más bajos o las alturas que ya trae la nube.
//! 3. Los troncos se buscan en una franja horizontal (por defecto de 0,7 a 3,5 m sobre el suelo):
//!    se conservan los puntos cuyo entorno está estirado en vertical, se agrupan y a los grupos se
//!    les quitan las ramas repitiendo los dos pasos. A cada grupo se le ajusta un eje principal.
//! 4. Cada tronco se sigue hacia arriba y hacia abajo desde la franja, ajustando un círculo a una
//!    rebanada horizontal fina cada pocos centímetros y prediciendo el centro siguiente a partir de
//!    los últimos, de modo que se siguen los troncos inclinados y curvos. Una sección cuenta solo
//!    si tiene suficientes puntos, repartidos alrededor de suficiente parte de la circunferencia y
//!    ninguno dentro; las secciones que no concuerdan con sus vecinas se descartan y los huecos
//!    cortos se interpolan.
//! 5. Trozas: desde la altura de la base hacia arriba, un diámetro cada `log_len` metros y la
//!    fórmula de Smalian para el volumen de cada troza, ((A1 + A2) / 2) · L, hasta que el tronco es
//!    tan fino como el diámetro de punta (la última troza se corta donde el diámetro lo alcanza).
//! 6. Opcionalmente, cada punto de la nube se asigna a su eje de tronco más cercano y se escribe
//!    como una nube por árbol.

// El código de matrices pequeñas de tamaño fijo se lee mejor con índices.
#![allow(clippy::needless_range_loop)]

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::csf::{self, CsfParams};
use crate::grid::Grid;
use crate::las::{Point, Reader, Writer, WriterOptions};
use crate::vector::{ColKind, Table, Val};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct TlsParams {
    /// Vóxel (m) al que se reduce la nube.
    pub voxel: f64,
    /// Las z de la nube ya son alturas sobre el suelo.
    pub normalized: bool,
    /// Distancia entre las partículas de la tela (m) cuando se simula el suelo.
    pub cloth: f64,
    /// Franja donde se buscan los troncos, alturas sobre el suelo.
    pub stripe: (f64, f64),
    /// Rondas de filtrado vertical y agrupación («quitar ramas»).
    pub peel: usize,
    /// Radio (m) del entorno que se usa para la verticalidad.
    pub vert_radius: f64,
    /// Menor verticalidad (0–1) de un punto de tronco.
    pub vert_min: f64,
    /// Menor número de vóxeles de un grupo de tronco en la franja.
    pub min_stem_points: usize,
    /// Extensión vertical (m) que debe tener un grupo para ser un tronco.
    pub height_range: f64,
    /// Mayor y menor diámetro (m) de una sección de tronco.
    pub max_diam: f64,
    pub min_diam: f64,
    /// Semigrosor (m) de una rebanada de sección.
    pub slice: f64,
    /// Menor número de vóxeles de una sección, sectores (de 16) que debe ocupar y vóxeles
    /// permitidos dentro del círculo interior.
    pub min_section_points: usize,
    pub min_sectors: usize,
    pub max_inner: usize,
    /// Altura (m) sobre el suelo donde empieza la primera troza.
    pub base_height: f64,
    /// Diámetro (m) en el que termina el fuste comercial.
    pub top_diam: f64,
    /// Distancia (m) entre los diámetros medidos a lo largo del tronco: la longitud de una troza.
    pub log_len: f64,
    /// Highest section (m).
    pub max_height: f64,
    /// Distancia (m) hasta la que se asigna un punto a un tronco para las nubes por árbol.
    pub assign_dist: f64,
    /// Distancia (m) entre las bases de troncos que crecen en una misma cepa (brotes de un monte
    /// bajo o ramas de una bifurcación).
    pub stool_dist: f64,
    /// Altura (m) del diámetro. Los troncos que están separados a esta altura son brotes de una
    /// cepa, cada uno un árbol; los troncos que son uno por debajo y se bifurcan por encima son un
    /// solo árbol bifurcado.
    pub fork_height: f64,
    /// Mayor tramo (m) de un tronco que puede quedar sin medir (una oclusión, follaje, una parte
    /// poco densa) antes de darlo por perdido. La búsqueda se ensancha con el tramo.
    pub max_gap: f64,
    /// Proporción (0–1) de la altura del árbol que se espera que sea su altura comercial, que se
    /// usa para estimarla en un tronco que se perdió cuando aún era grueso y muy por debajo de la
    /// cima del árbol. 0: sin estimación (el tronco queda truncado).
    pub lost_top_share: f64,
}

impl Default for TlsParams {
    fn default() -> Self {
        Self {
            voxel: 0.03,
            normalized: false,
            cloth: 0.5,
            stripe: (0.7, 3.5),
            peel: 2,
            vert_radius: 0.1,
            vert_min: 0.7,
            min_stem_points: 300,
            height_range: 0.7,
            max_diam: 1.0,
            min_diam: 0.04,
            slice: 0.05,
            min_section_points: 25,
            min_sectors: 9,
            max_inner: 5,
            base_height: 0.10,
            top_diam: 0.07,
            log_len: 0.5,
            max_height: 40.0,
            assign_dist: 8.0,
            stool_dist: 0.6,
            fork_height: 1.3,
            max_gap: 2.0,
            lost_top_share: 0.0,
        }
    }
}

impl TlsParams {
    fn check(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::Param(m.to_string()));
        if !(self.voxel >= 0.005) {
            return bad("el vóxel debe medir al menos 5 mm");
        }
        if !(self.log_len >= 0.1) {
            return bad("the log length must be at least 0.1 m");
        }
        if !(self.top_diam > 0.0 && self.top_diam < self.max_diam) {
            return bad("the top diameter must be positive and below the largest diameter");
        }
        if !(self.base_height >= 0.0) {
            return bad("the base height cannot be negative");
        }
        if !(self.stripe.0 >= 0.0 && self.stripe.1 > self.stripe.0) {
            return bad("the stripe needs a lower limit below its upper limit");
        }
        if !(self.min_diam > 0.0 && self.min_diam < self.max_diam) {
            return bad("the smallest diameter must be below the largest");
        }
        if !(self.stool_dist >= 0.0) {
            return bad("the distance between the stems of a stool cannot be negative");
        }
        if !(self.fork_height > 0.0) {
            return bad("the height of the diameter must be positive");
        }
        if !(self.max_gap >= 0.0) {
            return bad("the longest gap cannot be negative");
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------------- utilidades pequeñas

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

/// Autovector del mayor autovalor de una matriz simétrica de 3×3.
fn principal(m: [[f64; 3]; 3]) -> [f64; 3] {
    eigen(m).1
}

/// Autovalores (el mayor primero) y autovector del mayor, por rotaciones de Jacobi.
pub(crate) fn eigen(m: [[f64; 3]; 3]) -> ([f64; 3], [f64; 3]) {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..12 {
        let (mut p, mut q, mut big) = (0, 1, a[0][1].abs());
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > big {
                (p, q, big) = (i, j, a[i][j].abs());
            }
        }
        if big < 1e-14 {
            break;
        }
        let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
        let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
        let (c, s) = (1.0 / (t * t + 1.0).sqrt(), t / (t * t + 1.0).sqrt());
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
        for row in &mut v {
            let (vp, vq) = (row[p], row[q]);
            row[p] = c * vp - s * vq;
            row[q] = s * vp + c * vq;
        }
    }
    let mut vals = [a[0][0], a[1][1], a[2][2]];
    let k = (0..3)
        .max_by(|&i, &j| a[i][i].total_cmp(&a[j][j]))
        .unwrap_or(0);
    vals.sort_by(|x, y| y.total_cmp(x));
    (vals, [v[0][k], v[1][k], v[2][k]])
}

pub(crate) fn covariance(
    pts: impl Iterator<Item = [f64; 3]> + Clone,
) -> ([f64; 3], [[f64; 3]; 3], usize) {
    let (mut n, mut m) = (0usize, [0.0; 3]);
    for p in pts.clone() {
        n += 1;
        for k in 0..3 {
            m[k] += p[k];
        }
    }
    if n == 0 {
        return (m, [[0.0; 3]; 3], 0);
    }
    for v in &mut m {
        *v /= n as f64;
    }
    let mut c = [[0.0; 3]; 3];
    for p in pts {
        let d = [p[0] - m[0], p[1] - m[1], p[2] - m[2]];
        for i in 0..3 {
            for j in 0..3 {
                c[i][j] += d[i] * d[j];
            }
        }
    }
    for row in &mut c {
        for v in row {
            *v /= n as f64;
        }
    }
    (m, c, n)
}

// ----------------------------------------------------------------------------- la nube

struct Cloud {
    /// Esquina inferior de la nube: las coordenadas de abajo son relativas a ella.
    origin: [f64; 3],
    xyz: Vec<[f32; 3]>,
    class: Vec<u8>,
    /// Altura sobre el suelo.
    z0: Vec<f32>,
    date: Option<String>,
    epsg: Option<u16>,
    /// La nube, antes de los vóxeles: puntos leídos.
    read: u64,
}

fn load(input: &Path, p: &TlsParams) -> Result<Cloud> {
    let r = Reader::open(input)?;
    let b = r.header.bounds;
    let origin = [b[0], b[1], b[2]];
    let v = p.voxel;
    let mut seen: FxMap<u64, ()> = FxMap::default();
    let (mut xyz, mut class) = (Vec::new(), Vec::new());
    let mut read = 0u64;
    r.scan(|pt| {
        read += 1;
        if pt.is_noise() {
            return;
        }
        let (x, y, z) = (pt.x - origin[0], pt.y - origin[1], pt.z - origin[2]);
        let key = pack(
            (x / v).floor() as i64,
            (y / v).floor() as i64,
            (z / v).floor() as i64,
        );
        if seen.insert(key, ()).is_none() {
            xyz.push([x as f32, y as f32, z as f32]);
            class.push(pt.class);
        }
    })?;
    if xyz.is_empty() {
        return Err(Error::Param(format!(
            "{}: no hay puntos que analizar",
            input.display()
        )));
    }
    Ok(Cloud {
        origin,
        z0: Vec::new(),
        xyz,
        class,
        date: r.capture_date(),
        epsg: r.header.epsg,
        read,
    })
}

/// Alturas sobre el suelo de cada punto; la rejilla de suelo usada (coordenadas relativas).
fn normalize(c: &mut Cloud, p: &TlsParams) -> Result<Option<Grid>> {
    if p.normalized {
        c.z0 = c.xyz.iter().map(|q| q[2] + c.origin[2] as f32).collect();
        return Ok(None);
    }
    let (mut x1, mut y1) = (0f32, 0f32);
    for q in &c.xyz {
        x1 = x1.max(q[0]);
        y1 = y1.max(q[1]);
    }
    let cell = p.cloth.max(0.1);
    let mut g = Grid::covering([0.0, 0.0, f64::from(x1) + cell, f64::from(y1) + cell], cell)?;
    let n_ground = c.class.iter().filter(|k| **k == 2).count();
    let ground: Vec<usize> = if n_ground * 200 >= c.class.len() {
        // la clase de suelo propia de la entrega
        (0..c.class.len()).filter(|&i| c.class[i] == 2).collect()
    } else {
        // simulación de tela sobre el punto más bajo de cada celda de 0,25 m
        let mut low: FxMap<u64, usize> = FxMap::default();
        for (i, q) in c.xyz.iter().enumerate() {
            let k = pack(
                (q[0] / 0.25).floor() as i64,
                (q[1] / 0.25).floor() as i64,
                0,
            );
            low.entry(k)
                .and_modify(|j| {
                    if q[2] < c.xyz[*j][2] {
                        *j = i;
                    }
                })
                .or_insert(i);
        }
        let idx: Vec<usize> = low.into_values().collect();
        let pts: Vec<Point> = idx
            .iter()
            .map(|&i| Point {
                x: f64::from(c.xyz[i][0]),
                y: f64::from(c.xyz[i][1]),
                z: f64::from(c.xyz[i][2]),
                number_of_returns: 1,
                return_number: 1,
                ..Point::default()
            })
            .collect();
        let flags = csf::ground(
            &pts,
            &CsfParams {
                resolution: cell,
                rigidness: 2,
                threshold: 0.25,
                ..CsfParams::default()
            },
        )?;
        idx.into_iter()
            .zip(flags)
            .filter_map(|(i, f)| f.then_some(i))
            .collect()
    };
    if ground.is_empty() {
        return Err(Error::Param(
            "no se encuentra suelo: indica que las alturas ya son sobre el suelo (nube normalizada) o revisa la nube"
                .into(),
        ));
    }
    let mut sum = vec![0f64; g.len()];
    let mut cnt = vec![0u32; g.len()];
    for &i in &ground {
        let q = c.xyz[i];
        if let Some(k) = g.index(f64::from(q[0]), f64::from(q[1])) {
            sum[k] += f64::from(q[2]);
            cnt[k] += 1;
        }
    }
    for k in 0..g.len() {
        g.data[k] = if cnt[k] > 0 {
            (sum[k] / f64::from(cnt[k])) as f32
        } else {
            f32::NAN
        };
    }
    g.fill_holes();
    c.z0 = c
        .xyz
        .iter()
        .map(|q| {
            let gz = g.sample(f64::from(q[0]), f64::from(q[1])).unwrap_or(q[2]);
            q[2] - gz
        })
        .collect();
    Ok(Some(g))
}

// ----------------------------------------------------------------------------- troncos en la
// franja

/// Un tronco hallado en la franja: un eje principal por `c`, apuntando hacia arriba.
#[derive(Clone, Debug)]
struct Stem {
    /// Punto del eje a la altura sobre el suelo `h0` (coordenadas relativas, x y).
    c: [f64; 2],
    h0: f64,
    /// Deriva horizontal del eje por metro de altura.
    slope: [f64; 2],
    n: usize,
    /// Las alturas entre las que se midió el eje. Dentro de ellas es donde está el tronco, y se fía
    /// más de ellas que de una predicción a partir de las secciones ya medidas: eso es lo que
    /// mantiene cada una en su sitio a las ramas de una bifurcación, que comparten tronco por
    /// debajo.
    from: f64,
    to: f64,
}

/// Verticalidad de los puntos de `set` (el coseno entre la dirección principal de su entorno y la
/// vertical), usando solo vecinos de `set`.
fn verticality(xyz: &[[f32; 3]], set: &[u32], radius: f64) -> Vec<f32> {
    let r = radius as f32;
    let key = |q: &[f32; 3]| {
        (
            (q[0] / r).floor() as i64,
            (q[1] / r).floor() as i64,
            (q[2] / r).floor() as i64,
        )
    };
    let mut order: Vec<u32> = (0..set.len() as u32).collect();
    let keys: Vec<u64> = set
        .iter()
        .map(|&i| {
            let k = key(&xyz[i as usize]);
            pack(k.0, k.1, k.2)
        })
        .collect();
    order.par_sort_unstable_by_key(|&j| keys[j as usize]);
    let mut cells: FxMap<u64, (u32, u32)> = FxMap::default();
    let mut s = 0usize;
    while s < order.len() {
        let k = keys[order[s] as usize];
        let mut e = s + 1;
        while e < order.len() && keys[order[e] as usize] == k {
            e += 1;
        }
        cells.insert(k, (s as u32, (e - s) as u32));
        s = e;
    }
    let r2 = r * r;
    (0..set.len())
        .into_par_iter()
        .map(|j| {
            let q = xyz[set[j] as usize];
            let (cx, cy, cz) = key(&q);
            let mut nb: Vec<[f64; 3]> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(&(st, n)) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) {
                            for &o in &order[st as usize..(st + n) as usize] {
                                let w = xyz[set[o as usize] as usize];
                                let d = (w[0] - q[0]).powi(2)
                                    + (w[1] - q[1]).powi(2)
                                    + (w[2] - q[2]).powi(2);
                                if d <= r2 {
                                    nb.push([f64::from(w[0]), f64::from(w[1]), f64::from(w[2])]);
                                }
                            }
                        }
                    }
                }
            }
            if nb.len() < 6 {
                return 0.0;
            }
            let (_, cov, _) = covariance(nb.iter().copied());
            principal(cov)[2].abs() as f32
        })
        .collect()
}

/// Grupos de puntos más cercanos que `eps` (encadenados): el número de grupo de cada uno y sus
/// tamaños.
fn group(xyz: &[[f32; 3]], set: &[u32], eps: f64) -> (Vec<u32>, Vec<usize>) {
    let e = eps as f32;
    let key = |q: &[f32; 3]| {
        (
            (q[0] / e).floor() as i64,
            (q[1] / e).floor() as i64,
            (q[2] / e).floor() as i64,
        )
    };
    let mut cells: FxMap<u64, Vec<u32>> = FxMap::default();
    for (j, &i) in set.iter().enumerate() {
        let k = key(&xyz[i as usize]);
        cells.entry(pack(k.0, k.1, k.2)).or_default().push(j as u32);
    }
    let mut label = vec![u32::MAX; set.len()];
    let mut sizes = Vec::new();
    let e2 = e * e;
    let mut stack = Vec::new();
    for s in 0..set.len() {
        if label[s] != u32::MAX {
            continue;
        }
        let id = sizes.len() as u32;
        label[s] = id;
        stack.push(s);
        let mut n = 0;
        while let Some(j) = stack.pop() {
            n += 1;
            let q = xyz[set[j] as usize];
            let (cx, cy, cz) = key(&q);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = cells.get(&pack(cx + dx, cy + dy, cz + dz)) {
                            for &o in list {
                                if label[o as usize] != u32::MAX {
                                    continue;
                                }
                                let w = xyz[set[o as usize] as usize];
                                let d = (w[0] - q[0]).powi(2)
                                    + (w[1] - q[1]).powi(2)
                                    + (w[2] - q[2]).powi(2);
                                if d <= e2 {
                                    label[o as usize] = id;
                                    stack.push(o as usize);
                                }
                            }
                        }
                    }
                }
            }
        }
        sizes.push(n);
    }
    (label, sizes)
}

/// Un tronco seguido por los niveles de la franja: (altura, x, y, diámetro, vóxeles) de su círculo
/// en cada nivel en que se encontró. Un círculo a una altura: (altura, x, y, diámetro, vóxeles).
type Circle = (f64, f64, f64, f64, usize);

struct Chain {
    at: Vec<Circle>,
    last_level: usize,
}

impl Chain {
    /// Dónde se espera el círculo de la cadena a la altura `h`: su último centro prolongado con la
    /// deriva entre sus círculos primero y último.
    fn predict(&self, h: f64) -> (f64, f64) {
        let (a, b) = (self.at[0], self.at[self.at.len() - 1]);
        let span = b.0 - a.0;
        if span < 1e-6 {
            return (b.1, b.2);
        }
        let drift = |from: f64, to: f64| ((to - from) / span).clamp(-0.47, 0.47);
        (
            b.1 + drift(a.1, b.1) * (h - b.0),
            b.2 + drift(a.2, b.2) * (h - b.0),
        )
    }

    /// El eje a lo largo del cual están los círculos, como tronco: una recta de mínimos cuadrados
    /// de cada coordenada sobre la altura.
    fn stem(&self) -> Stem {
        let n = self.at.len() as f64;
        let mean = |f: &dyn Fn(&Circle) -> f64| self.at.iter().map(f).sum::<f64>() / n;
        let (h0, x0, y0) = (mean(&|a| a.0), mean(&|a| a.1), mean(&|a| a.2));
        let var: f64 = self.at.iter().map(|a| (a.0 - h0).powi(2)).sum();
        let drift = |at: &dyn Fn(&Circle) -> f64, m: f64| {
            if var < 1e-9 {
                0.0
            } else {
                (self
                    .at
                    .iter()
                    .map(|a| (a.0 - h0) * (at(a) - m))
                    .sum::<f64>()
                    / var)
                    .clamp(-0.47, 0.47)
            }
        };
        Stem {
            c: [x0, y0],
            h0,
            slope: [drift(&|a| a.1, x0), drift(&|a| a.2, y0)],
            n: self.at.iter().map(|a| a.4).sum(),
            from: self.at[0].0,
            to: self.at[self.at.len() - 1].0,
        }
    }
}

/// Los círculos de un grupo de la franja seguidos de nivel en nivel: en varios niveles se buscan
/// todos los círculos del grupo (varios troncos que se tocan son varios círculos, no uno ancho), y
/// los círculos de niveles consecutivos que se continúan forman una cadena, un candidato a tronco.
fn chain_candidates(c: &Cloud, ids: &[u32], p: &TlsParams) -> Vec<Chain> {
    let (lo, hi) = (p.stripe.0 + p.slice, p.stripe.1 - p.slice);
    let mut levels = Vec::new();
    let mut h = lo;
    while h <= hi + 1e-9 {
        levels.push(h);
        h += 0.25;
    }
    let mut chains: Vec<Chain> = Vec::new();
    for (li, &h) in levels.iter().enumerate() {
        let pts: Vec<[f32; 2]> = ids
            .iter()
            .filter(|&&i| (f64::from(c.z0[i as usize]) - h).abs() <= p.slice)
            .map(|&i| [c.xyz[i as usize][0], c.xyz[i as usize][1]])
            .collect();
        if pts.len() < 8 {
            continue;
        }
        let n = pts.len() as f64;
        let centre = (
            pts.iter().map(|q| f64::from(q[0])).sum::<f64>() / n,
            pts.iter().map(|q| f64::from(q[1])).sum::<f64>() / n,
        );
        let mut taken = vec![false; chains.len()];
        let fits = circles_in(&pts, centre, f64::INFINITY, p, None, 6);
        for f in fits {
            // la cadena cuyo círculo, prolongado, es el más cercano y de un tamaño parecido
            let mut best: Option<(usize, f64)> = None;
            for (k, ch) in chains.iter().enumerate() {
                if taken[k] || li - ch.last_level > 2 {
                    continue;
                }
                let (px, py) = ch.predict(h);
                let (away, d_last) = ((f.0 - px).hypot(f.1 - py), ch.at[ch.at.len() - 1].3);
                if away <= 0.06 + 0.3 * f.2.max(d_last)
                    && (0.6..=1.6).contains(&(f.2 / d_last))
                    && best.is_none_or(|b| away < b.1)
                {
                    best = Some((k, away));
                }
            }
            match best {
                Some((k, _)) => {
                    chains[k].at.push((h, f.0, f.1, f.2, f.3));
                    chains[k].last_level = li;
                    taken[k] = true;
                }
                None => {
                    chains.push(Chain {
                        at: vec![(h, f.0, f.1, f.2, f.3)],
                        last_level: li,
                    });
                    taken.push(true);
                }
            }
        }
    }
    chains
}

/// Si una cadena es un tronco: necesita unos cuantos niveles; una rama que sale de una bifurcación
/// dentro de la franja puede tener solo dos, si empieza justo encima de un círculo de un tronco que
/// los tiene (`solid`, las cadenas de tres niveles o más, de cualquier grupo).
fn is_stem(ch: &Chain, solid: &[&Chain]) -> bool {
    ch.at.len() >= 3
        || (ch.at.len() == 2
            && solid.iter().any(|s| {
                s.at.iter().any(|a| {
                    (0.0..=1.0).contains(&(ch.at[0].0 - a.0))
                        && (a.1 - ch.at[0].1).hypot(a.2 - ch.at[0].2) < 0.5
                })
            }))
}

fn find_stems(c: &Cloud, p: &TlsParams) -> Vec<Stem> {
    let mut set: Vec<u32> = (0..c.xyz.len() as u32)
        .filter(|&i| {
            let h = f64::from(c.z0[i as usize]);
            h >= p.stripe.0 && h <= p.stripe.1
        })
        .collect();
    let eps = p.voxel * 2.2;
    let mut labels: Vec<u32> = Vec::new();
    for _ in 0..p.peel.max(1) {
        if set.is_empty() {
            return Vec::new();
        }
        let v = verticality(&c.xyz, &set, p.vert_radius);
        set = set
            .iter()
            .zip(&v)
            .filter(|(_, v)| f64::from(**v) >= p.vert_min)
            .map(|(i, _)| *i)
            .collect();
        let sizes;
        (labels, sizes) = group(&c.xyz, &set, eps);
        let keep: Vec<bool> = sizes.iter().map(|n| *n >= p.min_stem_points).collect();
        let kept: Vec<(u32, u32)> = set
            .iter()
            .zip(&labels)
            .filter(|(_, l)| keep[**l as usize])
            .map(|(i, l)| (*i, *l))
            .collect();
        set = kept.iter().map(|k| k.0).collect();
        labels = kept.iter().map(|k| k.1).collect();
    }
    if set.is_empty() {
        return Vec::new();
    }
    let mut by: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, l) in set.iter().zip(&labels) {
        by.entry(*l).or_default().push(*i);
    }
    let groups: Vec<Vec<u32>> = by
        .into_values()
        .filter(|ids| {
            let (lo, hi) = ids.iter().fold((f64::MAX, f64::MIN), |a, &i| {
                let z = f64::from(c.z0[i as usize]);
                (a.0.min(z), a.1.max(z))
            });
            hi - lo >= p.height_range
        })
        .collect();
    // primero los círculos de cada grupo: una rama de una bifurcación puede estar en un grupo
    // propio, aparte del tronco que tiene debajo, y lo que muestra que es una rama es el tronco que
    // termina justo debajo
    let all: Vec<Vec<Chain>> = groups
        .iter()
        .map(|ids| chain_candidates(c, ids, p))
        .collect();
    let solid: Vec<&Chain> = all.iter().flatten().filter(|ch| ch.at.len() >= 3).collect();
    let mut stems = Vec::new();
    for (ids, chains) in groups.iter().zip(&all) {
        let found: Vec<Stem> = chains
            .iter()
            .filter(|ch| is_stem(ch, &solid))
            .map(Chain::stem)
            .collect();
        if !found.is_empty() {
            stems.extend(found);
            continue;
        }
        // no se pudo seguir ningún círculo (troncos vistos solo en arcos): el eje de todo el grupo
        let pts = ids.iter().map(|&i| {
            let q = c.xyz[i as usize];
            [
                f64::from(q[0]),
                f64::from(q[1]),
                f64::from(c.z0[i as usize]),
            ]
        });
        let (m, cov, n) = covariance(pts.clone());
        let (zmin, zmax) = pts
            .clone()
            .fold((f64::MAX, f64::MIN), |a, q| (a.0.min(q[2]), a.1.max(q[2])));
        let mut d = principal(cov);
        if d[2] < 0.0 {
            d = [-d[0], -d[1], -d[2]];
        }
        if d[2] < 0.7 {
            continue;
        }
        let slope = [d[0] / d[2], d[1] / d[2]];
        // un tronco es una columna fina alrededor de su eje
        let far = pts
            .clone()
            .map(|q| {
                let t = q[2] - m[2];
                ((q[0] - m[0] - slope[0] * t).powi(2) + (q[1] - m[1] - slope[1] * t).powi(2)).sqrt()
            })
            .fold(0.0, f64::max);
        if far > p.max_diam * 0.75 {
            continue;
        }
        stems.push(Stem {
            c: [m[0], m[1]],
            h0: m[2],
            slope,
            n,
            from: zmin,
            to: zmax,
        });
    }
    stems.sort_by(|a, b| a.c[0].total_cmp(&b.c[0]).then(a.c[1].total_cmp(&b.c[1])));
    // dos grupos de un mismo tronco (un hueco en la franja): se conserva el mayor
    let mut out: Vec<Stem> = Vec::new();
    for s in stems {
        let near = out.iter_mut().find(|o| {
            let t = s.h0 - o.h0;
            let (px, py) = (o.c[0] + o.slope[0] * t, o.c[1] + o.slope[1] * t);
            ((px - s.c[0]).powi(2) + (py - s.c[1]).powi(2)).sqrt() < 0.12
        });
        match near {
            Some(o) if s.n > o.n => *o = s,
            Some(_) => {}
            None => out.push(s),
        }
    }
    out
}

// ----------------------------------------------------------------------------- sections

/// Vóxeles por celda horizontal, ordenados por altura, para cortar rebanadas deprisa.
struct SliceIndex {
    cell: f32,
    order: Vec<u32>,
    cells: FxMap<u64, (u32, u32)>,
}

impl SliceIndex {
    fn new(c: &Cloud) -> Self {
        let cell = 0.5f32;
        let keys: Vec<u64> = c
            .xyz
            .iter()
            .map(|q| {
                pack(
                    (q[0] / cell).floor() as i64,
                    (q[1] / cell).floor() as i64,
                    0,
                )
            })
            .collect();
        let mut order: Vec<u32> = (0..c.xyz.len() as u32).collect();
        order.par_sort_unstable_by(|&a, &b| {
            keys[a as usize]
                .cmp(&keys[b as usize])
                .then(c.z0[a as usize].total_cmp(&c.z0[b as usize]))
        });
        let mut cells: FxMap<u64, (u32, u32)> = FxMap::default();
        let mut s = 0usize;
        while s < order.len() {
            let k = keys[order[s] as usize];
            let mut e = s + 1;
            while e < order.len() && keys[order[e] as usize] == k {
                e += 1;
            }
            cells.insert(k, (s as u32, (e - s) as u32));
            s = e;
        }
        Self { cell, order, cells }
    }

    /// Los puntos a menos de `rad` de `(x, y)` y entre las alturas `lo` y `hi`.
    fn slice(&self, c: &Cloud, x: f64, y: f64, rad: f64, lo: f64, hi: f64) -> Vec<[f32; 2]> {
        let cell = f64::from(self.cell);
        let (cx0, cx1) = (
            ((x - rad) / cell).floor() as i64,
            ((x + rad) / cell).floor() as i64,
        );
        let (cy0, cy1) = (
            ((y - rad) / cell).floor() as i64,
            ((y + rad) / cell).floor() as i64,
        );
        let mut out = Vec::new();
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                let Some(&(s, n)) = self.cells.get(&pack(cx, cy, 0)) else {
                    continue;
                };
                let list = &self.order[s as usize..(s + n) as usize];
                let a = list.partition_point(|&i| f64::from(c.z0[i as usize]) < lo);
                for &i in &list[a..] {
                    if f64::from(c.z0[i as usize]) > hi {
                        break;
                    }
                    let q = c.xyz[i as usize];
                    if (f64::from(q[0]) - x).hypot(f64::from(q[1]) - y) <= rad {
                        out.push([q[0], q[1]]);
                    }
                }
            }
        }
        out
    }
}

/// Un círculo por mínimos cuadrados: arranque algebraico y luego Gauss–Newton sobre las distancias
/// geométricas.
fn fit_circle(pts: &[[f32; 2]]) -> Option<(f64, f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let (mx, my) = pts.iter().fold((0.0, 0.0), |a, q| {
        (a.0 + f64::from(q[0]), a.1 + f64::from(q[1]))
    });
    let (mx, my) = (mx / n as f64, my / n as f64);
    let (mut suu, mut suv, mut svv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for q in pts {
        let (u, v) = (f64::from(q[0]) - mx, f64::from(q[1]) - my);
        suu += u * u;
        suv += u * v;
        svv += v * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-12 {
        return None;
    }
    let (b1, b2) = (0.5 * (suuu + suvv), 0.5 * (svvv + svuu));
    let (mut uc, mut vc) = ((b1 * svv - b2 * suv) / det, (b2 * suu - b1 * suv) / det);
    let mut r = (uc * uc + vc * vc + (suu + svv) / n as f64).sqrt();
    for _ in 0..6 {
        let (mut a, mut b, mut c2, mut g1, mut g2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for q in pts {
            let (u, v) = (f64::from(q[0]) - mx - uc, f64::from(q[1]) - my - vc);
            let d = u.hypot(v).max(1e-12);
            let res = d - r;
            let (ju, jv) = (-u / d, -v / d);
            a += ju * ju;
            b += ju * jv;
            c2 += jv * jv;
            g1 += ju * res;
            g2 += jv * res;
        }
        let dd = a * c2 - b * b;
        if dd.abs() < 1e-12 {
            break;
        }
        let (du, dv) = ((c2 * g1 - b * g2) / dd, (a * g2 - b * g1) / dd);
        uc -= du;
        vc -= dv;
        r = pts
            .iter()
            .map(|q| (f64::from(q[0]) - mx - uc).hypot(f64::from(q[1]) - my - vc))
            .sum::<f64>()
            / n as f64;
        if du.abs() + dv.abs() < 1e-6 {
            break;
        }
    }
    (r.is_finite() && uc.is_finite() && vc.is_finite()).then_some((mx + uc, my + vc, r))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Altura sobre el suelo.
    pub h: f64,
    /// Centro y diámetro (coordenadas relativas hasta que se hace el resultado).
    pub x: f64,
    pub y: f64,
    pub d: f64,
    pub ok: bool,
    pub points: usize,
    pub sectors: usize,
}

/// Un círculo por tres puntos.
fn circumcircle(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> Option<(f64, f64, f64)> {
    let (ax, ay) = (f64::from(a[0]), f64::from(a[1]));
    let (bx, by) = (f64::from(b[0]) - ax, f64::from(b[1]) - ay);
    let (cx, cy) = (f64::from(c[0]) - ax, f64::from(c[1]) - ay);
    let d = 2.0 * (bx * cy - by * cx);
    if d.abs() < 1e-9 {
        return None;
    }
    let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
    let (ux, uy) = ((cy * b2 - by * c2) / d, (bx * c2 - cx * b2) / d);
    Some((ax + ux, ay + uy, ux.hypot(uy)))
}

/// El círculo cuya circunferencia está más cubierta, entre los que pasan por tripletes al azar:
/// robusto frente a ramas, hojas y vecinos que están en la rebanada. `tol` es la distancia a la que
/// puede estar un punto; `per_m` los puntos que tiene un anillo completo por cada metro de
/// circunferencia. Lo que cuenta es la cobertura, no cuántos puntos: un círculo ancho trazado por
/// arcos de varios troncos tiene tantos puntos como el anillo de un tronco, pero cubre una fracción
/// de su propia circunferencia.
fn ransac_circle(
    pts: &[[f32; 2]],
    rmin: f64,
    rmax: f64,
    guess: (f64, f64),
    reach: f64,
    tol: f64,
    per_m: f64,
) -> Option<(f64, f64, f64)> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let mut state = 0x2545_F491_4F6C_DD1Du64 ^ (n as u64);
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    let (mut best, mut best_cover) = (None, 0.0f64);
    for _ in 0..150 {
        let (i, j, k) = (next(), next(), next());
        if i == j || j == k || i == k {
            continue;
        }
        let Some((cx, cy, r)) = circumcircle(pts[i], pts[j], pts[k]) else {
            continue;
        };
        if r < rmin || r > rmax || (cx - guess.0).hypot(cy - guess.1) > reach {
            continue;
        }
        let inliers = pts
            .iter()
            .filter(|q| ((f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) - r).abs() <= tol)
            .count();
        let cover = inliers as f64 / (std::f64::consts::TAU * r * per_m);
        if inliers >= 6 && cover > best_cover {
            (best, best_cover) = (Some((cx, cy, r)), cover);
        }
    }
    best
}

/// Un círculo hallado en una rebanada: centro, diámetro, vóxeles que concuerdan y sectores que
/// ocupan.
type Fit = (f64, f64, f64, usize, usize);

/// Los círculos de una rebanada, el mejor respaldado primero, como mucho `max`. Tras cada uno se
/// descartan los puntos de su alrededor, de modo que los círculos de troncos que se tocan (brotes
/// de una cepa, ramas de una bifurcación) se hallan uno a uno y no como un único círculo ancho
/// alrededor de ambos.
fn circles_in(
    pts: &[[f32; 2]],
    guess: (f64, f64),
    reach: f64,
    p: &TlsParams,
    prev: Option<f64>,
    max: usize,
) -> Vec<Fit> {
    let tol = p.voxel * 0.75 + 0.004;
    let mut rest = pts.to_vec();
    let mut out = Vec::new();
    for _ in 0..max {
        let Some(f) = circle_at(&rest, guess, reach, p, prev) else {
            break;
        };
        let cut = f.2 / 2.0 + tol * 2.0;
        rest.retain(|q| (f64::from(q[0]) - f.0).hypot(f64::from(q[1]) - f.1) > cut);
        out.push(f);
    }
    out
}

/// Cuánto más lejos (m) hay que buscar un tronco que no se ha medido en `gap` metros: la deriva con
/// que se prolongó es menos segura cuanto más largo es el tramo.
fn widening(gap: f64) -> f64 {
    0.06 * gap.clamp(0.0, 3.0)
}

/// El círculo de una rebanada más cercano a donde se espera el tronco, si la rebanada contiene una
/// sección de tronco. `prev` es el diámetro de la sección anterior, si la hay: el número de puntos
/// y sectores que se piden sigue el tamaño del tronco. Elegir el círculo más cercano, y no el mejor
/// respaldado, es lo que impide que un tronco salte a un vecino más grueso a unos centímetros.
fn measure(
    pts: &[[f32; 2]],
    guess: (f64, f64),
    p: &TlsParams,
    prev: Option<f64>,
    gap: f64,
) -> Option<Fit> {
    let d_ref = prev.unwrap_or(0.3);
    // el tronco puede estar más lejos de la estimación prolongada cuanto más tiempo lleva sin verse
    let reach = (d_ref * 1.2 + 0.15).max(0.3) + widening(gap);
    let away = |f: &Fit| (f.0 - guess.0).hypot(f.1 - guess.1);
    circles_in(pts, guess, reach, p, prev, 3)
        .into_iter()
        .min_by(|a, b| away(a).total_cmp(&away(b)))
}

/// Ajusta y comprueba un círculo de una rebanada a menos de `reach` de `guess`; `None` cuando no
/// hay ninguno que sea una sección de tronco.
fn circle_at(
    pts: &[[f32; 2]],
    guess: (f64, f64),
    reach: f64,
    p: &TlsParams,
    prev: Option<f64>,
) -> Option<Fit> {
    let d_ref = prev.unwrap_or(0.3);
    // lo que da un tronco de este tamaño en una rebanada de este grosor, con este vóxel
    let expected =
        0.5 * (std::f64::consts::PI * d_ref / p.voxel) * (2.0 * p.slice / p.voxel).max(1.0);
    let min_pts = (p.min_section_points as f64).min(expected).max(8.0) as usize;
    let min_sectors = if d_ref < 0.15 {
        p.min_sectors.min(7)
    } else {
        p.min_sectors
    };
    if pts.len() < min_pts {
        return None;
    }
    let tol = p.voxel * 0.75 + 0.004;
    // puntos por metro de un anillo completo: las capas de vóxeles de la rebanada, entre el vóxel
    let per_m = (2.0 * p.slice / p.voxel).max(1.0) / p.voxel;
    let (cx, cy, r) = ransac_circle(
        pts,
        p.min_diam / 2.0,
        p.max_diam / 2.0,
        guess,
        reach,
        tol,
        per_m,
    )?;
    // se refina con los puntos que concuerdan, unas rondas, con una tolerancia que sigue los datos
    let mut kept: Vec<[f32; 2]> = pts
        .iter()
        .copied()
        .filter(|q| ((f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) - r).abs() <= tol * 1.5)
        .collect();
    let (mut cx, mut cy, mut r) = fit_circle(&kept)?;
    for _ in 0..3 {
        let t = (2.5 * rms_residual(&kept, cx, cy, r)).max(tol * 0.6);
        let next: Vec<[f32; 2]> = pts
            .iter()
            .copied()
            .filter(|q| ((f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) - r).abs() <= t)
            .collect();
        if next.len() < min_pts {
            return None;
        }
        let (a, b, c) = fit_circle(&next)?;
        (cx, cy, r) = (a, b, c);
        kept = next;
    }
    if r * 2.0 < p.min_diam || r * 2.0 > p.max_diam {
        return None;
    }
    // el residuo permitido crece con el tronco, pero solo hasta cierto punto: un círculo ancho
    // alrededor de dos troncos que se tocan tiene un residuo grande que el 10 % de su radio dejaría
    // pasar
    let floor = p.voxel * 0.6;
    if rms_residual(&kept, cx, cy, r) > (0.1 * r).clamp(floor, floor + 0.02) {
        return None;
    }
    let mut occupied = [false; 16];
    for q in &kept {
        let a = (f64::from(q[1]) - cy).atan2(f64::from(q[0]) - cx);
        occupied[(((a + std::f64::consts::PI) / std::f64::consts::TAU * 16.0) as usize).min(15)] =
            true;
    }
    let sectors = occupied.iter().filter(|o| **o).count();
    // un tronco es una superficie: poco de la rebanada cae dentro de él. Un círculo alrededor de
    // dos troncos que se tocan, o de una rama con follaje, tiene vóxeles en su círculo interior.
    let inner = pts
        .iter()
        .filter(|q| (f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) < r * 0.5)
        .count();
    (sectors >= min_sectors && inner <= p.max_inner).then_some((
        cx,
        cy,
        2.0 * r,
        kept.len(),
        sectors,
    ))
}

fn rms_residual(pts: &[[f32; 2]], cx: f64, cy: f64, r: f64) -> f64 {
    if pts.is_empty() {
        return 0.0;
    }
    (pts.iter()
        .map(|q| ((f64::from(q[0]) - cx).hypot(f64::from(q[1]) - cy) - r).powi(2))
        .sum::<f64>()
        / pts.len() as f64)
        .sqrt()
}

/// Secciones de un tronco, desde la altura de la base hasta donde el tronco se pierde o es más fino
/// que el diámetro de punta, separadas `step` metros.
fn track(c: &Cloud, idx: &SliceIndex, stem: &Stem, p: &TlsParams, step: f64) -> Vec<Section> {
    let at_axis = |h: f64| {
        let t = h - stem.h0;
        (stem.c[0] + stem.slope[0] * t, stem.c[1] + stem.slope[1] * t)
    };
    let slice_at =
        |x: f64, y: f64, rad: f64, h: f64| idx.slice(c, x, y, rad, h - p.slice, h + p.slice);
    // se empieza a la altura del pecho o a la altura más cercana de la franja que da una sección
    let j_of = |h: f64| ((h - p.base_height) / step).round() as i64;
    let h_of = |j: i64| p.base_height + j as f64 * step;
    let mut starts: Vec<f64> = vec![1.3, 1.5, 1.1, 1.7, 0.9, 1.9, 2.2, 2.6, 3.0];
    starts.retain(|h| *h >= p.stripe.0 && *h <= p.stripe.1);
    if starts.is_empty() {
        starts.push((p.stripe.0 + p.stripe.1) / 2.0);
    }
    let rad0 = p.max_diam / 2.0 * 1.2 + 0.05;
    let mut first = None;
    for h in starts {
        let j = j_of(h);
        let hh = h_of(j);
        let (ax, ay) = at_axis(hh);
        let pts = slice_at(ax, ay, rad0, hh);
        if let Some(m) = measure(&pts, (ax, ay), p, None, 0.0) {
            first = Some((j, m));
            break;
        }
    }
    let Some((j0, m0)) = first else {
        return Vec::new();
    };
    let mk = |h: f64, m: Option<(f64, f64, f64, usize, usize)>, fallback: (f64, f64)| match m {
        Some((x, y, d, n, s)) => Section {
            h,
            x,
            y,
            d,
            ok: true,
            points: n,
            sectors: s,
        },
        None => Section {
            h,
            x: fallback.0,
            y: fallback.1,
            d: f64::NAN,
            ok: false,
            points: 0,
            sectors: 0,
        },
    };
    let mut up: Vec<Section> = vec![mk(h_of(j0), Some(m0), (0.0, 0.0))];
    // hacia arriba y luego hacia abajo, con las mismas reglas
    for dir in [1i64, -1i64] {
        let mut chain: Vec<Section> = vec![up[0].clone()];
        let mut fails = 0;
        let mut thin = 0;
        let mut j = j0;
        let mut slope = stem.slope;
        loop {
            j += dir;
            let h = h_of(j);
            if (dir == 1 && h > p.max_height) || (dir == -1 && j < 0) {
                break;
            }
            let last_ok = chain
                .iter()
                .rev()
                .find(|s| s.ok)
                .cloned()
                .unwrap_or_else(|| up[0].clone());
            let dh = h - last_ok.h;
            // donde se midió el tronco, su propio eje; en otro caso, la última sección prolongada
            let (px, py) = if (stem.from - 0.3..=stem.to + 0.3).contains(&h) {
                at_axis(h)
            } else {
                (last_ok.x + slope[0] * dh, last_ok.y + slope[1] * dh)
            };
            // el tramo desde la última sección medida: la búsqueda se ensancha con él
            let gap = dh.abs();
            let rad = (last_ok.d * 0.75 + 0.1 + widening(gap)).min(p.max_diam);
            let pts = slice_at(px, py, rad, h);
            let mut m = measure(&pts, (px, py), p, Some(last_ok.d), gap);
            // un cambio brusco de tamaño o de posición no es una sección de tronco
            if let Some((x, y, d, ..)) = m {
                let jump = (x - px).hypot(y - py) > 0.15 + 0.5 * last_ok.d + widening(gap);
                let size = (d - last_ok.d).abs() > 0.3 * last_ok.d + 0.03;
                if jump || size {
                    m = None;
                }
            }
            match m {
                Some((x, y, d, ..)) => {
                    fails = 0;
                    if dh.abs() > 1e-9 {
                        let s = [(x - last_ok.x) / dh, (y - last_ok.y) / dh];
                        // mezcla: la deriva de un tronco cambia despacio y se mantiene por debajo
                        // de 25 grados
                        for k in 0..2 {
                            slope[k] = (0.5 * slope[k] + 0.5 * s[k]).clamp(-0.47, 0.47);
                        }
                    }
                    thin = if d < p.top_diam * 0.85 { thin + 1 } else { 0 };
                }
                None => fails += 1,
            }
            chain.push(mk(h, m, (px, py)));
            // perdido: el tronco ha estado sin medir durante el mayor tramo permitido (hacia abajo
            // el suelo está cerca y merece la pena probar uno más largo) o es tan fino como la
            // punta
            if dir == 1 && (gap >= p.max_gap - 1e-9 && fails > 0 || thin >= 2) {
                break;
            }
            if dir == -1 && gap >= p.max_gap.max(2.0) - 1e-9 && fails > 0 {
                break;
            }
        }
        up.extend(chain.into_iter().skip(1));
    }
    up.sort_by(|a, b| a.h.total_cmp(&b.h));
    reject_outliers(&mut up);
    // los fallos finales no aportan información
    while up.last().is_some_and(|s| !s.ok) {
        up.pop();
    }
    up
}

/// Las secciones que no concuerdan con ninguna de sus dos vecinas no son secciones de tronco.
/// Pierden su diámetro además de su `ok`: un diámetro dejado en una sección rechazada pasaría por
/// una medida allí donde el relleno de huecos no puede sustituirlo (un hueco de más de un metro
/// entre las secciones de alrededor) y la cubicación lo tomaría.
fn reject_outliers(secs: &mut [Section]) {
    let ok_idx: Vec<usize> = (0..secs.len()).filter(|&i| secs[i].ok).collect();
    let mut rejected = Vec::new();
    // toda comparación lee los diámetros tal como se midieron, de modo que los rechazos esperan
    for w in 1..ok_idx.len().saturating_sub(1) {
        let (a, b, n) = (ok_idx[w - 1], ok_idx[w], ok_idx[w + 1]);
        let mean = (secs[a].d + secs[n].d) / 2.0;
        if (secs[b].d - mean).abs() > 0.2 * mean + 0.02
            && (secs[b].d - secs[a].d).abs() > 0.1 * secs[a].d
        {
            rejected.push(b);
        }
    }
    for b in rejected {
        secs[b].ok = false;
        secs[b].d = f64::NAN;
    }
}

/// Un tronco se estrecha hacia arriba. A partir de 1 m de altura (por debajo, el tronco puede
/// ensancharse hacia la raíz) las secciones conservadas forman una envolvente que nunca sube: la
/// menor mediana de tres secciones consecutivas conservadas hasta ahora. Una sección más ancha que
/// ella (en más de un 8 % y 1 cm) ha cogido una rama, un tronco vecino o una bifurcación y pierde
/// su diámetro; al ser un suelo de medianas, ni una sección fina suelta ni un lento aumento de
/// secciones anchas pueden mover la envolvente. Devuelve cuántas secciones se descartaron.
fn enforce_taper(secs: &mut [Section]) -> usize {
    let mut last: Vec<f64> = Vec::new();
    let mut envelope = f64::INFINITY;
    let mut dropped = 0;
    for s in secs.iter_mut().filter(|s| s.ok && s.h >= 1.0) {
        if s.d > envelope * 1.08 + 0.01 {
            s.ok = false;
            s.d = f64::NAN;
            dropped += 1;
            continue;
        }
        last.push(s.d);
        if last.len() > 3 {
            last.remove(0);
        }
        if last.len() >= 2 {
            let mut w = last.clone();
            w.sort_by(f64::total_cmp);
            envelope = envelope.min(w[w.len() / 2]);
        }
    }
    dropped
}

// ----------------------------------------------------------------------------- cubing

#[derive(Clone, Debug, PartialEq)]
pub struct Log {
    pub from: f64,
    pub to: f64,
    pub d_from: f64,
    pub d_to: f64,
    /// Smalian's volume, m³.
    pub volume: f64,
    /// 0 para el tronco, 1 o más para cada rama por encima de una bifurcación.
    pub branch: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cubing {
    pub logs: Vec<Log>,
    pub volume: f64,
    /// Altura a la que el tronco alcanza el diámetro de punta o donde se pierde.
    pub top_h: f64,
    /// El tronco se perdió antes de alcanzar el diámetro de punta.
    pub truncated: bool,
    /// El tronco se perdió y el tramo desde allí hasta el diámetro de punta es una estimación.
    pub estimated: bool,
}

fn area(d: f64) -> f64 {
    std::f64::consts::PI * d * d / 4.0
}

/// Diámetro a la altura `h` interpolado linealmente en `profile` (alturas ascendentes).
pub(crate) fn diam_at(profile: &[(f64, f64)], h: f64) -> Option<f64> {
    let (first, last) = (profile.first()?, profile.last()?);
    if h < first.0 - 1e-9 || h > last.0 + 1e-9 {
        return None;
    }
    let k = profile.partition_point(|q| q.0 < h);
    if k == 0 {
        return Some(first.1);
    }
    let (a, b) = (profile[k - 1], profile[k.min(profile.len() - 1)]);
    if (b.0 - a.0).abs() < 1e-12 {
        return Some(b.1);
    }
    Some(a.1 + (b.1 - a.1) * (h - a.0) / (b.0 - a.0))
}

/// Cubicación de Smalian de un tronco a partir de su perfil de diámetros `profile` ((altura,
/// diámetro), ascendente): trozas de `log_len` desde `base` hacia arriba, la última cortada donde
/// el diámetro baja hasta `top_d`.
pub fn smalian(profile: &[(f64, f64)], base: f64, log_len: f64, top_d: f64) -> Cubing {
    let mut out = Cubing::default();
    let Some(&(h_end, _)) = profile.last() else {
        return out;
    };
    // por debajo de la sección más baja se toma el diámetro de la más baja
    let first = profile[0];
    let pf: Vec<(f64, f64)> = if first.0 > base {
        std::iter::once((base, first.1))
            .chain(profile.iter().copied())
            .collect()
    } else {
        profile.to_vec()
    };
    let Some(mut d) = diam_at(&pf, base) else {
        return out;
    };
    if d < top_d {
        return out;
    }
    let mut h = base;
    out.top_h = base;
    loop {
        let hn = (h + log_len).min(h_end);
        let dn = diam_at(&pf, hn).unwrap_or(d);
        if dn >= top_d {
            let l = hn - h;
            if l > 1e-9 {
                let v = (area(d) + area(dn)) / 2.0 * l;
                out.logs.push(Log {
                    from: h,
                    to: hn,
                    d_from: d,
                    d_to: dn,
                    volume: v,
                    branch: 0,
                });
                out.volume += v;
            }
            out.top_h = hn;
            if hn >= h_end - 1e-9 {
                out.truncated = true;
                break;
            }
            (h, d) = (hn, dn);
            continue;
        }
        // el diámetro alcanza el diámetro de punta dentro de esta troza: se busca dónde
        let mut cut = hn;
        let mut prev = (h, d);
        for &(ph, pd) in pf.iter().filter(|q| q.0 > h) {
            if pd < top_d {
                cut = prev.0 + (prev.1 - top_d) / (prev.1 - pd).max(1e-12) * (ph - prev.0);
                break;
            }
            prev = (ph, pd);
        }
        let l = cut - h;
        if l > 1e-9 {
            let v = (area(d) + area(top_d)) / 2.0 * l;
            out.logs.push(Log {
                from: h,
                to: cut,
                d_from: d,
                d_to: top_d,
                volume: v,
                branch: 0,
            });
            out.volume += v;
        }
        out.top_h = cut;
        break;
    }
    out
}

// ----------------------------------------------------------------------------- el resultado

#[derive(Clone, Debug, PartialEq)]
pub struct TreeResult {
    pub id: u32,
    /// Posición del tronco en la base y el suelo allí (coordenadas del mapa).
    pub x: f64,
    pub y: f64,
    pub ground_z: f64,
    /// Diámetro a 1,3 m, en m.
    pub dbh: Option<f64>,
    pub height: f64,
    /// Inclinación del eje del tronco respecto a la vertical, en grados.
    pub tilt: f64,
    pub cubing: Cubing,
    pub sections: Vec<Section>,
    pub points: usize,
    /// Proporción de las secciones a lo largo de la longitud medida que pasaron las comprobaciones.
    pub quality: f64,
    pub d_base: f64,
    pub flags: String,
    /// Eje por (x, y) en la base con esta deriva horizontal por metro (para las nubes por árbol).
    pub slope: [f64; 2],
    /// La cepa en la que crece este tronco (el mismo número para los brotes de una cepa) y cuántos
    /// brotes tiene: 1 para un tronco solo.
    pub stool: u32,
    pub sprouts: u32,
    /// Altura de la bifurcación, para un árbol que se bifurca por encima de la altura del diámetro
    /// (1,3 m): su tronco de abajo es uno y cada rama de arriba se cubica aparte.
    pub fork_h: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct TlsResult {
    pub trees: Vec<TreeResult>,
    pub date: Option<String>,
    pub epsg: Option<u16>,
    pub points_read: u64,
    pub voxels: usize,
    origin: [f64; 3],
}

/// El punto más alto del grupo propio del tronco: los vóxeles de 0,3 m a menos de 1,5 m del eje del
/// tronco, extendidos hacia arriba con la inclinación del tronco, que se encadenan hasta lo alto
/// del tronco medido.
fn tree_height(c: &Cloud, idx: &SliceIndex, secs: &[Section], slope: [f64; 2]) -> f64 {
    let Some(top) = secs.iter().rev().find(|s| s.ok) else {
        return 0.0;
    };
    let mut vox: FxMap<u64, ((i64, i64, i64), f64)> = FxMap::default();
    for (z, x, y) in idx.slice_all(c, top.x, top.y, 8.0) {
        let t = z - top.h;
        let (ax, ay) = (top.x + slope[0] * t, top.y + slope[1] * t);
        if (x - ax).hypot(y - ay) > 1.5 || z < 0.5 {
            continue;
        }
        let k = (
            (x / 0.3).floor() as i64,
            (y / 0.3).floor() as i64,
            (z / 0.3).floor() as i64,
        );
        let e = vox.entry(pack(k.0, k.1, k.2)).or_insert((k, z));
        e.1 = e.1.max(z);
    }
    let start = (
        (top.x / 0.3).floor() as i64,
        (top.y / 0.3).floor() as i64,
        (top.h / 0.3).floor() as i64,
    );
    let mut best = top.h;
    let mut seen: FxMap<u64, ()> = FxMap::default();
    let mut stack = vec![start];
    seen.insert(pack(start.0, start.1, start.2), ());
    while let Some((a, b, d)) = stack.pop() {
        if let Some((_, z)) = vox.get(&pack(a, b, d)) {
            best = best.max(*z);
        }
        for da in -1..=1 {
            for db in -1..=1 {
                for dd in -1..=1 {
                    let k = pack(a + da, b + db, d + dd);
                    if vox.contains_key(&k) && seen.insert(k, ()).is_none() {
                        stack.push((a + da, b + db, d + dd));
                    }
                }
            }
        }
    }
    best
}

impl SliceIndex {
    /// (altura, x, y) de cada punto a menos de `rad` de `(x, y)`, sea cual sea su altura.
    fn slice_all(&self, c: &Cloud, x: f64, y: f64, rad: f64) -> Vec<(f64, f64, f64)> {
        let cell = f64::from(self.cell);
        let mut out = Vec::new();
        for cx in ((x - rad) / cell).floor() as i64..=((x + rad) / cell).floor() as i64 {
            for cy in ((y - rad) / cell).floor() as i64..=((y + rad) / cell).floor() as i64 {
                if let Some(&(s, n)) = self.cells.get(&pack(cx, cy, 0)) {
                    for &i in &self.order[s as usize..(s + n) as usize] {
                        let q = c.xyz[i as usize];
                        if (f64::from(q[0]) - x).hypot(f64::from(q[1]) - y) <= rad {
                            out.push((
                                f64::from(c.z0[i as usize]),
                                f64::from(q[0]),
                                f64::from(q[1]),
                            ));
                        }
                    }
                }
            }
        }
        out
    }
}

/// Mide y cubica los árboles de una nube.
pub fn analyse(input: &Path, p: &TlsParams) -> Result<TlsResult> {
    p.check()?;
    let mut c = load(input, p)?;
    crate::check_cancel()?;
    let ground = normalize(&mut c, p)?;
    crate::check_cancel()?;
    let stems = find_stems(&c, p);
    crate::check_cancel()?;
    let idx = SliceIndex::new(&c);
    let n_sub = (p.log_len / 0.25).ceil().max(1.0);
    let step = p.log_len / n_sub;
    let measured: Vec<Option<TreeResult>> = stems
        .par_iter()
        .map(|stem| {
            if crate::is_cancelled() {
                return None;
            }
            let secs = track(&c, &idx, stem, p, step);
            if secs.iter().filter(|s| s.ok).count() < 2 {
                return None;
            }
            Some(build_tree(&c, &idx, ground.as_ref(), stem, secs, p))
        })
        .collect();
    crate::check_cancel()?;
    let mut trees = crate::stools::resolve(measured.into_iter().flatten().collect(), p);
    if p.lost_top_share > 0.0 {
        estimate_lost_tops(&mut trees, p);
    }
    trees.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    // números: los árboles en orden y las cepas en el orden en que llega su primer árbol
    let mut stools: HashMap<u32, u32> = HashMap::new();
    for (k, t) in trees.iter_mut().enumerate() {
        t.id = k as u32 + 1;
        let next = stools.len() as u32 + 1;
        t.stool = *stools.entry(t.stool).or_insert(next);
    }
    Ok(TlsResult {
        trees,
        date: c.date.clone(),
        epsg: c.epsg,
        points_read: c.read,
        voxels: c.xyz.len(),
        origin: c.origin,
    })
}

/// Un tronco que se perdió cuando aún era más grueso que el diámetro de punta, muy por debajo de la
/// cima del árbol, no se midió hasta su final: algo (follaje, un verticilo de ramas, una oclusión)
/// lo ocultó, porque un tronco no deja de ser cubicable a 8 cm y 20 m de altura. Su altura
/// comercial se estima entonces como `p.lost_top_share` de la altura del árbol (lo que espera el
/// usuario de la especie, por ejemplo 0,65 para el eucalipto), y el diámetro baja linealmente desde
/// la última sección hasta el diámetro de punta allí. Los árboles bifurcados se dejan como están.
/// Los árboles reciben la marca `ESTIMADO` en lugar de `TRUNCADO`.
fn estimate_lost_tops(trees: &mut [TreeResult], p: &TlsParams) {
    let profile_of = |t: &TreeResult| -> Vec<(f64, f64)> {
        t.sections
            .iter()
            .filter(|s| s.ok || s.d.is_finite())
            .map(|s| (s.h, s.d))
            .collect()
    };
    let ratio = p.lost_top_share.clamp(0.1, 0.95);
    for t in trees.iter_mut() {
        if !t.cubing.truncated || t.fork_h.is_some() {
            continue;
        }
        let mut profile = profile_of(t);
        let Some(&(last_h, last_d)) = profile.last() else {
            continue;
        };
        let h_com = ratio * t.height;
        // perdido cerca de la cima del árbol, de verdad es el final; perdido antes de la
        // estimación, se conserva
        if t.height - last_h <= 1.0 || h_com <= last_h + p.log_len || last_d < p.top_diam {
            continue;
        }
        profile.push((h_com, p.top_diam));
        profile.push((h_com + 0.05, p.top_diam * 0.9));
        let mut cubing = smalian(&profile, p.base_height, p.log_len, p.top_diam);
        cubing.estimated = true;
        cubing.truncated = false;
        t.cubing = cubing;
        let flags: Vec<&str> = t
            .flags
            .split(' ')
            .filter(|f| !f.is_empty() && *f != "TRUNCADO")
            .chain(std::iter::once("ESTIMADO"))
            .collect();
        t.flags = flags.join(" ");
    }
}

fn build_tree(
    c: &Cloud,
    idx: &SliceIndex,
    ground: Option<&Grid>,
    stem: &Stem,
    mut secs: Vec<Section>,
    p: &TlsParams,
) -> TreeResult {
    let mut flags = Vec::new();
    if enforce_taper(&mut secs) > 0 {
        flags.push("ENSANCHA");
    }
    // las secciones finales que quedan sin diámetro no aportan información
    while secs.last().is_some_and(|s| !s.ok) {
        secs.pop();
    }
    // los huecos cortos se rellenan a partir de las secciones de alrededor
    let ok: Vec<(f64, f64)> = secs.iter().filter(|s| s.ok).map(|s| (s.h, s.d)).collect();
    let fine = secs.windows(2).next().map_or(0.25, |w| w[1].h - w[0].h);
    let mut filled = 0;
    for s in &mut secs {
        if !s.ok {
            let k = ok.partition_point(|q| q.0 < s.h);
            if k > 0 && k < ok.len() && ok[k].0 - ok[k - 1].0 <= 1.0 + 1e-9 {
                s.d = diam_at(&ok, s.h).unwrap_or(f64::NAN);
                filled += 1;
            }
        }
    }
    let profile: Vec<(f64, f64)> = secs
        .iter()
        .filter(|s| s.ok || s.d.is_finite())
        .map(|s| (s.h, s.d))
        .collect();
    let cubing = smalian(&profile, p.base_height, p.log_len, p.top_diam);
    let dbh =
        diam_at(&profile, 1.3).filter(|_| secs.iter().any(|s| s.ok && (s.h - 1.3).abs() <= 0.35));
    if dbh.is_none() {
        flags.push("SIN_DIAM");
    }
    // un perfil que todavía se ensancha por encima de la altura del pecho (la envolvente empieza en
    // las secciones que encuentra, que pueden ser las equivocadas) se deja para revisarlo a ojo
    if !flags.contains(&"ENSANCHA")
        && dbh.is_some_and(|d| profile.iter().any(|&(h, x)| h > 1.3 && x > d * 1.2 + 0.01))
    {
        flags.push("ENSANCHA");
    }
    if cubing.truncated {
        flags.push("TRUNCADO");
    }
    if cubing.logs.is_empty() {
        flags.push("SIN_VOLUMEN");
    }
    if filled > 0 {
        flags.push("INTERPOLADO");
    }
    let first_ok = secs.iter().find(|s| s.ok);
    let last_ok = secs.iter().rev().find(|s| s.ok);
    let (tilt, slope) = match (first_ok, last_ok) {
        (Some(a), Some(b)) if b.h - a.h > 0.5 => {
            let s = [(b.x - a.x) / (b.h - a.h), (b.y - a.y) / (b.h - a.h)];
            (s[0].hypot(s[1]).atan().to_degrees(), s)
        }
        _ => (
            stem.slope[0].hypot(stem.slope[1]).atan().to_degrees(),
            stem.slope,
        ),
    };
    let height = tree_height(c, idx, &secs, slope);
    if tilt > 25.0 {
        flags.push("INCLINADO");
    }
    let (bx, by) = first_ok.map_or((stem.c[0], stem.c[1]), |s| {
        (
            s.x - slope[0] * (s.h - p.base_height),
            s.y - slope[1] * (s.h - p.base_height),
        )
    });
    let gz = ground.and_then(|g| g.sample(bx, by)).map_or(0.0, f64::from);
    let span = last_ok.map_or(0.0, |l| l.h) - p.base_height;
    let total = (span / fine.max(1e-6)).round().max(1.0);
    let good = secs.iter().filter(|s| s.ok).count() as f64;
    TreeResult {
        id: 0,
        x: bx,
        y: by,
        ground_z: gz,
        dbh,
        height,
        tilt,
        d_base: diam_at(&profile, p.base_height).unwrap_or(f64::NAN),
        points: secs.iter().map(|s| s.points).sum(),
        quality: (good / total).min(1.0) * 100.0,
        cubing,
        sections: secs,
        flags: flags.join(" "),
        slope,
        stool: 0,
        sprouts: 1,
        fork_h: None,
    }
}

// ----------------------------------------------------------------------------- outputs

impl TlsResult {
    fn abs(&self, t: &TreeResult) -> (f64, f64, f64) {
        (
            t.x + self.origin[0],
            t.y + self.origin[1],
            t.ground_z + self.origin[2],
        )
    }
}

fn tree_table(r: &TlsResult, p: &TlsParams) -> (Vec<[f64; 3]>, Table) {
    let cols: Vec<(String, ColKind)> = vec![
        ("TreeID".into(), ColKind::Num(6, 0)),
        ("X".into(), ColKind::Num(12, 3)),
        ("Y".into(), ColKind::Num(13, 3)),
        ("Z_SUELO".into(), ColKind::Num(9, 2)),
        ("DIAM_130_CM".into(), ColKind::Num(7, 1)),
        ("ALT_TOT".into(), ColKind::Num(7, 2)),
        ("ALT_LIBRE_RAMAS".into(), ColKind::Num(7, 2)),
        ("VOL_M3".into(), ColKind::Num(9, 4)),
        ("NTROZAS".into(), ColKind::Num(4, 0)),
        ("D_BASE_CM".into(), ColKind::Num(7, 1)),
        ("D_TOP_CM".into(), ColKind::Num(7, 1)),
        ("AB_M2".into(), ColKind::Num(8, 4)),
        ("INCLIN".into(), ColKind::Num(5, 1)),
        ("PUNTOS".into(), ColKind::Num(8, 0)),
        ("CALIDAD".into(), ColKind::Num(5, 0)),
        ("AVISOS".into(), ColKind::Text(40)),
        (crate::vector::CAPTURE_FIELD.into(), ColKind::Text(10)),
        ("CEPA".into(), ColKind::Num(6, 0)),
        ("BROTES".into(), ColKind::Num(3, 0)),
        ("H_BIF".into(), ColKind::Num(6, 2)),
    ];
    let mut pts = Vec::new();
    let mut rows = Vec::new();
    for t in &r.trees {
        let (x, y, gz) = r.abs(t);
        pts.push([x, y, gz]);
        let dbh = t
            .dbh
            .map_or(Val::Text(String::new()), |d| Val::Num(d * 100.0));
        let ab = t
            .dbh
            .map_or(Val::Text(String::new()), |d| Val::Num(area(d)));
        let d_top = t.cubing.logs.last().map_or(f64::NAN, |l| l.d_to);
        let n = |v: f64| {
            if v.is_finite() {
                Val::Num(v)
            } else {
                Val::Text(String::new())
            }
        };
        rows.push(vec![
            Val::Int(i64::from(t.id)),
            Val::Num(x),
            Val::Num(y),
            Val::Num(gz),
            dbh,
            Val::Num(t.height),
            Val::Num((t.cubing.top_h - p.base_height).max(0.0)),
            Val::Num(t.cubing.volume),
            Val::Int(t.cubing.logs.len() as i64),
            n(t.d_base * 100.0),
            n(d_top * 100.0),
            ab,
            Val::Num(t.tilt),
            Val::Int(t.points as i64),
            Val::Num(t.quality.round()),
            Val::Text(t.flags.clone()),
            Val::Text(r.date.clone().unwrap_or_default()),
            Val::Int(i64::from(t.stool)),
            Val::Int(i64::from(t.sprouts)),
            t.fork_h.map_or(Val::Text(String::new()), Val::Num),
        ]);
    }
    (
        pts,
        Table {
            columns: cols,
            rows,
        },
    )
}

fn csv_text(t: &Table) -> String {
    let mut s = t
        .columns
        .iter()
        .map(|c| c.0.clone())
        .collect::<Vec<_>>()
        .join(",");
    s.push('\n');
    for row in &t.rows {
        let cells: Vec<String> = row
            .iter()
            .map(|v| match v {
                Val::Text(x) => x.clone(),
                Val::Num(x) => format!("{x:.4}"),
                Val::Int(x) => x.to_string(),
            })
            .collect();
        s.push_str(&cells.join(","));
        s.push('\n');
    }
    s
}

/// Escribe los árboles como un shapefile de puntos (en la base de cada tronco) y/o un CSV, y las
/// trozas de cada árbol como `<nombre>_trozas.csv`. Devuelve los archivos.
pub fn write_results(
    r: &TlsResult,
    p: &TlsParams,
    out: &Path,
    formats: crate::paths::Formats,
) -> Result<Vec<PathBuf>> {
    crate::paths::ensure_parent(out)?;
    let (pts, table) = tree_table(r, p);
    let mut files = Vec::new();
    if formats.vector {
        let shp = crate::paths::swap_ext(out, "shp");
        let prj = r.epsg.and_then(crate::vector::prj_for_epsg);
        // el nombre de una columna de shapefile tiene como mucho 10 caracteres: dos de los nombres
        // más largos reciben una forma corta
        let mut shp_table = table.clone();
        for (name, _) in &mut shp_table.columns {
            match name.as_str() {
                "DIAM_130_CM" => *name = "DIAM130_CM".into(),
                "ALT_LIBRE_RAMAS" => *name = "ALT_LIBRE".into(),
                _ => {}
            }
        }
        crate::vector::write_points(&shp, &pts, true, &shp_table, prj.as_deref())?;
        files.push(shp);
    }
    if formats.csv {
        let csv = crate::paths::swap_ext(out, "csv");
        std::fs::write(&csv, csv_text(&table))?;
        files.push(csv);
    }
    // las trozas
    let mut logs = Table {
        columns: vec![
            ("TreeID".into(), ColKind::Num(6, 0)),
            ("Troza".into(), ColKind::Num(4, 0)),
            ("H_DESDE".into(), ColKind::Num(7, 2)),
            ("H_HASTA".into(), ColKind::Num(7, 2)),
            ("D_DESDE_CM".into(), ColKind::Num(7, 1)),
            ("D_HASTA_CM".into(), ColKind::Num(7, 1)),
            ("LONG_M".into(), ColKind::Num(6, 2)),
            ("VOL_M3".into(), ColKind::Num(9, 5)),
            ("RAMA".into(), ColKind::Num(3, 0)),
        ],
        rows: Vec::new(),
    };
    for t in &r.trees {
        for (k, l) in t.cubing.logs.iter().enumerate() {
            logs.rows.push(vec![
                Val::Int(i64::from(t.id)),
                Val::Int(k as i64 + 1),
                Val::Num(l.from),
                Val::Num(l.to),
                Val::Num(l.d_from * 100.0),
                Val::Num(l.d_to * 100.0),
                Val::Num(l.to - l.from),
                Val::Num(l.volume),
                Val::Int(i64::from(l.branch)),
            ]);
        }
    }
    let lp = out.with_file_name(format!("{}_trozas.csv", crate::paths::stem(out)));
    std::fs::write(&lp, csv_text(&logs))?;
    files.push(lp);
    Ok(files)
}

/// Nubes por árbol escritas en una sola lectura de la entrada.
const OPEN_CLOUDS: usize = 150;

/// Asigna cada punto de la nube a su eje de tronco más cercano (a menos de `assign_dist`, por
/// encima del suelo) y escribe una nube por árbol en `dir`; devuelve cuántas nubes y puntos.
pub fn write_tree_clouds(
    input: &Path,
    r: &TlsResult,
    p: &TlsParams,
    dir: &Path,
) -> Result<(usize, u64)> {
    if r.trees.is_empty() {
        return Ok((0, 0));
    }
    std::fs::create_dir_all(dir)?;
    let template = Reader::open(input)?;
    let ext = input
        .extension()
        .map_or("las".to_string(), |e| e.to_string_lossy().into_owned());
    let o = r.origin;
    let cell = p.assign_dist.max(1.0);
    let mut grid: FxMap<u64, Vec<usize>> = FxMap::default();
    for (k, t) in r.trees.iter().enumerate() {
        grid.entry(pack(
            (t.x / cell).floor() as i64,
            (t.y / cell).floor() as i64,
            0,
        ))
        .or_default()
        .push(k);
    }
    let rec_len = usize::from(template.header.record_len);
    let mut total = 0u64;
    let mut n = 0;
    // un archivo por árbol está abierto mientras se lee la nube; el sistema limita cuántos archivos
    // puede tener abiertos un programa (256 en macOS), así que los árboles van por tandas, una
    // lectura de la nube por cada una
    for lo in (0..r.trees.len()).step_by(OPEN_CLOUDS) {
        let hi = (lo + OPEN_CLOUDS).min(r.trees.len());
        let mut writers: HashMap<usize, Writer> = HashMap::new();
        template.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let Some(pt) = template.decode(rec) else {
                    continue;
                };
                if pt.is_noise() {
                    continue;
                }
                let (x, y, z) = (pt.x - o[0], pt.y - o[1], pt.z);
                let (cx, cy) = ((x / cell).floor() as i64, (y / cell).floor() as i64);
                let mut best: Option<(f64, usize)> = None;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        let Some(list) = grid.get(&pack(cx + dx, cy + dy, 0)) else {
                            continue;
                        };
                        for &k in list {
                            let t = &r.trees[k];
                            let gz = t.ground_z + o[2];
                            let h = z - gz;
                            if h < 0.2 {
                                continue;
                            }
                            // distancia al eje a esta altura
                            let (ax, ay) = (
                                t.x + t.slope[0] * (h - p.base_height),
                                t.y + t.slope[1] * (h - p.base_height),
                            );
                            let d = (x - ax).hypot(y - ay);
                            if d <= p.assign_dist && best.is_none_or(|b| d < b.0) {
                                best = Some((d, k));
                            }
                        }
                    }
                }
                if let Some((_, k)) = best
                    && (lo..hi).contains(&k)
                {
                    if let std::collections::hash_map::Entry::Vacant(v) = writers.entry(k) {
                        let path = dir.join(format!("arbol_{:04}.{ext}", r.trees[k].id));
                        v.insert(Writer::create_with(
                            &path,
                            &template,
                            &WriterOptions::small(),
                        )?);
                    }
                    if let Some(w) = writers.get_mut(&k) {
                        w.write(rec)?;
                        total += 1;
                    }
                }
            }
            Ok(())
        })?;
        n += writers.len();
        for (_, w) in writers {
            w.finish()?;
        }
    }
    Ok((n, total))
}

/// Cuántos puntos dio `classify_stems` a los troncos, a las ramas y dejó como estaban.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StemReport {
    pub stem: u64,
    pub branch: u64,
    pub other: u64,
}

/// El tronco medido de un árbol en función de la altura: el centro y el radio de sus secciones.
struct StemProfile {
    /// Altura sobre el suelo, centro (coordenadas relativas) y radio de cada sección buena.
    h: Vec<f64>,
    c: Vec<[f64; 2]>,
    r: Vec<f64>,
    /// Altura del árbol: se toma que el tronco sigue hasta ella por encima de la última sección.
    top: f64,
    /// Deriva horizontal y cambio de radio por metro por encima de la última sección.
    drift: [f64; 2],
    taper: f64,
}

impl StemProfile {
    fn of(t: &TreeResult) -> Option<Self> {
        let mut ok: Vec<&Section> = t.sections.iter().filter(|s| s.ok && s.d > 0.0).collect();
        ok.sort_by(|a, b| a.h.total_cmp(&b.h));
        let last = *ok.last()?;
        // estrechamiento en el último metro de secciones, sin crecer nunca
        let near: Vec<&&Section> = ok.iter().filter(|s| last.h - s.h <= 1.0).collect();
        let taper = match (near.first(), near.last()) {
            (Some(a), Some(b)) if b.h - a.h > 0.2 => {
                ((b.d - a.d) / 2.0 / (b.h - a.h)).clamp(-0.02, -0.005)
            }
            _ => -0.01,
        };
        Some(Self {
            h: ok.iter().map(|s| s.h).collect(),
            c: ok.iter().map(|s| [s.x, s.y]).collect(),
            r: ok.iter().map(|s| s.d / 2.0).collect(),
            top: t.height.max(last.h),
            drift: t.slope,
            taper,
        })
    }

    /// Centro y radio a la altura `h`, o `None` por encima del tronco.
    fn at(&self, h: f64) -> Option<([f64; 2], f64)> {
        let n = self.h.len();
        let last = self.h[n - 1];
        if h > self.top {
            return None;
        }
        if h >= last {
            let dh = h - last;
            let r = (self.r[n - 1] + self.taper * dh).max(0.03);
            return Some((
                [
                    self.c[n - 1][0] + self.drift[0] * dh,
                    self.c[n - 1][1] + self.drift[1] * dh,
                ],
                r,
            ));
        }
        let k = self.h.partition_point(|v| *v <= h);
        if k == 0 {
            // por debajo de la primera sección: su círculo, movido a lo largo de la inclinación
            let dh = h - self.h[0];
            return Some((
                [
                    self.c[0][0] + self.drift[0] * dh,
                    self.c[0][1] + self.drift[1] * dh,
                ],
                self.r[0],
            ));
        }
        let (a, b) = (k - 1, k);
        let u = (h - self.h[a]) / (self.h[b] - self.h[a]).max(1e-9);
        let l = |x: f64, y: f64| x + (y - x) * u;
        Some((
            [l(self.c[a][0], self.c[b][0]), l(self.c[a][1], self.c[b][1])],
            l(self.r[a], self.r[b]),
        ))
    }
}

/// Escribe la nube con los puntos de los troncos en la clase [`crate::classes::STEM`] y los demás
/// puntos cercanos a un tronco (hasta `assign_dist` de su eje, por encima de 0,2 m) en
/// [`crate::classes::BRANCH`]; el resto conserva su clase. Un punto pertenece al tronco cuando está
/// dentro del círculo medido de su árbol a esa altura, más un margen de un vóxel. Las alturas donde
/// no se midió el tronco toman el círculo entre las secciones de alrededor y, por encima de la
/// última, el tronco sigue hasta la cima del árbol a lo largo de su inclinación, adelgazando de 0,5
/// a 2 cm por metro de radio.
pub fn classify_stems(
    input: &Path,
    r: &TlsResult,
    p: &TlsParams,
    output: &Path,
) -> Result<StemReport> {
    use rayon::prelude::*;
    if input == output {
        return Err(Error::Param(
            "la nube clasificada debe ir a un archivo nuevo, no sobre el original".into(),
        ));
    }
    crate::paths::ensure_parent(output)?;
    let reader = Reader::open(input)?;
    let (old_format, rec_len) = (reader.header.format, usize::from(reader.header.record_len));
    // las clases por encima de 31 necesitan un byte de clase completo: los archivos antiguos se
    // escriben como LAS 1.4
    let upgrade = if old_format < 6 {
        Some(crate::las::upgraded_format(old_format).ok_or_else(|| {
            Error::Unsupported(format!(
                "el formato de punto {old_format} no admite clases por encima de 31: convierte antes la nube"
            ))
        })?)
    } else {
        None
    };
    let template = match upgrade {
        Some(n) => Reader::open(input)?.upgraded(n),
        None => Reader::open(input)?,
    };
    let format = template.header.format;
    let mut w = Writer::create(output, &template)?;
    let o = r.origin;
    let cell = p.assign_dist.max(1.0);
    let margin = p.voxel.max(0.02);
    let profiles: Vec<Option<StemProfile>> = r.trees.iter().map(StemProfile::of).collect();
    let mut grid: FxMap<u64, Vec<usize>> = FxMap::default();
    for (k, t) in r.trees.iter().enumerate() {
        grid.entry(pack(
            (t.x / cell).floor() as i64,
            (t.y / cell).floor() as i64,
            0,
        ))
        .or_default()
        .push(k);
    }
    // 0: no tocado, 1: tronco, 2: rama
    let kind = |rec: &[u8]| -> u8 {
        let Some(pt) = reader.decode(rec) else {
            return 0;
        };
        if pt.is_noise() {
            return 0;
        }
        let (x, y, z) = (pt.x - o[0], pt.y - o[1], pt.z);
        let (cx, cy) = ((x / cell).floor() as i64, (y / cell).floor() as i64);
        let mut near = false;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(list) = grid.get(&pack(cx + dx, cy + dy, 0)) else {
                    continue;
                };
                for &k in list {
                    let t = &r.trees[k];
                    let h = z - (t.ground_z + o[2]);
                    if h < 0.2 {
                        continue;
                    }
                    let (ax, ay) = (
                        t.x + t.slope[0] * (h - p.base_height),
                        t.y + t.slope[1] * (h - p.base_height),
                    );
                    if (x - ax).hypot(y - ay) > p.assign_dist {
                        continue;
                    }
                    near = true;
                    if let Some(prof) = &profiles[k]
                        && let Some((c, rad)) = prof.at(h)
                        && (x - c[0]).hypot(y - c[1]) <= rad + margin
                    {
                        return 1;
                    }
                }
            }
        }
        if near { 2 } else { 0 }
    };
    let mut rep = StemReport::default();
    let mut conv = Vec::new();
    reader.scan_raw(|bytes| {
        let recs: Vec<&[u8]> = bytes.chunks_exact(rec_len).collect();
        let kinds: Vec<u8> = recs.par_iter().map(|rec| kind(rec)).collect();
        for (rec, k) in recs.iter().zip(kinds) {
            let class = match k {
                1 => {
                    rep.stem += 1;
                    Some(crate::classes::STEM)
                }
                2 => {
                    rep.branch += 1;
                    Some(crate::classes::BRANCH)
                }
                _ => {
                    rep.other += 1;
                    None
                }
            };
            if let Some(new) = upgrade {
                crate::las::upgrade_record(old_format, new, rec, &mut conv);
                if let Some(c) = class {
                    crate::las::set_class(format, &mut conv, c);
                }
                w.write(&conv)?;
            } else if let Some(c) = class {
                conv.clear();
                conv.extend_from_slice(rec);
                crate::las::set_class(format, &mut conv, c);
                w.write(&conv)?;
            } else {
                w.write(rec)?;
            }
        }
        Ok(())
    })?;
    w.finish()?;
    Ok(rep)
}

/// Todo lo que hace una ejecución: medir, cubicar, escribir los resultados y, si se pide, las nubes
/// por árbol.
#[derive(Clone, Debug)]
pub struct TlsJob {
    pub input: PathBuf,
    /// La tabla de árboles (`.shp` o `.csv`).
    pub output: PathBuf,
    pub params: TlsParams,
    pub formats: crate::paths::Formats,
    pub tree_clouds: bool,
    /// Escribe también la nube con los puntos de los troncos y de las ramas clasificados.
    pub stem_classes: bool,
}

impl TlsJob {
    /// La carpeta de las nubes por árbol: `<nombre de salida>_arboles`.
    pub fn clouds_dir(&self) -> PathBuf {
        self.output
            .with_file_name(format!("{}_arboles", crate::paths::stem(&self.output)))
    }
}

impl TlsJob {
    /// La nube con troncos y ramas clasificados: `<nombre de salida>_troncos` con el formato de la
    /// entrada.
    pub fn stems_path(&self) -> PathBuf {
        let ext = self
            .input
            .extension()
            .map_or("las".to_string(), |e| e.to_string_lossy().into_owned());
        self.output.with_file_name(format!(
            "{}_troncos.{ext}",
            crate::paths::stem(&self.output)
        ))
    }
}

pub fn run(job: &TlsJob) -> Result<String> {
    let r = analyse(&job.input, &job.params)?;
    let files = write_results(&r, &job.params, &job.output, job.formats)?;
    let mut text = summary(&r, &job.output);
    if r.trees.is_empty() {
        text.push_str(
            ". No se encontró ningún tronco: revisa la franja de búsqueda y la altura de los datos",
        );
    }
    if job.tree_clouds {
        let (n, pts) = write_tree_clouds(&job.input, &r, &job.params, &job.clouds_dir())?;
        text.push_str(&format!(
            "
{n} nubes por árbol ({pts} puntos): {}",
            job.clouds_dir().display()
        ));
    }
    if job.stem_classes && !r.trees.is_empty() {
        let rep = classify_stems(&job.input, &r, &job.params, &job.stems_path())?;
        text.push_str(&format!(
            "
Troncos (clase {}): {} puntos; ramas (clase {}): {} puntos: {}",
            crate::classes::STEM,
            rep.stem,
            crate::classes::BRANCH,
            rep.branch,
            job.stems_path().display()
        ));
    }
    let _ = files;
    Ok(text)
}

/// Una línea sobre la ejecución.
pub fn summary(r: &TlsResult, out: &Path) -> String {
    let vol: f64 = r.trees.iter().map(|t| t.cubing.volume).sum();
    let stools = r
        .trees
        .iter()
        .filter(|t| t.sprouts > 1)
        .map(|t| t.stool)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let forked = r.trees.iter().filter(|t| t.fork_h.is_some()).count();
    let truncated: Vec<&TreeResult> = r.trees.iter().filter(|t| t.cubing.truncated).collect();
    let mut extra = String::new();
    if stools > 0 {
        extra.push_str(&format!(", {stools} cepas con varios brotes"));
    }
    if forked > 0 {
        extra.push_str(&format!(", {forked} bifurcados"));
    }
    let mut text = format!(
        "{} individuos con volumen estimado{extra}, {:.3} m³ en total ({} puntos leídos, {} vóxeles): {}",
        r.trees.len(),
        vol,
        r.points_read,
        r.voxels,
        out.display()
    );
    let estimated: Vec<&TreeResult> = r.trees.iter().filter(|t| t.cubing.estimated).collect();
    if !estimated.is_empty() {
        let part: f64 = estimated.iter().map(|t| t.cubing.volume).sum();
        text.push_str(&format!(
            "\n  Aviso: en {} de {} individuos (ESTIMADO) el tronco se perdió estando aún grueso y lejos de la copa; su altura libre de ramas se estima con la proporción de la altura total que se ha indicado, y su volumen ({part:.3} m³) incluye ese tramo estimado",
            estimated.len(),
            r.trees.len()
        ));
    }
    if !truncated.is_empty() {
        let part: f64 = truncated.iter().map(|t| t.cubing.volume).sum();
        text.push_str(&format!(
            "\n  Aviso: en {} de {} individuos (TRUNCADO) el tronco no se pudo seguir hasta el diámetro en punta: \
             su volumen ({part:.3} m³) es solo el del tramo medido, y el total se queda corto",
            truncated.len(),
            r.trees.len()
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stem_that_widens_upwards_loses_the_wide_sections() {
        // el tronco de 8-9 cm recoge una rama a partir de 2,6 m de altura: 12, 14, 16 cm
        let mut secs: Vec<Section> = [
            (0.1, 0.089),
            (0.6, 0.088),
            (1.1, 0.088),
            (1.6, 0.080),
            (2.1, 0.086),
            (2.6, 0.119),
            (3.1, 0.144),
            (3.6, 0.162),
        ]
        .iter()
        .map(|&(h, d)| sec(h, d))
        .collect();
        assert_eq!(enforce_taper(&mut secs), 3);
        assert!(secs[..5].iter().all(|s| s.ok));
        assert!(secs[5..].iter().all(|s| !s.ok && s.d.is_nan()));
        // un ensanchamiento de la base por debajo de un metro es el pie de la raíz, no una rama
        let mut flare: Vec<Section> = [(0.1, 0.40), (0.6, 0.31), (1.1, 0.30), (1.6, 0.29)]
            .iter()
            .map(|&(h, d)| sec(h, d))
            .collect();
        assert_eq!(enforce_taper(&mut flare), 0);
        // ni lo es el ruido de unos milímetros en un tronco de tamaño constante
        let mut even: Vec<Section> = (0..12)
            .map(|k| sec(0.5 * f64::from(k), 0.30 + 0.01 * f64::from(k % 2)))
            .collect();
        assert_eq!(enforce_taper(&mut even), 0);
        // ni puede un tronco ensancharse un poco cada vez
        let mut creep: Vec<Section> = (0..10)
            .map(|k| sec(1.0 + 0.5 * f64::from(k), 0.080 + 0.008 * f64::from(k)))
            .collect();
        assert!(enforce_taper(&mut creep) >= 6);
        assert!(
            creep
                .iter()
                .filter(|s| s.ok)
                .all(|s| s.d <= 0.08 * 1.08 + 0.01 + 0.008)
        );
    }

    #[test]
    fn smalian_is_exact_for_a_cylinder() {
        let prof: Vec<(f64, f64)> = (0..=40).map(|k| (f64::from(k) * 0.25, 0.3)).collect();
        let c = smalian(&prof, 0.1, 0.5, 0.07);
        let exact = area(0.3) * (10.0 - 0.1);
        assert!((c.volume - exact).abs() < 1e-9, "{} {exact}", c.volume);
        assert!(c.truncated);
        assert_eq!(c.logs.len(), 20);
    }

    #[test]
    fn smalian_cuts_a_cone_at_the_top_diameter() {
        // d baja de 0,4 en 0 a 0 en 20: es 0,07 en 16,5
        let prof: Vec<(f64, f64)> = (0..=76)
            .map(|k| {
                (
                    f64::from(k) * 0.25,
                    0.4 * (1.0 - f64::from(k) * 0.25 / 20.0),
                )
            })
            .collect();
        let c = smalian(&prof, 0.1, 0.5, 0.07);
        assert!((c.top_h - 16.5).abs() < 0.01, "{}", c.top_h);
        assert!(!c.truncated);
        // volumen del tronco de cono entre 0,1 y 16,5 m
        let (d0, d1) = (0.4 * (1.0 - 0.1 / 20.0), 0.07);
        let l = 16.4;
        let exact = std::f64::consts::PI * l / 12.0 * (d0 * d0 + d0 * d1 + d1 * d1);
        assert!(
            (c.volume - exact).abs() / exact < 0.01,
            "{} {exact}",
            c.volume
        );
        let last = c.logs.last();
        assert!(last.is_some_and(|l| (l.d_to - 0.07).abs() < 1e-9));
    }

    #[test]
    fn circle_fit_recovers_a_circle() {
        let pts: Vec<[f32; 2]> = (0..60)
            .map(|k| {
                let a = f64::from(k) / 60.0 * std::f64::consts::TAU;
                [(3.0 + 0.2 * a.cos()) as f32, (5.0 + 0.2 * a.sin()) as f32]
            })
            .collect();
        let (x, y, r) = fit_circle(&pts).unwrap_or_default();
        assert!((x - 3.0).abs() < 1e-4 && (y - 5.0).abs() < 1e-4 && (r - 0.2).abs() < 1e-4);
    }

    fn sec(h: f64, d: f64) -> Section {
        Section {
            h,
            x: 0.0,
            y: 0.0,
            d,
            ok: d.is_finite(),
            points: 30,
            sectors: 12,
        }
    }

    #[test]
    fn a_rejected_section_loses_its_diameter() {
        // 0,50 m entre 0,30 y 0,28 no concuerda con ninguno de los dos, y las secciones que podrían
        // haber ocupado su lugar fallan: el metro y medio de alrededor es un hueco que el relleno
        // deja en paz
        let mut s = vec![
            sec(1.0, 0.30),
            sec(1.5, 0.50),
            sec(1.75, f64::NAN),
            sec(2.5, f64::NAN),
            sec(3.0, 0.28),
        ];
        reject_outliers(&mut s);
        assert!(!s[1].ok && s[1].d.is_nan());
        assert!(s[0].ok && s[0].d == 0.30 && s[4].ok && s[4].d == 0.28);
        // no queda nada de la sección que pueda tomarse como medida
        assert!(s.iter().filter(|x| x.ok || x.d.is_finite()).count() == 2);
    }

    #[test]
    fn rejections_are_decided_on_the_diameters_as_measured() {
        // 0,50 y luego 0,31: si rechazar la primera borrara su diámetro antes de comparar la
        // segunda, la segunda se compararía con NaN. Las dos se juzgan sobre los originales.
        let mut s = vec![
            sec(1.0, 0.30),
            sec(1.25, 0.50),
            sec(1.5, 0.31),
            sec(1.75, 0.30),
        ];
        reject_outliers(&mut s);
        assert!(!s[1].ok);
        assert!(s[2].ok && s[2].d == 0.31);
    }

    /// Vóxeles de un anillo de radio `r` centrado en `(cx, cy)`, cada 2 cm, en dos capas.
    fn ring(cx: f64, cy: f64, r: f64) -> Vec<[f32; 2]> {
        let n = (std::f64::consts::TAU * r / 0.02).round() as usize;
        (0..n * 5)
            .map(|k| {
                let a = (k % n) as f64 * std::f64::consts::TAU / n as f64;
                [(cx + r * a.cos()) as f32, (cy + r * a.sin()) as f32]
            })
            .collect()
    }

    #[test]
    fn circles_of_stems_that_touch_are_found_one_by_one() {
        let p = TlsParams {
            voxel: 0.02,
            ..TlsParams::default()
        };
        for r in [0.08, 0.12, 0.15, 0.25] {
            let found = circles_in(&ring(1.0, 2.0, r), (1.0, 2.0), f64::INFINITY, &p, None, 6);
            assert_eq!(found.len(), 1, "r = {r}");
            assert!(
                (found[0].2 - 2.0 * r).abs() < 0.01,
                "r = {r}: {:?}",
                found[0]
            );
        }
        // two sprouts 35 cm apart
        let mut two = ring(0.0, 0.0, 0.08);
        two.extend(ring(0.35, 0.0, 0.08));
        assert_eq!(
            circles_in(&two, (0.2, 0.0), f64::INFINITY, &p, None, 6).len(),
            2
        );
    }

    #[test]
    fn principal_axis_of_a_line() {
        let v = principal([[0.01, 0.0, 0.0], [0.0, 0.01, 0.0], [0.0, 0.0, 5.0]]);
        assert!(v[2].abs() > 0.999);
    }

    #[test]
    fn stem_profile_interpolates_between_sections_and_goes_on_above() {
        let prof = StemProfile {
            h: vec![1.0, 3.0],
            c: vec![[0.0, 0.0], [0.2, 0.0]],
            r: vec![0.10, 0.08],
            top: 6.0,
            drift: [0.1, 0.0],
            taper: -0.01,
        };
        // a medio camino entre las secciones
        let (c, r) = prof.at(2.0).unwrap_or_default();
        assert!((c[0] - 0.1).abs() < 1e-9 && (r - 0.09).abs() < 1e-9);
        // dos metros por encima de la última: deriva a lo largo de la inclinación y se adelgaza
        let (c, r) = prof.at(5.0).unwrap_or_default();
        assert!((c[0] - 0.4).abs() < 1e-9 && (r - 0.06).abs() < 1e-9);
        // por encima del árbol: no hay tronco
        assert!(prof.at(6.5).is_none());
    }

    fn stem_lost_at(last_h: f64, height: f64, p: &TlsParams) -> TreeResult {
        let sections: Vec<Section> = (0..)
            .map(|k| 0.1 + 0.25 * f64::from(k))
            .take_while(|h| *h <= last_h)
            .map(|h| Section {
                h,
                x: 0.0,
                y: 0.0,
                d: 0.20 - 0.004 * h,
                ok: true,
                points: 30,
                sectors: 12,
            })
            .collect();
        let profile: Vec<(f64, f64)> = sections.iter().map(|s| (s.h, s.d)).collect();
        TreeResult {
            id: 1,
            x: 0.0,
            y: 0.0,
            ground_z: 0.0,
            dbh: Some(0.19),
            height,
            tilt: 0.0,
            cubing: smalian(&profile, p.base_height, p.log_len, p.top_diam),
            sections,
            points: 0,
            quality: 100.0,
            d_base: 0.2,
            flags: "TRUNCADO".into(),
            slope: [0.0, 0.0],
            stool: 1,
            sprouts: 1,
            fork_h: None,
        }
    }

    #[test]
    fn a_stem_lost_thick_and_far_below_the_top_gets_the_expected_merchantable_height() {
        let p = TlsParams {
            lost_top_share: 0.65,
            ..TlsParams::default()
        };
        // perdido a 4 m con 18 cm en un árbol de 20 m: su altura comercial es 13 m
        let mut trees = vec![stem_lost_at(4.0, 20.0, &p), stem_lost_at(19.5, 20.0, &p)];
        assert!(trees[0].cubing.truncated);
        let before = trees[0].cubing.volume;
        estimate_lost_tops(&mut trees, &p);
        assert!(trees[0].cubing.estimated && !trees[0].cubing.truncated);
        assert!((trees[0].cubing.top_h - 13.0).abs() < 0.1);
        assert!(trees[0].cubing.volume > before);
        assert!(trees[0].flags.contains("ESTIMADO") && !trees[0].flags.contains("TRUNCADO"));
        // perdido a menos de un metro de la cima del árbol: ahí es donde termina el tronco
        assert!(trees[1].cubing.truncated && !trees[1].cubing.estimated);
    }
}
