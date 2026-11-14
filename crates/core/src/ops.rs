//! Streaming utilities, written from the standard definitions and built for the same
//! premise as the rest: one pass, memory that follows the output and not the cloud. Point statistics on a grid (density, count, elevation, intensity),
//! contour lines and hillshade from a surface, and the tile layout for splitting a cloud.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::clip::{ClipJob, ClipMode};
use crate::filter::Filter;
use crate::grid::Grid;
use crate::las::{Reader, Writer};
use crate::vector::{Polygon, Region, Shape2};
use crate::view::{self, Image, Style};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    /// Points per square map unit.
    Density,
    Count,
    MeanZ,
    MaxZ,
    MinZ,
    RangeZ,
    MeanIntensity,
    MaxIntensity,
}

/// A grid of a statistic of the points in each cell, in one pass.
pub fn raster_stat(
    inputs: &[PathBuf],
    cell: f64,
    stat: Stat,
    filter: &Filter,
) -> Result<(Grid, Option<u16>)> {
    raster_stat_weighted(inputs, cell, stat, filter, None)
}

/// As [`raster_stat`], with the value of each point multiplied by `weight` of the point when
/// given (used for the intensity).
fn raster_stat_weighted(
    inputs: &[PathBuf],
    cell: f64,
    stat: Stat,
    filter: &Filter,
    weight: Option<fn(&crate::las::Point) -> f64>,
) -> Result<(Grid, Option<u16>)> {
    let unified = crate::crs::unify(inputs, false)?;
    let inputs = unified.all(inputs);
    let inputs = inputs.as_slice();
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut epsg = None;
    let readers: Vec<Reader> = inputs
        .iter()
        .map(|p| Reader::open(p))
        .collect::<Result<_>>()?;
    for r in &readers {
        let h = r.header.bounds;
        b = [
            b[0].min(h[0]),
            b[1].min(h[1]),
            b[2].max(h[3]),
            b[3].max(h[4]),
        ];
        epsg = epsg.or(r.header.epsg);
    }
    let mut g = Grid::covering(b, cell)?;
    let n = g.data.len();
    let mut count = vec![0u32; n];
    let mut acc = vec![0f64; n];
    let mut lo = vec![f64::INFINITY; n];
    let mut hi = vec![f64::NEG_INFINITY; n];
    let mut seen = 0u64;
    for r in &readers {
        r.scan(|p| {
            if !filter.keeps(p) || !filter.z_ok(p.z) {
                return;
            }
            seen += 1;
            if filter.decimate > 1 && !(seen - 1).is_multiple_of(filter.decimate) {
                return;
            }
            let Some(i) = g.index(p.x, p.y) else { return };
            let v = match stat {
                Stat::MeanIntensity | Stat::MaxIntensity => {
                    f64::from(p.intensity) * weight.map_or(1.0, |w| w(p))
                }
                _ => p.z,
            };
            count[i] += 1;
            acc[i] += v;
            lo[i] = lo[i].min(v);
            hi[i] = hi[i].max(v);
        })?;
    }
    for i in 0..n {
        if count[i] == 0 {
            continue;
        }
        let c = f64::from(count[i]);
        g.data[i] = match stat {
            Stat::Density => c / (cell * cell),
            Stat::Count => c,
            Stat::MeanZ | Stat::MeanIntensity => acc[i] / c,
            Stat::MaxZ | Stat::MaxIntensity => hi[i],
            Stat::MinZ => lo[i],
            Stat::RangeZ => hi[i] - lo[i],
        } as f32;
    }
    // density of empty cells is zero, not unknown
    if matches!(stat, Stat::Density | Stat::Count) {
        for (i, v) in g.data.iter_mut().enumerate() {
            if count[i] == 0 {
                *v = 0.0;
            }
        }
    }
    Ok((g, epsg))
}

/// The mean spacing between the points of a cloud, from its header: the square root of the
/// area of its box over its number of points. It is an estimate (an oblique strip fills only
/// part of its box, and the spacing of the first returns alone is larger).
pub fn mean_spacing(input: &Path) -> Option<f64> {
    let h = Reader::open(input).ok()?.header;
    let area = (h.bounds[3] - h.bounds[0]) * (h.bounds[4] - h.bounds[1]);
    (h.count > 0 && area > 0.0).then(|| (area / h.count as f64).sqrt())
}

/// The factor that brings the intensity of a point to what it would be at the nadir of its
/// flight line. The energy that returns falls with the square of the range, and the range grows
/// as 1 / cos(scan angle) over flat ground, so the factor is 1 / cos²: the flying height cancels,
/// and no trajectory is needed. Angles over 45° are not corrected further.
fn angle_factor(p: &crate::las::Point) -> f64 {
    let c = f64::from(p.scan_angle.abs().min(45.0)).to_radians().cos();
    1.0 / (c * c)
}

