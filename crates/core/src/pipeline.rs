//! Procesamiento por teselas: ruido, suelo y edificios clasificados tesela a tesela, en paralelo, y
//! escritos como una nube clasificada o rasterizados en un MDT.

use crate::axelsson::{self, AxelssonParams};
use crate::buildings::{self, BuildingParams};
use crate::csf::{self, CsfParams};
use crate::extras::{self, Extras};
use crate::footprint::Footprint;
use crate::grid::{Grid, MeanGrid};
use crate::ground::{GroundFilter, GroundParams};
use crate::idw;
use crate::las::{self, Points, Reader, Writer};
use crate::mcc::{self, MccParams};
use crate::noise::{self, NoiseParams};
use crate::overlap;
use crate::pmf::{self, PmfParams};
use crate::rules::{self, ClassRule};
use crate::smrf::{self, SmrfParams};
use crate::tile::{Layout, Tile, Tiled};
use crate::{Error, Result, tin};

pub const NOISE: u8 = 7;
pub const GROUND: u8 = 2;
pub const UNCLASSIFIED: u8 = 1;

/// Cómo se encuentran los puntos de suelo.
#[derive(Clone, Debug, PartialEq)]
pub enum Ground {
    /// Kraus & Pfeifer iterative filter.
    Fusion(GroundParams),
    /// Multiscale Curvature Classification.
    Mcc(MccParams),
    /// Cloth Simulation Filter.
    Csf(CsfParams),
    /// Progressive Morphological Filter.
    Pmf(PmfParams),
    /// Simple Morphological Filter (Pingel et al. 2013).
    Smrf(SmrfParams),
    /// Progressive TIN densification (Axelsson).
    Axelsson(AxelssonParams),
    /// Los puntos ya clasificados como suelo (clase 2).
    Classes,
}

/// Cómo se encuentra el ruido.
#[derive(Clone, Debug, PartialEq)]
pub enum Noise {
    /// Puntos aislados en un bloque de celdas de 3 x 3 x 3.
    Isolated(NoiseParams),
    /// Eliminación estadística de valores atípicos sobre la distancia media a los k vecinos más
    /// próximos.
    Sor { k: usize, m: f64 },
    /// Ruido bajo: puntos muy por debajo de las alturas de su celda (mínimo local extendido).
    Elm { cell: f64, threshold: f64 },
}

/// Qué clasificar.
#[derive(Clone, Debug, PartialEq)]
pub struct Steps {
    /// Aparta antes que nada los puntos de las pasadas que solapan con otra (clase 12), con celdas
    /// de tantos metros.
    pub overlap: Option<f64>,
    pub noise: Option<Noise>,
    pub ground: Ground,
    pub buildings: Option<BuildingParams>,
    /// Estratos de vegetación, cables y postes.
    pub extras: Extras,
    /// Reglas sobre el eco y el ángulo de barrido, aplicadas al final y en orden.
    pub rules: Vec<ClassRule>,
}

/// División del trabajo en teselas.
#[derive(Clone, Debug, PartialEq)]
pub struct Tiling {
    pub size: f64,
    pub buffer: f64,
    /// 0: un hilo por núcleo.
    pub threads: usize,
}

impl Default for Tiling {
    fn default() -> Self {
        Self {
            size: 250.0,
            buffer: 30.0,
            threads: 0,
        }
    }
}

/// Cuántos puntos clasificó cada paso, sobre los puntos propios de las teselas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub tiles: usize,
    pub points: u64,
    pub noise: u64,
    pub ground: u64,
    pub buildings: u64,
    pub vegetation: u64,
    pub wires: u64,
    pub poles: u64,
    /// Puntos cuya clase cambió una regla sobre el eco o el ángulo de barrido.
    pub rules: u64,
}

/// Lo que decidieron los pasos para los puntos de una tesela.
struct TileClasses {
    /// La clase de cada punto (None: conserva la suya).
    class: Vec<Option<u8>>,
    /// Qué puntos son suelo, para el MDT.
    ground: Vec<bool>,
    /// Qué puntos cambió una regla sobre el eco o el ángulo de barrido.
    ruled: Vec<bool>,
}

