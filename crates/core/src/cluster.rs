//! Lo que comparten las herramientas que agrupan cosas: la unión-búsqueda que las fusiona, el
//! encadenamiento de puntos que están a una distancia unos de otros y la mediana.

use std::collections::HashMap;

/// Unión-búsqueda sobre `n` elementos (índices guardados como `u32`: se usa en rásteres de cientos
/// de millones de celdas). La raíz de un conjunto es su menor índice, de modo que los grupos salen
/// igual sea cual sea el orden en que se unieron los pares.
pub struct Dsu(Vec<u32>);

impl Dsu {
    pub fn new(n: usize) -> Self {
        assert!(u32::try_from(n).is_ok(), "too many items for a union-find");
        Self((0..n as u32).collect())
    }

    /// Un elemento nuevo, solo en su conjunto; devuelve su índice.
    pub fn push(&mut self) -> usize {
        let i = u32::try_from(self.0.len()).expect("too many items for a union-find");
        self.0.push(i);
        i as usize
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn find(&mut self, i: usize) -> usize {
        let mut x = i as u32;
        while self.0[x as usize] != x {
            let p = self.0[x as usize];
            self.0[x as usize] = self.0[p as usize];
            x = self.0[x as usize];
        }
        x as usize
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b) as u32;
        }
    }
}

/// El grupo de cada uno de `n` puntos (dados por `at`): dos puntos a menos de `link` uno del otro
/// están en el mismo, y también todo lo encadenado a ellos. `zscale` pesa las alturas frente a las
/// distancias en planta (1: la distancia en el espacio; 0: solo en planta; entre medias, dos puntos
/// separados `link / zscale` en vertical siguen enlazados). Un grupo se nombra por su primer punto.
pub fn link_labels(
    n: usize,
    at: impl Fn(usize) -> [f64; 3] + Sync,
    link: f64,
    zscale: f64,
) -> Vec<usize> {
    let key = |p: &[f64; 3]| -> (i64, i64, i64) {
        (
            (p[0] / link).floor() as i64,
            (p[1] / link).floor() as i64,
            (p[2] * zscale / link).floor() as i64,
        )
    };
    let mut cells: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for j in 0..n {
        cells.entry(key(&at(j))).or_default().push(j);
    }
    let dz = i64::from(zscale > 0.0);
    let l2 = link * link;
    let mut dsu = Dsu::new(n);
    for j in 0..n {
        let q = at(j);
        let (cx, cy, cz) = key(&q);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for ddz in -dz..=dz {
                    for &o in cells
                        .get(&(cx + dx, cy + dy, cz + ddz))
                        .into_iter()
                        .flatten()
                    {
                        if o > j {
                            let w = at(o);
                            let d2 = (w[0] - q[0]).powi(2)
                                + (w[1] - q[1]).powi(2)
                                + ((w[2] - q[2]) * zscale).powi(2);
                            if d2 <= l2 {
                                dsu.union(j, o);
                            }
                        }
                    }
                }
            }
        }
    }
    (0..n).map(|j| dsu.find(j)).collect()
}

/// Coseno del giro en el punto del medio de tres puntos en fila (8°).
pub const STRAIGHT: f64 = 0.99;

