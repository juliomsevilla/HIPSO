//! Huecos del dosel: las manchas conectadas de un modelo de altura de copas (CHM) por debajo de una
//! altura. Cada hueco se mide (superficie, perímetro, forma, alturas del CHM dentro, centro) y se
//! guarda como polígono; además se resume qué fracción del dosel es hueco.
//!
//! Un hueco que toca el borde del ráster o una celda sin dato está cortado y su tamaño real no se
//! conoce, por lo que se deja fuera salvo que se pida lo contrario. Las celdas sin dato nunca son
//! hueco. La conectividad es la de las aristas (4 vecinas).

use std::path::PathBuf;

use rayon::prelude::*;

use crate::Error;
use crate::grid::Grid;
use crate::obia::segment::label_components;
use crate::obia::vectorize::Tracer;
use crate::obia::{Labels, Lattice};
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Result, paths};

pub struct GapJob {
    /// El CHM (un GeoTIFF en un sistema proyectado, celda en metros).
    pub input: PathBuf,
    /// Una celda con una altura menor que esta (m) está en un hueco.
    pub height: f64,
    /// Los huecos menores que esto (m²) se descartan.
    pub min_area: f64,
    /// Los huecos mayores que esto (m²) se descartan: no son huecos del dosel sino claros.
    pub max_area: f64,
    /// Conserva los huecos cortados por el borde del ráster o por celdas sin dato.
    pub include_edge: bool,
    /// Ruta base de los resultados.
    pub output: PathBuf,
    pub raster: bool,
    pub gpkg: bool,
}

impl GapJob {
    pub fn new(input: PathBuf, output: PathBuf) -> Self {
        Self {
            input,
            height: 5.0,
            min_area: 1.0,
            max_area: 10_000.0,
            include_edge: false,
            output,
            raster: false,
            gpkg: false,
        }
    }
}

/// Lo que se mide de un hueco.
#[derive(Clone, Debug, PartialEq)]
pub struct Gap {
    pub id: u32,
    pub cells: u64,
    pub area: f64,
    pub perimeter: f64,
    /// Perímetro / (2 √(π área)): 1 para un círculo, más cuanto más alargado o recortado.
    pub shape: f64,
    /// Diámetro del círculo de la misma superficie.
    pub diameter: f64,
    pub mean_h: f64,
    pub min_h: f64,
    pub max_h: f64,
    pub x: f64,
    pub y: f64,
}

pub struct Gaps {
    pub labels: Labels,
    pub list: Vec<Gap>,
    /// Superficie (m²) de las celdas con dato.
    pub valid_area: f64,
    /// Superficie (m²) de todas las celdas por debajo de la altura, de cualquier tamaño.
    pub low_area: f64,
}

