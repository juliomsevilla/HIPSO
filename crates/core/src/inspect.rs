//! A report on a LAS/LAZ file: what the header says, what the points are, and where the two
//! disagree. One streaming pass, so it is quick on any size of cloud.

use std::path::{Path, PathBuf};

use crate::Result;
use crate::las::{self, Reader, VlrInfo};

/// Histogram bins over the elevation range.
const Z_BINS: usize = 8192;

/// One file's report.
#[derive(Clone, Debug, Default)]
pub struct FileReport {
    pub path: PathBuf,
    pub file_size: u64,
    pub version: (u8, u8),
    pub format: u8,
    pub compressed: bool,
    pub record_len: u16,
    /// Bytes per record beyond the format's own.
    pub extra_bytes: u16,
    /// Points the header declares, and points read.
    pub points: u64,
    pub points_read: u64,
    pub software: String,
    pub system_id: String,
    /// Creation day of the year and year, when set.
    pub creation: Option<(u16, u16)>,
    pub scale: [f64; 3],
    pub offset: [f64; 3],
    /// min x, min y, min z, max x, max y, max z, as the header says and as the points are.
    pub header_bounds: [f64; 6],
    pub bounds: [f64; 6],
    pub epsg: Option<u16>,
    pub crs_name: Option<String>,
    pub wkt: Option<String>,
    pub vlrs: Vec<VlrInfo>,
    /// Points by return number 1..=15 as the header says and as they are.
    pub returns_header: Vec<u64>,
    pub returns: [u64; 16],
    /// Points by number of returns of their pulse.
    pub pulse_sizes: [u64; 16],
    /// Points by class (0..=255).
    pub classes: Vec<(u8, u64)>,
    pub intensity: (u16, u16, f64),
    /// Elevation at 1, 5, 25, 50, 75, 95 and 99 % of the points.
    pub z_percentiles: [f64; 7],
    /// Points far below and far above the bulk of the cloud.
    pub z_outliers: (u64, u64),
    pub gps_time: Option<(f64, f64)>,
    /// Day of capture (`YYYY-MM-DD`) from the GPS time, or the file's creation date.
    pub capture_date: Option<String>,
    pub scan_angle: (f32, f32),
    /// Flight lines (point source IDs) and their points, most points first.
    pub flight_lines: Vec<(u16, u64)>,
    pub has_rgb: bool,
    pub rgb_max: u16,
    pub withheld: u64,
    pub overlap: u64,
    pub synthetic: u64,
    pub key_points: u64,
    /// Area of the bounding box, and of the cells that hold points.
    pub area_bbox: f64,
    pub area_covered: f64,
    /// Points per square unit over the covered area, and first returns per square unit.
    pub density: f64,
    pub pulse_density: f64,
    /// Mean distance between points, from the density.
    pub spacing: f64,
    pub index_files: Vec<String>,
    pub warnings: Vec<String>,
}

/// The name of a LAS class code (ASPRS, LAS 1.4 R15).
pub fn class_name(class: u8) -> &'static str {
    crate::classes::name(class)
}

/// What a point format holds, in words.
pub fn format_description(format: u8) -> &'static str {
    match format {
        0 => "XYZ, intensidad, retornos, clase, ángulo y línea de vuelo",
        1 => "formato 0 más tiempo GPS",
        2 => "formato 0 más color RGB",
        3 => "formato 0 más tiempo GPS y color RGB",
        4 => "formato 1 más forma de onda",
        5 => "formato 3 más forma de onda",
        6 => "LAS 1.4: tiempo GPS, canal del escáner y ángulo ampliado",
        7 => "formato 6 más color RGB",
        8 => "formato 6 más RGB e infrarrojo cercano",
        9 => "formato 6 más forma de onda",
        10 => "formato 6 más RGB, infrarrojo y forma de onda",
        _ => "formato desconocido",
    }
}

