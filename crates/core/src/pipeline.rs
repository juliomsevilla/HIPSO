//! Tiled processing: noise, ground and buildings classified tile by tile, in parallel, and
//! written out as a classified cloud or rasterised into a DTM.

use crate::buildings::{self, BuildingParams};
use crate::csf::{self, CsfParams};
use crate::extras::{self, Extras};
use crate::grid::{Grid, MeanGrid};
use crate::ground::{GroundFilter, GroundParams};
use crate::idw;
use crate::las::{self, Points, Reader, Writer};
use crate::mcc::{self, MccParams};
use crate::noise::{self, NoiseParams};
use crate::pmf::{self, PmfParams};
use crate::tile::{Layout, Tile, Tiled};
use crate::{Error, Result, tin};

pub const NOISE: u8 = 7;
pub const GROUND: u8 = 2;
pub const UNCLASSIFIED: u8 = 1;

/// How ground points are found.
#[derive(Clone, Debug, PartialEq)]
pub enum Ground {
    /// FUSION's GroundFilter (Kraus & Pfeifer).
    Fusion(GroundParams),
    /// Multiscale Curvature Classification.
    Mcc(MccParams),
    /// Cloth Simulation Filter.
    Csf(CsfParams),
    /// Progressive Morphological Filter.
    Pmf(PmfParams),
    /// The points already classified as ground (class 2).
    Classes,
}

/// How noise is found.
#[derive(Clone, Debug, PartialEq)]
pub enum Noise {
    /// Isolated points in a 3 x 3 x 3 block of cells (lasnoise, lidR's ivf).
    Isolated(NoiseParams),
    /// Statistical outlier removal on the mean distance to the k nearest neighbours.
    Sor { k: usize, m: f64 },
}

/// What to classify.
#[derive(Clone, Debug, PartialEq)]
pub struct Steps {
    pub noise: Option<Noise>,
    pub ground: Ground,
    pub buildings: Option<BuildingParams>,
    /// Vegetation strata, wires and poles.
    pub extras: Extras,
}

/// Tiling of the work.
#[derive(Clone, Debug, PartialEq)]
pub struct Tiling {
    pub size: f64,
    pub buffer: f64,
    /// 0: one thread per core.
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

/// How many points each step classified, over the tiles' own points.
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
}

