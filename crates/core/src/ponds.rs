//! Ponding: where water stays on a terrain model.
//!
//! The model is filled until every depression drains (priority flood: Barnes, Lehman and Mulla
//! 2014); a cell is under water when the filled surface lies above the ground by more than the
//! minimum depth, and connected cells (sharing an edge) form one pond. Each pond is measured:
//! area, depth, volume, the level at which it spills and the area that drains into it. Ponds
//! can be kept or dropped by size and, as an extra check, by the slope of the ground under them.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::hydro;
use crate::obia::vectorize::Tracer;
use crate::obia::{Labels, Lattice};
use crate::terrain::{self, Edges};
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result, check_cancel, paths};

/// How the water is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// The water a depression holds up to its spill level: the model is filled and the depth is
    /// the filled surface minus the ground. Slope and drained area (when asked for) are checked
    /// on each pond as a whole.
    Depression,
    /// Low spots against the smoothed model: the depth is the *filled smoothed* surface minus the
    /// *unsmoothed* ground, so it also picks up hollows that are not closed depressions, such as
    /// a furrow bottom under the smoothed ridge. Slope (of the filled surface) and drained area
    /// are checked on every cell.
    Local,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PondJob {
    pub method: Method,
    /// A value that means "no data" in a model that does not say so (0 in some exports); when
    /// `None`, zeros are taken for it only if they cover a good part of a model whose other
    /// values are all well above 0.
    pub nodata: Option<f64>,
    /// The terrain model, in a metric system.
    pub input: PathBuf,
    /// Base path of the results (its extension is replaced by what each file needs).
    pub output: PathBuf,
    /// Standard deviation, in map units, of a Gaussian smoothing of the model before filling
    /// (0: none). Evens out the noise of a model made from photogrammetry or sparse returns.
    pub smooth: f64,
    /// Water shallower than this (model units) is not a pond.
    pub min_depth: f64,
    /// Ponds smaller than this many map units² are dropped.
    pub min_area: f64,
    /// Ponds larger or deeper than these are dropped: reservoirs, ditches and pits are not
    /// puddles (`None`: no limit).
    pub max_area: Option<f64>,
    pub max_depth: Option<f64>,
    /// Ponds holding less than this volume (model units × map units²) are dropped.
    pub min_volume: f64,
    /// Drop ponds whose ground is steeper than this mean slope, degrees.
    pub max_slope: Option<f64>,
    /// Drop ponds that drain an area smaller than this, map units².
    pub min_catchment: Option<f64>,
    /// Write the pond numbers as a raster, and the water depth as another.
    pub raster: bool,
    pub depth_raster: bool,
    /// Write the pond polygons as a shapefile, and as a GeoPackage.
    pub shp: bool,
    pub gpkg: bool,
}

impl PondJob {
    pub fn new(input: PathBuf, output: PathBuf) -> Self {
        Self {
            method: Method::Depression,
            nodata: None,
            input,
            output,
            smooth: 0.0,
            min_depth: 0.05,
            min_area: 0.0,
            max_area: None,
            max_depth: None,
            min_volume: 0.0,
            max_slope: None,
            min_catchment: None,
            raster: true,
            depth_raster: false,
            shp: true,
            gpkg: false,
        }
    }
}

/// What was measured about one pond.
#[derive(Clone, Debug, PartialEq)]
pub struct Pond {
    pub id: u32,
    pub cells: u64,
    pub area: f64,
    pub max_depth: f64,
    pub mean_depth: f64,
    pub volume: f64,
    /// Level of the water when the pond is full.
    pub spill: f64,
    pub catchment: f64,
    pub slope: f64,
}

/// The ponds of a terrain model and the water depth of every cell.
pub struct Ponds {
    pub labels: Labels,
    pub depth: Grid,
    pub list: Vec<Pond>,
}

