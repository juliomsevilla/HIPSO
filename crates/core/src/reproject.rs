//! Moves clouds, rasters and regions from one coordinate system to another, with the
//! transformations of `rustlidar-crs` (the datum shift grids of Spain are built in).
//!
//! Only the horizontal position changes; heights are kept (a datum shift changes them by
//! centimetres at most, and a geoid change is not a coordinate-system change).

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rustlidar_crs::SpatialRef;
pub use rustlidar_crs::Transformer;

use crate::grid::Grid;
use crate::las::{self, Reader, Writer};
use crate::vector::{Polygon, Region, Shape2};
use crate::{Error, Result};

/// The transformer between two EPSG codes.
pub fn transformer(from: u16, to: u16) -> Result<Transformer> {
    let parse = |c: u16| {
        SpatialRef::parse(&format!("EPSG:{c}"))
            .map_err(|e| Error::Param(format!("el sistema EPSG:{c} no se reconoce: {e}")))
    };
    Transformer::new(&parse(from)?, &parse(to)?)
        .map_err(|e| Error::Param(format!("no se puede pasar de EPSG:{from} a EPSG:{to}: {e}")))
}

fn geographic(code: u16) -> bool {
    (4000..5000).contains(&code)
}

/// Writes `src` in the system `to`, with the points' order, attributes and records unchanged.
/// Points that cannot be transformed are dropped; how many were read and dropped is returned.
pub fn cloud(src: &Path, dst: &Path, from: u16, to: u16) -> Result<(u64, u64)> {
    if geographic(from) || geographic(to) {
        return Err(Error::Unsupported(
            "las nubes no se reproyectan desde o hacia coordenadas geográficas (grados): usa un sistema proyectado".into(),
        ));
    }
    let t = transformer(from, to)?;
    let r = Reader::open(src)?;
    let b = r.header.bounds;
    let (x0, y0, x1, y1) = t
        .transform_bounds((b[0], b[1], b[3], b[4]))
        .map_err(|e| Error::Param(format!("la nube cae fuera del sistema EPSG:{to}: {e}")))?;
    // stored relative to a round value near the middle, as the new coordinates are far from the old
    let round = |v: f64| (v / 1000.0).round() * 1000.0;
    let offset = [
        round((x0 + x1) / 2.0),
        round((y0 + y1) / 2.0),
        r.header.offset[2],
    ];
    let out = Reader::open(src)?
        .with_projection(&las::epsg_record(to), 1, Some(to), None)
        .with_offset(offset);
    let (hin, hout) = (r.header.clone(), out.header.clone());
    let rec_len = usize::from(hin.record_len);
    let mut w = Writer::create(dst, &out)?;
    let dropped = std::sync::atomic::AtomicU64::new(0);
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
                match t.transform(x, y) {
                    Ok((nx, ny)) => las::set_xyz(&hout, rec, nx, ny, z),
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
    w.finish()?;
    Ok((read, dropped.into_inner()))
}

/// The clouds of a work, all in the system of the first one that declares it.
#[derive(Debug, Default)]
pub struct ProjectClouds {
    /// The files to load: the originals, or their converted copies, in order and without repeats.
    pub files: Vec<PathBuf>,
    /// What happened, for the log.
    pub notes: Vec<String>,
    /// How many files had to be converted (or found already converted).
    pub moved: usize,
}

/// The system the work is in, and how many of `paths` are in another one (so the caller can say
/// that loading will take longer before it starts). Geographic clouds count too.
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

/// Puts the clouds in the system of the first one that declares it. Those in another system are
/// converted into a LAZ copy next to them (`<name>_epsg<code>.laz`, in the results folder), which
/// later loads find ready as long as it is newer than the original. Clouds that cannot be
/// converted are left out, and the notes say why.
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

/// The grid in the system `to`, if it is in another one (`from`): `None` when there is nothing to do.
pub fn grid_if_needed(g: &Grid, from: Option<u16>, to: Option<u16>) -> Result<Option<Grid>> {
    match (from, to) {
        (Some(f), Some(t)) if crate::crs::relation(t, f) == crate::crs::Relation::Different => {
            grid(g, f, t).map(Some)
        }
        _ => Ok(None),
    }
}

/// Moves the vertices of a vector layer to the system `to`; the bounds are worked out again.
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

/// A raster holding only whole numbers (classes, labels) must not be blurred by interpolation.
fn categorical(g: &Grid) -> bool {
    let step = (g.data.len() / 20_000).max(1);
    g.data
        .iter()
        .step_by(step)
        .all(|v| v.is_nan() || v.fract() == 0.0)
}

/// The raster in the system `to`: the same cell size when the units agree (to 1%: between UTM
/// zones the scale differs by a few parts per thousand, and round cells are worth that), the
/// cells aligned on multiples of it, values interpolated (or taken from the nearest cell, for
/// rasters of whole numbers).
pub fn grid(g: &Grid, from: u16, to: u16) -> Result<Grid> {
    let fwd = transformer(from, to)?;
    let (w, h) = (g.cols as f64 * g.cell, g.rows as f64 * g.cell);
    let (x0, y0, x1, y1) = fwd
        .transform_bounds((g.x0, g.y0, g.x0 + w, g.y0 + h))
        .map_err(|e| Error::Param(format!("el ráster cae fuera del sistema EPSG:{to}: {e}")))?;
    // the cell of the new system: the old one's size where the two systems share units
    let (cx, cy) = (g.x0 + w / 2.0, g.y0 + h / 2.0);
    let (ax, ay) = fwd
        .transform(cx, cy)
        .map_err(|e| Error::Param(e.to_string()))?;
    let (bx, by) = fwd
        .transform(cx + g.cell, cy)
        .map_err(|e| Error::Param(e.to_string()))?;
    let step = (bx - ax).hypot(by - ay);
    let cell = if (step / g.cell - 1.0).abs() < 1e-2 {
        g.cell
    } else {
        step
    };
    grid_onto(g, from, to, Grid::covering([x0, y0, x1, y1], cell)?)
}

/// The raster on the cells of `onto` (an empty grid, in the system `to`): what a model must be
/// to be combined cell by cell with another one.
pub fn grid_onto(g: &Grid, from: u16, to: u16, mut out: Grid) -> Result<Grid> {
    let inv = transformer(to, from)?;
    let nearest = categorical(g);
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
                            // `sample` clamps to the edge: outside the raster there is no value
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

/// Writes `input` (a cloud or a raster) in the system `to`. `from` says what the file is in when
/// it does not declare it. Returns a line for the report.
pub fn file(input: &Path, output: &Path, to: u16, from: Option<u16>) -> Result<String> {
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
        let (read, dropped) = cloud(input, output, from, to)?;
        let tail = if dropped > 0 {
            format!("; {dropped} puntos fuera del sistema de destino, descartados")
        } else {
            String::new()
        };
        (format!("{read} puntos"), tail)
    } else if crate::raster::is_raster_ext(&ext) {
        let g = Grid::read(input)?;
        let moved = grid(&g, from, to)?;
        let out = output.with_extension("tif");
        moved.write_tiff(&out, Some(to))?;
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

/// The regions in the system `to`. Rectangles become polygons (they are no longer axis-aligned);
/// a circle keeps its centre's image and its radius, whose distortion is parts per thousand.
pub fn regions(list: &[Region], from: u16, to: u16) -> Result<Vec<Region>> {
    let t = transformer(from, to)?;
    let bad = |e: rustlidar_crs::Error| {
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

    fn dir(tag: &str) -> Result<PathBuf> {
        let d = std::env::temp_dir().join(format!("rl_reproj_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    /// A LAS 1.2 file of format 1 with the points, declared to be in `epsg`.
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
        // offset near the points, scale 1 mm
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

    fn read_xyz(path: &Path) -> Result<Vec<[f64; 3]>> {
        let r = Reader::open(path)?;
        let mut v = Vec::new();
        r.scan(|p| v.push([p.x, p.y, p.z]))?;
        Ok(v)
    }

    #[test]
    fn a_cloud_moves_between_utm_zones_and_datums() -> Result<()> {
        let d = dir("cloud")?;
        // around Madrid, in ED50 UTM 30N
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
        // the known shift of ED50 to ETRS89 in Madrid: about -109 m east, -207 m north
        assert!((out[0][0] - pts[0][0] + 109.4).abs() < 1.0, "{:?}", out[0]);
        assert!((out[0][1] - pts[0][1] + 207.5).abs() < 1.0, "{:?}", out[0]);
        std::fs::remove_dir_all(d)?;
        Ok(())
    }

    #[test]
    fn a_raster_keeps_its_surface_across_zones() -> Result<()> {
        // a plane in UTM 29N, over Galicia, moved to UTM 30N
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
        // already in one system: nothing is converted
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
        // a cloud in ED50 UTM 30N whose ETRS89 coordinates are a regular lattice
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
}
