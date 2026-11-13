//! Voronoi diagrams and Delaunay neighbours of a set of points.
//!
//! The Delaunay triangulation comes from `delaunator`; the cell of a site is the area closer to it
//! than to any other, which is the intersection of the half-planes towards its Delaunay
//! neighbours (a neighbour is all that can bound a cell). Cells of the sites on the edge of the set
//! have no end: with no clip they are `None`, and with a clip they are cut by it. Repeated
//! sites (the same x and y) are one site: the first keeps the cell and the others get none and
//! an area of 0. Coordinates are shifted to the centre of the points before the arithmetic, so
//! that values like 4 500 000 do not eat the precision.

use std::collections::HashMap;

use geo::{Area, BooleanOps, MultiPolygon};

use crate::geomops;
use crate::vector::Polygon;
use crate::view::{Feature, GeomKind, VectorData};
use crate::{Error, Result};

/// What the cells are cut by.
#[derive(Clone, Debug)]
pub enum Clip {
    /// Nothing: the cells that have no end are left out.
    None,
    /// A rectangle: min x, min y, max x, max y.
    Bbox([f64; 4]),
    /// A polygon (with its holes).
    Polygon(Polygon),
}

/// The cells of a set of sites.
pub struct Diagram {
    /// The cell of each site, in the order they were given; `None` for those without one.
    pub cells: Vec<Option<MultiPolygon<f64>>>,
    /// The site each one stands for: itself, or the first one with the same place.
    pub site_of_cell: Vec<usize>,
}

/// A triangulation of the distinct sites.
struct Mesh {
    /// Centred coordinates of the distinct sites.
    pts: Vec<[f64; 2]>,
    centre: [f64; 2],
    /// Index (in the input) of each distinct site.
    orig: Vec<usize>,
    /// For each input site, the distinct site it is.
    unique_of: Vec<usize>,
    /// Delaunay neighbours of each distinct site.
    nbrs: Vec<Vec<usize>>,
}

fn mesh(sites: &[[f64; 2]]) -> Result<Mesh> {
    if let Some(i) = sites
        .iter()
        .position(|s| !s[0].is_finite() || !s[1].is_finite())
    {
        return Err(Error::Param(format!(
            "el punto {} no tiene coordenadas válidas",
            i + 1
        )));
    }
    let mut seen: HashMap<(u64, u64), usize> = HashMap::new();
    let (mut orig, mut unique_of) = (Vec::new(), Vec::with_capacity(sites.len()));
    for (i, s) in sites.iter().enumerate() {
        let u = *seen
            .entry((s[0].to_bits(), s[1].to_bits()))
            .or_insert_with(|| {
                orig.push(i);
                orig.len() - 1
            });
        unique_of.push(u);
    }
    if orig.len() < 3 {
        return Err(Error::Param(
            "hacen falta al menos 3 puntos distintos para un diagrama de Voronoi".into(),
        ));
    }
    let n = orig.len() as f64;
    let centre = [
        orig.iter().map(|&i| sites[i][0]).sum::<f64>() / n,
        orig.iter().map(|&i| sites[i][1]).sum::<f64>() / n,
    ];
    let pts: Vec<[f64; 2]> = orig
        .iter()
        .map(|&i| [sites[i][0] - centre[0], sites[i][1] - centre[1]])
        .collect();
    let xy: Vec<delaunator::Point> = pts
        .iter()
        .map(|p| delaunator::Point { x: p[0], y: p[1] })
        .collect();
    let t = delaunator::triangulate(&xy);
    if t.triangles.is_empty() {
        return Err(Error::Param(
            "los puntos están alineados: no forman triángulos".into(),
        ));
    }
    let mut nbrs: Vec<Vec<usize>> = vec![Vec::new(); pts.len()];
    for e in 0..t.triangles.len() {
        let next = if e % 3 == 2 { e - 2 } else { e + 1 };
        let (a, b) = (t.triangles[e], t.triangles[next]);
        nbrs[a].push(b);
        nbrs[b].push(a);
    }
    for v in &mut nbrs {
        v.sort_unstable();
        v.dedup();
    }
    Ok(Mesh {
        pts,
        centre,
        orig,
        unique_of,
        nbrs,
    })
}

