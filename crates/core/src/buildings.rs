//! Clasificación de edificios y vegetación, para modelos del terreno más limpios en zonas
//! construidas.
//!
//! Se construye una referencia del terreno a partir del punto más bajo de cada celda mediante una
//! apertura morfológica más ancha que cualquier edificio, que elimina los tejados decida lo que
//! decida el filtro de suelo sobre ellos. Las celdas que quedan a más de `offset` por encima de
//! ella forman regiones candidatas; una región es un edificio cuando es suficientemente grande,
//! lisa (los tejados son planos, las copas son rugosas) y está limitada por escalones bruscos
//! (bordes de tejado, a diferencia de los lados suaves de una colina). Los puntos de las celdas de
//! edificio por encima del desplazamiento pasan a ser edificios (clase 6), incluso los que el
//! filtro de suelo había conservado como suelo; los demás puntos que no son suelo por encima del
//! desplazamiento pasan a ser vegetación alta (clase 5). Funciona en memoria: ejecútese sobre
//! teselas con un margen de al menos la mitad de la ventana.

use std::collections::VecDeque;

use crate::features;
use crate::grid::Grid;
use crate::las::Point;

#[derive(Clone, Debug, PartialEq)]
pub struct BuildingParams {
    /// Tamaño de celda de las rejillas de análisis.
    pub cell: f64,
    /// Altura sobre el terreno a partir de la cual los puntos pueden ser edificios o árboles.
    pub offset: f64,
    /// El edificio más ancho esperado: la ventana de la apertura del terreno.
    pub window: f64,
    /// Menor huella de edificio, en unidades cuadradas.
    pub min_area: f64,
    /// Rugosidad (m: desviación típica de la distancia de los vecinos a su mejor plano) hasta la
    /// cual un punto puede ser tejado.
    pub planar: f64,
    /// Proporción de los ecos de una celda que atravesó algo (primero de varios, o intermedio) por
    /// encima de la cual la celda es vegetación y no un tejado: un tejado detiene el pulso.
    pub penetrating: f64,
    /// Altura (m) sobre la superficie más baja de una celda de tejado hasta la cual un punto es el
    /// tejado; lo que está por encima (una copa sobre el tejado) es vegetación.
    pub roof_depth: f64,
    /// Radio (m) del entorno que dice si un punto está sobre un plano.
    pub radius: f64,
    /// Planaridad (0 a 1) que necesita una semilla de tejado en su entorno.
    pub seed_planarity: f64,
    /// Semillas de tejado que necesita una celda para ser el núcleo de un tejado.
    pub seeds_per_cell: u32,
    /// Celdas en las que crece el edificio alrededor de su tejado (aleros y paredes).
    pub eaves: usize,
    /// Proporción (0 a 1) de los lados de un tejado que deben bajar bruscamente al terreno de
    /// alrededor: un tejado termina en una pared, una ladera no.
    pub sharp: f64,
}

impl Default for BuildingParams {
    fn default() -> Self {
        Self {
            cell: 1.0,
            offset: 2.5,
            window: 60.0,
            min_area: 40.0,
            planar: 0.07,
            penetrating: 0.7,
            roof_depth: 1.5,
            radius: 1.5,
            seed_planarity: 0.6,
            seeds_per_cell: 3,
            eaves: 2,
            sharp: 0.9,
        }
    }
}

pub const BUILDING: u8 = 6;
pub const HIGH_VEGETATION: u8 = 5;

