//! Reclassify a raster: each value (or range of values) becomes another value, as a table of
//! rules says. The first rule that matches a cell decides; what no rule matches is kept, made
//! empty or given a fixed value. Written to the strips of the raster, so memory follows a block
//! of rows whatever the size.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::pyramid::Pyramid;
use crate::rastertools::Dtype;
use crate::{Error, Result};

/// What a rule looks at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cond {
    /// This value.
    Eq(f64),
    /// From `lo` to `hi`, both included.
    Range(f64, f64),
    Lt(f64),
    Le(f64),
    Gt(f64),
    Ge(f64),
    /// The empty cells.
    NoData,
}

/// What a matching cell becomes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Out {
    Value(f32),
    NoData,
    /// The value it has (for the cells no rule matches).
    Keep,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub cond: Cond,
    pub to: Out,
}

/// The rules and what happens to the cells no rule matches.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub rules: Vec<Rule>,
    pub otherwise: Out,
    /// The text said what to do with the other cells (`else = …`).
    pub explicit_else: bool,
}

fn number(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite())
}

fn is_nodata_word(s: &str) -> bool {
    matches!(s, "nodata" | "vacio" | "vacío" | "sin datos" | "nan" | "null")
}

fn parse_cond(s: &str) -> Option<Cond> {
    let t = s.trim().to_lowercase();
    if is_nodata_word(&t) {
        return Some(Cond::NoData);
    }
    for (p, f) in [
        ("<=", Cond::Le as fn(f64) -> Cond),
        (">=", Cond::Ge),
        ("<", Cond::Lt),
        (">", Cond::Gt),
    ] {
        if let Some(rest) = t.strip_prefix(p) {
            return number(rest).map(f);
        }
    }
    if let Some((a, b)) = t.split_once("..") {
        return Some(Cond::Range(number(a)?, number(b)?));
    }
    if let Some(v) = number(&t) {
        return Some(Cond::Eq(v));
    }
    // "a - b", or "a-b" where neither side is empty (a minus sign inside a number is not a cut)
    let cuts = t
        .char_indices()
        .filter(|(i, c)| *c == '-' && *i > 0 && !t[..*i].trim_end().ends_with(['e', '-']))
        .map(|(i, _)| i);
    for i in cuts {
        if let (Some(a), Some(b)) = (number(&t[..i]), number(&t[i + 1..])) {
            return Some(Cond::Range(a, b));
        }
    }
    None
}

fn parse_out(s: &str) -> Option<Out> {
    let t = s.trim().to_lowercase();
    if is_nodata_word(&t) {
        Some(Out::NoData)
    } else if matches!(t.as_str(), "keep" | "igual" | "conservar" | "mantener") {
        Some(Out::Keep)
    } else {
        number(&t).map(|v| Out::Value(v as f32))
    }
}

impl Table {
    /// The rules from text, one per line or separated by `;`: `10 = 1`, `0..5 = 2`,
    /// `5 - 9 = 3`, `< 0 = nodata`, `>= 100 = 4`, `nodata = 0` and `else = nodata` (or a value, or
    /// `keep`). A decimal comma is read as a point.
    pub fn parse(text: &str) -> Result<Table> {
        let mut t = Table {
            rules: Vec::new(),
            otherwise: Out::Keep,
            explicit_else: false,
        };
        for line in text.lines().flat_map(|l| l.split(';')) {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = |why: &str| Error::Param(format!("regla «{line}»: {why}"));
            let (l, r) = line
                .rsplit_once('=')
                .ok_or_else(|| bad("falta el «=» (por ejemplo: 0..10 = 1)"))?;
            let to = parse_out(r).ok_or_else(|| bad("el resultado debe ser un número o «nodata»"))?;
            if matches!(l.trim().to_lowercase().as_str(), "else" | "otros" | "resto" | "*") {
                t.otherwise = to;
                t.explicit_else = true;
                continue;
            }
            if to == Out::Keep {
                return Err(bad("«conservar» solo vale para «else»"));
            }
            let cond = parse_cond(l).ok_or_else(|| {
                bad("la condición debe ser un valor, un intervalo (a..b), < > <= >= o nodata")
            })?;
            if let Cond::Range(a, b) = cond
                && a > b
            {
                return Err(bad("el intervalo va del menor al mayor"));
            }
            t.rules.push(Rule { cond, to });
        }
        if t.rules.is_empty() && !t.explicit_else {
            return Err(Error::Param("no hay ninguna regla".into()));
        }
        Ok(t)
    }

