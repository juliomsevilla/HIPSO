//! Flight lines of an airborne cloud, found from the gaps in the GPS time.
//!
//! An aircraft flies a line, turns for tens of seconds without recording, and flies the next one.
//! Sorting the times of all the points and cutting where the gap is longer than a limit (30 s by
//! default) leaves one stretch of time per line. Memory is one number per tenth of a second the
//! aircraft recorded, not per point: the cloud is read twice, once to find the stretches and
//! once to say which one each point belongs to.
//!
//! Standard GPS time (the `global_encoding` bit 0 clear) is the time of the week and starts again
//! on Sunday at midnight: a flight over that moment shows a jump back, which is reported and not
//! mended. Formats 0 and 2 have no time at all.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::las::{self, Reader, Writer};
use crate::{Error, Result};

/// Where a flight line is cut and what is too small to be one.
#[derive(Clone, Debug, PartialEq)]
pub struct FlightParams {
    /// A silence longer than this (s) separates two lines.
    pub gap: f64,
    /// Stretches with fewer points are not lines.
    pub min_points: u64,
    /// Stretches shorter than this (s) are not lines.
    pub min_duration: f64,
    /// The time is counted in bins of this size (s).
    pub bin: f64,
}

impl Default for FlightParams {
    fn default() -> Self {
        Self {
            gap: 30.0,
            min_points: 1000,
            min_duration: 0.0,
            bin: 0.1,
        }
    }
}

/// One flight line.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// From 1, in order of time.
    pub id: u32,
    pub t0: f64,
    pub t1: f64,
    pub points: u64,
    /// min x, min y, max x, max y.
    pub bounds: [f64; 4],
    /// Mean scan angle (degrees, signed).
    pub mean_scan: f32,
    /// The outline of the points of the line (their convex hull, from a sample of one point in
    /// every 5 m square), closed by the first vertex not being repeated.
    pub hull: Vec<[f64; 2]>,
}

/// Which stretch of time each moment belongs to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeIndex {
    bin: f64,
    /// First bin of each stretch, ascending.
    starts: Vec<i64>,
    /// Line of each stretch; 0 for the ones dropped.
    ids: Vec<u32>,
}

impl TimeIndex {
    /// The line a point at time `t` belongs to (0: none).
    pub fn line_at(&self, t: f64) -> u32 {
        if !t.is_finite() || t == 0.0 || self.starts.is_empty() {
            return 0;
        }
        let b = (t / self.bin).floor() as i64;
        match self.starts.partition_point(|s| *s <= b) {
            0 => 0,
            k => self.ids[k - 1],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlightReport {
    pub lines: Vec<Line>,
    /// Points in stretches that were too small to be a line.
    pub discarded: u64,
    /// Points with a time of 0 (no GPS time), which belong to no line.
    pub no_time: u64,
    pub points: u64,
    /// The time jumps back by more than 100 000 s somewhere: the week time started again.
    pub week_reset: bool,
    pub index: TimeIndex,
    pub epsg: Option<u16>,
}

fn check_time(r: &Reader, path: &Path) -> Result<()> {
    if !matches!(r.header.format, 1 | 3..=10) {
        return Err(Error::Param(format!(
            "{}: el archivo no tiene tiempo GPS (formato de punto {})",
            path.display(),
            r.header.format
        )));
    }
    Ok(())
}

/// Finds the flight lines of the clouds (their times count together).
pub fn detect(inputs: &[PathBuf], p: &FlightParams) -> Result<FlightReport> {
    if !(p.gap > 0.0) || !(p.bin > 0.0) || p.bin > p.gap {
        return Err(Error::Param(
            "el hueco entre líneas debe ser positivo y mayor que el tamaño del intervalo de tiempo"
                .into(),
        ));
    }
    if inputs.is_empty() {
        return Err(Error::Param("no hay nubes de entrada".into()));
    }
    // pass 1: the bins of time that have points
    let mut bins: HashSet<i64> = HashSet::new();
    let mut rep = FlightReport::default();
    let readers: Vec<Reader> = inputs
        .iter()
        .map(|i| Reader::open(i))
        .collect::<Result<_>>()?;
    for (r, path) in readers.iter().zip(inputs) {
        check_time(r, path)?;
        let rec_len = usize::from(r.header.record_len);
        let mut last = f64::NAN;
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                rep.points += 1;
                let t = r.decode_extra(rec).gps_time;
                if !t.is_finite() || t == 0.0 {
                    rep.no_time += 1;
                    continue;
                }
                if last.is_finite() && last - t > 100_000.0 {
                    rep.week_reset = true;
                }
                last = t;
                bins.insert((t / p.bin).floor() as i64);
            }
            crate::check_cancel()
        })?;
    }
    let mut sorted: Vec<i64> = bins.into_iter().collect();
    sorted.sort_unstable();
    // the stretches: a silence longer than the gap cuts
    let mut starts: Vec<i64> = Vec::new();
    for (k, b) in sorted.iter().enumerate() {
        if k == 0 || (b - sorted[k - 1] - 1) as f64 * p.bin > p.gap {
            starts.push(*b);
        }
    }
    // pass 2: what each stretch holds
    struct Acc {
        /// One point in each 5 m square, for the outline.
        sample: std::collections::HashMap<(i64, i64), [f64; 2]>,
        points: u64,
        bounds: [f64; 4],
        scan: f64,
        t0: f64,
        t1: f64,
    }
    let mut acc: Vec<Acc> = starts
        .iter()
        .map(|_| Acc {
            sample: std::collections::HashMap::new(),
            points: 0,
            bounds: [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            scan: 0.0,
            t0: f64::INFINITY,
            t1: f64::NEG_INFINITY,
        })
        .collect();
    for r in &readers {
        let rec_len = usize::from(r.header.record_len);
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                let e = r.decode_extra(rec);
                if !e.gps_time.is_finite() || e.gps_time == 0.0 {
                    continue;
                }
                let b = (e.gps_time / p.bin).floor() as i64;
                let Some(k) = starts.partition_point(|s| *s <= b).checked_sub(1) else {
                    continue;
                };
                let a = &mut acc[k];
                a.points += 1;
                a.scan += f64::from(e.scan_angle);
                a.t0 = a.t0.min(e.gps_time);
                a.t1 = a.t1.max(e.gps_time);
                if let Some(q) = r.decode(rec) {
                    a.sample
                        .entry(((q.x / 5.0).floor() as i64, (q.y / 5.0).floor() as i64))
                        .or_insert([q.x, q.y]);
                    a.bounds = [
                        a.bounds[0].min(q.x),
                        a.bounds[1].min(q.y),
                        a.bounds[2].max(q.x),
                        a.bounds[3].max(q.y),
                    ];
                }
            }
            crate::check_cancel()
        })?;
    }
    // keep what is big enough, numbered by time
    let mut ids = vec![0u32; starts.len()];
    for (k, a) in acc.iter().enumerate() {
        if a.points >= p.min_points && a.t1 - a.t0 >= p.min_duration {
            let id = rep.lines.len() as u32 + 1;
            ids[k] = id;
            rep.lines.push(Line {
                id,
                t0: a.t0,
                t1: a.t1,
                points: a.points,
                bounds: a.bounds,
                mean_scan: (a.scan / a.points as f64) as f32,
                hull: hull_of(a.sample.values().copied()),
            });
        } else {
            rep.discarded += a.points;
        }
    }
    rep.epsg = readers.first().and_then(|r| r.header.epsg);
    rep.index = TimeIndex {
        bin: p.bin,
        starts,
        ids,
    };
    Ok(rep)
}

