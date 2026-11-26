//! Recorte de nubes de puntos con polígonos, círculos y rectángulos: selección de puntos por
//! región. Una pasada sobre la nube escribe cualquier número de salidas, y las parcelas se pueden
//! medir sobre la marcha (CloudMetrics).

use std::path::{Path, PathBuf};

use crate::filter::Filter;
use crate::grid::Grid;
use crate::las::{self, Reader, Writer, WriterOptions};
use crate::metrics;
use crate::vector::{Region, RegionIndex};
use crate::{Error, Result};

/// Qué va adónde.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipMode {
    /// Un archivo con los puntos dentro de cualquier región.
    Merge,
    /// Un archivo con los puntos dentro de ninguna.
    Outside,
    /// Un archivo por región.
    PerRegion,
    /// Sin archivo de puntos: solo las métricas de cada región.
    MetricsOnly,
}

/// CloudMetrics de los puntos de cada región, a medida que se recorta.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleMetrics {
    /// Ruta base de la tabla de métricas (su extensión se sustituye por los formatos elegidos).
    pub csv: PathBuf,
    pub formats: crate::paths::Formats,
    /// Altura de corte para las métricas de cobertura (height break).
    pub heightbreak: f64,
    /// Solo cuentan los retornos por encima de esta altura (altura mínima).
    pub min_height: f64,
    /// Las métricas que escribir, por el nombre de su columna (`Elev P90`, `Int mean`…); vacío:
    /// todas. Un nombre que no es una columna es un error que lista los válidos.
    pub only: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ClipJob {
    pub inputs: Vec<PathBuf>,
    pub regions: Vec<Region>,
    pub mode: ClipMode,
    /// El archivo para `Merge` y `Outside`; para `PerRegion`, `<dir>/<nombre>.<ext>` da
    /// `<dir>/<nombre>_<región>.<ext>`.
    pub output: PathBuf,
    pub filter: Filter,
    /// Un modelo del suelo: los puntos reciben como elevación su altura sobre él.
    pub dtm: Option<PathBuf>,
    /// Un modelo del suelo por entrada, en orden, en lugar de `dtm` (cada nube menos su propio
    /// MDT).
    pub dtms: Vec<PathBuf>,
    /// Se suma a cada elevación (o altura) escrita.
    pub z_offset: f64,
    /// Un punto va solo a la primera región que lo contiene (el comportamiento de PolyClipData), no
    /// a todas.
    pub exclusive: bool,
    /// Escribe también un archivo para las regiones sin puntos.
    pub keep_empty: bool,
    /// Archivos abiertos a la vez; más regiones requieren más pasadas sobre la nube.
    pub max_open: usize,
    /// El archivo del que se leyeron las regiones, para poder comprobar su sistema de coordenadas
    /// contra el de las nubes (las regiones dibujadas en el visor no lo tienen).
    pub regions_source: Option<PathBuf>,
    pub metrics: Option<SampleMetrics>,
}

impl ClipJob {
    pub fn new(inputs: Vec<PathBuf>, regions: Vec<Region>, output: PathBuf) -> Self {
        Self {
            inputs,
            regions,
            mode: ClipMode::Merge,
            output,
            filter: Filter::default(),
            dtm: None,
            dtms: Vec::new(),
            z_offset: 0.0,
            exclusive: false,
            keep_empty: false,
            max_open: 64,
            regions_source: None,
            metrics: None,
        }
    }
}

/// Una salida de un recorte.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipFile {
    pub region: String,
    pub path: PathBuf,
    pub points: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipReport {
    pub files: Vec<ClipFile>,
    pub points_read: u64,
    pub points_written: u64,
    /// Puntos que se dejan fuera porque el modelo del suelo no tiene nada bajo ellos.
    pub no_ground: u64,
    /// Pasadas sobre la nube (las regiones se toman en grupos de `max_open`).
    pub passes: usize,
}

/// Una versión del nombre de una región segura como nombre de archivo.
fn safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '(' | ')') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches(|c| c == '_' || c == '.').to_string();
    if s.is_empty() { "region".into() } else { s }
}

