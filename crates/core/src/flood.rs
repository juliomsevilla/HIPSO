//! Flood simulation on a terrain model.
//!
//! Instead of flooding level by level, the lowest water level at which each cell can be reached
//! from the seeds is worked out once (a priority flood that spreads from the seeds over the
//! lowest cells first: the cell takes the larger of the level that arrived and its own height).
//! Then, for any level `L`, a cell is flooded when its reach is at most `L`, and the water over
//! it is `L - z`; a series of levels costs almost nothing more than one.
//!
//! With connectivity, water only fills what can be reached from the seeds over cells that are
//! not higher than the level: a pit behind a ridge stays dry. The bathtub model (`Seeds::All`)
//! has no connectivity: every cell at or under the level is flooded, whether water could get
//! there or not. It shows the depressions of a model but overstates a real flood.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::PathBuf;

use crate::grid::Grid;
use crate::hydro::{key, neighbour};
use crate::obia::segment::label_components;
use crate::{Error, Result};

/// Memory a cell takes while flooding (the model, the reach, the visited mark and the queue).
const BYTES_PER_CELL: f64 = 20.0;

/// Where the water comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Seeds {
    /// Points on the map (a river, a breach, the sea): each must fall on a cell with value.
    Points(Vec<[f64; 2]>),
    /// Every cell on the edge of the raster, or next to an empty cell.
    Border,
    /// No connectivity: the bathtub model.
    All,
}

/// What the levels are measured from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relative {
    /// The level is an elevation.
    Absolute,
    /// The level is added to the lowest elevation of the model.
    MinZ,
    /// The level is added to the elevation of the first seed.
    Seed,
}

pub struct FloodJob {
    pub dtm: PathBuf,
    pub seeds: Seeds,
    pub levels: Vec<f64>,
    pub relative: Relative,
    /// Cells let water through their 4 sides (4) or also their corners (8).
    pub connectivity: u8,
    /// Base path of the results: `<output>.csv`, `<output>_alcance.tif`, `<output>_<nivel>.tif`…
    pub output: PathBuf,
    /// A raster of the water depth for each level.
    pub depth: bool,
    /// A polygon layer with the flooded area of each level.
    pub extent: bool,
    pub csv: bool,
}

impl FloodJob {
    pub fn new(dtm: PathBuf, seeds: Seeds, levels: Vec<f64>, output: PathBuf) -> Self {
        Self {
            dtm,
            seeds,
            levels,
            relative: Relative::Absolute,
            connectivity: 8,
            output,
            depth: true,
            extent: false,
            csv: true,
        }
    }
}

/// The lowest water level that reaches each cell from `seeds` (cell indices), or empty where
/// it cannot be reached (cells with no value, and what the empty cells cut off).
pub fn reach_level(g: &Grid, seeds: &[usize], conn: u8) -> Result<Grid> {
    let n = g.data.len();
    let mut reach = g.like();
    let mut seen = vec![false; n];
    let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
    let index = |i: usize| -> Result<u32> {
        u32::try_from(i).map_err(|_| Error::Param("el ráster tiene demasiadas celdas".into()))
    };
    for &s in seeds {
        let Some(z) = g.data.get(s).copied().filter(|z| !z.is_nan()) else {
            continue;
        };
        if !seen[s] {
            seen[s] = true;
            reach.data[s] = z;
            heap.push(Reverse((key(z), index(s)?)));
        }
    }
    // the four sides are the even directions (E, S, W, N)
    let step = if conn == 4 { 2 } else { 1 };
    let mut popped = 0usize;
    while let Some(Reverse((_, i))) = heap.pop() {
        popped += 1;
        if popped.is_multiple_of(1 << 20) {
            crate::check_cancel()?;
        }
        let i = i as usize;
        let a = reach.data[i];
        for k in (0..8).step_by(step) {
            let Some(j) = neighbour(g, i, k) else {
                continue;
            };
            let z = g.data[j];
            if seen[j] || z.is_nan() {
                continue;
            }
            seen[j] = true;
            let r = a.max(z);
            reach.data[j] = r;
            heap.push(Reverse((key(r), index(j)?)));
        }
    }
    Ok(reach)
}

