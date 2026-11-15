//! Rutas de menor coste sobre un modelo del terreno.
//!
//! Cada celda tiene un coste de cruzarla; moverse entre vecinas (ocho) cuesta la distancia por el
//! coste medio de las dos celdas, y la ruta es la de menor total (Dijkstra). Las celdas vacías y
//! las que están fuera de los límites (demasiado empinadas, fuera del rango de elevaciones, dentro
//! de una barrera) no se pueden cruzar. El coste sale de la pendiente, de un ráster de costes del
//! usuario o del tiempo de marcha de la función de senderismo de Tobler, que depende de la
//! dirección del paso (cuesta arriba, cuesta abajo) y da segundos.
//!
//! Moverse en ocho direcciones favorece las direcciones rectas y diagonales: una ruta entre dos
//! puntos es como mucho un 8 % más larga que la línea recta y se pega a la rejilla. Las áreas
//! preferidas reducen el coste en el factor dado.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::PathBuf;

use crate::grid::Grid;
use crate::hydro::{neighbour, reach};
use crate::terrain::{self, Edges};
use crate::vector;
use crate::{Error, Result};

/// Cómo se valora cruzar una celda.
#[derive(Clone, Debug, PartialEq)]
pub enum CostModel {
    /// `1 + (pendiente / referencia) ^ potencia` por metro, con la pendiente en porcentaje.
    Slope { power: f64, reference_pct: f64 },
    /// Tiempo de marcha de Tobler (segundos) sobre la superficie; cuesta arriba y cuesta abajo
    /// difieren.
    Tobler,
    /// Un ráster con el coste por metro de cada celda (sobre las celdas del modelo).
    Raster(PathBuf),
    /// Cada celda cuesta 1 por metro: la ruta más corta.
    Uniform,
}

#[derive(Clone, Debug, Default)]
pub struct Constraints {
    /// Las celdas más empinadas que esto no se pueden cruzar (por ciento).
    pub max_slope_pct: Option<f64>,
    /// Las celdas fuera de este rango de elevaciones no se pueden cruzar.
    pub elev: Option<(f64, f64)>,
    /// Polígonos que no se pueden cruzar.
    pub barriers: Option<PathBuf>,
    /// Polígonos donde el coste se multiplica por el factor (por debajo de 1: preferidos).
    pub prefer: Option<(PathBuf, f64)>,
}

pub struct PathJob {
    pub dtm: PathBuf,
    /// El origen, las paradas en orden y el destino (coordenadas del mapa).
    pub points: Vec<[f64; 2]>,
    pub model: CostModel,
    pub constraints: Constraints,
    /// Con varios destinos: el primer punto es el origen y los demás son alternativas; la ruta va
    /// al que cuesta menos.
    pub nearest: bool,
    /// Escribe también un ráster con el coste de llegar a cada celda desde el origen.
    pub cost_distance: bool,
    /// Ruta base de los resultados: `<salida>.shp` y `<salida>_coste.tif`.
    pub output: PathBuf,
}

impl PathJob {
    pub fn new(dtm: PathBuf, points: Vec<[f64; 2]>, output: PathBuf) -> Self {
        Self {
            dtm,
            points,
            model: CostModel::Slope {
                power: 2.0,
                reference_pct: 10.0,
            },
            constraints: Constraints::default(),
            nearest: false,
            cost_distance: false,
            output,
        }
    }
}

/// Una ruta y a cuánto asciende.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub pts: Vec<[f64; 3]>,
    pub length_2d: f64,
    pub length_3d: f64,
    pub cost: f64,
    pub mean_slope: f64,
    pub max_slope: f64,
    pub climb: f64,
    pub descent: f64,
}

/// Lo que cuesta un paso entre dos celdas, o `None` cuando no se puede dar.
pub trait Steps {
    fn step(&self, from: usize, to: usize, k: usize) -> Option<f64>;
}

/// Un coste por metro en cada celda; NaN no se puede cruzar.
struct PerMetre<'a>(&'a Grid);

