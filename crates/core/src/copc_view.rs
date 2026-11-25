//! Detalle por vista de nubes COPC: en lugar de leer una ventana del mapa a un nivel único, elige
//! los nodos del octree según lo que ve la cámara 3D. Un nodo se refina (se leen sus hijos) mientras
//! se ve más grande que `min_px` píxeles, empezando por los que se ven mayores y hasta un
//! presupuesto de puntos: lo cercano llega a todo su detalle, lo lejano se queda en niveles
//! gruesos, y lo que está fuera del encuadre no se lee. Los nodos descodificados se guardan en una
//! caché para que mover un poco la cámara solo lea los nuevos.

use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

use rayon::prelude::*;

use crate::copc::{Copc, Node};
use crate::las::Reader;
use crate::view::{Orbit, PointStore};
use crate::{Error, Result};

/// Un nodo: archivo, nivel y posición.
type Key = (usize, i32, i32, i32, i32);

/// El presupuesto de puntos de la caché: pasado esto se descartan los nodos menos recientes.
const CACHE_FACTOR: u64 = 3;

fn u16_at(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

/// Un nodo en la cola de refinado: los de mayor tamaño aparente salen primero.
#[derive(PartialEq)]
struct Item {
    px: f64,
    key: Key,
}

impl Eq for Item {}

impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Item {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.px
            .total_cmp(&other.px)
            .then_with(|| other.key.cmp(&self.key))
    }
}

/// Las nubes COPC de una vista, con la caché de nodos ya leídos.
pub struct ViewLoader {
    clouds: Vec<Copc>,
    nodes: Vec<HashMap<(i32, i32, i32, i32), Node>>,
    origin: [f64; 3],
    files: Vec<String>,
    spacing0: f64,
    has_rgb: bool,
    has_tree: bool,
    cache: HashMap<Key, (Arc<PointStore>, u64)>,
    cached_points: u64,
    clock: u64,
}

impl ViewLoader {
    /// Abre las nubes COPC de `paths`. `None` cuando alguna no es COPC.
    pub fn open(paths: &[std::path::PathBuf], origin: [f64; 3], files: &[String]) -> Result<Option<Self>> {
        let mut clouds = Vec::new();
        for p in paths {
            match Copc::open(p)? {
                Some(c) => clouds.push(c),
                None => return Ok(None),
            }
        }
        let nodes = clouds
            .iter()
            .map(|c| {
                Ok(c.nodes(None, None)?
                    .into_iter()
                    .map(|n| ((n.level, n.x, n.y, n.z), n))
                    .collect())
            })
            .collect::<Result<Vec<_>>>()?;
        let spacing0 = clouds
            .iter()
            .map(|c| c.spacing())
            .fold(f64::INFINITY, f64::min);
        let has_rgb = clouds
            .iter()
            .any(|c| matches!(c.header().format, 2 | 3 | 5 | 7 | 8 | 10));
        let has_tree = clouds
            .iter()
            .any(|c| c.reader().extra_field("treeID").is_some());
        Ok(Some(Self {
            clouds,
            nodes,
            origin,
            files: files.to_vec(),
            spacing0,
            has_rgb,
            has_tree,
            cache: HashMap::new(),
            cached_points: 0,
            clock: 0,
        }))
    }

