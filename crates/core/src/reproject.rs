//! Lleva nubes, rásteres y regiones de un sistema de coordenadas a otro, con las transformaciones
//! de `hipso-crs` (las rejillas de ajuste de datum de España están incorporadas).
//!
//! Las alturas cambian solo cuando se pide una referencia vertical (véase [`Vertical`]): entonces
//! pasan por el modelo de geoide del par, como lo elige la transformación de `hipso-crs`. Sin ella
//! se conservan tal como están. Las aproximaciones (sin ajuste de datum, sin geoide para el punto,
//! una operación inferior porque falta una rejilla) se rechazan por defecto; véase [`Policy`].

use std::path::{Path, PathBuf};

use hipso_crs::SpatialRef;
pub use hipso_crs::Transformer;
pub use hipso_crs::{GridOptions, set_grid_options};
pub use hipso_crs::policy::{Approximation, Policy, set_policy_overrides};
use rayon::prelude::*;

use crate::grid::Grid;
use crate::las::{self, Reader, Writer};
use crate::vector::{Polygon, Region, Shape2};
use crate::{Error, Result};

/// El transformador entre dos códigos EPSG.
pub fn transformer(from: u16, to: u16) -> Result<Transformer> {
    let parse = |c: u16| {
        SpatialRef::parse(&format!("EPSG:{c}"))
            .map_err(|e| Error::Param(format!("el sistema EPSG:{c} no se reconoce: {e}")))
    };
    Transformer::new(&parse(from)?, &parse(to)?)
        .map_err(|e| Error::Param(format!("no se puede pasar de EPSG:{from} a EPSG:{to}: {e}")))
}

/// Una referencia vertical de alturas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalRef {
    /// Altura sobre el elipsoide del datum horizontal (lo que da el GNSS).
    Ellipsoidal,
    /// Un sistema vertical por código EPSG (5782: alturas ortométricas REDNAP; 3855: altura
    /// EGM2008).
    Epsg(u16),
}

impl VerticalRef {
    /// `ellipsoidal` (o `ell`) o un código EPSG (`5782`, `EPSG:5782`).
    pub fn parse(s: &str) -> Result<Self> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("ellipsoidal") || t.eq_ignore_ascii_case("ell") {
            return Ok(Self::Ellipsoidal);
        }
        t.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == ':')
            .parse::<u16>()
            .map(Self::Epsg)
            .map_err(|_| {
                Error::Param(format!(
                    "referencia vertical «{s}» no válida: usa «ellipsoidal» o un código EPSG"
                ))
            })
    }
}

/// Las referencias verticales de un cambio de alturas: aquella en la que están los datos y aquella
/// que se les da. Con `output` sin fijar, las alturas se dejan como están.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Vertical {
    pub input: Option<VerticalRef>,
    pub output: Option<VerticalRef>,
}

/// Un transformador con alturas: los códigos horizontales más los verticales cuando se dan.
pub fn transformer_3d(from: u16, to: u16, v: Vertical) -> Result<Transformer> {
    let parse = |h: u16, v: Option<VerticalRef>| {
        let spec = spec_of(h, v);
        SpatialRef::parse(&spec)
            .map_err(|e| Error::Param(format!("el sistema {spec} no se reconoce: {e}")))
    };
    let (a, b) = (parse(from, v.input)?, parse(to, v.output)?);
    let opts = hipso_crs::TransformOptions {
        source_ellipsoidal_height: v.input == Some(VerticalRef::Ellipsoidal),
        target_ellipsoidal_height: v.output == Some(VerticalRef::Ellipsoidal),
        ..Default::default()
    };
    Transformer::with_options(&a, &b, &opts).map_err(|e| {
        Error::Param(format!(
            "no se puede pasar de {} a {}: {e}",
            spec_of(from, v.input),
            spec_of(to, v.output)
        ))
    })
}

fn spec_of(h: u16, v: Option<VerticalRef>) -> String {
    match v {
        Some(VerticalRef::Epsg(v)) => format!("EPSG:{h}+{v}"),
        _ => format!("EPSG:{h}"),
    }
}

/// Lo que hizo una reproyección de una nube.
#[derive(Debug, Clone, Default)]
pub struct CloudReport {
    pub read: u64,
    /// Puntos fuera del dominio de la proyección de destino.
    pub dropped: u64,
    /// Cuántos puntos pasaron por cada aproximación que aceptó la política.
    pub approximated: Vec<(Approximation, u64)>,
    /// Frases para el informe.
    pub notes: Vec<String>,
}

fn geographic(code: u16) -> bool {
    (4000..5000).contains(&code)
}

/// Escribe `src` en el sistema `to`, con el orden, los atributos y los registros de los puntos sin
/// cambios. Los puntos que no se pueden transformar se descartan; se devuelve cuántos se leyeron y
/// cuántos se descartaron.
pub fn cloud(src: &Path, dst: &Path, from: u16, to: u16) -> Result<(u64, u64)> {
    let r = cloud_with(src, dst, from, to, Vertical::default(), Policy::from_env())?;
    Ok((r.read, r.dropped))
}