impl Steps for PerMetre<'_> {
    fn step(&self, a: usize, b: usize, k: usize) -> Option<f64> {
        let (ca, cb) = (self.0.data[a], self.0.data[b]);
        if ca.is_nan() || cb.is_nan() {
            return None;
        }
        Some(self.0.cell * reach(k) * f64::from(ca + cb) / 2.0)
    }
}

/// La función de senderismo de Tobler sobre la superficie, por un factor de cada celda (NaN:
/// bloqueada).
struct Tobler<'a> {
    z: &'a Grid,
    factor: &'a [f32],
}

impl Steps for Tobler<'_> {
    fn step(&self, a: usize, b: usize, k: usize) -> Option<f64> {
        let (za, zb) = (self.z.data[a], self.z.data[b]);
        let (fa, fb) = (self.factor[a], self.factor[b]);
        if za.is_nan() || zb.is_nan() || fa.is_nan() || fb.is_nan() {
            return None;
        }
        let d = self.z.cell * reach(k);
        let dz = f64::from(zb - za);
        let slope = dz / d;
        // km/h; la distancia sobre la superficie en metros
        let speed = 6.0 * (-3.5 * (slope + 0.05).abs()).exp();
        let surface = d.hypot(dz);
        Some(surface / (speed * 1000.0 / 3600.0) * f64::from(fa + fb) / 2.0)
    }
}

/// El menor coste de llegar a cada celda desde `from` y el camino de vuelta (la dirección, de 0 a
/// 7, de la celda anterior); se detiene en el primero de `targets` que fija, si hay alguno.
/// Devuelve los costes, las direcciones y ese destino.
pub fn dijkstra(
    g: &Grid,
    steps: &dyn Steps,
    from: usize,
    targets: &[usize],
) -> Result<(Vec<f64>, Vec<u8>, Option<usize>)> {
    let n = g.data.len();
    let mut cost = vec![f64::INFINITY; n];
    let mut back = vec![255u8; n];
    let mut heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::new();
    let index = |i: usize| -> Result<u32> {
        u32::try_from(i).map_err(|_| Error::Param("el ráster tiene demasiadas celdas".into()))
    };
    cost[from] = 0.0;
    heap.push(Reverse((0.0f64.to_bits(), index(from)?)));
    let mut popped = 0usize;
    while let Some(Reverse((bits, i))) = heap.pop() {
        let i = i as usize;
        // los decimales positivos se ordenan como sus bits
        if f64::from_bits(bits) > cost[i] {
            continue;
        }
        popped += 1;
        if popped.is_multiple_of(1 << 20) {
            crate::check_cancel()?;
        }
        if targets.contains(&i) {
            return Ok((cost, back, Some(i)));
        }
        for k in 0..8 {
            let Some(j) = neighbour(g, i, k) else {
                continue;
            };
            let Some(s) = steps.step(i, j, k) else {
                continue;
            };
            let c = cost[i] + s;
            if c < cost[j] {
                cost[j] = c;
                back[j] = ((k + 4) % 8) as u8;
                heap.push(Reverse((c.to_bits(), index(j)?)));
            }
        }
    }
    Ok((cost, back, None))
}

/// Las celdas de la ruta de `from` a `to` (ambas incluidas), a partir de las direcciones de vuelta.
fn trace(g: &Grid, back: &[u8], from: usize, to: usize) -> Vec<usize> {
    let mut cells = vec![to];
    let mut at = to;
    while at != from {
        let Some(next) = neighbour(g, at, usize::from(back[at])) else {
            break;
        };
        cells.push(next);
        at = next;
    }
    cells.reverse();
    cells
}

/// La ruta más barata de la celda `from` a la celda `to` sobre una rejilla de coste por metro (NaN
/// no se puede cruzar): las celdas y el coste total.
pub fn route(cost: &Grid, from: usize, to: usize) -> Result<(Vec<usize>, f64)> {
    let (c, back, hit) = dijkstra(cost, &PerMetre(cost), from, &[to])?;
    if hit.is_none() {
        return Err(Error::Param("no hay ruta entre esos puntos".into()));
    }
    Ok((trace(cost, &back, from, to), c[to]))
}

