//! Objetos como polígonos: el contorno exacto de las celdas de cada objeto, huecos incluidos.
//!
//! Cada lado de celda que tiene una etiqueta distinta al otro lado se convierte en una arista
//! dirigida entre dos esquinas del retículo, siempre con el objeto a su derecha. Las aristas se
//! ordenan en una lista por objeto por conteo (de modo que el coste es lineal en las celdas sea
//! cual sea el número de objetos), y seguirlas da anillos cerrados: los anillos exteriores van en
//! el sentido de las agujas del reloj y los huecos en sentido contrario. En una esquina donde dos
//! celdas de un objeto solo se tocan, el anillo se aparta de la celda que toca, de modo que los
//! anillos del objeto nunca se cruzan.

use rayon::prelude::*;

use super::attributes::AttrTable;
use super::classify::Classified;
use super::{Labels, Lattice};
use crate::vector::{PolyTable, prj_for_epsg};
use crate::{Error, Result, check_cancel};

/// Los anillos de un objeto, como coordenadas del mapa; cada anillo está abierto (el primer vértice
/// no se repite al final).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub outers: Vec<Vec<[f64; 2]>>,
    pub holes: Vec<Vec<[f64; 2]>>,
}

/// Los límites de todos los objetos de una rejilla de etiquetas.
pub struct Tracer<'a> {
    labels: &'a Labels,
    /// `edges[start[i]..start[i + 1]]` son las aristas del objeto `i + 1`.
    start: Vec<usize>,
    edges: Vec<(u32, u32)>,
}

impl<'a> Tracer<'a> {
    pub fn new(labels: &'a Labels) -> Result<Self> {
        let Lattice { cols, rows, .. } = labels.lat;
        if (cols + 1).saturating_mul(rows + 1) >= u32::MAX as usize {
            return Err(Error::Param("la rejilla es demasiado grande".into()));
        }
        let ids = &labels.ids;
        let n = labels.count as usize;
        let w = cols + 1;
        // dónde cada lado de la celda (x, y) tiene otra etiqueta al otro lado
        let sides = |x: usize, y: usize, l: u32| {
            let i = y * cols + x;
            [
                y == 0 || ids[i - cols] != l,
                x + 1 == cols || ids[i + 1] != l,
                y + 1 == rows || ids[i + cols] != l,
                x == 0 || ids[i - 1] != l,
            ]
        };
        let mut start = vec![0usize; n + 1];
        for y in 0..rows {
            if y % 1024 == 0 {
                check_cancel()?;
            }
            for x in 0..cols {
                let l = ids[y * cols + x];
                if l != 0 {
                    start[l as usize] += sides(x, y, l).iter().filter(|s| **s).count();
                }
            }
        }
        // start[l] guarda el recuento del objeto l; se convierten en desplazamientos
        let mut total = 0usize;
        for s in start.iter_mut().take(n + 1) {
            let c = *s;
            *s = total;
            total += c;
        }
        // start[0] es 0 y no se usa (etiqueta 0); las aristas del objeto i empiezan en start[i + 1
        // - 1 + ...] de modo que el objeto `o` (desde 0) empieza en start[o]
        let mut offsets: Vec<usize> = start[1..].to_vec();
        offsets.push(total);
        let mut fill = offsets.clone();
        let mut edges = vec![(0u32, 0u32); total];
        let v = |x: usize, y: usize| (y * w + x) as u32;
        for y in 0..rows {
            if y % 1024 == 0 {
                check_cancel()?;
            }
            for x in 0..cols {
                let l = ids[y * cols + x];
                if l == 0 {
                    continue;
                }
                let o = (l - 1) as usize;
                let [top, right, bottom, left] = sides(x, y, l);
                let mut put = |from: u32, to: u32| {
                    edges[fill[o]] = (from, to);
                    fill[o] += 1;
                };
                if top {
                    put(v(x, y), v(x + 1, y));
                }
                if right {
                    put(v(x + 1, y), v(x + 1, y + 1));
                }
                if bottom {
                    put(v(x + 1, y + 1), v(x, y + 1));
                }
                if left {
                    put(v(x, y + 1), v(x, y));
                }
            }
        }
        Ok(Self {
            labels,
            start: offsets,
            edges,
        })
    }