/// La clase de cada punto de una tesela tras los pasos (None: conserva su clase) y cuáles son suelo
/// para el MDT.
fn classify_tile(work: &mut Points, s: &Steps) -> Result<TileClasses> {
    let n = work.pts.len();
    let mut class: Vec<Option<u8>> = vec![None; n];
    // Los puntos de solape se apartan primero y se dejan fuera de todo paso posterior, como el
    // ruido.
    if let Some(cell) = s.overlap {
        for (k, over) in overlap::find(&work.pts, cell).into_iter().enumerate() {
            if over {
                class[k] = Some(overlap::OVERLAP);
                if let Some(q) = work.pts.get_mut(k) {
                    q.withheld = true;
                }
            }
        }
    }
    // Los puntos que resultan ser ruido se marcan en su sitio, de modo que los pasos posteriores
    // los saltan.
    if let Some(method) = &s.noise {
        let flags = match method {
            Noise::Isolated(np) => noise::isolated(&work.pts, np),
            Noise::Sor { k, m } => noise::sor(&work.pts, *k, *m),
            Noise::Elm { cell, threshold } => noise::elm(&work.pts, *cell, *threshold),
        };
        for (k, iso) in flags.into_iter().enumerate() {
            if iso {
                class[k] = Some(NOISE);
                if let Some(q) = work.pts.get_mut(k) {
                    q.class = NOISE;
                }
            }
        }
    }
    let work = &*work;
    let mut ground: Vec<bool> = match &s.ground {
        Ground::Fusion(p) => {
            let (f, _) = GroundFilter::run(work, p)?;
            work.pts
                .iter()
                .map(|q| !q.is_noise() && f.is_ground(q))
                .collect()
        }
        Ground::Mcc(p) => mcc::ground(&work.pts, p),
        Ground::Csf(p) => csf::ground(&work.pts, p)?,
        Ground::Pmf(p) => pmf::ground(&work.pts, p),
        Ground::Smrf(p) => smrf::ground(&work.pts, p),
        Ground::Axelsson(p) => axelsson::ground(&work.pts, p)?,
        Ground::Classes => work
            .pts
            .iter()
            .map(|q| q.class == GROUND && !q.is_noise())
            .collect(),
    };
    if s.ground != Ground::Classes {
        for (k, q) in work.pts.iter().enumerate() {
            if !q.is_noise() {
                class[k] = Some(if ground[k] { GROUND } else { UNCLASSIFIED });
            }
        }
    }
    if let Some(bp) = &s.buildings {
        for (k, c) in buildings::classify(&work.pts, &ground, bp)
            .into_iter()
            .enumerate()
        {
            if let Some(c) = c {
                class[k] = Some(c);
                ground[k] = false;
            }
        }
    }
    if s.extras.any() {
        extras::apply(&work.pts, &ground, &mut class, &s.extras);
    }
    let ruled = rules::apply(&work.pts, &mut class, &mut ground, &s.rules);
    Ok(TileClasses {
        class,
        ground,
        ruled,
    })
}

/// La clase de cada punto tras los pasos, para puntos en memoria (los de una tesela): la clase que
/// decidió un paso o, si ninguno, la propia del punto. El ruido que hallan los pasos se marca
/// también en `work`.
pub fn classify_points(work: &mut Points, s: &Steps) -> Result<Vec<u8>> {
    let TileClasses { class, .. } = classify_tile(work, s)?;
    Ok(work
        .pts
        .iter()
        .zip(class)
        .map(|(q, c)| c.unwrap_or(q.class))
        .collect())
}

fn open_all(inputs: &[std::path::PathBuf]) -> Result<Vec<Reader>> {
    inputs.iter().map(|p| Reader::open(p)).collect()
}

fn union(readers: &[Reader]) -> [f64; 4] {
    readers.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, r| {
            let h = r.header.bounds;
            [
                b[0].min(h[0]),
                b[1].min(h[1]),
                b[2].max(h[3]),
                b[3].max(h[4]),
            ]
        },
    )
}

/// Clasifica `inputs` y escribe sus puntos, con sus clases nuevas, en `output` (LAZ cuando termina
/// en `.laz`).
pub fn classify(
    inputs: &[std::path::PathBuf],
    output: &std::path::Path,
    s: &Steps,
    t: &Tiling,
) -> Result<Report> {
    classify_with_context(inputs, &[], output, s, t)
}

