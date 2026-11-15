//! Volúmenes entre un ráster de superficie y una referencia, para todo el ráster o para regiones.
//!
//! La superficie es un MDS o un CHM; la referencia es un MDT, un nivel o nada (un CHM ya contiene
//! alturas sobre el suelo). Cada celda de superficie tiene `h = superficie - referencia` de alto:
//! las celdas al menos `min_height` por encima de la referencia suman `h * celda²` al volumen sobre
//! ella, las que están al menos tan por debajo suman al volumen bajo ella. Las regiones que se
//! solapan se miden una a una.

use std::path::{Path, PathBuf};

use crate::grid::Grid;
use crate::vector::{self, Region};
use crate::{Error, Result};

/// Contra qué se mide la superficie.
#[derive(Clone, Copy, Debug)]
pub enum Reference<'a> {
    /// La superficie ya contiene alturas (un modelo de altura de copas).
    Zero,
    /// Un plano horizontal a esta elevación.
    Level(f64),
    /// Un modelo del terreno, muestreado en el centro de cada celda de superficie.
    Dtm(&'a Grid),
}

/// La medida de una región.
#[derive(Clone, Debug, PartialEq)]
pub struct VolumeRow {
    pub name: String,
    /// Área (m²) de las celdas medidas: las que tienen valor de superficie y una referencia.
    pub area: f64,
    /// Volumen (m³) sobre la referencia y bajo ella.
    pub above: f64,
    pub below: f64,
    /// Altura media y mayor de las celdas por encima de la referencia (NaN cuando no hay ninguna).
    pub mean_h: f64,
    pub max_h: f64,
    /// Celdas con un valor de superficie que el MDT no cubre: no medidas.
    pub no_reference: u64,
}

impl VolumeRow {
    pub fn net(&self) -> f64 {
        self.above - self.below
    }
}

/// Columnas y filas de las celdas cuyos centros pueden estar dentro de `bbox` (x mínima, y mínima,
/// x máxima, y máxima).
fn window(g: &Grid, bbox: [f64; 4]) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let top = g.y0 + g.rows as f64 * g.cell;
    let lim = |v: f64, n: usize| v.clamp(0.0, n as f64) as usize;
    (
        lim(((bbox[0] - g.x0) / g.cell).floor(), g.cols)
            ..lim(((bbox[2] - g.x0) / g.cell).ceil(), g.cols),
        lim(((top - bbox[3]) / g.cell).floor(), g.rows)
            ..lim(((top - bbox[1]) / g.cell).ceil(), g.rows),
    )
}

/// Volúmenes de la superficie sobre `reference`, una fila por región (una fila para todo el ráster
/// cuando no hay ninguna). Las alturas más cercanas que `min_height` a la referencia cuentan como
/// nada.
pub fn volumes(
    surface: &Grid,
    reference: Reference,
    regions: &[Region],
    min_height: f64,
) -> Result<Vec<VolumeRow>> {
    if !(min_height >= 0.0) {
        return Err(Error::Param(
            "la altura mínima no puede ser negativa".into(),
        ));
    }
    let cell_area = surface.cell * surface.cell;
    let measure = |name: String, bbox: [f64; 4], inside: &dyn Fn(f64, f64) -> bool| {
        let (cols, rows) = window(surface, bbox);
        let mut row = VolumeRow {
            name,
            area: 0.0,
            above: 0.0,
            below: 0.0,
            mean_h: f64::NAN,
            max_h: f64::NAN,
            no_reference: 0,
        };
        let (mut n_above, mut sum_h) = (0u64, 0.0);
        for r in rows {
            for c in cols.clone() {
                let z = surface.data[r * surface.cols + c];
                if z.is_nan() {
                    continue;
                }
                let (x, y) = surface.center(c, r);
                if !inside(x, y) {
                    continue;
                }
                let base = match reference {
                    Reference::Zero => Some(0.0),
                    Reference::Level(l) => Some(l),
                    // `sample` limita al borde: fuera del MDT no hay referencia
                    Reference::Dtm(d) => d.index(x, y).and_then(|_| d.sample(x, y)).map(f64::from),
                };
                let Some(base) = base else {
                    row.no_reference += 1;
                    continue;
                };
                let h = f64::from(z) - base;
                row.area += cell_area;
                row.max_h = row.max_h.max(h);
                if h > 0.0 && h >= min_height {
                    row.above += h * cell_area;
                    n_above += 1;
                    sum_h += h;
                } else if h < 0.0 && -h >= min_height {
                    row.below -= h * cell_area;
                }
            }
        }
        if n_above > 0 {
            row.mean_h = sum_h / n_above as f64;
        }
        row
    };
    if regions.is_empty() {
        let b = [
            surface.x0,
            surface.y0,
            surface.x0 + surface.cols as f64 * surface.cell,
            surface.y0 + surface.rows as f64 * surface.cell,
        ];
        return Ok(vec![measure("Total".into(), b, &|_, _| true)]);
    }
    let mut out = Vec::with_capacity(regions.len());
    for r in regions {
        crate::check_cancel()?;
        out.push(measure(r.name.clone(), r.bbox(), &|x, y| r.contains(x, y)));
    }
    Ok(out)
}