/// Mean intensity per cell as an image of the data: holes of up to `fill` cells across are filled
/// from their neighbours (larger gaps, such as water, stay empty), and the grid is cut to the
/// cells that have points, not to the header's box, which for oblique flight strips or a wrong
/// header holds a lot of nothing. With `angle` the intensity is first corrected for the scan
/// angle ([`angle_factor`]). Empty cells are no data.
pub fn intensity_image(
    inputs: &[PathBuf],
    cell: f64,
    filter: &Filter,
    fill: usize,
    angle: bool,
) -> Result<(Grid, Option<u16>)> {
    let (mut g, epsg) = raster_stat_weighted(
        inputs,
        cell,
        Stat::MeanIntensity,
        filter,
        angle.then_some(angle_factor as fn(&crate::las::Point) -> f64),
    )?;
    if fill > 0 {
        g.fill_holes_within(fill);
    }
    let g = g.trimmed().ok_or_else(|| {
        Error::Param("ningún punto pasa el filtro: no hay intensidad que guardar".into())
    })?;
    Ok((g, epsg))
}

/// The tone range of a set of intensity grids: the 2nd and 98th percentile of all their cells
/// together, so every sheet is stretched alike. Two histogram passes: the first, over the whole
/// range, finds where the bulk of the values lies (a few huge values would otherwise make every
/// bin too wide), the second measures the percentiles inside that.
pub fn tone_range(grids: &[&Grid]) -> Option<(f32, f32)> {
    let vals = || {
        grids
            .iter()
            .flat_map(|g| g.data.iter().copied().filter(|v| v.is_finite()))
    };
    let (lo, hi) = vals().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| {
        (a.min(v), b.max(v))
    });
    if !lo.is_finite() {
        return None;
    }
    if hi <= lo {
        return Some((lo, lo + 1.0));
    }
    const BINS: usize = 4096;
    // value at a share of the cells, from a histogram over [from, to]; cells outside count at
    // the ends
    let quantiles = |from: f32, to: f32, shares: [f64; 2]| -> [f32; 2] {
        let mut hist = vec![0u64; BINS];
        let k = (BINS - 1) as f32 / (to - from);
        let mut n = 0u64;
        for v in vals() {
            hist[(((v - from) * k).max(0.0) as usize).min(BINS - 1)] += 1;
            n += 1;
        }
        shares.map(|share| {
            let target = (n as f64 * share) as u64;
            let mut acc = 0u64;
            for (i, c) in hist.iter().enumerate() {
                acc += c;
                if acc > target {
                    return from + i as f32 / k;
                }
            }
            to
        })
    };
    let [a0, b0] = quantiles(lo, hi, [0.01, 0.99]);
    let (a0, b0) = if b0 > a0 { (a0, b0) } else { (lo, hi) };
    let [a, b] = quantiles(a0, b0, [0.02, 0.98]);
    Some(if b > a { (a, b) } else { (a0, b0) })
}

/// Contour lines of a surface as chained polylines `(level, vertices)`.
pub fn contour_lines(g: &Grid, interval: f64) -> Vec<(f64, Vec<[f64; 2]>)> {
    let q = |v: f64| (v / (g.cell * 1e-3)).round() as i64;
    let mut by_level: HashMap<i64, Vec<[f64; 4]>> = HashMap::new();
    for s in view::contours(g, interval) {
        by_level
            .entry((s[4] * 1000.0).round() as i64)
            .or_default()
            .push([s[0], s[1], s[2], s[3]]);
    }
    let mut levels: Vec<_> = by_level.into_iter().collect();
    levels.sort_by_key(|l| l.0);
    let mut out = Vec::new();
    for (lk, segs) in levels {
        let level = lk as f64 / 1000.0;
        let mut ends: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (k, s) in segs.iter().enumerate() {
            ends.entry((q(s[0]), q(s[1]))).or_default().push(k);
            ends.entry((q(s[2]), q(s[3]))).or_default().push(k);
        }
        let mut used = vec![false; segs.len()];
        let take = |ends: &HashMap<(i64, i64), Vec<usize>>, used: &[bool], at: (i64, i64)| {
            ends.get(&at)
                .and_then(|v| v.iter().copied().find(|k| !used[*k]))
        };
        for start in 0..segs.len() {
            if used[start] {
                continue;
            }
            used[start] = true;
            let s = segs[start];
            let mut line = vec![[s[0], s[1]], [s[2], s[3]]];
            // grow at the tail, then at the head
            for head in [false, true] {
                loop {
                    let tip = if head { line[0] } else { line[line.len() - 1] };
                    let Some(k) = take(&ends, &used, (q(tip[0]), q(tip[1]))) else {
                        break;
                    };
                    used[k] = true;
                    let t = segs[k];
                    let next = if (q(t[0]), q(t[1])) == (q(tip[0]), q(tip[1])) {
                        [t[2], t[3]]
                    } else {
                        [t[0], t[1]]
                    };
                    if head {
                        line.insert(0, next);
                    } else {
                        line.push(next);
                    }
                }
            }
            out.push((level, line));
        }
    }
    out
}