/// Gaussian smoothing with a separable kernel; empty cells stay empty and weigh nothing.
fn gaussian(g: &Grid, sigma: f64) -> Grid {
    let s = sigma / g.cell;
    if s < 0.3 {
        return g.clone();
    }
    let rad = (3.0 * s).ceil() as isize;
    let k: Vec<f32> = (-rad..=rad)
        .map(|i| (-(i * i) as f64 / (2.0 * s * s)).exp() as f32)
        .collect();
    let (cols, rows) = (g.cols, g.rows);
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![f32::NAN; src.len()];
        out.par_chunks_mut(cols).enumerate().for_each(|(r, row)| {
            for (c, o) in row.iter_mut().enumerate() {
                if src[r * cols + c].is_nan() {
                    continue;
                }
                let (mut sum, mut w) = (0.0f32, 0.0f32);
                for (n, kw) in (-rad..=rad).zip(&k) {
                    let (rr, cc) = if horizontal {
                        (r as isize, c as isize + n)
                    } else {
                        (r as isize + n, c as isize)
                    };
                    if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                        continue;
                    }
                    let v = src[rr as usize * cols + cc as usize];
                    if !v.is_nan() {
                        sum += v * kw;
                        w += kw;
                    }
                }
                *o = sum / w;
            }
        });
        out
    };
    let mut out = g.like();
    out.data = pass(&pass(&g.data, true), false);
    out
}

/// Connected components (sharing an edge) of the cells where `mask` is true, numbered from 1 in
/// raster order; the number of components.
fn components(mask: &[bool], cols: usize, rows: usize) -> (Vec<u32>, u32) {
    let n = mask.len();
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(p: &mut [u32], mut i: u32) -> u32 {
        while p[i as usize] != i {
            p[i as usize] = p[p[i as usize] as usize];
            i = p[i as usize];
        }
        i
    }
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            if !mask[i] {
                continue;
            }
            for j in [(c > 0).then(|| i - 1), (r > 0).then(|| i - cols)].into_iter().flatten() {
                if mask[j] {
                    let (a, b) = (find(&mut parent, i as u32), find(&mut parent, j as u32));
                    if a != b {
                        parent[a.max(b) as usize] = a.min(b);
                    }
                }
            }
        }
    }
    let mut ids = vec![0u32; n];
    let mut number = vec![0u32; n];
    let mut count = 0u32;
    for i in 0..n {
        if !mask[i] {
            continue;
        }
        let root = find(&mut parent, i as u32) as usize;
        if number[root] == 0 {
            count += 1;
            number[root] = count;
        }
        ids[i] = number[root];
    }
    (ids, count)
}

/// Empties the cells that hold the no-data value; says what was done, if anything.
fn apply_nodata(g: &mut Grid, value: Option<f64>) -> Option<String> {
    let total = g.data.len().max(1);
    let (v, auto) = match value {
        Some(v) => (v as f32, false),
        None => {
            let zeros = g.data.iter().filter(|x| **x == 0.0).count();
            let low = g
                .data
                .iter()
                .filter(|x| x.is_finite() && **x != 0.0)
                .fold(f32::INFINITY, |a, x| a.min(*x));
            if zeros * 100 < total || !(low > 1.0) {
                return None;
            }
            (0.0, true)
        }
    };
    let mut n = 0usize;
    for x in &mut g.data {
        if *x == v {
            *x = f32::NAN;
            n += 1;
        }
    }
    (n > 0).then(|| {
        format!(
            "{} de las celdas ({:.1} %) valen {v}: se toman como sin dato{}",
            n,
            100.0 * n as f64 / total as f64,
            if auto { " (el modelo no declara valor sin dato; con --nodata se elige otro)" } else { "" }
        )
    })
}

