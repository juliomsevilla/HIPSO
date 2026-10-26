//! Forest change between two dates, from two canopy height models on the same cells: how much
//! each cell grew or lost, where the canopy opened or closed, which patches were disturbed, what
//! happened in each stand, and how much each tree grew.
//!
//! The difference of two models is only as good as their vertical agreement: a ground model that
//! is a few decimetres off between dates shifts every cell. The report says how big that shift is
//! (the median difference over cells that are ground in both dates).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::grid::Grid;
use crate::obia::attributes::attributes;
use crate::obia::segment::{Method, SegmentParams, segment};
use crate::obia::vectorize::write_polygons;
use crate::obia::{Lattice, LayerStack, write_classes_tiff};
use crate::paths::{self, Formats};
use crate::trees::{self, Smoothing, TopParams};
use crate::vector::{ReadOptions, Region, read_regions};
use crate::{Error, Result, check_cancel};

/// What a cell did between the dates (the codes of the class raster).
pub const STABLE: u16 = 1;
pub const GREW: u16 = 2;
pub const SHRANK: u16 = 3;
pub const COVER_LOST: u16 = 4;
pub const COVER_GAINED: u16 = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct TreeChange {
    pub tops: TopParams,
    /// Smooth both models first with a window of this size (0: no).
    pub smooth: f64,
    /// A tree of the first date and one of the second are the same when closer than this.
    pub max_dist: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangeJob {
    /// The canopy height models of the first and of the second date, on the same cells.
    pub before: PathBuf,
    pub after: PathBuf,
    /// Base path of the results.
    pub output: PathBuf,
    /// A height change smaller than this (in absolute value) is noise, not growth or loss.
    pub min_change: f64,
    /// Height from which a cell counts as canopy.
    pub cover_height: f64,
    /// Patches of lost canopy smaller than this many cells are not reported as disturbances.
    pub min_patch: u32,
    /// Years between the dates, for yearly rates.
    pub years: Option<f64>,
    /// Stands (polygons) to summarise, and the field that names them.
    pub regions: Option<(PathBuf, Option<String>)>,
    pub trees: Option<TreeChange>,
    pub formats: Formats,
}

/// The files a change job writes.
pub struct Outputs {
    pub difference: PathBuf,
    pub classes: PathBuf,
    pub patches: Option<PathBuf>,
    pub patches_table: Option<PathBuf>,
    pub stands: Option<PathBuf>,
    pub trees: Option<PathBuf>,
}

impl ChangeJob {
    pub fn outputs(&self) -> Outputs {
        let f = self.formats.fixed();
        let named = |tag: &str, ext: &str| {
            let b = paths::swap_ext(&self.output, ext);
            let stem = b
                .file_stem()
                .map_or(String::new(), |s| s.to_string_lossy().into_owned());
            b.with_file_name(format!("{stem}_{tag}.{ext}"))
        };
        Outputs {
            difference: named("dchm", "tif"),
            classes: named("cambio", "tif"),
            patches: f.vector.then(|| named("perturbaciones", "shp")),
            patches_table: f.csv.then(|| named("perturbaciones", "csv")),
            stands: self.regions.is_some().then(|| named("rodales", "csv")),
            trees: self.trees.is_some().then(|| named("arboles", "csv")),
        }
    }
}

/// The median of the values (the slice is reordered); NaN when there are none.
fn median(v: &mut [f32]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let m = v.len() / 2;
    v.select_nth_unstable_by(m, |a, b| a.total_cmp(b));
    f64::from(v[m])
}

/// What happened in one cell. `None` when either date has no value.
fn classify(before: f32, after: f32, min_change: f32, cover: f32) -> Option<u16> {
    if before.is_nan() || after.is_nan() {
        return None;
    }
    Some(match (before >= cover, after >= cover) {
        (true, false) => COVER_LOST,
        (false, true) => COVER_GAINED,
        _ => {
            let d = after - before;
            if d >= min_change {
                GREW
            } else if d <= -min_change {
                SHRANK
            } else {
                STABLE
            }
        }
    })
}

/// One stand's summary.
struct StandRow {
    name: String,
    cells: u64,
    count: [u64; 6],
    cover_before: u64,
    cover_after: u64,
    sum_diff: f64,
}

fn stand_rows(
    regions: &[Region],
    lat: &Lattice,
    class: &[u16],
    diff: &[f32],
    before: &[f32],
    after: &[f32],
    cover: f32,
) -> Vec<StandRow> {
    regions
        .iter()
        .map(|r| {
            let b = r.bbox();
            let c0 = (((b[0] - lat.x0) / lat.cell).floor().max(0.0)) as usize;
            let c1 = (((b[2] - lat.x0) / lat.cell).ceil().max(0.0) as usize).min(lat.cols);
            let r0 = (lat.rows as f64 - ((b[3] - lat.y0) / lat.cell).ceil()).max(0.0) as usize;
            let r1 = ((lat.rows as f64 - ((b[1] - lat.y0) / lat.cell).floor()).max(0.0) as usize)
                .min(lat.rows);
            let mut row = StandRow {
                name: r.name.clone(),
                cells: 0,
                count: [0; 6],
                cover_before: 0,
                cover_after: 0,
                sum_diff: 0.0,
            };
            for y in r0..r1 {
                for x in c0..c1 {
                    let i = y * lat.cols + x;
                    if class[i] == 0 {
                        continue;
                    }
                    let cx = lat.x0 + (x as f64 + 0.5) * lat.cell;
                    let cy = lat.y0 + (lat.rows - y) as f64 * lat.cell - lat.cell / 2.0;
                    if !r.contains(cx, cy) {
                        continue;
                    }
                    row.cells += 1;
                    row.count[class[i] as usize] += 1;
                    row.cover_before += u64::from(before[i] >= cover);
                    row.cover_after += u64::from(after[i] >= cover);
                    row.sum_diff += f64::from(diff[i]);
                }
            }
            row
        })
        .collect()
}

/// A tree of one date.
struct Tip {
    x: f64,
    y: f64,
    h: f32,
}

/// Pairs the trees of both dates: each of the second date takes the nearest free tree of the first
/// within `max_dist`, the closest pairs first. Returns `(index before, index after, distance)`.
fn match_trees(before: &[Tip], after: &[Tip], max_dist: f64) -> Vec<(usize, usize, f64)> {
    let cell = max_dist.max(1e-6);
    let key = |x: f64, y: f64| ((x / cell).floor() as i64, (y / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, t) in before.iter().enumerate() {
        grid.entry(key(t.x, t.y)).or_default().push(i);
    }
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (j, t) in after.iter().enumerate() {
        let (kx, ky) = key(t.x, t.y);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &i in grid.get(&(kx + dx, ky + dy)).into_iter().flatten() {
                    let d = (before[i].x - t.x).hypot(before[i].y - t.y);
                    if d <= max_dist {
                        pairs.push((d, i, j));
                    }
                }
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let (mut used_b, mut used_a) = (vec![false; before.len()], vec![false; after.len()]);
    let mut out = Vec::new();
    for (d, i, j) in pairs {
        if !used_b[i] && !used_a[j] {
            used_b[i] = true;
            used_a[j] = true;
            out.push((i, j, d));
        }
    }
    out
}

fn tips(g: &Grid, tc: &TreeChange) -> Vec<Tip> {
    let surface = if tc.smooth > 0.0 {
        trees::smooth(g, tc.smooth, Smoothing::Mean, tc.tops.shape)
    } else {
        g.clone()
    };
    trees::tops(&surface, &tc.tops)
        .into_iter()
        .map(|t| Tip {
            x: t.x,
            y: t.y,
            h: t.height,
        })
        .collect()
}

pub fn change(j: &ChangeJob) -> Result<String> {
    // `after` is moved onto the cells of `before`, and the stands to their system
    let mut files = vec![j.before.as_path(), j.after.as_path()];
    files.extend(j.regions.iter().map(|r| r.0.as_path()));
    let unified = crate::crs::unify_like(&files, true, Some(&j.before))?;
    let warnings = unified.warnings.clone();
    let epsg = Grid::read_epsg(&j.before);
    let a = Grid::read(&j.before)?;
    let b = Grid::read(&unified.path(&j.after))?;
    let lat = Lattice::of(&a);
    if !lat.same(&Lattice::of(&b)) {
        return Err(Error::Param(
            "los dos modelos de alturas no coinciden celda a celda: iguálalos antes con Ráster → Igualar rásteres"
                .into(),
        ));
    }
    if !(j.min_change >= 0.0) {
        return Err(Error::Param(
            "el cambio mínimo no puede ser negativo".into(),
        ));
    }
    let n = a.data.len();
    let (min_change, cover) = (j.min_change as f32, j.cover_height as f32);
    // the difference and what each cell did
    let diff: Vec<f32> = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(x, y)| {
            if x.is_nan() || y.is_nan() {
                f32::NAN
            } else {
                y - x
            }
        })
        .collect();
    let class: Vec<u16> = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(&x, &y)| classify(x, y, min_change, cover).unwrap_or(0))
        .collect();
    check_cancel()?;
    // how well the two models agree where nothing grows: cells that are ground in both dates
    let mut ground: Vec<f32> = a
        .data
        .iter()
        .zip(&b.data)
        .zip(&diff)
        .filter(|((x, y), d)| {
            **x < 0.5 && **y < 0.5 && x.is_finite() && y.is_finite() && d.is_finite()
        })
        .map(|(_, d)| *d)
        .collect();
    let bias = median(&mut ground);
    let out = j.outputs();
    paths::ensure_parent(&j.output)?;
    lat.grid(diff.clone()).write_tiff(&out.difference, epsg)?;
    write_classes_tiff(&out.classes, &lat, epsg, &class)?;
    let area = |cells: u64| cells as f64 * lat.cell * lat.cell / 10_000.0;
    let mut count = [0u64; 6];
    for &c in &class {
        count[c as usize] += 1;
    }
    let valid: u64 = count[1..].iter().sum();
    let mean_diff = diff
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| f64::from(*v))
        .sum::<f64>()
        / valid.max(1) as f64;
    let per_year = |v: f64| {
        j.years
            .filter(|y| *y > 0.0)
            .map(|y| format!(" ({:+.2} por año)", v / y))
    };
    let mut lines = vec![format!(
        "Cambio entre las dos fechas sobre {:.1} ha con datos en las dos:",
        area(valid)
    )];
    lines.push(format!(
        "  altura media: {:+.2} m{}",
        mean_diff,
        per_year(mean_diff).unwrap_or_default()
    ));
    for (code, name) in [
        (GREW, "crece"),
        (SHRANK, "baja"),
        (STABLE, "estable"),
        (COVER_LOST, "pierde la copa"),
        (COVER_GAINED, "gana copa"),
    ] {
        lines.push(format!(
            "  {name}: {:.2} ha ({:.1} %)",
            area(count[code as usize]),
            100.0 * count[code as usize] as f64 / valid.max(1) as f64
        ));
    }
    if bias.is_finite() {
        lines.push(format!(
            "  acuerdo vertical de los modelos (mediana de la diferencia donde ambos son suelo): {bias:+.2} m{}",
            if bias.abs() >= j.min_change.max(0.1) {
                " ⚠ comparable al cambio mínimo: revisa que los MDT de las dos fechas coincidan"
            } else {
                ""
            }
        ));
    }
    lines.push(format!("  diferencia: {}", out.difference.display()));
    lines.push(format!(
        "  clases (1 estable, 2 crece, 3 baja, 4 pierde copa, 5 gana copa): {}",
        out.classes.display()
    ));
    check_cancel()?;
    // patches of lost canopy
    if out.patches.is_some() || out.patches_table.is_some() {
        let mut stack = LayerStack::new(lat, epsg);
        stack.push("clase", class.iter().map(|&c| f32::from(c)).collect())?;
        stack.push("antes", a.data.clone())?;
        stack.push("despues", b.data.clone())?;
        stack.push("dchm", diff.clone())?;
        let labels = segment(
            &stack,
            &SegmentParams {
                method: Method::Mask {
                    layer: "clase".into(),
                    min: f32::from(COVER_LOST as u8),
                    max: f32::from(COVER_LOST as u8),
                    diagonal: false,
                },
                min_cells: j.min_patch.max(1),
            },
        )?;
        if labels.count > 0 {
            let only = vec![
                "antes".to_string(),
                "despues".to_string(),
                "dchm".to_string(),
            ];
            let table = attributes(&labels, &stack, &only)?;
            let lost: f64 = (0..labels.count as usize)
                .map(|o| f64::from(table.cols[0][o]))
                .sum::<f64>()
                / 10_000.0;
            if let Some(p) = &out.patches {
                let written = write_polygons(p, &labels, &table, None, 0.0, epsg)?;
                lines.push(format!(
                    "  perturbaciones (pérdida de copa de al menos {} celdas): {written} manchas, {lost:.2} ha: {}",
                    j.min_patch.max(1),
                    p.display()
                ));
            }
            if let Some(p) = &out.patches_table {
                table.write_csv(p)?;
                lines.push(format!("  tabla de perturbaciones: {}", p.display()));
            }
        } else {
            lines.push("  no hay manchas de pérdida de copa de ese tamaño".to_string());
        }
    }
    // stands
    if let (Some((file, field)), Some(path)) = (&j.regions, &out.stands) {
        let regions = read_regions(
            file,
            &ReadOptions {
                name_field: field.clone(),
                ..Default::default()
            },
        )?;
        let regions = unified.regions(&regions, Some(file))?;
        let rows = stand_rows(&regions, &lat, &class, &diff, &a.data, &b.data, cover);
        let mut csv = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(
            csv,
            "rodal,area_ha,cobertura_antes_pct,cobertura_despues_pct,dchm_medio,crece_ha,baja_ha,pierde_copa_ha,gana_copa_ha"
        )?;
        for r in &rows {
            let cells = r.cells.max(1) as f64;
            writeln!(
                csv,
                "{},{:.3},{:.1},{:.1},{:.3},{:.3},{:.3},{:.3},{:.3}",
                r.name.replace(',', ";"),
                area(r.cells),
                100.0 * r.cover_before as f64 / cells,
                100.0 * r.cover_after as f64 / cells,
                r.sum_diff / cells,
                area(r.count[GREW as usize]),
                area(r.count[SHRANK as usize]),
                area(r.count[COVER_LOST as usize]),
                area(r.count[COVER_GAINED as usize]),
            )?;
        }
        csv.flush()?;
        lines.push(format!("  rodales ({}): {}", rows.len(), path.display()));
    }
    // trees
    if let (Some(tc), Some(path)) = (&j.trees, &out.trees) {
        let (tb, ta) = (tips(&a, tc), tips(&b, tc));
        let pairs = match_trees(&tb, &ta, tc.max_dist);
        let mut csv = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(
            csv,
            "estado,x,y,altura_antes,altura_despues,crecimiento,crecimiento_anual,distancia"
        )?;
        let (mut used_b, mut used_a) = (vec![false; tb.len()], vec![false; ta.len()]);
        let mut growth: Vec<f64> = Vec::new();
        for &(i, k, d) in &pairs {
            used_b[i] = true;
            used_a[k] = true;
            let g = f64::from(ta[k].h - tb[i].h);
            growth.push(g);
            writeln!(
                csv,
                "emparejado,{:.3},{:.3},{:.2},{:.2},{:.2},{},{:.2}",
                ta[k].x,
                ta[k].y,
                tb[i].h,
                ta[k].h,
                g,
                j.years
                    .filter(|y| *y > 0.0)
                    .map_or(String::new(), |y| format!("{:.3}", g / y)),
                d
            )?;
        }
        for (t, _) in tb.iter().zip(&used_b).filter(|(_, u)| !**u) {
            writeln!(csv, "perdido,{:.3},{:.3},{:.2},,,,", t.x, t.y, t.h)?;
        }
        for (t, _) in ta.iter().zip(&used_a).filter(|(_, u)| !**u) {
            writeln!(csv, "nuevo,{:.3},{:.3},,{:.2},,,", t.x, t.y, t.h)?;
        }
        csv.flush()?;
        let mean = growth.iter().sum::<f64>() / growth.len().max(1) as f64;
        let sd = (growth.iter().map(|g| (g - mean).powi(2)).sum::<f64>()
            / growth.len().max(1) as f64)
            .sqrt();
        lines.push(format!(
            "  árboles: {} de la primera fecha, {} de la segunda, {} emparejados (crecimiento {:+.2} ± {:.2} m{}), {} perdidos, {} nuevos: {}",
            tb.len(),
            ta.len(),
            pairs.len(),
            mean,
            sd,
            per_year(mean).unwrap_or_default(),
            tb.len() - pairs.len(),
            ta.len() - pairs.len(),
            path.display()
        ));
    }
    let _ = n;
    Ok(lines.join("\n") + &crate::crs::notes(&warnings))
}

