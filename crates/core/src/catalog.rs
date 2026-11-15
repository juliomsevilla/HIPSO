//! Un catálogo de una entrega de hojas LAS/LAZ, hecho solo a partir de sus cabeceras (no se lee
//! ningún punto, así que mil hojas tardan un segundo más o menos), y las comprobaciones que dicen
//! si las hojas forman un teselado que se puede procesar: mismo sistema de coordenadas, ninguna
//! hoja repetida, ningún solape que cuente puntos dos veces, ningún hueco dentro del bloque.

use std::path::{Path, PathBuf};

use crate::las::Reader;
use crate::{Error, Result};

/// Lo que dice la cabecera de una hoja.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    /// min x, min y, min z, max x, max y, max z.
    pub bounds: [f64; 6],
    pub points: u64,
    pub version: (u8, u8),
    pub format: u8,
    pub compressed: bool,
    pub scale: [f64; 3],
    pub epsg: Option<u16>,
    pub vertical_epsg: Option<u16>,
    /// Se declara una proyección (un código EPSG o un WKT).
    pub has_crs: bool,
    /// COPC, o un archivo `.lax` junto a ella: leer una parte de la hoja no necesita la hoja
    /// entera.
    pub indexed: bool,
    pub file_size: u64,
}

impl Entry {
    /// Anchura y altura de la extensión.
    pub fn size(&self) -> (f64, f64) {
        (
            self.bounds[3] - self.bounds[0],
            self.bounds[4] - self.bounds[1],
        )
    }

    fn area(&self) -> f64 {
        let (w, h) = self.size();
        w.max(0.0) * h.max(0.0)
    }

    /// `[min x, min y, max x, max y]`.
    pub fn footprint(&self) -> [f64; 4] {
        [
            self.bounds[0],
            self.bounds[1],
            self.bounds[3],
            self.bounds[4],
        ]
    }
}

/// Cuán grave es un problema del teselado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// El resultado del proceso sería erróneo o la ejecución se detendría.
    Error,
    /// El resultado es utilizable, con una salvedad.
    Warning,
    /// Worth knowing.
    Note,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Unreadable,
    Empty,
    Duplicate,
    NoCrs,
    MixedCrs,
    MixedVertical,
    MixedFormat,
    MixedScale,
    Overlap,
    Strips,
    Gap,
    Unindexed,
}

impl Kind {
    /// Un nombre corto para guiones.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Unreadable => "unreadable",
            Kind::Empty => "empty",
            Kind::Duplicate => "duplicate",
            Kind::NoCrs => "no_crs",
            Kind::MixedCrs => "mixed_crs",
            Kind::MixedVertical => "mixed_vertical",
            Kind::MixedFormat => "mixed_format",
            Kind::MixedScale => "mixed_scale",
            Kind::Overlap => "overlap",
            Kind::Strips => "strips",
            Kind::Gap => "gap",
            Kind::Unindexed => "unindexed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub kind: Kind,
    pub severity: Severity,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    /// Archivos que no se pudieron leer, con el motivo.
    pub failed: Vec<(PathBuf, String)>,
}

/// The LAS and LAZ files of `paths`: files as they are, folders searched (one level, not their
/// subfolders) for `.las`, `.laz` and `.copc.laz`, in name order.
pub fn expand(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(p)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|f| {
                    f.is_file()
                        && f.extension().is_some_and(|e| {
                            e.eq_ignore_ascii_case("las") || e.eq_ignore_ascii_case("laz")
                        })
                })
                .collect();
            found.sort();
            out.extend(found);
        } else {
            out.push(p.clone());
        }
    }
    if out.is_empty() {
        return Err(Error::Param(
            "no hay archivos LAS ni LAZ que catalogar".into(),
        ));
    }
    Ok(out)
}