    /// The value a cell becomes (`NaN` for empty). `v` is `NaN` for an empty cell.
    #[inline]
    pub fn apply(&self, v: f32) -> f32 {
        let x = f64::from(v);
        for r in &self.rules {
            let hit = match r.cond {
                Cond::NoData => v.is_nan(),
                _ if v.is_nan() => false,
                Cond::Eq(a) => x == a,
                Cond::Range(a, b) => x >= a && x <= b,
                Cond::Lt(a) => x < a,
                Cond::Le(a) => x <= a,
                Cond::Gt(a) => x > a,
                Cond::Ge(a) => x >= a,
            };
            if hit {
                return match r.to {
                    Out::Value(o) => o,
                    _ => f32::NAN,
                };
            }
        }
        match self.otherwise {
            Out::Value(o) => o,
            Out::NoData => f32::NAN,
            Out::Keep => v,
        }
    }

    /// The values the table can write, when they are all fixed (`None` if it keeps values).
    fn written(&self) -> Option<Vec<f32>> {
        if self.otherwise == Out::Keep {
            return None;
        }
        let mut v: Vec<f32> = self
            .rules
            .iter()
            .filter_map(|r| match r.to {
                Out::Value(o) => Some(o),
                _ => None,
            })
            .collect();
        if let Out::Value(o) = self.otherwise {
            v.push(o);
        }
        Some(v)
    }

    fn uses_nodata(&self) -> bool {
        self.otherwise == Out::NoData || self.rules.iter().any(|r| r.to == Out::NoData)
    }
}

#[derive(Clone, Debug)]
pub struct ReclassJob {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Band to reclassify, from 1.
    pub band: usize,
    pub table: Table,
    /// A value of the input that means "no data" (besides what its file declares).
    pub src_nodata: Option<f32>,
    /// Sample type of the output; chosen from the values the table writes when `None`.
    pub dtype: Option<Dtype>,
    /// What empty cells hold; chosen with the type when `None`.
    pub nodata: Option<f32>,
    pub compression: Option<crate::tiled::Compression>,
}

impl ReclassJob {
    pub fn new(input: PathBuf, output: PathBuf, table: Table) -> Self {
        Self {
            input,
            output,
            band: 1,
            table,
            src_nodata: None,
            dtype: None,
            nodata: None,
            compression: None,
        }
    }
}

/// The type and the "no data" value that hold what the table writes.
fn pick_type(job: &ReclassJob, source_bits: u8) -> (Dtype, f32) {
    let whole = |v: &f32| v.fract() == 0.0 && *v >= 0.0;
    let (dtype, nd) = match (job.dtype, job.table.written()) {
        (Some(d), _) => (d, None),
        (None, Some(v)) if v.iter().all(whole) => {
            let top = v.iter().copied().fold(0.0f32, f32::max);
            let want = job.nodata.filter(|n| whole(n));
            let room = |limit: f32| top < limit && want.is_none_or(|n| n <= limit && !v.contains(&n));
            if room(255.0) {
                (Dtype::U8, Some(255.0))
            } else if room(65535.0) {
                (Dtype::U16, Some(65535.0))
            } else {
                (Dtype::F32, None)
            }
        }
        (None, Some(_)) => (Dtype::F32, None),
        // values kept: the type of the input
        (None, None) => (
            match source_bits {
                0..=8 => Dtype::U8,
                9..=16 => Dtype::U16,
                _ => Dtype::F32,
            },
            None,
        ),
    };
    let default = nd.unwrap_or(match dtype {
        Dtype::U8 => 255.0,
        Dtype::U16 => 65535.0,
        Dtype::F32 => -9999.0,
    });
    (dtype, job.nodata.unwrap_or(default))
}