/// `ring` cut to the half-plane `a x + b y <= c` (Sutherland and Hodgman).
fn cut(ring: &[[f64; 2]], a: f64, b: f64, c: f64) -> Vec<[f64; 2]> {
    let side = |p: &[f64; 2]| a * p[0] + b * p[1] - c;
    let mut out = Vec::with_capacity(ring.len() + 1);
    for k in 0..ring.len() {
        let (p, q) = (ring[k], ring[(k + 1) % ring.len()]);
        let (sp, sq) = (side(&p), side(&q));
        if sp <= 0.0 {
            out.push(p);
        }
        if (sp < 0.0 && sq > 0.0) || (sp > 0.0 && sq < 0.0) {
            let t = sp / (sp - sq);
            out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        }
    }
    out
}

fn ring_area(r: &[[f64; 2]]) -> f64 {
    let n = r.len();
    (0..n)
        .map(|k| r[k][0] * r[(k + 1) % n][1] - r[(k + 1) % n][0] * r[k][1])
        .sum::<f64>()
        .abs()
        / 2.0
}

impl Mesh {
    /// The cell of distinct site `u`, inside the box `[x0, y0, x1, y1]` (centred coordinates).
    fn box_cell(&self, u: usize, b: [f64; 4]) -> Vec<[f64; 2]> {
        let mut ring = vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]];
        let s = self.pts[u];
        for &n in &self.nbrs[u] {
            let q = self.pts[n];
            // closer to s than to q: (q - s) . p <= (|q|^2 - |s|^2) / 2
            let (a, bb) = (q[0] - s[0], q[1] - s[1]);
            let c = (q[0] * q[0] + q[1] * q[1] - s[0] * s[0] - s[1] * s[1]) / 2.0;
            ring = cut(&ring, a, bb, c);
            if ring.len() < 3 {
                break;
            }
        }
        ring
    }

    /// Centred box that holds every site with `margin` to spare.
    fn extent(&self, margin: f64) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in &self.pts {
            b = [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ];
        }
        [b[0] - margin, b[1] - margin, b[2] + margin, b[3] + margin]
    }
}

/// The ring in map coordinates as a polygon.
fn polygon_of(ring: &[[f64; 2]], centre: [f64; 2]) -> geo::Polygon<f64> {
    let mut pts: Vec<geo::Coord<f64>> = ring
        .iter()
        .map(|p| geo::Coord {
            x: p[0] + centre[0],
            y: p[1] + centre[1],
        })
        .collect();
    if let Some(f) = pts.first().copied() {
        pts.push(f);
    }
    geo::Polygon::new(geo::LineString::new(pts), Vec::new())
}

/// The clip as a box in centred coordinates and, for a polygon, the polygon to intersect with.
fn clip_box(m: &Mesh, clip: &Clip) -> (Option<[f64; 4]>, Option<MultiPolygon<f64>>) {
    match clip {
        Clip::None => {
            let b = m.extent(0.0);
            let size = (b[2] - b[0]).max(b[3] - b[1]).max(1.0);
            // far enough that no cell with an end is cut before it reaches the edge
            (Some(m.extent(4.0 * size)), None)
        }
        Clip::Bbox(b) => (
            Some([
                b[0] - m.centre[0],
                b[1] - m.centre[1],
                b[2] - m.centre[0],
                b[3] - m.centre[1],
            ]),
            None,
        ),
        Clip::Polygon(p) => {
            let bb = p.bbox;
            (
                Some([
                    bb[0] - m.centre[0] - 1.0,
                    bb[1] - m.centre[1] - 1.0,
                    bb[2] - m.centre[0] + 1.0,
                    bb[3] - m.centre[1] + 1.0,
                ]),
                Some(geomops::multipolygon_of(&p.rings)),
            )
        }
    }
}