fn entry_of(path: &Path) -> Result<Entry> {
    let r = Reader::open(path)?;
    let h = &r.header;
    let copc = r
        .meta
        .vlrs
        .iter()
        .any(|v| v.user == "copc" && v.record == 1);
    Ok(Entry {
        path: path.to_path_buf(),
        bounds: h.bounds,
        points: h.count,
        version: h.version,
        format: h.format,
        compressed: h.compressed,
        scale: h.scale,
        epsg: h.epsg,
        vertical_epsg: h.vertical_epsg,
        has_crs: h.epsg.is_some() || r.meta.wkt.is_some(),
        indexed: copc || path.with_extension("lax").exists(),
        file_size: r.meta.file_size,
    })
}

/// Lee la cabecera de cada archivo. Un archivo que no se puede leer se lista en `failed` y los
/// demás siguen; que ningún archivo sea legible es un error.
pub fn catalog(paths: &[PathBuf]) -> Result<Catalog> {
    let files = expand(paths)?;
    let mut cat = Catalog::default();
    for f in &files {
        crate::check_cancel()?;
        match entry_of(f) {
            Ok(e) => cat.entries.push(e),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => cat.failed.push((f.clone(), e.to_string())),
        }
    }
    if cat.entries.is_empty() {
        return Err(Error::Param(format!(
            "ningún archivo se pudo leer; el primero, {}: {}",
            cat.failed[0].0.display(),
            cat.failed[0].1
        )));
    }
    Ok(cat)
}

/// Los puntos de `paths` a menos de `margin` metros de `extent` (`[x mínima, y mínima, x máxima, y
/// máxima]`), en una pasada sobre cada archivo que la alcanza: la franja de una hoja vecina que
/// linda con esta, guardada en memoria para poder usarla una y otra vez al precio de una lectura.
pub fn halo_points(
    paths: &[PathBuf],
    extent: [f64; 4],
    margin: f64,
) -> Result<Vec<crate::las::Point>> {
    let (x0, y0, x1, y1) = (
        extent[0] - margin,
        extent[1] - margin,
        extent[2] + margin,
        extent[3] + margin,
    );
    let mut out = Vec::new();
    for p in paths {
        crate::check_cancel()?;
        let r = Reader::open(p)?;
        let b = r.header.bounds;
        if b[0] > x1 || b[3] < x0 || b[1] > y1 || b[4] < y0 {
            continue;
        }
        r.scan(|pt| {
            if pt.x >= x0 && pt.x < x1 && pt.y >= y0 && pt.y < y1 {
                out.push(*pt);
            }
        })?;
    }
    Ok(out)
}