    /// El contorno del objeto `id` (desde 1), sus anillos simplificados en `simplify` unidades del
    /// mapa cuando es positivo (Douglas–Peucker; los objetos vecinos pueden dejar entonces huecos
    /// finos).
    pub fn outline(&self, id: u32, simplify: f64) -> Outline {
        let o = (id - 1) as usize;
        let mut edges = self.edges[self.start[o]..self.start[o + 1]].to_vec();
        edges.sort_unstable();
        let w = self.labels.lat.cols + 1;
        let xy = |v: u32| ((v as usize % w) as i64, (v as usize / w) as i64);
        let mut used = vec![false; edges.len()];
        let mut out = Outline::default();
        for first in 0..edges.len() {
            if used[first] {
                continue;
            }
            let mut ring: Vec<u32> = Vec::new();
            let mut at = first;
            loop {
                used[at] = true;
                let (from, to) = edges[at];
                ring.push(from);
                // las aristas que salen de `to` y la que gira más a la derecha
                let lo = edges.partition_point(|e| e.0 < to);
                let (fx, fy) = xy(from);
                let (tx, ty) = xy(to);
                let (dx, dy) = (tx - fx, ty - fy);
                let mut next = None;
                let mut best = i64::MIN;
                for (j, e) in edges
                    .iter()
                    .enumerate()
                    .skip(lo)
                    .take_while(|(_, e)| e.0 == to)
                {
                    if used[j] && j != first {
                        continue;
                    }
                    let (ex, ey) = xy(e.1);
                    let turn = dx * (ey - ty) - dy * (ex - tx);
                    if turn > best {
                        best = turn;
                        next = Some(j);
                    }
                }
                match next {
                    Some(j) if j != first => at = j,
                    _ => break,
                }
            }
            // se conservan solo las esquinas
            let m = ring.len();
            let mut pts: Vec<[f64; 2]> = Vec::new();
            for k in 0..m {
                let (px, py) = xy(ring[(k + m - 1) % m]);
                let (cx, cy) = xy(ring[k]);
                let (nx, ny) = xy(ring[(k + 1) % m]);
                if (cx - px) * (ny - cy) - (cy - py) * (nx - cx) != 0 {
                    pts.push(self.labels.lat.corner(cx as usize, cy as usize));
                }
            }
            if pts.len() < 3 {
                continue;
            }
            if simplify > 0.0 {
                let s = simplify_ring(&pts, simplify);
                if s.len() >= 3 {
                    pts = s;
                }
            }
            // cordón de zapatos: negativo es el sentido de las agujas del reloj en el mapa
            if signed_area(&pts) < 0.0 {
                out.outers.push(pts);
            } else {
                out.holes.push(pts);
            }
        }
        out
    }
}