/// [`cloud`] con un cambio de alturas y una política. Un punto que la política rechaza detiene toda
/// la operación con un error (un rechazo nunca es un punto descartado en silencio).
pub fn cloud_with(
    src: &Path,
    dst: &Path,
    from: u16,
    to: u16,
    vertical: Vertical,
    policy: Policy,
) -> Result<CloudReport> {
    if geographic(from) || geographic(to) {
        return Err(Error::Unsupported(
            "las nubes no se reproyectan desde o hacia coordenadas geográficas (grados): usa un sistema proyectado".into(),
        ));
    }
    let r = Reader::open(src)?;
    let mut vertical = vertical;
    let mut notes = Vec::new();
    if vertical.input.is_none() {
        vertical.input = r.header.vertical_epsg.map(VerticalRef::Epsg);
    }
    if vertical.output.is_some() && vertical.input.is_none() {
        return Err(Error::Param(
            "la nube no declara su referencia vertical: indica la de origen (--vertical-in) para cambiar las alturas".into(),
        ));
    }
    if vertical.output.is_none() {
        if let Some(v) = vertical.input {
            notes.push(format!("alturas sin cambios (referencia vertical {v:?}); indica la de destino para cambiarlas"));
        } else {
            notes.push("la nube no declara referencia vertical: las alturas no se tocan".into());
        }
        // las alturas se quedan: ninguna referencia vertical en ningún lado
        vertical.input = None;
    }
    let t = transformer_3d(from, to, vertical)?;
    let flat = transformer(from, to)?;
    let b = r.header.bounds;
    let (x0, y0, x1, y1) = flat
        .transform_bounds((b[0], b[1], b[3], b[4]))
        .map_err(|e| Error::Param(format!("la nube cae fuera del sistema EPSG:{to}: {e}")))?;
    // guardado relativo a un valor redondo cerca del medio, pues las coordenadas nuevas están lejos
    // de las antiguas
    let round = |v: f64| (v / 1000.0).round() * 1000.0;
    let offset = [
        round((x0 + x1) / 2.0),
        round((y0 + y1) / 2.0),
        r.header.offset[2],
    ];
    // Los formatos de punto LAS 1.4 del 6 en adelante llevan el sistema como WKT, que contiene el
    // sistema compuesto; los formatos más antiguos declaran la referencia vertical en las claves
    // GeoTIFF
    let vertical_code = match vertical.output {
        Some(VerticalRef::Epsg(v)) => Some(v),
        _ => None,
    };
    let compound = vertical_code
        .filter(|_| r.header.format >= 6)
        .and_then(|v| las::compound_wkt(to, v));
    let out = Reader::open(src)?;
    let out = match compound {
        Some(wkt) => {
            // bit 4 de la codificación global: el sistema de coordenadas es WKT
            out.with_projection(&las::wkt_record(&wkt), 1, Some(to), Some(wkt))
                .with_global_encoding_bits(0x10)
        }
        None => out.with_projection(&las::epsg_record_with_vertical(to, vertical_code), 1, Some(to), None),
    }
    .with_offset(offset);
    let mut out = out;
    out.header.vertical_epsg = match vertical.output {
        Some(VerticalRef::Epsg(v)) => Some(v),
        _ => None,
    };
    let (hin, hout) = (r.header.clone(), out.header.clone());
    let rec_len = usize::from(hin.record_len);
    let mut w = Writer::create(dst, &out)?;
    let dropped = std::sync::atomic::AtomicU64::new(0);
    let counts: [std::sync::atomic::AtomicU64; 3] = Default::default();
    let refused: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let read = r.scan_raw(|bytes| {
        let mut buf = bytes.to_vec();
        let keep: Vec<bool> = buf
            .par_chunks_mut(rec_len)
            .map(|rec| {
                let at = |k: usize| {
                    rec.get(4 * k..4 * k + 4)
                        .and_then(|s| s.try_into().ok())
                        .map(i32::from_le_bytes)
                };
                let (Some(xi), Some(yi), Some(zi)) = (at(0), at(1), at(2)) else {
                    return false;
                };
                let x = f64::from(xi) * hin.scale[0] + hin.offset[0];
                let y = f64::from(yi) * hin.scale[1] + hin.offset[1];
                let z = f64::from(zi) * hin.scale[2] + hin.offset[2];
                match t.transform_checked(x, y, z, policy) {
                    Ok(((nx, ny, nz), approx)) => {
                        for (k, a) in Approximation::ALL.iter().enumerate() {
                            if approx.contains(*a) {
                                counts[k].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        las::set_xyz(&hout, rec, nx, ny, nz)
                    }
                    Err(hipso_crs::Error::Policy(m)) => {
                        if let Ok(mut r) = refused.lock() {
                            r.get_or_insert(m);
                        }
                        false
                    }
                    Err(_) => false,
                }
            })
            .collect();
        for (rec, ok) in buf.chunks_exact(rec_len).zip(&keep) {
            if *ok {
                w.write(rec)?;
            } else {
                dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        Ok(())
    })?;
    if let Some(m) = refused.into_inner().ok().flatten() {
        drop(w);
        let _ = std::fs::remove_file(dst);
        return Err(Error::Param(format!("transformación rechazada: {m}")));
    }
    w.finish()?;
    let approximated: Vec<(Approximation, u64)> = Approximation::ALL
        .iter()
        .zip(&counts)
        .map(|(a, c)| (*a, c.load(std::sync::atomic::Ordering::Relaxed)))
        .filter(|(_, n)| *n > 0)
        .collect();
    for (a, n) in &approximated {
        notes.push(format!("{n} puntos: {}", a.describe()));
    }
    Ok(CloudReport {
        read,
        dropped: dropped.into_inner(),
        approximated,
        notes,
    })
}

/// Las nubes de un trabajo, todas en el sistema de la primera que lo declara.
#[derive(Debug, Default)]
pub struct ProjectClouds {
    /// Los archivos que cargar: los originales o sus copias convertidas, en orden y sin repetidos.
    pub files: Vec<PathBuf>,
    /// Lo que ocurrió, para el registro.
    pub notes: Vec<String>,
    /// Cuántos archivos hubo que convertir (o se encontraron ya convertidos).
    pub moved: usize,
}

/// El sistema en que está el trabajo y cuántos de `paths` están en otro (para que quien llama pueda
/// decir que la carga tardará más antes de empezar). Las nubes geográficas también cuentan.
pub fn clouds_in_other_system(paths: &[PathBuf]) -> (Option<u16>, usize) {
    let epsgs: Vec<Option<u16>> = paths.iter().map(|p| crate::crs::epsg_of(p)).collect();
    let target = epsgs.iter().flatten().next().copied();
    let n = match target {
        Some(t) => epsgs
            .iter()
            .flatten()
            .filter(|e| crate::crs::relation(t, **e) == crate::crs::Relation::Different)
            .count(),
        None => 0,
    };
    (target, n)
}

/// Pone las nubes en el sistema de la primera que lo declara. Las que están en otro sistema se
/// convierten en una copia LAZ junto a ellas (`<nombre>_epsg<código>.laz`, en la carpeta de
/// resultados), que las cargas posteriores encuentran lista mientras sea más reciente que el
/// original. Las nubes que no se pueden convertir se dejan fuera, y las notas dicen por qué.
pub fn project_clouds(paths: &[PathBuf]) -> ProjectClouds {
    let mut out = ProjectClouds::default();
    let (target, _) = clouds_in_other_system(paths);
    for p in paths {
        let name = p.file_name().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let from = crate::crs::epsg_of(p);
        let file = match (target, from) {
            (Some(t), Some(e)) if crate::crs::relation(t, e) == crate::crs::Relation::Different => {
                let copy = crate::paths::product(p, &format!("epsg{t}"), "laz");
                let newer = |a: &Path, b: &Path| match (
                    a.metadata().and_then(|m| m.modified()),
                    b.metadata().and_then(|m| m.modified()),
                ) {
                    (Ok(x), Ok(y)) => x >= y,
                    _ => false,
                };
                if newer(&copy, p) && crate::crs::epsg_of(&copy) == Some(t) {
                    out.notes.push(format!(
                        "«{name}» estaba en {} y ya tenía una copia en {}: se usa {}",
                        crate::crs::name(e),
                        crate::crs::name(t),
                        copy.display()
                    ));
                    out.moved += 1;
                    copy
                } else {
                    let started = std::time::Instant::now();
                    let part = copy.with_extension("part.laz");
                    let done = crate::paths::ensure_parent(&copy)
                        .map_err(Error::from)
                        .and_then(|()| cloud(p, &part, e, t))
                        .and_then(|r| {
                            std::fs::rename(&part, &copy)
                                .map(|()| r)
                                .map_err(Error::from)
                        });
                    match done {
                        Ok((read, dropped)) => {
                            let lost = if dropped > 0 {
                                format!(
                                    "; {dropped} puntos fuera del sistema de destino, descartados"
                                )
                            } else {
                                String::new()
                            };
                            out.notes.push(format!(
                                "«{name}» estaba en {} y se ha pasado a {} ({read} puntos en {:.1} s{lost}): {}",
                                crate::crs::name(e),
                                crate::crs::name(t),
                                started.elapsed().as_secs_f64(),
                                copy.display()
                            ));
                            out.moved += 1;
                            copy
                        }
                        Err(err) => {
                            let _ = std::fs::remove_file(&part);
                            out.notes.push(format!("«{name}» no se carga: {err}"));
                            continue;
                        }
                    }
                }
            }
            (Some(t), None) => {
                out.notes.push(format!(
                    "«{name}» no declara su sistema de coordenadas: se carga como si estuviera en {}",
                    crate::crs::name(t)
                ));
                p.clone()
            }
            _ => p.clone(),
        };
        if !out.files.contains(&file) {
            out.files.push(file);
        }
    }
    out
}

/// La rejilla en el sistema `to`, si está en otro (`from`): `None` cuando no hay nada que hacer.
pub fn grid_if_needed(g: &Grid, from: Option<u16>, to: Option<u16>) -> Result<Option<Grid>> {
    match (from, to) {
        (Some(f), Some(t)) if crate::crs::relation(t, f) == crate::crs::Relation::Different => {
            grid(g, f, t).map(Some)
        }
        _ => Ok(None),
    }
}

/// El sistema en el que probablemente está una nube que no declara ninguno, a partir de dónde cae
/// una capa de sistema conocido: `None` cuando la capa ya se solapa con la nube tal como está
/// (comparten sistema) o, si no, el primero de `candidates` en el que la capa sí se solapa. Los
/// límites son x mínima, y mínima, x máxima, y máxima.
pub fn guess_system(layer: [f64; 4], from: u16, target: [f64; 4], candidates: &[u16]) -> Option<u16> {
    let overlap = |a: [f64; 4], b: [f64; 4]| a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3];
    if !layer.iter().all(|v| v.is_finite()) || overlap(layer, target) {
        return None;
    }
    let pts = [
        (layer[0], layer[1]),
        (layer[2], layer[1]),
        (layer[2], layer[3]),
        (layer[0], layer[3]),
        ((layer[0] + layer[2]) / 2.0, (layer[1] + layer[3]) / 2.0),
    ];
    candidates.iter().copied().find(|&c| {
        let Ok(t) = transformer(from, c) else { return false };
        let moved: Vec<(f64, f64)> = pts.iter().filter_map(|p| t.transform(p.0, p.1).ok()).collect();
        if moved.len() < pts.len() {
            return false;
        }
        let b = moved.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
            [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)]
        });
        overlap(b, target)
    })
}