/// Reads a file once and reports on it.
pub fn inspect(path: &Path) -> Result<FileReport> {
    let reader = Reader::open(path)?;
    let h = &reader.header;
    let m = &reader.meta;
    let mut r = FileReport {
        path: path.to_path_buf(),
        file_size: m.file_size,
        version: h.version,
        format: h.format,
        compressed: h.compressed,
        record_len: h.record_len,
        points: h.count,
        software: m.software.clone(),
        system_id: m.system_id.clone(),
        creation: (m.creation.1 > 0).then_some(m.creation),
        scale: h.scale,
        offset: h.offset,
        header_bounds: h.bounds,
        epsg: h.epsg,
        crs_name: m.wkt.as_deref().and_then(las::wkt_name),
        wkt: m.wkt.clone(),
        vlrs: m.vlrs.clone(),
        returns_header: m.by_return.clone(),
        ..Default::default()
    };
    let base = [20u16, 28, 26, 34, 57, 63, 30, 36, 38, 59, 67]
        .get(usize::from(h.format))
        .copied()
        .unwrap_or(h.record_len);
    r.extra_bytes = h.record_len.saturating_sub(base);

    // Occupancy grid: cells holding about 8 points on average, from the header's own figures.
    let (w, hh) = (
        (h.bounds[3] - h.bounds[0]).max(0.0),
        (h.bounds[4] - h.bounds[1]).max(0.0),
    );
    r.area_bbox = w * hh;
    let d0 = if r.area_bbox > 0.0 {
        h.count as f64 / r.area_bbox
    } else {
        1.0
    };
    let mut cell = (8.0 / d0.max(1e-9)).sqrt().clamp(1.0, 25.0);
    while (w / cell + 1.0) * (hh / cell + 1.0) > 32e6 {
        cell *= 2.0;
    }
    let (gc, gr) = ((w / cell) as usize + 1, (hh / cell) as usize + 1);
    let mut occupied = vec![0u64; (gc * gr).div_ceil(64)];

    let (zlo, zhi) = (h.bounds[2], h.bounds[5]);
    let zspan = (zhi - zlo).max(1e-9);
    let mut zhist = vec![0u64; Z_BINS];
    let mut lines: std::collections::HashMap<u16, u64> = std::collections::HashMap::new();
    let mut class_counts = [0u64; 256];
    let (mut imin, mut imax, mut isum) = (u16::MAX, 0u16, 0.0f64);
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let (mut gmin, mut gmax) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut amin, mut amax) = (f32::INFINITY, f32::NEG_INFINITY);
    let rgb_format = matches!(h.format, 2 | 3 | 5 | 7 | 8 | 10);
    let rec_len = usize::from(h.record_len);
    let has_gps = matches!(h.format, 1 | 3 | 4 | 5..=10);
    let mut first_returns = 0u64;

    r.points_read = reader.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let (Some(p), e) = (reader.decode(rec), reader.decode_extra(rec)) else {
                continue;
            };
            b = [
                b[0].min(p.x),
                b[1].min(p.y),
                b[2].min(p.z),
                b[3].max(p.x),
                b[4].max(p.y),
                b[5].max(p.z),
            ];
            let k = (((p.z - zlo) / zspan * Z_BINS as f64) as isize).clamp(0, Z_BINS as isize - 1)
                as usize;
            zhist[k] += 1;
            if let (Some(c), Some(rw)) = (
                (((p.x - h.bounds[0]) / cell).floor() as isize)
                    .try_into()
                    .ok()
                    .filter(|c: &usize| *c < gc),
                (((p.y - h.bounds[1]) / cell).floor() as isize)
                    .try_into()
                    .ok()
                    .filter(|r: &usize| *r < gr),
            ) {
                let i = rw * gc + c;
                occupied[i / 64] |= 1 << (i % 64);
            }
            r.returns[usize::from(p.return_number.min(15))] += 1;
            r.pulse_sizes[usize::from(p.number_of_returns.min(15))] += 1;
            first_returns += u64::from(p.return_number <= 1);
            class_counts[usize::from(p.class)] += 1;
            imin = imin.min(p.intensity);
            imax = imax.max(p.intensity);
            isum += f64::from(p.intensity);
            *lines.entry(e.source_id).or_insert(0) += 1;
            if has_gps {
                gmin = gmin.min(e.gps_time);
                gmax = gmax.max(e.gps_time);
            }
            amin = amin.min(e.scan_angle);
            amax = amax.max(e.scan_angle);
            if let Some(c) = e.rgb {
                r.rgb_max = r.rgb_max.max(c[0]).max(c[1]).max(c[2]);
            }
            r.withheld += u64::from(p.withheld);
            r.overlap += u64::from(e.overlap);
            r.synthetic += u64::from(e.synthetic);
            r.key_points += u64::from(e.key_point);
        }
        Ok(())
    })?;

    let n = r.points_read.max(1) as f64;
    r.bounds = if r.points_read > 0 { b } else { [0.0; 6] };
    r.classes = (0u16..=255)
        .filter(|&c| class_counts[c as usize] > 0)
        .map(|c| (c as u8, class_counts[c as usize]))
        .collect();
    r.intensity = (if r.points_read > 0 { imin } else { 0 }, imax, isum / n);
    r.has_rgb = rgb_format && r.rgb_max > 0;
    r.gps_time = (has_gps && gmax >= gmin).then_some((gmin, gmax));
    r.capture_date = if has_gps && gmax >= gmin {
        crate::las::gps_date(gmin, reader.meta.global_encoding, reader.meta.creation)
    } else {
        crate::las::gps_date(f64::NAN, reader.meta.global_encoding, reader.meta.creation)
    };
    r.scan_angle = if amin <= amax {
        (amin, amax)
    } else {
        (0.0, 0.0)
    };
    let mut fl: Vec<(u16, u64)> = lines.into_iter().collect();
    fl.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    fl.truncate(256);
    r.flight_lines = fl;

    // Percentiles from the histogram, at the middle of the bin where they fall.
    let at = |q: f64| {
        let target = (q * n).max(1.0);
        let mut acc = 0.0;
        for (i, c) in zhist.iter().enumerate() {
            acc += *c as f64;
            if acc >= target {
                return zlo + (i as f64 + 0.5) / Z_BINS as f64 * zspan;
            }
        }
        zhi
    };
    r.z_percentiles = [
        at(0.01),
        at(0.05),
        at(0.25),
        at(0.5),
        at(0.75),
        at(0.95),
        at(0.99),
    ];
    // Outliers: beyond three times the span of the central 98 % from either end of it.
    let spread = (r.z_percentiles[6] - r.z_percentiles[0]).max(1.0);
    let (low, high) = (
        r.z_percentiles[0] - 3.0 * spread,
        r.z_percentiles[6] + 3.0 * spread,
    );
    let bin_of = |z: f64| {
        (((z - zlo) / zspan * Z_BINS as f64) as isize).clamp(0, Z_BINS as isize - 1) as usize
    };
    r.z_outliers = (
        zhist.iter().take(bin_of(low)).sum::<u64>() * u64::from(low > zlo),
        zhist.iter().skip(bin_of(high) + 1).sum::<u64>() * u64::from(high < zhi),
    );

    r.area_covered = occupied
        .iter()
        .map(|w| u64::from(w.count_ones()))
        .sum::<u64>() as f64
        * cell
        * cell;
    if r.area_covered > 0.0 {
        r.density = n / r.area_covered;
        r.pulse_density = first_returns as f64 / r.area_covered;
        r.spacing = (1.0 / r.density).sqrt();
    }
    r.index_files = ["lax", "ldi", "ldx", "ldr", "prj"]
        .iter()
        .filter_map(|e| {
            let sib = path.with_extension(e);
            sib.is_file().then(|| {
                sib.file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned())
                    .to_string()
            })
        })
        .collect();
    r.warnings = warnings(&r);
    Ok(r)
}