/// De dónde viene la referencia de una ejecución.
#[derive(Clone, Debug)]
pub enum ReferenceSource {
    Zero,
    Level(f64),
    Dtm(PathBuf),
}

#[derive(Clone, Debug)]
pub struct VolumeJob {
    pub surface: PathBuf,
    pub reference: ReferenceSource,
    /// Vacío: todo el ráster.
    pub regions: Vec<Region>,
    pub min_height: f64,
    /// Nombre de la hoja (nube) a la que pertenecen los rásteres.
    pub sheet: String,
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Las filas como tabla CSV; con `sheets` (un nombre por fila), una primera columna dice la hoja.
pub fn csv_text(rows: &[VolumeRow], sheets: Option<&[String]>) -> String {
    let num = |v: f64| {
        if v.is_finite() {
            format!("{v:.3}")
        } else {
            String::new()
        }
    };
    let mut s = String::from(if sheets.is_some() { "Hoja," } else { "" });
    s.push_str(
        "Region,Area_m2,Vol_sobre_m3,Vol_bajo_m3,Vol_neto_m3,H_media_m,H_max_m,Celdas_sin_MDT\n",
    );
    for (k, r) in rows.iter().enumerate() {
        if let Some(names) = sheets {
            s.push_str(&csv_field(names.get(k).map_or("", String::as_str)));
            s.push(',');
        }
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_field(&r.name),
            num(r.area),
            num(r.above),
            num(r.below),
            num(r.net()),
            num(r.mean_h),
            num(r.max_h),
            r.no_reference
        ));
    }
    s
}

/// Mide los volúmenes de cada trabajo y escribe una tabla con todas las filas (una columna `Hoja`
/// dice la hoja cuando hay varias); una línea sobre el resultado.
pub fn run_all(jobs: &[VolumeJob], output: &Path) -> Result<String> {
    let (mut rows, mut names) = (Vec::new(), Vec::new());
    for job in jobs {
        crate::check_cancel()?;
        let surface = Grid::read(&job.surface)?;
        let dtm;
        let reference = match &job.reference {
            ReferenceSource::Zero => Reference::Zero,
            ReferenceSource::Level(l) => Reference::Level(*l),
            ReferenceSource::Dtm(p) => {
                dtm = Grid::read(p)?;
                Reference::Dtm(&dtm)
            }
        };
        let part = volumes(&surface, reference, &job.regions, job.min_height)?;
        names.extend(part.iter().map(|_| job.sheet.clone()));
        rows.extend(part);
    }
    crate::paths::ensure_parent(output)?;
    std::fs::write(
        output,
        csv_text(&rows, (jobs.len() > 1).then_some(&names[..])),
    )?;
    Ok(summary(&rows, output))
}

/// Los nombres de los campos de volumen de una capa de polígonos, en el orden de los valores de
/// [`VolumeRow`].
pub const LAYER_FIELDS: [&str; 7] = ["Area_m2", "Vol_sobre", "Vol_bajo", "Vol_neto", "H_media", "H_max", "Sin_MDT"];

