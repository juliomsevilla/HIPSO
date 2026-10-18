//! Streaming utilities taken from the useful end of WhiteboxTools' LiDAR set, built for the same
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
                Stat::MeanIntensity | Stat::MaxIntensity => f64::from(p.intensity),
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
            "tiles need a positive size and a buffer of 0 or more".into(),
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
            "too many tiles: use a larger tile size".into(),
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
            "the new file must not overwrite the original".into(),
        ));
    }
    let r = Reader::open(input)?;
    if (r.header.epsg.is_some() || r.meta.wkt.is_some()) && !replace {
        return Err(Error::Param(format!(
            "{} already has a coordinate system: choose to replace it",
            input.display()
        )));
    }
    let (records, count, epsg, wkt) = match source {
        CrsSource::Cloud(p) => {
            let s = Reader::open(p)?;
            let (rec, n) = s.projection_records();
            if n == 0 {
                return Err(Error::Param(format!(
                    "{} has no coordinate system to copy",
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