/// [`classify`] con `context`: archivos (las hojas vecinas de un bloque) cuyos puntos ven los
/// filtros cerca de los bordes de `inputs` pero que no se clasifican ni se escriben.
pub fn classify_with_context(
    inputs: &[std::path::PathBuf],
    context: &[std::path::PathBuf],
    output: &std::path::Path,
    s: &Steps,
    t: &Tiling,
) -> Result<Report> {
    crate::paths::guard_output(inputs, output)?;
    crate::paths::guard_output(context, output)?;
    let readers = open_all(inputs)?;
    let context = open_all(context)?;
    let template = readers
        .first()
        .ok_or_else(|| Error::Param("no hay archivos de entrada".into()))?;
    let layout = Layout::new(union(&readers), t.size, t.buffer, 1.0)?;
    let tiled = Tiled::spool_with_context(&readers, &context, layout)?;
    let format = tiled.header.format;
    let rec_len = usize::from(tiled.header.record_len);
    let mut writer = Writer::create(output, template)?;
    let mut report = Report::default();
    tiled.process(
        t.threads,
        |mut tile: Tile| {
            let TileClasses { class, ruled, .. } = classify_tile(&mut tile.points, s)?;
            // Los registros propios de la tesela, reclasificados.
            let mut out = Vec::new();
            let mut r = Report {
                tiles: 1,
                ..Default::default()
            };
            for (k, rec) in tile.records.chunks_exact(rec_len).enumerate() {
                if !tile.own.get(k).copied().unwrap_or(false) {
                    continue;
                }
                let mut rec = rec.to_vec();
                r.rules += u64::from(ruled.get(k).copied().unwrap_or(false));
                if let Some(c) = class.get(k).copied().flatten() {
                    las::set_class(format, &mut rec, c);
                    match c {
                        NOISE => r.noise += 1,
                        GROUND => r.ground += 1,
                        buildings::BUILDING => r.buildings += 1,
                        extras::LOW_VEGETATION
                        | extras::MEDIUM_VEGETATION
                        | buildings::HIGH_VEGETATION => r.vegetation += 1,
                        extras::WIRE => r.wires += 1,
                        extras::POLE => r.poles += 1,
                        _ => {}
                    }
                } else if s.ground == Ground::Classes
                    && tile.points.pts.get(k).is_some_and(|q| q.class == GROUND)
                {
                    r.ground += 1;
                }
                r.points += 1;
                out.extend_from_slice(&rec);
            }
            Ok((out, r))
        },
        |(records, r)| {
            for rec in records.chunks_exact(rec_len) {
                writer.write(rec)?;
            }
            report.tiles += r.tiles;
            report.points += r.points;
            report.noise += r.noise;
            report.ground += r.ground;
            report.buildings += r.buildings;
            report.vegetation += r.vegetation;
            report.wires += r.wires;
            report.poles += r.poles;
            report.rules += r.rules;
            Ok(())
        },
    )?;
    writer.finish()?;
    Ok(report)
}

/// Cómo se hace el MDT a partir de los puntos de suelo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surface {
    /// Elevación media de los puntos de suelo en cada celda (superficie media).
    Mean,
    /// Interpolado sobre una triangulación de Delaunay de los puntos de suelo.
    Tin,
    /// Ponderación por el inverso de la distancia de los puntos de suelo más próximos.
    Idw,
}

/// Un MDT de `inputs`, clasificado y rasterizado tesela a tesela; los huecos se rellenan al final.
pub fn dtm(
    inputs: &[std::path::PathBuf],
    cell: f64,
    s: &Steps,
    surface: Surface,
    t: &Tiling,
    breaklines: Option<&crate::breaklines::Breaklines>,
) -> Result<(Grid, Report, Option<u16>)> {
    dtm_in(inputs, &[], cell, s, surface, t, breaklines)
}