/// Mide los volúmenes de los polígonos del archivo `source` y los escribe como capa de polígonos:
/// los polígonos con sus propios atributos más los campos de volumen (un campo de la capa con el
/// mismo nombre se sustituye). Una línea sobre el resultado.
pub fn run_layer(job: &VolumeJob, source: &Path, output: &Path) -> Result<String> {
    use crate::geomops::{self, Feature, PolyLayer};
    crate::check_cancel()?;
    let surface = Grid::read(&job.surface)?;
    let dtm;
    let reference = match &job.reference {
        ReferenceSource::Zero => Reference::Zero,
        ReferenceSource::Level(l) => Reference::Level(*l),
        ReferenceSource::Dtm(p) => {
            dtm = Grid::read(p)?;
            Reference::Dtm(&dtm)
        }
    };
    let rows = volumes(&surface, reference, &job.regions, job.min_height)?;
    if rows.len() != job.regions.len() {
        return Err(Error::Param("las filas de volumen no corresponden a los polígonos".into()));
    }
    let num = |v: f64| if v.is_finite() { format!("{v:.3}") } else { String::new() };
    let features = job
        .regions
        .iter()
        .zip(&rows)
        .map(|(region, r)| {
            let mut attrs: Vec<(String, String)> = region
                .attrs
                .iter()
                .filter(|(k, _)| !LAYER_FIELDS.iter().any(|f| f.eq_ignore_ascii_case(k)))
                .cloned()
                .collect();
            let values = [num(r.area), num(r.above), num(r.below), num(r.net()), num(r.mean_h), num(r.max_h), r.no_reference.to_string()];
            attrs.extend(LAYER_FIELDS.iter().map(|f| f.to_string()).zip(values));
            Feature { geom: geomops::multipolygon_of(&geomops::rings_of(region)), attrs }
        })
        .collect();
    let prj = std::fs::read_to_string(source.with_extension("prj"))
        .ok()
        .or_else(|| crate::crs::epsg_of(source).and_then(vector::prj_for_epsg));
    crate::paths::ensure_parent(output)?;
    geomops::write_polygons(&PolyLayer { features, prj }, output)?;
    Ok(summary(&rows, output))
}

/// Una línea sobre un conjunto de filas.
pub fn summary(rows: &[VolumeRow], out: &Path) -> String {
    let (above, below, area): (f64, f64, f64) = rows.iter().fold((0.0, 0.0, 0.0), |a, r| {
        (a.0 + r.above, a.1 + r.below, a.2 + r.area)
    });
    let skipped: u64 = rows.iter().map(|r| r.no_reference).sum();
    let mut text = format!(
        "{} región(es), {area:.1} m²: {above:.3} m³ sobre la referencia, {below:.3} m³ bajo ella",
        rows.len()
    );
    if skipped > 0 {
        text.push_str(&format!(" ({skipped} celdas sin MDT no se midieron)"));
    }
    format!("{text}: {}", out.display())
}

/// Campos que identifican una región, por orden de preferencia; el primero que tiene el archivo
/// nombra las filas, de modo que una tabla de copas de árboles conserva el `TREEID` de los árboles
/// y se enlaza con su tabla.
const NAME_FIELDS: &[&str] = &["treeid", "tree_id", "id", "fid", "nombre", "name"];

