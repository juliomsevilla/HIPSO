//! Taking points out of LAS/LAZ files: to ASCII columns and to 3D point shapefiles, with the
//! same filters as the other tools. Streams the whole file, so memory does not depend on size.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::filter::Filter;
use crate::las::Reader;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    X,
    Y,
    Z,
    Intensity,
    ReturnNumber,
    Returns,
    Class,
    SourceId,
    Red,
    Green,
    Blue,
}

impl Item {
    pub const ALL: [Item; 11] = [
        Item::X,
        Item::Y,
        Item::Z,
        Item::Intensity,
        Item::ReturnNumber,
        Item::Returns,
        Item::Class,
        Item::SourceId,
        Item::Red,
        Item::Green,
        Item::Blue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Item::X => "X",
            Item::Y => "Y",
            Item::Z => "Z",
            Item::Intensity => "Intensidad",
            Item::ReturnNumber => "N.º de retorno",
            Item::Returns => "Retornos",
            Item::Class => "Clase",
            Item::SourceId => "ID de fuente",
            Item::Red => "Rojo",
            Item::Green => "Verde",
            Item::Blue => "Azul",
        }
    }

    fn column(self) -> &'static str {
        match self {
            Item::X => "X",
            Item::Y => "Y",
            Item::Z => "Z",
            Item::Intensity => "Intensity",
            Item::ReturnNumber => "ReturnNumber",
            Item::Returns => "NumberOfReturns",
            Item::Class => "Classification",
            Item::SourceId => "PointSourceID",
            Item::Red => "Red",
            Item::Green => "Green",
            Item::Blue => "Blue",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AsciiJob {
    pub inputs: Vec<PathBuf>,
    /// One file for all inputs, or the folder that gets one `.csv` per input.
    pub output: PathBuf,
    pub items: Vec<Item>,
    pub delimiter: char,
    pub header: bool,
    pub filter: Filter,
    pub decimals: usize,
}

fn rgb_at(format: u8) -> Option<usize> {
    match format {
        2 => Some(20),
        3 | 5 => Some(28),
        7 | 8 | 10 => Some(30),
        _ => None,
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

/// Calls `f(point, record)` for every kept point of a file; how many were kept.
fn each(
    r: &Reader,
    filter: &Filter,
    mut f: impl FnMut(&crate::las::Point, &[u8]) -> Result<()>,
) -> Result<u64> {
    let (rec_len, mut n, mut kept) = (usize::from(r.header.record_len), 0u64, 0u64);
    r.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let Some(p) = r.decode(rec) else { continue };
            if !filter.keeps(&p) {
                continue;
            }
            n += 1;
            if filter.decimate > 1 && (n - 1) % filter.decimate != 0 {
                continue;
            }
            kept += 1;
            f(&p, rec)?;
        }
        Ok(())
    })?;
    Ok(kept)
}

/// Writes the chosen items of every kept point as delimited text; returns (files, points).
pub fn ascii(job: &AsciiJob) -> Result<(Vec<PathBuf>, u64)> {
    if job.items.is_empty() {
        return Err(Error::Param("choose at least one item to export".into()));
    }
    let many = job.inputs.len() > 1 && job.output.is_dir();
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut shared: Option<BufWriter<std::fs::File>> = None;
    for (k, path) in job.inputs.iter().enumerate() {
        let r = Reader::open(path)?;
        let out_path = if many {
            job.output.join(format!("{}.csv", crate::paths::stem(path)))
        } else {
            job.output.clone()
        };
        if many || shared.is_none() {
            let mut f = BufWriter::new(std::fs::File::create(&out_path)?);
            if job.header {
                let names: Vec<&str> = job.items.iter().map(|i| i.column()).collect();
                writeln!(f, "{}", names.join(&job.delimiter.to_string()))?;
            }
            shared = Some(f);
        }
        let Some(w) = shared.as_mut() else { continue };
        let rgb = rgb_at(r.header.format);
        let src_at = if r.header.format >= 6 { 20 } else { 18 };
        let d = job.delimiter;
        let dec = job.decimals;
        let n = each(&r, &job.filter, |p, rec| {
            for (c, item) in job.items.iter().enumerate() {
                if c > 0 {
                    write!(w, "{d}")?;
                }
                match item {
                    Item::X => write!(w, "{:.dec$}", p.x)?,
                    Item::Y => write!(w, "{:.dec$}", p.y)?,
                    Item::Z => write!(w, "{:.dec$}", p.z)?,
                    Item::Intensity => write!(w, "{}", p.intensity)?,
                    Item::ReturnNumber => write!(w, "{}", p.return_number)?,
                    Item::Returns => write!(w, "{}", p.number_of_returns)?,
                    Item::Class => write!(w, "{}", p.class)?,
                    Item::SourceId => write!(w, "{}", u16_at(rec, src_at))?,
                    Item::Red => write!(w, "{}", rgb.map_or(0, |a| u16_at(rec, a)))?,
                    Item::Green => write!(w, "{}", rgb.map_or(0, |a| u16_at(rec, a + 2)))?,
                    Item::Blue => write!(w, "{}", rgb.map_or(0, |a| u16_at(rec, a + 4)))?,
                }
            }
            writeln!(w)?;
            Ok(())
        })?;
        total += n;
        if many || k + 1 == job.inputs.len() {
            w.flush()?;
            files.push(out_path);
        }
    }
    Ok((files, total))
}

/// Writes the kept points as a PointZ shapefile (X, Y, Z, plus class, intensity and return
/// attributes).
pub fn shapefile_points(input: &Path, output: &Path, filter: &Filter) -> Result<u64> {
    use shapefile::dbase::{FieldName, FieldValue, Record, TableWriterBuilder};
    let name = |s: &str| FieldName::try_from(s).map_err(|e| Error::Param(format!("{e:?}")));
    let table = TableWriterBuilder::new()
        .add_numeric_field(name("CLASS")?, 3, 0)
        .add_numeric_field(name("INTENSITY")?, 6, 0)
        .add_numeric_field(name("RETURN")?, 2, 0);
    let mut w =
        shapefile::Writer::from_path(output, table).map_err(|e| Error::Format(e.to_string()))?;
    let r = Reader::open(input)?;
    each(&r, filter, |p, _| {
        let mut rec = Record::default();
        rec.insert(
            "CLASS".into(),
            FieldValue::Numeric(Some(f64::from(p.class))),
        );
        rec.insert(
            "INTENSITY".into(),
            FieldValue::Numeric(Some(f64::from(p.intensity))),
        );
        rec.insert(
            "RETURN".into(),
            FieldValue::Numeric(Some(f64::from(p.return_number))),
        );
        w.write_shape_and_record(&shapefile::PointZ::new(p.x, p.y, p.z, 0.0), &rec)
            .map_err(|e| Error::Format(e.to_string()))
    })
}