/// Halla y mide los huecos de un CHM en memoria.
pub fn find(g: &Grid, job: &GapJob) -> Result<Gaps> {
    if !(g.cell > 1e-3) {
        return Err(Error::Param(
            "la celda mide menos de 1 mm: el ráster parece estar en grados; reproyéctalo a un sistema métrico (UTM)".into(),
        ));
    }
    if !job.height.is_finite() || !(job.min_area >= 0.0) || !(job.max_area >= job.min_area) {
        return Err(Error::Param(
            "la altura debe ser un número y el área máxima no puede ser menor que la mínima".into(),
        ));
    }
    let (cols, rows) = (g.cols, g.rows);
    let h = job.height as f32;
    let low = |i: usize| {
        let v = g.data[i];
        v.is_finite() && v < h
    };
    let (ids, count) = label_components(cols, rows, false, low, |_, _| true)?;
    let cell2 = g.cell * g.cell;
    #[derive(Clone)]
    struct Sum {
        cells: u64,
        edge: bool,
        perim: u64,
        sum: f64,
        min: f32,
        max: f32,
        sx: f64,
        sy: f64,
    }
    let mut sums = vec![
        Sum {
            cells: 0,
            edge: false,
            perim: 0,
            sum: 0.0,
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
            sx: 0.0,
            sy: 0.0
        };
        count as usize + 1
    ];
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            let id = ids[i] as usize;
            if id == 0 {
                continue;
            }
            let s = &mut sums[id];
            s.cells += 1;
            let v = g.data[i];
            s.sum += f64::from(v);
            s.min = s.min.min(v);
            s.max = s.max.max(v);
            let (x, y) = g.center(c, r);
            s.sx += x;
            s.sy += y;
            // cada lado que da a otra cosa que no sea el mismo hueco es perímetro; si da al borde
            // o a una celda sin dato, el hueco está cortado
            for (dc, dr) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (nc, nr) = (c as i64 + dc, r as i64 + dr);
                if nc < 0 || nr < 0 || nc >= cols as i64 || nr >= rows as i64 {
                    s.perim += 1;
                    s.edge = true;
                    continue;
                }
                let j = nr as usize * cols + nc as usize;
                if ids[j] as usize != id {
                    s.perim += 1;
                    if !g.data[j].is_finite() {
                        s.edge = true;
                    }
                }
            }
        }
    }
    let valid_area = g.data.iter().filter(|v| v.is_finite()).count() as f64 * cell2;
    let low_area = sums.iter().map(|s| s.cells).sum::<u64>() as f64 * cell2;
    let mut renumber = vec![0u32; count as usize + 1];
    let mut list = Vec::new();
    for (id, s) in sums.iter().enumerate().skip(1) {
        let area = s.cells as f64 * cell2;
        if area < job.min_area || area > job.max_area || (s.edge && !job.include_edge) {
            continue;
        }
        let number = list.len() as u32 + 1;
        renumber[id] = number;
        let perimeter = s.perim as f64 * g.cell;
        list.push(Gap {
            id: number,
            cells: s.cells,
            area,
            perimeter,
            shape: perimeter / (2.0 * (std::f64::consts::PI * area).sqrt()),
            diameter: 2.0 * (area / std::f64::consts::PI).sqrt(),
            mean_h: s.sum / s.cells as f64,
            min_h: f64::from(s.min),
            max_h: f64::from(s.max),
            x: s.sx / s.cells as f64,
            y: s.sy / s.cells as f64,
        });
    }
    let ids: Vec<u32> = ids.iter().map(|&l| renumber[l as usize]).collect();
    let labels = Labels {
        lat: Lattice::of(g),
        count: list.len() as u32,
        ids,
    };
    Ok(Gaps {
        labels,
        list,
        valid_area,
        low_area,
    })
}