/// Writes contour lines of a surface as a polyline shapefile with a `LEVEL` field.
pub fn write_contours(g: &Grid, interval: f64, path: &Path, epsg: Option<u16>) -> Result<usize> {
    use crate::vector::{ColKind, Table, Val, prj_for_epsg, write_polylines};
    let lines = contour_lines(g, interval);
    let table = Table {
        columns: vec![("LEVEL".into(), ColKind::Num(12, 2))],
        rows: lines.iter().map(|l| vec![Val::Num(l.0)]).collect(),
    };
    let geom: Vec<Vec<[f64; 3]>> = lines
        .iter()
        .map(|l| l.1.iter().map(|p| [p[0], p[1], l.0]).collect())
        .collect();
    crate::paths::ensure_parent(path)?;
    write_polylines(
        path,
        &geom,
        true,
        &table,
        epsg.and_then(prj_for_epsg).as_deref(),
    )?;
    Ok(geom.len())
}

/// Hillshade of a surface as an image (white light, no colour).
pub fn hillshade(g: &Grid, azimuth: f64, altitude: f64) -> Image {
    let mut img = view::shade(g, &Style::default(), azimuth, altitude, false);
    for px in img.rgba.as_chunks_mut::<4>().0 {
        if px[3] == 0 {
            px.copy_from_slice(&[0, 0, 0, 0]);
        }
    }
    img
}

/// Square tiles over `bounds` (min x, min y, max x, max y), each grown by `buffer`, as clip
/// regions named `<prefix>_<col>_<row>`.
pub fn tile_regions(bounds: [f64; 4], size: f64, buffer: f64, prefix: &str) -> Result<Vec<Region>> {
    if !(size > 0.0) || !(buffer >= 0.0) {
        return Err(Error::Param(
            "las teselas necesitan un tamaño positivo y un margen de 0 o más".into(),
        ));
    }
    let x0 = (bounds[0] / size).floor() * size;
    let y0 = (bounds[1] / size).floor() * size;
    let (cols, rows) = (
        ((bounds[2] - x0) / size).floor() as usize + 1,
        ((bounds[3] - y0) / size).floor() as usize + 1,
    );
    if cols.saturating_mul(rows) > 1 << 20 {
        return Err(Error::Param(
            "demasiadas teselas: usa teselas más grandes".into(),
        ));
    }
    let mut out = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let (xa, ya) = (x0 + c as f64 * size, y0 + r as f64 * size);
            out.push(Region {
                name: format!("{prefix}_{}_{}", xa.round() as i64, ya.round() as i64),
                // the upper edges stop just short, so a point on a shared edge belongs to one tile
                shape: Shape2::Rect([
                    xa - buffer,
                    ya - buffer,
                    xa + size + buffer - 1e-6,
                    ya + size + buffer - 1e-6,
                ]),
                attrs: vec![],
            });
        }
    }
    Ok(out)
}

/// A clip job over the whole extent of the clouds: joins them, filters them, converts between
/// LAS and LAZ and normalises (`dtm`), all through the clip engine.
pub fn whole_cloud_job(inputs: Vec<PathBuf>, output: PathBuf) -> Result<ClipJob> {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in &inputs {
        let h = Reader::open(p)?.header.bounds;
        b = [
            b[0].min(h[0]),
            b[1].min(h[1]),
            b[2].max(h[3]),
            b[3].max(h[4]),
        ];
    }
    let region = Region {
        name: "todo".into(),
        shape: Shape2::Rect([b[0] - 1.0, b[1] - 1.0, b[2] + 1.0, b[3] + 1.0]),
        attrs: vec![],
    };
    let mut job = ClipJob::new(inputs, vec![region], output);
    job.mode = ClipMode::Merge;
    Ok(job)
}