/// [`dtm`] con `context`: archivos (las hojas vecinas de un bloque) cuyos puntos ven los filtros y
/// las triangulaciones cerca de los bordes de `inputs`, pero que no dan ninguna celda propia.
pub fn dtm_in(
    inputs: &[std::path::PathBuf],
    context: &[std::path::PathBuf],
    cell: f64,
    s: &Steps,
    surface: Surface,
    t: &Tiling,
    breaklines: Option<&crate::breaklines::Breaklines>,
) -> Result<(Grid, Report, Option<u16>)> {
    let readers = open_all(inputs)?;
    let context = open_all(context)?;
    let epsg = readers.first().and_then(|r| r.header.epsg);
    let b = union(&readers);
    let layout = Layout::new(b, t.size, t.buffer, cell)?;
    let mut out = Grid::covering(b, cell)?;
    let fp = Footprint::new(b, cell);
    let mut tiled = Tiled::spool_with_context(&readers, &context, layout)?;
    tiled.keep_records = false;
    let mut report = Report::default();
    tiled.process(
        t.threads,
        |mut tile: Tile| {
            for q in tile.points.pts.iter().filter(|q| !q.is_noise()) {
                fp.add(q.x, q.y);
            }
            let TileClasses { class, ground, .. } = classify_tile(&mut tile.points, s)?;
            let e = tile.core;
            let cols = ((e[2] - e[0]) / cell).round() as usize;
            let rows = ((e[3] - e[1]) / cell).round() as usize;
            let mut g = Grid::new(e[0], e[1], cell, cols, rows)?;
            match surface {
                Surface::Mean => {
                    let mut m = MeanGrid::new(g, tile.points.bounds[2])?;
                    for (q, _) in tile.points.pts.iter().zip(&ground).filter(|(_, g)| **g) {
                        m.add(q.x, q.y, q.z);
                    }
                    g = m.finish();
                }
                Surface::Tin | Surface::Idw => {
                    let mut pts: Vec<[f64; 3]> = tile
                        .points
                        .pts
                        .iter()
                        .zip(&ground)
                        .filter(|(_, g)| **g)
                        .map(|(q, _)| [q.x, q.y, q.z])
                        .collect();
                    if let Some(lines) = breaklines {
                        let b = tile.points.bounds;
                        lines.apply(&mut pts, [b[0], b[1], b[3], b[4]], cell);
                    }
                    if surface == Surface::Tin {
                        tin::rasterize(&pts, &mut g);
                    } else {
                        idw::rasterize(&pts, &mut g, false);
                    }
                }
            }
            let mut r = Report {
                tiles: 1,
                ..Default::default()
            };
            for (k, own) in tile.own.iter().enumerate() {
                if *own {
                    r.points += 1;
                    r.ground += u64::from(ground[k]);
                    match class[k] {
                        Some(NOISE) => r.noise += 1,
                        Some(buildings::BUILDING) => r.buildings += 1,
                        Some(buildings::HIGH_VEGETATION) => r.vegetation += 1,
                        _ => {}
                    }
                }
            }
            Ok((g, r))
        },
        |(g, r)| {
            // Pega la tesela en el mosaico.
            for row in 0..g.rows {
                for col in 0..g.cols {
                    let (x, y) = g.center(col, row);
                    if let (Some(i), Some(&v)) = (out.index(x, y), g.data.get(row * g.cols + col))
                        && !v.is_nan()
                        && let Some(slot) = out.data.get_mut(i)
                    {
                        *slot = v;
                    }
                }
            }
            report.tiles += r.tiles;
            report.points += r.points;
            report.noise += r.noise;
            report.ground += r.ground;
            report.buildings += r.buildings;
            report.vegetation += r.vegetation;
            report.wires += r.wires;
            report.poles += r.poles;
            report.rules += r.rules;
            Ok(())
        },
    )?;
    if report.ground == 0 {
        return Err(Error::Param("no se encuentra ningún punto de suelo".into()));
    }
    out.fill_holes();
    fp.apply(&mut out);
    Ok((out, report, epsg))
}

/// Modelo de altura de copas sin picos (Khosravipour et al. 2014): un TIN de los puntos por encima
/// de cada umbral de altura, el primero sin límite de arista y los demás descartando los triángulos
/// con aristas más largas que `max_edge`, apilados tomando el más alto.
#[derive(Clone, Debug, PartialEq)]
pub struct PitFree {
    pub thresholds: Vec<f64>,
    pub max_edge: f64,
}

impl Default for PitFree {
    fn default() -> Self {
        Self {
            thresholds: vec![0.0, 2.0, 5.0, 10.0, 15.0],
            max_edge: 1.0,
        }
    }
}

/// Un CHM sin picos de `inputs` sobre `dtm`, tesela a tesela.
pub fn chm_pitfree(
    inputs: &[std::path::PathBuf],
    dtm: &Grid,
    cell: f64,
    p: &PitFree,
    t: &Tiling,
) -> Result<(Grid, Option<u16>)> {
    chm_pitfree_in(inputs, &[], dtm, cell, p, t)
}