/// A cuál de las hojas `candidates` (índices de `entries`) pertenece un punto: aquella cuya
/// extensión lo contiene o, fuera de todas, la más cercana; un empate va al índice menor. Cada
/// celda de un bloque pertenece exactamente a una hoja, de modo que ninguna se escribe dos veces ni
/// se deja fuera.
pub fn owner_of(entries: &[Entry], candidates: &[usize], x: f64, y: f64) -> Option<usize> {
    candidates
        .iter()
        .copied()
        .map(|k| {
            let b = entries[k].footprint();
            let dx = (b[0] - x).max(0.0).max(x - b[2]);
            let dy = (b[1] - y).max(0.0).max(y - b[3]);
            (dx * dx + dy * dy, k)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(_, k)| k)
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len() % 2 == 1 {
        v[m]
    } else {
        (v[m - 1] + v[m]) / 2.0
    }
}

/// Un ráster grueso de las extensiones: qué celdas cubren las hojas, cada caja ampliada `grow` por
/// cada lado.
struct Cover {
    x0: f64,
    y0: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    on: Vec<bool>,
}

impl Cover {
    fn new(entries: &[Entry], cell: f64, grow: f64) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for e in entries {
            x0 = x0.min(e.bounds[0]);
            y0 = y0.min(e.bounds[1]);
            x1 = x1.max(e.bounds[3]);
            y1 = y1.max(e.bounds[4]);
        }
        // un margen de una celda vacía alrededor, con el que se distingue el exterior de los huecos
        let mut cell = cell;
        while ((x1 - x0) / cell + 3.0) * ((y1 - y0) / cell + 3.0) > 4.0e6 {
            cell *= 2.0;
        }
        let cols = ((x1 - x0) / cell).ceil() as usize + 3;
        let rows = ((y1 - y0) / cell).ceil() as usize + 3;
        let mut c = Self {
            x0: x0 - cell,
            y0: y0 - cell,
            cell,
            cols,
            rows,
            on: vec![false; cols * rows],
        };
        for e in entries {
            c.mark(e.footprint(), grow);
        }
        c
    }

    fn mark(&mut self, b: [f64; 4], grow: f64) {
        // una celda está cubierta cuando su centro está dentro de la caja
        let lo = |v: f64, o: f64| ((v - grow - o) / self.cell - 0.5).ceil().max(0.0) as usize;
        let hi = |v: f64, o: f64, n: usize| {
            let k = ((v + grow - o) / self.cell - 0.5).floor();
            if k < 0.0 {
                None
            } else {
                Some((k as usize).min(n - 1))
            }
        };
        let (c0, r0) = (lo(b[0], self.x0), lo(b[1], self.y0));
        let (Some(c1), Some(r1)) = (hi(b[2], self.x0, self.cols), hi(b[3], self.y0, self.rows))
        else {
            return;
        };
        for r in r0..=r1 {
            for c in c0..=c1 {
                self.on[r * self.cols + c] = true;
            }
        }
    }

    fn area(&self) -> f64 {
        self.on.iter().filter(|v| **v).count() as f64 * self.cell * self.cell
    }

    /// Áreas (m²) de las regiones vacías que no llegan al exterior, la mayor primero.
    fn holes(&self) -> Vec<f64> {
        let mut seen = vec![false; self.on.len()];
        let flood = |start: usize, seen: &mut Vec<bool>| -> usize {
            let mut stack = vec![start];
            seen[start] = true;
            let mut n = 0;
            while let Some(i) = stack.pop() {
                n += 1;
                let (c, r) = (i % self.cols, i / self.cols);
                let mut next = |c2: usize, r2: usize| {
                    let j = r2 * self.cols + c2;
                    if !self.on[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                };
                if c > 0 {
                    next(c - 1, r);
                }
                if c + 1 < self.cols {
                    next(c + 1, r);
                }
                if r > 0 {
                    next(c, r - 1);
                }
                if r + 1 < self.rows {
                    next(c, r + 1);
                }
            }
            n
        };
        // el exterior: el margen está vacío, así que su primera celda lo alcanza todo
        flood(0, &mut seen);
        let mut out = Vec::new();
        for i in 0..self.on.len() {
            if !self.on[i] && !seen[i] {
                out.push(flood(i, &mut seen) as f64 * self.cell * self.cell);
            }
        }
        out.sort_by(|a, b| b.total_cmp(a));
        out
    }
}

/// El solape de dos extensiones: `(área, proporción de la menor)`, cuando cada uno de sus lados es
/// más largo que `tolerance`.
fn overlap(a: &Entry, b: &Entry, tolerance: f64) -> Option<(f64, f64)> {
    let ox = a.bounds[3].min(b.bounds[3]) - a.bounds[0].max(b.bounds[0]);
    let oy = a.bounds[4].min(b.bounds[4]) - a.bounds[1].max(b.bounds[1]);
    if ox <= tolerance || oy <= tolerance {
        return None;
    }
    let small = a.area().min(b.area());
    Some((ox * oy, if small > 0.0 { ox * oy / small } else { 0.0 }))
}

fn name(e: &Entry) -> String {
    e.path.file_name().map_or_else(
        || e.path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn some(names: &[String], limit: usize) -> String {
    let mut s = names[..names.len().min(limit)].join(", ");
    if names.len() > limit {
        s.push_str(&format!(" y {} más", names.len() - limit));
    }
    s
}

impl Catalog {
    pub fn points(&self) -> u64 {
        self.entries.iter().map(|e| e.points).sum()
    }

    /// `[x mínima, y mínima, z mínima, x máxima, y máxima, z máxima]` del bloque.
    pub fn bounds(&self) -> [f64; 6] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for e in &self.entries {
            for k in 0..3 {
                b[k] = b[k].min(e.bounds[k]);
                b[k + 3] = b[k + 3].max(e.bounds[k + 3]);
            }
        }
        b
    }

    /// El lado (m) de una hoja típica: la mediana de los dos lados de cada extensión.
    pub fn sheet_side(&self) -> (f64, f64) {
        (
            median(self.entries.iter().map(|e| e.size().0).collect()),
            median(self.entries.iter().map(|e| e.size().1).collect()),
        )
    }

    fn cover(&self, tolerance: f64, grow: f64) -> Cover {
        let (w, h) = self.sheet_side();
        let cell = (w.min(h) / 8.0).max(tolerance).max(1e-3);
        Cover::new(&self.entries, cell, grow)
    }

    /// Área (m²) que cubren las extensiones, contando una vez los solapes.
    pub fn area(&self, tolerance: f64) -> f64 {
        self.cover(tolerance, 0.0).area()
    }

    /// Las hojas, distintas de `k`, cuya extensión llega a `buffer` metros de la de la hoja `k`.
    pub fn neighbours(&self, k: usize, buffer: f64) -> Vec<usize> {
        let a = self.entries[k].footprint();
        (0..self.entries.len())
            .filter(|&j| {
                let b = self.entries[j].footprint();
                j != k
                    && b[0] <= a[2] + buffer
                    && b[2] >= a[0] - buffer
                    && b[1] <= a[3] + buffer
                    && b[3] >= a[1] - buffer
            })
            .collect()
    }

    /// Todo lo que está mal o es raro en el teselado. `tolerance` (m) es cuánto pueden solaparse
    /// las extensiones, o quedarse cortas unas de otras, sin que cuente.
    pub fn issues(&self, tolerance: f64) -> Vec<Issue> {
        let mut out = Vec::new();
        let mut add = |kind: Kind, severity: Severity, message: String| {
            out.push(Issue {
                kind,
                severity,
                message,
            });
        };
        for (p, why) in &self.failed {
            add(
                Kind::Unreadable,
                Severity::Error,
                format!("{}: {why}", p.display()),
            );
        }
        let es = &self.entries;
        let empty: Vec<String> = es.iter().filter(|e| e.points == 0).map(name).collect();
        if !empty.is_empty() {
            add(
                Kind::Empty,
                Severity::Warning,
                format!("{} hoja(s) sin puntos: {}", empty.len(), some(&empty, 5)),
            );
        }
        // la misma hoja dos veces
        let mut twins: Vec<String> = Vec::new();
        for (i, a) in es.iter().enumerate() {
            for b in &es[i + 1..] {
                if a.bounds == b.bounds && a.points == b.points && a.points > 0 {
                    twins.push(format!("{} = {}", name(a), name(b)));
                }
            }
        }
        if !twins.is_empty() {
            add(
                Kind::Duplicate,
                Severity::Error,
                format!(
                    "{} pareja(s) de hojas idénticas (misma extensión y número de puntos), que contarían los puntos dos veces: {}",
                    twins.len(),
                    some(&twins, 5)
                ),
            );
        }
        // coordinate systems
        let none: Vec<String> = es.iter().filter(|e| !e.has_crs).map(name).collect();
        if !none.is_empty() {
            add(
                Kind::NoCrs,
                Severity::Warning,
                format!(
                    "{} hoja(s) sin sistema de coordenadas declarado: {}",
                    none.len(),
                    some(&none, 5)
                ),
            );
        }
        let mut codes: Vec<u16> = es.iter().filter_map(|e| e.epsg).collect();
        codes.sort_unstable();
        codes.dedup();
        if codes.len() > 1 {
            let list = codes
                .iter()
                .map(|c| format!("EPSG:{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            add(
                Kind::MixedCrs,
                Severity::Error,
                format!(
                    "las hojas están en sistemas distintos ({list}): reproyecta antes de procesar"
                ),
            );
        }
        let mut vert: Vec<Option<u16>> = es.iter().map(|e| e.vertical_epsg).collect();
        vert.sort_unstable();
        vert.dedup();
        if vert.len() > 1 {
            add(
                Kind::MixedVertical,
                Severity::Warning,
                "las hojas no comparten la referencia vertical (o algunas no la declaran): las alturas pueden no ser comparables"
                    .into(),
            );
        }
        let mut formats: Vec<u8> = es.iter().map(|e| e.format).collect();
        formats.sort_unstable();
        formats.dedup();
        if formats.len() > 1 {
            let list = formats
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            add(
                Kind::MixedFormat,
                Severity::Warning,
                format!(
                    "formatos de punto distintos ({list}): lo que algunos no llevan (hora GPS, color, infrarrojo) se pierde donde las hojas se unen"
                ),
            );
        }
        let scale0 = es[0].scale;
        if es.iter().any(|e| e.scale != scale0) {
            add(
                Kind::MixedScale,
                Severity::Note,
                "las hojas cuantizan las coordenadas con escalas distintas".into(),
            );
        }
        // solapes: barrido sobre los bordes oeste
        let mut order: Vec<usize> = (0..es.len()).collect();
        order.sort_by(|&a, &b| es[a].bounds[0].total_cmp(&es[b].bounds[0]));
        let (mut pairs, mut area, mut heavy) = (0usize, 0.0f64, 0usize);
        let mut examples: Vec<String> = Vec::new();
        for (n, &i) in order.iter().enumerate() {
            for &j in &order[n + 1..] {
                if es[j].bounds[0] >= es[i].bounds[3] - tolerance {
                    break;
                }
                if let Some((a, share)) = overlap(&es[i], &es[j], tolerance) {
                    pairs += 1;
                    area += a;
                    if share >= 0.5 {
                        heavy += 1;
                    }
                    if examples.len() < 3 {
                        examples.push(format!(
                            "{} y {} ({:.0} % de la menor)",
                            name(&es[i]),
                            name(&es[j]),
                            share * 100.0
                        ));
                    }
                }
            }
        }
        if heavy * 2 > pairs && heavy > 0 {
            add(
                Kind::Strips,
                Severity::Note,
                format!(
                    "{heavy} de {pairs} solapes pasan de la mitad de la hoja menor: parecen pasadas de vuelo, no un teselado (los puntos del solape están en las dos)"
                ),
            );
        } else if pairs > 0 {
            add(
                Kind::Overlap,
                Severity::Warning,
                format!(
                    "{pairs} pareja(s) de hojas se solapan más de {tolerance} m ({:.0} m² en total), y los puntos del solape se cuentan dos veces: {}",
                    area,
                    examples.join("; ")
                ),
            );
        }
        // huecos dentro del bloque (un hueco más estrecho que la tolerancia no cuenta)
        let holes = self.cover(tolerance, tolerance / 2.0).holes();
        let cell = self.cover(tolerance, 0.0).cell;
        let big: Vec<f64> = holes
            .into_iter()
            .filter(|a| *a > 2.0 * cell * cell)
            .collect();
        if !big.is_empty() {
            let total: f64 = big.iter().sum();
            add(
                Kind::Gap,
                Severity::Warning,
                format!(
                    "{} hueco(s) rodeado(s) de hojas, {:.0} m² en total (el mayor, {:.0} m²): faltan hojas o hay zonas sin datos",
                    big.len(),
                    total,
                    big[0]
                ),
            );
        }
        let loose: Vec<String> = es
            .iter()
            .filter(|e| !e.indexed && e.compressed)
            .map(name)
            .collect();
        if !loose.is_empty() && es.len() > 1 {
            add(
                Kind::Unindexed,
                Severity::Note,
                format!(
                    "{} de {} hojas LAZ sin índice espacial (COPC o .lax): leer una parte obliga a leerlas enteras",
                    loose.len(),
                    es.len()
                ),
            );
        }
        out
    }

    /// El resumen y los problemas, como texto.
    pub fn report(&self, tolerance: f64) -> String {
        let es = &self.entries;
        let b = self.bounds();
        let area = self.area(tolerance);
        let (w, h) = self.sheet_side();
        let mut formats: Vec<String> = es
            .iter()
            .map(|e| format!("LAS {}.{} formato {}", e.version.0, e.version.1, e.format))
            .collect();
        formats.sort();
        formats.dedup();
        let mut codes: Vec<String> = es
            .iter()
            .map(|e| {
                e.epsg
                    .map_or_else(|| "sin declarar".to_string(), |c| format!("EPSG:{c}"))
            })
            .collect();
        codes.sort();
        codes.dedup();
        let mut s = format!(
            "Catálogo: {} hoja(s), {} puntos\n  extensión  X {:.2} a {:.2} · Y {:.2} a {:.2} · Z {:.2} a {:.2}\n  superficie {:.4} km² cubiertos por las extensiones\n  densidad   {:.2} puntos/m² sobre esa superficie\n  hojas      {:.0} x {:.0} m (mediana), de {} a {} puntos\n  formato    {}\n  sistema    {}\n  índice     {} de {} con índice espacial",
            es.len(),
            self.points(),
            b[0],
            b[3],
            b[1],
            b[4],
            b[2],
            b[5],
            area / 1.0e6,
            if area > 0.0 {
                self.points() as f64 / area
            } else {
                0.0
            },
            w,
            h,
            es.iter().map(|e| e.points).min().unwrap_or(0),
            es.iter().map(|e| e.points).max().unwrap_or(0),
            formats.join("; "),
            codes.join(", "),
            es.iter().filter(|e| e.indexed).count(),
            es.len(),
        );
        let issues = self.issues(tolerance);
        if issues.is_empty() {
            s.push_str("\n  comprobación: sin problemas");
        }
        for sev in [Severity::Error, Severity::Warning, Severity::Note] {
            for i in issues.iter().filter(|i| i.severity == sev) {
                let tag = match sev {
                    Severity::Error => "ERROR",
                    Severity::Warning => "AVISO",
                    Severity::Note => "nota ",
                };
                s.push_str(&format!("\n  {tag} [{}] {}", i.kind.id(), i.message));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, b: [f64; 4], epsg: Option<u16>) -> Entry {
        Entry {
            path: PathBuf::from(name),
            bounds: [b[0], b[1], 0.0, b[2], b[3], 10.0],
            points: 1000,
            version: (1, 4),
            format: 6,
            compressed: true,
            scale: [0.01; 3],
            epsg,
            vertical_epsg: None,
            has_crs: epsg.is_some(),
            indexed: true,
            file_size: 1,
        }
    }

    /// Un bloque de `n x n` hojas de 100 m.
    fn block(n: usize, skip: Option<(usize, usize)>) -> Catalog {
        let mut es = Vec::new();
        for i in 0..n {
            for j in 0..n {
                if skip == Some((i, j)) {
                    continue;
                }
                let (x, y) = (i as f64 * 100.0, j as f64 * 100.0);
                es.push(entry(
                    &format!("s_{i}_{j}.laz"),
                    [x, y, x + 100.0, y + 100.0],
                    Some(25830),
                ));
            }
        }
        Catalog {
            entries: es,
            failed: Vec::new(),
        }
    }

    fn kinds(c: &Catalog) -> Vec<Kind> {
        c.issues(1.0).iter().map(|i| i.kind).collect()
    }

    #[test]
    fn a_clean_tiling_has_no_issues() {
        let c = block(3, None);
        assert!(kinds(&c).is_empty(), "{:?}", c.issues(1.0));
        assert!((c.area(1.0) - 90_000.0).abs() < 1.0);
    }

    #[test]
    fn a_hole_inside_is_found_and_the_outside_is_not() {
        let c = block(3, Some((1, 1)));
        let i = c.issues(1.0);
        assert_eq!(i.iter().filter(|i| i.kind == Kind::Gap).count(), 1, "{i:?}");
        // una esquina que falta es el exterior, no un hueco
        assert!(!kinds(&block(3, Some((0, 0)))).contains(&Kind::Gap));
    }

    #[test]
    fn overlap_and_flight_lines_are_told_apart() {
        let mut c = block(2, None);
        // una hoja desplazada 20 m sobre su vecina: una costura de solape
        c.entries
            .push(entry("seam.laz", [80.0, 0.0, 180.0, 100.0], Some(25830)));
        assert!(kinds(&c).contains(&Kind::Overlap));
        // franjas: el mismo terreno cubierto una y otra vez
        let mut s = Catalog::default();
        for k in 0..4 {
            s.entries.push(entry(
                &format!("l{k}.laz"),
                [k as f64 * 10.0, 0.0, 400.0 + k as f64 * 10.0, 100.0],
                Some(25830),
            ));
        }
        let kinds = kinds(&s);
        assert!(
            kinds.contains(&Kind::Strips) && !kinds.contains(&Kind::Overlap),
            "{kinds:?}"
        );
    }

    #[test]
    fn mixed_systems_and_a_sheet_twice_are_errors() {
        let mut c = block(2, None);
        c.entries[0].epsg = Some(25829);
        c.entries[3] = c.entries[2].clone();
        let i = c.issues(1.0);
        for k in [Kind::MixedCrs, Kind::Duplicate] {
            assert!(
                i.iter()
                    .any(|i| i.kind == k && i.severity == Severity::Error),
                "{k:?} {i:?}"
            );
        }
    }

    #[test]
    fn neighbours_within_the_buffer() {
        let c = block(3, None);
        // la hoja del medio toca a todas las demás; una esquina toca a tres
        assert_eq!(c.neighbours(4, 5.0).len(), 8);
        assert_eq!(c.neighbours(0, 5.0).len(), 3);
        // un margen de nada sigue encontrando las que comparten un borde
        assert_eq!(c.neighbours(0, 0.0).len(), 3);
    }

    #[test]
    fn a_cell_belongs_to_one_sheet() {
        let c = block(2, None);
        let all: Vec<usize> = (0..c.entries.len()).collect();
        // dentro de una hoja, fuera del bloque (la más cercana) y sobre la costura (el índice
        // menor)
        let name = |x: f64, y: f64| {
            let k = owner_of(&c.entries, &all, x, y).unwrap_or(usize::MAX);
            c.entries[k].path.display().to_string()
        };
        assert_eq!(name(50.0, 50.0), "s_0_0.laz");
        assert_eq!(name(150.0, 150.0), "s_1_1.laz");
        assert_eq!(name(-30.0, 50.0), "s_0_0.laz");
        assert_eq!(name(100.0, 50.0), "s_0_0.laz");
        assert_eq!(name(100.0, 150.0), "s_0_1.laz");
    }

    #[test]
    fn the_report_names_what_is_wrong() {
        let r = block(3, Some((1, 1))).report(1.0);
        assert!(r.contains("9") || r.contains("8 hoja"), "{r}");
        assert!(r.contains("[gap]"), "{r}");
    }
}