/// Writes the reclassified band as a GeoTIFF with tiles and overviews. Returns a line for the
/// report.
pub fn run(job: &ReclassJob) -> Result<String> {
    let pyr = Pyramid::open(&job.input)?;
    if job.band == 0 || job.band > pyr.bands {
        return Err(Error::Param(format!(
            "{} tiene {} banda(s): no existe la banda {}",
            job.input.display(),
            pyr.bands,
            job.band
        )));
    }
    let (dtype, nodata) = pick_type(job, pyr.bits);
    if job.table.uses_nodata()
        && dtype != Dtype::F32
        && job.table.written().is_some_and(|v| v.contains(&nodata))
    {
        return Err(Error::Param(format!(
            "el valor {nodata} es a la vez una clase y el valor de «sin datos»: elige otro valor de «sin datos» o el tipo decimal"
        )));
    }
    let (cols, rows) = (pyr.levels[0].cols as usize, pyr.levels[0].rows as usize);
    let declared = pyr.nodata();
    let src_nd = job.src_nodata;
    let band = job.band - 1;
    let step = (16_000_000 / cols.max(1)).clamp(8, 4096);
    let spec = crate::tiled::Spec {
        cols,
        rows,
        bands: 1,
        dtype,
        cell: pyr.cell,
        x0: pyr.x0,
        top: pyr.y_top,
        epsg: pyr.epsg,
        nodata,
        compression: job
            .compression
            .unwrap_or_else(|| crate::tiled::auto_compression(dtype, 1)),
        // classes are not averaged in the overviews
        nearest: dtype != Dtype::F32,
        bigtiff: false,
    };
    crate::paths::ensure_parent(&job.output)?;
    crate::progress::begin(rows as u64);
    let table = &job.table;
    let counts = std::sync::Mutex::new((0u64, 0u64));
    crate::tiled::write(&job.output, spec, step, |r0, r1| {
        crate::progress::advance((r1 - r0) as u64);
        let mut img = pyr.read_rows(r0, r1)?;
        let mut data = std::mem::take(&mut img.bands[band]);
        if data.len() != (r1 - r0) * cols {
            return Err(Error::Format("el bloque leído no tiene el tamaño esperado".into()));
        }
        let changed: u64 = data
            .par_chunks_mut(1 << 16)
            .map(|chunk| {
                let mut n = 0u64;
                for v in chunk {
                    let src = if !v.is_finite() || declared == Some(*v) || src_nd == Some(*v) {
                        f32::NAN
                    } else {
                        *v
                    };
                    let out = table.apply(src);
                    n += u64::from(!(out == src || (out.is_nan() && src.is_nan())));
                    *v = out;
                }
                n
            })
            .sum();
        if let Ok(mut c) = counts.lock() {
            c.0 += changed;
            c.1 += data.len() as u64;
        }
        Ok(data)
    })?;
    let (changed, total) = counts.into_inner().unwrap_or((0, 0));
    Ok(format!(
        "{}: {} de {} celdas cambiadas ({} x {}), tipo {} → {}",
        crate::paths::stem(&job.input),
        changed,
        total,
        cols,
        rows,
        match dtype {
            Dtype::U8 => "8 bits",
            Dtype::U16 => "16 bits",
            Dtype::F32 => "decimal",
        },
        job.output.display()
    ))
}

