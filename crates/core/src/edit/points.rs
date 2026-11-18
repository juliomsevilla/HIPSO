//! Un conjunto de puntos dado por sus coordenadas: lo que guarda la selección de la varita mágica.
//!
//! La varita decide mirando a los vecinos de un punto, y al guardar cada punto del archivo se
//! evalúa solo, sin ellos. Por eso la edición lleva el conjunto ya calculado, y un punto cumple si
//! sus coordenadas están en él. Las coordenadas se redondean al centímetro respecto de un origen
//! (la nube mostrada guarda `f32`, así que no coinciden al último decimal con las del archivo): un
//! punto que cae a menos de un milímetro de una arista del redondeo se busca también en la celda
//! de al lado.

/// Lado de la celda del redondeo, en metros.
const STEP: f64 = 0.01;
/// Fracción de celda por debajo o por encima de la cual se mira también la celda vecina (1 mm).
const EDGE: f64 = 0.1;

#[derive(Clone, PartialEq)]
pub struct PointSet {
    origin: [f64; 3],
    /// Celdas ocupadas, ordenadas y sin repetir.
    keys: Vec<[i32; 3]>,
    lo: [i32; 3],
    hi: [i32; 3],
}

impl std::fmt::Debug for PointSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PointSet({} celdas)", self.keys.len())
    }
}

impl PointSet {
    /// El conjunto de los puntos dados, con `origin` como referencia del redondeo.
    pub fn new(origin: [f64; 3], pts: impl IntoIterator<Item = [f64; 3]>) -> Self {
        let keys = pts
            .into_iter()
            .map(|p| {
                let k = |a: usize| ((p[a] - origin[a]) / STEP).round() as i32;
                [k(0), k(1), k(2)]
            })
            .collect();
        Self::from_keys(origin, keys)
    }

    /// Desde celdas ya redondeadas (lo que se lee de un archivo de ediciones).
    pub fn from_keys(origin: [f64; 3], mut keys: Vec<[i32; 3]>) -> Self {
        keys.sort_unstable();
        keys.dedup();
        let mut lo = [i32::MAX; 3];
        let mut hi = [i32::MIN; 3];
        for k in &keys {
            for a in 0..3 {
                lo[a] = lo[a].min(k[a]);
                hi[a] = hi[a].max(k[a]);
            }
        }
        Self { origin, keys, lo, hi }
    }

    pub fn origin(&self) -> [f64; 3] {
        self.origin
    }

    pub fn keys(&self) -> &[[i32; 3]] {
        &self.keys
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Si hay un punto del conjunto en `(x, y, z)`.
    pub fn contains(&self, x: f64, y: f64, z: f64) -> bool {
        if self.keys.is_empty() {
            return false;
        }
        // por cada eje, la celda del punto y, si cae junto a una arista, la de al lado
        let mut cand = [[0i32; 2]; 3];
        let mut n = [1usize; 3];
        for (a, v) in [x, y, z].into_iter().enumerate() {
            let f = (v - self.origin[a]) / STEP;
            if !f.is_finite() || f.abs() > f64::from(i32::MAX - 2) {
                return false;
            }
            // las celdas se redondean al más cercano: la arista está a media celda del centro
            let base = f.round();
            let off = f - base;
            let base = base as i32;
            cand[a][0] = base;
            if off > 0.5 - EDGE {
                cand[a][1] = base + 1;
                n[a] = 2;
            } else if off < EDGE - 0.5 {
                cand[a][1] = base - 1;
                n[a] = 2;
            }
            if base < self.lo[a] - 1 || base > self.hi[a] + 1 {
                return false;
            }
        }
        for i in 0..n[0] {
            for j in 0..n[1] {
                for k in 0..n[2] {
                    let key = [cand[0][i], cand[1][j], cand[2][k]];
                    if self.keys.binary_search(&key).is_ok() {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_points_despite_f32_noise() {
        let origin = [440_000.0, 4_400_000.0, 600.0];
        let pts: Vec<[f64; 3]> = (0..1000)
            .map(|i| {
                let t = f64::from(i);
                [origin[0] + t * 0.137, origin[1] + t * 0.0713, origin[2] + (t * 0.3).sin()]
            })
            .collect();
        let set = PointSet::new(origin, pts.iter().copied());
        for p in &pts {
            // el ruido de una coordenada guardada como f32 relativa al origen
            let noisy = |a: usize| origin[a] + f64::from((p[a] - origin[a]) as f32) + 0.0003;
            assert!(set.contains(noisy(0), noisy(1), noisy(2)), "{p:?}");
            let noisy = |a: usize| origin[a] + f64::from((p[a] - origin[a]) as f32) - 0.0003;
            assert!(set.contains(noisy(0), noisy(1), noisy(2)), "{p:?}");
        }
        assert!(!set.contains(origin[0] + 5.0, origin[1], origin[2]));
        assert!(!set.contains(origin[0] + 0.137 + 0.05, origin[1] + 0.0713, origin[2] + 0.29));
    }
}
