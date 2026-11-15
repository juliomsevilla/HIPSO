//! Pasadas de una nube aérea, halladas a partir de los saltos del tiempo GPS.
//!
//! Un avión vuela una línea, gira durante decenas de segundos sin grabar y vuela la siguiente.
//! Ordenar los tiempos de todos los puntos y cortar donde el salto es mayor que un límite (30 s por
//! defecto) deja un tramo de tiempo por línea. La memoria es un número por cada décima de segundo
//! que grabó el avión, no por punto: la nube se lee dos veces, una para hallar los tramos y otra
//! para decir a cuál pertenece cada punto.
//!
//! El tiempo GPS estándar (el bit 0 de `global_encoding` sin activar) es el tiempo de la semana y
//! vuelve a empezar el domingo a medianoche: un vuelo por encima de ese momento muestra un salto
//! atrás, que se comunica y no se arregla. Los formatos 0 y 2 no tienen tiempo alguno.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::las::{self, Reader, Writer};
use crate::{Error, Result};

/// Dónde se corta una pasada y qué es demasiado pequeño para serlo.
#[derive(Clone, Debug, PartialEq)]
pub struct FlightParams {
    /// Un silencio mayor que esto (s) separa dos líneas.
    pub gap: f64,
    /// Los tramos con menos puntos no son líneas.
    pub min_points: u64,
    /// Los tramos más cortos que esto (s) no son líneas.
    pub min_duration: f64,
    /// El tiempo se cuenta en intervalos de este tamaño (s).
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

/// Una pasada.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Desde 1, en orden de tiempo.
    pub id: u32,
    pub t0: f64,
    pub t1: f64,
    pub points: u64,
    /// min x, min y, max x, max y.
    pub bounds: [f64; 4],
    /// Mean scan angle (degrees, signed).
    pub mean_scan: f32,
    /// El contorno de los puntos de la línea (su envolvente convexa, a partir de una muestra de un
    /// punto por cada cuadrado de 5 m), cerrado por no repetirse el primer vértice.
    pub hull: Vec<[f64; 2]>,
}

/// A qué tramo de tiempo pertenece cada momento.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeIndex {
    bin: f64,
    /// Primer intervalo de cada tramo, ascendente.
    starts: Vec<i64>,
    /// Línea de cada tramo; 0 para los descartados.
    ids: Vec<u32>,
}

impl TimeIndex {
    /// La línea a la que pertenece un punto en el tiempo `t` (0: ninguna).
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
    /// Puntos en tramos demasiado pequeños para ser una línea.
    pub discarded: u64,
    /// Puntos con un tiempo de 0 (sin tiempo GPS), que no pertenecen a ninguna línea.
    pub no_time: u64,
    pub points: u64,
    /// El tiempo salta hacia atrás más de 100 000 s en algún sitio: el tiempo de semana volvió a
    /// empezar.
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

/// Halla las pasadas de las nubes (sus tiempos cuentan juntos).
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
    // pasada 1: los intervalos de tiempo que tienen puntos
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
    // los tramos: un silencio mayor que el salto corta
    let mut starts: Vec<i64> = Vec::new();
    for (k, b) in sorted.iter().enumerate() {
        if k == 0 || (b - sorted[k - 1] - 1) as f64 * p.bin > p.gap {
            starts.push(*b);
        }
    }
    // pasada 2: lo que contiene cada tramo
    struct Acc {
        /// Un punto por cada cuadrado de 5 m, para el contorno.
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
    // se conserva lo bastante grande, numerado por tiempo
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

/// La envolvente convexa de unos puntos, sin el vértice de cierre.
fn hull_of(pts: impl Iterator<Item = [f64; 2]>) -> Vec<[f64; 2]> {
    use geo::ConvexHull;
    let mp: geo::MultiPoint<f64> = pts.map(|p| geo::Point::new(p[0], p[1])).collect();
    let mut ring: Vec<[f64; 2]> = mp.convex_hull().exterior().0.iter().map(|c| [c.x, c.y]).collect();
    ring.pop();
    ring
}

/// Dónde se escribe el número de línea.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    None,
    /// El identificador de fuente del punto (hasta 65 535 líneas).
    SourceId,
    /// El byte de datos de usuario (hasta 255 líneas).
    UserData,
}

/// Escribe copias de las nubes en `out_dir` (`<nombre>_lineas.las`/`.laz`) con el número de la
/// pasada de cada punto (0 donde no tiene) en el campo `into`.
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

/// Las líneas como tabla CSV.
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

/// Las líneas como capa de polígonos: el contorno de cada una con sus cifras.
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

/// Lo que pidió el usuario de las pasadas.
pub struct FlightJob {
    pub inputs: Vec<PathBuf>,
    pub params: FlightParams,
    pub into: Write,
    /// Adónde van la tabla y las copias.
    pub output_dir: PathBuf,
    pub csv: bool,
    /// Una capa de polígonos con el contorno de cada línea.
    pub vector: bool,
}

/// Detecta las líneas, escribe la tabla y las copias si se piden e informa en texto.
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
