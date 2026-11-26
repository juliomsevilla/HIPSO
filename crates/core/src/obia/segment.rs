//! Agrupación de celdas en objetos.
//!
//! Dos métodos: celdas conectadas cuyo valor está en un rango ([`Method::Mask`]) y la fusión
//! multirresolución de Baatz y Schäpe (2000) ([`Method::Merge`]). El segundo parte de manchas
//! planas de las capas, construye el grafo de manchas vecinas y fusiona, ronda tras ronda, cada par
//! que es el mejor ajuste uno del otro mientras el coste de fusionar (el crecimiento de la
//! heterogeneidad de las manchas en valor y, si se pide, en forma) se mantenga por debajo de la
//! escala. Fusionar solo los mejores pares mutuos mantiene paralela una ronda y hace el resultado
//! independiente de cualquier orden de visita.

use rayon::prelude::*;

use super::{Labels, Lattice, LayerStack};
use crate::cluster::Dsu;
use crate::{Error, Result, check_cancel};

#[derive(Clone, Debug, PartialEq)]
pub enum Method {
    /// Los grupos conectados de celdas cuyo valor en `layer` está entre `min` y `max`.
    Mask {
        layer: String,
        min: f32,
        max: f32,
        /// Las celdas que se tocan por una esquina van juntas.
        diagonal: bool,
    },
    Merge(MergeParams),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MergeParams {
    /// Las manchas se fusionan mientras el coste de hacerlo sea menor que `scale` al cuadrado. El
    /// coste se mide en celdas por desviaciones típicas de las capas (estandarizadas), de modo que
    /// una escala mayor da objetos mayores; empiécese alrededor de 5 a 15.
    pub scale: f64,
    /// Peso de la forma (compacidad y suavidad) frente al valor, de 0 a 0,9.
    pub shape: f64,
    /// Dentro de la forma, el peso de la compacidad frente a la suavidad, de 0 a 1.
    pub compact: f64,
    /// Un peso por capa (nombre, peso); las capas no nombradas pesan 1 y un peso de 0 deja fuera
    /// una capa.
    pub weights: Vec<(String, f32)>,
    /// Anchura, en desviaciones típicas, de los intervalos de valor que deciden qué celdas vecinas
    /// empiezan en la misma mancha: menor es más lento y más fiel.
    pub init: f64,
}

impl Default for MergeParams {
    fn default() -> Self {
        Self {
            scale: 8.0,
            shape: 0.1,
            compact: 0.5,
            weights: Vec::new(),
            init: 0.25,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SegmentParams {
    pub method: Method,
    /// Los objetos menores que esto se fusionan con su vecino más parecido (`Merge`) o se descartan
    /// (`Mask`).
    pub min_cells: u32,
}

/// Componentes conexas de las celdas para las que se cumple `valid`, uniéndose dos vecinas cuando
/// `same` lo dice. Las etiquetas van desde 1 en orden de lectura de la primera celda de cada
/// componente; 0 marca las celdas no válidas. Devuelve las etiquetas y cuántas componentes hay.
pub(crate) fn label_components(
    cols: usize,
    rows: usize,
    diagonal: bool,
    valid: impl Fn(usize) -> bool,
    same: impl Fn(usize, usize) -> bool,
) -> Result<(Vec<u32>, u32)> {
    let mut lab = vec![0u32; cols * rows];
    let mut parent = Dsu::new(1);
    for y in 0..rows {
        if y % 1024 == 0 {
            check_cancel()?;
        }
        for x in 0..cols {
            let i = y * cols + x;
            if !valid(i) {
                continue;
            }
            let mut cand = [0u32; 4];
            let mut m = 0;
            let mut look = |j: usize, lab: &[u32]| {
                if lab[j] != 0 && same(i, j) {
                    cand[m] = lab[j];
                    m += 1;
                }
            };
            if x > 0 {
                look(i - 1, &lab);
            }
            if y > 0 {
                look(i - cols, &lab);
                if diagonal {
                    if x > 0 {
                        look(i - cols - 1, &lab);
                    }
                    if x + 1 < cols {
                        look(i - cols + 1, &lab);
                    }
                }
            }
            if m == 0 {
                lab[i] = parent
                    .push()
                    .ok_or_else(|| Error::Param("demasiados objetos iniciales".into()))?
                    as u32;
            } else {
                for &c in &cand[1..m] {
                    parent.union(cand[0] as usize, c as usize);
                }
                lab[i] = parent.find(cand[0] as usize) as u32;
            }
        }
    }
    let mut dense = vec![0u32; parent.len()];
    let mut next = 0u32;
    for l in lab.iter_mut().filter(|l| **l != 0) {
        let r = parent.find(*l as usize);
        if dense[r] == 0 {
            next += 1;
            dense[r] = next;
        }
        *l = dense[r];
    }
    Ok((lab, next))
}

/// Renumera los objetos en orden de lectura y convierte en 0 los de menos de `min_cells`.
fn finish(mut ids: Vec<u32>, count: u32, min_cells: u32) -> (Vec<u32>, u32) {
    let mut size = vec![0u32; count as usize + 1];
    for &l in &ids {
        size[l as usize] += 1;
    }
    let mut map = vec![0u32; count as usize + 1];
    let mut next = 0u32;
    for l in ids.iter_mut().filter(|l| **l != 0) {
        let m = &mut map[*l as usize];
        if *m == 0 {
            if size[*l as usize] < min_cells.max(1) {
                *m = u32::MAX;
            } else {
                next += 1;
                *m = next;
            }
        }
        *l = if *m == u32::MAX { 0 } else { *m };
    }
    (ids, next)
}

/// Divide las celdas de la pila en objetos.
pub fn segment(stack: &LayerStack, p: &SegmentParams) -> Result<Labels> {
    let lat = stack.lat;
    let (ids, count) = match &p.method {
        Method::Mask {
            layer,
            min,
            max,
            diagonal,
        } => {
            let l = stack
                .index_of(layer)
                .ok_or_else(|| Error::Param(format!("no existe la capa '{layer}'")))?;
            let d = &stack.layers[l].data;
            let (lo, hi) = (*min, *max);
            let (lab, n) = label_components(
                lat.cols,
                lat.rows,
                *diagonal,
                |i| d[i] >= lo && d[i] <= hi,
                |_, _| true,
            )?;
            finish(lab, n, p.min_cells)
        }
        Method::Merge(m) => merge(stack, m, p.min_cells)?,
    };
    Ok(Labels { lat, ids, count })
}

/// Un borde de `len` aristas de celda entre dos manchas, `a < b`.
#[derive(Clone, Copy)]
struct Edge {
    a: u32,
    b: u32,
    len: u32,
}

/// Las manchas y los bordes entre ellas.
struct Rag {
    /// Capas en uso y sus pesos (que suman 1).
    k: usize,
    w: Vec<f64>,
    shape: f64,
    compact: f64,
    n: Vec<u32>,
    /// Por mancha y capa: la suma de los valores y de sus cuadrados.
    sum: Vec<f64>,
    sq: Vec<f64>,
    /// Aristas del contorno de la mancha.
    per: Vec<u32>,
    /// Primera columna, primera fila, última columna, última fila.
    bb: Vec<[u32; 4]>,
    edges: Vec<Edge>,
}

impl Rag {
    /// Lo que costaría fusionar las manchas `a` y `b`, que comparten `shared` aristas de celda.
    fn cost(&self, a: usize, b: usize, shared: u32) -> f64 {
        let (na, nb) = (f64::from(self.n[a]), f64::from(self.n[b]));
        let nm = na + nb;
        let mut color = 0.0;
        for j in 0..self.k {
            let (sa, sb) = (self.sum[a * self.k + j], self.sum[b * self.k + j]);
            let (qa, qb) = (self.sq[a * self.k + j], self.sq[b * self.k + j]);
            // n veces la desviación típica
            let hm = (nm * (qa + qb) - (sa + sb) * (sa + sb)).max(0.0).sqrt();
            let ha = (na * qa - sa * sa).max(0.0).sqrt();
            let hb = (nb * qb - sb * sb).max(0.0).sqrt();
            color += self.w[j] * (hm - ha - hb);
        }
        if self.shape <= 0.0 {
            return color;
        }
        let (la, lb) = (f64::from(self.per[a]), f64::from(self.per[b]));
        let lm = la + lb - 2.0 * f64::from(shared);
        let compact = lm * nm.sqrt() - la * na.sqrt() - lb * nb.sqrt();
        let outline = |bb: [u32; 4]| 2.0 * f64::from(bb[2] - bb[0] + 1 + bb[3] - bb[1] + 1);
        let (ba, bb) = (self.bb[a], self.bb[b]);
        let bm = [
            ba[0].min(bb[0]),
            ba[1].min(bb[1]),
            ba[2].max(bb[2]),
            ba[3].max(bb[3]),
        ];
        let smooth = nm * lm / outline(bm) - na * la / outline(ba) - nb * lb / outline(bb);
        (1.0 - self.shape) * color
            + self.shape * (self.compact * compact + (1.0 - self.compact) * smooth)
    }

    /// Para cada mancha, el coste de fusionarla con su mejor vecina y quién es (gana el número
    /// menor en caso de empate).
    fn best(&self) -> (Vec<f64>, Vec<u32>) {
        let costs: Vec<f64> = self
            .edges
            .par_iter()
            .map(|e| self.cost(e.a as usize, e.b as usize, e.len))
            .collect();
        let mut cost = vec![f64::INFINITY; self.n.len()];
        let mut nb = vec![u32::MAX; self.n.len()];
        for (e, &c) in self.edges.iter().zip(&costs) {
            for (x, y) in [(e.a, e.b), (e.b, e.a)] {
                let x = x as usize;
                if c < cost[x] || (c == cost[x] && y < nb[x]) {
                    cost[x] = c;
                    nb[x] = y;
                }
            }
        }
        (cost, nb)
    }

    /// Fusiona las manchas unidas en `parent` (una unión-búsqueda con el miembro menor como raíz);
    /// devuelve adónde fue cada mancha antigua.
    fn merge(&mut self, parent: &mut Dsu) -> Vec<u32> {
        let old = self.n.len();
        let mut new_id = vec![0u32; old];
        let mut remap = vec![0u32; old];
        let mut count = 0u32;
        for (r, id) in new_id.iter_mut().enumerate() {
            if parent.find(r) == r {
                *id = count;
                count += 1;
            }
        }
        for (r, m) in remap.iter_mut().enumerate() {
            *m = new_id[parent.find(r)];
        }
        let cnt = count as usize;
        let k = self.k;
        let mut n = vec![0u32; cnt];
        let mut sum = vec![0f64; cnt * k];
        let mut sq = vec![0f64; cnt * k];
        let mut per = vec![0u32; cnt];
        let mut bb = vec![[u32::MAX, u32::MAX, 0, 0]; cnt];
        for (r, &t) in remap.iter().enumerate() {
            let t = t as usize;
            n[t] += self.n[r];
            per[t] += self.per[r];
            for j in 0..k {
                sum[t * k + j] += self.sum[r * k + j];
                sq[t * k + j] += self.sq[r * k + j];
            }
            let (s, o) = (&mut bb[t], self.bb[r]);
            *s = [
                s[0].min(o[0]),
                s[1].min(o[1]),
                s[2].max(o[2]),
                s[3].max(o[3]),
            ];
        }
        let mut edges: Vec<Edge> = Vec::with_capacity(self.edges.len());
        for e in &self.edges {
            let (a, b) = (remap[e.a as usize], remap[e.b as usize]);
            if a == b {
                per[a as usize] = per[a as usize].saturating_sub(2 * e.len);
            } else {
                edges.push(Edge {
                    a: a.min(b),
                    b: a.max(b),
                    len: e.len,
                });
            }
        }
        edges.par_sort_unstable_by_key(|e| (u64::from(e.a) << 32) | u64::from(e.b));
        let mut joined: Vec<Edge> = Vec::with_capacity(edges.len());
        for e in edges {
            match joined.last_mut() {
                Some(l) if l.a == e.a && l.b == e.b => l.len += e.len,
                _ => joined.push(e),
            }
        }
        self.n = n;
        self.sum = sum;
        self.sq = sq;
        self.per = per;
        self.bb = bb;
        self.edges = joined;
        remap
    }
}

/// Celdas de borde guardadas antes de contarlas en las aristas (unos cientos de MB como mucho).
const KEY_CHUNK: usize = 16_000_000;

/// Cuenta las claves de borde (`a << 32 | b`, una por arista de celda) en `edges`, que sigue
/// ordenado por pares; `keys` queda vacío.
fn add_borders(edges: &mut Vec<Edge>, keys: &mut Vec<u64>) {
    keys.par_sort_unstable();
    let mut fresh: Vec<Edge> = Vec::new();
    for &key in keys.iter() {
        let (a, b) = ((key >> 32) as u32, key as u32);
        match fresh.last_mut() {
            Some(e) if e.a == a && e.b == b => e.len += 1,
            _ => fresh.push(Edge { a, b, len: 1 }),
        }
    }
    keys.clear();
    keys.shrink_to(KEY_CHUNK + 1);
    if edges.is_empty() {
        *edges = fresh;
        return;
    }
    let old = std::mem::take(edges);
    edges.reserve(old.len() + fresh.len());
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < fresh.len() {
        let take_old =
            j == fresh.len() || (i < old.len() && (old[i].a, old[i].b) <= (fresh[j].a, fresh[j].b));
        let e = if take_old { old[i] } else { fresh[j] };
        if take_old {
            i += 1
        } else {
            j += 1
        }
        match edges.last_mut() {
            Some(l) if l.a == e.a && l.b == e.b => l.len += e.len,
            _ => edges.push(e),
        }
    }
}

fn merge(stack: &LayerStack, m: &MergeParams, min_cells: u32) -> Result<(Vec<u32>, u32)> {
    let Lattice { cols, rows, .. } = stack.lat;
    if cols > u32::MAX as usize / 2 || rows > u32::MAX as usize / 2 {
        return Err(Error::Param("la rejilla es demasiado grande".into()));
    }
    // capas en uso y sus pesos
    let mut sel: Vec<usize> = Vec::new();
    let mut w: Vec<f64> = Vec::new();
    for (i, l) in stack.layers.iter().enumerate() {
        let weight = m
            .weights
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&l.name))
            .map_or(1.0, |(_, v)| f64::from(*v));
        if weight > 0.0 {
            sel.push(i);
            w.push(weight);
        }
    }
    for (name, _) in &m.weights {
        if stack.index_of(name).is_none() {
            return Err(Error::Param(format!("no existe la capa '{name}'")));
        }
    }
    if sel.is_empty() {
        return Err(Error::Param("hace falta al menos una capa con peso".into()));
    }
    if !(m.scale > 0.0) || !(m.init > 0.0) {
        return Err(Error::Param(
            "la escala y el paso inicial deben ser positivos".into(),
        ));
    }
    let total: f64 = w.iter().sum();
    for v in &mut w {
        *v /= total;
    }
    let k = sel.len();
    let data: Vec<&[f32]> = sel
        .iter()
        .map(|&i| stack.layers[i].data.as_slice())
        .collect();
    // estandarizar: media y desviación típica de cada capa sobre sus celdas válidas
    let (mean, inv): (Vec<f64>, Vec<f64>) = data
        .iter()
        .map(|d| {
            let (mut s, mut q, mut c) = (0f64, 0f64, 0u64);
            let shift = d
                .iter()
                .find(|v| v.is_finite())
                .copied()
                .map_or(0.0, f64::from);
            for &v in d.iter().filter(|v| v.is_finite()) {
                let z = f64::from(v) - shift;
                s += z;
                q += z * z;
                c += 1;
            }
            if c == 0 {
                return (0.0, 1.0);
            }
            let mu = s / c as f64;
            let sd = (q / c as f64 - mu * mu).max(0.0).sqrt();
            (shift + mu, if sd > 1e-12 { 1.0 / sd } else { 1.0 })
        })
        .unzip();
    let z = |j: usize, i: usize| (f64::from(data[j][i]) - mean[j]) * inv[j];
    let valid = |i: usize| data.iter().all(|d| d[i].is_finite());
    // manchas de celdas en los mismos intervalos de valor
    let step: Vec<f64> = inv.iter().map(|v| v / m.init).collect();
    let bin = |j: usize, i: usize| ((f64::from(data[j][i]) - mean[j]) * step[j]).floor() as i64;
    let (lab, patches) = label_components(cols, rows, false, valid, |i, j| {
        (0..k).all(|l| bin(l, i) == bin(l, j))
    })?;
    if patches == 0 {
        return Ok((lab, 0));
    }
    let r0 = patches as usize;
    // las estadísticas y los bordes de las manchas, en una pasada
    let mut rag = Rag {
        k,
        w,
        shape: m.shape.clamp(0.0, 0.9),
        compact: m.compact.clamp(0.0, 1.0),
        n: vec![0; r0],
        sum: vec![0.0; r0 * k],
        sq: vec![0.0; r0 * k],
        per: vec![0; r0],
        bb: vec![[u32::MAX, u32::MAX, 0, 0]; r0],
        edges: Vec::new(),
    };
    let mut keys: Vec<u64> = Vec::new();
    for y in 0..rows {
        if y % 1024 == 0 {
            check_cancel()?;
        }
        // los bordes se cuentan según llegan: una clave por celda de borde ocuparía un gigabyte en
        // un mosaico grande
        if keys.len() > KEY_CHUNK {
            add_borders(&mut rag.edges, &mut keys);
        }
        for x in 0..cols {
            let i = y * cols + x;
            let l = lab[i];
            if l == 0 {
                continue;
            }
            let r = (l - 1) as usize;
            rag.n[r] += 1;
            for j in 0..k {
                let v = z(j, i);
                rag.sum[r * k + j] += v;
                rag.sq[r * k + j] += v * v;
            }
            let bb = &mut rag.bb[r];
            *bb = [
                bb[0].min(x as u32),
                bb[1].min(y as u32),
                bb[2].max(x as u32),
                bb[3].max(y as u32),
            ];
            let mut outline = 0;
            let mut side = |j: Option<usize>, border: bool| match j.map(|j| lab[j]) {
                Some(o) if o == l => {}
                Some(o) if o != 0 => {
                    outline += 1;
                    if border {
                        let (a, b) = (l.min(o) - 1, l.max(o) - 1);
                        keys.push((u64::from(a) << 32) | u64::from(b));
                    }
                }
                _ => outline += 1,
            };
            // derecha y abajo cuentan cada borde una vez; izquierda y arriba solo añaden al
            // contorno
            side((x + 1 < cols).then_some(i + 1), true);
            side((y + 1 < rows).then_some(i + cols), true);
            side((x > 0).then(|| i - 1), false);
            side((y > 0).then(|| i - cols), false);
            rag.per[r] += outline;
        }
    }
    add_borders(&mut rag.edges, &mut keys);
    drop(keys);
    // dónde está ahora cada mancha inicial
    let mut cur: Vec<u32> = (0..r0 as u32).collect();
    let limit = m.scale * m.scale;
    loop {
        check_cancel()?;
        let (cost, nb) = rag.best();
        let mut parent = Dsu::new(rag.n.len());
        let mut merges = 0usize;
        for e in &rag.edges {
            let (a, b) = (e.a as usize, e.b as usize);
            if nb[a] == e.b && nb[b] == e.a && cost[a] < limit {
                parent.union(e.a as usize, e.b as usize);
                merges += 1;
            }
        }
        if merges == 0 {
            break;
        }
        let remap = rag.merge(&mut parent);
        cur.par_iter_mut().for_each(|c| *c = remap[*c as usize]);
    }
    // las manchas por debajo del tamaño mínimo se unen a su vecina más parecida
    if min_cells > 1 {
        loop {
            check_cancel()?;
            let (_, nb) = rag.best();
            let mut parent = Dsu::new(rag.n.len());
            let mut merges = 0usize;
            for (a, &b) in nb.iter().enumerate() {
                if rag.n[a] < min_cells && b != u32::MAX {
                    parent.union(a, b as usize);
                    merges += 1;
                }
            }
            if merges == 0 {
                break;
            }
            let remap = rag.merge(&mut parent);
            cur.par_iter_mut().for_each(|c| *c = remap[*c as usize]);
        }
    }
    let groups = rag.n.len();
    let ids: Vec<u32> = lab
        .par_iter()
        .map(|&l| if l == 0 { 0 } else { cur[(l - 1) as usize] + 1 })
        .collect();
    Ok(finish(ids, groups as u32, min_cells))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obia::Lattice;

    #[test]
    fn borders_counted_in_chunks_match_counting_at_once() {
        let key = |a: u64, b: u64| (a << 32) | b;
        let mut edges = Vec::new();
        add_borders(&mut edges, &mut vec![key(0, 1), key(0, 1), key(2, 3)]);
        add_borders(&mut edges, &mut vec![key(0, 1), key(1, 2), key(2, 3)]);
        let got: Vec<(u32, u32, u32)> = edges.iter().map(|e| (e.a, e.b, e.len)).collect();
        assert_eq!(got, [(0, 1, 3), (1, 2, 1), (2, 3, 2)]);
    }

    fn stack(cols: usize, rows: usize, f: impl Fn(usize, usize) -> f32) -> LayerStack {
        let lat = Lattice {
            x0: 0.0,
            y0: 0.0,
            cell: 1.0,
            cols,
            rows,
        };
        let mut s = LayerStack::new(lat, None);
        let data = (0..cols * rows).map(|i| f(i % cols, i / cols)).collect();
        assert!(s.push("v", data).is_ok());
        s
    }

    #[test]
    fn a_mask_gives_its_connected_groups() -> Result<()> {
        // dos bloques de 3 x 3 separados, una sola celda que toca al primero por una esquina
        let s = stack(12, 6, |x, y| {
            let block = (x < 3 && y < 3) || (x >= 8 && y < 3);
            let corner = x == 3 && y == 3;
            if block || corner { 1.0 } else { 0.0 }
        });
        let run = |diagonal, min_cells| {
            segment(
                &s,
                &SegmentParams {
                    method: Method::Mask {
                        layer: "v".into(),
                        min: 0.5,
                        max: 2.0,
                        diagonal,
                    },
                    min_cells,
                },
            )
        };
        assert_eq!(run(false, 1)?.count, 3);
        assert_eq!(run(true, 1)?.count, 2);
        // los grupos pequeños se descartan y el resto se vuelve a numerar
        let l = run(false, 2)?;
        assert_eq!(l.count, 2);
        assert_eq!(l.ids[3 * 12 + 3], 0);
        assert_eq!(l.ids[0], 1);
        assert_eq!(l.ids[8], 2);
        Ok(())
    }

    /// Un escalón con ruido: dos mesetas separadas 10.
    fn plateaus() -> LayerStack {
        stack(40, 30, |x, y| {
            let noise = (((x * 31 + y * 17) % 7) as f32 - 3.0) * 0.05;
            (if x < 20 { 2.0 } else { 12.0 }) + noise
        })
    }

    #[test]
    fn merging_finds_the_plateaus() -> Result<()> {
        let p = |scale| SegmentParams {
            method: Method::Merge(MergeParams {
                scale,
                ..Default::default()
            }),
            min_cells: 4,
        };
        let l = segment(&plateaus(), &p(8.0))?;
        assert_eq!(l.count, 2);
        let (a, b) = (l.ids[0], l.ids[39]);
        assert!(a != 0 && b != 0 && a != b);
        for (i, &id) in l.ids.iter().enumerate() {
            assert_eq!(id, if i % 40 < 20 { a } else { b });
        }
        // una escala mayor que el paso fusiona todo
        let all = segment(&plateaus(), &p(1000.0))?;
        assert_eq!(all.count, 1);
        // el resultado no depende de la planificación de los hilos
        assert_eq!(segment(&plateaus(), &p(8.0))?.ids, l.ids);
        Ok(())
    }

    #[test]
    fn merging_leaves_empty_cells_out() -> Result<()> {
        let s = stack(
            10,
            10,
            |x, _| if x == 5 { f32::NAN } else { x as f32 * 10.0 },
        );
        let l = segment(
            &s,
            &SegmentParams {
                method: Method::Merge(MergeParams::default()),
                min_cells: 1,
            },
        )?;
        assert!(
            l.ids
                .iter()
                .enumerate()
                .all(|(i, &v)| (v == 0) == (i % 10 == 5))
        );
        assert!(l.count >= 2);
        Ok(())
    }

    #[test]
    fn unknown_layers_are_reported() {
        let r = segment(
            &plateaus(),
            &SegmentParams {
                method: Method::Mask {
                    layer: "nada".into(),
                    min: 0.0,
                    max: 1.0,
                    diagonal: false,
                },
                min_cells: 1,
            },
        );
        assert!(matches!(r, Err(Error::Param(_))));
    }
}