/// Whether a cell cut by the far box touches it: it has no end.
fn reaches(ring: &[[f64; 2]], b: [f64; 4]) -> bool {
    let eps = 1e-9 * (b[2] - b[0]).max(b[3] - b[1]).max(1.0);
    ring.iter().any(|p| {
        (p[0] - b[0]).abs() < eps
            || (p[0] - b[2]).abs() < eps
            || (p[1] - b[1]).abs() < eps
            || (p[1] - b[3]).abs() < eps
    })
}

/// The Voronoi cells of `sites`, cut by `clip`.
pub fn diagram(sites: &[[f64; 2]], clip: &Clip) -> Result<Diagram> {
    let m = mesh(sites)?;
    let (bx, poly) = clip_box(&m, clip);
    let bx = bx.unwrap_or_else(|| m.extent(1.0));
    let mut cells: Vec<Option<MultiPolygon<f64>>> = vec![None; sites.len()];
    for u in 0..m.pts.len() {
        if u % 4096 == 0 {
            crate::check_cancel()?;
        }
        let ring = m.box_cell(u, bx);
        if ring.len() < 3 {
            continue;
        }
        if matches!(clip, Clip::None) && reaches(&ring, bx) {
            continue;
        }
        let cell = MultiPolygon::new(vec![polygon_of(&ring, m.centre)]);
        cells[m.orig[u]] = Some(match &poly {
            Some(p) => cell.intersection(p),
            None => cell,
        });
    }
    let site_of_cell = m.unique_of.iter().map(|&u| m.orig[u]).collect();
    Ok(Diagram {
        cells,
        site_of_cell,
    })
}

/// The area of each site's cell cut by `clip`: NaN where the cell has no end and there is no clip,
/// 0 for the sites that repeat another one.
pub fn cell_areas(sites: &[[f64; 2]], clip: &Clip) -> Result<Vec<f64>> {
    let m = mesh(sites)?;
    let (bx, poly) = clip_box(&m, clip);
    let bx = bx.unwrap_or_else(|| m.extent(1.0));
    let mut out = vec![0.0; sites.len()];
    for u in 0..m.pts.len() {
        if u % 4096 == 0 {
            crate::check_cancel()?;
        }
        let ring = m.box_cell(u, bx);
        let i = m.orig[u];
        if ring.len() < 3 {
            out[i] = if matches!(clip, Clip::None) {
                f64::NAN
            } else {
                0.0
            };
            continue;
        }
        if matches!(clip, Clip::None) && reaches(&ring, bx) {
            out[i] = f64::NAN;
            continue;
        }
        out[i] = match &poly {
            Some(p) => MultiPolygon::new(vec![polygon_of(&ring, m.centre)])
                .intersection(p)
                .unsigned_area(),
            None => ring_area(&ring),
        };
    }
    Ok(out)
}

/// The Delaunay neighbours of each site (the indices of the other sites it shares a triangle edge
/// with); empty for the sites that repeat another one.
pub fn neighbors(sites: &[[f64; 2]]) -> Result<Vec<Vec<usize>>> {
    let m = mesh(sites)?;
    let mut out = vec![Vec::new(); sites.len()];
    for (u, v) in m.nbrs.iter().enumerate() {
        out[m.orig[u]] = v.iter().map(|&n| m.orig[n]).collect();
    }
    Ok(out)
}

/// The edges of the Delaunay triangulation, each once, in map coordinates.
pub fn delaunay_edges(sites: &[[f64; 2]]) -> Result<Vec<([f64; 2], [f64; 2])>> {
    let m = mesh(sites)?;
    let at = |u: usize| [m.pts[u][0] + m.centre[0], m.pts[u][1] + m.centre[1]];
    let mut out = Vec::new();
    for (u, v) in m.nbrs.iter().enumerate() {
        for &n in v.iter().filter(|&&n| n > u) {
            out.push((at(u), at(n)));
        }
    }
    Ok(out)
}