/// [`chm_pitfree`] con `context`: archivos (las hojas vecinas de un bloque) cuyos puntos ven los
/// filtros y las triangulaciones cerca de los bordes de `inputs`, pero que no dan ninguna celda
/// propia.
pub fn chm_pitfree_in(
    inputs: &[std::path::PathBuf],
    context: &[std::path::PathBuf],
    dtm: &Grid,
    cell: f64,
    p: &PitFree,
    t: &Tiling,
) -> Result<(Grid, Option<u16>)> {
    let readers = open_all(inputs)?;
    let context = open_all(context)?;
    let epsg = readers.first().and_then(|r| r.header.epsg);
    let b = union(&readers);
    let layout = Layout::new(b, t.size, t.buffer, cell)?;
    let mut out = Grid::covering(b, cell)?;
    let fp = Footprint::new(b, cell);
    let mut tiled = Tiled::spool_with_context(&readers, &context, layout)?;
    tiled.keep_records = false;
    tiled.process(
        t.threads,
        |tile: Tile| {
            for q in tile.points.pts.iter().filter(|q| !q.is_noise()) {
                fp.add(q.x, q.y);
            }
            let e = tile.core;
            let cols = ((e[2] - e[0]) / cell).round() as usize;
            let rows = ((e[3] - e[1]) / cell).round() as usize;
            let mut chm = Grid::new(e[0], e[1], cell, cols, rows)?;
            let heights: Vec<[f64; 3]> = tile
                .points
                .pts
                .iter()
                .filter(|q| !q.is_noise())
                .filter_map(|q| {
                    dtm.sample(q.x, q.y)
                        .map(|g| [q.x, q.y, (q.z - f64::from(g)).max(0.0)])
                })
                .collect();
            for (k, th) in p.thresholds.iter().enumerate() {
                let layer_pts: Vec<[f64; 3]> =
                    heights.iter().filter(|q| q[2] >= *th).copied().collect();
                let mut layer = chm.like();
                let edge = if k == 0 { f64::INFINITY } else { p.max_edge };
                tin::rasterize_with(&layer_pts, &mut layer, edge, tin::Thin::Highest);
                for (c, l) in chm.data.iter_mut().zip(&layer.data) {
                    if !l.is_nan() && (c.is_nan() || *l > *c) {
                        *c = *l;
                    }
                }
            }
            Ok(chm)
        },
        |g| {
            for row in 0..g.rows {
                for col in 0..g.cols {
                    let (x, y) = g.center(col, row);
                    if let (Some(i), Some(&v)) = (out.index(x, y), g.data.get(row * g.cols + col))
                        && !v.is_nan()
                        && let Some(slot) = out.data.get_mut(i)
                    {
                        *slot = v;
                    }
                }
            }
            Ok(())
        },
    )?;
    out.fill_holes();
    fp.apply(&mut out);
    Ok((out, epsg))
}

/// Un modelo digital de superficie (retornos más altos) en celdas de `cell`, tesela a tesela: por
/// un TIN del punto más alto de cada celda o por ponderación por el inverso de la distancia de
/// ellos.
pub fn dsm(
    inputs: &[std::path::PathBuf],
    cell: f64,
    interpolation: Surface,
    t: &Tiling,
) -> Result<(Grid, Option<u16>)> {
    dsm_in(inputs, &[], cell, interpolation, t)
}

/// [`dsm`] con `context`: archivos (las hojas vecinas de un bloque) cuyos puntos ven los filtros y
/// las triangulaciones cerca de los bordes de `inputs`, pero que no dan ninguna celda propia.
pub fn dsm_in(
    inputs: &[std::path::PathBuf],
    context: &[std::path::PathBuf],
    cell: f64,
    interpolation: Surface,
    t: &Tiling,
) -> Result<(Grid, Option<u16>)> {
    let readers = open_all(inputs)?;
    let context = open_all(context)?;
    let epsg = readers.first().and_then(|r| r.header.epsg);
    let b = union(&readers);
    let layout = Layout::new(b, t.size, t.buffer, cell)?;
    let mut out = Grid::covering(b, cell)?;
    let fp = Footprint::new(b, cell);
    let mut tiled = Tiled::spool_with_context(&readers, &context, layout)?;
    tiled.keep_records = false;
    tiled.process(
        t.threads,
        |tile: Tile| {
            for q in tile.points.pts.iter().filter(|q| !q.is_noise()) {
                fp.add(q.x, q.y);
            }
            let e = tile.core;
            let cols = ((e[2] - e[0]) / cell).round() as usize;
            let rows = ((e[3] - e[1]) / cell).round() as usize;
            let mut g = Grid::new(e[0], e[1], cell, cols, rows)?;
            let pts: Vec<[f64; 3]> = tile
                .points
                .pts
                .iter()
                .filter(|q| !q.is_noise())
                .map(|q| [q.x, q.y, q.z])
                .collect();
            match interpolation {
                Surface::Idw => idw::rasterize(&pts, &mut g, true),
                _ => tin::rasterize_with(&pts, &mut g, tin::MAX_EDGE, tin::Thin::Highest),
            }
            Ok(g)
        },
        |g| {
            for row in 0..g.rows {
                for col in 0..g.cols {
                    let (x, y) = g.center(col, row);
                    if let (Some(i), Some(&v)) = (out.index(x, y), g.data.get(row * g.cols + col))
                        && !v.is_nan()
                        && let Some(slot) = out.data.get_mut(i)
                    {
                        *slot = v;
                    }
                }
            }
            Ok(())
        },
    )?;
    out.fill_holes();
    fp.apply(&mut out);
    Ok((out, epsg))
}