/// Lee regiones de un archivo vectorial para una ejecución de volúmenes.
pub fn read_regions(path: &Path) -> Result<Vec<Region>> {
    let name_field = shapefile::dbase::Reader::from_path(path.with_extension("dbf"))
        .ok()
        .and_then(|r| {
            let names: Vec<String> = r.fields().iter().map(|f| f.name().to_string()).collect();
            NAME_FIELDS
                .iter()
                .find_map(|w| names.iter().find(|n| n.eq_ignore_ascii_case(w)).cloned())
        });
    let opts = vector::ReadOptions {
        name_field,
        ..Default::default()
    };
    vector::read_regions(path, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::Shape2;

    fn grid(z: f32) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 2.0, 4, 4)?;
        g.data.fill(z);
        Ok(g)
    }

    #[test]
    fn a_plateau_over_a_flat_dtm() -> Result<()> {
        // 16 celdas de 4 m² a 12 m sobre un terreno a 10 m: 64 m² x 2 m
        let rows = volumes(&grid(12.0)?, Reference::Dtm(&grid(10.0)?), &[], 0.0)?;
        assert_eq!(rows.len(), 1);
        assert!((rows[0].area - 64.0).abs() < 1e-9);
        assert!((rows[0].above - 128.0).abs() < 1e-9);
        assert_eq!(rows[0].below, 0.0);
        assert!((rows[0].mean_h - 2.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn heights_below_the_reference_and_the_minimum_height() -> Result<()> {
        let rows = volumes(&grid(9.0)?, Reference::Level(10.0), &[], 0.0)?;
        assert!((rows[0].below - 64.0).abs() < 1e-9 && rows[0].above == 0.0);
        // una superficie de 2 m bajo una altura mínima de 3 m cuenta como nada
        let rows = volumes(&grid(2.0)?, Reference::Zero, &[], 3.0)?;
        assert_eq!(rows[0].above, 0.0);
        assert!(rows[0].mean_h.is_nan());
        Ok(())
    }

    #[test]
    fn a_region_takes_only_its_cells() -> Result<()> {
        // la mitad izquierda del ráster: x de 0 a 4
        let left = Region {
            name: "izquierda".into(),
            shape: Shape2::Rect([0.0, 0.0, 4.0, 8.0]),
            attrs: vec![],
        };
        let rows = volumes(&grid(5.0)?, Reference::Zero, &[left], 0.0)?;
        assert!((rows[0].area - 32.0).abs() < 1e-9);
        assert!((rows[0].above - 160.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn cells_outside_the_dtm_are_counted_apart() -> Result<()> {
        let mut small = Grid::new(0.0, 0.0, 2.0, 2, 2)?;
        small.data.fill(0.0);
        // el MDT cubre solo el cuarto suroeste
        let rows = volumes(&grid(1.0)?, Reference::Dtm(&small), &[], 0.0)?;
        assert!(rows[0].no_reference > 0);
        assert!(rows[0].area < 64.0);
        Ok(())
    }

    #[test]
    fn the_volumes_of_a_polygon_file_land_in_its_table() -> Result<()> {
        use crate::geomops::{self, Feature, PolyLayer};
        let dir = std::env::temp_dir().join(format!("hp_vol_layer_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let surf = dir.join("chm.tif");
        grid(5.0)?.write_tiff(&surf, None)?;
        let src = dir.join("zonas.shp");
        let sq = |x: f64| vec![[x, 0.0], [x, 4.0], [x + 4.0, 4.0], [x + 4.0, 0.0]];
        let layer = PolyLayer {
            features: vec![
                Feature { geom: geomops::multipolygon_of(&[sq(0.0)]), attrs: vec![("zona".into(), "A".into())] },
                Feature { geom: geomops::multipolygon_of(&[sq(4.0)]), attrs: vec![("zona".into(), "B".into())] },
            ],
            prj: None,
        };
        geomops::write_polygons(&layer, &src)?;
        let regions = read_regions(&src)?;
        let job = VolumeJob { surface: surf, reference: ReferenceSource::Zero, regions, min_height: 0.0, sheet: "h".into() };
        let out = dir.join("zonas_volumen.shp");
        run_layer(&job, &src, &out)?;
        let back = geomops::read_polygons(&out)?;
        assert_eq!(back.features.len(), 2);
        let get = |i: usize, k: &str| back.features[i].attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get(0, "zona").as_deref(), Some("A"));
        // 4 celdas de 4 m² a 5 m
        assert_eq!(get(0, "Vol_sobre").and_then(|v| v.parse::<f64>().ok()), Some(80.0));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn csv_quotes_names() {
        let row = VolumeRow {
            name: "a,b".into(),
            area: 1.0,
            above: 2.0,
            below: 0.5,
            mean_h: f64::NAN,
            max_h: 1.0,
            no_reference: 0,
        };
        let t = csv_text(&[row], None);
        assert!(t.contains("\"a,b\",1.000,2.000,0.500,1.500,,1.000,0"));
    }
}