/// Los estadísticos de una ruta sobre las celdas `cells`, con las alturas de `z`.
fn describe(z: &Grid, cells: &[usize], cost: f64) -> Route {
    let pts: Vec<[f64; 3]> = cells
        .iter()
        .map(|&i| {
            let (x, y) = z.center(i % z.cols, i / z.cols);
            [x, y, f64::from(z.data[i])]
        })
        .collect();
    let (mut l2, mut l3, mut climb, mut descent, mut abs_dz, mut max_slope) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0f64);
    for w in pts.windows(2) {
        let d = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        let dz = w[1][2] - w[0][2];
        l2 += d;
        l3 += d.hypot(dz);
        abs_dz += dz.abs();
        if dz > 0.0 {
            climb += dz;
        } else {
            descent -= dz;
        }
        if d > 0.0 {
            max_slope = max_slope.max(100.0 * dz.abs() / d);
        }
    }
    Route {
        pts,
        length_2d: l2,
        length_3d: l3,
        cost,
        mean_slope: if l2 > 0.0 { 100.0 * abs_dz / l2 } else { 0.0 },
        max_slope,
        climb,
        descent,
    }
}

fn marked(path: &std::path::Path) -> Result<Vec<vector::Region>> {
    crate::volume::read_regions(path)
}

/// El coste por metro de cada celda para el trabajo (NaN donde no se puede cruzar) y, para el
/// modelo de Tobler, el factor de las áreas preferidas por sí solo.
fn surface_for(g: &Grid, job: &PathJob) -> Result<Vec<f32>> {
    let n = g.data.len();
    let slope = terrain::slope(g, 1.0, true, Edges::RepeatEdge);
    let mut f: Vec<f32> = match &job.model {
        CostModel::Uniform | CostModel::Tobler => g
            .data
            .iter()
            .map(|v| if v.is_nan() { f32::NAN } else { 1.0 })
            .collect(),
        CostModel::Slope {
            power,
            reference_pct,
        } => {
            if !(*reference_pct > 0.0) || !(*power > 0.0) {
                return Err(Error::Param("el modelo de pendiente pide una potencia y una pendiente de referencia mayores que 0".into()));
            }
            g.data
                .iter()
                .zip(&slope.data)
                .map(|(z, s)| {
                    if z.is_nan() {
                        f32::NAN
                    } else if s.is_nan() {
                        1.0
                    } else {
                        (1.0 + (f64::from(*s) / reference_pct).powf(*power)) as f32
                    }
                })
                .collect()
        }
        CostModel::Raster(p) => {
            let r = Grid::read(p)?;
            if r.cols != g.cols || r.rows != g.rows || (r.cell - g.cell).abs() > g.cell * 1e-6 {
                return Err(Error::Param(
                    "el ráster de costes no tiene las mismas celdas que el modelo: igualarlos antes".into(),
                ));
            }
            r.data
                .iter()
                .zip(&g.data)
                .map(|(c, z)| {
                    if z.is_nan() || !(*c > 0.0) {
                        f32::NAN
                    } else {
                        *c
                    }
                })
                .collect()
        }
    };
    let c = &job.constraints;
    for (i, cost) in f.iter_mut().enumerate().take(n) {
        if cost.is_nan() {
            continue;
        }
        let blocked_by_slope = c
            .max_slope_pct
            .is_some_and(|m| f64::from(slope.data[i]) > m);
        let outside = c
            .elev
            .is_some_and(|(lo, hi)| f64::from(g.data[i]) < lo || f64::from(g.data[i]) > hi);
        if blocked_by_slope || outside {
            *cost = f32::NAN;
        }
    }
    if let Some(p) = &c.barriers {
        for region in marked(p)? {
            mark(g, &region, &mut f, |_| f32::NAN);
        }
    }
    if let Some((p, factor)) = &c.prefer {
        if !(*factor > 0.0) {
            return Err(Error::Param(
                "el factor de las zonas preferidas debe ser mayor que 0".into(),
            ));
        }
        let k = *factor as f32;
        for region in marked(p)? {
            mark(g, &region, &mut f, |v| v * k);
        }
    }
    Ok(f)
}

