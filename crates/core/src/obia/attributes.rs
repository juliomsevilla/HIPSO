//! What describes an object: its size and shape, and the statistics of every layer over its
//! cells. One pass over the cells, with the sums kept in vectors indexed by object number.

use std::io::Write;
use std::path::Path;

use super::{Labels, LayerStack};
use crate::{Error, Result, check_cancel};

/// One row per object (object `i` is `ids[i]`, numbered from 1), one column per attribute.
#[derive(Clone, Debug, Default)]
pub struct AttrTable {
    pub names: Vec<String>,
    /// Column-major: `cols[c][o]` is attribute `c` of object `o`; NaN when it has no value.
    pub cols: Vec<Vec<f32>>,
    pub objects: usize,
    /// Centre of each object in map coordinates, apart from the columns because a float with
    /// seven digits cannot hold a UTM coordinate to the centimetre.
    pub centroid: Vec<[f64; 2]>,
}

impl AttrTable {
    pub fn column(&self, name: &str) -> Option<&[f32]> {
        self.names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|c| self.cols[c].as_slice())
    }

    /// The object number of row `o`.
    pub fn id(o: usize) -> u32 {
        o as u32 + 1
    }

    /// The text of the value at (`o`, column `c`): empty for NaN.
    pub fn text(&self, c: usize, o: usize) -> String {
        let v = self.cols[c][o];
        if v.is_nan() {
            String::new()
        } else if v.fract() == 0.0 && v.abs() < 1e9 {
            format!("{}", v as i64)
        } else {
            format!("{v:.4}")
        }
    }

    /// A text table: the object number, then the attributes.
    pub fn write_csv(&self, path: &Path) -> Result<()> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        write!(out, "id,x,y")?;
        for n in &self.names {
            write!(out, ",{n}")?;
        }
        writeln!(out)?;
        for o in 0..self.objects {
            let [x, y] = self.centroid.get(o).copied().unwrap_or([f64::NAN; 2]);
            write!(out, "{},{x:.3},{y:.3}", Self::id(o))?;
            for c in 0..self.cols.len() {
                write!(out, ",{}", self.text(c, o))?;
            }
            writeln!(out)?;
            if o % 65_536 == 0 {
                check_cancel()?;
            }
        }
        out.flush()?;
        Ok(())
    }
}

/// Describes each object: `area` (map units squared), `perimetro`, `compacidad` (1 would be a
/// disc; a square scores 0.79), `alargamiento` (0 round, towards 1 a line), the centroid (in
/// [`AttrTable::centroid`]), and for every layer its `_media`, `_desv` (standard deviation), `_min` and `_max` over the
/// object's cells. Layers named in `only` (all when empty) are described.
pub fn attributes(labels: &Labels, stack: &LayerStack, only: &[String]) -> Result<AttrTable> {
    attributes_with(labels, stack, only, None)
}

/// What describes the canopy of an object besides the statistics: the share of its cells where the
/// first layer (the canopy height model) reaches `cover_height`, and the median and 95th percentile
/// of that layer. They need the values of the object kept for a moment, hence the separate switch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canopy {
    pub cover_height: f32,
}