/// What looks wrong, or worth knowing, in plain words.
fn warnings(r: &FileReport) -> Vec<String> {
    let mut w = Vec::new();
    if r.points_read != r.points {
        w.push(format!(
            "La cabecera declara {} puntos y se han leído {}.",
            r.points, r.points_read
        ));
    }
    if r.epsg.is_none() && r.wkt.is_none() {
        w.push("El archivo no declara su sistema de coordenadas: hay que indicarlo (por ejemplo EPSG:25829 para ETRS89 UTM 29N).".into());
    }
    if r.version == (1, 0) {
        w.push("Versión LAS 1.0, muy antigua: algunos programas no la leen bien.".into());
    }
    let tol = r.scale.iter().cloned().fold(0.0, f64::max) * 1.5;
    let off = (0..6)
        .map(|k| (r.header_bounds[k] - r.bounds[k]).abs())
        .fold(0.0, f64::max);
    if r.points_read > 0 && off > tol.max(1e-6) {
        w.push(format!(
            "Las extensiones de la cabecera no coinciden con los puntos (z real {:.2} a {:.2}, cabecera {:.2} a {:.2}).",
            r.bounds[2], r.bounds[5], r.header_bounds[2], r.header_bounds[5]
        ));
    }
    let by_hdr: Vec<u64> = (1..=5)
        .map(|k| r.returns_header.get(k - 1).copied().unwrap_or(0))
        .collect();
    let by_real: Vec<u64> = (1..=5).map(|k| r.returns[k]).collect();
    if r.returns_header.iter().any(|&c| c > 0) && by_hdr != by_real {
        w.push("El recuento de retornos de la cabecera no coincide con el real.".into());
    }
    let out = r.z_outliers.0 + r.z_outliers.1;
    if out > 0 {
        w.push(format!(
            "Posible ruido: {} puntos muy por debajo y {} muy por encima del grueso de la nube (el 98 % está entre {:.1} y {:.1} m). Conviene filtrarlo antes de calcular nada.",
            r.z_outliers.0, r.z_outliers.1, r.z_percentiles[0], r.z_percentiles[6]
        ));
    }
    let only_unclassified = r.classes.iter().all(|(c, _)| *c <= 1);
    if only_unclassified {
        w.push("Ninguno de los puntos está clasificado: el suelo hay que calcularlo.".into());
    } else if !r.classes.iter().any(|(c, _)| *c == 2) {
        w.push("Hay clases, pero ninguna de suelo (2).".into());
    }
    if r.intensity.1 == 0 && r.points_read > 0 {
        w.push("La intensidad está vacía.".into());
    }
    if r.density > 0.0 && r.density < 1.0 {
        w.push(format!("Densidad baja ({:.2} puntos/m²): la detección de árboles individuales será poco fiable.", r.density));
    }
    if r.withheld > 0 {
        w.push(format!(
            "{} puntos marcados como retenidos (withheld); se ignoran.",
            r.withheld
        ));
    }
    if r.extra_bytes > 0 {
        w.push(format!(
            "Los registros llevan {} bytes extra por punto.",
            r.extra_bytes
        ));
    }
    w
}