/// La clase nueva de cada punto, si la hay: [`BUILDING`] o [`HIGH_VEGETATION`]. `ground` dice qué
/// puntos conservó el filtro de suelo (los tejados que conservó se siguen encontrando y
/// reclasificando).
///
/// Los puntos de tejado se hallan uno a uno, no sobre un ráster de la superficie más baja: un punto
/// por encima del terreno es una semilla de tejado cuando los puntos a su alrededor están sobre un
/// plano (planaridad y rugosidad de su entorno) y su eco no pasó más allá (un tejado detiene el
/// pulso; una copa lo deja pasar). Las celdas con suficientes semillas, unidas a través de la
/// cumbrera de un tejado a dos aguas, forman las regiones que son grandes y tienen lados empinados;
/// una región y las celdas a su alrededor (los aleros y las paredes) son edificio hasta
/// `roof_depth` por encima del tejado, y lo que está por encima es vegetación.
pub fn classify(pts: &[Point], ground: &[bool], p: &BuildingParams) -> Vec<Option<u8>> {
    let mut out = vec![None; pts.len()];
    let valid: Vec<&Point> = pts.iter().filter(|q| !q.is_noise()).collect();
    if valid.is_empty() || !(p.cell > 0.0) {
        return out;
    }
    let b = valid.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, q| [b[0].min(q.x), b[1].min(q.y), b[2].max(q.x), b[3].max(q.y)],
    );
    let Ok(mut low) = Grid::covering(b, p.cell) else {
        return out;
    };
    for q in &valid {
        if let Some(v) = low.index(q.x, q.y).and_then(|i| low.data.get_mut(i))
            && (v.is_nan() || (q.z as f32) < *v)
        {
            *v = q.z as f32;
        }
    }
    low.fill_holes();
    let k = ((p.window / p.cell).round() as usize) | 1;
    let terrain = opening(&low, k);
    let (cols, rows) = (low.cols, low.rows);
    let n = cols * rows;

    let cell_of = |q: &Point| low.index(q.x, q.y);
    let through = |q: &Point| q.number_of_returns > 1 && q.return_number < q.number_of_returns;
    // Puntos que pueden ser tejado o copa: por encima del terreno. Los que conservó el filtro de
    // suelo se miran aparte y se someten a una prueba más estricta: en una ladera empinada la
    // estimación del terreno es demasiado baja y el suelo queda por encima de ella, pero un tejado
    // que el filtro tomó por suelo tiene bordes marcados.
    let above: Vec<bool> = pts
        .iter()
        .map(|q| {
            !q.is_noise() && cell_of(q).is_some_and(|i| q.z as f32 - terrain[i] > p.offset as f32)
        })
        .collect();
    let is_ground = |k: usize| ground.get(k).copied().unwrap_or(false);
    let free: Vec<bool> = (0..pts.len()).map(|k| above[k] && !is_ground(k)).collect();
    let kept: Vec<bool> = (0..pts.len()).map(|k| above[k] && is_ground(k)).collect();
    // Celdas cuyo retorno más bajo está muy por encima del terreno: un tejado o una copa sin paso a
    // través.
    let raised: Vec<bool> = (0..n)
        .map(|i| low.data[i] - terrain[i] > p.offset as f32)
        .collect();
    let min_cells = (p.min_area / (p.cell * p.cell)).ceil() as usize;

    // Las celdas de edificio entre los puntos de `mask` (de las cuales `sharp_share` de los lados
    // de una región deben bajar bruscamente a lo que hay alrededor) y la semilla de tejado más alta
    // de cada celda.
    let detect = |mask: &[bool], sharp_share: f64| -> (Vec<bool>, Vec<f32>) {
        let mut building = vec![false; n];
        let mut seed_top = vec![f32::NEG_INFINITY; n];
        if !mask.iter().any(|m| *m) {
            return (building, seed_top);
        }
        let feats = features::compute(pts, mask, p.radius);
        let mut seeds = vec![0u32; n];
        let mut counted = vec![0u32; n];
        let mut through_n = vec![0u32; n];
        for (k, q) in pts.iter().enumerate() {
            if !mask[k] {
                continue;
            }
            let Some(i) = cell_of(q) else { continue };
            counted[i] += 1;
            through_n[i] += u32::from(through(q));
            let planar = feats[k].is_some_and(|f| {
                f64::from(f.planarity) >= p.seed_planarity && f64::from(f.roughness) <= p.planar
            });
            if planar && !through(q) {
                seeds[i] += 1;
                seed_top[i] = seed_top[i].max(q.z as f32);
            }
        }
        let solid: Vec<bool> = (0..n)
            .map(|i| {
                counted[i] > 0 && (through_n[i] as f32) <= p.penetrating as f32 * counted[i] as f32
            })
            .collect();
        let core: Vec<bool> = (0..n)
            .map(|i| solid[i] && seeds[i] >= p.seeds_per_cell)
            .collect();
        // La cumbrera de un tejado a dos aguas no tiene un entorno plano: se cierran huecos de una
        // celda o dos entre celdas núcleo, solo dentro de celdas sólidas.
        let dilated: Vec<bool> = (0..n)
            .map(|i| core[i] || neighbours8(i, cols, rows).any(|j| core[j]))
            .collect();
        let cand: Vec<bool> = (0..n)
            .map(|i| {
                core[i]
                    || (solid[i] && dilated[i] && neighbours8(i, cols, rows).all(|j| dilated[j]))
            })
            .collect();
        let mut seen = vec![false; n];
        for start in 0..n {
            if !cand[start] || seen[start] {
                continue;
            }
            let mut region = Vec::new();
            let mut queue = VecDeque::from([start]);
            seen[start] = true;
            while let Some(i) = queue.pop_front() {
                region.push(i);
                for j in neighbours8(i, cols, rows) {
                    if cand[j] && !seen[j] {
                        seen[j] = true;
                        queue.push_back(j);
                    }
                }
            }
            // Los aleros no son planos: el área cuenta el anillo de celdas sólidas alrededor de la
            // región.
            let mut ring: Vec<usize> = region
                .iter()
                .flat_map(|&i| neighbours8(i, cols, rows))
                .filter(|&j| !cand[j] && solid[j])
                .collect();
            ring.sort_unstable();
            ring.dedup();
            if region.len() + ring.len() < min_cells {
                continue;
            }
            // Escalones hacia el terreno alrededor de la región y sus aleros: marcados en los
            // bordes del tejado, suaves en las laderas.
            let inside: std::collections::HashSet<usize> =
                region.iter().chain(ring.iter()).copied().collect();
            let mut roof_low: Vec<f32> = region.iter().map(|&i| low.data[i]).collect();
            roof_low.sort_by(f32::total_cmp);
            let roof_low = roof_low[roof_low.len() / 2];
            let (mut edges, mut sharp) = (0usize, 0usize);
            for &i in &inside {
                for j in neighbours4(i, cols, rows) {
                    if !inside.contains(&j) && !raised[j] {
                        edges += 1;
                        if roof_low - low.data[j] > 0.5 * p.offset as f32 {
                            sharp += 1;
                        }
                    }
                }
            }
            let ok = if sharp_share > 0.5 {
                edges > 0 && sharp as f64 >= sharp_share * edges as f64
            } else {
                edges == 0 || sharp as f64 >= sharp_share * edges as f64
            };
            if ok {
                for &i in &region {
                    building[i] = true;
                }
            }
        }
        // Aleros y paredes: las celdas sólidas alrededor de un edificio, de `eaves` celdas de
        // profundidad.
        for _ in 0..p.eaves {
            let grown: Vec<usize> = (0..n)
                .filter(|&i| {
                    !building[i] && solid[i] && neighbours8(i, cols, rows).any(|j| building[j])
                })
                .collect();
            for i in grown {
                building[i] = true;
            }
        }
        (building, seed_top)
    };
    let (b_free, top_free) = detect(&free, p.sharp);
    let (b_kept, top_kept) = detect(&kept, p.sharp.max(0.8));

    for (k, q) in pts.iter().enumerate() {
        if !above[k] {
            continue;
        }
        let Some(i) = cell_of(q) else { continue };
        let z = q.z as f32;
        let top = top_free[i].max(top_kept[i]);
        let roof = (b_free[i] || b_kept[i])
            && if top > f32::NEG_INFINITY {
                z <= top + p.roof_depth as f32
            } else {
                !through(q) || z - low.data[i] <= p.roof_depth as f32
            };
        if roof {
            out[k] = Some(BUILDING);
        } else if !is_ground(k) {
            out[k] = Some(HIGH_VEGETATION);
        }
    }
    out
}