/// `<dir>/<nombre>_<nombre>.<ext>`, con un número añadido cuando dos regiones comparten nombre.
fn region_path(base: &Path, name: &str, used: &mut Vec<PathBuf>) -> PathBuf {
    let stem = base
        .file_stem()
        .map_or("recorte".into(), |s| s.to_string_lossy().into_owned());
    let ext = base
        .extension()
        .map_or("las".into(), |e| e.to_string_lossy().into_owned());
    let mut p = base.with_file_name(format!("{stem}_{}.{ext}", safe(name)));
    let mut n = 2;
    while used.contains(&p) {
        p = base.with_file_name(format!("{stem}_{}_{n}.{ext}", safe(name)));
        n += 1;
    }
    used.push(p.clone());
    p
}

/// Lo que reúne una región para sus métricas.
#[derive(Default)]
struct Sample {
    heights: Vec<f32>,
    ret: Vec<u8>,
    intensity: Vec<f32>,
}

/// El trabajo con todo en un mismo sistema de coordenadas: las nubes y los modelos en otro se
/// convierten en copias temporales, las regiones en memoria.
pub fn unified(job: &ClipJob) -> Result<(ClipJob, crate::crs::Unified)> {
    let mut files = job.inputs.clone();
    files.extend(job.dtm.clone());
    files.extend(job.dtms.iter().cloned());
    files.extend(job.regions_source.clone());
    let u = crate::crs::unify(&files, true)?;
    let moved = ClipJob {
        inputs: u.all(&job.inputs),
        dtm: job.dtm.as_ref().map(|p| u.path(p)),
        dtms: u.all(&job.dtms),
        regions: u.regions(&job.regions, job.regions_source.as_deref())?,
        // las regiones están ahora en el sistema de las nubes: una segunda pasada no tiene nada que
        // convertir
        regions_source: None,
        ..job.clone()
    };
    Ok((moved, u))
}

