//! Stockpile volumes: how much material there is over a base surface inside each pile's outline.
//!
//! The surface (a DSM) is measured against a base that is built for each pile in one of several
//! ways: a fixed level, the lowest or the median height along the outline, a least-squares plane
//! through the outline, a surface interpolated from the outline, or a terrain model of the site
//! before the pile. The measure itself is [`crate::volume::volumes`], which decides by the
//! centre of each cell: the volume of a pile has an error of the order of the perimeter times
//! half a cell times the height at the edge, and it is smaller with finer cells.
//!
//! The base is only as good as the outline: samples of the surface along it are taken as ground,
//! so an outline drawn through the pile (instead of around its foot) lifts the base.

use std::path::PathBuf;

use crate::geomops::{self, Feature, PolyLayer};
use crate::grid::Grid;
use crate::terrain::{self, Edges};
use crate::vector::{self, Region};
use crate::volume::{self, Reference};
use crate::{Error, Result};

/// How the base surface under a pile is made.
#[derive(Clone, Debug, PartialEq)]
pub enum Base {
    /// A horizontal plane at this elevation.
    Level(f64),
    /// The lowest height of the surface along the outline, as a horizontal plane.
    LowestPerimeter,
    /// The median height of the surface along the outline, as a horizontal plane.
    MedianPerimeter,
    /// The plane that best fits the heights along the outline (least squares).
    Plane,
    /// A surface interpolated from the heights along the outline (triangulated).
    Interpolated,
    /// A terrain model of the site before the pile.
    Dtm(PathBuf),
}

pub struct PileJob {
    pub surface: PathBuf,
    /// The outlines: a polygon layer.
    pub piles: PathBuf,
    pub base: Base,
    /// Heights closer than this to the base count as nothing (m).
    pub min_height: f64,
    /// Density of the material (t/m³); with it each pile has a mass.
    pub density: Option<f64>,
    /// Base path of the results; the layer is `<output>.shp`.
    pub output: PathBuf,
    pub csv: bool,
    /// Rasters with the base surface and the height over it, inside the outlines.
    pub base_raster: bool,
    pub height_raster: bool,
}

impl PileJob {
    pub fn new(surface: PathBuf, piles: PathBuf, output: PathBuf) -> Self {
        Self {
            surface,
            piles,
            base: Base::Plane,
            min_height: 0.0,
            density: None,
            output,
            csv: true,
            base_raster: false,
            height_raster: false,
        }
    }
}

/// The measure of one pile.
#[derive(Clone, Debug, PartialEq)]
pub struct PileRow {
    pub id: usize,
    pub name: String,
    /// Area inside the outline that has surface and base (m²), and the same on the slope (m²).
    pub area_2d: f64,
    pub area_3d: f64,
    /// Mean height of the base under the pile.
    pub base_z: f64,
    /// Volume over the base, and the part of the surface that is under it (m³).
    pub volume: f64,
    pub below: f64,
    pub mean_h: f64,
    pub max_h: f64,
    pub perimeter: f64,
    pub centroid: [f64; 2],
    /// Tonnes, when a density was given.
    pub mass: Option<f64>,
    /// Cells of the outline that the base could not be made for (not measured).
    pub no_base: u64,
}