/// Lleva los vértices de una capa vectorial al sistema `to`; los límites se vuelven a calcular.
pub fn vector_data(d: &mut crate::view::VectorData, from: u16, to: u16) -> Result<()> {
    let t = transformer(from, to)?;
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for f in &mut d.features {
        for part in &mut f.parts {
            for v in part.iter_mut() {
                let (x, y) = t.transform(v[0], v[1]).map_err(|e| {
                    Error::Param(format!("un vértice cae fuera del sistema EPSG:{to}: {e}"))
                })?;
                *v = [x, y];
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    d.bounds = b;
    Ok(())
}

/// Un ráster que solo contiene números enteros (clases, etiquetas) no debe difuminarse con la
/// interpolación.
fn categorical(g: &Grid) -> bool {
    let step = (g.data.len() / 20_000).max(1);
    g.data
        .iter()
        .step_by(step)
        .all(|v| v.is_nan() || v.fract() == 0.0)
}

/// El ráster en el sistema `to`: el mismo tamaño de celda cuando las unidades coinciden (al 1 %:
/// entre zonas UTM la escala difiere en unas pocas partes por mil, y las celdas redondas valen
/// eso), las celdas alineadas en múltiplos de él, los valores interpolados (o tomados de la celda
/// más cercana, para rásteres de números enteros).
pub fn grid(g: &Grid, from: u16, to: u16) -> Result<Grid> {
    let (b, cell) = target_frame_of([g.x0, g.y0, g.cell], g.cols, g.rows, from, to)?;
    grid_onto(g, from, to, Grid::covering(b, cell)?)
}

/// La extensión `[x0, y0, x1, y1]` y la celda del ráster que [`grid`] hace a partir de uno cuya
/// esquina inferior izquierda y celda son `origin` y que tiene `cols` x `rows` celdas.
pub(crate) fn target_frame_of(
    origin: [f64; 3],
    cols: usize,
    rows: usize,
    from: u16,
    to: u16,
) -> Result<([f64; 4], f64)> {
    let [gx0, gy0, gcell] = origin;
    let fwd = transformer(from, to)?;
    let (w, h) = (cols as f64 * gcell, rows as f64 * gcell);
    let (x0, y0, x1, y1) = fwd
        .transform_bounds((gx0, gy0, gx0 + w, gy0 + h))
        .map_err(|e| Error::Param(format!("el ráster cae fuera del sistema EPSG:{to}: {e}")))?;
    // la celda del sistema nuevo: el tamaño de la antigua donde los dos sistemas comparten unidades
    let (cx, cy) = (gx0 + w / 2.0, gy0 + h / 2.0);
    let (ax, ay) = fwd
        .transform(cx, cy)
        .map_err(|e| Error::Param(e.to_string()))?;
    let (bx, by) = fwd
        .transform(cx + gcell, cy)
        .map_err(|e| Error::Param(e.to_string()))?;
    let step = (bx - ax).hypot(by - ay);
    let cell = if (step / gcell - 1.0).abs() < 1e-2 {
        gcell
    } else {
        step
    };
    Ok(([x0, y0, x1, y1], cell))
}

/// Un ráster (todas sus bandas) en el sistema `to`, en franjas de filas sea cual sea su tamaño
/// (véase [`crate::rastertools::warp`]). `resample` y `cell` toman por defecto lo que conviene a
/// los datos y a las unidades. La política se aplica a las esquinas y al centro del ráster. Las
/// alturas no se tocan.
pub fn raster_file(
    input: &Path,
    output: &Path,
    to: u16,
    from: Option<u16>,
    resample: Option<crate::rastertools::Resample>,
    cell: Option<f64>,
    compression: Option<crate::tiled::Compression>,
    policy: Policy,
) -> Result<String> {
    use crate::rastertools::{Target, WarpJob, warp};
    let pyr = crate::pyramid::Pyramid::open(input)?;
    if let Some(from) = from.or(pyr.epsg)
        && from != to
    {
        let e = pyr.extent();
        let (w, h) = (e[2] - e[0], e[3] - e[1]);
        let t = transformer_3d(from, to, Vertical::default())?;
        for (x, y) in [
            (e[0], e[1]),
            (e[2], e[1]),
            (e[0], e[3]),
            (e[2], e[3]),
            (e[0] + w / 2.0, e[1] + h / 2.0),
        ] {
            if let Err(hipso_crs::Error::Policy(m)) = t.transform_checked(x, y, 0.0, policy) {
                return Err(Error::Param(format!("transformación rechazada: {m}")));
            }
        }
    }
    let mut job = WarpJob::new(
        input.to_path_buf(),
        output.to_path_buf(),
        Target::Epsg { code: to, cell },
    );
    job.from = from;
    job.resample = resample;
    job.compression = compression;
    warp(&job)
}

/// El ráster en el sistema `to` con sus valores (alturas) en otra referencia vertical. Sin cambio
/// de alturas esto es [`grid`]. `from` y `to` pueden ser el mismo: entonces solo cambian las
/// alturas.
///
/// La política se aplica a la operación horizontal en las esquinas y el centro del ráster y a la
/// altura de cada celda. Un rechazo detiene la operación con un error.
pub fn grid_with(
    g: &Grid,
    from: u16,
    to: u16,
    vertical: Vertical,
    policy: Policy,
) -> Result<Grid> {
    let mut out = if from == to {
        g.clone()
    } else {
        let w = g.cols as f64 * g.cell;
        let h = g.rows as f64 * g.cell;
        let t = transformer_3d(from, to, Vertical::default())?;
        for (x, y) in [
            (g.x0, g.y0),
            (g.x0 + w, g.y0),
            (g.x0, g.y0 + h),
            (g.x0 + w, g.y0 + h),
            (g.x0 + w / 2.0, g.y0 + h / 2.0),
        ] {
            match t.transform_checked(x, y, 0.0, policy) {
                Err(hipso_crs::Error::Policy(m)) => {
                    return Err(Error::Param(format!("transformación rechazada: {m}")));
                }
                _ => {}
            }
        }
        grid(g, from, to)?
    };
    let Some(vo) = vertical.output else {
        return Ok(out);
    };
    let Some(vi) = vertical.input else {
        return Err(Error::Param(
            "el ráster no declara su referencia vertical: indica la de origen (--vertical-in)"
                .into(),
        ));
    };
    if vi == vo {
        return Ok(out);
    }
    let t = transformer_3d(to, to, Vertical { input: Some(vi), output: Some(vo) })?;
    let (cols, x0, y0, cell, rows) = (out.cols, out.x0, out.y0, out.cell, out.rows);
    let refused: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    out.data
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(row, line)| {
            let y = y0 + (rows - row) as f64 * cell - cell / 2.0;
            for (col, v) in line.iter_mut().enumerate() {
                if v.is_nan() {
                    continue;
                }
                let x = x0 + (col as f64 + 0.5) * cell;
                *v = match t.transform_checked(x, y, f64::from(*v), policy) {
                    Ok(((_, _, z), _)) => z as f32,
                    Err(hipso_crs::Error::Policy(m)) => {
                        if let Ok(mut r) = refused.lock() {
                            r.get_or_insert(m);
                        }
                        f32::NAN
                    }
                    Err(_) => f32::NAN,
                };
            }
        });
    if let Some(m) = refused.into_inner().ok().flatten() {
        return Err(Error::Param(format!("transformación rechazada: {m}")));
    }
    Ok(out)
}

/// El ráster sobre las celdas de `onto` (una rejilla vacía, en el sistema `to`): lo que debe ser un
/// modelo para combinarse celda a celda con otro.
pub fn grid_onto(g: &Grid, from: u16, to: u16, out: Grid) -> Result<Grid> {
    grid_onto_with(g, from, to, out, categorical(g))
}

/// [`grid_onto`] diciendo si se toma la celda más cercana (rásteres de números enteros) o se
/// interpola.
fn grid_onto_with(g: &Grid, from: u16, to: u16, mut out: Grid, nearest: bool) -> Result<Grid> {
    let inv = transformer(to, from)?;
    let cols = out.cols;
    let (ox, oy, ocell, orows) = (out.x0, out.y0, out.cell, out.rows);
    out.data
        .par_chunks_mut(cols)
        .enumerate()
        .for_each(|(row, line)| {
            let y = oy + (orows - row) as f64 * ocell - ocell / 2.0;
            for (col, v) in line.iter_mut().enumerate() {
                let x = ox + (col as f64 + 0.5) * ocell;
                *v = match inv.transform(x, y) {
                    Ok((sx, sy)) => {
                        let got = if nearest {
                            g.index(sx, sy).and_then(|i| g.data.get(i).copied())
                        } else {
                            // `sample` limita al borde: fuera del ráster no hay valor
                            g.index(sx, sy).and_then(|_| g.sample(sx, sy))
                        };
                        got.filter(|v| !v.is_nan()).unwrap_or(f32::NAN)
                    }
                    Err(_) => f32::NAN,
                };
            }
        });
    Ok(out)
}

/// Escribe `input` (una nube o un ráster) en el sistema `to`. `from` dice en qué está el archivo
/// cuando no lo declara. Devuelve una línea para el informe.
pub fn file(input: &Path, output: &Path, to: u16, from: Option<u16>) -> Result<String> {
    file_with(input, output, to, from, Vertical::default(), Policy::from_env())
}

/// [`file`] con un cambio de alturas y una política (véanse [`Vertical`] y [`Policy`]).
pub fn file_with(
    input: &Path,
    output: &Path,
    to: u16,
    from: Option<u16>,
    vertical: Vertical,
    policy: Policy,
) -> Result<String> {
    let name = input.file_name().map_or_else(
        || input.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let from = from.or_else(|| crate::crs::epsg_of(input)).ok_or_else(|| {
        Error::Param(format!(
            "«{name}» no declara su sistema de coordenadas: indica el de origen"
        ))
    })?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let (what, tail) = if ext == "las" || ext == "laz" {
        let r = cloud_with(input, output, from, to, vertical, policy)?;
        let mut tail = String::new();
        if r.dropped > 0 {
            tail.push_str(&format!(
                "; {} puntos fuera del sistema de destino, descartados",
                r.dropped
            ));
        }
        for n in &r.notes {
            tail.push_str(&format!("\n  aviso: {n}"));
        }
        (format!("{} puntos", r.read), tail)
    } else if crate::raster::is_raster_ext(&ext)
        && vertical.output.is_none()
        && crate::pyramid::Pyramid::open(input).is_ok()
    {
        let out = output.with_extension("tif");
        return raster_file(input, &out, to, Some(from), None, None, None, policy);
    } else if crate::raster::is_raster_ext(&ext) {
        let g = Grid::read(input)?;
        let mut vertical = vertical;
        if vertical.input.is_none() {
            vertical.input = Grid::read_vertical_epsg(input).map(VerticalRef::Epsg);
        }
        let moved = grid_with(&g, from, to, vertical, policy)?;
        let out = output.with_extension("tif");
        let vcode = match vertical.output {
            Some(VerticalRef::Epsg(v)) => Some(v),
            _ => None,
        };
        moved.write_tiff_vertical(&out, Some(to), vcode)?;
        (
            format!("ráster de {} x {} celdas", moved.cols, moved.rows),
            String::new(),
        )
    } else {
        return Err(Error::Unsupported(format!(
            "«{name}»: solo se reproyectan nubes LAS/LAZ y rásteres"
        )));
    };
    Ok(format!(
        "{name}: {what}, de {} a {}{tail} → {}",
        crate::crs::name(from),
        crate::crs::name(to),
        output.display()
    ))
}

/// Las regiones en el sistema `to`. Los rectángulos pasan a ser polígonos (ya no están alineados
/// con los ejes); un círculo conserva la imagen de su centro y su radio, cuya distorsión es de
/// partes por mil.
pub fn regions(list: &[Region], from: u16, to: u16) -> Result<Vec<Region>> {
    let t = transformer(from, to)?;
    let bad = |e: hipso_crs::Error| {
        Error::Param(format!("una zona cae fuera del sistema EPSG:{to}: {e}"))
    };
    list.iter()
        .map(|r| {
            let shape = match &r.shape {
                Shape2::Polygon(p) => {
                    let rings = p
                        .rings
                        .iter()
                        .map(|ring| {
                            ring.iter()
                                .map(|v| t.transform(v[0], v[1]).map(|(x, y)| [x, y]).map_err(bad))
                                .collect::<Result<Vec<_>>>()
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Shape2::Polygon(Polygon::new(rings))
                }
                Shape2::Buffered(b) => {
                    let rings = b
                        .poly
                        .rings
                        .iter()
                        .map(|ring| {
                            ring.iter()
                                .map(|v| t.transform(v[0], v[1]).map(|(x, y)| [x, y]).map_err(bad))
                                .collect::<Result<Vec<_>>>()
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Shape2::Buffered(Box::new(crate::vector::Buffered::new(
                        Polygon::new(rings),
                        b.dist,
                    )))
                }
                Shape2::Circle { cx, cy, r } => {
                    let (x, y) = t.transform(*cx, *cy).map_err(bad)?;
                    Shape2::Circle {
                        cx: x,
                        cy: y,
                        r: *r,
                    }
                }
                Shape2::Rect(b) => {
                    let ring = [[b[0], b[1]], [b[0], b[3]], [b[2], b[3]], [b[2], b[1]]]
                        .iter()
                        .map(|v| t.transform(v[0], v[1]).map(|(x, y)| [x, y]).map_err(bad))
                        .collect::<Result<Vec<_>>>()?;
                    Shape2::Polygon(Polygon::new(vec![ring]))
                }
            };
            Ok(Region {
                name: r.name.clone(),
                shape,
                attrs: r.attrs.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_layer_in_degrees_is_placed_over_a_cloud_with_no_system() {
        // una capa alrededor de Santiago (ETRS89 geográfico) y una tesela en UTM 29N
        let layer = [-8.6, 42.8, -8.5, 42.9];
        let tile = [530_000.0, 4_745_000.0, 540_000.0, 4_755_000.0];
        assert_eq!(guess_system(layer, 4258, tile, &[25829, 25830]), Some(25829));
        // ya sobre la nube tal como está: nada que hacer
        assert_eq!(guess_system(tile, 25829, tile, &[25829, 25830]), None);
        // ni de lejos cerca de ella
        assert_eq!(guess_system([10.0, 50.0, 11.0, 51.0], 4258, tile, &[25829, 25830]), None);
    }

    fn dir(tag: &str) -> Result<PathBuf> {
        let d = std::env::temp_dir().join(format!("rl_reproj_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    /// Un archivo LAS 1.2 de formato 1 con los puntos, declarado en `epsg`.
    fn write_las(path: &Path, pts: &[[f64; 3]], epsg: u16) -> Result<()> {
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
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        // desplazamiento cerca de los puntos, escala 1 mm
        let off = [
            (lo[0] / 1000.0).floor() * 1000.0,
            (lo[1] / 1000.0).floor() * 1000.0,
            0.0,
        ];
        for (at, v) in [
            (131, 0.001),
            (139, 0.001),
            (147, 0.001),
            (155, off[0]),
            (163, off[1]),
            (171, off[2]),
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
            for k in 0..3 {
                let q = ((p[k] - off[k]) / 0.001).round() as i32;
                rec[4 * k..4 * k + 4].copy_from_slice(&q.to_le_bytes());
            }
            rec[14] = 1 | (1 << 3);
            h.extend_from_slice(&rec);
        }
        let plain = path.with_extension("plain.las");
        std::fs::write(&plain, h)?;
        let r = Reader::open(&plain)?.with_projection(&las::epsg_record(epsg), 1, Some(epsg), None);
        let mut w = Writer::create(path, &r)?;
        Reader::open(&plain)?.scan_raw(|b| {
            for rec in b.as_chunks::<28>().0 {
                w.write(rec)?;
            }
            Ok(())
        })?;
        w.finish()?;
        std::fs::remove_file(plain)?;
        Ok(())
    }

    /// Un archivo LAS 1.4 de formato de punto 6, sin proyección, con los puntos.
    fn write_las14(path: &Path, pts: &[[f64; 3]]) -> Result<()> {
        let mut h = vec![0u8; 375];
        h[..4].copy_from_slice(b"LASF");
        h[24] = 1;
        h[25] = 4;
        h[94..96].copy_from_slice(&375u16.to_le_bytes());
        h[96..100].copy_from_slice(&375u32.to_le_bytes());
        h[104] = 6;
        h[105..107].copy_from_slice(&30u16.to_le_bytes());
        h[247..255].copy_from_slice(&(pts.len() as u64).to_le_bytes());
        let off = [(pts[0][0] / 1000.0).floor() * 1000.0, (pts[0][1] / 1000.0).floor() * 1000.0, 0.0];
        for k in 0..3 {
            h[131 + 8 * k..139 + 8 * k].copy_from_slice(&0.001f64.to_le_bytes());
            h[155 + 8 * k..163 + 8 * k].copy_from_slice(&off[k].to_le_bytes());
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in pts {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        for (k, v) in [hi[0], lo[0], hi[1], lo[1], hi[2], lo[2]].iter().enumerate() {
            h[179 + 8 * k..187 + 8 * k].copy_from_slice(&v.to_le_bytes());
        }
        for p in pts {
            let mut rec = [0u8; 30];
            for k in 0..3 {
                let q = ((p[k] - off[k]) / 0.001).round() as i32;
                rec[4 * k..4 * k + 4].copy_from_slice(&q.to_le_bytes());
            }
            rec[14] = 1 | (1 << 4);
            h.extend_from_slice(&rec);
        }
        std::fs::write(path, h)?;
        Ok(())
    }

    #[test]
    fn a_las_14_cloud_gets_the_compound_system_as_wkt() -> Result<()> {
        let d = dir("compound")?;
        let (plain, src, dst) = (d.join("p.las"), d.join("a.las"), d.join("b.las"));
        write_las14(&plain, &[[440_295.0, 4_474_235.0, 700.0], [440_395.0, 4_474_335.0, 710.0]])?;
        let r = Reader::open(&plain)?.with_projection(&las::epsg_record(25830), 1, Some(25830), None);
        let mut w = Writer::create(&src, &r)?;
        Reader::open(&plain)?.scan_raw(|b| {
            for rec in b.as_chunks::<30>().0 {
                w.write(rec)?;
            }
            Ok(())
        })?;
        w.finish()?;
        let v = Vertical {
            input: Some(VerticalRef::Ellipsoidal),
            output: Some(VerticalRef::Epsg(5782)),
        };
        cloud_with(&src, &dst, 25830, 25830, v, Policy::STRICT)?;
        let out = Reader::open(&dst)?;
        assert_eq!(out.header.format, 6);
        assert_eq!((out.header.epsg, out.header.vertical_epsg), (Some(25830), Some(5782)));
        let wkt = out.meta.wkt.clone().unwrap_or_default();
        assert!(wkt.starts_with("COMPD_CS["), "{wkt}");
        // el bit WKT de la codificación global está activado y no quedan claves GeoTIFF
        assert_eq!(out.meta.global_encoding & 0x10, 0x10);
        assert!(out.projection_records().0.windows(14).all(|w| w != b"GeoKeyDirector"));
        std::fs::remove_dir_all(&d)?;
        Ok(())
    }

    fn read_xyz(path: &Path) -> Result<Vec<[f64; 3]>> {
        let r = Reader::open(path)?;
        let mut v = Vec::new();
        r.scan(|p| v.push([p.x, p.y, p.z]))?;
        Ok(v)
    }

    #[test]
    fn heights_change_with_the_geoid_when_asked() -> Result<()> {
        let d = dir("heights")?;
        let pts = vec![[440_295.0, 4_474_235.0, 700.0], [440_395.0, 4_474_335.0, 710.0]];
        let (src, dst) = (d.join("e.las"), d.join("o.las"));
        write_las(&src, &pts, 25830)?;
        let v = Vertical {
            input: Some(VerticalRef::Ellipsoidal),
            output: Some(VerticalRef::Epsg(5782)),
        };
        let r = cloud_with(&src, &dst, 25830, 25830, v, Policy::STRICT)?;
        assert_eq!((r.read, r.dropped), (2, 0));
        let out = read_xyz(&dst)?;
        // la ondulación del geoide en Madrid es de unos +51 m
        assert!((out[0][2] - 648.89).abs() < 0.05, "{}", out[0][2]);
        assert!((out[0][0] - 440_295.0).abs() < 0.01);
        assert_eq!(Reader::open(&dst)?.header.vertical_epsg, Some(5782));
        // la salida declara su referencia vertical: volver atrás no necesita `input`
        let back = d.join("b.las");
        let v2 = Vertical {
            input: None,
            output: Some(VerticalRef::Ellipsoidal),
        };
        cloud_with(&dst, &back, 25830, 25830, v2, Policy::STRICT)?;
        let z = read_xyz(&back)?;
        assert!((z[1][2] - 710.0).abs() < 0.01, "{}", z[1][2]);
        Ok(())
    }

    #[test]
    fn a_height_change_without_a_declared_source_is_asked_for() -> Result<()> {
        let d = dir("novert")?;
        let (src, dst) = (d.join("a.las"), d.join("b.las"));
        write_las(&src, &[[440_295.0, 4_474_235.0, 700.0]], 25830)?;
        let v = Vertical {
            input: None,
            output: Some(VerticalRef::Epsg(5782)),
        };
        let e = cloud_with(&src, &dst, 25830, 25830, v, Policy::STRICT).unwrap_err();
        assert!(e.to_string().contains("--vertical-in"), "{e}");
        // fuera de la cobertura del geoide la política estricta rechaza en lugar de conservar la
        // altura
        let paris = d.join("p.las");
        write_las(&paris, &[[452_000.0, 5_411_000.0, 100.0]], 25831)?;
        let v = Vertical {
            input: Some(VerticalRef::Ellipsoidal),
            output: Some(VerticalRef::Epsg(5782)),
        };
        let e = cloud_with(&paris, &d.join("q.las"), 25831, 25831, v, Policy::STRICT).unwrap_err();
        assert!(e.to_string().contains("rechazada"), "{e}");
        assert!(!d.join("q.las").exists());
        let r = cloud_with(&paris, &d.join("q.las"), 25831, 25831, v, Policy::LENIENT)?;
        assert_eq!(r.approximated.len(), 1);
        Ok(())
    }

    #[test]
    fn a_cloud_moves_between_utm_zones_and_datums() -> Result<()> {
        let d = dir("cloud")?;
        // alrededor de Madrid, en ED50 UTM 30N
        let pts: Vec<[f64; 3]> = (0..50)
            .map(|i| {
                [
                    440_000.0 + f64::from(i) * 7.3,
                    4_474_000.0 + f64::from(i) * 3.1,
                    650.0 + f64::from(i),
                ]
            })
            .collect();
        let (src, dst) = (d.join("ed50.las"), d.join("etrs.las"));
        write_las(&src, &pts, 23030)?;
        let (read, dropped) = cloud(&src, &dst, 23030, 25830)?;
        assert_eq!((read, dropped), (50, 0));
        let out = read_xyz(&dst)?;
        assert_eq!(Reader::open(&dst)?.header.epsg, Some(25830));
        let t = transformer(23030, 25830)?;
        for (p, q) in pts.iter().zip(&out) {
            let (x, y) = t
                .transform(p[0], p[1])
                .map_err(|e| Error::Param(e.to_string()))?;
            assert!(
                (x - q[0]).abs() < 2e-3 && (y - q[1]).abs() < 2e-3 && (p[2] - q[2]).abs() < 2e-3
            );
        }
        // el desplazamiento conocido de ED50 a ETRS89 en Madrid: unos -109 m este, -207 m norte
        assert!((out[0][0] - pts[0][0] + 109.4).abs() < 1.0, "{:?}", out[0]);
        assert!((out[0][1] - pts[0][1] + 207.5).abs() < 1.0, "{:?}", out[0]);
        std::fs::remove_dir_all(d)?;
        Ok(())
    }

    #[test]
    fn a_raster_changes_vertical_reference_cell_by_cell() -> Result<()> {
        let mut g = Grid::new(440_000.0, 4_474_000.0, 10.0, 20, 20)?;
        for v in g.data.iter_mut() {
            *v = 700.0;
        }
        g.data[0] = f32::NAN;
        let v = Vertical {
            input: Some(VerticalRef::Ellipsoidal),
            output: Some(VerticalRef::Epsg(5782)),
        };
        let m = grid_with(&g, 25830, 25830, v, Policy::STRICT)?;
        assert!(m.data[0].is_nan());
        assert!((m.data[100] - 648.9).abs() < 0.3, "{}", m.data[100]);
        let none = Vertical { input: None, output: v.output };
        assert!(grid_with(&g, 25830, 25830, none, Policy::STRICT).is_err());
        let d = dir("rvert")?;
        let (src, dst) = (d.join("r.tif"), d.join("o.tif"));
        g.write_tiff(&src, Some(25830))?;
        file_with(&src, &dst, 25830, None, v, Policy::STRICT)?;
        assert_eq!(Grid::read_vertical_epsg(&dst), Some(5782));
        Ok(())
    }

    #[test]
    fn a_raster_keeps_its_surface_across_zones() -> Result<()> {
        // un plano en UTM 29N, sobre Galicia, llevado a UTM 30N
        let mut g = Grid::new(600_000.0, 4_700_000.0, 5.0, 200, 200)?;
        let plane = |x: f64, y: f64| (0.01 * (x - 600_000.0) + 0.02 * (y - 4_700_000.0)) as f32;
        for r in 0..g.rows {
            for c in 0..g.cols {
                let (x, y) = g.center(c, r);
                g.data[r * g.cols + c] = plane(x, y);
            }
        }
        let m = grid(&g, 25829, 25830)?;
        assert_eq!(m.cell, 5.0);
        let back = transformer(25830, 25829)?;
        let mut checked = 0;
        for r in (0..m.rows).step_by(7) {
            for c in (0..m.cols).step_by(7) {
                let v = m.data[r * m.cols + c];
                if v.is_nan() {
                    continue;
                }
                let (x, y) = m.center(c, r);
                let (sx, sy) = back
                    .transform(x, y)
                    .map_err(|e| Error::Param(e.to_string()))?;
                assert!((v - plane(sx, sy)).abs() < 0.06, "{v} {}", plane(sx, sy));
                checked += 1;
            }
        }
        assert!(checked > 100);
        Ok(())
    }

    #[test]
    fn mixed_files_are_unified_and_cleaned_up() -> Result<()> {
        let d = dir("unify")?;
        let pts: Vec<[f64; 3]> = (0..20)
            .map(|i| [500_000.0 + f64::from(i), 4_500_000.0, 700.0])
            .collect();
        write_las(&d.join("nube.las"), &pts, 25830)?;
        let mut g = Grid::new(499_900.0, 4_499_900.0, 10.0, 50, 50)?;
        g.data.fill(3.0);
        let (to29, _) = (transformer(25830, 25829)?, ());
        let (x, y) = to29
            .transform(499_900.0, 4_499_900.0)
            .map_err(|e| Error::Param(e.to_string()))?;
        let mut g29 = Grid::covering([x, y, x + 800.0, y + 800.0], 10.0)?;
        g29.data.fill(3.0);
        g29.write_tiff(&d.join("mdt.tif"), Some(25829))?;
        let files = [d.join("nube.las"), d.join("mdt.tif")];
        let tmp;
        {
            let u = crate::crs::unify(&files, false)?;
            assert_eq!(u.target, Some(25830));
            let moved = u.path(&files[1]);
            assert_ne!(moved, files[1]);
            assert_eq!(u.path(&files[0]), files[0]);
            assert_eq!(Grid::read_epsg(&moved), Some(25830));
            assert!(
                u.warnings.iter().any(|w| w.contains("mdt.tif")),
                "{:?}",
                u.warnings
            );
            tmp = moved;
        }
        assert!(!tmp.exists(), "the converted copy is removed");
        // ya en un sistema: no se convierte nada
        write_las(&d.join("otra.las"), &pts, 25830)?;
        let u = crate::crs::unify(&[d.join("nube.las"), d.join("otra.las")], false)?;
        assert!(u.warnings.is_empty());
        assert_eq!(u.path(&d.join("otra.las")), d.join("otra.las"));
        std::fs::remove_dir_all(d)?;
        Ok(())
    }

    #[test]
    fn clipping_with_plots_in_another_system_keeps_the_right_points() -> Result<()> {
        let d = dir("clip")?;
        // una nube en ED50 UTM 30N cuyas coordenadas ETRS89 son un retículo regular
        let to_ed50 = transformer(25830, 23030)?;
        let mut pts = Vec::new();
        let mut inside = 0;
        for i in 0..40 {
            for j in 0..40 {
                let (x, y) = (
                    440_005.0 + f64::from(i) * 10.0,
                    4_474_005.0 + f64::from(j) * 10.0,
                );
                if (440_105.0..=440_295.0).contains(&x) && (4_474_105.0..=4_474_295.0).contains(&y)
                {
                    inside += 1;
                }
                let (ex, ey) = to_ed50
                    .transform(x, y)
                    .map_err(|e| Error::Param(e.to_string()))?;
                pts.push([ex, ey, 700.0]);
            }
        }
        write_las(&d.join("ed50.las"), &pts, 23030)?;
        std::fs::write(
            d.join("p.geojson"),
            r#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::25830"}},"features":[{"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[440100,4474100],[440300,4474100],[440300,4474300],[440100,4474300],[440100,4474100]]]}}]}"#,
        )?;
        let regions = crate::geojson::read_regions(&d.join("p.geojson"), &Default::default())?;
        let mut job =
            crate::clip::ClipJob::new(vec![d.join("ed50.las")], regions, d.join("out.las"));
        job.regions_source = Some(d.join("p.geojson"));
        let report = crate::jobs::clip(&job)?;
        assert!(report.contains(&format!(" {inside} puntos  ")), "{report}");
        std::fs::remove_dir_all(d)?;
        Ok(())
    }

    #[test]
    fn regions_follow_their_system() -> Result<()> {
        let poly = Polygon::new(vec![vec![
            [500_000.0, 4_500_000.0],
            [500_100.0, 4_500_000.0],
            [500_100.0, 4_500_100.0],
        ]]);
        let list = vec![
            Region {
                name: "a".into(),
                shape: Shape2::Polygon(poly),
                attrs: vec![],
            },
            Region {
                name: "b".into(),
                shape: Shape2::Circle {
                    cx: 500_000.0,
                    cy: 4_500_000.0,
                    r: 12.5,
                },
                attrs: vec![],
            },
            Region {
                name: "c".into(),
                shape: Shape2::Rect([500_000.0, 4_500_000.0, 500_100.0, 4_500_100.0]),
                attrs: vec![],
            },
        ];
        let moved = regions(&list, 25830, 25829)?;
        let t = transformer(25830, 25829)?;
        let (x, y) = t
            .transform(500_000.0, 4_500_000.0)
            .map_err(|e| Error::Param(e.to_string()))?;
        match &moved[1].shape {
            Shape2::Circle { cx, cy, r } => {
                assert!((cx - x).abs() < 1e-6 && (cy - y).abs() < 1e-6 && *r == 12.5)
            }
            _ => return Err(Error::Param("un círculo sigue siendo un círculo".into())),
        }
        assert!(matches!(moved[2].shape, Shape2::Polygon(_)));
        assert_eq!(moved[0].name, "a");
        Ok(())
    }

    /// Un ráster movido por `raster_file` (franjas, nodos interpolados) coincide con el ráster
    /// movido entero por `grid`: mismas celdas, valores dentro de una pequeña fracción del relieve.
    #[test]
    fn strips_move_a_raster_to_the_same_cells_as_the_whole_raster() -> Result<()> {
        let d = dir("strips")?;
        // una superficie con colinas en UTM 30N cerca del borde de la zona (para que el paso a 29N
        // la deforme)
        let mut g = Grid::new(260_000.0, 4_800_000.0, 5.0, 700, 500)?;
        for r in 0..g.rows {
            for c in 0..g.cols {
                g.data[r * g.cols + c] =
                    300.0 + 20.0 * (c as f32 / 40.0).sin() + 15.0 * (r as f32 / 25.0).cos();
            }
        }
        for r in 200..215 {
            for c in 300..330 {
                g.data[r * g.cols + c] = f32::NAN;
            }
        }
        let input = d.join("dem.tif");
        g.write_tiff(&input, Some(25830))?;
        let whole = grid(&Grid::read(&input)?, 25830, 25829)?;
        let out = d.join("moved.tif");
        raster_file(
            &input,
            &out,
            25829,
            None,
            Some(crate::rastertools::Resample::Bilinear),
            None,
            None,
            Policy::STRICT,
        )?;
        let strips = Grid::read(&out)?;
        assert_eq!((strips.cols, strips.rows), (whole.cols, whole.rows));
        assert_eq!((strips.x0, strips.y0), (whole.x0, whole.y0));
        let (mut compared, mut worst, mut differ) = (0, 0f32, 0);
        for (a, b) in whole.data.iter().zip(&strips.data) {
            if a.is_nan() != b.is_nan() {
                differ += 1;
            } else if !a.is_nan() {
                compared += 1;
                worst = worst.max((a - b).abs());
            }
        }
        // las celdas del borde (medio píxel de diferencia en dónde terminan los datos) pueden
        // diferir
        assert!(compared > 100_000 && worst < 0.2 && differ < 2_000, "{compared} cells, worst {worst}, {differ} differ");
        std::fs::remove_dir_all(&d)?;
        Ok(())
    }
}