/// The cell indices the seeds stand for.
pub fn seed_cells(g: &Grid, seeds: &Seeds) -> Result<Vec<usize>> {
    match seeds {
        Seeds::Points(pts) => {
            if pts.is_empty() {
                return Err(Error::Param(
                    "falta al menos un punto de origen del agua".into(),
                ));
            }
            pts.iter()
                .map(|p| {
                    g.index(p[0], p[1])
                        .filter(|&i| !g.data[i].is_nan())
                        .ok_or_else(|| {
                            Error::Param(format!(
                                "el punto de origen ({}, {}) cae fuera del modelo o en una celda sin valor",
                                p[0], p[1]
                            ))
                        })
                })
                .collect()
        }
        Seeds::Border => {
            let cells: Vec<usize> = (0..g.data.len())
                .filter(|&i| {
                    !g.data[i].is_nan()
                        && (0..8).any(|k| neighbour(g, i, k).is_none_or(|j| g.data[j].is_nan()))
                })
                .collect();
            if cells.is_empty() {
                return Err(Error::Param("el modelo no tiene borde con valor".into()));
            }
            Ok(cells)
        }
        Seeds::All => Ok((0..g.data.len()).filter(|&i| !g.data[i].is_nan()).collect()),
    }
}

/// The water of one level.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelStats {
    pub level: f64,
    /// Flooded area (m²) and volume (m³).
    pub area: f64,
    pub volume: f64,
    pub mean_depth: f64,
    pub max_depth: f64,
    /// Separate bodies of water.
    pub basins: u32,
}

/// The depth of the water at `level` (empty where it is dry) and what it adds up to.
pub fn at_level(g: &Grid, reach: &Grid, level: f64, conn: u8) -> Result<(Grid, LevelStats)> {
    let mut depth = g.like();
    let cell_area = g.cell * g.cell;
    let (mut n, mut vol, mut max) = (0u64, 0.0, 0.0f64);
    for (d, (z, r)) in depth.data.iter_mut().zip(g.data.iter().zip(&reach.data)) {
        if !r.is_nan() && f64::from(*r) <= level {
            let w = level - f64::from(*z);
            *d = w as f32;
            n += 1;
            vol += w * cell_area;
            max = max.max(w);
        }
    }
    let (_, basins) = label_components(
        g.cols,
        g.rows,
        conn == 8,
        |i| !depth.data[i].is_nan(),
        |_, _| true,
    )?;
    Ok((
        depth,
        LevelStats {
            level,
            area: n as f64 * cell_area,
            volume: vol,
            mean_depth: if n > 0 {
                vol / cell_area / n as f64
            } else {
                0.0
            },
            max_depth: max,
            basins,
        },
    ))
}

fn level_name(level: f64) -> String {
    let t = format!("{level:.3}");
    t.trim_end_matches('0')
        .trim_end_matches('.')
        .replace('.', "_")
        .replace('-', "m")
}