/// A report in text, for the command line and logs.
pub fn render(r: &FileReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let mb = r.file_size as f64 / 1_048_576.0;
    let _ = writeln!(s, "{}", r.path.display());
    let _ = writeln!(
        s,
        "  Archivo: {:.1} MB · LAS {}.{} · formato {} ({}){}",
        mb,
        r.version.0,
        r.version.1,
        r.format,
        format_description(r.format),
        if r.compressed { " · LAZ" } else { "" }
    );
    if !r.software.is_empty() || !r.system_id.is_empty() {
        let _ = writeln!(s, "  Creado con: {} {}", r.software, r.system_id);
    }
    let _ = writeln!(s, "  Puntos: {}", r.points_read);
    let _ = writeln!(
        s,
        "  Extensión X: {:.2} a {:.2} ({:.1} m)",
        r.bounds[0],
        r.bounds[3],
        r.bounds[3] - r.bounds[0]
    );
    let _ = writeln!(
        s,
        "  Extensión Y: {:.2} a {:.2} ({:.1} m)",
        r.bounds[1],
        r.bounds[4],
        r.bounds[4] - r.bounds[1]
    );
    let _ = writeln!(s, "  Elevación Z: {:.2} a {:.2}", r.bounds[2], r.bounds[5]);
    let p = &r.z_percentiles;
    let _ = writeln!(
        s,
        "  Elevación percentiles 1/5/25/50/75/95/99: {:.1} / {:.1} / {:.1} / {:.1} / {:.1} / {:.1} / {:.1}",
        p[0], p[1], p[2], p[3], p[4], p[5], p[6]
    );
    let crs = match (&r.crs_name, r.epsg) {
        (Some(n), Some(e)) => format!("{n} (EPSG:{e})"),
        (Some(n), None) => n.clone(),
        (None, Some(e)) => format!("EPSG:{e}"),
        (None, None) => "no declarado".into(),
    };
    let _ = writeln!(s, "  Sistema de coordenadas: {crs}");
    let _ = writeln!(
        s,
        "  Densidad: {:.2} puntos/m² ({:.2} pulsos/m²), separación media {:.2} m, sobre {:.2} ha cubiertas de {:.2} ha de la caja",
        r.density,
        r.pulse_density,
        r.spacing,
        r.area_covered / 1e4,
        r.area_bbox / 1e4
    );
    let _ = writeln!(s, "  Escala {:?} · desplazamiento {:?}", r.scale, r.offset);
    let rets: Vec<String> = (1..=7)
        .filter(|&k| r.returns[k] > 0)
        .map(|k| format!("{k}: {}", r.returns[k]))
        .collect();
    let _ = writeln!(s, "  Retornos: {}", rets.join(" · "));
    let _ = writeln!(s, "  Clases:");
    for (c, n) in &r.classes {
        let _ = writeln!(
            s,
            "    {c:>3} {:<28} {:>10}  {:>5.1} %",
            class_name(*c),
            n,
            *n as f64 / r.points_read.max(1) as f64 * 100.0
        );
    }
    let _ = writeln!(
        s,
        "  Intensidad: {} a {} (media {:.0})",
        r.intensity.0, r.intensity.1, r.intensity.2
    );
    if let Some(d) = &r.capture_date {
        let _ = writeln!(s, "  Fecha de captura: {d}");
    }
    if let Some((a, b)) = r.gps_time {
        let _ = writeln!(s, "  Tiempo GPS: {a:.3} a {b:.3}");
    }
    let _ = writeln!(
        s,
        "  Ángulo de escaneo: {:.0}° a {:.0}°",
        r.scan_angle.0, r.scan_angle.1
    );
    let _ = writeln!(s, "  Líneas de vuelo: {}", r.flight_lines.len());
    if r.has_rgb {
        let _ = writeln!(s, "  Color RGB: sí (máximo {})", r.rgb_max);
    }
    if !r.index_files.is_empty() {
        let _ = writeln!(s, "  Archivos asociados: {}", r.index_files.join(", "));
    }
    if r.warnings.is_empty() {
        let _ = writeln!(s, "  Avisos: ninguno");
    } else {
        let _ = writeln!(s, "  Avisos:");
        for w in &r.warnings {
            let _ = writeln!(s, "    ! {w}");
        }
    }
    s
}