/// Aplica `f` a las celdas cuyo centro está dentro de `region`.
fn mark(g: &Grid, region: &vector::Region, f: &mut [f32], op: impl Fn(f32) -> f32) {
    let b = region.bbox();
    let top = g.y0 + g.rows as f64 * g.cell;
    let lim = |v: f64, n: usize| v.clamp(0.0, n as f64) as usize;
    let (c0, c1) = (
        lim(((b[0] - g.x0) / g.cell).floor(), g.cols),
        lim(((b[2] - g.x0) / g.cell).ceil(), g.cols),
    );
    let (r0, r1) = (
        lim(((top - b[3]) / g.cell).floor(), g.rows),
        lim(((top - b[1]) / g.cell).ceil(), g.rows),
    );
    for r in r0..r1 {
        for c in c0..c1 {
            let (x, y) = g.center(c, r);
            let i = r * g.cols + c;
            if !f[i].is_nan() && region.contains(x, y) {
                f[i] = op(f[i]);
            }
        }
    }
}

/// Halla la ruta del trabajo sobre `g`: la ruta y, con `cost_distance`, el ráster de costes.
pub fn find(g: &Grid, job: &PathJob) -> Result<(Route, Option<Grid>)> {
    if job.points.len() < 2 {
        return Err(Error::Param("hacen falta un origen y un destino".into()));
    }
    let cell_of = |p: &[f64; 2], what: &str| -> Result<usize> {
        g.index(p[0], p[1])
            .filter(|&i| !g.data[i].is_nan())
            .ok_or_else(|| {
                Error::Param(format!(
                    "{what} ({}, {}) cae fuera del modelo o en una celda sin valor",
                    p[0], p[1]
                ))
            })
    };
    let cells: Vec<usize> = job
        .points
        .iter()
        .enumerate()
        .map(|(k, p)| {
            cell_of(
                p,
                &if k == 0 {
                    "el origen".to_string()
                } else {
                    format!("el punto {}", k + 1)
                },
            )
        })
        .collect::<Result<_>>()?;
    let f = surface_for(g, job)?;
    let grid_f = {
        let mut x = g.like();
        x.data.copy_from_slice(&f);
        x
    };
    let tobler = Tobler { z: g, factor: &f };
    let per_metre = PerMetre(&grid_f);
    let steps: &dyn Steps = if job.model == CostModel::Tobler {
        &tobler
    } else {
        &per_metre
    };
    for (k, &c) in cells.iter().enumerate() {
        if f[c].is_nan() {
            return Err(Error::Param(format!(
                "el punto {} cae en una zona que no se puede cruzar (barrera, pendiente o cota fuera de límites)",
                k + 1
            )));
        }
    }
    let mut path: Vec<usize> = Vec::new();
    let mut total = 0.0;
    if job.nearest {
        let (c, back, hit) = dijkstra(g, steps, cells[0], &cells[1..])?;
        let Some(to) = hit else {
            return Err(Error::Param("no hay ruta a ninguno de los destinos".into()));
        };
        total = c[to];
        path = trace(g, &back, cells[0], to);
    } else {
        for w in cells.windows(2) {
            crate::check_cancel()?;
            let (c, back, hit) = dijkstra(g, steps, w[0], &[w[1]])?;
            if hit.is_none() {
                return Err(Error::Param("no hay ruta entre esos puntos".into()));
            }
            total += c[w[1]];
            let part = trace(g, &back, w[0], w[1]);
            if path.is_empty() {
                path = part;
            } else {
                path.extend(part.into_iter().skip(1));
            }
        }
    }
    let distance = if job.cost_distance {
        let (c, _, _) = dijkstra(g, steps, cells[0], &[])?;
        let mut out = g.like();
        for (o, v) in out.data.iter_mut().zip(&c) {
            if v.is_finite() {
                *o = *v as f32;
            }
        }
        Some(out)
    } else {
        None
    };
    Ok((describe(g, &path, total), distance))
}