/// The convex hull of some points, without the closing vertex.
fn hull_of(pts: impl Iterator<Item = [f64; 2]>) -> Vec<[f64; 2]> {
    use geo::ConvexHull;
    let mp: geo::MultiPoint<f64> = pts.map(|p| geo::Point::new(p[0], p[1])).collect();
    let mut ring: Vec<[f64; 2]> = mp.convex_hull().exterior().0.iter().map(|c| [c.x, c.y]).collect();
    ring.pop();
    ring
}

/// Where the line number is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    None,
    /// The point source ID (up to 65 535 lines).
    SourceId,
    /// The user data byte (up to 255 lines).
    UserData,
}

/// Writes copies of the clouds into `out_dir` (`<name>_lineas.las`/`.laz`) with the number of
/// the flight line of each point (0 where it has none) in the field `into`.
pub fn assign(
    inputs: &[PathBuf],
    out_dir: &Path,
    rep: &FlightReport,
    into: Write,
) -> Result<Vec<PathBuf>> {
    let cap = match into {
        Write::None => return Ok(Vec::new()),
        Write::SourceId => usize::from(u16::MAX),
        Write::UserData => usize::from(u8::MAX),
    };
    if rep.lines.len() > cap {
        return Err(Error::Param(format!(
            "hay {} líneas y {} solo admite hasta {cap}: usa el otro campo",
            rep.lines.len(),
            if into == Write::UserData {
                "el dato de usuario"
            } else {
                "el identificador de pasada"
            }
        )));
    }
    std::fs::create_dir_all(out_dir)?;
    let mut out = Vec::new();
    for input in inputs {
        let r = Reader::open(input)?;
        check_time(&r, input)?;
        let ext = input.extension().map_or("las".to_string(), |e| {
            e.to_string_lossy().to_ascii_lowercase()
        });
        let name = crate::paths::stem(input);
        let path = out_dir.join(format!("{name}_lineas.{ext}"));
        if path == *input {
            return Err(Error::Param("la copia no puede ser el original".into()));
        }
        let mut w = Writer::create(&path, &r)?;
        let (format, rec_len) = (r.header.format, usize::from(r.header.record_len));
        let mut buf = vec![0u8; rec_len];
        r.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                buf.copy_from_slice(rec);
                let id = rep.index.line_at(r.decode_extra(rec).gps_time);
                match into {
                    Write::SourceId => las::set_source_id(format, &mut buf, id as u16),
                    Write::UserData => las::set_user_data(&mut buf, id as u8),
                    Write::None => {}
                }
                w.write(&buf)?;
            }
            crate::check_cancel()
        })?;
        w.finish()?;
        out.push(path);
    }
    Ok(out)
}