pub fn run(job: &ClipJob) -> Result<ClipReport> {
    crate::paths::guard_output(&job.inputs, &job.output)?;
    if job.regions.is_empty() {
        return Err(Error::Param("no hay zonas con las que recortar".into()));
    }
    let (job, _unified) = unified(job)?;
    let job = &job;
    let readers: Vec<Reader> = job
        .inputs
        .iter()
        .map(|p| Reader::open(p))
        .collect::<Result<_>>()?;
    let template = readers
        .first()
        .ok_or_else(|| Error::Param("no hay archivos de entrada".into()))?;
    for r in &readers {
        if r.header.format != template.header.format
            || r.header.record_len != template.header.record_len
        {
            return Err(Error::Unsupported(format!(
                "{}: su formato de punto es distinto del del primer archivo; recórtalos por separado",
                r.path().display()
            )));
        }
    }
    if let Some(parent) = job.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let shared = job.dtm.as_deref().map(Grid::read).transpose()?;
    let own: Vec<Grid> = job
        .dtms
        .iter()
        .map(|p| Grid::read(p))
        .collect::<Result<_>>()?;
    if !own.is_empty() && own.len() != job.inputs.len() {
        return Err(Error::Param(
            "indica un MDT por cada nube de entrada para normalizarlas".into(),
        ));
    }
    let mut report = ClipReport::default();
    let mut sample_rows: Vec<(usize, Vec<String>)> = Vec::new();
    let mut used = Vec::new();

    // Los grupos de regiones que maneja cada pasada: todas a la vez, salvo un archivo por región.
    let groups: Vec<std::ops::Range<usize>> = match job.mode {
        ClipMode::PerRegion => {
            let n = job.max_open.max(1);
            (0..job.regions.len())
                .step_by(n)
                .map(|s| s..(s + n).min(job.regions.len()))
                .collect()
        }
        _ => std::iter::once(0..job.regions.len()).collect(),
    };
    // Una nube comprimida leída varias veces se descomprime una vez. (un archivo COPC no: su árbol
    // permite a cada pasada leer solo los nodos que necesita)
    let readers: Vec<Reader> = if groups.len() > 1 {
        readers
            .into_iter()
            .map(|r| {
                if crate::copc::Copc::open(r.path())?.is_some() {
                    Ok(r)
                } else {
                    r.for_passes()
                }
            })
            .collect::<Result<_>>()?
    } else {
        readers
    };
    let template = &readers[0];
    let paths: Vec<PathBuf> = match job.mode {
        ClipMode::PerRegion => job
            .regions
            .iter()
            .map(|r| region_path(&job.output, &r.name, &mut used))
            .collect(),
        _ => Vec::new(),
    };

    crate::progress::begin(
        readers.iter().map(|r| r.header.count).sum::<u64>() * groups.len() as u64,
    );
    for group in groups {
        report.passes += 1;
        let regions = &job.regions[group.clone()];
        let index = RegionIndex::new(regions);
        let bb = index.bbox();
        let opts = if job.mode == ClipMode::PerRegion {
            WriterOptions::small()
        } else {
            WriterOptions::default()
        };
        let mut writers: Vec<Option<Writer>> = Vec::new();
        let mut counts: Vec<u64> = vec![0; regions.len()];
        let mut samples: Vec<Sample> = Vec::new();
        match job.mode {
            ClipMode::PerRegion => {
                for k in 0..regions.len() {
                    writers.push(if job.keep_empty {
                        Some(Writer::create_with(
                            &paths[group.start + k],
                            template,
                            &opts,
                        )?)
                    } else {
                        None
                    });
                }
                if job.metrics.is_some() {
                    samples = (0..regions.len()).map(|_| Sample::default()).collect();
                }
            }
            ClipMode::MetricsOnly => {
                writers.push(None);
                samples = (0..regions.len()).map(|_| Sample::default()).collect();
            }
            _ => {
                writers.push(Some(Writer::create_with(&job.output, template, &opts)?));
                if job.metrics.is_some() {
                    samples = (0..regions.len()).map(|_| Sample::default()).collect();
                }
            }
        }
        let mut merged_count = 0u64;
        let mut decimate_n = 0u64;
        for (ri, reader) in readers.iter().enumerate() {
            let dtm = if own.is_empty() {
                shared.as_ref()
            } else {
                own.get(ri)
            };
            let h = &reader.header;
            // Un archivo cuya extensión no alcanza ninguna región del grupo no se lee (salvo que
            // las regiones sean lo que se deja fuera).
            let reach = h.bounds[0] <= bb[2]
                && h.bounds[3] >= bb[0]
                && h.bounds[1] <= bb[3]
                && h.bounds[4] >= bb[1];
            if !reach && job.mode != ClipMode::Outside {
                continue;
            }
            let rec_len = usize::from(h.record_len);
            let same = h.scale == template.header.scale && h.offset == template.header.offset;
            // Un archivo COPC se lee por los nodos que tocan las regiones cuando son bastantes
            // menos que los del archivo entero.
            let copc_nodes = if job.mode == ClipMode::Outside {
                None
            } else {
                match crate::copc::Copc::open(reader.path())? {
                    Some(c) => {
                        let nodes = c.nodes(Some(bb), None)?;
                        let wanted: u64 = nodes.iter().map(|n| u64::from(n.points)).sum();
                        (wanted.saturating_mul(10) < h.count.saturating_mul(7))
                            .then_some((c, nodes))
                    }
                    None => None,
                }
            };
            let mut body = |bytes: &[u8]| -> Result<()> {
                crate::progress::advance((bytes.len() / rec_len.max(1)) as u64);
                for rec in bytes.chunks_exact(rec_len) {
                    let Some(p) = reader.decode(rec) else {
                        continue;
                    };
                    if !job.filter.keeps(&p) {
                        continue;
                    }
                    decimate_n += 1;
                    if job.filter.decimate > 1 && !decimate_n.is_multiple_of(job.filter.decimate) {
                        continue;
                    }
                    let mut z = p.z;
                    if let Some(g) = dtm {
                        match g.sample(p.x, p.y) {
                            Some(gz) => z = p.z - f64::from(gz),
                            None => {
                                report.no_ground += 1;
                                continue;
                            }
                        }
                    }
                    z += job.z_offset;
                    if !job.filter.z_ok(z) {
                        continue;
                    }
                    let mut hit: Vec<usize> = Vec::new();
                    index.for_each(p.x, p.y, |k| hit.push(k));
                    if job.exclusive && hit.len() > 1 {
                        hit.sort_unstable();
                        hit.truncate(1);
                    }
                    let take = match job.mode {
                        ClipMode::Outside => hit.is_empty(),
                        _ => !hit.is_empty(),
                    };
                    if !take {
                        continue;
                    }
                    let mut out: Vec<u8>;
                    let bytes_out: &[u8] = if same && dtm.is_none() && job.z_offset == 0.0 {
                        rec
                    } else {
                        out = rec.to_vec();
                        if !las::set_xyz(&template.header, &mut out, p.x, p.y, z) {
                            return Err(Error::Param(
                                "un punto fuera del rango de coordenadas del primer archivo".into(),
                            ));
                        }
                        &out
                    };
                    match job.mode {
                        ClipMode::PerRegion => {
                            for &k in &hit {
                                counts[k] += 1;
                                if writers[k].is_none() {
                                    writers[k] = Some(Writer::create_with(
                                        &paths[group.start + k],
                                        template,
                                        &opts,
                                    )?);
                                }
                                if let Some(w) = writers[k].as_mut() {
                                    w.write(bytes_out)?;
                                }
                                collect(&mut samples, k, job, z, p.return_number, p.intensity);
                            }
                        }
                        _ => {
                            if let Some(Some(w)) = writers.first_mut() {
                                w.write(bytes_out)?;
                            }
                            merged_count += 1;
                            for &k in &hit {
                                counts[k] += 1;
                                collect(&mut samples, k, job, z, p.return_number, p.intensity);
                            }
                        }
                    }
                }
                Ok(())
            };
            let scanned = match &copc_nodes {
                Some((c, nodes)) => c.read_raw(nodes, &mut body)?,
                None => reader.scan_raw(&mut body)?,
            };
            if report.passes == 1 {
                report.points_read += scanned;
            }
        }
        match job.mode {
            ClipMode::PerRegion => {
                for (k, w) in writers.into_iter().enumerate() {
                    if let Some(w) = w {
                        let n = w.finish()?;
                        report.points_written += n;
                        report.files.push(ClipFile {
                            region: regions[k].name.clone(),
                            path: paths[group.start + k].clone(),
                            points: n,
                        });
                    }
                }
            }
            _ => {
                for w in writers.into_iter().flatten() {
                    w.finish()?;
                }
                report.points_written += merged_count;
                if job.mode != ClipMode::MetricsOnly {
                    report.files.push(ClipFile {
                        region: if job.mode == ClipMode::Outside {
                            "fuera".into()
                        } else {
                            "dentro".into()
                        },
                        path: job.output.clone(),
                        points: merged_count,
                    });
                }
            }
        }
        if let Some(m) = &job.metrics {
            for (k, mut s) in samples.into_iter().enumerate() {
                let row =
                    metrics::sample_row(&mut s.heights, &s.ret, &mut s.intensity, m.heightbreak);
                sample_rows.push((group.start + k, row));
            }
        }
    }
    if let Some(m) = &job.metrics {
        write_metrics(m, job, &sample_rows)?;
    }
    Ok(report)
}