/// Finds and measures the ponds of a terrain model in memory.
pub fn find(g: &Grid, job: &PondJob) -> Result<Ponds> {
    if !(g.cell > 1e-3) {
        return Err(Error::Param(
            "la celda mide menos de 1 mm: el ráster parece estar en grados; reproyéctalo a un sistema métrico (UTM)"
                .into(),
        ));
    }
    if !(job.min_depth >= 0.0) {
        return Err(Error::Param("la profundidad mínima no puede ser negativa".into()));
    }
    let base = if job.smooth > 0.0 { gaussian(g, job.smooth) } else { g.clone() };
    check_cancel()?;
    let filled = hydro::drainage(&base, 1.0)?.filled;
    check_cancel()?;
    let n = base.data.len();
    let mut depth = base.like();
    let min = job.min_depth as f32;
    let local = job.method == Method::Local;
    let accumulation = hydro::flow_accumulation(&base, 1.0);
    let slope = terrain::slope(&base, 1.0, false, Edges::NoData);
    let mask: Vec<bool> = if local {
        // slope of the filled surface and drained area, cell by cell
        let mut fg = base.like();
        fg.data.clone_from(&filled);
        let fslope = terrain::slope(&fg, 1.0, false, Edges::NoData);
        (0..n)
            .map(|i| {
                let d = filled[i] - g.data[i];
                if !d.is_nan() {
                    depth.data[i] = d;
                }
                d > min
                    && job.max_slope.is_none_or(|m| f64::from(fslope.data[i]) < m)
                    && job.min_catchment.is_none_or(|m| f64::from(accumulation.data[i]) > m)
            })
            .collect()
    } else {
        (0..n)
            .map(|i| {
                let d = filled[i] - base.data[i];
                if !d.is_nan() {
                    depth.data[i] = d;
                }
                d > min
            })
            .collect()
    };
    let (mut ids, count) = components(&mask, g.cols, g.rows);
    check_cancel()?;
    let cell2 = g.cell * g.cell;
    #[derive(Clone, Default)]
    struct Sum {
        cells: u64,
        depth: f64,
        max: f32,
        spill: f64,
        catchment: f32,
        slope: f64,
        slope_n: u64,
    }
    let mut sums = vec![Sum::default(); count as usize + 1];
    for i in 0..n {
        let id = ids[i] as usize;
        if id == 0 {
            continue;
        }
        let s = &mut sums[id];
        s.cells += 1;
        s.depth += f64::from(depth.data[i]);
        s.max = s.max.max(depth.data[i]);
        s.spill = f64::from(filled[i]);
        let a = accumulation.data[i];
        if !a.is_nan() {
            s.catchment = s.catchment.max(a);
        }
        let sl = slope.data[i];
        if !sl.is_nan() {
            s.slope += f64::from(sl);
            s.slope_n += 1;
        }
    }
    // the ponds that stay, numbered again from 1
    let mut renumber = vec![0u32; count as usize + 1];
    let mut list: Vec<Pond> = Vec::new();
    for (id, s) in sums.iter().enumerate().skip(1) {
        let area = s.cells as f64 * cell2;
        let volume = s.depth * cell2;
        let mean_slope = if s.slope_n > 0 { s.slope / s.slope_n as f64 } else { 0.0 };
        let keep = area >= job.min_area
            && job.max_area.is_none_or(|m| area <= m)
            && job.max_depth.is_none_or(|m| f64::from(s.max) <= m)
            && volume >= job.min_volume
            && (local || job.max_slope.is_none_or(|m| mean_slope <= m))
            && (local || job.min_catchment.is_none_or(|m| f64::from(s.catchment) >= m));
        if !keep {
            continue;
        }
        let number = list.len() as u32 + 1;
        renumber[id] = number;
        list.push(Pond {
            id: number,
            cells: s.cells,
            area,
            max_depth: f64::from(s.max),
            mean_depth: s.depth / s.cells as f64,
            volume,
            spill: s.spill,
            catchment: f64::from(s.catchment),
            slope: mean_slope,
        });
    }
    for (i, id) in ids.iter_mut().enumerate() {
        *id = renumber[*id as usize];
        if *id == 0 {
            depth.data[i] = if depth.data[i].is_nan() { f32::NAN } else { 0.0 };
        }
    }
    let labels = Labels { lat: Lattice::of(g), count: list.len() as u32, ids };
    Ok(Ponds { labels, depth, list })
}