    /// Los nodos que hay que leer para la cámara `o` en una vista `w`×`h`.
    pub fn select(&self, o: &Orbit, w: usize, h: usize, budget: u64, min_px: f64) -> Vec<Key> {
        let proj = o.projector(w, h);
        let (sy, cy) = o.yaw.sin_cos();
        let (sp, cp) = o.pitch.sin_cos();
        // la posición de la cámara en el mapa
        let eye = [
            o.target[0] - o.dist * cp * sy,
            o.target[1] - o.dist * cp * cy,
            o.target[2] + o.dist * sp / o.exaggeration.max(1e-6),
        ];
        let f = Orbit::focal(w, h);
        let (wf, hf) = (w as f64, h as f64);
        let margin = 0.1 * wf.max(hf);
        // tamaño aparente en píxeles de un cubo, o `None` si no se ve
        let size_of = |c: [f64; 6]| -> Option<f64> {
            let side = c[3] - c[0];
            let inside = (0..3).all(|k| eye[k] >= c[k] && eye[k] <= c[k + 3]);
            if inside {
                return Some(f64::INFINITY);
            }
            let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            let mut front = 0;
            for i in 0..8 {
                let p = [
                    if i & 1 == 0 { c[0] } else { c[3] },
                    if i & 2 == 0 { c[1] } else { c[4] },
                    if i & 4 == 0 { c[2] } else { c[5] },
                ];
                if let Some((x, y, _)) = proj.apply(p) {
                    front += 1;
                    lo = [lo[0].min(x), lo[1].min(y)];
                    hi = [hi[0].max(x), hi[1].max(y)];
                }
            }
            if front == 0 {
                return None;
            }
            if front == 8 {
                if hi[0] < -margin || hi[1] < -margin || lo[0] > wf + margin || lo[1] > hf + margin {
                    return None;
                }
                return Some((hi[0] - lo[0]).max(hi[1] - lo[1]));
            }
            // el cubo cruza el plano de la cámara: se estima por la distancia al cubo
            let d = (0..3)
                .map(|k| (c[k] - eye[k]).max(0.0).max(eye[k] - c[k + 3]).powi(2))
                .sum::<f64>()
                .sqrt()
                .max(1e-3);
            Some(side * f / d)
        };
        let mut heap = BinaryHeap::new();
        for (fi, c) in self.clouds.iter().enumerate() {
            if let Some(px) = size_of(c.cube(0, 0, 0, 0)) {
                heap.push(Item { px, key: (fi, 0, 0, 0, 0) });
            }
        }
        let mut out = Vec::new();
        let mut points = 0u64;
        while let Some(Item { key, .. }) = heap.pop() {
            let (fi, l, x, y, z) = key;
            if let Some(n) = self.nodes[fi].get(&(l, x, y, z)) {
                points += u64::from(n.points);
                out.push(key);
                if points >= budget {
                    break;
                }
            }
            for d in 0..8 {
                let (cx, cy2, cz) = (2 * x + (d & 1), 2 * y + ((d >> 1) & 1), 2 * z + ((d >> 2) & 1));
                if !self.nodes[fi].contains_key(&(l + 1, cx, cy2, cz)) {
                    continue;
                }
                if let Some(px) = size_of(self.clouds[fi].cube(l + 1, cx, cy2, cz))
                    && px >= min_px
                {
                    heap.push(Item { px, key: (fi, l + 1, cx, cy2, cz) });
                }
            }
        }
        out
    }