/// The surface along the rings of `region`, every half cell; points outside the raster or in
/// holes are left out.
fn perimeter(surface: &Grid, region: &Region) -> Vec<[f64; 3]> {
    let step = (surface.cell / 2.0).max(1e-9);
    let mut out = Vec::new();
    for ring in geomops::rings_of(region) {
        let n = ring.len();
        for k in 0..n {
            let (a, b) = (ring[k], ring[(k + 1) % n]);
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            let m = ((len / step).ceil() as usize).max(1);
            for s in 0..m {
                let t = s as f64 / m as f64;
                let (x, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                if surface.index(x, y).is_some()
                    && let Some(z) = surface.sample(x, y)
                {
                    out.push([x, y, f64::from(z)]);
                }
            }
        }
    }
    out
}

fn ring_length(region: &Region) -> f64 {
    geomops::rings_of(region)
        .iter()
        .map(|r| {
            let n = r.len();
            (0..n)
                .map(|k| {
                    let (a, b) = (r[k], r[(k + 1) % n]);
                    (b[0] - a[0]).hypot(b[1] - a[1])
                })
                .sum::<f64>()
        })
        .sum()
}

/// Solves the 3 x 3 system `m x = v` by elimination with pivoting; `None` when it is singular.
fn solve3(mut m: [[f64; 3]; 3], mut v: [f64; 3]) -> Option<[f64; 3]> {
    let scale = m
        .iter()
        .flatten()
        .fold(0.0f64, |a, x| a.max(x.abs()))
        .max(1e-300);
    for col in 0..3 {
        let p = (col..3).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[p][col].abs() < scale * 1e-12 {
            return None;
        }
        m.swap(col, p);
        v.swap(col, p);
        for r in col + 1..3 {
            let f = m[r][col] / m[col][col];
            let pivot = m[col];
            for (x, q) in m[r].iter_mut().zip(pivot).skip(col) {
                *x -= f * q;
            }
            v[r] -= f * v[col];
        }
    }
    let mut x = [0.0; 3];
    for r in (0..3).rev() {
        let s: f64 = (r + 1..3).map(|c| m[r][c] * x[c]).sum();
        x[r] = (v[r] - s) / m[r][r];
    }
    Some(x)
}

/// The plane `z = a + b (x - cx) + c (y - cy)` through `pts` by least squares: `(cx, cy, a, b, c)`.
fn fit_plane(pts: &[[f64; 3]]) -> Result<[f64; 5]> {
    if pts.len() < 3 {
        return Err(Error::Param(
            "el contorno tiene menos de 3 puntos con altura: no se puede ajustar un plano".into(),
        ));
    }
    let n = pts.len() as f64;
    let (cx, cy) = (
        pts.iter().map(|p| p[0]).sum::<f64>() / n,
        pts.iter().map(|p| p[1]).sum::<f64>() / n,
    );
    let (mut sx, mut sy, mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut sz, mut sxz, mut syz) = (0.0, 0.0, 0.0);
    for p in pts {
        let (x, y) = (p[0] - cx, p[1] - cy);
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        sz += p[2];
        sxz += x * p[2];
        syz += y * p[2];
    }
    let s = solve3(
        [[n, sx, sy], [sx, sxx, sxy], [sy, sxy, syy]],
        [sz, sxz, syz],
    )
    .ok_or_else(|| {
        Error::Param("los puntos del contorno están alineados: no definen un plano".into())
    })?;
    Ok([cx, cy, s[0], s[1], s[2]])
}

/// The base surface under a pile, on the cells of `crop` (a window of the surface that holds
/// the outline); and a note when part of it had to be filled differently.
pub fn base_surface(
    crop: &Grid,
    region: &Region,
    base: &Base,
    dtm: Option<&Grid>,
) -> Result<(Grid, Option<String>)> {
    let mut g = crop.like();
    let fill = |g: &mut Grid, v: f64| g.data.fill(v as f32);
    let mut note = None;
    match base {
        Base::Level(z) => fill(&mut g, *z),
        Base::LowestPerimeter | Base::MedianPerimeter => {
            let mut zs: Vec<f64> = perimeter(crop, region).iter().map(|p| p[2]).collect();
            if zs.is_empty() {
                return Err(Error::Param(
                    "el contorno no cae sobre el modelo de superficie".into(),
                ));
            }
            zs.sort_by(f64::total_cmp);
            let v = if matches!(base, Base::LowestPerimeter) {
                zs[0]
            } else {
                let m = zs.len() / 2;
                if zs.len() % 2 == 1 {
                    zs[m]
                } else {
                    (zs[m - 1] + zs[m]) / 2.0
                }
            };
            fill(&mut g, v);
        }
        Base::Plane => {
            let [cx, cy, a, b, c] = fit_plane(&perimeter(crop, region))?;
            for r in 0..g.rows {
                for col in 0..g.cols {
                    let (x, y) = g.center(col, r);
                    g.data[r * g.cols + col] = (a + b * (x - cx) + c * (y - cy)) as f32;
                }
            }
        }
        Base::Interpolated => {
            let pts = perimeter(crop, region);
            if pts.len() < 3 {
                return Err(Error::Param(
                    "el contorno tiene menos de 3 puntos con altura: no se puede interpolar".into(),
                ));
            }
            crate::tin::rasterize_with(&pts, &mut g, 1e12, crate::tin::Thin::Nearest);
            // the triangles cover the hull of the outline, so the cells inside it are filled;
            // what is left (rounding at the very edge) takes the weighted mean of the outline
            let before = g.data.iter().filter(|v| v.is_nan()).count();
            if before > 0 {
                crate::idw::rasterize(&pts, &mut g, false);
                let left = g.data.iter().filter(|v| v.is_nan()).count();
                note = Some(format!(
                    "{} celdas del borde se rellenaron por distancia inversa ({} siguen sin base)",
                    before - left,
                    left
                ));
            }
        }
        Base::Dtm(_) => {
            let d =
                dtm.ok_or_else(|| Error::Param("falta el modelo del terreno de partida".into()))?;
            g = d.resampled_onto(crop);
        }
    }
    Ok((g, note))
}

/// The piles measured, the base and the height rasters when asked, and notes about the way.
pub type PileRun = (Vec<PileRow>, Option<Grid>, Option<Grid>, Vec<String>);

/// Measures every pile of `regions` on `surface`. Returns the rows and, when asked, the base
/// and the height over it as rasters on the lattice of `surface`.
pub fn piles(
    surface: &Grid,
    regions: &[Region],
    job: &PileJob,
    dtm: Option<&Grid>,
) -> Result<PileRun> {
    if !(job.min_height >= 0.0) {
        return Err(Error::Param(
            "la altura mínima no puede ser negativa".into(),
        ));
    }
    if let Some(d) = job.density
        && !(d > 0.0)
    {
        return Err(Error::Param("la densidad debe ser mayor que 0".into()));
    }
    let mut rows = Vec::with_capacity(regions.len());
    let mut notes = Vec::new();
    let mut base_out = job.base_raster.then(|| surface.like());
    let mut height_out = job.height_raster.then(|| surface.like());
    let cell_area = surface.cell * surface.cell;
    for (k, region) in regions.iter().enumerate() {
        crate::check_cancel()?;
        let bb = region.bbox();
        let m = surface.cell * 2.0;
        let crop = surface.crop([bb[0] - m, bb[1] - m, bb[2] + m, bb[3] + m])?;
        let (base, note) = base_surface(&crop, region, &job.base, dtm)
            .map_err(|e| Error::Param(format!("pila {} ({}): {e}", k + 1, region.name)))?;
        if let Some(n) = note {
            notes.push(format!("pila {} ({}): {n}", k + 1, region.name));
        }
        let v = volume::volumes(
            &crop,
            Reference::Dtm(&base),
            std::slice::from_ref(region),
            job.min_height,
        )?;
        let v = &v[0];
        // area on the slope, and where the pile is
        let slope = terrain::slope(&crop, 1.0, false, Edges::RepeatEdge);
        let (mut a3, mut base_sum, mut base_n) = (0.0, 0.0, 0u64);
        let (mut sx, mut sy, mut sn) = (0.0, 0.0, 0.0);
        for r in 0..crop.rows {
            for c in 0..crop.cols {
                let i = r * crop.cols + c;
                let (x, y) = crop.center(c, r);
                if crop.data[i].is_nan() || !region.contains(x, y) || base.data[i].is_nan() {
                    continue;
                }
                let s = f64::from(slope.data[i]);
                let cos = if s.is_nan() {
                    1.0
                } else {
                    s.to_radians().cos().max(0.05)
                };
                a3 += cell_area / cos;
                base_sum += f64::from(base.data[i]);
                base_n += 1;
                sx += x;
                sy += y;
                sn += 1.0;
                let h = crop.data[i] - base.data[i];
                if let Some(o) = base_out.as_mut()
                    && let Some(j) = surface.index(x, y)
                {
                    o.data[j] = base.data[i];
                }
                if let Some(o) = height_out.as_mut()
                    && let Some(j) = surface.index(x, y)
                    && f64::from(h) >= job.min_height
                {
                    o.data[j] = h;
                }
            }
        }
        rows.push(PileRow {
            id: k + 1,
            name: region.name.clone(),
            area_2d: v.area,
            area_3d: a3,
            base_z: if base_n > 0 {
                base_sum / base_n as f64
            } else {
                f64::NAN
            },
            volume: v.above,
            below: v.below,
            mean_h: v.mean_h,
            max_h: v.max_h,
            perimeter: ring_length(region),
            centroid: if sn > 0.0 {
                [sx / sn, sy / sn]
            } else {
                [f64::NAN; 2]
            },
            mass: job.density.map(|d| v.above * d),
            no_base: v.no_reference,
        });
    }
    Ok((rows, base_out, height_out, notes))
}

fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.3}")
    } else {
        String::new()
    }
}