/// The classes found in a raster and how many cells each has (the first `limit` distinct values
/// found while scanning an overview of at most 4 million cells): helps to write the rules.
pub fn values_of(path: &Path, band: usize, limit: usize) -> Result<Vec<(f32, u64)>> {
    let pyr = Pyramid::open(path)?;
    let (img, _) = pyr.overview(4_000_000)?;
    let b = img
        .bands
        .get(band.saturating_sub(1))
        .ok_or_else(|| Error::Param("no existe esa banda".into()))?;
    let nd = pyr.nodata();
    let mut map: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
    for v in b {
        if v.is_finite() && nd != Some(*v) {
            if v.fract() != 0.0 {
                return Ok(Vec::new());
            }
            *map.entry((*v + 1.0e9) as u32).or_insert(0) += 1;
            if map.len() > limit {
                return Ok(Vec::new());
            }
        }
    }
    Ok(map.into_iter().map(|(k, n)| (k as f32 - 1.0e9, n)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;

    #[test]
    fn rules_parse_and_the_first_match_wins() -> Result<()> {
        let t = Table::parse("0..10 = 1\n10 - 20 = 2; >= 100 = 3\n-5 = 9\nnodata = 0\nelse = nodata")?;
        assert_eq!(t.rules.len(), 5);
        assert_eq!(t.apply(5.0), 1.0);
        assert_eq!(t.apply(10.0), 1.0, "the first rule that matches decides");
        assert_eq!(t.apply(15.5), 2.0);
        assert_eq!(t.apply(100.0), 3.0);
        assert_eq!(t.apply(-5.0), 9.0);
        assert_eq!(t.apply(f32::NAN), 0.0);
        assert!(t.apply(50.0).is_nan());
        let keep = Table::parse("1 = 5")?;
        assert_eq!(keep.apply(7.0), 7.0);
        assert!(!keep.explicit_else);
        let neg = Table::parse("-10 - -2 = 1\n-1,5..0,5 = 2\n< -20 = 3")?;
        assert_eq!(neg.apply(-3.0), 1.0);
        assert_eq!(neg.apply(0.0), 2.0);
        assert_eq!(neg.apply(-30.0), 3.0);
        assert!(Table::parse("hola").is_err());
        assert!(Table::parse("5 = x").is_err());
        assert!(Table::parse("9..2 = 1").is_err());
        assert!(Table::parse("").is_err());
        Ok(())
    }

    #[test]
    fn a_raster_is_reclassified_into_the_smallest_type() -> Result<()> {
        let d = std::env::temp_dir().join(format!("rl-reclass-{}", std::process::id()));
        std::fs::create_dir_all(&d)?;
        let mut g = Grid::new(0.0, 0.0, 1.0, 40, 30)?;
        for (i, v) in g.data.iter_mut().enumerate() {
            *v = (i % 40) as f32 * 2.5;
        }
        g.data[3] = f32::NAN;
        g.write_tiff(&d.join("dem.tif"), Some(25830))?;
        let table = Table::parse("<25 = 1\n25..60 = 2\nelse = 3")?;
        let job = ReclassJob::new(d.join("dem.tif"), d.join("cls.tif"), table);
        let line = run(&job)?;
        let out = crate::raster::read_image(&d.join("cls.tif"))?;
        assert_eq!(out.bands[0][0], 1.0);
        assert_eq!(out.bands[0][10], 2.0);
        assert_eq!(out.bands[0][39], 3.0);
        // the empty cell goes to the "else" class, as no rule names it
        assert_eq!(out.bands[0][3], 3.0, "{line}");
        let p = Pyramid::open(&d.join("cls.tif"))?;
        assert_eq!(p.bits, 8);
        // a table that names the empty cells keeps them empty
        let table = Table::parse("nodata = nodata\n<25 = 1\nelse = 2")?;
        run(&ReclassJob::new(d.join("dem.tif"), d.join("c2.tif"), table))?;
        let g2 = Grid::read(&d.join("c2.tif"))?;
        assert!(g2.data[3].is_nan() && g2.data[0] == 1.0);
        std::fs::remove_dir_all(&d)?;
        Ok(())
    }
}