/// A clip made of the polygons of a layer.
pub fn clip_from_layer(path: &std::path::Path) -> Result<Clip> {
    let regions = crate::vector::read_regions(path, &crate::vector::ReadOptions::default())?;
    let rings: Vec<Vec<[f64; 2]>> = regions.iter().flat_map(geomops::rings_of).collect();
    if rings.is_empty() {
        return Err(Error::Param(format!(
            "{}: no tiene polígonos",
            path.display()
        )));
    }
    Ok(Clip::Polygon(Polygon::new(rings)))
}

/// A box a little bigger than the points of `d` (5 % of the larger side, at least 1).
pub fn clip_box_of(d: &VectorData) -> Clip {
    let b = d.bounds;
    let m = ((b[2] - b[0]).max(b[3] - b[1]) * 0.05).max(1.0);
    Clip::Bbox([b[0] - m, b[1] - m, b[2] + m, b[3] + m])
}

/// The points of a layer as sites, with their feature numbers.
fn sites_of(d: &VectorData) -> Result<(Vec<[f64; 2]>, Vec<usize>)> {
    let (mut sites, mut rows) = (Vec::new(), Vec::new());
    for (k, f) in d.features.iter().enumerate() {
        if let Some(p) = f.parts.iter().flatten().next() {
            sites.push(*p);
            rows.push(k);
        }
    }
    if sites.is_empty() {
        return Err(Error::Param("la capa no tiene puntos".into()));
    }
    Ok((sites, rows))
}