/// Une los puntos que están en fila: tres puntos, cada uno como mucho a `max_step` y al menos a
/// `min_step` del siguiente, con el giro en el del medio por debajo de 8° y la pendiente de ambos
/// tramos (|dz| sobre la distancia) dentro de `max_slope`, están en un conjunto. Tres en fila por
/// casualidad, tan separados, apenas ocurre en una copa; los trozos de un conductor que un vuelo ve
/// aquí y allá a lo largo de su longitud están en fila. `straight` es el coseno del mayor giro.
pub fn in_a_row(
    pts: &[[f64; 3]],
    min_step: f64,
    max_step: f64,
    max_slope: f64,
    straight: f64,
) -> Dsu {
    let mut dsu = Dsu::new(pts.len());
    if pts.len() < 3 || max_step <= 0.0 {
        return dsu;
    }
    let cell = |c: &[f64; 3]| -> (i64, i64, i64) {
        (
            (c[0] / max_step).floor() as i64,
            (c[1] / max_step).floor() as i64,
            (c[2] / max_step).floor() as i64,
        )
    };
    let mut by_cell: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (j, p) in pts.iter().enumerate() {
        by_cell.entry(cell(p)).or_default().push(j);
    }
    // el paso unitario de a a b, si es un paso de una fila
    let step = |a: &[f64; 3], b: &[f64; 3]| -> Option<[f64; 3]> {
        let v = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        (d >= min_step && d <= max_step && (v[2] / d).abs() <= max_slope)
            .then(|| [v[0] / d, v[1] / d, v[2] / d])
    };
    for (j, p) in pts.iter().enumerate() {
        let (cx, cy, cz) = cell(p);
        let mut near: Vec<(usize, [f64; 3])> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &o in by_cell.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                        if let Some(u) = step(p, &pts[o]) {
                            near.push((o, u));
                        }
                    }
                }
            }
        }
        // dos puntos en lados opuestos de este, en la misma línea
        for a in 0..near.len() {
            for b in a + 1..near.len() {
                let (u, v) = (near[a].1, near[b].1);
                if -(u[0] * v[0] + u[1] * v[1] + u[2] * v[2]) >= straight {
                    dsu.union(j, near[a].0);
                    dsu.union(j, near[b].0);
                }
            }
        }
    }
    dsu
}

/// Los grupos de `labels` como listas de índices, ordenados por su primer punto.
pub fn groups(labels: &[usize]) -> Vec<Vec<usize>> {
    let mut by: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &g) in labels.iter().enumerate() {
        by.entry(g).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = by.into_values().collect();
    out.sort_by_key(|g| g[0]);
    out
}

/// Cuánto se dispersan los valores sin sus rezagados: la distancia entre los percentiles 5 y 95,
/// por 1,25 (de modo que es la anchura de una dispersión uniforme, sean cuales sean los rezagados).
/// La anchura de una torre en planta es esto, y no la caja de sus puntos, que estira lo que cuelga
/// de ella (los conductores que llegan a ella, una rama que la toca).
pub fn core_spread(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    (at(0.95) - at(0.05)) * 1.25
}

/// La mediana de `v` (la del medio superior con un recuento par); reordena `v`.
pub fn median(v: &mut [f64]) -> f64 {
    let m = v.len() / 2;
    v.select_nth_unstable_by(m, |a, b| a.total_cmp(b));
    v[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chained_points_share_a_group_and_a_far_one_does_not() {
        let p = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 0.0, 5.0],
        ];
        let at = |i: usize| p[i];
        let l = link_labels(5, at, 1.2, 1.0);
        assert_eq!(l[0], l[2]);
        assert_ne!(l[0], l[3]);
        assert_ne!(l[0], l[4], "5 m above is not linked in space");
        // solo en planta, el punto 5 m por encima se une; con las alturas pesando menos, también
        assert_eq!(link_labels(5, at, 1.2, 0.0)[4], l[0]);
        assert_eq!(link_labels(5, at, 1.2, 0.2)[4], l[0]);
        assert_eq!(groups(&l).len(), 3);
    }

    #[test]
    fn three_points_in_a_row_join_and_a_bend_or_a_pair_do_not() {
        let p = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.1],
            [20.0, 0.0, 0.0],
            [20.0, 10.0, 0.0],
            [100.0, 100.0, 0.0],
            [110.0, 100.0, 0.0],
        ];
        let mut d = in_a_row(&p, 2.0, 15.0, 0.5, STRAIGHT);
        assert_eq!(d.find(0), d.find(2));
        assert_eq!(d.find(0), d.find(1));
        assert_ne!(d.find(3), d.find(0), "a right-angle bend is not in the row");
        assert_ne!(d.find(4), d.find(5), "two points are not a row");
    }

    #[test]
    fn the_core_spread_leaves_out_a_few_stragglers() {
        let mut v: Vec<f64> = (0..100).map(|i| f64::from(i) * 0.05).collect();
        v.push(40.0);
        v.push(-30.0);
        let w = core_spread(&v);
        assert!((5.5..7.0).contains(&w), "{w}");
        assert_eq!(core_spread(&[]), 0.0);
    }

    #[test]
    fn the_median_of_odd_and_even_counts() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 3.0);
    }
}