/// The lines as a CSV table.
pub fn csv_text(rep: &FlightReport) -> String {
    let mut s = String::from(
        "Linea,T_inicio,T_fin,Duracion_s,Puntos,X_min,Y_min,X_max,Y_max,Angulo_medio\n",
    );
    for l in &rep.lines {
        s.push_str(&format!(
            "{},{:.3},{:.3},{:.1},{},{:.3},{:.3},{:.3},{:.3},{:.2}\n",
            l.id,
            l.t0,
            l.t1,
            l.t1 - l.t0,
            l.points,
            l.bounds[0],
            l.bounds[1],
            l.bounds[2],
            l.bounds[3],
            l.mean_scan
        ));
    }
    s
}

/// The lines as a layer of polygons: the outline of each one with its figures.
pub fn layer(rep: &FlightReport) -> crate::view::VectorData {
    use crate::view::{Feature, VectorData};
    let mut d = VectorData {
        polygons: true,
        fields: ["linea", "t_inicio", "t_fin", "duracion", "puntos", "angulo"].map(String::from).to_vec(),
        bounds: [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
        ..Default::default()
    };
    for l in rep.lines.iter().filter(|l| l.hull.len() >= 3) {
        for q in &l.hull {
            d.bounds = [d.bounds[0].min(q[0]), d.bounds[1].min(q[1]), d.bounds[2].max(q[0]), d.bounds[3].max(q[1])];
        }
        let values = vec![
            l.id.to_string(),
            format!("{:.3}", l.t0),
            format!("{:.3}", l.t1),
            format!("{:.1}", l.t1 - l.t0),
            l.points.to_string(),
            format!("{:.2}", l.mean_scan),
        ];
        d.num.push(values.iter().map(|t| t.parse().unwrap_or(f64::NAN)).collect());
        d.text.push(values);
        d.features.push(Feature { parts: vec![l.hull.clone()] });
    }
    d
}

/// What the user asked of the flight lines.
pub struct FlightJob {
    pub inputs: Vec<PathBuf>,
    pub params: FlightParams,
    pub into: Write,
    /// Where the table and the copies go.
    pub output_dir: PathBuf,
    pub csv: bool,
    /// A polygon layer with the outline of each line.
    pub vector: bool,
}

/// Detects the lines, writes the table and the copies if asked, and reports in text.
pub fn run(job: &FlightJob) -> Result<String> {
    let rep = detect(&job.inputs, &job.params)?;
    let mut lines = vec![format!(
        "{} líneas de vuelo en {} puntos (hueco de {} s)",
        rep.lines.len(),
        rep.points,
        job.params.gap
    )];
    for l in &rep.lines {
        lines.push(format!(
            "  línea {}: {:.1} s ({:.1} a {:.1}), {} puntos, ángulo medio {:.1}°",
            l.id,
            l.t1 - l.t0,
            l.t0,
            l.t1,
            l.points,
            l.mean_scan
        ));
    }
    if rep.discarded > 0 {
        lines.push(format!(
            "  {} puntos en tramos demasiado pequeños para ser una línea",
            rep.discarded
        ));
    }
    if rep.no_time > 0 {
        lines.push(format!(
            "  {} puntos sin tiempo GPS (valor 0): no pertenecen a ninguna línea",
            rep.no_time
        ));
    }
    if rep.week_reset {
        lines.push("  AVISO: el tiempo salta hacia atrás más de 100 000 s: es tiempo de semana y el vuelo cruza el domingo a medianoche; las líneas de después pueden estar mal cortadas. No se corrige".into());
    }
    if job.csv {
        std::fs::create_dir_all(&job.output_dir)?;
        let name = job
            .inputs
            .first()
            .map_or("nube".to_string(), |p| crate::paths::stem(p));
        let p = job.output_dir.join(format!("{name}_lineas_de_vuelo.csv"));
        std::fs::write(&p, csv_text(&rep))?;
        lines.push(format!("  tabla: {}", p.display()));
    }
    if job.vector {
        std::fs::create_dir_all(&job.output_dir)?;
        let name = job.inputs.first().map_or("nube".to_string(), |p| crate::paths::stem(p));
        let p = job.output_dir.join(format!("{name}_lineas_de_vuelo.shp"));
        let prj = rep.epsg.and_then(crate::vector::prj_for_epsg);
        crate::vector::write_layer(&p, &layer(&rep), crate::view::GeomKind::Polygon, prj.as_deref())?;
        lines.push(format!("  contornos de las líneas: {}", p.display()));
    }
    for p in assign(&job.inputs, &job.output_dir, &rep, job.into)? {
        lines.push(format!("  copia con la línea de vuelo: {}", p.display()));
    }
    Ok(lines.join("\n"))
}