/// The Voronoi polygons of the points of a layer, with the attributes of their point.
pub fn layer(d: &VectorData, clip: &Clip) -> Result<VectorData> {
    if d.polygons {
        return Err(Error::Param(
            "el diagrama de Voronoi parte de una capa de puntos".into(),
        ));
    }
    let (sites, rows) = sites_of(d)?;
    let diag = diagram(&sites, clip)?;
    let mut out = VectorData {
        polygons: true,
        fields: d.fields.clone(),
        bounds: [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        ..Default::default()
    };
    for (i, cell) in diag.cells.iter().enumerate() {
        let Some(c) = cell else { continue };
        let parts = geomops::parts_of(c);
        if parts.is_empty() {
            continue;
        }
        for q in parts.iter().flatten() {
            out.bounds = [
                out.bounds[0].min(q[0]),
                out.bounds[1].min(q[1]),
                out.bounds[2].max(q[0]),
                out.bounds[3].max(q[1]),
            ];
        }
        out.features.push(Feature { parts });
        out.text
            .push(d.text.get(rows[i]).cloned().unwrap_or_default());
        out.num
            .push(d.num.get(rows[i]).cloned().unwrap_or_default());
    }
    Ok(out)
}

/// The Delaunay edges of the points of a layer as a line layer.
pub fn edges_layer(d: &VectorData) -> Result<VectorData> {
    let (sites, _) = sites_of(d)?;
    let edges = delaunay_edges(&sites)?;
    let mut out = VectorData {
        fields: vec!["id".into()],
        bounds: d.bounds,
        ..Default::default()
    };
    for (k, (a, b)) in edges.into_iter().enumerate() {
        out.features.push(Feature {
            parts: vec![vec![a, b]],
        });
        out.text.push(vec![(k + 1).to_string()]);
        out.num.push(vec![(k + 1) as f64]);
    }
    Ok(out)
}

/// Reads the points of `input`, writes their Voronoi polygons (and the Delaunay edges next to
/// them if asked), and reports.
pub fn run(
    input: &std::path::Path,
    output: &std::path::Path,
    clip: &Clip,
    delaunay: bool,
) -> Result<String> {
    let (d, kind, prj) = crate::vecops::read_layer(input)?;
    if kind != GeomKind::Point {
        return Err(Error::Param(
            "el diagrama de Voronoi parte de una capa de puntos".into(),
        ));
    }
    let cells = layer(&d, clip)?;
    crate::paths::ensure_parent(output)?;
    crate::vector::write_layer(output, &cells, GeomKind::Polygon, prj.as_deref())?;
    let mut text = format!(
        "{} polígonos de Voronoi de {} puntos -> {}",
        cells.features.len(),
        d.features.len(),
        output.display()
    );
    if matches!(clip, Clip::None) && cells.features.len() < d.features.len() {
        text.push_str(&format!(
            "\n  {} puntos del borde no tienen celda cerrada: usa un recorte para tenerlas",
            d.features.len() - cells.features.len()
        ));
    }
    if delaunay {
        let lines = edges_layer(&d)?;
        let p = output.with_file_name(format!(
            "{}_delaunay.{}",
            crate::paths::stem(output),
            output
                .extension()
                .map_or("shp".into(), |e| e.to_string_lossy().to_string())
        ));
        crate::vector::write_layer(&p, &lines, GeomKind::Line, prj.as_deref())?;
        text.push_str(&format!("\n  aristas de Delaunay: {}", p.display()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lattice(n: usize, s: f64) -> Vec<[f64; 2]> {
        let mut v = Vec::new();
        for r in 0..n {
            for c in 0..n {
                v.push([500_000.0 + c as f64 * s, 4_500_000.0 + r as f64 * s]);
            }
        }
        v
    }

    #[test]
    fn a_regular_lattice_has_square_cells() -> Result<()> {
        let (n, s) = (6, 2.5);
        let sites = lattice(n, s);
        let a = cell_areas(&sites, &Clip::None)?;
        for r in 0..n {
            for c in 0..n {
                let area = a[r * n + c];
                if r == 0 || c == 0 || r == n - 1 || c == n - 1 {
                    assert!(area.is_nan());
                } else {
                    assert!((area - s * s).abs() < 1e-6, "{area}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn the_centre_of_a_square_has_a_diamond_of_half_the_area() -> Result<()> {
        let sites = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [5.0, 5.0],
        ];
        let a = cell_areas(&sites, &Clip::Bbox([0.0, 0.0, 10.0, 10.0]))?;
        assert!((a[4] - 50.0).abs() < 1e-9, "{}", a[4]);
        assert!((a.iter().sum::<f64>() - 100.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn the_cells_cut_by_a_box_partition_it_exactly() -> Result<()> {
        let mut state = 7u64;
        let mut rnd = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 11) as f64 / (1u64 << 53) as f64
        };
        let sites: Vec<[f64; 2]> = (0..200)
            .map(|_| [500_000.0 + rnd() * 100.0, 4_500_000.0 + rnd() * 80.0])
            .collect();
        let b = [499_950.0, 4_499_950.0, 500_150.0, 4_500_130.0];
        let total: f64 = cell_areas(&sites, &Clip::Bbox(b))?.iter().sum();
        let exact = (b[2] - b[0]) * (b[3] - b[1]);
        assert!((total / exact - 1.0).abs() < 1e-9, "{total} / {exact}");
        // the diagram holds the same cells
        let d = diagram(&sites, &Clip::Bbox(b))?;
        let sum: f64 = d.cells.iter().flatten().map(|m| m.unsigned_area()).sum();
        assert!((sum / exact - 1.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn a_polygon_clip_keeps_only_what_is_inside() -> Result<()> {
        let sites = lattice(5, 10.0);
        let square = Polygon::new(vec![vec![
            [500_005.0, 4_500_005.0],
            [500_035.0, 4_500_005.0],
            [500_035.0, 4_500_035.0],
            [500_005.0, 4_500_035.0],
        ]]);
        let total: f64 = cell_areas(&sites, &Clip::Polygon(square))?.iter().sum();
        assert!((total - 900.0).abs() < 1e-6, "{total}");
        Ok(())
    }

    #[test]
    fn repeated_sites_count_once() -> Result<()> {
        let mut sites = lattice(4, 5.0);
        sites.push(sites[5]);
        let b = [499_990.0, 4_499_990.0, 500_030.0, 4_500_030.0];
        let a = cell_areas(&sites, &Clip::Bbox(b))?;
        assert_eq!(a[16], 0.0);
        assert!(a[5] > 0.0);
        assert!((a.iter().sum::<f64>() - 1600.0).abs() < 1e-6);
        let d = diagram(&sites, &Clip::Bbox(b))?;
        assert!(d.cells[16].is_none() && d.cells[5].is_some());
        assert_eq!(d.site_of_cell[16], 5);
        Ok(())
    }

    #[test]
    fn too_few_or_aligned_points_are_refused() {
        assert!(cell_areas(&[[0.0, 0.0], [1.0, 0.0]], &Clip::None).is_err());
        let line: Vec<[f64; 2]> = (0..10).map(|k| [k as f64, 2.0 * k as f64]).collect();
        let e = cell_areas(&line, &Clip::None)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(e.contains("alineados"), "{e}");
        assert!(cell_areas(&[[0.0, 0.0], [1.0, f64::NAN], [2.0, 3.0]], &Clip::None).is_err());
    }

    #[test]
    fn the_neighbours_of_a_lattice_and_its_edges() -> Result<()> {
        let sites = lattice(3, 1.0);
        let n = neighbors(&sites)?;
        // the centre touches the four around it (and, in a perfect lattice, maybe a diagonal
        // chosen by the triangulation): at least 4, at most 8, all distinct and not itself
        assert!(n[4].len() >= 4 && n[4].len() <= 8 && !n[4].contains(&4));
        for (i, v) in n.iter().enumerate() {
            assert!(v.iter().all(|j| n[*j].contains(&i)));
        }
        let edges = delaunay_edges(&sites)?;
        assert!(edges.len() >= 12);
        Ok(())
    }

    #[test]
    fn a_point_layer_becomes_polygons_with_its_attributes() -> Result<()> {
        let sites = lattice(4, 5.0);
        let d = VectorData {
            points: true,
            fields: vec!["nombre".into()],
            features: sites
                .iter()
                .map(|s| Feature {
                    parts: vec![vec![*s]],
                })
                .collect(),
            text: (0..16).map(|i| vec![format!("p{i}")]).collect(),
            num: (0..16).map(|_| vec![f64::NAN]).collect(),
            bounds: [500_000.0, 4_500_000.0, 500_015.0, 4_500_015.0],
            ..Default::default()
        };
        let none = layer(&d, &Clip::None)?;
        assert_eq!(none.features.len(), 4);
        let clipped = layer(
            &d,
            &Clip::Bbox([499_990.0, 4_499_990.0, 500_030.0, 4_500_030.0]),
        )?;
        assert_eq!(clipped.features.len(), 16);
        assert_eq!(clipped.text[3][0], "p3");
        assert!(clipped.polygons);
        Ok(())
    }

    #[test]
    fn the_layer_is_written() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_voronoi_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let sites = lattice(4, 5.0);
        let d = VectorData {
            points: true,
            fields: vec!["id".into()],
            features: sites
                .iter()
                .map(|s| Feature {
                    parts: vec![vec![*s]],
                })
                .collect(),
            text: (0..16).map(|i| vec![i.to_string()]).collect(),
            num: (0..16).map(|i| vec![f64::from(i)]).collect(),
            bounds: [500_000.0, 4_500_000.0, 500_015.0, 4_500_015.0],
            ..Default::default()
        };
        crate::vector::write_layer(&dir.join("p.shp"), &d, GeomKind::Point, None)?;
        let msg = run(
            &dir.join("p.shp"),
            &dir.join("v.shp"),
            &Clip::Bbox([499_990.0, 4_499_990.0, 500_030.0, 4_500_030.0]),
            true,
        )?;
        assert!(
            msg.starts_with("16 polígonos de Voronoi de 16 puntos"),
            "{msg}"
        );
        assert_eq!(
            crate::view::read_vector(&dir.join("v.shp"))?.features.len(),
            16
        );
        assert!(dir.join("v_delaunay.shp").exists());
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
