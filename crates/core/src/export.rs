//! Extracción de puntos de archivos LAS/LAZ: a LAS/LAZ, a columnas ASCII y a shapefiles de puntos
//! 3D, con los mismos filtros que las demás herramientas y, opcionalmente, alturas sobre el suelo
//! (cada nube menos su propio MDT). Recorre el archivo entero en flujo, de modo que la memoria no
//! depende del tamaño.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::filter::Filter;
use crate::grid::Grid;
use crate::las::{self, Reader, Writer, WriterOptions};
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
    /// Un archivo para todas las entradas, o la carpeta que recibe un `.csv` por entrada.
    pub output: PathBuf,
    pub items: Vec<Item>,
    pub delimiter: char,
    pub header: bool,
    pub filter: Filter,
    pub decimals: usize,
    /// Un MDT por entrada para exportar alturas sobre el suelo; vacío: las elevaciones tal como
    /// están.
    pub dtms: Vec<PathBuf>,
}

/// Lo que escribió una exportación.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub points: u64,
    /// Puntos que se dejan fuera porque el MDT no los cubre (solo al normalizar).
    pub no_ground: u64,
}

impl Exported {
    pub fn add(&mut self, other: &Exported) {
        self.points += other.points;
        self.no_ground += other.no_ground;
    }

    /// La frase que muestran las herramientas tras la exportación.
    pub fn text(&self, to: &str) -> String {
        let mut s = format!("{} puntos exportados a {to}", self.points);
        if self.no_ground > 0 {
            s.push_str(&format!(
                " ({} sin MDT debajo no se exportaron)",
                self.no_ground
            ));
        }
        s
    }
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

/// Llama a `f(punto, registro)` para cada punto conservado de un archivo (con `ground`, a su altura
/// sobre él; los puntos que el MDT no cubre se saltan); cuántos se conservaron y a cuántos les
/// faltó suelo.
fn each(
    r: &Reader,
    filter: &Filter,
    ground: Option<&Grid>,
    mut f: impl FnMut(&crate::las::Point, &[u8]) -> Result<()>,
) -> Result<Exported> {
    let (rec_len, mut n) = (usize::from(r.header.record_len), 0u64);
    let mut out = Exported::default();
    r.scan_raw(|bytes| {
        crate::progress::advance((bytes.len() / rec_len.max(1)) as u64);
        for rec in bytes.chunks_exact(rec_len) {
            let Some(mut p) = r.decode(rec) else { continue };
            if !filter.keeps(&p) {
                continue;
            }
            n += 1;
            if filter.decimate > 1 && (n - 1) % filter.decimate != 0 {
                continue;
            }
            if let Some(g) = ground {
                match g.sample(p.x, p.y) {
                    Some(gz) => p.z -= f64::from(gz),
                    None => {
                        out.no_ground += 1;
                        continue;
                    }
                }
            }
            if !filter.z_ok(p.z) {
                continue;
            }
            out.points += 1;
            f(&p, rec)?;
        }
        Ok(())
    })?;
    Ok(out)
}

/// Dice al contador de progreso cuántos puntos contienen las entradas (antes de un bucle sobre
/// varias).
pub fn begin_for(inputs: &[PathBuf]) {
    crate::progress::begin(points_in(inputs));
}

fn points_in(inputs: &[PathBuf]) -> u64 {
    inputs
        .iter()
        .filter_map(|p| Reader::open(p).ok())
        .map(|r| r.header.count)
        .sum()
}

/// El MDT de la entrada `k`, cuando el trabajo normaliza.
fn ground_of(dtms: &[PathBuf], k: usize) -> Result<Option<Grid>> {
    dtms.get(k).map(|p| Grid::read(p)).transpose()
}

/// Escribe los elementos elegidos de cada punto conservado como texto delimitado; devuelve
/// (archivos, puntos).
pub fn ascii(job: &AsciiJob) -> Result<(Vec<PathBuf>, Exported)> {
    begin_for(&job.inputs);
    if job.items.is_empty() {
        return Err(Error::Param("elige al menos un dato que exportar".into()));
    }
    let many = job.inputs.len() > 1 && job.output.is_dir();
    let mut files = Vec::new();
    let mut total = Exported::default();
    let mut shared: Option<BufWriter<std::fs::File>> = None;
    for (k, path) in job.inputs.iter().enumerate() {
        let r = Reader::open(path)?;
        let ground = ground_of(&job.dtms, k)?;
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
        let n = each(&r, &job.filter, ground.as_ref(), |p, rec| {
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
        total.add(&n);
        if many || k + 1 == job.inputs.len() {
            w.flush()?;
            files.push(out_path);
        }
    }
    Ok((files, total))
}

/// Escribe los puntos conservados como shapefile PointZ (X, Y, Z, más atributos de clase,
/// intensidad y retorno).
pub fn shapefile_points(
    input: &Path,
    output: &Path,
    filter: &Filter,
    dtm: Option<&Path>,
) -> Result<Exported> {
    use shapefile::dbase::{FieldName, FieldValue, Record, TableWriterBuilder};
    let name = |s: &str| FieldName::try_from(s).map_err(|e| Error::Param(format!("{e:?}")));
    let table = TableWriterBuilder::new()
        .add_numeric_field(name("CLASS")?, 3, 0)
        .add_numeric_field(name("INTENSITY")?, 6, 0)
        .add_numeric_field(name("RETURN")?, 2, 0);
    let mut w =
        shapefile::Writer::from_path(output, table).map_err(|e| Error::Format(e.to_string()))?;
    crate::progress::begin_if_idle(points_in(&[input.to_path_buf()]));
    let r = Reader::open(input)?;
    let ground = dtm.map(Grid::read).transpose()?;
    each(&r, filter, ground.as_ref(), |p, _| {
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

/// Escribe los puntos conservados como archivo LAS o LAZ (según la extensión de `output`),
/// conservando cada campo de los originales; con `dtm`, Z pasa a ser la altura sobre el suelo.
pub fn cloud(input: &Path, output: &Path, filter: &Filter, dtm: Option<&Path>) -> Result<Exported> {
    crate::progress::begin_if_idle(points_in(&[input.to_path_buf()]));
    let r = Reader::open(input)?;
    let ground = dtm.map(Grid::read).transpose()?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut w = Writer::create_with(output, &r, &WriterOptions::default())?;
    let mut copy = Vec::new();
    let done = each(&r, filter, ground.as_ref(), |p, rec| {
        if ground.is_none() {
            return w.write(rec);
        }
        copy.clear();
        copy.extend_from_slice(rec);
        if !las::set_xyz(&r.header, &mut copy, p.x, p.y, p.z) {
            return Err(Error::Param(
                "una altura fuera del rango de coordenadas del archivo".into(),
            ));
        }
        w.write(&copy)
    })?;
    w.finish()?;
    Ok(done)
}