/// [`attributes`], with the canopy columns `cobertura` (percent), `<capa>_p50` and `<capa>_p95` of
/// the first layer when `canopy` is given.
pub fn attributes_with(
    labels: &Labels,
    stack: &LayerStack,
    only: &[String],
    canopy: Option<Canopy>,
) -> Result<AttrTable> {
    if !labels.lat.same(&stack.lat) {
        return Err(Error::Param(
            "las etiquetas y las capas no están en la misma rejilla".into(),
        ));
    }
    let lat = labels.lat;
    let (cols, rows) = (lat.cols, lat.rows);
    let n_obj = labels.count as usize;
    let used: Vec<usize> = (0..stack.layers.len())
        .filter(|&i| {
            only.is_empty()
                || only
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(&stack.layers[i].name))
        })
        .collect();
    for n in only {
        if stack.index_of(n).is_none() {
            return Err(Error::Param(format!("no existe la capa '{n}'")));
        }
    }
    let k = used.len();
    let mut size = vec![0u32; n_obj];
    let mut outline = vec![0u32; n_obj];
    // first cell of each object, and the moments of the cells' positions relative to it
    let mut origin = vec![[0u32; 2]; n_obj];
    let mut mom = vec![[0f64; 5]; n_obj];
    let mut count = vec![0u32; n_obj * k];
    let mut sum = vec![0f64; n_obj * k];
    let mut sq = vec![0f64; n_obj * k];
    let mut lo = vec![f32::INFINITY; n_obj * k];
    let mut hi = vec![f32::NEG_INFINITY; n_obj * k];
    let ids = &labels.ids;
    for y in 0..rows {
        if y % 1024 == 0 {
            check_cancel()?;
        }
        for x in 0..cols {
            let i = y * cols + x;
            let l = ids[i];
            if l == 0 {
                continue;
            }
            let o = (l - 1) as usize;
            if size[o] == 0 {
                origin[o] = [x as u32, y as u32];
            }
            size[o] += 1;
            let (dx, dy) = (
                f64::from(x as u32) - f64::from(origin[o][0]),
                f64::from(y as u32) - f64::from(origin[o][1]),
            );
            let m = &mut mom[o];
            m[0] += dx;
            m[1] += dy;
            m[2] += dx * dx;
            m[3] += dy * dy;
            m[4] += dx * dy;
            let differs = |j: Option<usize>| j.is_none_or(|j| ids[j] != l);
            outline[o] += u32::from(differs((x + 1 < cols).then_some(i + 1)))
                + u32::from(differs((y + 1 < rows).then_some(i + cols)))
                + u32::from(differs((x > 0).then(|| i - 1)))
                + u32::from(differs((y > 0).then(|| i - cols)));
            for (c, &li) in used.iter().enumerate() {
                let v = stack.layers[li].data[i];
                if v.is_nan() {
                    continue;
                }
                let s = o * k + c;
                let vf = f64::from(v);
                count[s] += 1;
                sum[s] += vf;
                sq[s] += vf * vf;
                lo[s] = lo[s].min(v);
                hi[s] = hi[s].max(v);
            }
        }
    }
    let cell = lat.cell;
    let mut names: Vec<String> = ["area", "perimetro", "compacidad", "alargamiento"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for &li in &used {
        for suffix in ["media", "desv", "min", "max"] {
            names.push(format!("{}_{suffix}", stack.layers[li].name));
        }
    }
    let mut table = vec![vec![f32::NAN; n_obj]; names.len()];
    let mut centroid = vec![[f64::NAN; 2]; n_obj];
    for o in 0..n_obj {
        let n = f64::from(size[o]);
        if size[o] == 0 {
            continue;
        }
        let m = mom[o];
        let (mx, my) = (m[0] / n, m[1] / n);
        // covariance of the cell centres: the cell's own spread (1/12 per axis) keeps a
        // single-cell or one-row object from having a zero axis
        let cxx = m[2] / n - mx * mx + 1.0 / 12.0;
        let cyy = m[3] / n - my * my + 1.0 / 12.0;
        let cxy = m[4] / n - mx * my;
        let half = (cxx + cyy) / 2.0;
        let radius = (((cxx - cyy) / 2.0).powi(2) + cxy * cxy).sqrt();
        let (l1, l2) = (half + radius, (half - radius).max(0.0));
        let per_cells = f64::from(outline[o]);
        table[0][o] = (n * cell * cell) as f32;
        table[1][o] = (per_cells * cell) as f32;
        table[2][o] = (4.0 * std::f64::consts::PI * n / (per_cells * per_cells)) as f32;
        table[3][o] = (1.0 - (l2 / l1).sqrt()) as f32;
        let (col, row) = (
            f64::from(origin[o][0]) + mx + 0.5,
            f64::from(origin[o][1]) + my + 0.5,
        );
        centroid[o] = [lat.x0 + col * cell, lat.y0 + (rows as f64 - row) * cell];
        for c in 0..k {
            let s = o * k + c;
            if count[s] == 0 {
                continue;
            }
            let cn = f64::from(count[s]);
            let mean = sum[s] / cn;
            let base = 4 + c * 4;
            table[base][o] = mean as f32;
            table[base + 1][o] = (sq[s] / cn - mean * mean).max(0.0).sqrt() as f32;
            table[base + 2][o] = lo[s];
            table[base + 3][o] = hi[s];
        }
    }
    if let (Some(c), Some(first)) = (canopy, stack.layers.first()) {
        let (cover, p50, p95) = canopy_columns(labels, &first.data, c.cover_height)?;
        names.push("cobertura".into());
        table.push(cover);
        names.push(format!("{}_p50", first.name));
        table.push(p50);
        names.push(format!("{}_p95", first.name));
        table.push(p95);
    }
    Ok(AttrTable {
        names,
        cols: table,
        objects: n_obj,
        centroid,
    })
}