fn layer(p: &Gaps) -> Result<VectorData> {
    let tracer = Tracer::new(&p.labels)?;
    let outlines: Vec<_> = (1..=p.labels.count)
        .into_par_iter()
        .map(|id| tracer.outline(id, 0.0))
        .collect();
    let fields: Vec<String> = [
        "id", "area_m2", "perim_m", "forma", "diam_m", "h_media", "h_min", "h_max", "x", "y",
    ]
    .map(String::from)
    .to_vec();
    let mut d = VectorData {
        fields,
        polygons: true,
        bounds: [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        ..Default::default()
    };
    for (gap, o) in p.list.iter().zip(outlines) {
        let mut parts = o.outers;
        parts.extend(o.holes);
        for q in parts.iter().flatten() {
            d.bounds = [
                d.bounds[0].min(q[0]),
                d.bounds[1].min(q[1]),
                d.bounds[2].max(q[0]),
                d.bounds[3].max(q[1]),
            ];
        }
        let values = vec![
            gap.id.to_string(),
            format!("{:.2}", gap.area),
            format!("{:.2}", gap.perimeter),
            format!("{:.3}", gap.shape),
            format!("{:.2}", gap.diameter),
            format!("{:.3}", gap.mean_h),
            format!("{:.3}", gap.min_h),
            format!("{:.3}", gap.max_h),
            format!("{:.2}", gap.x),
            format!("{:.2}", gap.y),
        ];
        d.num.push(
            values
                .iter()
                .map(|t| t.parse().unwrap_or(f64::NAN))
                .collect(),
        );
        d.text.push(values);
        d.features.push(Feature { parts });
    }
    Ok(d)
}

/// Halla los huecos del CHM de `job.input` y escribe lo pedido; devuelve el resumen.
pub fn run(job: &GapJob) -> Result<String> {
    let g = Grid::read(&job.input)?;
    let epsg = Grid::read_epsg(&job.input);
    let gaps = find(&g, job)?;
    paths::ensure_parent(&job.output)?;
    let total: f64 = gaps.list.iter().map(|x| x.area).sum();
    let mut areas: Vec<f64> = gaps.list.iter().map(|x| x.area).collect();
    areas.sort_by(f64::total_cmp);
    let median = if areas.is_empty() {
        f64::NAN
    } else {
        areas[areas.len() / 2]
    };
    let ha = gaps.valid_area / 10_000.0;
    let mut lines = vec![format!(
        "{} huecos de entre {} y {} m² con el dosel por debajo de {} m: {:.1} m² ({:.2} % del terreno con dato; {:.2} % si se cuentan todas las celdas bajas); mediana {:.1} m²; {:.2} huecos/ha",
        gaps.list.len(),
        job.min_area,
        job.max_area,
        job.height,
        total,
        100.0 * total / gaps.valid_area.max(1e-9),
        100.0 * gaps.low_area / gaps.valid_area.max(1e-9),
        median,
        gaps.list.len() as f64 / ha.max(1e-9)
    )];
    let data = layer(&gaps)?;
    let prj = epsg.and_then(crate::vector::prj_for_epsg);
    let shp = paths::swap_ext(&job.output, "shp");
    crate::vector::write_layer(&shp, &data, GeomKind::Polygon, prj.as_deref())?;
    lines.push(format!("  huecos (polígonos): {}", shp.display()));
    if job.gpkg {
        let path = paths::swap_ext(&job.output, "gpkg");
        crate::gpkg_write::write(
            &path,
            &[crate::gpkg_write::Table {
                name: "huecos",
                data: &data,
                kind: GeomKind::Polygon,
                epsg,
            }],
        )?;
        lines.push(format!("  huecos (GeoPackage): {}", path.display()));
    }
    if job.raster {
        let path = paths::swap_ext(&job.output, "tif");
        gaps.labels.write_tiff(&path, epsg)?;
        lines.push(format!("  huecos (ráster): {}", path.display()));
    }
    let csv = paths::swap_ext(&job.output, "csv");
    let mut text =
        String::from("ID,Area_m2,Perimetro_m,Forma,Diametro_m,H_media,H_min,H_max,X,Y\n");
    for x in &gaps.list {
        text.push_str(&format!(
            "{},{:.2},{:.2},{:.3},{:.2},{:.3},{:.3},{:.3},{:.2},{:.2}\n",
            x.id, x.area, x.perimeter, x.shape, x.diameter, x.mean_h, x.min_h, x.max_h, x.x, x.y
        ));
    }
    std::fs::write(&csv, text)?;
    lines.push(format!("  tabla: {}", csv.display()));
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chm(cols: usize, rows: usize, f: impl Fn(usize, usize) -> f32) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 1.0, cols, rows)?;
        for r in 0..rows {
            for c in 0..cols {
                g.data[r * cols + c] = f(c, r);
            }
        }
        Ok(g)
    }

    fn job() -> GapJob {
        GapJob::new(PathBuf::new(), PathBuf::new())
    }

    #[test]
    fn a_square_hole_has_its_area_and_perimeter() -> Result<()> {
        // 12 x 12 de dosel de 20 m con un hueco de 3 x 4 en medio
        let g = chm(12, 12, |c, r| {
            if (4..7).contains(&c) && (5..9).contains(&r) {
                0.5
            } else {
                20.0
            }
        })?;
        let gaps = find(&g, &job())?;
        assert_eq!(gaps.list.len(), 1);
        let x = &gaps.list[0];
        assert_eq!(x.area, 12.0);
        assert_eq!(x.perimeter, 14.0);
        assert!((x.mean_h - 0.5).abs() < 1e-9);
        assert!((gaps.valid_area - 144.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn edge_holes_are_cut_and_dropped_unless_asked() -> Result<()> {
        let g = chm(10, 10, |c, _| if c < 3 { 0.0 } else { 20.0 })?;
        assert!(find(&g, &job())?.list.is_empty());
        let mut j = job();
        j.include_edge = true;
        assert_eq!(find(&g, &j)?.list.len(), 1);
        Ok(())
    }

    #[test]
    fn nodata_is_never_a_gap_and_cuts_the_holes_it_touches() -> Result<()> {
        let g = chm(10, 10, |c, r| match (c, r) {
            (4, 4) => f32::NAN,
            (3..=5, 3..=5) => 0.0,
            _ => 20.0,
        })?;
        assert!(find(&g, &job())?.list.is_empty());
        Ok(())
    }

    #[test]
    fn size_limits_apply() -> Result<()> {
        let g = chm(12, 12, |c, r| {
            if (4..7).contains(&c) && (5..9).contains(&r) {
                0.5
            } else {
                20.0
            }
        })?;
        let mut j = job();
        j.min_area = 13.0;
        assert!(find(&g, &j)?.list.is_empty());
        Ok(())
    }
}