/// Halla la ruta y la escribe como polilínea 3D; devuelve el resumen.
pub fn run(job: &PathJob) -> Result<String> {
    if let Ok(pyr) = crate::pyramid::Pyramid::open(&job.dtm)
        && let Some(mem) = crate::copc_cache::total_memory()
    {
        let e = pyr.extent();
        let cells = ((e[2] - e[0]) / pyr.cell) * ((e[3] - e[1]) / pyr.cell);
        // el modelo, el coste, el coste pendiente (8 bytes), el camino de vuelta y la pendiente
        let need = cells * 32.0;
        if need > mem as f64 * 0.7 {
            return Err(Error::Param(format!(
                "buscar una ruta sobre {:.0} millones de celdas necesita unos {:.0} GB de memoria (este equipo tiene {:.0} GB): recorta el modelo a la zona de la ruta o usa una celda mayor",
                cells / 1e6,
                need / 1e9,
                mem as f64 / 1e9
            )));
        }
    }
    let g = Grid::read(&job.dtm)?;
    let epsg = Grid::read_epsg(&job.dtm);
    let (r, distance) = find(&g, job)?;
    use crate::vector::{ColKind, Table, Val, prj_for_epsg, write_polylines};
    let table = Table {
        columns: vec![
            ("ID".into(), ColKind::Num(4, 0)),
            ("LONG_2D".into(), ColKind::Num(12, 2)),
            ("LONG_3D".into(), ColKind::Num(12, 2)),
            ("COSTE".into(), ColKind::Num(14, 2)),
            ("PEND_MED".into(), ColKind::Num(8, 2)),
            ("PEND_MAX".into(), ColKind::Num(8, 2)),
            ("ASCENSO".into(), ColKind::Num(10, 2)),
            ("DESCENSO".into(), ColKind::Num(10, 2)),
        ],
        rows: vec![vec![
            Val::Num(1.0),
            Val::Num(r.length_2d),
            Val::Num(r.length_3d),
            Val::Num(r.cost),
            Val::Num(r.mean_slope),
            Val::Num(r.max_slope),
            Val::Num(r.climb),
            Val::Num(r.descent),
        ]],
    };
    crate::paths::ensure_parent(&job.output)?;
    let path = crate::paths::swap_ext(&job.output, "shp");
    write_polylines(
        &path,
        std::slice::from_ref(&r.pts),
        true,
        &table,
        epsg.and_then(prj_for_epsg).as_deref(),
    )?;
    let cost_text = if job.model == CostModel::Tobler {
        let m = (r.cost / 60.0).round() as u64;
        format!("{} h {:02} min a pie", m / 60, m % 60)
    } else {
        format!("coste {:.1}", r.cost)
    };
    let mut lines = vec![format!(
        "ruta de {:.0} m en planta ({:.0} m sobre el terreno), {cost_text}; pendiente media {:.1} %, máxima {:.1} %; sube {:.0} m y baja {:.0} m -> {}",
        r.length_2d,
        r.length_3d,
        r.mean_slope,
        r.max_slope,
        r.climb,
        r.descent,
        path.display()
    )];
    if let Some(d) = distance {
        let p = crate::paths::swap_ext(&job.output, "tif");
        let p = p.with_file_name(format!("{}_coste.tif", crate::paths::stem(&p)));
        d.write_tiff(&p, epsg)?;
        lines.push(format!(
            "  coste acumulado desde el origen: {}",
            p.display()
        ));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cols: usize, rows: usize, f: impl Fn(usize, usize) -> f32) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 1.0, cols, rows)?;
        for r in 0..rows {
            for c in 0..cols {
                g.data[r * cols + c] = f(c, r);
            }
        }
        Ok(g)
    }

    fn cell(g: &Grid, c: usize, r: usize) -> usize {
        r * g.cols + c
    }

    #[test]
    fn with_a_uniform_cost_the_route_is_the_shortest_on_the_grid() -> Result<()> {
        let g = grid(20, 20, |_, _| 1.0)?;
        let (cells, cost) = route(&g, cell(&g, 0, 0), cell(&g, 10, 4))?;
        // cuatro pasos diagonales y seis rectos
        let exact = 6.0 + 4.0 * std::f64::consts::SQRT_2;
        assert!((cost - exact).abs() < 1e-9, "{cost} / {exact}");
        assert_eq!(cells.len(), 11);
        assert_eq!(cells[0], cell(&g, 0, 0));
        assert_eq!(cells[10], cell(&g, 10, 4));
        Ok(())
    }

    #[test]
    fn a_wall_with_a_gap_is_crossed_by_the_gap() -> Result<()> {
        let g = grid(
            21,
            21,
            |c, r| if c == 10 && r != 15 { f32::NAN } else { 1.0 },
        )?;
        let (cells, _) = route(&g, cell(&g, 0, 10), cell(&g, 20, 10))?;
        assert!(cells.contains(&cell(&g, 10, 15)));
        assert!(cells.iter().all(|&i| !g.data[i].is_nan()));
        Ok(())
    }

    #[test]
    fn a_destination_closed_in_by_empty_cells_has_no_route() -> Result<()> {
        let g = grid(9, 9, |c, r| {
            if (3..=5).contains(&c) && (3..=5).contains(&r) && !(c == 4 && r == 4) {
                f32::NAN
            } else {
                1.0
            }
        })?;
        let e = route(&g, cell(&g, 0, 0), cell(&g, 4, 4))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(e.contains("no hay ruta"), "{e}");
        Ok(())
    }

    /// El coste de cada celda relajando cada arista hasta que nada mejora.
    fn bellman_ford(g: &Grid, from: usize) -> Vec<f64> {
        let mut c = vec![f64::INFINITY; g.data.len()];
        c[from] = 0.0;
        loop {
            let mut changed = false;
            for i in 0..g.data.len() {
                if !c[i].is_finite() {
                    continue;
                }
                for k in 0..8 {
                    if let Some(j) = neighbour(g, i, k)
                        && let Some(s) = PerMetre(g).step(i, j, k)
                        && c[i] + s < c[j] - 1e-15
                    {
                        c[j] = c[i] + s;
                        changed = true;
                    }
                }
            }
            if !changed {
                return c;
            }
        }
    }

    #[test]
    fn it_agrees_with_bellman_ford_on_random_costs_and_is_symmetric() -> Result<()> {
        let mut state = 99u64;
        let mut rnd = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        for _ in 0..30 {
            let mut g = grid(8, 8, |_, _| 1.0)?;
            for v in &mut g.data {
                *v = match rnd() % 10 {
                    0 => f32::NAN,
                    k => 0.5 + k as f32 / 3.0,
                };
            }
            let (a, b) = (cell(&g, 0, 0), cell(&g, 7, 7));
            g.data[a] = 1.0;
            g.data[b] = 1.0;
            let oracle = bellman_ford(&g, a);
            let found = route(&g, a, b);
            match found {
                Ok((_, cost)) => {
                    assert!((cost - oracle[b]).abs() < 1e-9, "{cost} / {}", oracle[b]);
                    // el coste con un precio simétrico es el mismo en ambos sentidos
                    let (_, back) = route(&g, b, a)?;
                    assert!((back - cost).abs() < 1e-9);
                }
                Err(_) => assert!(oracle[b].is_infinite()),
            }
        }
        Ok(())
    }

    #[test]
    fn walking_time_avoids_a_steep_hill_that_the_shortest_route_climbs() -> Result<()> {
        // suelo llano con un cono empinado en medio
        let g = grid(41, 41, |c, r| {
            let d = ((c as f64 - 20.0).powi(2) + (r as f64 - 20.0).powi(2)).sqrt();
            (12.0 - d).max(0.0) as f32 * 2.0
        })?;
        let pts = vec![[0.5, 20.5], [40.5, 20.5]];
        let mut j = PathJob::new(PathBuf::new(), pts, PathBuf::new());
        j.model = CostModel::Uniform;
        let (short, _) = find(&g, &j)?;
        j.model = CostModel::Tobler;
        let (walk, _) = find(&g, &j)?;
        assert!(short.max_slope > 100.0, "{}", short.max_slope);
        assert!(
            walk.max_slope < short.max_slope / 2.0,
            "{} / {}",
            walk.max_slope,
            short.max_slope
        );
        assert!(walk.length_2d > short.length_2d);
        Ok(())
    }

    #[test]
    fn a_stop_never_makes_the_route_shorter() -> Result<()> {
        let g = grid(30, 30, |_, _| 0.0)?;
        let mut j = PathJob::new(
            PathBuf::new(),
            vec![[0.5, 0.5], [29.5, 0.5]],
            PathBuf::new(),
        );
        j.model = CostModel::Uniform;
        let (direct, _) = find(&g, &j)?;
        j.points = vec![[0.5, 0.5], [15.5, 20.5], [29.5, 0.5]];
        let (via, _) = find(&g, &j)?;
        assert!(via.cost >= direct.cost && via.length_2d > direct.length_2d);
        // el más cercano de dos destinos
        j.points = vec![[0.5, 0.5], [29.5, 29.5], [5.5, 0.5]];
        j.nearest = true;
        let (near, _) = find(&g, &j)?;
        assert!((near.length_2d - 5.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn limits_barriers_and_preferred_areas_shape_the_route() -> Result<()> {
        let g = grid(30, 30, |c, _| c as f32 * 0.0)?;
        let dir = std::env::temp_dir().join(format!("hipso_path_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // una barrera en el camino, de y = 0 a y = 24
        let layer = crate::geomops::PolyLayer {
            features: vec![crate::geomops::Feature {
                geom: crate::geomops::multipolygon_of(&[vec![
                    [14.0, 0.0],
                    [16.0, 0.0],
                    [16.0, 24.0],
                    [14.0, 24.0],
                ]]),
                attrs: Vec::new(),
            }],
            prj: None,
        };
        crate::geomops::write_polygons(&layer, &dir.join("b.shp"))?;
        let mut j = PathJob::new(
            PathBuf::new(),
            vec![[0.5, 5.5], [29.5, 5.5]],
            PathBuf::new(),
        );
        j.model = CostModel::Uniform;
        j.constraints.barriers = Some(dir.join("b.shp"));
        let (r, _) = find(&g, &j)?;
        assert!(
            r.pts.iter().any(|p| p[1] > 24.0),
            "the route has to go around"
        );
        // un punto dentro de la barrera no puede ser un inicio
        j.points = vec![[15.0, 5.5], [29.5, 5.5]];
        assert!(find(&g, &j).is_err());
        // el límite de pendiente
        let steep = grid(10, 10, |c, _| c as f32 * 2.0)?;
        let mut s = PathJob::new(PathBuf::new(), vec![[0.5, 5.5], [9.5, 5.5]], PathBuf::new());
        s.constraints.max_slope_pct = Some(50.0);
        assert!(find(&steep, &s).is_err());
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn the_route_is_written_with_the_cost_raster() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_route_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        grid(30, 30, |c, _| c as f32 * 0.2)?.write_tiff(&dir.join("d.tif"), Some(25829))?;
        let mut j = PathJob::new(
            dir.join("d.tif"),
            vec![[0.5, 5.5], [29.5, 20.5]],
            dir.join("ruta.shp"),
        );
        j.cost_distance = true;
        let msg = run(&j)?;
        assert!(msg.starts_with("ruta de "), "{msg}");
        assert!(dir.join("ruta.shp").exists() && dir.join("ruta_coste.tif").exists());
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