/// Cover (percent), median and 95th percentile of `data` over each object, by gathering the values
/// of each object in one run of a vector (counting sort) and selecting in place.
fn canopy_columns(
    labels: &Labels,
    data: &[f32],
    cover_height: f32,
) -> Result<(Vec<f32>, Vec<f32>, Vec<f32>)> {
    let n = labels.count as usize;
    let mut valid = vec![0u32; n];
    let mut above = vec![0u32; n];
    for (i, &l) in labels.ids.iter().enumerate() {
        let v = data[i];
        if l != 0 && !v.is_nan() {
            valid[l as usize - 1] += 1;
            above[l as usize - 1] += u32::from(v >= cover_height);
        }
    }
    check_cancel()?;
    let mut start = vec![0usize; n + 1];
    for o in 0..n {
        start[o + 1] = start[o] + valid[o] as usize;
    }
    let mut fill = start.clone();
    let mut values = vec![0f32; start[n]];
    for (i, &l) in labels.ids.iter().enumerate() {
        let v = data[i];
        if l != 0 && !v.is_nan() {
            let o = l as usize - 1;
            values[fill[o]] = v;
            fill[o] += 1;
        }
    }
    check_cancel()?;
    let mut cover = vec![f32::NAN; n];
    let mut p50 = vec![f32::NAN; n];
    let mut p95 = vec![f32::NAN; n];
    for o in 0..n {
        if valid[o] == 0 {
            continue;
        }
        cover[o] = 100.0 * above[o] as f32 / valid[o] as f32;
        let run = &mut values[start[o]..start[o + 1]];
        let at = |p: f64| ((run.len() - 1) as f64 * p).round() as usize;
        let (i95, i50) = (at(0.95), at(0.5));
        run.select_nth_unstable_by(i95, |a, b| a.total_cmp(b));
        p95[o] = run[i95];
        run[..i95].select_nth_unstable_by(i50.min(i95), |a, b| a.total_cmp(b));
        p50[o] = run[i50.min(i95)];
    }
    Ok((cover, p50, p95))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obia::Lattice;

    #[test]
    fn describes_size_shape_and_layers() -> Result<()> {
        // object 1: a 2 x 2 block with values 1..4; object 2: a row of four cells
        let lat = Lattice {
            x0: 0.0,
            y0: 0.0,
            cell: 2.0,
            cols: 4,
            rows: 3,
        };
        #[rustfmt::skip]
        let ids = vec![
            1, 1, 0, 0,
            1, 1, 0, 0,
            2, 2, 2, 2,
        ];
        let labels = Labels { lat, ids, count: 2 };
        let mut stack = LayerStack::new(lat, None);
        stack.push(
            "h",
            vec![
                1.0,
                2.0,
                9.0,
                9.0,
                3.0,
                4.0,
                9.0,
                9.0,
                5.0,
                5.0,
                5.0,
                f32::NAN,
            ],
        )?;
        let t = attributes(&labels, &stack, &[])?;
        assert_eq!(t.objects, 2);
        let col = |n: &str| t.column(n).map(<[f32]>::to_vec).unwrap_or_default();
        assert_eq!(col("area"), vec![16.0, 16.0]);
        assert_eq!(col("perimetro"), vec![16.0, 20.0]);
        assert_eq!(col("h_media")[0], 2.5);
        assert_eq!(col("h_min")[0], 1.0);
        assert_eq!(col("h_max")[0], 4.0);
        assert_eq!(col("h_media")[1], 5.0);
        assert_eq!(col("h_desv")[1], 0.0);
        // a square is more compact than a line, and less elongated
        assert!(col("compacidad")[0] > col("compacidad")[1]);
        assert!(col("alargamiento")[0] < 0.01 && col("alargamiento")[1] > 0.5);
        // centroid of the block: x in [0, 4], y in the upper two of three rows
        assert!((t.centroid[0][0] - 2.0).abs() < 1e-9 && (t.centroid[0][1] - 4.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn canopy_cover_and_percentiles() -> Result<()> {
        let lat = Lattice {
            x0: 0.0,
            y0: 0.0,
            cell: 1.0,
            cols: 5,
            rows: 2,
        };
        let labels = Labels {
            lat,
            ids: vec![1, 1, 1, 1, 1, 2, 2, 2, 2, 0],
            count: 2,
        };
        let mut stack = LayerStack::new(lat, None);
        // object 1: 0..4 m; object 2: 10, 20, 30 and an empty cell
        stack.push(
            "h",
            vec![0.0, 1.0, 2.0, 3.0, 4.0, 10.0, 20.0, 30.0, f32::NAN, 99.0],
        )?;
        let t = attributes_with(&labels, &stack, &[], Some(Canopy { cover_height: 2.0 }))?;
        let col = |n: &str| t.column(n).map(<[f32]>::to_vec).unwrap_or_default();
        assert_eq!(col("cobertura"), vec![60.0, 100.0]);
        assert_eq!(col("h_p50"), vec![2.0, 20.0]);
        assert_eq!(col("h_p95")[0], 4.0);
        assert_eq!(col("h_p95")[1], 30.0);
        // without the switch the columns are not there
        assert!(
            attributes(&labels, &stack, &[])?
                .column("cobertura")
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn layers_must_share_the_lattice() {
        let lat = Lattice {
            x0: 0.0,
            y0: 0.0,
            cell: 1.0,
            cols: 2,
            rows: 2,
        };
        let labels = Labels {
            lat: Lattice { cols: 3, ..lat },
            ids: vec![0; 6],
            count: 0,
        };
        assert!(attributes(&labels, &LayerStack::new(lat, None), &[]).is_err());
    }
}