    /// Descodifica los nodos que faltan en la caché.
    fn fetch(&mut self, missing: &[Key]) -> Result<()> {
        let mut by_file: HashMap<usize, Vec<Node>> = HashMap::new();
        for &(fi, l, x, y, z) in missing {
            if let Some(n) = self.nodes[fi].get(&(l, x, y, z)) {
                by_file.entry(fi).or_default().push(*n);
            }
        }
        for (fi, mut list) in by_file {
            let c = &self.clouds[fi];
            let r: &Reader = c.reader();
            let tree_field = r.extra_field("treeID");
            let h = &r.header;
            let rec_len = usize::from(h.record_len);
            let (src_at, rgb_at) = (
                if h.format >= 6 { 20 } else { 18 },
                match h.format {
                    2 => Some(20),
                    3 | 5 => Some(28),
                    7 | 8 | 10 => Some(30),
                    _ => None,
                },
            );
            list.sort_unstable_by_key(|n| n.offset);
            // grupos de nodos que `read_raw` entrega de una vez, en el orden del archivo
            let mut i = 0;
            while i < list.len() {
                let mut j = i;
                let mut pts = 0u64;
                while j < list.len() && (j == i || pts + u64::from(list[j].points) <= (1 << 22)) {
                    pts += u64::from(list[j].points);
                    j += 1;
                }
                let group = list[i..j].to_vec();
                let mut results: Vec<(Key, PointStore)> = Vec::new();
                let origin = self.origin;
                let (has_rgb, has_tree) = (self.has_rgb, self.has_tree);
                c.read_raw(&group, |bytes| {
                    let mut at = 0usize;
                    for n in &group {
                        let len = n.points as usize * rec_len;
                        let Some(chunk) = bytes.get(at..at + len) else {
                            return Err(Error::Format("COPC: nodo incompleto".into()));
                        };
                        at += len;
                        let rows: Vec<_> = chunk
                            .par_chunks(rec_len * 8192)
                            .map(|block| {
                                block
                                    .chunks_exact(rec_len)
                                    .filter_map(|rec| {
                                        let p = r.decode(rec)?;
                                        let rgb = rgb_at.map(|a| {
                                            let q = |k: usize| (u16_at(rec, a + 2 * k) >> 8) as u8;
                                            [q(0), q(1), q(2)]
                                        });
                                        Some((
                                            p.x,
                                            p.y,
                                            p.z,
                                            p.intensity,
                                            p.return_number | (p.number_of_returns << 4),
                                            p.class,
                                            u16_at(rec, src_at),
                                            (r.decode_extra(rec).scan_angle * 10.0).round() as i16,
                                            rgb,
                                            tree_field.map_or(0, |f| f.id(rec)),
                                        ))
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .collect();
                        let mut s = PointStore {
                            origin,
                            stride: 1,
                            ..PointStore::default()
                        };
                        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                        for (x, y, z, i, ret, class, src, scan, rgb, tree) in rows.into_iter().flatten() {
                            s.x.push((x - origin[0]) as f32);
                            s.y.push((y - origin[1]) as f32);
                            s.z.push((z - origin[2]) as f32);
                            s.intensity.push(i);
                            s.ret.push(ret);
                            s.class.push(class);
                            s.src.push(src);
                            s.scan.push(scan);
                            s.file.push(fi.min(255) as u8);
                            if has_rgb {
                                s.rgb.push(rgb.unwrap_or([150, 150, 150]));
                            }
                            if has_tree {
                                s.tree.push(tree);
                            }
                            s.lvl.push(n.level.clamp(0, 255) as u8);
                            for (k, v) in [x, y, z].into_iter().enumerate() {
                                lo[k] = lo[k].min(v);
                                hi[k] = hi[k].max(v);
                            }
                        }
                        s.bounds = [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]];
                        results.push(((fi, n.level, n.x, n.y, n.z), s));
                    }
                    Ok(())
                })
                .map_err(|e| Error::Format(format!("COPC: {e}")))?;
                for (k, s) in results {
                    self.cached_points += s.len() as u64;
                    self.cache.insert(k, (Arc::new(s), self.clock));
                }
                i = j;
            }
        }
        Ok(())
    }

    /// El almacén de los puntos que ve la cámara `o`. Lee los nodos que no estén en la caché.
    pub fn load(&mut self, o: &Orbit, w: usize, h: usize, budget: u64, min_px: f64) -> Result<PointStore> {
        self.clock += 1;
        let keys = self.select(o, w, h, budget, min_px);
        let missing: Vec<Key> = keys
            .iter()
            .filter(|k| !self.cache.contains_key(*k))
            .copied()
            .collect();
        self.fetch(&missing)?;
        for k in &keys {
            if let Some(e) = self.cache.get_mut(k) {
                e.1 = self.clock;
            }
        }
        // la caché no pasa de unas veces el presupuesto: se descartan los nodos menos recientes
        let limit = budget * CACHE_FACTOR;
        if self.cached_points > limit {
            let mut order: Vec<(u64, Key)> = self.cache.iter().map(|(k, v)| (v.1, *k)).collect();
            order.sort_unstable();
            for (age, k) in order {
                if self.cached_points <= limit || age == self.clock {
                    break;
                }
                if let Some((s, _)) = self.cache.remove(&k) {
                    self.cached_points -= s.len() as u64;
                }
            }
        }
        let mut out = PointStore {
            origin: self.origin,
            files: self.files.clone(),
            stride: 1,
            lvl_spacing0: self.spacing0,
            ..PointStore::default()
        };
        let parts: Vec<&Arc<PointStore>> = keys
            .iter()
            .filter_map(|k| self.cache.get(k).map(|e| &e.0))
            .collect();
        let n: usize = parts.iter().map(|p| p.len()).sum();
        out.x.reserve(n);
        out.y.reserve(n);
        out.z.reserve(n);
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in parts {
            out.x.extend_from_slice(&p.x);
            out.y.extend_from_slice(&p.y);
            out.z.extend_from_slice(&p.z);
            out.intensity.extend_from_slice(&p.intensity);
            out.ret.extend_from_slice(&p.ret);
            out.class.extend_from_slice(&p.class);
            out.src.extend_from_slice(&p.src);
            out.scan.extend_from_slice(&p.scan);
            out.file.extend_from_slice(&p.file);
            out.rgb.extend_from_slice(&p.rgb);
            out.tree.extend_from_slice(&p.tree);
            out.lvl.extend_from_slice(&p.lvl);
            if !p.is_empty() {
                for k in 0..3 {
                    lo[k] = lo[k].min(p.bounds[k]);
                    hi[k] = hi[k].max(p.bounds[k + 3]);
                }
            }
        }
        out.total = out.x.len() as u64;
        out.bounds = if out.x.is_empty() {
            [0.0; 6]
        } else {
            [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
        };
        Ok(out)
    }
}