/// The files a job needs, to be checked for existence by callers.
pub fn inputs(j: &ChangeJob) -> Vec<&Path> {
    let mut v = vec![j.before.as_path(), j.after.as_path()];
    v.extend(j.regions.iter().map(|r| r.0.as_path()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cone of a tree at `(cx, cy)`: `h` high, `r` wide.
    fn cone(g: &mut Grid, cx: f64, cy: f64, h: f32, r: f64) {
        for row in 0..g.rows {
            for col in 0..g.cols {
                let (x, y) = g.center(col, row);
                let d = (x - cx).hypot(y - cy);
                if d < r {
                    let v = h * (1.0 - (d / r) as f32);
                    let i = row * g.cols + col;
                    g.data[i] = g.data[i].max(v);
                }
            }
        }
    }

    #[test]
    fn growth_cut_and_new_tree_are_told_apart() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_change_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let new = || -> Result<Grid> {
            let mut g = Grid::new(1000.0, 2000.0, 1.0, 90, 40)?;
            g.data.fill(0.0);
            Ok(g)
        };
        // before: trees A (grows), B (is cut) and C (stays); after: A taller, B gone, D new
        let (mut before, mut after) = (new()?, new()?);
        for (cx, h) in [(1010.0, 12.0), (1040.0, 14.0), (1070.0, 10.0)] {
            cone(&mut before, cx, 2020.0, h, 8.0);
        }
        cone(&mut after, 1010.0, 2020.0, 14.0, 8.0); // A grows 2 m
        cone(&mut after, 1070.0, 2020.0, 10.0, 8.0); // C unchanged
        cone(&mut after, 1025.0, 2005.0, 9.0, 6.0); // D new
        before.write_tiff(&dir.join("a.tif"), Some(25830))?;
        after.write_tiff(&dir.join("b.tif"), Some(25830))?;
        let regions = dir.join("rodales.geojson");
        std::fs::write(
            &regions,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"n":"izq"},
            "geometry":{"type":"Polygon","coordinates":[[[1000,2000],[1030,2000],[1030,2040],[1000,2040],[1000,2000]]]}}]}"#,
        )?;
        let job = ChangeJob {
            before: dir.join("a.tif"),
            after: dir.join("b.tif"),
            output: dir.join("c.shp"),
            min_change: 1.0,
            cover_height: 2.0,
            min_patch: 10,
            years: Some(2.0),
            regions: Some((regions, Some("n".into()))),
            trees: Some(TreeChange {
                tops: TopParams {
                    hmin: 3.0,
                    a: 5.0,
                    b: 0.0,
                    c: 0.0,
                    ..Default::default()
                },
                smooth: 0.0,
                max_dist: 3.0,
            }),
            formats: Formats {
                vector: true,
                csv: true,
            },
        };
        let text = change(&job)?;
        assert!(text.contains("pierde la copa"), "{text}");
        let out = job.outputs();
        // the class raster: tree B's centre lost its canopy, A's centre grew, C's did not change
        let classes = crate::obia::Labels::read_tiff(&out.classes)?;
        let at = |x: f64, y: f64| classes.ids[classes.lat.index(x, y).unwrap_or(0)];
        assert_eq!(at(1040.5, 2020.5), u32::from(COVER_LOST));
        assert_eq!(at(1010.5, 2020.5), u32::from(GREW));
        assert_eq!(at(1070.5, 2020.5), u32::from(STABLE));
        assert_eq!(at(1025.5, 2005.5), u32::from(COVER_GAINED));
        // one patch of lost canopy (B), one tree grew 2 m, one lost, one new
        let patches = std::fs::read_to_string(out.patches_table.as_ref().ok_or(Error::Cancelled)?)?;
        assert_eq!(patches.lines().count(), 2, "{patches}");
        let trees = std::fs::read_to_string(out.trees.as_ref().ok_or(Error::Cancelled)?)?;
        assert!(
            trees.contains("perdido") && trees.contains("nuevo"),
            "{trees}"
        );
        let grown = trees
            .lines()
            .find(|l| l.starts_with("emparejado,1009") || l.starts_with("emparejado,1010"))
            .unwrap_or_default();
        // the cone's apex falls between cell centres, so the growth reads a little under 2 m
        let cols: Vec<&str> = grown.split(',').collect();
        let (g, yearly): (f64, f64) = (
            cols.get(5).and_then(|v| v.parse().ok()).unwrap_or(0.0),
            cols.get(6).and_then(|v| v.parse().ok()).unwrap_or(0.0),
        );
        assert!(
            (1.5..=2.1).contains(&g) && (yearly - g / 2.0).abs() < 0.01,
            "{trees}"
        );
        // stand on the left: tree A grows, B (x=1040) is outside it
        let stands = std::fs::read_to_string(out.stands.as_ref().ok_or(Error::Cancelled)?)?;
        assert!(
            stands.lines().nth(1).is_some_and(|l| l.starts_with("izq,")),
            "{stands}"
        );
        // models off the same cells are refused
        let mut shifted = new()?;
        shifted.x0 += 0.5;
        shifted.write_tiff(&dir.join("c.tif"), Some(25830))?;
        let bad = ChangeJob {
            after: dir.join("c.tif"),
            ..job
        };
        assert!(change(&bad).is_err());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