/// Runs the simulation and writes what was asked; returns the summary.
pub fn run(job: &FloodJob) -> Result<String> {
    if job.levels.is_empty() || job.levels.iter().any(|l| !l.is_finite()) {
        return Err(Error::Param("falta al menos un nivel de agua".into()));
    }
    if job.levels.len() > 1000 {
        return Err(Error::Param(
            "hay demasiados niveles (1000 como máximo)".into(),
        ));
    }
    if job.connectivity != 4 && job.connectivity != 8 {
        return Err(Error::Param("la conectividad es 4 u 8".into()));
    }
    if let Ok(pyr) = crate::pyramid::Pyramid::open(&job.dtm)
        && let Some(mem) = crate::copc_cache::total_memory()
    {
        let e = pyr.extent();
        let cells = ((e[2] - e[0]) / pyr.cell) * ((e[3] - e[1]) / pyr.cell);
        let need = cells * BYTES_PER_CELL;
        if need > mem as f64 * 0.7 {
            return Err(Error::Param(format!(
                "la inundación sobre {:.0} millones de celdas necesita unos {:.0} GB de memoria (este equipo tiene {:.0} GB): el agua se sigue por todo el ráster a la vez. Usa una celda mayor o recorta la zona",
                cells / 1e6,
                need / 1e9,
                mem as f64 / 1e9
            )));
        }
    }
    let g = Grid::read(&job.dtm)?;
    let epsg = Grid::read_epsg(&job.dtm);
    let seeds = seed_cells(&g, &job.seeds)?;
    let zmin = g
        .data
        .iter()
        .copied()
        .filter(|v| !v.is_nan())
        .fold(f64::INFINITY, |a, v| a.min(f64::from(v)));
    let base = match job.relative {
        Relative::Absolute => 0.0,
        Relative::MinZ => zmin,
        Relative::Seed => f64::from(g.data[seeds[0]]),
    };
    let levels: Vec<f64> = job.levels.iter().map(|l| l + base).collect();
    crate::progress::begin(levels.len() as u64 + 1);
    let reach = reach_level(&g, &seeds, job.connectivity)?;
    crate::progress::advance(1);
    crate::paths::ensure_parent(&job.output)?;
    let mut lines = Vec::new();
    let tif = |tag: &str| {
        let p = crate::paths::swap_ext(&job.output, "tif");
        p.with_file_name(format!("{}_{tag}.tif", crate::paths::stem(&p)))
    };
    let p = tif("alcance");
    reach.write_tiff(&p, epsg)?;
    lines.push(format!(
        "  alcance (cota mínima de inundación): {}",
        p.display()
    ));
    let mut all = Vec::new();
    for &level in &levels {
        crate::check_cancel()?;
        let (depth, st) = at_level(&g, &reach, level, job.connectivity)?;
        if job.depth {
            let p = tif(&level_name(level));
            depth.write_tiff(&p, epsg)?;
        }
        if job.extent {
            let mut mask = g.like();
            for (m, d) in mask.data.iter_mut().zip(&depth.data) {
                if !d.is_nan() {
                    *m = 1.0;
                }
            }
            let shp = crate::paths::swap_ext(&job.output, "shp");
            let shp = shp.with_file_name(format!(
                "{}_{}.shp",
                crate::paths::stem(&shp),
                level_name(level)
            ));
            let mut v = crate::rastervec::VecJob::new(job.dtm.clone(), shp);
            v.diagonal = job.connectivity == 8;
            let data = crate::rastervec::vectorize(&mask, &v)?;
            let prj = epsg.and_then(crate::vector::prj_for_epsg);
            crate::vector::write_layer(
                &v.output,
                &data,
                crate::view::GeomKind::Polygon,
                prj.as_deref(),
            )?;
        }
        crate::progress::advance(1);
        all.push(st);
    }
    if job.depth {
        lines.push(format!(
            "  profundidad por nivel: {}",
            tif("<nivel>").display()
        ));
    }
    if job.csv {
        let p = crate::paths::swap_ext(&job.output, "csv");
        let mut s =
            String::from("Nivel_m,Area_m2,Volumen_m3,Prof_media_m,Prof_max_m,Masas_de_agua\n");
        for st in &all {
            s.push_str(&format!(
                "{:.3},{:.3},{:.3},{:.3},{:.3},{}\n",
                st.level, st.area, st.volume, st.mean_depth, st.max_depth, st.basins
            ));
        }
        std::fs::write(&p, s)?;
        lines.push(format!("  tabla: {}", p.display()));
    }
    let mut head = Vec::new();
    for st in &all {
        head.push(format!(
            "nivel {:.2}: {:.1} m² inundados, {:.2} m³, máx. {:.2} m, {} masa(s) de agua",
            st.level, st.area, st.volume, st.max_depth, st.basins
        ));
    }
    if matches!(job.seeds, Seeds::All) {
        head.push(
            "modelo de bañera, sin conectividad: se inunda todo lo que está bajo el nivel, llegue o no el agua".into(),
        );
    }
    head.extend(lines);
    Ok(head.join("\n"))
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

    /// Low ground on the left, a wall, and a pit lower than the water on the right.
    fn walled() -> Result<Grid> {
        grid(9, 9, |c, _| match c {
            0..=3 => 2.0,
            4 => 5.0,
            _ => 1.0,
        })
    }

    #[test]
    fn a_pit_behind_a_wall_stays_dry_unless_it_is_a_bathtub() -> Result<()> {
        let g = walled()?;
        let seed = seed_cells(&g, &Seeds::Points(vec![[0.5, 0.5]]))?;
        let reach = reach_level(&g, &seed, 8)?;
        let (depth, st) = at_level(&g, &reach, 3.0, 8)?;
        // the left half (4 x 9 cells) is under 1 m of water; the wall and the pit are dry
        assert_eq!(st.area, 36.0);
        assert!(depth.data[8].is_nan() && depth.data[4].is_nan());
        assert_eq!(st.basins, 1);
        let all = seed_cells(&g, &Seeds::All)?;
        let tub = reach_level(&g, &all, 8)?;
        let (_, st) = at_level(&g, &tub, 3.0, 8)?;
        // the pit is wet too: 4 x 9 and 4 x 9 cells, as two bodies of water
        assert_eq!(st.area, 72.0);
        assert_eq!(st.basins, 2);
        Ok(())
    }

    #[test]
    fn the_volume_of_a_paraboloid_bowl_is_the_formula() -> Result<()> {
        // z = r², cells of 0.1 m; at level 4 the water holds pi L² / 2 cubic metres
        let n = 80;
        let mut g = Grid::new(-4.0, -4.0, 0.1, n, n)?;
        for r in 0..n {
            for c in 0..n {
                let (x, y) = g.center(c, r);
                g.data[r * n + c] = (x * x + y * y) as f32;
            }
        }
        let seed = seed_cells(&g, &Seeds::Points(vec![[0.05, 0.05]]))?;
        let reach = reach_level(&g, &seed, 8)?;
        let (_, st) = at_level(&g, &reach, 4.0, 8)?;
        let exact = std::f64::consts::PI * 16.0 / 2.0;
        assert!(
            (st.volume / exact - 1.0).abs() < 0.02,
            "{} / {exact}",
            st.volume
        );
        assert_eq!(st.basins, 1);
        Ok(())
    }

    #[test]
    fn a_higher_level_floods_what_a_lower_one_does() -> Result<()> {
        let g = grid(12, 12, |c, r| ((c * 7 + r * 13) % 11) as f32)?;
        let seed = seed_cells(&g, &Seeds::Points(vec![[0.5, 0.5]]))?;
        let reach = reach_level(&g, &seed, 4)?;
        let mut before: Option<Grid> = None;
        for level in [2.0, 4.0, 6.0, 8.0, 10.0] {
            let (d, _) = at_level(&g, &reach, level, 4)?;
            if let Some(b) = &before {
                for (lo, hi) in b.data.iter().zip(&d.data) {
                    assert!(lo.is_nan() || !hi.is_nan());
                }
            }
            before = Some(d);
        }
        Ok(())
    }

    /// Water rising level by level, the slow way: the lowest level at which each cell is reached.
    fn slow_reach(g: &Grid, seeds: &[usize], conn: u8) -> Vec<f32> {
        let mut levels: Vec<f32> = g.data.iter().copied().filter(|v| !v.is_nan()).collect();
        levels.sort_by(f32::total_cmp);
        levels.dedup();
        let mut out = vec![f32::NAN; g.data.len()];
        for &l in &levels {
            let mut wet = vec![false; g.data.len()];
            let mut stack: Vec<usize> = seeds.iter().copied().filter(|&s| g.data[s] <= l).collect();
            for &s in &stack {
                wet[s] = true;
            }
            while let Some(i) = stack.pop() {
                for k in (0..8).step_by(if conn == 4 { 2 } else { 1 }) {
                    if let Some(j) = neighbour(g, i, k)
                        && !wet[j]
                        && g.data[j] <= l
                    {
                        wet[j] = true;
                        stack.push(j);
                    }
                }
            }
            for (o, w) in out.iter_mut().zip(&wet) {
                if *w && o.is_nan() {
                    *o = l;
                }
            }
        }
        out
    }

    #[test]
    fn it_agrees_with_the_slow_way_on_random_models() -> Result<()> {
        let mut state = 12345u64;
        let mut rnd = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) % 100) as f32 / 10.0
        };
        for round in 0..20 {
            let mut g = grid(8, 8, |_, _| 0.0)?;
            for v in &mut g.data {
                *v = rnd();
            }
            if round % 5 == 0 {
                g.data[10] = f32::NAN;
                g.data[27] = f32::NAN;
            }
            let seed: Vec<usize> = (0..g.data.len())
                .filter(|&i| !g.data[i].is_nan())
                .take(2)
                .collect();
            for conn in [4, 8] {
                let fast = reach_level(&g, &seed, conn)?;
                let slow = slow_reach(&g, &seed, conn);
                for (a, b) in fast.data.iter().zip(&slow) {
                    assert!(
                        (a.is_nan() && b.is_nan()) || a == b,
                        "round {round} conn {conn}: {a} / {b}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn seeds_out_of_the_model_are_refused() -> Result<()> {
        let mut g = walled()?;
        g.data[0] = f32::NAN;
        assert!(seed_cells(&g, &Seeds::Points(vec![[0.5, 8.5]])).is_err());
        assert!(seed_cells(&g, &Seeds::Points(vec![[50.0, 50.0]])).is_err());
        assert!(seed_cells(&g, &Seeds::Points(vec![])).is_err());
        assert!(seed_cells(&g, &Seeds::Border).is_ok());
        Ok(())
    }

    #[test]
    fn the_levels_can_be_relative_and_the_results_are_written() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_flood_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let g = walled()?;
        g.write_tiff(&dir.join("d.tif"), Some(25829))?;
        let mut job = FloodJob::new(
            dir.join("d.tif"),
            Seeds::Points(vec![[0.5, 0.5]]),
            vec![1.0, 2.0],
            dir.join("res.shp"),
        );
        // relative to the seed (2 m): levels 3 and 4
        job.relative = Relative::Seed;
        job.extent = true;
        let msg = run(&job)?;
        assert!(msg.contains("nivel 3.00: 36.0 m²"), "{msg}");
        let csv = std::fs::read_to_string(dir.join("res.csv"))?;
        assert_eq!(csv.lines().count(), 3);
        assert!(dir.join("res_alcance.tif").exists());
        assert!(dir.join("res_3.tif").exists() && dir.join("res_4.tif").exists());
        assert!(dir.join("res_3.shp").exists());
        job.levels.clear();
        assert!(run(&job).is_err());
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