/// Guarda los números de un punto para las métricas de una región.
fn collect(samples: &mut [Sample], k: usize, job: &ClipJob, z: f64, ret: u8, intensity: u16) {
    let (Some(m), Some(s)) = (&job.metrics, samples.get_mut(k)) else {
        return;
    };
    if z > m.min_height {
        s.heights.push(z as f32);
        s.ret.push(ret);
        s.intensity.push(f32::from(intensity));
    }
}

fn write_metrics(m: &SampleMetrics, job: &ClipJob, rows: &[(usize, Vec<String>)]) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = m.csv.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    // el día o los días en que se capturaron las nubes, una métrica más de cada región
    let mut dates: Vec<String> = job
        .inputs
        .iter()
        .filter_map(|p| Reader::open(p).ok().and_then(|r| r.capture_date()))
        .collect();
    dates.sort();
    dates.dedup();
    let date = if dates.is_empty() {
        "NA".to_string()
    } else {
        dates.join("|")
    };
    let mut rows: Vec<&(usize, Vec<String>)> = rows.iter().collect();
    rows.sort_by_key(|r| r.0);
    let all = metrics::sample_header(m.heightbreak);
    // las columnas pedidas, en el orden pedido (todas, en su propio orden, cuando no se pide
    // ninguna)
    let keep: Vec<usize> = if m.only.is_empty() {
        (0..all.len()).collect()
    } else {
        m.only
            .iter()
            .map(|name| {
                all.iter().position(|c| c.eq_ignore_ascii_case(name.trim())).ok_or_else(|| {
                    Error::Param(format!(
                        "«{}» no es una métrica; las hay así: {}",
                        name.trim(),
                        all.join(", ")
                    ))
                })
            })
            .collect::<Result<_>>()?
    };
    let head: Vec<String> = keep.iter().map(|&k| all[k].clone()).collect();
    let pick = |row: &[String]| -> Vec<String> { keep.iter().map(|&k| row[k].clone()).collect() };
    let f = m.formats.fixed();
    if f.csv {
        let path = crate::paths::swap_ext(&m.csv, "csv");
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
        writeln!(out, "Muestra,Area,{},Date", head.join(","))?;
        for (k, row) in &rows {
            let r = &job.regions[*k];
            writeln!(
                out,
                "{},{:.2},{},{date}",
                r.name.replace(',', " "),
                r.area(),
                pick(row).join(",")
            )?;
        }
        out.flush()?;
        if !f.vector {
            return Ok(());
        }
    }
    // un shapefile: el propio polígono de cada región con las métricas como atributos
    let mut columns = vec!["Muestra".to_string(), "Area".to_string()];
    columns.extend(head);
    columns.push("Date".to_string());
    let prj = job
        .inputs
        .first()
        .and_then(|p| Reader::open(p).ok())
        .and_then(|r| r.header.epsg)
        .and_then(crate::vector::prj_for_epsg);
    let mut table: Option<crate::vector::PolyTable> = None;
    for (k, row) in &rows {
        let r = &job.regions[*k];
        let mut values = vec![r.name.clone(), format!("{:.2}", r.area())];
        values.extend(pick(row));
        values.push(date.clone());
        let rings: Vec<Vec<[f64; 2]>> = match &r.shape {
            crate::vector::Shape2::Polygon(p) => p.rings.clone(),
            crate::vector::Shape2::Buffered(b) => b.poly.rings.clone(),
            crate::vector::Shape2::Circle { cx, cy, r } => crate::edit::circle(*cx, *cy, *r).rings,
            crate::vector::Shape2::Rect(b) => {
                vec![vec![[b[0], b[1]], [b[0], b[3]], [b[2], b[3]], [b[2], b[1]]]]
            }
        };
        if table.is_none() {
            table = Some(crate::vector::PolyTable::create(
                &crate::paths::swap_ext(&m.csv, "shp"),
                &columns,
                &values,
                prj.as_deref(),
            )?);
        }
        if let Some(t) = table.as_mut() {
            t.push(&rings, &values)?;
        }
    }
    if let Some(t) = table {
        t.finish()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::Shape2;

    /// Un archivo LAS 1.2 de formato 1 con los puntos (x, y, z, clase) dados en una rejilla de
    /// 0,01.
    fn write_las(path: &Path, pts: &[(f64, f64, f64, u8)]) -> Result<()> {
        let mut h = vec![0u8; 227];
        h[..4].copy_from_slice(b"LASF");
        h[24] = 1;
        h[25] = 2;
        h[94..96].copy_from_slice(&227u16.to_le_bytes());
        h[96..100].copy_from_slice(&227u32.to_le_bytes());
        h[104] = 1;
        h[105..107].copy_from_slice(&28u16.to_le_bytes());
        h[107..111].copy_from_slice(&(pts.len() as u32).to_le_bytes());
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in pts {
            for (k, v) in [p.0, p.1, p.2].into_iter().enumerate() {
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
        }
        for (at, v) in [
            (131, 0.01),
            (139, 0.01),
            (147, 0.01),
            (155, 0.0),
            (163, 0.0),
            (171, 0.0),
        ] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for (at, v) in [
            (179, hi[0]),
            (187, lo[0]),
            (195, hi[1]),
            (203, lo[1]),
            (211, hi[2]),
            (219, lo[2]),
        ] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for p in pts {
            let mut rec = [0u8; 28];
            for (k, v) in [p.0, p.1, p.2].into_iter().enumerate() {
                rec[4 * k..4 * k + 4].copy_from_slice(&((v / 0.01).round() as i32).to_le_bytes());
            }
            rec[12..14].copy_from_slice(&50u16.to_le_bytes());
            rec[14] = 1 | (1 << 3);
            rec[15] = p.3;
            h.extend_from_slice(&rec);
        }
        std::fs::write(path, h)?;
        Ok(())
    }

    fn cloud(dir: &Path) -> Result<PathBuf> {
        let mut pts = Vec::new();
        for i in 0..100 {
            for j in 0..100 {
                // Suelo a 100 m con un árbol de 10 m en medio de cada bloque de 50 m.
                let (x, y) = (f64::from(i), f64::from(j));
                let tree = (i % 50 - 25_i32).abs() < 3 && (j % 50 - 25_i32).abs() < 3;
                pts.push((
                    1000.0 + x,
                    2000.0 + y,
                    if tree { 110.0 } else { 100.0 },
                    if tree { 5 } else { 2 },
                ));
            }
        }
        let p = dir.join("cloud.las");
        write_las(&p, &pts)?;
        Ok(p)
    }

    fn tmp(tag: &str) -> Result<PathBuf> {
        let d = std::env::temp_dir().join(format!("rl_clip_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    fn circle(name: &str, cx: f64, cy: f64, r: f64) -> Region {
        Region {
            name: name.into(),
            shape: Shape2::Circle { cx, cy, r },
            attrs: vec![],
        }
    }

    #[test]
    fn only_the_metrics_asked_for_are_written() -> Result<()> {
        let dir = tmp("only")?;
        let input = cloud(&dir)?;
        let mut job = ClipJob::new(vec![input], vec![circle("A", 1025.0, 2025.0, 10.0)], dir.join("out/plot.las"));
        job.mode = ClipMode::MetricsOnly;
        let metrics = |only: &[&str]| SampleMetrics {
            csv: dir.join("out/m.csv"),
            formats: crate::paths::Formats { vector: false, csv: true },
            heightbreak: 2.0,
            min_height: -1e9,
            only: only.iter().map(|s| s.to_string()).collect(),
        };
        job.metrics = Some(metrics(&["Elev P90", "elev mean", "Int mean"]));
        run(&job)?;
        let csv = std::fs::read_to_string(dir.join("out/m.csv"))?;
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some("Muestra,Area,Elev P90,Elev mean,Int mean,Date"));
        assert_eq!(lines.next().map(|l| l.split(',').count()), Some(6));
        job.metrics = Some(metrics(&["Nada"]));
        let e = run(&job).unwrap_err().to_string();
        assert!(e.contains("no es una métrica") && e.contains("Elev P90"), "{e}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn each_region_gets_its_own_file_and_metrics() -> Result<()> {
        let dir = tmp("each")?;
        let input = cloud(&dir)?;
        let regions = vec![
            circle("A", 1025.0, 2025.0, 10.0),
            circle("B", 1075.0, 2075.0, 10.0),
        ];
        let mut job = ClipJob::new(vec![input], regions, dir.join("out/plot.las"));
        job.mode = ClipMode::PerRegion;
        job.metrics = Some(SampleMetrics {
            csv: dir.join("out/m.csv"),
            formats: crate::paths::Formats {
                vector: true,
                csv: true,
            },
            heightbreak: 2.0,
            min_height: -1e9,
            only: Vec::new(),
        });
        let r = run(&job)?;
        assert_eq!(r.files.len(), 2);
        assert!(r.files[0].path.ends_with("plot_A.las"));
        // Un disco de radio 10 contiene unos 314 puntos; ambos se parecen.
        assert!(
            (300..330).contains(&r.files[0].points),
            "{}",
            r.files[0].points
        );
        assert_eq!(r.files[0].points, r.files[1].points);
        let back = Reader::open(&r.files[0].path)?;
        assert_eq!(back.header.count, r.files[0].points);
        let csv = std::fs::read_to_string(dir.join("out/m.csv"))?;
        assert_eq!(csv.lines().count(), 3);
        assert!(
            csv.lines()
                .next()
                .is_some_and(|h| h.starts_with("Muestra,Area,Total return count"))
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_copc_file_is_read_only_where_the_regions_are() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_clip_copc_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let input = dir.join("nube.copc.laz");
        let pts = crate::copc::tests::build(&input)?;
        let window = [10.0, 10.0, 30.0, 30.0];
        let region = Region {
            name: "ventana".into(),
            shape: Shape2::Rect(window),
            attrs: Vec::new(),
        };
        let job = ClipJob::new(vec![input], vec![region], dir.join("out.las"));
        let r = run(&job)?;
        let expected = pts
            .iter()
            .filter(|p| p[0] >= 10.0 && p[0] <= 30.0 && p[1] >= 10.0 && p[1] <= 30.0)
            .count() as u64;
        assert_eq!(r.points_written, expected);
        // no se leyó toda la nube
        assert!(r.points_read < 400, "{}", r.points_read);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn outside_and_inside_split_the_cloud_and_a_dtm_normalises() -> Result<()> {
        let dir = tmp("split")?;
        let input = cloud(&dir)?;
        let regions = vec![Region {
            name: "r".into(),
            shape: Shape2::Rect([1000.0, 2000.0, 1049.5, 2099.5]),
            attrs: vec![],
        }];
        let mut inside = ClipJob::new(vec![input.clone()], regions.clone(), dir.join("in.las"));
        let a = run(&inside)?;
        let mut outside = ClipJob::new(vec![input.clone()], regions, dir.join("out.las"));
        outside.mode = ClipMode::Outside;
        let b = run(&outside)?;
        assert_eq!(a.points_written + b.points_written, 10_000);
        // Alturas sobre un suelo llano a 100 m: los árboles son de 10 m, el suelo de 0.
        let mut g = Grid::new(990.0, 1990.0, 10.0, 12, 12)?;
        g.data.fill(100.0);
        g.write_tiff(&dir.join("dtm.tif"), None)?;
        inside.dtm = Some(dir.join("dtm.tif"));
        inside.filter.zmin = Some(5.0);
        inside.output = dir.join("trees.las");
        let c = run(&inside)?;
        assert!(c.points_written > 0 && c.points_written < a.points_written / 10);
        let back = Reader::open(&dir.join("trees.las"))?;
        assert!(
            (back.header.bounds[2] - 10.0).abs() < 0.02
                && (back.header.bounds[5] - 10.0).abs() < 0.02
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn three_polygons_give_three_files_named_by_their_field_and_a_buffer_widens_each() -> Result<()> {
        let dir = tmp("buffer")?;
        let input = cloud(&dir)?;
        let poly = |name: &str, x: f64| Region {
            name: name.into(),
            shape: Shape2::Polygon(crate::vector::Polygon::new(vec![vec![
                [x, 2020.0],
                [x, 2040.0],
                [x + 20.0, 2040.0],
                [x + 20.0, 2020.0],
            ]])),
            attrs: vec![],
        };
        let regions = vec![poly("norte", 1000.0), poly("centro", 1030.0), poly("sur", 1060.0)];
        let mut job = ClipJob::new(vec![input.clone()], regions.clone(), dir.join("p.las"));
        job.mode = ClipMode::PerRegion;
        let plain = run(&job)?;
        assert_eq!(plain.files.len(), 3);
        for n in ["norte", "centro", "sur"] {
            assert!(plain.files.iter().any(|f| f.path.to_string_lossy().contains(n)), "{n}");
        }
        let wide: Vec<Region> = regions.iter().map(|r| r.buffered(10.0)).collect();
        let mut job2 = ClipJob::new(vec![input.clone()], wide, dir.join("w.las"));
        job2.mode = ClipMode::PerRegion;
        let grown = run(&job2)?;
        // los límites de cada salida alcanzan 10 m (a la resolución de la nube) más allá del
        // polígono
        for (a, b) in plain.files.iter().zip(&grown.files) {
            assert!(b.points > a.points);
        }
        let ba = Reader::open(&grown.files[1].path)?.header.bounds;
        assert!(ba[0] <= 1030.0 - 9.0 && ba[0] >= 1030.0 - 10.0 - 1e-6, "{ba:?}");
        assert!(ba[4] >= 2040.0 + 9.0 && ba[4] <= 2040.0 + 10.0 + 1e-6, "{ba:?}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn overlapping_regions_share_points_unless_exclusive() -> Result<()> {
        let dir = tmp("overlap")?;
        let input = cloud(&dir)?;
        let regions = vec![
            circle("A", 1050.0, 2050.0, 20.0),
            circle("B", 1055.0, 2050.0, 20.0),
        ];
        let mut job = ClipJob::new(vec![input], regions, dir.join("p.las"));
        job.mode = ClipMode::PerRegion;
        let shared = run(&job)?;
        job.exclusive = true;
        let first_wins = run(&job)?;
        assert!(shared.files[1].points > first_wins.files[1].points);
        assert_eq!(first_wins.files[0].points, shared.files[0].points);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn region_names_are_safe_and_unique_in_file_names() {
        let mut used = Vec::new();
        let base = Path::new("out/plot.laz");
        assert!(region_path(base, "P 1/a", &mut used).ends_with("plot_P_1_a.laz"));
        assert!(region_path(base, "P 1/a", &mut used).ends_with("plot_P_1_a_2.laz"));
    }
}