/// The rows as a CSV table.
pub fn csv_text(rows: &[PileRow]) -> String {
    let mut s = String::from(
        "Id,Nombre,Area_2D_m2,Area_3D_m2,Cota_base_m,Volumen_m3,Vol_bajo_m3,H_media_m,H_max_m,Perimetro_m,X,Y,Masa_t\n",
    );
    for r in rows {
        let name = if r.name.contains([',', '"', '\n']) {
            format!("\"{}\"", r.name.replace('"', "\"\""))
        } else {
            r.name.clone()
        };
        s.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.id,
            name,
            num(r.area_2d),
            num(r.area_3d),
            num(r.base_z),
            num(r.volume),
            num(r.below),
            num(r.mean_h),
            num(r.max_h),
            num(r.perimeter),
            num(r.centroid[0]),
            num(r.centroid[1]),
            r.mass.map_or(String::new(), num)
        ));
    }
    s
}

/// The fields the layer gets, after the ones of the outlines.
pub const LAYER_FIELDS: [&str; 9] = [
    "area_2d",
    "area_3d",
    "base_z",
    "vol_sobre",
    "vol_bajo",
    "h_media",
    "h_max",
    "perimetro",
    "masa_t",
];

/// Measures the piles in `job.piles` over `job.surface` and writes what was asked; returns
/// the summary.
pub fn run(job: &PileJob) -> Result<String> {
    if let Ok(pyr) = crate::pyramid::Pyramid::open(&job.surface)
        && let Some(mem) = crate::copc_cache::total_memory()
    {
        let e = pyr.extent();
        let cells = ((e[2] - e[0]) / pyr.cell) * ((e[3] - e[1]) / pyr.cell);
        if cells * 12.0 > mem as f64 * 0.7 {
            return Err(Error::Param(format!(
                "el modelo tiene {:.0} millones de celdas y no cabe en la memoria: recórtalo a la zona de las pilas",
                cells / 1e6
            )));
        }
    }
    let surface = Grid::read(&job.surface)?;
    let epsg = Grid::read_epsg(&job.surface);
    let regions = volume::read_regions(&job.piles)?;
    if regions.is_empty() {
        return Err(Error::Param("la capa de pilas no tiene polígonos".into()));
    }
    let dtm = match &job.base {
        Base::Dtm(p) => Some(Grid::read(p)?),
        _ => None,
    };
    let (rows, base, height, notes) = piles(&surface, &regions, job, dtm.as_ref())?;
    crate::paths::ensure_parent(&job.output)?;
    let mut lines = notes;
    // the layer: the outlines with their own fields and the measures
    let features: Vec<Feature> = regions
        .iter()
        .zip(&rows)
        .map(|(region, r)| {
            let mut attrs: Vec<(String, String)> = region
                .attrs
                .iter()
                .filter(|(k, _)| !LAYER_FIELDS.iter().any(|f| f.eq_ignore_ascii_case(k)))
                .cloned()
                .collect();
            let values = [
                num(r.area_2d),
                num(r.area_3d),
                num(r.base_z),
                num(r.volume),
                num(r.below),
                num(r.mean_h),
                num(r.max_h),
                num(r.perimeter),
                r.mass.map_or(String::new(), num),
            ];
            attrs.extend(LAYER_FIELDS.iter().map(|f| (*f).to_string()).zip(values));
            Feature {
                geom: geomops::multipolygon_of(&geomops::rings_of(region)),
                attrs,
            }
        })
        .collect();
    let prj = std::fs::read_to_string(job.piles.with_extension("prj"))
        .ok()
        .or_else(|| epsg.and_then(vector::prj_for_epsg));
    let shp = crate::paths::swap_ext(&job.output, "shp");
    geomops::write_polygons(&PolyLayer { features, prj }, &shp)?;
    lines.push(format!("  pilas (polígonos): {}", shp.display()));
    if job.csv {
        let path = crate::paths::swap_ext(&job.output, "csv");
        std::fs::write(&path, csv_text(&rows))?;
        lines.push(format!("  tabla: {}", path.display()));
    }
    let stem = |tag: &str| {
        let p = crate::paths::swap_ext(&job.output, "tif");
        p.with_file_name(format!("{}_{tag}.tif", crate::paths::stem(&p)))
    };
    if let Some(g) = base {
        let p = stem("base");
        g.write_tiff(&p, epsg)?;
        lines.push(format!("  superficie base: {}", p.display()));
    }
    if let Some(g) = height {
        let p = stem("altura");
        g.write_tiff(&p, epsg)?;
        lines.push(format!("  altura sobre la base: {}", p.display()));
    }
    let total: f64 = rows.iter().map(|r| r.volume).sum();
    let mut head = format!("{} pilas: {total:.2} m³", rows.len());
    if let Some(d) = job.density {
        head.push_str(&format!(", {:.2} t (densidad {d})", total * d));
    }
    let skipped: u64 = rows.iter().map(|r| r.no_base).sum();
    if skipped > 0 {
        head.push_str(&format!(" ({skipped} celdas sin base no se midieron)"));
    }
    lines.insert(0, head);
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::Shape2;

    fn region(shape: Shape2) -> Region {
        Region {
            name: "P1".into(),
            shape,
            attrs: Vec::new(),
        }
    }

    fn job(base: Base) -> PileJob {
        let mut j = PileJob::new(PathBuf::new(), PathBuf::new(), PathBuf::new());
        j.base = base;
        j
    }

    /// A surface of 100 x 100 m with cells of `cell`, `f(x, y)` high.
    fn surface(cell: f64, f: impl Fn(f64, f64) -> f64) -> Result<Grid> {
        let n = (100.0 / cell) as usize;
        let mut g = Grid::new(0.0, 0.0, cell, n, n)?;
        for r in 0..n {
            for c in 0..n {
                let (x, y) = g.center(c, r);
                g.data[r * n + c] = f(x, y) as f32;
            }
        }
        Ok(g)
    }

    #[test]
    fn a_cone_has_a_third_of_its_cylinder() -> Result<()> {
        let (h, rad) = (5.0, 10.0);
        let g = surface(0.5, |x, y| {
            (h - h / rad * (x - 50.0).hypot(y - 50.0)).max(0.0)
        })?;
        let reg = region(Shape2::Circle {
            cx: 50.0,
            cy: 50.0,
            r: rad + 2.0,
        });
        let (rows, ..) = piles(&g, &[reg], &job(Base::Level(0.0)), None)?;
        let exact = std::f64::consts::PI * rad * rad * h / 3.0;
        assert!(
            (rows[0].volume / exact - 1.0).abs() < 0.01,
            "{} / {exact}",
            rows[0].volume
        );
        assert!((rows[0].max_h - h).abs() < 0.3);
        assert!(rows[0].area_3d > rows[0].area_2d);
        Ok(())
    }

    #[test]
    fn a_box_aligned_with_the_cells_is_exact() -> Result<()> {
        let g = surface(1.0, |x, y| {
            if (20.0..30.0).contains(&x) && (40.0..46.0).contains(&y) {
                3.0
            } else {
                0.0
            }
        })?;
        let reg = region(Shape2::Rect([20.0, 40.0, 30.0, 46.0]));
        let mut j = job(Base::Level(0.0));
        j.density = Some(1.8);
        let (rows, ..) = piles(&g, &[reg], &j, None)?;
        assert_eq!(rows[0].volume, 180.0);
        assert_eq!(rows[0].area_2d, 60.0);
        assert_eq!(rows[0].mass, Some(324.0));
        assert_eq!(rows[0].perimeter, 32.0);
        Ok(())
    }

    #[test]
    fn on_a_slope_the_plane_gives_the_prism_and_a_level_gives_more() -> Result<()> {
        // ground z = 0.2 x; a pile 2 m high on 12..18 in x and 42..48 in y
        let g = surface(0.5, |x, y| {
            0.2 * x
                + if (12.0..18.0).contains(&x) && (42.0..48.0).contains(&y) {
                    2.0
                } else {
                    0.0
                }
        })?;
        // an outline around the foot of the pile
        let reg = region(Shape2::Rect([10.0, 40.0, 20.0, 50.0]));
        let mut j = job(Base::Plane);
        j.min_height = 0.01;
        let (plane, ..) = piles(&g, std::slice::from_ref(&reg), &j, None)?;
        assert!(
            (plane[0].volume - 6.0 * 6.0 * 2.0).abs() < 0.5,
            "{}",
            plane[0].volume
        );
        let (level, ..) = piles(
            &g,
            std::slice::from_ref(&reg),
            &job(Base::LowestPerimeter),
            None,
        )?;
        assert!(
            level[0].volume > plane[0].volume + 20.0,
            "{} / {}",
            level[0].volume,
            plane[0].volume
        );
        let (median, ..) = piles(&g, &[reg], &job(Base::MedianPerimeter), None)?;
        assert!(median[0].volume.is_finite());
        Ok(())
    }

    #[test]
    fn an_interpolated_base_works_with_an_l_shaped_outline() -> Result<()> {
        let g = surface(0.5, |x, y| {
            if (12.0..16.0).contains(&x) && (12.0..16.0).contains(&y) {
                2.0
            } else {
                0.0
            }
        })?;
        let ring = vec![
            [10.0, 10.0],
            [30.0, 10.0],
            [30.0, 20.0],
            [20.0, 20.0],
            [20.0, 30.0],
            [10.0, 30.0],
        ];
        let reg = region(Shape2::Polygon(vector::Polygon::new(vec![ring])));
        let mut j = job(Base::Interpolated);
        j.min_height = 0.01;
        let (rows, base, ..) = piles(&g, &[reg], &j, None)?;
        assert!((rows[0].volume - 32.0).abs() < 0.5, "{}", rows[0].volume);
        assert!(base.is_none());
        assert_eq!(rows[0].no_base, 0);
        Ok(())
    }

    #[test]
    fn a_terrain_model_is_the_base_and_gaps_do_not_count() -> Result<()> {
        let mut g = surface(1.0, |x, y| {
            if (20.0..30.0).contains(&x) && (40.0..46.0).contains(&y) {
                5.0
            } else {
                2.0
            }
        })?;
        // a hole in the surface inside the outline
        let i = g.index(25.5, 43.5).ok_or(Error::Param("fuera".into()))?;
        g.data[i] = f32::NAN;
        let dtm = surface(2.0, |_, _| 2.0)?;
        let reg = region(Shape2::Rect([20.0, 40.0, 30.0, 46.0]));
        let (rows, ..) = piles(&g, &[reg], &job(Base::Dtm(PathBuf::new())), Some(&dtm))?;
        // 59 cells of 3 m
        assert!(
            (rows[0].volume - 59.0 * 3.0).abs() < 1e-3,
            "{}",
            rows[0].volume
        );
        assert!(rows[0].volume.is_finite());
        Ok(())
    }

    #[test]
    fn bad_outlines_and_settings_are_refused() -> Result<()> {
        let g = surface(1.0, |_, _| 0.0)?;
        // a thin outline whose perimeter has all its points on one line
        let reg = region(Shape2::Polygon(vector::Polygon::new(vec![vec![
            [10.0, 10.0],
            [20.0, 10.0],
            [30.0, 10.0],
        ]])));
        assert!(piles(&g, &[reg], &job(Base::Plane), None).is_err());
        let mut j = job(Base::Level(0.0));
        j.min_height = -1.0;
        assert!(piles(&g, &[], &j, None).is_err());
        let mut j = job(Base::Level(0.0));
        j.density = Some(0.0);
        assert!(piles(&g, &[], &j, None).is_err());
        Ok(())
    }

    #[test]
    fn the_results_are_written() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_piles_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let g = surface(1.0, |x, y| {
            if (20.0..30.0).contains(&x) && (40.0..46.0).contains(&y) {
                3.0
            } else {
                0.0
            }
        })?;
        g.write_tiff(&dir.join("mds.tif"), Some(25829))?;
        let layer = PolyLayer {
            features: vec![Feature {
                geom: geomops::multipolygon_of(&[vec![
                    [18.0, 38.0],
                    [32.0, 38.0],
                    [32.0, 48.0],
                    [18.0, 48.0],
                ]]),
                attrs: vec![("nombre".into(), "pila A".into())],
            }],
            prj: None,
        };
        geomops::write_polygons(&layer, &dir.join("pilas.shp"))?;
        let mut j = PileJob::new(
            dir.join("mds.tif"),
            dir.join("pilas.shp"),
            dir.join("res.shp"),
        );
        j.base = Base::LowestPerimeter;
        j.base_raster = true;
        j.height_raster = true;
        let msg = run(&j)?;
        assert!(msg.starts_with("1 pilas: 180.00 m³"), "{msg}");
        let csv = std::fs::read_to_string(dir.join("res.csv"))?;
        assert!(
            csv.lines().nth(1).is_some_and(|l| l.contains("180.000")),
            "{csv}"
        );
        let back = crate::view::read_vector(&dir.join("res.shp"))?;
        assert_eq!(back.features.len(), 1);
        assert!(back.fields.iter().any(|f| f == "vol_sobre"));
        assert!(back.fields.iter().any(|f| f == "nombre"));
        assert!(dir.join("res_base.tif").exists() && dir.join("res_altura.tif").exists());
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