fn neighbours8(i: usize, cols: usize, rows: usize) -> impl Iterator<Item = usize> {
    let (c, r) = ((i % cols) as isize, (i / cols) as isize);
    (-1isize..=1)
        .flat_map(move |dr| (-1isize..=1).map(move |dc| (c + dc, r + dr)))
        .filter(move |&(nc, nr)| {
            (nc, nr) != (c, r) && nc >= 0 && nr >= 0 && (nc as usize) < cols && (nr as usize) < rows
        })
        .map(move |(nc, nr)| nr as usize * cols + nc as usize)
}

fn neighbours4(i: usize, cols: usize, rows: usize) -> impl Iterator<Item = usize> {
    let (c, r) = ((i % cols) as isize, (i / cols) as isize);
    [(c - 1, r), (c + 1, r), (c, r - 1), (c, r + 1)]
        .into_iter()
        .filter(move |&(nc, nr)| nc >= 0 && nr >= 0 && (nc as usize) < cols && (nr as usize) < rows)
        .map(move |(nc, nr)| nr as usize * cols + nc as usize)
}

/// Apertura gris (erosión y luego dilatación) con una ventana cuadrada de `k` celdas.
pub(crate) fn opening(g: &Grid, k: usize) -> Vec<f32> {
    let e = filter2(&g.data, g.cols, g.rows, k, f32::min);
    filter2(&e, g.cols, g.rows, k, f32::max)
}