fn signed_area(r: &[[f64; 2]]) -> f64 {
    r.iter()
        .zip(r.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.0
}

/// Douglas–Peucker sobre un anillo cerrado, dividido en el vértice más alejado del primero.
fn simplify_ring(ring: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let n = ring.len();
    let dist2 = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    let far = (1..n)
        .max_by(|&i, &j| dist2(ring[0], ring[i]).total_cmp(&dist2(ring[0], ring[j])))
        .unwrap_or(0);
    if far == 0 {
        return ring.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[far] = true;
    let path_a: Vec<usize> = (0..=far).collect();
    let path_b: Vec<usize> = (far..n).chain(std::iter::once(0)).collect();
    for path in [path_a, path_b] {
        let mut stack = vec![(0usize, path.len() - 1)];
        while let Some((a, b)) = stack.pop() {
            if b <= a + 1 {
                continue;
            }
            let (p, q) = (ring[path[a]], ring[path[b]]);
            let (vx, vy) = (q[0] - p[0], q[1] - p[1]);
            let len2 = (vx * vx + vy * vy).max(f64::MIN_POSITIVE);
            let mut worst = (0.0, a);
            for (k, &idx) in path.iter().enumerate().take(b).skip(a + 1) {
                let c = ring[idx];
                let t = (((c[0] - p[0]) * vx + (c[1] - p[1]) * vy) / len2).clamp(0.0, 1.0);
                let d = (c[0] - p[0] - t * vx).hypot(c[1] - p[1] - t * vy);
                if d > worst.0 {
                    worst = (d, k);
                }
            }
            if worst.0 > tol {
                keep[path[worst.1]] = true;
                stack.push((a, worst.1));
                stack.push((worst.1, b));
            }
        }
    }
    ring.iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(p, _)| *p)
        .collect()
}

/// Nombres de campo DBF para las columnas y la lista de los que se cambiaron con sus nombres
/// completos. Los estadísticos de una capa (`<capa>_media`, `_desv`, `_min`, `_max`, `_p50`,
/// `_p95`) pasan a ser `L<n>_med`, `_dsv`, `_min`, `_max`, `_p50`, `_p95`, siendo `n` el orden de
/// la capa; los nombres que ya caben se conservan.
fn short_columns(columns: &[String]) -> (Vec<String>, Vec<(String, String)>) {
    const SUFFIX: [(&str, &str); 6] = [
        ("_media", "med"),
        ("_desv", "dsv"),
        ("_min", "min"),
        ("_max", "max"),
        ("_p50", "p50"),
        ("_p95", "p95"),
    ];
    let fits = |n: &str| n.len() <= 10 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let mut layers: Vec<String> = Vec::new();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut short = Vec::new();
    let mut legend = Vec::new();
    for c in columns {
        let mut name = c.clone();
        if c == "alargamiento" {
            name = "alargam".into();
        } else if let Some((suffix, code)) = SUFFIX.iter().find(|(s, _)| c.ends_with(s)) {
            let layer = &c[..c.len() - suffix.len()];
            let k = layers.iter().position(|l| l == layer).unwrap_or_else(|| {
                layers.push(layer.to_string());
                layers.len() - 1
            });
            if !fits(c) {
                name = format!("L{}_{code}", k + 1);
            }
        }
        // los nombres deben seguir siendo únicos fueran lo que fueran
        let base = name.clone();
        let mut n = 2;
        while !used.insert(name.to_ascii_uppercase()) {
            name = format!("{}{n}", &base[..base.len().min(8)]);
            n += 1;
        }
        if name != *c {
            legend.push((name.clone(), c.clone()));
        }
        short.push(name);
    }
    (short, legend)
}

/// Escribe los objetos como shapefile de polígonos (huecos incluidos) con sus atributos, y la clase
/// y su confianza cuando los objetos se clasificaron. Devuelve cuántos objetos se escribieron.
pub fn write_polygons(
    path: &std::path::Path,
    labels: &Labels,
    table: &AttrTable,
    classes: Option<&Classified>,
    simplify: f64,
    epsg: Option<u16>,
) -> Result<usize> {
    let tracer = Tracer::new(labels)?;
    let mut columns: Vec<String> = vec!["id".into()];
    // una fila de plantilla para que los decimales no se tomen por números enteros
    let mut template: Vec<String> = vec!["1".into()];
    if classes.is_some() {
        columns.push("clase".into());
        template.push("a".into());
        columns.push("confianza".into());
        template.push("0.5".into());
    }
    for n in ["x", "y"]
        .iter()
        .map(|s| s.to_string())
        .chain(table.names.iter().cloned())
    {
        columns.push(n);
        template.push("0.5".into());
    }
    let prj = epsg.and_then(prj_for_epsg);
    // nombres que caben en un campo DBF: los estadísticos de una capa pasan a ser `L<n>_med`… (el
    // nombre de la capa suele ser largo), y `<nombre>_campos.txt` dice qué es cada uno
    let (short, legend) = short_columns(&columns);
    let mut out = PolyTable::create(path, &short, &template, prj.as_deref())?;
    if !legend.is_empty() {
        let stem = path
            .file_stem()
            .map_or(String::new(), |s| s.to_string_lossy().into_owned());
        let mut text = String::from("campo\tsignificado\n");
        for (s, l) in &legend {
            text.push_str(&format!("{s}\t{l}\n"));
        }
        std::fs::write(path.with_file_name(format!("{stem}_campos.txt")), text)?;
    }
    let total = labels.count as usize;
    const BATCH: usize = 4096;
    let mut written = 0;
    for first in (0..total).step_by(BATCH) {
        check_cancel()?;
        let last = (first + BATCH).min(total);
        let outlines: Vec<Outline> = (first..last)
            .into_par_iter()
            .map(|o| tracer.outline(o as u32 + 1, simplify))
            .collect();
        for (k, ring) in outlines.iter().enumerate() {
            let o = first + k;
            let mut row: Vec<String> = vec![(o + 1).to_string()];
            if let Some(c) = classes {
                let code = c.class.get(o).copied().unwrap_or(0) as usize;
                row.push(if code > 0 {
                    c.names.get(code - 1).cloned().unwrap_or_default()
                } else {
                    String::new()
                });
                row.push(format!(
                    "{:.3}",
                    c.confidence.get(o).copied().unwrap_or(0.0)
                ));
            }
            let [cx, cy] = table.centroid.get(o).copied().unwrap_or([f64::NAN; 2]);
            row.push(format!("{cx:.3}"));
            row.push(format!("{cy:.3}"));
            for col in 0..table.cols.len() {
                row.push(table.text(col, o));
            }
            out.push_with_holes(&ring.outers, &ring.holes, &row)?;
            written += 1;
        }
    }
    out.finish()?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_layer_names_get_readable_dbf_fields() {
        let cols: Vec<String> = [
            "id",
            "area",
            "alargamiento",
            "nmds_655500_4370500_media",
            "nmds_655500_4370500_desv",
            "ortho_b1_media",
            "cobertura",
            "chm_media",
        ]
        .map(String::from)
        .to_vec();
        let (short, legend) = short_columns(&cols);
        assert_eq!(
            short,
            [
                "id",
                "area",
                "alargam",
                "L1_med",
                "L1_dsv",
                "L2_med",
                "cobertura",
                "chm_media"
            ]
        );
        // solo lo que cambió está en la leyenda, con el nombre completo
        assert!(legend.contains(&("L1_dsv".to_string(), "nmds_655500_4370500_desv".to_string())));
        assert_eq!(legend.len(), 4);
    }

    fn labels(cols: usize, rows: usize, ids: Vec<u32>) -> Labels {
        let count = ids.iter().copied().max().unwrap_or(0);
        Labels {
            lat: Lattice {
                x0: 0.0,
                y0: 0.0,
                cell: 1.0,
                cols,
                rows,
            },
            ids,
            count,
        }
    }

    #[test]
    fn a_square_is_one_clockwise_ring_of_four_corners() -> Result<()> {
        let l = labels(4, 4, vec![0, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0]);
        let t = Tracer::new(&l)?;
        let o = t.outline(1, 0.0);
        assert_eq!(o.outers.len(), 1);
        assert!(o.holes.is_empty());
        assert_eq!(o.outers[0].len(), 4);
        assert!((signed_area(&o.outers[0]) + 4.0).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn an_island_makes_a_hole_and_its_own_polygon() -> Result<()> {
        // el objeto 1 es un bloque de 5 x 5 con la celda central dada al objeto 2
        let mut ids = vec![1u32; 25];
        ids[12] = 2;
        let l = labels(5, 5, ids);
        let t = Tracer::new(&l)?;
        let ring = t.outline(1, 0.0);
        assert_eq!((ring.outers.len(), ring.holes.len()), (1, 1));
        let net = -signed_area(&ring.outers[0]) - signed_area(&ring.holes[0]);
        assert!((net - 24.0).abs() < 1e-9);
        let island = t.outline(2, 0.0);
        assert_eq!((island.outers.len(), island.holes.len()), (1, 0));
        Ok(())
    }

    #[test]
    fn cells_that_only_touch_at_a_corner_stay_two_rings() -> Result<()> {
        let l = labels(2, 2, vec![1, 0, 0, 1]);
        let o = Tracer::new(&l)?.outline(1, 0.0);
        assert_eq!((o.outers.len(), o.holes.len()), (2, 0));
        assert!(o.outers.iter().all(|r| r.len() == 4));
        Ok(())
    }

    #[test]
    fn areas_add_up_for_every_object() -> Result<()> {
        // unos pocos objetos irregulares: el área del polígono es igual al número de celdas
        let cols = 12;
        let rows = 9;
        let ids: Vec<u32> = (0..cols * rows)
            .map(|i| {
                let (x, y) = (i % cols, i / cols);
                match (x / 4 + y / 3) % 4 {
                    0 => 1,
                    1 => 2,
                    2 if x % 2 == 0 => 3,
                    _ => 0,
                }
            })
            .collect();
        let l = labels(cols, rows, ids.clone());
        let t = Tracer::new(&l)?;
        for id in 1..=l.count {
            let o = t.outline(id, 0.0);
            let area: f64 = o
                .outers
                .iter()
                .chain(&o.holes)
                .map(|r| -signed_area(r))
                .sum();
            let cells = ids.iter().filter(|&&v| v == id).count() as f64;
            assert!(
                (area - cells).abs() < 1e-9,
                "object {id}: {area} vs {cells}"
            );
        }
        Ok(())
    }

    #[test]
    fn simplifying_drops_small_steps() -> Result<()> {
        // una escalera diagonal de escalones de una celda
        let n = 12;
        let ids: Vec<u32> = (0..n * n).map(|i| u32::from(i % n <= i / n)).collect();
        let l = labels(n, n, ids);
        let t = Tracer::new(&l)?;
        let exact = t.outline(1, 0.0);
        let smooth = t.outline(1, 1.0);
        assert!(smooth.outers[0].len() < exact.outers[0].len());
        Ok(())
    }

    #[test]
    fn writes_a_shapefile_with_holes() -> Result<()> {
        let mut ids = vec![1u32; 25];
        ids[12] = 2;
        let l = labels(5, 5, ids);
        let table = AttrTable {
            names: vec!["area".into()],
            cols: vec![vec![24.0, 1.0]],
            objects: 2,
            centroid: vec![[2.5, 2.5], [2.5, 2.5]],
        };
        let path = std::env::temp_dir().join(format!("rl_obia_shp_{}.shp", std::process::id()));
        let n = write_polygons(&path, &l, &table, None, 0.0, None)?;
        assert_eq!(n, 2);
        let mut reader =
            shapefile::Reader::from_path(&path).map_err(|e| Error::Format(e.to_string()))?;
        let shapes = reader.read().map_err(|e| Error::Format(e.to_string()))?;
        let rings: Vec<usize> = shapes
            .iter()
            .map(|(s, _)| match s {
                shapefile::Shape::Polygon(p) => p.rings().len(),
                _ => 0,
            })
            .collect();
        for ext in ["shp", "shx", "dbf", "prj", "cpg"] {
            let _ = std::fs::remove_file(path.with_extension(ext));
        }
        assert_eq!(rings, vec![2, 1]);
        Ok(())
    }
}