/// Polygons of a regions file as plan selection regions for the editing engine.
pub fn polygons_of(regions: &[Region]) -> Vec<crate::edit::Region> {
    regions
        .iter()
        .map(|r| {
            crate::edit::Region::Plan(match &r.shape {
                Shape2::Polygon(p) => p.clone(),
                Shape2::Buffered(b) => b.poly.clone(),
                Shape2::Circle { cx, cy, r } => crate::edit::circle(*cx, *cy, *r),
                Shape2::Rect(b) => Polygon::new(vec![vec![
                    [b[0], b[1]],
                    [b[2], b[1]],
                    [b[2], b[3]],
                    [b[0], b[3]],
                ]]),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_range_ignores_the_extremes_and_empty_cells() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 1000, 1)?;
        for (i, v) in g.data.iter_mut().enumerate() {
            *v = i as f32;
        }
        g.data[0] = f32::NAN;
        for v in &mut g.data[995..] {
            *v = 1.0e6; // outliers, half a percent of the cells
        }
        let (lo, hi) = tone_range(&[&g]).ok_or(Error::Param("sin rango".into()))?;
        assert!((10.0..40.0).contains(&lo), "lo {lo}");
        assert!((900.0..1000.0).contains(&hi), "hi {hi}");
        assert!(tone_range(&[&Grid::new(0.0, 0.0, 1.0, 2, 2)?]).is_none());
        Ok(())
    }

    #[test]
    fn angle_factor_is_one_at_nadir_and_grows_with_the_angle() {
        let p = |a: f32| crate::las::Point {
            scan_angle: a,
            ..Default::default()
        };
        assert_eq!(angle_factor(&p(0.0)), 1.0);
        assert!((angle_factor(&p(-20.0)) - 1.0 / 20f64.to_radians().cos().powi(2)).abs() < 1e-9);
        assert_eq!(angle_factor(&p(60.0)), angle_factor(&p(45.0)));
    }

    #[test]
    fn contours_chain_into_one_line_per_level() -> Result<()> {
        let mut g = Grid::new(0.0, 0.0, 1.0, 10, 6)?;
        for r in 0..6 {
            for c in 0..10 {
                g.data[r * 10 + c] = c as f32;
            }
        }
        // a plane rising to the east: each level crosses the six rows in one straight line
        let lines = contour_lines(&g, 2.5);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.1.len() == 6));
        Ok(())
    }

    #[test]
    fn tiles_cover_the_extent_with_buffer() -> Result<()> {
        let t = tile_regions([0.0, 0.0, 250.0, 90.0], 100.0, 5.0, "t")?;
        assert_eq!(t.len(), 3);
        assert_eq!(t[1].name, "t_100_0");
        assert_eq!(t[1].bbox(), [95.0, -5.0, 205.0 - 1e-6, 105.0 - 1e-6]);
        Ok(())
    }
}

/// Where the projection to give a cloud comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrsSource {
    /// Another cloud that has one.
    Cloud(PathBuf),
    /// An EPSG code.
    Epsg(u16),
}

/// Copies `input` to `output` with a coordinate system added (or replaced, with `replace`). The
/// points are not touched, only the header records; one streaming pass.
pub fn assign_projection(
    input: &Path,
    source: &CrsSource,
    output: &Path,
    replace: bool,
) -> Result<String> {
    if input == output {
        return Err(Error::Param(
            "el archivo nuevo no debe sobrescribir el original".into(),
        ));
    }
    let r = Reader::open(input)?;
    if (r.header.epsg.is_some() || r.meta.wkt.is_some()) && !replace {
        return Err(Error::Param(format!(
            "{} ya tiene sistema de coordenadas: elige reemplazarlo",
            input.display()
        )));
    }
    let (records, count, epsg, wkt) = match source {
        CrsSource::Cloud(p) => {
            let s = Reader::open(p)?;
            let (rec, n) = s.projection_records();
            if n == 0 {
                return Err(Error::Param(format!(
                    "{} no tiene sistema de coordenadas que copiar",
                    p.display()
                )));
            }
            (rec, n, s.header.epsg, s.meta.wkt.clone())
        }
        CrsSource::Epsg(code) => (crate::las::epsg_record(*code), 1, Some(*code), None),
    };
    let r = r.with_projection(&records, count, epsg, wkt);
    crate::paths::ensure_parent(output)?;
    let mut w = Writer::create(output, &r)?;
    let rec_len = usize::from(r.header.record_len);
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            w.write(rec)?;
        }
        Ok(())
    })?;
    let n = w.finish()?;
    Ok(format!(
        "{n} puntos con el sistema de coordenadas {}: {}",
        epsg.map_or("de la nube de referencia".to_string(), |e| format!(
            "EPSG:{e}"
        )),
        output.display()
    ))
}