/// Un mínimo o máximo móvil separable sobre una ventana de `k` x `k` (ventanas recortadas en los
/// bordes).
fn filter2(v: &[f32], cols: usize, rows: usize, k: usize, f: fn(f32, f32) -> f32) -> Vec<f32> {
    let h = run(v, cols, rows, k, f, true);
    run(&h, cols, rows, k, f, false)
}

/// Una dirección del filtro móvil, con una cola monótona: O(1) por celda.
fn run(
    v: &[f32],
    cols: usize,
    rows: usize,
    k: usize,
    f: fn(f32, f32) -> f32,
    along_rows: bool,
) -> Vec<f32> {
    let mut out = v.to_vec();
    let r = k / 2;
    let (lines, len) = if along_rows {
        (rows, cols)
    } else {
        (cols, rows)
    };
    let idx = |line: usize, pos: usize| {
        if along_rows {
            line * cols + pos
        } else {
            pos * cols + line
        }
    };
    // f(a, b) == a significa que a es al menos tan bueno como b.
    let better = |a: f32, b: f32| f(a, b) == a;
    let mut q: VecDeque<usize> = VecDeque::new();
    for line in 0..lines {
        q.clear();
        let mut next = 0usize;
        for pos in 0..len {
            let hi = (pos + r).min(len - 1);
            while next <= hi {
                let x = v[idx(line, next)];
                while q.back().is_some_and(|&b| better(x, v[idx(line, b)])) {
                    q.pop_back();
                }
                q.push_back(next);
                next += 1;
            }
            while q.front().is_some_and(|&a| a + r < pos) {
                q.pop_front();
            }
            if let Some(&a) = q.front() {
                out[idx(line, pos)] = v[idx(line, a)];
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_min_and_max() {
        let v = [5.0, 1.0, 4.0, 3.0, 9.0];
        assert_eq!(
            run(&v, 5, 1, 3, f32::min, true),
            vec![1.0, 1.0, 1.0, 3.0, 3.0]
        );
        assert_eq!(
            run(&v, 5, 1, 3, f32::max, true),
            vec![5.0, 5.0, 4.0, 9.0, 9.0]
        );
    }

    fn pt(x: f64, y: f64, z: f64, ret: (u8, u8)) -> Point {
        Point {
            x,
            y,
            z,
            return_number: ret.0,
            number_of_returns: ret.1,
            ..Default::default()
        }
    }

    /// Una colina suave que sube 6 m hacia el este, con `roofs`: (x0, x1, y0, y1, altura de la
    /// cumbrera o del tejado sobre el suelo, a dos aguas). Puntos cada medio metro.
    fn scene(roofs: &[(f64, f64, f64, f64, f64, bool)]) -> Vec<Point> {
        let mut pts = Vec::new();
        for i in 0..240 {
            for j in 0..240 {
                let (x, y) = (f64::from(i) * 0.5 + 0.25, f64::from(j) * 0.5 + 0.25);
                let ground = 100.0 + 6.0 * (x / 120.0);
                let mut z = ground;
                for &(x0, x1, y0, y1, h, pitched) in roofs {
                    if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                        // un tejado a dos aguas sube desde los aleros hasta la cumbrera a lo largo
                        // de x
                        let rise = if pitched {
                            h * (1.0 - ((x - (x0 + x1) / 2.0).abs() / ((x1 - x0) / 2.0)))
                        } else {
                            0.0
                        };
                        z = ground + if pitched { 3.0 + rise } else { h };
                    }
                }
                pts.push(pt(x, y, z, (1, 1)));
            }
        }
        pts
    }

    fn in_box(q: &Point, b: (f64, f64, f64, f64)) -> bool {
        (b.0..b.1).contains(&q.x) && (b.2..b.3).contains(&q.y)
    }

    #[test]
    fn a_box_is_a_building_and_a_hill_is_not() {
        let pts = scene(&[(20.0, 40.0, 50.0, 70.0, 8.0, false)]);
        let ground: Vec<bool> = pts.iter().map(|_| true).collect();
        let cls = classify(&pts, &ground, &BuildingParams::default());
        let roof: Vec<_> = pts
            .iter()
            .zip(&cls)
            .filter(|(q, _)| in_box(q, (20.0, 40.0, 50.0, 70.0)))
            .collect();
        let hit = roof.iter().filter(|(_, c)| **c == Some(BUILDING)).count();
        assert!(hit * 100 >= roof.len() * 95, "{hit} of {} roof points", roof.len());
        // nada fuera del tejado y sus aleros es un edificio
        let wrong = pts
            .iter()
            .zip(&cls)
            .filter(|(q, c)| **c == Some(BUILDING) && !in_box(q, (17.0, 43.0, 47.0, 73.0)))
            .count();
        assert_eq!(wrong, 0);
    }

    #[test]
    fn a_pitched_roof_with_a_ridge_is_one_building() {
        // 12 m de ancho, 20 m de largo, a dos aguas a lo largo de x: dos planos que se juntan en
        // una cumbrera
        let pts = scene(&[(30.0, 42.0, 50.0, 70.0, 3.0, true)]);
        let ground: Vec<bool> = pts.iter().map(|_| true).collect();
        let cls = classify(&pts, &ground, &BuildingParams::default());
        let roof: Vec<_> = pts
            .iter()
            .zip(&cls)
            .filter(|(q, _)| in_box(q, (30.0, 42.0, 50.0, 70.0)))
            .collect();
        let hit = roof.iter().filter(|(_, c)| **c == Some(BUILDING)).count();
        assert!(hit * 100 >= roof.len() * 90, "{hit} of {} roof points", roof.len());
    }

    #[test]
    fn a_crown_is_vegetation_not_a_building() {
        // una copa de 12 x 12 m a 9 m de altura: rugosa, y sus ecos son primeros de varios
        let mut pts = scene(&[]);
        let mut s = 99u64;
        let mut next = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / f64::from(1u32 << 31)
        };
        for q in &mut pts {
            if in_box(q, (50.0, 62.0, 50.0, 62.0)) {
                q.z += 6.0 + 4.0 * next();
                q.return_number = 1;
                q.number_of_returns = 3;
            }
        }
        let ground: Vec<bool> = pts.iter().map(|_| false).collect();
        let cls = classify(&pts, &ground, &BuildingParams::default());
        assert_eq!(cls.iter().filter(|c| **c == Some(BUILDING)).count(), 0);
        assert!(cls.iter().filter(|c| **c == Some(HIGH_VEGETATION)).count() > 0);
    }

    #[test]
    fn a_crown_over_a_roof_stays_vegetation() {
        let mut pts = scene(&[(20.0, 40.0, 50.0, 70.0, 8.0, false)]);
        let mut s = 7u64;
        let mut next = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / f64::from(1u32 << 31)
        };
        // puntos de copa a 4 a 7 m por encima del tejado, primeros de varios ecos, sobre una
        // esquina de él
        let mut crown = Vec::new();
        for q in &pts {
            if in_box(q, (30.0, 36.0, 60.0, 66.0)) {
                crown.push(pt(q.x, q.y, q.z + 4.0 + 3.0 * next(), (1, 3)));
            }
        }
        let n = pts.len();
        pts.extend(crown);
        let ground: Vec<bool> = pts.iter().map(|_| false).collect();
        let cls = classify(&pts, &ground, &BuildingParams::default());
        let crown_building = cls[n..].iter().filter(|c| **c == Some(BUILDING)).count();
        assert_eq!(crown_building, 0, "crown points taken for roof");
        let roof_taken = cls[..n]
            .iter()
            .zip(&pts)
            .filter(|(c, q)| **c == Some(BUILDING) && in_box(q, (20.0, 40.0, 50.0, 70.0)))
            .count();
        assert!(roof_taken > 1400, "{roof_taken} roof points");
    }
}