/// Reports on several files and their sum.
pub struct Summary {
    pub files: Vec<FileReport>,
    pub points: u64,
    pub bounds: [f64; 6],
    pub density: f64,
}

pub fn inspect_many(paths: &[PathBuf]) -> Result<Summary> {
    let files = paths
        .iter()
        .map(|p| inspect(p))
        .collect::<Result<Vec<_>>>()?;
    let points = files.iter().map(|f| f.points_read).sum();
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut area = 0.0;
    for f in &files {
        for k in 0..3 {
            bounds[k] = bounds[k].min(f.bounds[k]);
            bounds[k + 3] = bounds[k + 3].max(f.bounds[k + 3]);
        }
        area += f.area_covered;
    }
    Ok(Summary {
        files,
        points,
        bounds,
        density: if area > 0.0 {
            points as f64 / area
        } else {
            0.0
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epsg_is_read_from_wkt_of_either_style() {
        let wkt1 = r#"PROJCS["ETRS89 / UTM zone 29N",GEOGCS["ETRS89",AUTHORITY["EPSG","4258"]],AUTHORITY["EPSG","25829"]]"#;
        assert_eq!(las::epsg_from_wkt(wkt1), Some(25829));
        assert_eq!(
            las::wkt_name(wkt1).as_deref(),
            Some("ETRS89 / UTM zone 29N")
        );
        let wkt2 = r#"PROJCRS["ETRS89 / UTM zone 29N",BASEGEOGCRS["ETRS89",ID["EPSG",4258]],ID["EPSG",25829]]"#;
        assert_eq!(las::epsg_from_wkt(wkt2), Some(25829));
    }

    #[test]
    fn class_names_cover_the_standard_table() {
        assert_eq!(class_name(2), "Suelo");
        assert_eq!(class_name(70), "Definido por el usuario");
    }
}