/// The ponds as a polygon layer.
fn layer(p: &Ponds, simplify: f64) -> Result<VectorData> {
    let tracer = Tracer::new(&p.labels)?;
    let outlines: Vec<_> = (1..=p.labels.count)
        .into_par_iter()
        .map(|id| tracer.outline(id, simplify))
        .collect();
    let fields: Vec<String> = [
        "id", "area_m2", "prof_max", "prof_med", "vol_m3", "cota_desb", "cuenca_m2", "pend_med",
    ]
    .map(String::from)
    .to_vec();
    let mut d = VectorData {
        fields,
        polygons: true,
        bounds: [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
        ..Default::default()
    };
    for (pond, o) in p.list.iter().zip(outlines) {
        let mut parts = o.outers;
        parts.extend(o.holes);
        for q in parts.iter().flatten() {
            d.bounds = [
                d.bounds[0].min(q[0]),
                d.bounds[1].min(q[1]),
                d.bounds[2].max(q[0]),
                d.bounds[3].max(q[1]),
            ];
        }
        let values = vec![
            pond.id.to_string(),
            format!("{:.2}", pond.area),
            format!("{:.3}", pond.max_depth),
            format!("{:.3}", pond.mean_depth),
            format!("{:.3}", pond.volume),
            format!("{:.3}", pond.spill),
            format!("{:.1}", pond.catchment),
            format!("{:.2}", pond.slope),
        ];
        d.num.push(values.iter().map(|t| t.parse().unwrap_or(f64::NAN)).collect());
        d.text.push(values);
        d.features.push(Feature { parts });
    }
    Ok(d)
}

/// Finds the ponds of the model in `job.input` and writes what was asked; returns the summary.
pub fn run(job: &PondJob) -> Result<String> {
    let mut g = Grid::read(&job.input)?;
    let epsg = Grid::read_epsg(&job.input);
    let note = apply_nodata(&mut g, job.nodata);
    let ponds = find(&g, job)?;
    paths::ensure_parent(&job.output)?;
    let (mut lines, area, volume) = (
        note.into_iter().collect::<Vec<_>>(),
        ponds.list.iter().map(|p| p.area).sum::<f64>(),
        ponds.list.iter().map(|p| p.volume).sum::<f64>(),
    );
    lines.push(format!(
        "{} charcos: {:.1} m² ({:.3} % del terreno), {:.2} m³ de agua",
        ponds.list.len(),
        area,
        100.0 * area / (g.data.iter().filter(|v| !v.is_nan()).count() as f64 * g.cell * g.cell).max(1e-9),
        volume
    ));
    if let Some(deepest) = ponds.list.iter().max_by(|a, b| a.max_depth.total_cmp(&b.max_depth)) {
        lines.push(format!(
            "  el más profundo: {:.3} (charco {}), el mayor: {:.1} m²",
            deepest.max_depth,
            deepest.id,
            ponds.list.iter().map(|p| p.area).fold(0.0, f64::max)
        ));
    }
    if job.raster {
        let path = paths::swap_ext(&job.output, "tif");
        ponds.labels.write_tiff(&path, epsg)?;
        lines.push(format!("  charcos (ráster): {}", path.display()));
    }
    if job.depth_raster {
        let path = paths::swap_ext(&job.output, "tif");
        let path = path.with_file_name(format!("{}_profundidad.tif", paths::stem(&path)));
        ponds.depth.write_tiff(&path, epsg)?;
        lines.push(format!("  profundidad del agua: {}", path.display()));
    }
    if job.shp || job.gpkg {
        let data = layer(&ponds, g.cell * 0.0)?;
        let prj = epsg.and_then(crate::vector::prj_for_epsg);
        if job.shp {
            let path = paths::swap_ext(&job.output, "shp");
            crate::vector::write_layer(&path, &data, GeomKind::Polygon, prj.as_deref())?;
            lines.push(format!("  charcos (polígonos): {}", path.display()));
        }
        if job.gpkg {
            let path = paths::swap_ext(&job.output, "gpkg");
            crate::gpkg_write::write(
                &path,
                &[crate::gpkg_write::Table {
                    name: "charcos",
                    data: &data,
                    kind: GeomKind::Polygon,
                    epsg,
                }],
            )?;
            lines.push(format!("  charcos (GeoPackage): {}", path.display()));
        }
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gently tilted plane with two bowls and a small one inside the big one.
    fn terrain() -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 0.5, 80, 60)?;
        for r in 0..g.rows {
            for c in 0..g.cols {
                let (x, y) = g.center(c, r);
                let mut z = 10.0 - 0.001 * x;
                // a 12 cm deep bowl, 6 m across, with a 6 cm deeper one in its middle
                let d1 = (x - 12.0).hypot(y - 15.0);
                if d1 < 3.0 {
                    z -= 0.12;
                    if d1 < 1.0 {
                        z -= 0.06;
                    }
                }
                // a 3 cm one: below the minimum
                if (x - 30.0).hypot(y - 15.0) < 2.0 {
                    z -= 0.03;
                }
                g.data[r * g.cols + c] = z as f32;
            }
        }
        Ok(g)
    }

    #[test]
    fn finds_the_ponds_and_measures_them() -> Result<()> {
        let g = terrain()?;
        let job = PondJob::new("x".into(), "y".into());
        let p = find(&g, &job)?;
        // the shallow bowl is below 5 cm; the deep one is a single pond, nested one included
        assert_eq!(p.list.len(), 1, "{:?}", p.list);
        let pond = &p.list[0];
        assert!(pond.max_depth > 0.15 && pond.max_depth < 0.19, "{pond:?}");
        // a disc of radius 3 m: about 28 m², the water as deep as its lowest spill
        assert!((pond.area - 28.3).abs() < 3.0, "{pond:?}");
        assert!(pond.volume > 1.0 && pond.volume < pond.area * pond.max_depth, "{pond:?}");
        assert_eq!(p.labels.ids.iter().filter(|i| **i == 1).count() as u64, pond.cells);
        Ok(())
    }

    #[test]
    fn a_lower_minimum_depth_finds_both_and_the_size_filter_drops_one() -> Result<()> {
        let g = terrain()?;
        let mut job = PondJob::new("x".into(), "y".into());
        job.min_depth = 0.02;
        assert_eq!(find(&g, &job)?.list.len(), 2);
        job.min_area = 20.0;
        assert_eq!(find(&g, &job)?.list.len(), 1);
        job.max_slope = Some(0.0001);
        assert_eq!(find(&g, &job)?.list.len(), 0);
        Ok(())
    }

    #[test]
    fn the_results_are_written() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_ponds_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        terrain()?.write_tiff(&dir.join("mdt.tif"), Some(25830))?;
        let mut job = PondJob::new(dir.join("mdt.tif"), dir.join("charcos.shp"));
        job.gpkg = true;
        job.depth_raster = true;
        let text = run(&job)?;
        assert!(text.contains("1 charcos"), "{text}");
        let ids = Grid::read(&dir.join("charcos.tif"))?;
        assert!(ids.data.contains(&1.0));
        let depth = Grid::read(&dir.join("charcos_profundidad.tif"))?;
        assert!(depth.data.iter().any(|v| *v > 0.15));
        assert_eq!(crate::view::read_vector(&dir.join("charcos.shp"))?.features.len(), 1);
        assert_eq!(crate::view::read_vector(&dir.join("charcos.gpkg"))?.features.len(), 1);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