/// The class of every point of a tile after the steps (None: keep its class), and which are
/// ground for the DTM.
fn classify_tile(work: &mut Points, s: &Steps) -> Result<(Vec<Option<u8>>, Vec<bool>)> {
    let n = work.pts.len();
    let mut class: Vec<Option<u8>> = vec![None; n];
    // Points found to be noise are marked in place, so the later steps skip them.
    if let Some(method) = &s.noise {
        let flags = match method {
            Noise::Isolated(np) => noise::isolated(&work.pts, np),
            Noise::Sor { k, m } => noise::sor(&work.pts, *k, *m),
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
        Ground::Csf(p) => csf::ground(&work.pts, p),
        Ground::Pmf(p) => pmf::ground(&work.pts, p),
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
    Ok((class, ground))
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

/// Classifies `inputs` and writes their points, with their new classes, to `output` (LAZ when
/// it ends in `.laz`).
pub fn classify(
    inputs: &[std::path::PathBuf],
    output: &std::path::Path,
    s: &Steps,
    t: &Tiling,
) -> Result<Report> {
    let readers = open_all(inputs)?;
    let template = readers
        .first()
        .ok_or_else(|| Error::Param("no input files".into()))?;
    let layout = Layout::new(union(&readers), t.size, t.buffer, 1.0)?;
    let tiled = Tiled::spool(&readers, layout)?;
    let format = tiled.header.format;
    let rec_len = usize::from(tiled.header.record_len);
    let mut writer = Writer::create(output, template)?;
    let mut report = Report::default();
    tiled.process(
        t.threads,
        |mut tile: Tile| {
            let (class, _) = classify_tile(&mut tile.points, s)?;
            // The tile's own records, reclassified.
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
            Ok(())
        },
    )?;
    writer.finish()?;
    Ok(report)
}

/// How the DTM is made from the ground points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surface {
    /// Mean elevation of the ground points in each cell (FUSION's GridSurfaceCreate).
    Mean,
    /// Interpolated on a Delaunay triangulation of the ground points.
    Tin,
    /// Inverse-distance weighting of the nearest ground points.
    Idw,
}

/// A DTM of `inputs`, classified and rasterised tile by tile; holes are filled at the end.
pub fn dtm(
    inputs: &[std::path::PathBuf],
    cell: f64,
    s: &Steps,
    surface: Surface,
    t: &Tiling,
) -> Result<(Grid, Report, Option<u16>)> {
    let readers = open_all(inputs)?;
    let epsg = readers.first().and_then(|r| r.header.epsg);
    let b = union(&readers);
    let layout = Layout::new(b, t.size, t.buffer, cell)?;
    let mut out = Grid::covering(b, cell)?;
    let mut tiled = Tiled::spool(&readers, layout)?;
    tiled.keep_records = false;
    let mut report = Report::default();
    tiled.process(
        t.threads,
        |mut tile: Tile| {
            let (class, ground) = classify_tile(&mut tile.points, s)?;
            let e = tile.core;
            let cols = ((e[2] - e[0]) / cell).round() as usize;
            let rows = ((e[3] - e[1]) / cell).round() as usize;
            let mut g = Grid::new(e[0], e[1], cell, cols, rows)?;
            match surface {
                Surface::Mean => {
                    let mut m = MeanGrid::new(g, tile.points.bounds[2]);
                    for (q, _) in tile.points.pts.iter().zip(&ground).filter(|(_, g)| **g) {
                        m.add(q.x, q.y, q.z);
                    }
                    g = m.finish();
                }
                Surface::Tin => {
                    let pts: Vec<[f64; 3]> = tile
                        .points
                        .pts
                        .iter()
                        .zip(&ground)
                        .filter(|(_, g)| **g)
                        .map(|(q, _)| [q.x, q.y, q.z])
                        .collect();
                    tin::rasterize(&pts, &mut g);
                }
                Surface::Idw => {
                    let pts: Vec<[f64; 3]> = tile
                        .points
                        .pts
                        .iter()
                        .zip(&ground)
                        .filter(|(_, g)| **g)
                        .map(|(q, _)| [q.x, q.y, q.z])
                        .collect();
                    idw::rasterize(&pts, &mut g, false);
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
            // Paste the tile into the mosaic.
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
            Ok(())
        },
    )?;
    if report.ground == 0 {
        return Err(Error::Param("no ground point found".into()));
    }
    out.fill_holes();
    Ok((out, report, epsg))
}

/// Pit-free canopy height model (Khosravipour et al. 2014): a TIN of the points above each
/// height threshold, the first with no edge limit and the others dropping triangles with longer
/// edges than `max_edge`, stacked by taking the highest.
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

/// A pit-free CHM of `inputs` over `dtm`, tile by tile.
pub fn chm_pitfree(
    inputs: &[std::path::PathBuf],
    dtm: &Grid,
    cell: f64,
    p: &PitFree,
    t: &Tiling,
) -> Result<(Grid, Option<u16>)> {
    let readers = open_all(inputs)?;
    let epsg = readers.first().and_then(|r| r.header.epsg);
    let b = union(&readers);
    let layout = Layout::new(b, t.size, t.buffer, cell)?;
    let mut out = Grid::covering(b, cell)?;
    let mut tiled = Tiled::spool(&readers, layout)?;
    tiled.keep_records = false;
    tiled.process(
        t.threads,
        |tile: Tile| {
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
    Ok((out, epsg))
}

/// A digital surface model (highest returns) on cells of `cell`, tile by tile: through a TIN of
/// the highest point of each cell, or by inverse-distance weighting of them.
pub fn dsm(
    inputs: &[std::path::PathBuf],
    cell: f64,
    interpolation: Surface,
    t: &Tiling,
) -> Result<(Grid, Option<u16>)> {
    let readers = open_all(inputs)?;
    let epsg = readers.first().and_then(|r| r.header.epsg);
    let b = union(&readers);
    let layout = Layout::new(b, t.size, t.buffer, cell)?;
    let mut out = Grid::covering(b, cell)?;
    let mut tiled = Tiled::spool(&readers, layout)?;
    tiled.keep_records = false;
    tiled.process(
        t.threads,
        |tile: Tile| {
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
    Ok((out, epsg))
}
