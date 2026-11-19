//! Datos vectoriales: los polígonos, círculos y rectángulos con los que se recortan los puntos, y
//! shapefiles de ESRI leídos y escritos para ellos (rodales, parcelas de inventario, cimas de
//! árboles, copas, POI/AOI).

use std::collections::HashMap;
use std::path::Path;

use shapefile::dbase::{self, FieldName, FieldValue, TableWriterBuilder};
use shapefile::{Point as ShpPoint, PointZ, PolygonRing, Shape};

use crate::{Error, Result};

/// Vértices a partir de los cuales un polígono recibe una máscara de consulta: un punto lejos de
/// todo borde se responde entonces con una tabla en lugar de recorrer todos los bordes.
const MASK_FROM: usize = 200;

/// Un polígono con huecos y partes: un punto está dentro cuando está dentro de un número impar de
/// anillos.
#[derive(Clone, Debug)]
pub struct Polygon {
    pub rings: Vec<Vec<[f64; 2]>>,
    /// min x, min y, max x, max y.
    pub bbox: [f64; 4],
    mask: Option<Mask>,
}

#[derive(Clone, Debug)]
struct Mask {
    x0: f64,
    y0: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    /// 0 fuera, 1 dentro, 2 cerca de un borde.
    state: Vec<u8>,
}

impl Polygon {
    pub fn new(rings: Vec<Vec<[f64; 2]>>) -> Self {
        let mut bbox = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in rings.iter().flatten() {
            bbox = [
                bbox[0].min(p[0]),
                bbox[1].min(p[1]),
                bbox[2].max(p[0]),
                bbox[3].max(p[1]),
            ];
        }
        let vertices: usize = rings.iter().map(Vec::len).sum();
        let mask =
            (vertices >= MASK_FROM && bbox[0].is_finite()).then(|| Mask::build(&rings, bbox));
        Self { rings, bbox, mask }
    }

    #[inline]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        if x < self.bbox[0] || x > self.bbox[2] || y < self.bbox[1] || y > self.bbox[3] {
            return false;
        }
        if let Some(m) = &self.mask {
            match m.at(x, y) {
                0 => return false,
                1 => return true,
                _ => {}
            }
        }
        self.contains_exact(x, y)
    }

    /// Prueba par-impar sobre todo anillo.
    pub fn contains_exact(&self, x: f64, y: f64) -> bool {
        let mut inside = false;
        for ring in &self.rings {
            let n = ring.len();
            if n < 3 {
                continue;
            }
            let mut j = n - 1;
            for i in 0..n {
                let (a, b) = (ring[i], ring[j]);
                if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0]
                {
                    inside = !inside;
                }
                j = i;
            }
        }
        inside
    }

    /// El área: el neto de las áreas con signo de los anillos (los anillos exteriores van en el
    /// sentido de las agujas del reloj y los huecos al revés, como los guardan los shapefiles).
    pub fn area(&self) -> f64 {
        let net: f64 = self.rings.iter().map(|r| ring_signed_area(r)).sum();
        net.abs()
    }
}

/// El doble de nada: el área con signo por la suma del cordón de zapatos (positiva en sentido
/// antihorario).
fn ring_signed_area(r: &[[f64; 2]]) -> f64 {
    let n = r.len();
    if n < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    let mut j = n - 1;
    for i in 0..n {
        s += (r[j][0] + r[i][0]) * (r[j][1] - r[i][1]);
        j = i;
    }
    s / 2.0
}

impl Mask {
    fn build(rings: &[Vec<[f64; 2]>], bbox: [f64; 4]) -> Self {
        let (w, h) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
        let cell = (w.max(h) / 1024.0).max(1e-9);
        let (cols, rows) = ((w / cell) as usize + 2, (h / cell) as usize + 2);
        let mut state = vec![0u8; cols * rows];
        // Dentro: la línea central de cada fila contra cada borde, rellenando entre pares de
        // cruces.
        let mut xs: Vec<f64> = Vec::new();
        for r in 0..rows {
            let y = bbox[1] + (r as f64 + 0.5) * cell;
            xs.clear();
            for ring in rings {
                let n = ring.len();
                if n < 3 {
                    continue;
                }
                let mut j = n - 1;
                for i in 0..n {
                    let (a, b) = (ring[i], ring[j]);
                    if (a[1] > y) != (b[1] > y) {
                        xs.push((b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0]);
                    }
                    j = i;
                }
            }
            xs.sort_by(f64::total_cmp);
            for pair in xs.as_chunks::<2>().0 {
                let c0 = (((pair[0] - bbox[0]) / cell - 0.5).ceil().max(0.0)) as usize;
                let c1 = (((pair[1] - bbox[0]) / cell - 0.5).floor()).min(cols as f64 - 1.0);
                if c1 >= 0.0 {
                    for c in c0..=(c1 as usize) {
                        if let Some(s) = state.get_mut(r * cols + c) {
                            *s = 1;
                        }
                    }
                }
            }
        }
        // Bordes: cada celda por la que pasa un borde, y sus vecinas, se decide exactamente.
        for ring in rings {
            let n = ring.len();
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                let steps = (len / (cell * 0.5)).ceil().max(1.0) as usize;
                for k in 0..=steps {
                    let t = k as f64 / steps as f64;
                    let (x, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                    let (c, r) = (
                        ((x - bbox[0]) / cell) as isize,
                        ((y - bbox[1]) / cell) as isize,
                    );
                    for dr in -1..=1 {
                        for dc in -1..=1 {
                            let (cc, rr) = (c + dc, r + dr);
                            if cc >= 0 && rr >= 0 && (cc as usize) < cols && (rr as usize) < rows {
                                state[rr as usize * cols + cc as usize] = 2;
                            }
                        }
                    }
                }
            }
        }
        Self {
            x0: bbox[0],
            y0: bbox[1],
            cell,
            cols,
            rows,
            state,
        }
    }

    #[inline]
    fn at(&self, x: f64, y: f64) -> u8 {
        let (c, r) = (
            ((x - self.x0) / self.cell) as usize,
            ((y - self.y0) / self.cell) as usize,
        );
        if c >= self.cols || r >= self.rows {
            return 2;
        }
        self.state[r * self.cols + c]
    }
}

/// La forma de una región.
#[derive(Clone, Debug)]
pub enum Shape2 {
    Polygon(Polygon),
    Circle {
        cx: f64,
        cy: f64,
        r: f64,
    },
    /// min x, min y, max x, max y.
    Rect([f64; 4]),
    /// Un polígono ampliado (distancia > 0) o reducido (< 0) una distancia en unidades del mapa:
    /// los puntos cuya distancia al polígono está dentro de ella, con esquinas redondeadas.
    Buffered(Box<Buffered>),
}

/// Un polígono con una distancia de búfer y el índice que usa su prueba de distancia.
#[derive(Clone, Debug)]
pub struct Buffered {
    pub poly: Polygon,
    pub dist: f64,
    bbox: [f64; 4],
    seg_cell: f64,
    seg_cols: usize,
    seg_rows: usize,
    /// Por celda, los segmentos (anillo, índice) a menos de `|dist|` de ella.
    cells: Vec<Vec<(u32, u32)>>,
}

impl Buffered {
    pub fn new(poly: Polygon, dist: f64) -> Self {
        let d = dist.abs();
        let b = poly.bbox;
        let bbox = if dist > 0.0 {
            [b[0] - d, b[1] - d, b[2] + d, b[3] + d]
        } else {
            b
        };
        let diag = (b[2] - b[0]).hypot(b[3] - b[1]).max(1e-9);
        let seg_cell = d.max(diag / 256.0).max(1e-9);
        let cols = (((b[2] - b[0] + 2.0 * d) / seg_cell).ceil() as usize + 1).max(1);
        let rows = (((b[3] - b[1] + 2.0 * d) / seg_cell).ceil() as usize + 1).max(1);
        let mut cells = vec![Vec::new(); cols * rows];
        for (ri, ring) in poly.rings.iter().enumerate() {
            let n = ring.len();
            for i in 0..n {
                let (a, e) = (ring[i], ring[(i + 1) % n]);
                let (x0, x1) = (a[0].min(e[0]) - d, a[0].max(e[0]) + d);
                let (y0, y1) = (a[1].min(e[1]) - d, a[1].max(e[1]) + d);
                let cx = |x: f64| (((x - (b[0] - d)) / seg_cell).floor().max(0.0) as usize).min(cols - 1);
                let cy = |y: f64| (((y - (b[1] - d)) / seg_cell).floor().max(0.0) as usize).min(rows - 1);
                for r in cy(y0)..=cy(y1) {
                    for c in cx(x0)..=cx(x1) {
                        cells[r * cols + c].push((ri as u32, i as u32));
                    }
                }
            }
        }
        Self {
            poly,
            dist,
            bbox,
            seg_cell,
            seg_cols: cols,
            seg_rows: rows,
            cells,
        }
    }

    pub fn bbox(&self) -> [f64; 4] {
        self.bbox
    }

    /// Si algún segmento del contorno está a menos de `|dist|` del punto.
    fn near_boundary(&self, x: f64, y: f64) -> bool {
        let d = self.dist.abs();
        let b = self.poly.bbox;
        let c = ((x - (b[0] - d)) / self.seg_cell).floor();
        let r = ((y - (b[1] - d)) / self.seg_cell).floor();
        if c < 0.0 || r < 0.0 || c as usize >= self.seg_cols || r as usize >= self.seg_rows {
            return false;
        }
        self.cells[r as usize * self.seg_cols + c as usize]
            .iter()
            .any(|&(ri, i)| {
                let ring = &self.poly.rings[ri as usize];
                let (a, e) = (ring[i as usize], ring[(i as usize + 1) % ring.len()]);
                dist2_to_segment(x, y, a, e) <= d * d
            })
    }

    #[inline]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        if x < self.bbox[0] || x > self.bbox[2] || y < self.bbox[1] || y > self.bbox[3] {
            return false;
        }
        let inside = self.poly.contains(x, y);
        if self.dist >= 0.0 {
            inside || self.near_boundary(x, y)
        } else {
            inside && !self.near_boundary(x, y)
        }
    }
}

fn dist2_to_segment(x: f64, y: f64, a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((x - a[0]) * dx + (y - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (px, py) = (a[0] + t * dx, a[1] + t * dy);
    (x - px).powi(2) + (y - py).powi(2)
}

/// Una región con la que recortar: un rodal, una parcela de inventario, un área de interés.
#[derive(Clone, Debug)]
pub struct Region {
    pub name: String,
    pub shape: Shape2,
    /// Los valores de la tabla de atributos, como texto.
    pub attrs: Vec<(String, String)>,
}

impl Region {
    pub fn bbox(&self) -> [f64; 4] {
        match &self.shape {
            Shape2::Polygon(p) => p.bbox,
            Shape2::Circle { cx, cy, r } => [cx - r, cy - r, cx + r, cy + r],
            Shape2::Rect(b) => *b,
            Shape2::Buffered(b) => b.bbox(),
        }
    }

    #[inline]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match &self.shape {
            Shape2::Polygon(p) => p.contains(x, y),
            Shape2::Circle { cx, cy, r } => (x - cx).powi(2) + (y - cy).powi(2) <= r * r,
            Shape2::Rect(b) => x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3],
            Shape2::Buffered(b) => b.contains(x, y),
        }
    }

    pub fn area(&self) -> f64 {
        match &self.shape {
            Shape2::Polygon(p) => p.area(),
            Shape2::Circle { r, .. } => std::f64::consts::PI * r * r,
            Shape2::Rect(b) => (b[2] - b[0]) * (b[3] - b[1]),
            // el del polígono más la franja alrededor de su contorno, en primer orden (las esquinas
            // redondeadas añaden pi d^2 por cada vuelta de 360 grados)
            Shape2::Buffered(b) => {
                let perimeter: f64 = b
                    .poly
                    .rings
                    .iter()
                    .map(|r| {
                        let n = r.len();
                        (0..n)
                            .map(|i| (r[(i + 1) % n][0] - r[i][0]).hypot(r[(i + 1) % n][1] - r[i][1]))
                            .sum::<f64>()
                    })
                    .sum();
                (b.poly.area() + b.dist * perimeter + std::f64::consts::PI * b.dist * b.dist.abs())
                    .max(0.0)
            }
        }
    }

    /// Esta región ampliada `dist` unidades del mapa (reducida cuando es negativa). Un polígono no
    /// se convierte en un polígono mayor (como hace `geomops::buffer` con la biblioteca `geo`): los
    /// puntos se comparan con la distancia a su contorno con una rejilla de segmentos. Medido en un
    /// polígono de 50 000 vértices y 3 millones de puntos, eso tarda 2 ms en prepararse y 1,3 s en
    /// comprobar, donde construir el polígono del búfer tarda 5,6 s (16 s para un búfer de 100 m)
    /// antes de comprobar los mismos puntos en 0,4 s; y el polígono solo aproxima los arcos
    /// (discrepó de la distancia exacta en 1 800 de 2 millones de puntos a 100 m). El polígono gana
    /// solo cuando un búfer sirve a decenas de millones de puntos, lo que un polígono de un recorte
    /// rara vez encuentra. Los círculos y rectángulos se quedan como son; un polígono pasa a ser un
    /// polígono con búfer. La distancia está en las unidades del sistema de coordenadas de la
    /// región: grados si es geográfico.
    pub fn buffered(&self, dist: f64) -> Region {
        if dist == 0.0 {
            return self.clone();
        }
        let shape = match &self.shape {
            Shape2::Polygon(p) => Shape2::Buffered(Box::new(Buffered::new(p.clone(), dist))),
            Shape2::Buffered(b) => {
                Shape2::Buffered(Box::new(Buffered::new(b.poly.clone(), b.dist + dist)))
            }
            Shape2::Circle { cx, cy, r } => Shape2::Circle {
                cx: *cx,
                cy: *cy,
                r: (r + dist).max(0.0),
            },
            // un rectángulo ampliado es un rectángulo redondeado: como polígono
            Shape2::Rect(b) => {
                let ring = vec![[b[0], b[1]], [b[0], b[3]], [b[2], b[3]], [b[2], b[1]]];
                Shape2::Buffered(Box::new(Buffered::new(Polygon::new(vec![ring]), dist)))
            }
        };
        Region {
            name: self.name.clone(),
            shape,
            attrs: self.attrs.clone(),
        }
    }

    /// Las partes separadas de un polígono de varias partes (cada anillo exterior con los huecos
    /// que hay dentro) como regiones llamadas `<nombre>_p1`, `<nombre>_p2`… Una región de una
    /// parte, o que no es un polígono, se devuelve tal cual.
    pub fn split_parts(&self) -> Vec<Region> {
        let Shape2::Polygon(p) = &self.shape else {
            return vec![self.clone()];
        };
        let rings = &p.rings;
        // profundidad de un anillo: cuántos otros anillos contienen su primer vértice
        let holds = |outer: usize, inner: usize| {
            rings[inner].first().is_some_and(|v| {
                Polygon::new(vec![rings[outer].clone()]).contains_exact(v[0], v[1])
            })
        };
        let depth: Vec<usize> = (0..rings.len())
            .map(|i| (0..rings.len()).filter(|&j| j != i && holds(j, i)).count())
            .collect();
        let outers: Vec<usize> = (0..rings.len()).filter(|&i| depth[i] % 2 == 0).collect();
        if outers.len() <= 1 {
            return vec![self.clone()];
        }
        outers
            .iter()
            .enumerate()
            .map(|(k, &o)| {
                let mut parts = vec![rings[o].clone()];
                for h in 0..rings.len() {
                    // un hueco pertenece al anillo exterior más interno que lo contiene
                    if depth[h] % 2 == 1 && holds(o, h) && depth[o] + 1 == depth[h] {
                        parts.push(rings[h].clone());
                    }
                }
                Region {
                    name: format!("{}_p{}", self.name, k + 1),
                    shape: Shape2::Polygon(Polygon::new(parts)),
                    attrs: self.attrs.clone(),
                }
            })
            .collect()
    }
}

/// Halla las regiones que contienen un punto sin comprobarlas todas.
pub struct RegionIndex<'a> {
    regions: &'a [Region],
    bbox: [f64; 4],
    cell: f64,
    cols: usize,
    rows: usize,
    /// Por celda de la rejilla, las regiones cuya caja la alcanza; vacío cuando hay pocas regiones.
    cells: Vec<Vec<u32>>,
}

impl<'a> RegionIndex<'a> {
    pub fn new(regions: &'a [Region]) -> Self {
        let mut bbox = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for r in regions {
            let b = r.bbox();
            bbox = [
                bbox[0].min(b[0]),
                bbox[1].min(b[1]),
                bbox[2].max(b[2]),
                bbox[3].max(b[3]),
            ];
        }
        let mut idx = Self {
            regions,
            bbox,
            cell: 1.0,
            cols: 0,
            rows: 0,
            cells: Vec::new(),
        };
        if regions.len() > 8 && bbox[0].is_finite() {
            let (w, h) = ((bbox[2] - bbox[0]).max(1e-9), (bbox[3] - bbox[1]).max(1e-9));
            let target = (regions.len() as f64 * 4.0).min(1.0e6);
            idx.cell = ((w * h) / target).sqrt().max(w.max(h) / 4096.0);
            idx.cols = (w / idx.cell) as usize + 1;
            idx.rows = (h / idx.cell) as usize + 1;
            idx.cells = vec![Vec::new(); idx.cols * idx.rows];
            for (k, r) in regions.iter().enumerate() {
                let b = r.bbox();
                let (c0, c1) = idx.span(b[0], b[2], bbox[0], idx.cols);
                let (r0, r1) = idx.span(b[1], b[3], bbox[1], idx.rows);
                for rr in r0..=r1 {
                    for cc in c0..=c1 {
                        idx.cells[rr * idx.cols + cc].push(k as u32);
                    }
                }
            }
        }
        idx
    }

    fn span(&self, lo: f64, hi: f64, origin: f64, n: usize) -> (usize, usize) {
        let f = |v: f64| (((v - origin) / self.cell).floor().max(0.0) as usize).min(n - 1);
        (f(lo), f(hi))
    }

    /// La extensión de todas las regiones.
    pub fn bbox(&self) -> [f64; 4] {
        self.bbox
    }

    /// Llama a `f` con cada región que contiene el punto.
    #[inline]
    pub fn for_each(&self, x: f64, y: f64, mut f: impl FnMut(usize)) {
        if self.cells.is_empty() {
            for (k, r) in self.regions.iter().enumerate() {
                if r.contains(x, y) {
                    f(k);
                }
            }
            return;
        }
        let (c, r) = (
            ((x - self.bbox[0]) / self.cell).floor(),
            ((y - self.bbox[1]) / self.cell).floor(),
        );
        if c < 0.0 || r < 0.0 || c as usize >= self.cols || r as usize >= self.rows {
            return;
        }
        for &k in &self.cells[r as usize * self.cols + c as usize] {
            if self
                .regions
                .get(k as usize)
                .is_some_and(|reg| reg.contains(x, y))
            {
                f(k as usize);
            }
        }
    }

    /// La primera región (en el orden del archivo) que contiene el punto.
    #[inline]
    pub fn first(&self, x: f64, y: f64) -> Option<usize> {
        let mut best: Option<usize> = None;
        self.for_each(x, y, |k| best = Some(best.map_or(k, |b| b.min(k))));
        best
    }

    #[inline]
    pub fn any(&self, x: f64, y: f64) -> bool {
        let mut hit = false;
        self.for_each(x, y, |_| hit = true);
        hit
    }
}

/// Cómo se leen las regiones de un archivo.
#[derive(Clone, Debug, Default)]
pub struct ReadOptions {
    /// El atributo que da nombre a cada región: el nombre de un campo o su posición contando desde
    /// 1.
    pub name_field: Option<String>,
    /// Radio para las formas de puntos (centros de parcelas), en unidades del mapa.
    pub radius: Option<f64>,
    /// Un atributo que guarda el radio propio de cada punto.
    pub radius_field: Option<String>,
    /// Las parcelas dadas como puntos son cuadradas (o rectangulares) en lugar de circulares.
    pub square: bool,
    /// Lado fijo de los cuadrados, o los atributos que guardan los lados a lo largo de x e y
    /// (metros).
    pub side: Option<f64>,
    pub side_x_field: Option<String>,
    pub side_y_field: Option<String>,
}

/// Los nombres largos de los campos de una capa de métricas de parcelas cuyo `_campos.txt` falta:
/// nombre corto (en minúsculas) a nombre largo, para la altura de corte cuyas columnas coinciden
/// con más campos.
fn restored_plot_names(fields: &[String]) -> HashMap<String, String> {
    let mut best: (usize, HashMap<String, String>) = (0, HashMap::new());
    for hb in [2.0, 1.0, 1.3, 0.5, 1.5, 3.0, 4.0, 5.0, 10.0] {
        let mut cols = vec!["Muestra".to_string(), "Area".to_string()];
        cols.extend(crate::metrics::sample_header(hb));
        cols.push(CAPTURE_FIELD.to_string());
        let map: HashMap<String, String> = dbf_names(&cols)
            .into_iter()
            .map(|(s, l)| (s.to_lowercase(), l))
            .collect();
        let hits = fields
            .iter()
            .filter(|f| {
                map.get(&f.to_lowercase())
                    .is_some_and(|l| !l.eq_ignore_ascii_case(f))
            })
            .count();
        if hits > best.0 {
            best = (hits, map);
        }
    }
    // solo cuando se reconoce la mayor parte de la tabla: una capa de otra cosa se deja como está
    if best.0 * 2 >= fields.len() {
        best.1
    } else {
        HashMap::new()
    }
}

/// La tabla de atributos de un shapefile como texto: los nombres de los campos, con los nombres
/// largos restaurados de `<nombre>_campos.txt` cuando el proceso que la escribió tuvo que
/// acortarlos, y las filas.
pub(crate) fn attribute_table(path: &Path) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let dbf = path.with_extension("dbf");
    let bad = |e: dbase::Error| Error::Format(format!("{}: {e}", dbf.display()));
    let mut reader = dbase::Reader::from_path(&dbf).map_err(bad)?;
    let order: Vec<String> = reader
        .fields()
        .iter()
        .map(|f| f.name().to_string())
        .collect();
    let records = reader.read().map_err(bad)?;
    let stem = path
        .file_stem()
        .map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let long: HashMap<String, String> =
        std::fs::read_to_string(path.with_file_name(format!("{stem}_campos.txt")))
            .map(|t| {
                t.lines()
                    .skip(1)
                    .filter_map(|l| l.split_once('\t'))
                    .map(|(s, l)| (s.trim().to_lowercase(), l.trim().to_string()))
                    .collect()
            })
            .unwrap_or_default();
    // Sin el `_campos.txt` (una capa copiada sola) los nombres largos de una capa de parcelas se
    // vuelven a calcular: el acortamiento es determinista, así que la tabla de métricas de parcelas
    // da los mismos nombres cortos.
    let long = if long.is_empty() {
        restored_plot_names(&order)
    } else {
        long
    };
    let names = order
        .iter()
        .map(|n| {
            long.get(&n.to_lowercase())
                .cloned()
                .unwrap_or_else(|| n.clone())
        })
        .collect();
    let rows = records
        .into_iter()
        .map(|r| {
            let map: HashMap<String, FieldValue> = r.into();
            order
                .iter()
                .map(|k| map.get(k).map_or(String::new(), field_text))
                .collect()
        })
        .collect();
    Ok((names, rows))
}

fn field_text(v: &FieldValue) -> String {
    match v {
        FieldValue::Character(Some(s)) | FieldValue::Memo(s) => s.trim().to_string(),
        FieldValue::Numeric(Some(n)) | FieldValue::Currency(n) | FieldValue::Double(n) => {
            num_text(*n)
        }
        FieldValue::Float(Some(n)) => num_text(f64::from(*n)),
        FieldValue::Integer(i) => i.to_string(),
        FieldValue::Logical(Some(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn num_text(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn ring_points<P>(points: &[P], xy: impl Fn(&P) -> [f64; 2]) -> Vec<[f64; 2]> {
    points.iter().map(xy).collect()
}

/// Lee regiones de un shapefile (polígonos, o puntos con un radio) o de una lista de texto.
pub fn read_regions(path: &Path, opts: &ReadOptions) -> Result<Vec<Region>> {
    crate::check_vector_format(path)?;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "shp" {
        read_shapefile(path, opts)
    } else if ext == "geojson" || ext == "json" {
        crate::geojson::read_regions(path, opts)
    } else if ext == "kml" || ext == "kmz" {
        crate::kml::read_regions(path, opts)
    } else if ext == "gpkg" {
        crate::geojson::regions_of(&crate::gpkg::read_layer(path, None)?, opts)
    } else {
        read_text_regions(path, opts)
    }
}

fn read_shapefile(path: &Path, opts: &ReadOptions) -> Result<Vec<Region>> {
    // Los campos de la tabla en el orden propio del archivo (un registro llega como un mapa sin
    // orden).
    let order: Vec<String> = dbase::Reader::from_path(path.with_extension("dbf"))
        .map(|r| r.fields().iter().map(|f| f.name().to_string()).collect())
        .unwrap_or_default();
    let mut reader = shapefile::Reader::from_path(path).map_err(shp_open_err(path))?;
    let mut out = Vec::new();
    for (n, item) in reader.iter_shapes_and_records().enumerate() {
        let (shape, record) = item.map_err(shp_err)?;
        let map: HashMap<String, FieldValue> = record.into();
        let attrs: Vec<(String, String)> = order
            .iter()
            .filter_map(|k| map.get(k).map(|v| (k.clone(), field_text(v))))
            .collect();
        let by_name = |name: &str| {
            attrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        let name = match &opts.name_field {
            Some(f) => match f.parse::<usize>() {
                // Un número es una posición contando desde 1 entre los campos, como en la opción de
                // forma compatible con FUSION.
                Ok(i) if by_name(f).is_none() => {
                    attrs.get(i.saturating_sub(1)).map(|(_, v)| v.clone())
                }
                _ => by_name(f),
            },
            None => None,
        }
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{:03}", n + 1));
        let radius = opts
            .radius_field
            .as_deref()
            .and_then(by_name)
            .and_then(|v| v.parse::<f64>().ok())
            .or(opts.radius);
        let shape = match shape {
            Shape::Polygon(p) => Shape2::Polygon(Polygon::new(
                p.rings()
                    .iter()
                    .map(|r| ring_points(r.points(), |q: &ShpPoint| [q.x, q.y]))
                    .collect(),
            )),
            Shape::PolygonM(p) => Shape2::Polygon(Polygon::new(
                p.rings()
                    .iter()
                    .map(|r| ring_points(r.points(), |q| [q.x, q.y]))
                    .collect(),
            )),
            Shape::PolygonZ(p) => Shape2::Polygon(Polygon::new(
                p.rings()
                    .iter()
                    .map(|r| ring_points(r.points(), |q| [q.x, q.y]))
                    .collect(),
            )),
            Shape::Point(p) => plot_at(p.x, p.y, radius, opts, &by_name, path)?,
            Shape::PointM(p) => plot_at(p.x, p.y, radius, opts, &by_name, path)?,
            Shape::PointZ(p) => plot_at(p.x, p.y, radius, opts, &by_name, path)?,
            Shape::NullShape => continue,
            other => {
                return Err(Error::Unsupported(format!(
                    "{}: las formas de tipo {} no sirven para recortar puntos (usa polígonos, o puntos con un radio)",
                    path.display(),
                    other.shapetype()
                )));
            }
        };
        out.push(Region { name, shape, attrs });
    }
    Ok(out)
}

/// Una parcela centrada en un punto: un círculo, o un cuadrado o rectángulo (lados de los atributos
/// o fijos, en metros).
fn plot_at(
    x: f64,
    y: f64,
    radius: Option<f64>,
    opts: &ReadOptions,
    by_name: &dyn Fn(&str) -> Option<String>,
    path: &Path,
) -> Result<Shape2> {
    if !opts.square {
        return circle(x, y, radius, path);
    }
    let side = |field: &Option<String>| {
        field
            .as_deref()
            .and_then(by_name)
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| *v > 0.0)
    };
    let w = side(&opts.side_x_field).or(opts.side).filter(|v| *v > 0.0);
    let h = side(&opts.side_y_field).or(w);
    match (w, h) {
        (Some(w), Some(h)) => Ok(Shape2::Rect([
            x - w / 2.0,
            y - h / 2.0,
            x + w / 2.0,
            y + h / 2.0,
        ])),
        _ => Err(Error::Param(format!(
            "{}: las muestras son cuadradas: indica el lado (o el campo que lo tiene)",
            path.display()
        ))),
    }
}

fn circle(x: f64, y: f64, radius: Option<f64>, path: &Path) -> Result<Shape2> {
    match radius.filter(|r| *r > 0.0) {
        Some(r) => Ok(Shape2::Circle { cx: x, cy: y, r }),
        None => Err(Error::Param(format!(
            "{}: las formas son puntos: indica un radio (o el campo que lo tiene)",
            path.display()
        ))),
    }
}

/// Una lista de texto, una región por línea: `nombre x y radio`, `x y radio` o `nombre minx miny
/// maxx maxy`; separados por comas, puntos y comas, tabuladores o espacios; una primera línea que
/// no es de números es una cabecera.
fn read_text_regions(path: &Path, opts: &ReadOptions) -> Result<Vec<Region>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line
            .split([',', ';', '\t', ' '])
            .filter(|t| !t.is_empty())
            .collect();
        let nums: Vec<f64> = tokens.iter().filter_map(|t| t.parse().ok()).collect();
        if tokens.is_empty() || nums.len() < 2 {
            continue;
        }
        let named = tokens.first().is_some_and(|t| t.parse::<f64>().is_err());
        let name = if named {
            tokens[0].to_string()
        } else {
            format!("{:03}", out.len() + 1)
        };
        let shape = match nums.len() {
            2 => plot_at(nums[0], nums[1], opts.radius, opts, &|_| None, path)?,
            3 => Shape2::Circle {
                cx: nums[0],
                cy: nums[1],
                r: nums[2],
            },
            4 => Shape2::Rect([nums[0], nums[1], nums[2], nums[3]]),
            _ => continue,
        };
        out.push(Region {
            name,
            shape,
            attrs: Vec::new(),
        });
    }
    if out.is_empty() {
        return Err(Error::Format(format!(
            "{}: no se encuentran zonas en él",
            path.display()
        )));
    }
    Ok(out)
}

fn shp_err(e: shapefile::Error) -> Error {
    Error::Format(format!("shapefile: {e}"))
}

/// El error de abrir un shapefile, con su ruta.
fn shp_open_err(path: &Path) -> impl Fn(shapefile::Error) -> Error + '_ {
    move |e| Error::Format(format!("{}: {e}", path.display()))
}

/// Un valor de una tabla de atributos.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Text(String),
    Num(f64),
    Int(i64),
}

/// Una columna de una tabla de atributos.
#[derive(Clone, Debug, PartialEq)]
pub enum ColKind {
    /// Texto de hasta tantos bytes.
    Text(u8),
    /// Un número: anchura total y decimales.
    Num(u8, u8),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub columns: Vec<(String, ColKind)>,
    pub rows: Vec<Vec<Val>>,
}

impl Table {
    fn builder(&self) -> Result<TableWriterBuilder> {
        let mut b = TableWriterBuilder::new();
        for (name, kind) in &self.columns {
            let n = FieldName::try_from(name.as_str())
                .map_err(|e| Error::Param(format!("field '{name}': {e}")))?;
            b = match kind {
                ColKind::Text(len) => b.add_character_field(n, *len),
                ColKind::Num(len, dec) => b.add_numeric_field(n, *len, *dec),
            };
        }
        Ok(b)
    }

    fn record(&self, row: usize) -> dbase::Record {
        let mut rec = dbase::Record::default();
        for (k, (name, kind)) in self.columns.iter().enumerate() {
            let v = self.rows.get(row).and_then(|r| r.get(k));
            let value = match (kind, v) {
                (ColKind::Text(len), Some(Val::Text(s))) => {
                    FieldValue::Character(Some(truncate(s, usize::from(*len))))
                }
                (ColKind::Text(len), Some(Val::Num(n))) => {
                    FieldValue::Character(Some(truncate(&num_text(*n), usize::from(*len))))
                }
                (ColKind::Text(len), Some(Val::Int(n))) => {
                    FieldValue::Character(Some(truncate(&n.to_string(), usize::from(*len))))
                }
                (ColKind::Num(..), Some(Val::Num(n))) => FieldValue::Numeric(Some(*n)),
                (ColKind::Num(..), Some(Val::Int(n))) => FieldValue::Numeric(Some(*n as f64)),
                (ColKind::Num(..), _) => FieldValue::Numeric(None),
                (ColKind::Text(_), _) => FieldValue::Character(None),
            };
            rec.insert(name.clone(), value);
        }
        rec
    }
}

/// Como mucho `len` bytes de una cadena, cortados en el límite de un carácter.
fn truncate(s: &str, len: usize) -> String {
    let mut end = len.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Escribe puntos como shapefile, 3D cuando `with_z`; la fila `k` de `table` describe el punto `k`.
pub fn write_points(
    path: &Path,
    pts: &[[f64; 3]],
    with_z: bool,
    table: &Table,
    prj: Option<&str>,
) -> Result<()> {
    let mut w = shapefile::Writer::from_path(path, table.builder()?).map_err(shp_open_err(path))?;
    for (k, p) in pts.iter().enumerate() {
        let rec = table.record(k);
        if with_z {
            w.write_shape_and_record(&PointZ::new(p[0], p[1], p[2], shapefile::NO_DATA), &rec)
                .map_err(shp_err)?;
        } else {
            w.write_shape_and_record(&ShpPoint::new(p[0], p[1]), &rec)
                .map_err(shp_err)?;
        }
    }
    drop(w);
    write_prj(path, prj)
}

/// Escribe polígonos (cada anillo un anillo exterior, en el sentido de las agujas del reloj) como
/// shapefile.
pub fn write_polygons(
    path: &Path,
    polys: &[Polygon],
    table: &Table,
    prj: Option<&str>,
) -> Result<()> {
    let mut w = shapefile::Writer::from_path(path, table.builder()?).map_err(shp_open_err(path))?;
    for (k, poly) in polys.iter().enumerate() {
        let rings: Vec<PolygonRing<ShpPoint>> = poly
            .rings
            .iter()
            .filter(|r| r.len() >= 3)
            .map(|r| PolygonRing::Outer(r.iter().map(|p| ShpPoint::new(p[0], p[1])).collect()))
            .collect();
        if rings.is_empty() {
            continue;
        }
        let shape = shapefile::Polygon::with_rings(rings);
        w.write_shape_and_record(&shape, &table.record(k))
            .map_err(shp_err)?;
    }
    drop(w);
    write_prj(path, prj)
}

/// Escribe polígonos con huecos como shapefile: el elemento `k` es una lista de anillos, cada uno
/// marcado como hueco o no; la fila `k` de `table` lo describe. Los elementos sin anillos se dejan
/// fuera. Los anillos se cierran y se orientan como pide el formato. `prj` es el texto del archivo
/// `.prj`.
pub fn write_polygon_layer(
    path: &Path,
    features: &[Vec<(Vec<[f64; 2]>, bool)>],
    table: &Table,
    prj: Option<&str>,
) -> Result<()> {
    let mut w = shapefile::Writer::from_path(path, table.builder()?).map_err(shp_open_err(path))?;
    for (k, rings) in features.iter().enumerate() {
        let rings: Vec<PolygonRing<ShpPoint>> = rings
            .iter()
            .filter(|(r, _)| r.len() >= 3)
            .map(|(r, hole)| {
                let pts: Vec<ShpPoint> = r.iter().map(|p| ShpPoint::new(p[0], p[1])).collect();
                if *hole {
                    PolygonRing::Inner(pts)
                } else {
                    PolygonRing::Outer(pts)
                }
            })
            .collect();
        if rings.is_empty() {
            // el escritor del formato aquí no tiene forma vacía: un elemento sin geometría se deja
            // fuera
            continue;
        }
        let shape = shapefile::Polygon::with_rings(rings);
        w.write_shape_and_record(&shape, &table.record(k))
            .map_err(shp_err)?;
    }
    drop(w);
    write_prj(path, prj)
}

/// Escribe polilíneas (curvas de nivel, perfiles) como shapefile.
pub fn write_polylines(
    path: &Path,
    lines: &[Vec<[f64; 3]>],
    with_z: bool,
    table: &Table,
    prj: Option<&str>,
) -> Result<()> {
    let mut w = shapefile::Writer::from_path(path, table.builder()?).map_err(shp_open_err(path))?;
    for (k, line) in lines.iter().enumerate() {
        if line.len() < 2 {
            continue;
        }
        if with_z {
            let pts: Vec<PointZ> = line
                .iter()
                .map(|p| PointZ::new(p[0], p[1], p[2], shapefile::NO_DATA))
                .collect();
            w.write_shape_and_record(&shapefile::PolylineZ::new(pts), &table.record(k))
                .map_err(shp_err)?;
        } else {
            let pts: Vec<ShpPoint> = line.iter().map(|p| ShpPoint::new(p[0], p[1])).collect();
            w.write_shape_and_record(&shapefile::Polyline::new(pts), &table.record(k))
                .map_err(shp_err)?;
        }
    }
    drop(w);
    write_prj(path, prj)
}

fn write_prj(path: &Path, prj: Option<&str>) -> Result<()> {
    if let Some(text) = prj {
        std::fs::write(path.with_extension("prj"), text)?;
    }
    Ok(())
}

/// Lo que guarda un shapefile y dibuja un visor: polilíneas y polígonos como anillos de puntos 3D,
/// y puntos.
#[derive(Clone, Debug, Default)]
pub struct Drawing {
    pub points: Vec<[f64; 3]>,
    /// Partes abiertas y cerradas; `true` para un anillo de un polígono.
    pub lines: Vec<(Vec<[f64; 3]>, bool)>,
    /// Si las formas llevan sus propias elevaciones.
    pub has_z: bool,
}

/// Lee cualquier shapefile para mostrarlo, conservando las elevaciones cuando las tiene.
pub fn read_drawing(path: &Path) -> Result<Drawing> {
    let mut reader = shapefile::ShapeReader::from_path(path).map_err(shp_open_err(path))?;
    let mut d = Drawing::default();
    for shape in reader.iter_shapes() {
        match shape.map_err(shp_err)? {
            Shape::Point(p) => d.points.push([p.x, p.y, f64::NAN]),
            Shape::PointM(p) => d.points.push([p.x, p.y, f64::NAN]),
            Shape::PointZ(p) => {
                d.has_z = true;
                d.points.push([p.x, p.y, p.z]);
            }
            Shape::Polyline(l) => {
                for part in l.parts() {
                    d.lines
                        .push((part.iter().map(|p| [p.x, p.y, f64::NAN]).collect(), false));
                }
            }
            Shape::PolylineM(l) => {
                for part in l.parts() {
                    d.lines
                        .push((part.iter().map(|p| [p.x, p.y, f64::NAN]).collect(), false));
                }
            }
            Shape::PolylineZ(l) => {
                d.has_z = true;
                for part in l.parts() {
                    d.lines
                        .push((part.iter().map(|p| [p.x, p.y, p.z]).collect(), false));
                }
            }
            Shape::Polygon(p) => {
                for r in p.rings() {
                    d.lines.push((
                        r.points().iter().map(|q| [q.x, q.y, f64::NAN]).collect(),
                        true,
                    ));
                }
            }
            Shape::PolygonM(p) => {
                for r in p.rings() {
                    d.lines.push((
                        r.points().iter().map(|q| [q.x, q.y, f64::NAN]).collect(),
                        true,
                    ));
                }
            }
            Shape::PolygonZ(p) => {
                d.has_z = true;
                for r in p.rings() {
                    d.lines
                        .push((r.points().iter().map(|q| [q.x, q.y, q.z]).collect(), true));
                }
            }
            Shape::Multipoint(m) => d
                .points
                .extend(m.points().iter().map(|p| [p.x, p.y, f64::NAN])),
            Shape::MultipointZ(m) => {
                d.has_z = true;
                d.points.extend(m.points().iter().map(|p| [p.x, p.y, p.z]));
            }
            _ => {}
        }
    }
    Ok(d)
}

/// El código EPSG que nombra un texto `.prj`: de su autoridad o del nombre de una zona UTM de
/// ETRS89, WGS 84 o ED50.
pub fn epsg_from_prj(text: &str) -> Option<u16> {
    if let Some(code) = crate::las::epsg_from_wkt(text) {
        return Some(code);
    }
    let up = text.to_uppercase().replace(['_', ' '], "");
    let zone_at = up.find("UTMZONE")? + 7;
    let digits: String = up
        .get(zone_at..)?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let zone: u16 = digits.parse().ok().filter(|z| (1..=60).contains(z))?;
    let south = up.get(zone_at + digits.len()..)?.starts_with('S');
    if up.contains("ETRS") {
        Some(25800 + zone)
    } else if up.contains("WGS1984") || up.contains("WGS84") {
        Some(if south { 32700 + zone } else { 32600 + zone })
    } else if up.contains("ED50") || up.contains("EUROPEAN1950") {
        Some(23000 + zone)
    } else {
        None
    }
}

/// Un texto `.prj` (WKT de OGC con sus identificadores EPSG, que todo SIG lee igual) para un código
/// geográfico (WGS 84, ETRS89, ED50), para Web Mercator (3857) o para uno UTM de ETRS89, WGS 84 o
/// ED50.
pub fn prj_for_epsg(code: u16) -> Option<String> {
    // sistemas geográficos y la proyección de los mapas web: no son UTM
    const WGS84: &str = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]]"#;
    match code {
        4326 => return Some(WGS84.to_string()),
        4258 => {
            return Some(
                r#"GEOGCS["ETRS89",DATUM["European_Terrestrial_Reference_System_1989",SPHEROID["GRS 1980",6378137,298.257222101,AUTHORITY["EPSG","7019"]],AUTHORITY["EPSG","6258"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4258"]]"#
                    .to_string(),
            );
        }
        4230 => {
            return Some(
                r#"GEOGCS["ED50",DATUM["European_Datum_1950",SPHEROID["International 1924",6378388,297,AUTHORITY["EPSG","7022"]],AUTHORITY["EPSG","6230"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4230"]]"#
                    .to_string(),
            );
        }
        3857 => {
            return Some(format!(
                "PROJCS[\"WGS 84 / Pseudo-Mercator\",{WGS84},PROJECTION[\"Mercator_1SP\"],PARAMETER[\"central_meridian\",0],PARAMETER[\"scale_factor\",1],PARAMETER[\"false_easting\",0],PARAMETER[\"false_northing\",0],UNIT[\"metre\",1,AUTHORITY[\"EPSG\",\"9001\"]],AXIS[\"Easting\",EAST],AXIS[\"Northing\",NORTH],AUTHORITY[\"EPSG\",\"3857\"]]"
            ));
        }
        _ => {}
    }
    let (zone, name, geogcs, south) = match code {
        25828..=25838 => (
            code - 25800,
            "ETRS89",
            r#"GEOGCS["ETRS89",DATUM["European_Terrestrial_Reference_System_1989",SPHEROID["GRS 1980",6378137,298.257222101,AUTHORITY["EPSG","7019"]],AUTHORITY["EPSG","6258"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4258"]]"#,
            false,
        ),
        32601..=32660 | 32701..=32760 => {
            let south = code >= 32701;
            (
                if south { code - 32700 } else { code - 32600 },
                "WGS 84",
                r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]]"#,
                south,
            )
        }
        23028..=23038 => (
            code - 23000,
            "ED50",
            r#"GEOGCS["ED50",DATUM["European_Datum_1950",SPHEROID["International 1924",6378388,297,AUTHORITY["EPSG","7022"]],AUTHORITY["EPSG","6230"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4230"]]"#,
            false,
        ),
        _ => return None,
    };
    let cm = i32::from(zone) * 6 - 183;
    let hemi = if south { "S" } else { "N" };
    Some(format!(
        "PROJCS[\"{name} / UTM zone {zone}{hemi}\",{geogcs},PROJECTION[\"Transverse_Mercator\"],PARAMETER[\"latitude_of_origin\",0],PARAMETER[\"central_meridian\",{cm}],PARAMETER[\"scale_factor\",0.9996],PARAMETER[\"false_easting\",500000],PARAMETER[\"false_northing\",{}],UNIT[\"metre\",1,AUTHORITY[\"EPSG\",\"9001\"]],AXIS[\"Easting\",EAST],AXIS[\"Northing\",NORTH],AUTHORITY[\"EPSG\",\"{code}\"]]",
        if south { 10_000_000 } else { 0 }
    ))
}

#[cfg(test)]
mod tests {
    fn sq(x: f64, y: f64, side: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x, y + side], [x + side, y + side], [x + side, y]]
    }

    #[test]
    fn a_buffer_grows_a_polygon_by_exactly_the_distance() {
        let r = Region {
            name: "a".into(),
            shape: Shape2::Polygon(Polygon::new(vec![sq(0.0, 0.0, 100.0)])),
            attrs: vec![],
        };
        let b = r.buffered(10.0);
        // a lo largo de un lado: 10 m hacia fuera está dentro, 10,1 m está fuera
        assert!(b.contains(50.0, -9.9) && !b.contains(50.0, -10.1));
        assert!(b.contains(109.9, 50.0) && !b.contains(110.1, 50.0));
        // esquina redondeada: el alcance en diagonal es 10 m, no 10 * raíz de 2
        assert!(b.contains(-7.0, -7.0) && !b.contains(-7.2, -7.2));
        let bb = b.bbox();
        assert_eq!(bb, [-10.0, -10.0, 110.0, 110.0]);
        // shrinking
        let s = r.buffered(-10.0);
        assert!(s.contains(50.0, 50.0) && !s.contains(5.0, 50.0) && s.contains(10.5, 50.0));
        // la pared de un hueco también crece, hacia dentro del hueco
        let holed = Region {
            name: "h".into(),
            shape: Shape2::Polygon(Polygon::new(vec![
                sq(0.0, 0.0, 100.0),
                sq(40.0, 40.0, 20.0),
            ])),
            attrs: vec![],
        };
        let hb = holed.buffered(5.0);
        assert!(hb.contains(37.0, 50.0) && !hb.contains(50.0, 50.0));
    }

    #[test]
    fn a_multipart_feature_splits_into_its_parts_with_their_holes() {
        let r = Region {
            name: "stand".into(),
            shape: Shape2::Polygon(Polygon::new(vec![
                sq(0.0, 0.0, 100.0),
                sq(40.0, 40.0, 20.0).into_iter().rev().collect(),
                sq(200.0, 0.0, 50.0),
            ])),
            attrs: vec![("id".into(), "7".into())],
        };
        let parts = r.split_parts();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "stand_p1");
        assert!((parts[0].area() - (10_000.0 - 400.0)).abs() < 1e-6);
        assert!((parts[1].area() - 2_500.0).abs() < 1e-6);
        assert_eq!(parts[1].attrs[0].1, "7");
        // una parte sigue siendo una
        let one = Region {
            name: "x".into(),
            shape: Shape2::Polygon(Polygon::new(vec![sq(0.0, 0.0, 5.0)])),
            attrs: vec![],
        };
        assert_eq!(one.split_parts().len(), 1);
        assert_eq!(one.split_parts()[0].name, "x");
    }

    #[test]
    fn a_saved_layer_keeps_its_holes() {
        use crate::view::{Feature, GeomKind, VectorData};
        // el hueco dibujado en el mismo sentido que el anillo que lo rodea
        let outer = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [0.0, 0.0],
        ];
        let hole = vec![[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0], [2.0, 2.0]];
        let d = VectorData {
            features: vec![Feature {
                parts: vec![outer, hole],
            }],
            polygons: true,
            ..VectorData::default()
        };
        let path = std::env::temp_dir().join(format!("rl_holes_{}.shp", std::process::id()));
        assert!(write_layer(&path, &d, GeomKind::Polygon, None).is_ok());
        let shapes = shapefile::read_shapes_as::<_, shapefile::Polygon>(&path).unwrap_or_default();
        for e in ["shp", "shx", "dbf"] {
            let _ = std::fs::remove_file(path.with_extension(e));
        }
        let rings = shapes
            .first()
            .map(|p| p.rings().to_vec())
            .unwrap_or_default();
        assert_eq!(rings.len(), 2);
        assert!(matches!(rings[0], PolygonRing::Outer(_)));
        assert!(matches!(rings[1], PolygonRing::Inner(_)));
    }

    #[test]
    fn prj_text_names_the_code_and_reads_back() {
        for code in [25829u16, 25830, 32630, 23030, 4326, 4258, 4230, 3857] {
            let t = prj_for_epsg(code).unwrap_or_default();
            assert!(
                t.ends_with(&format!("AUTHORITY[\"EPSG\",\"{code}\"]]")),
                "{t}"
            );
            assert_eq!(epsg_from_prj(&t), Some(code), "{t}");
        }
    }

    /// Escribe una capa de cada tipo en la carpeta que nombra `HIPSO_MARKUP_OUT`, para abrirlas en
    /// otro programa (`cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn write_layer_samples() -> Result<()> {
        use crate::view::{Feature, GeomKind, VectorData};
        let Some(dir) = std::env::var_os("HIPSO_MARKUP_OUT") else {
            return Ok(());
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir)?;
        let prj = prj_for_epsg(25830);
        for (name, kind, parts) in [
            ("puntos", GeomKind::Point, vec![vec![[580200.0, 4802000.0]]]),
            (
                "lineas",
                GeomKind::Line,
                vec![vec![[580100.0, 4801900.0], [580300.0, 4801950.0]]],
            ),
            (
                "areas",
                GeomKind::Polygon,
                vec![vec![
                    [580100.0, 4801900.0],
                    [580300.0, 4801900.0],
                    [580300.0, 4802100.0],
                    [580100.0, 4801900.0],
                ]],
            ),
        ] {
            let d = VectorData {
                features: vec![Feature { parts }],
                fields: vec!["Nombre".into(), "Valor".into()],
                text: vec![vec!["uno".into(), "3.5".into()]],
                num: vec![vec![f64::NAN, 3.5]],
                ..VectorData::default()
            };
            write_layer(&dir.join(format!("{name}.shp")), &d, kind, prj.as_deref())?;
        }
        Ok(())
    }

    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<[f64; 2]> {
        // En el sentido de las agujas del reloj, como los anillos exteriores de un shapefile.
        vec![[x0, y0], [x0, y1], [x1, y1], [x1, y0], [x0, y0]]
    }

    #[test]
    fn holes_and_parts() {
        let outer = square(0.0, 0.0, 10.0, 10.0);
        let mut hole = square(4.0, 4.0, 6.0, 6.0);
        hole.reverse();
        let p = Polygon::new(vec![outer, hole, square(20.0, 0.0, 30.0, 10.0)]);
        assert!(p.contains(1.0, 1.0) && p.contains(25.0, 5.0));
        assert!(!p.contains(5.0, 5.0) && !p.contains(15.0, 5.0));
        assert!((p.area() - (100.0 - 4.0 + 100.0)).abs() < 1e-9);
    }

    #[test]
    fn the_lookup_mask_agrees_with_the_exact_test() {
        // Una estrella con muchos vértices, para que se use la máscara.
        let n = 400;
        let ring: Vec<[f64; 2]> = (0..n)
            .map(|i| {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                let r = if i % 2 == 0 { 100.0 } else { 60.0 };
                [r * a.cos(), r * a.sin()]
            })
            .collect();
        let p = Polygon::new(vec![ring]);
        assert!(p.mask.is_some());
        let mut seed = 12345u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..20_000 {
            let (x, y) = (next() * 220.0 - 110.0, next() * 220.0 - 110.0);
            assert_eq!(p.contains(x, y), p.contains_exact(x, y), "at {x},{y}");
        }
    }

    #[test]
    fn the_index_finds_regions_among_many() {
        let regions: Vec<Region> = (0..50)
            .map(|i| Region {
                name: format!("p{i}"),
                shape: Shape2::Circle {
                    cx: f64::from(i) * 30.0,
                    cy: 0.0,
                    r: 10.0,
                },
                attrs: vec![],
            })
            .collect();
        let idx = RegionIndex::new(&regions);
        assert_eq!(idx.first(60.0, 5.0), Some(2));
        assert_eq!(idx.first(60.0, 50.0), None);
        assert!(idx.any(1470.0, 0.0));
    }

    #[test]
    fn shapefiles_round_trip_with_attributes() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_vec_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("plots.shp");
        let polys = vec![
            Polygon::new(vec![square(0.0, 0.0, 10.0, 10.0)]),
            Polygon::new(vec![square(20.0, 0.0, 30.0, 10.0)]),
        ];
        let table = Table {
            columns: vec![
                ("NOMBRE".into(), ColKind::Text(20)),
                ("AREA".into(), ColKind::Num(12, 2)),
            ],
            rows: vec![
                vec![Val::Text("muestra A".into()), Val::Num(100.0)],
                vec![Val::Text("muestra B".into()), Val::Num(100.0)],
            ],
        };
        write_polygons(&path, &polys, &table, prj_for_epsg(25829).as_deref())?;
        let regions = read_regions(
            &path,
            &ReadOptions {
                name_field: Some("NOMBRE".into()),
                ..Default::default()
            },
        )?;
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[1].name, "muestra B");
        assert!(regions[0].contains(5.0, 5.0) && !regions[0].contains(25.0, 5.0));
        let prj = std::fs::read_to_string(path.with_extension("prj"))?;
        assert_eq!(epsg_from_prj(&prj), Some(25829));
        let by_position = read_regions(
            &path,
            &ReadOptions {
                name_field: Some("1".into()),
                ..Default::default()
            },
        )?;
        assert!(by_position.len() == 2);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn points_with_a_radius_become_plots() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_vecp_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("centers.shp");
        let table = Table {
            columns: vec![("ID".into(), ColKind::Num(5, 0))],
            rows: vec![vec![Val::Int(7)]],
        };
        write_points(&path, &[[100.0, 200.0, 0.0]], false, &table, None)?;
        assert!(read_regions(&path, &ReadOptions::default()).is_err());
        let r = read_regions(
            &path,
            &ReadOptions {
                radius: Some(12.5),
                name_field: Some("ID".into()),
                ..Default::default()
            },
        )?;
        assert_eq!(r[0].name, "7");
        assert!(r[0].contains(100.0, 212.0) && !r[0].contains(100.0, 213.0));
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn text_lists_and_prj_names() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_vect_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("plots.csv");
        std::fs::write(
            &path,
            "name,x,y,r\nP1,100,200,10\nP2,300,200,12\n400 400 5 5\n",
        )?;
        let r = read_regions(&path, &ReadOptions::default())?;
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].name, "P1");
        assert!(matches!(r[2].shape, Shape2::Rect(_)));
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(
            epsg_from_prj("PROJCS[\"ETRS_1989_UTM_Zone_29N\",GEOGCS[\"GCS_ETRS_1989\"]]"),
            Some(25829)
        );
        assert_eq!(
            epsg_from_prj("PROJCS[\"WGS 84 / UTM zone 30S\"]"),
            Some(32730)
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// Tablas de atributos sobre polígonos (métricas por celda o por parcela) como shapefiles.

/// Palabras de los nombres de métricas y la forma corta que se usa en un nombre de campo DBF de 10
/// caracteres.
fn short_word(w: &str) -> String {
    let s = match w.to_ascii_lowercase().as_str() {
        "elev" => "E",
        "int" => "I",
        "minimum" => "Min",
        "maximum" => "Max",
        "mean" => "Mn",
        "mode" => "Mo",
        "stddev" => "Sd",
        "variance" => "Var",
        "cv" => "Cv",
        "iq" => "Iq",
        "skewness" => "Skw",
        "kurtosis" => "Kur",
        "aad" => "Aad",
        "mad" => "Mad",
        "median" => "Med",
        "total" => "T",
        "return" | "returns" => "R",
        "count" => "N",
        "percentage" => "Pc",
        "first" => "F",
        "all" => "A",
        "above" => "Ab",
        "other" => "O",
        "canopy" => "Can",
        "relief" => "Rel",
        "ratio" => "Rat",
        "quadratic" => "Qm",
        "cubic" => "Cm",
        "profile" => "Pf",
        "area" => "Ar",
        "center" => "C",
        _ => "",
    };
    if !s.is_empty() {
        return s.to_string();
    }
    let digits: String = w.chars().filter(char::is_ascii_alphanumeric).collect();
    if digits.len() <= 4 {
        digits
            .to_ascii_uppercase()
            .chars()
            .take(1)
            .chain(digits.chars().skip(1))
            .collect()
    } else {
        digits.chars().take(3).collect()
    }
}

/// Nombres de campo DBF (10 caracteres como mucho, únicos) para nombres de columna largos, con los
/// nombres largos que representan. Las columnas de estratos pasan a ser `E`/`I` + `S` + número de
/// estrato + estadístico.
pub fn dbf_names(cols: &[String]) -> Vec<(String, String)> {
    let mut strata: HashMap<(char, String), usize> = HashMap::new();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for c in cols {
        let mut short =
            if c.len() <= 10 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                c.clone()
            } else if let Some((head, rest)) = c.split_once(" strata (") {
                let kind = if head.starts_with("Int") { 'I' } else { 'E' };
                let (name, stat) = rest.split_once(") ").unwrap_or((rest, ""));
                let n = strata.len();
                let k = *strata.entry((kind, name.to_string())).or_insert(n);
                let k = k % 100;
                let stat: String = stat.split_whitespace().map(short_word).collect();
                format!("{kind}S{k}{stat}")
            } else {
                c.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.')
                    .filter(|w| !w.is_empty())
                    .map(|w| {
                        if w.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                            w.replace('.', "")
                        } else {
                            short_word(w)
                        }
                    })
                    .collect()
            };
        short.retain(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        if short.is_empty() || short.starts_with(|ch: char| ch.is_ascii_digit()) {
            short.insert(0, 'F');
        }
        short.truncate(10);
        let mut n = 1;
        while !used.insert(short.to_ascii_uppercase()) {
            let tail = n.to_string();
            short.truncate(10 - tail.len());
            short.push_str(&tail);
            n += 1;
        }
        out.push((short, c.clone()));
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum FieldKind {
    Int,
    Float,
    Text,
}

/// Nombre de la columna con la fecha en que se capturaron los datos LiDAR (en blanco cuando no se
/// sabe). Nunca es el día en que se hizo la tabla.
pub const CAPTURE_FIELD: &str = "captura";

/// Registros con geometría de polígono escritos de uno en uno, de modo que una tabla con una fila
/// por celda de rejilla nunca está en memoria. Los tipos de columna salen de la primera fila:
/// enteros, decimales o texto. El `.dbf` de una tabla escrita directamente, registro a registro:
/// campos de ancho fijo, como dice el formato. (Un escritor genérico construye un mapa de valores
/// para cada registro, lo que para una tabla con una fila por celda de rejilla y decenas de
/// columnas tarda mucho más que escribir los bytes.)
struct DbfOut {
    out: std::io::BufWriter<std::fs::File>,
    /// Ancho y decimales de cada campo, en orden.
    fields: Vec<(usize, usize, bool)>,
    count: u32,
    buf: Vec<u8>,
}

impl DbfOut {
    /// `columns`: nombre (hasta 10 bytes), si es un número, ancho, decimales.
    fn create(path: &Path, columns: &[(String, bool, usize, usize)]) -> Result<Self> {
        use std::io::Write;
        let mut out = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path)?);
        let rec_len: usize = 1 + columns.iter().map(|c| c.2).sum::<usize>();
        let head_len = 32 + 32 * columns.len() + 1;
        let (y, m, d) = today();
        let mut h = vec![0u8; 32];
        h[0] = 0x03;
        h[1] = (y - 1900) as u8;
        h[2] = m as u8;
        h[3] = d as u8;
        h[8..10].copy_from_slice(&(head_len as u16).to_le_bytes());
        h[10..12].copy_from_slice(&(rec_len as u16).to_le_bytes());
        h[29] = 0xF0;
        out.write_all(&h)?;
        for (name, numeric, width, dec) in columns {
            let mut f = [0u8; 32];
            let b = name.as_bytes();
            f[..b.len().min(10)].copy_from_slice(&b[..b.len().min(10)]);
            f[11] = if *numeric { b'N' } else { b'C' };
            f[16] = *width as u8;
            f[17] = *dec as u8;
            out.write_all(&f)?;
        }
        out.write_all(&[0x0D])?;
        Ok(Self {
            out,
            fields: columns.iter().map(|c| (c.2, c.3, c.1)).collect(),
            count: 0,
            buf: Vec::with_capacity(rec_len),
        })
    }

    /// Añade un registro: un valor por campo (los números como texto que analizar, uno vacío es en
    /// blanco).
    fn push<'a>(&mut self, values: impl Iterator<Item = &'a str>) -> Result<()> {
        use std::io::Write;
        self.buf.clear();
        self.buf.push(b' ');
        for (&(width, dec, numeric), v) in self.fields.iter().zip(values) {
            let at = self.buf.len();
            if numeric && plain_number(v, width, dec) {
                // ya escrito como lo quiere el campo: el propio texto, alineado a la derecha
                self.buf.resize(at + width - v.len(), b' ');
                self.buf.extend_from_slice(v.as_bytes());
            } else if numeric {
                let text = match v.parse::<f64>() {
                    Ok(x) if x.is_finite() => {
                        let mut t = format!("{x:.dec$}");
                        // un número que no cabe pierde decimales antes que dígitos
                        let mut d = dec;
                        while t.len() > width && d > 0 {
                            d -= 1;
                            t = format!("{x:.d$}");
                        }
                        t
                    }
                    _ => String::new(),
                };
                let t = text.as_bytes();
                let t = &t[..t.len().min(width)];
                self.buf.resize(at + width - t.len(), b' ');
                self.buf.extend_from_slice(t);
            } else {
                let mut end = v.len().min(width);
                while !v.is_char_boundary(end) {
                    end -= 1;
                }
                self.buf.extend_from_slice(&v.as_bytes()[..end]);
                self.buf.resize(at + width, b' ');
            }
        }
        self.out.write_all(&self.buf)?;
        self.count += 1;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        self.out.write_all(&[0x1A])?;
        self.out.flush()?;
        let mut f = self.out.into_inner().map_err(|e| Error::Io(e.into_error()))?;
        f.seek(SeekFrom::Start(4))?;
        f.write_all(&self.count.to_le_bytes())?;
        Ok(())
    }
}

/// Si `v` es un número decimal simple (`-12`, `0.0038`) con como mucho `dec` decimales que cabe en
/// `width`: se puede escribir tal cual, sin leerlo como decimal y volver a darle formato (que es lo
/// que lleva el tiempo cuando una tabla tiene millones de filas).
fn plain_number(v: &str, width: usize, dec: usize) -> bool {
    let b = v.as_bytes();
    if b.is_empty() || b.len() > width {
        return false;
    }
    let digits = b.strip_prefix(b"-").unwrap_or(b);
    let (int, frac) = match digits.iter().position(|c| *c == b'.') {
        Some(k) => (&digits[..k], Some(&digits[k + 1..])),
        None => (digits, None),
    };
    !int.is_empty()
        && int.iter().all(u8::is_ascii_digit)
        && frac.is_none_or(|f| !f.is_empty() && f.len() <= dec && f.iter().all(u8::is_ascii_digit))
        // «-0» y «-0.0» se leerían de vuelta como cero negativo: que decida el formateador
        && !(b[0] == b'-' && int == b"0")
}

/// La fecha de hoy (año, mes, día), del reloj.
fn today() -> (i64, i64, i64) {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86_400) as i64);
    // fecha civil a partir de días desde 1970 (algoritmo de Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub struct PolyTable {
    shp: shapefile::ShapeWriter<std::io::BufWriter<std::fs::File>>,
    dbf: DbfOut,
    names: Vec<String>,
    /// Valor de la columna adicional `captura`, la fecha de captura de los datos LiDAR: en blanco
    /// salvo que la dé quien llama. `None` cuando una columna de los datos ya es la fecha de
    /// captura.
    capture: Option<String>,
    path: std::path::PathBuf,
    prj: Option<String>,
}

impl PolyTable {
    /// Crea `path` (y `<nombre>_campos.txt` con los nombres largos cuando se acortaron).
    pub fn create(
        path: &Path,
        columns: &[String],
        first_row: &[impl AsRef<str>],
        prj: Option<&str>,
    ) -> Result<Self> {
        // una columna `Date` de los datos es la fecha de captura: un campo de fecha, no dos
        let columns: Vec<String> = columns
            .iter()
            .map(|c| {
                if c.eq_ignore_ascii_case("date") {
                    CAPTURE_FIELD.to_string()
                } else {
                    c.clone()
                }
            })
            .collect();
        let columns = columns.as_slice();
        let named = dbf_names(columns);
        let kinds: Vec<FieldKind> = columns
            .iter()
            .enumerate()
            .map(|(k, _)| match first_row.get(k).map(|v| v.as_ref().trim()) {
                Some(v) if v.parse::<i64>().is_ok() => FieldKind::Int,
                Some(v) if v.parse::<f64>().is_ok() => FieldKind::Float,
                _ => FieldKind::Text,
            })
            .collect();
        let mut cols: Vec<(String, bool, usize, usize)> = named
            .iter()
            .zip(&kinds)
            .map(|((short, _), kind)| match kind {
                FieldKind::Int => (short.clone(), true, 12, 0),
                FieldKind::Float => (short.clone(), true, 18, 4),
                FieldKind::Text => (short.clone(), false, 50, 0),
            })
            .collect();
        // cada polígono dice cuándo se capturaron sus datos LiDAR; en blanco cuando no se sabe
        let capture = (!named
            .iter()
            .any(|(s, _)| s.eq_ignore_ascii_case(CAPTURE_FIELD)))
        .then(String::new);
        if capture.is_some() {
            cols.push((CAPTURE_FIELD.to_string(), false, 24, 0));
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let shp = shapefile::ShapeWriter::from_path(path).map_err(shp_open_err(path))?;
        let dbf = DbfOut::create(&path.with_extension("dbf"), &cols)?;
        if named.iter().any(|(s, l)| s != l) {
            let mut text = String::from("campo\tsignificado\n");
            for (s, l) in &named {
                text.push_str(&format!("{s}\t{l}\n"));
            }
            let stem = path
                .file_stem()
                .map_or(String::new(), |s| s.to_string_lossy().into_owned());
            std::fs::write(path.with_file_name(format!("{stem}_campos.txt")), text)?;
        }
        Ok(Self {
            shp,
            dbf,
            capture,
            names: named.into_iter().map(|n| n.0).collect(),
            path: path.to_path_buf(),
            prj: prj.map(str::to_string),
        })
    }

    /// Fija la fecha de captura de los datos LiDAR de los que proceden los polígonos (la columna
    /// adicional `captura`, que si no queda en blanco).
    pub fn with_capture(mut self, date: Option<String>) -> Self {
        if self.capture.is_some() {
            self.capture = Some(date.unwrap_or_default());
        }
        self
    }

    /// Escribe los atributos de un registro: los valores de la fila y luego la fecha de captura.
    fn attributes<S: AsRef<str>>(&mut self, row: &[S]) -> Result<()> {
        let names = &self.names;
        let capture = self.capture.as_deref();
        let values = names
            .iter()
            .enumerate()
            .map(|(k, name)| {
                let v = row.get(k).map_or("", |v| v.as_ref().trim());
                // `NA` es como dicen las tablas de texto que la fecha de captura no se conoce
                if name.eq_ignore_ascii_case(CAPTURE_FIELD) && v.eq_ignore_ascii_case("na") {
                    ""
                } else {
                    v
                }
            })
            .chain(capture);
        self.dbf.push(values)
    }

    /// Añade un registro: sus anillos (cada uno un anillo exterior) y un valor por columna.
    pub fn push<S: AsRef<str>>(&mut self, rings: &[Vec<[f64; 2]>], row: &[S]) -> Result<()> {
        let parts: Vec<PolygonRing<ShpPoint>> = rings
            .iter()
            .filter(|r| r.len() >= 3)
            .map(|r| {
                let area: f64 = r
                    .iter()
                    .zip(r.iter().cycle().skip(1))
                    .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
                    .sum();
                let mut pts: Vec<ShpPoint> = r.iter().map(|p| ShpPoint::new(p[0], p[1])).collect();
                // outer rings run clockwise
                if area > 0.0 {
                    pts.reverse();
                }
                if pts.first() != pts.last() {
                    pts.push(pts[0]);
                }
                PolygonRing::Outer(pts)
            })
            .collect();
        if parts.is_empty() {
            return Ok(());
        }
        self.shp
            .write_shape(&shapefile::Polygon::with_rings(parts))
            .map_err(shp_err)?;
        self.attributes(row)
    }

    /// Añade un registro hecho de anillos exteriores y los huecos practicados en ellos (anillos de
    /// cualquier orientación: los exteriores se escriben en el sentido de las agujas del reloj y
    /// los huecos en sentido contrario, como esperan los lectores).
    pub fn push_with_holes(
        &mut self,
        outers: &[Vec<[f64; 2]>],
        holes: &[Vec<[f64; 2]>],
        row: &[impl AsRef<str>],
    ) -> Result<()> {
        let oriented = |r: &Vec<[f64; 2]>, clockwise: bool| -> Option<Vec<ShpPoint>> {
            if r.len() < 3 {
                return None;
            }
            let area: f64 = r
                .iter()
                .zip(r.iter().cycle().skip(1))
                .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
                .sum();
            let mut pts: Vec<ShpPoint> = r.iter().map(|p| ShpPoint::new(p[0], p[1])).collect();
            // una suma positiva es antihoraria
            if (area > 0.0) == clockwise {
                pts.reverse();
            }
            if pts.first() != pts.last() {
                pts.push(pts[0]);
            }
            Some(pts)
        };
        let mut parts: Vec<PolygonRing<ShpPoint>> = outers
            .iter()
            .filter_map(|r| oriented(r, true))
            .map(PolygonRing::Outer)
            .collect();
        if parts.is_empty() {
            return Ok(());
        }
        parts.extend(
            holes
                .iter()
                .filter_map(|r| oriented(r, false))
                .map(PolygonRing::Inner),
        );
        self.shp
            .write_shape(&shapefile::Polygon::with_rings(parts))
            .map_err(shp_err)?;
        self.attributes(row)
    }

    pub fn finish(self) -> Result<()> {
        drop(self.shp);
        self.dbf.finish()?;
        write_prj(&self.path, self.prj.as_deref())
    }
}

/// Convierte un CSV de métricas en un shapefile de celdas cuadradas. Cada fila nombra su celda por
/// `row` y `col`; `centres` da el centro de cada `(row, col)` (rellenado a partir del primer
/// archivo que tiene columnas `center X`/`center Y`, de modo que el archivo de estratos, que no las
/// tiene, también puede usarlo).
pub fn metrics_csv_to_shp(
    csv: &Path,
    out: &Path,
    cell: f64,
    centres: &mut HashMap<(i64, i64), (f64, f64)>,
    prj: Option<&str>,
) -> Result<usize> {
    use std::io::BufRead;
    let mut lines = std::io::BufReader::new(std::fs::File::open(csv)?).lines();
    let header: Vec<String> = match lines.next() {
        Some(l) => l?.split(',').map(str::to_string).collect(),
        None => return Ok(0),
    };
    let at = |n: &str| header.iter().position(|h| h == n);
    let (ir, ic) = (at("row").unwrap_or(0), at("col").unwrap_or(1));
    let (ix, iy) = (at("center X"), at("center Y"));
    let mut table: Option<PolyTable> = None;
    let mut n = 0;
    for line in lines {
        let line = line?;
        let row: Vec<&str> = line.split(',').collect();
        let key = (
            row.get(ir).and_then(|v| v.parse().ok()).unwrap_or(0),
            row.get(ic).and_then(|v| v.parse().ok()).unwrap_or(0),
        );
        if let (Some(x), Some(y)) = (ix.and_then(|k| row.get(k)), iy.and_then(|k| row.get(k)))
            && let (Ok(x), Ok(y)) = (x.parse(), y.parse())
        {
            centres.insert(key, (x, y));
        }
        let Some(&(cx, cy)) = centres.get(&key) else {
            continue;
        };
        if table.is_none() {
            table = Some(PolyTable::create(out, &header, &row, prj)?);
        }
        let h = cell / 2.0;
        let ring = vec![
            [cx - h, cy - h],
            [cx - h, cy + h],
            [cx + h, cy + h],
            [cx + h, cy - h],
        ];
        if let Some(t) = table.as_mut() {
            t.push(&[ring], &row)?;
            n += 1;
        }
    }
    if let Some(t) = table {
        t.finish()?;
    }
    Ok(n)
}

/// Si el punto está dentro del anillo (regla par-impar).
fn ring_contains(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Guarda una capa hecha en el visor: sus elementos y su tabla de atributos (un campo es numérico
/// cuando todos sus valores lo son; si no, texto), con el texto `.prj` del sistema de coordenadas.
pub fn write_layer(
    path: &Path,
    d: &crate::view::VectorData,
    kind: crate::view::GeomKind,
    prj: Option<&str>,
) -> Result<()> {
    use crate::view::GeomKind;
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gpkg"))
    {
        let name: String = crate::paths::stem(path)
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        return crate::gpkg_write::write(
            path,
            &[crate::gpkg_write::Table {
                name: &name,
                data: d,
                kind,
                epsg: prj.and_then(epsg_from_prj),
            }],
        );
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("kml") || e.eq_ignore_ascii_case("kmz"))
    {
        return crate::kml::write_vector(path, d, kind, prj.and_then(epsg_from_prj));
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("geojson") || e.eq_ignore_ascii_case("json"))
    {
        return crate::geojson::write_vector(path, d, kind, prj.and_then(epsg_from_prj));
    }
    use crate::fieldtype::FieldType;
    // cada campo se escribe como lo que es: el tipo que se le dio o, si no, lo que digan sus
    // valores
    let types: Vec<FieldType> = (0..d.fields.len()).map(|c| d.field_type(c)).collect();
    let names: Vec<String> = dbf_names(&d.fields).into_iter().map(|n| n.0).collect();
    let mut b = TableWriterBuilder::new();
    for (name, ty) in names.iter().zip(&types) {
        let n = FieldName::try_from(name.as_str())
            .map_err(|e| Error::Param(format!("field '{name}': {e}")))?;
        b = match ty {
            FieldType::Integer => b.add_numeric_field(n, 18, 0),
            FieldType::Real => b.add_numeric_field(n, 18, 6),
            FieldType::Bool => b.add_logical_field(n),
            FieldType::Date => b.add_date_field(n),
            FieldType::Link => b.add_character_field(n, 254),
            FieldType::Text => b.add_character_field(n, 80),
        };
    }
    let mut w = shapefile::Writer::from_path(path, b).map_err(shp_open_err(path))?;
    for (k, f) in d.features.iter().enumerate() {
        let mut rec = dbase::Record::default();
        for (c, name) in names.iter().enumerate() {
            let t = d
                .text
                .get(k)
                .and_then(|r| r.get(c))
                .map_or("", String::as_str)
                .trim();
            let value = match types[c] {
                FieldType::Integer | FieldType::Real => FieldValue::Numeric(t.parse().ok()),
                FieldType::Bool => FieldValue::Logical(
                    FieldType::Bool.parse(t).ok().filter(|v| !v.1.is_nan()).map(|v| v.1 != 0.0),
                ),
                FieldType::Date => FieldValue::Date(
                    crate::fieldtype::parse_date(t).and_then(|days| {
                        let (y, m, dd) = crate::fieldtype::civil_from_days(days as i64);
                        dbase::Date::new(dd as u32, m as u32, y as u32).ok()
                    }),
                ),
                FieldType::Link => FieldValue::Character(Some(truncate(t, 254))),
                FieldType::Text => FieldValue::Character(Some(truncate(t, 80))),
            };
            rec.insert(name.clone(), value);
        }
        match kind {
            GeomKind::Point => {
                if let Some(p) = f.parts.first().and_then(|p| p.first()) {
                    w.write_shape_and_record(&ShpPoint::new(p[0], p[1]), &rec)
                        .map_err(shp_err)?;
                }
            }
            GeomKind::Line => {
                let parts: Vec<Vec<ShpPoint>> = f
                    .parts
                    .iter()
                    .filter(|p| p.len() >= 2)
                    .map(|p| p.iter().map(|q| ShpPoint::new(q[0], q[1])).collect())
                    .collect();
                if !parts.is_empty() {
                    w.write_shape_and_record(&shapefile::Polyline::with_parts(parts), &rec)
                        .map_err(shp_err)?;
                }
            }
            GeomKind::Polygon => {
                // un anillo dentro de un número impar de los demás es un hueco: los anillos
                // exteriores van en el sentido de las agujas del reloj y los huecos en sentido
                // contrario, que es lo que usan los lectores para distinguirlos
                let all: Vec<&Vec<[f64; 2]>> = f.parts.iter().filter(|r| r.len() >= 3).collect();
                let rings: Vec<PolygonRing<ShpPoint>> = all
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let inside = all
                            .iter()
                            .enumerate()
                            .filter(|(j, o)| *j != i && ring_contains(o, r[0]))
                            .count();
                        let hole = inside % 2 == 1;
                        let area: f64 = r
                            .iter()
                            .zip(r.iter().cycle().skip(1))
                            .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
                            .sum();
                        let mut pts: Vec<ShpPoint> =
                            r.iter().map(|p| ShpPoint::new(p[0], p[1])).collect();
                        // una suma positiva es antihoraria
                        if (area > 0.0) != hole {
                            pts.reverse();
                        }
                        if pts.first() != pts.last() {
                            pts.push(pts[0]);
                        }
                        if hole {
                            PolygonRing::Inner(pts)
                        } else {
                            PolygonRing::Outer(pts)
                        }
                    })
                    .collect();
                if !rings.is_empty() {
                    w.write_shape_and_record(&shapefile::Polygon::with_rings(rings), &rec)
                        .map_err(shp_err)?;
                }
            }
        }
    }
    drop(w);
    write_types_sidecar(path, d, &types);
    write_prj(path, prj)
}

/// El shapefile no distingue un hipervínculo de un texto: los campos que son enlaces van en
/// `<nombre>_tipos.txt` (un nombre por línea, como están en la tabla), que se borra cuando no hay
/// ninguno.
fn write_types_sidecar(path: &Path, d: &crate::view::VectorData, types: &[crate::fieldtype::FieldType]) {
    let side = crate::view::types_sidecar(path);
    let links: Vec<&str> = d
        .fields
        .iter()
        .zip(types)
        .filter(|(_, t)| **t == crate::fieldtype::FieldType::Link)
        .map(|(n, _)| n.as_str())
        .collect();
    if links.is_empty() {
        let _ = std::fs::remove_file(&side);
    } else {
        let _ = std::fs::write(&side, format!("hipervinculo\n{}\n", links.join("\n")));
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    fn square() -> Vec<Vec<[f64; 2]>> {
        vec![vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]]
    }

    /// La columna de la fecha de captura de una tabla de una fila escrita con `cols`/`row`.
    fn captura_of(name: &str, cols: &[&str], row: &[&str]) -> Result<(Vec<String>, String)> {
        let dir = std::env::temp_dir().join(format!("hipso_capture_{name}"));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("t.shp");
        let cols: Vec<String> = cols.iter().map(|s| s.to_string()).collect();
        let row: Vec<String> = row.iter().map(|s| s.to_string()).collect();
        let mut t = PolyTable::create(&path, &cols, &row, None)?;
        t.push(&square(), &row)?;
        t.finish()?;
        let (names, rows) = attribute_table(&path)?;
        let at = names
            .iter()
            .position(|n| n == CAPTURE_FIELD)
            .ok_or_else(|| Error::Param("no captura".into()))?;
        let _ = std::fs::remove_dir_all(&dir);
        Ok((names, rows[0][at].clone()))
    }

    #[test]
    fn the_only_date_is_the_capture_date_of_the_data() -> Result<()> {
        let (names, v) = captura_of("known", &["id", "Date"], &["1", "2012-05-18"])?;
        assert_eq!(
            names.iter().filter(|n| n.as_str() == CAPTURE_FIELD).count(),
            1
        );
        assert_eq!(names.len(), 2);
        assert_eq!(v, "2012-05-18");
        Ok(())
    }

    #[test]
    fn an_unknown_capture_date_is_blank() -> Result<()> {
        let (names, v) = captura_of("na", &["id", "Date"], &["1", "NA"])?;
        assert_eq!(names.len(), 2);
        assert_eq!(v, "");
        // una tabla sin fecha alguna recibe el campo, en blanco: nunca el día en que se hizo
        let (names, v) = captura_of("none", &["id"], &["1"])?;
        assert_eq!(names, vec!["id".to_string(), CAPTURE_FIELD.to_string()]);
        assert_eq!(v, "");
        Ok(())
    }
}

#[cfg(test)]
mod restored_names_tests {
    use super::*;

    #[test]
    fn a_plots_layer_without_its_campos_file_gets_its_long_names_back() -> Result<()> {
        let dir = std::env::temp_dir().join("hipso_restore_names");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("p.shp");
        let mut cols = vec!["Muestra".to_string(), "Area".to_string()];
        cols.extend(crate::metrics::sample_header(2.0));
        cols.push("Date".to_string());
        let row: Vec<String> = cols.iter().map(|_| "1.5".to_string()).collect();
        let mut t = PolyTable::create(&path, &cols, &row, None)?;
        t.push(
            &[vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]],
            &row,
        )?;
        t.finish()?;
        // con el archivo: nombres largos
        let (with, _) = attribute_table(&path)?;
        assert!(with.iter().any(|n| n == "Elev P95"), "{with:?}");
        // la capa copiada sola: los mismos nombres largos, recalculados
        std::fs::remove_file(dir.join("p_campos.txt"))?;
        let (without, _) = attribute_table(&path)?;
        assert_eq!(with, without);
        // una tabla de otra cosa se deja como está
        let other = dir.join("o.shp");
        let cols = vec!["id".to_string(), "uso".to_string(), "sup".to_string()];
        let row = vec!["1".to_string(), "x".to_string(), "2.5".to_string()];
        let mut o = PolyTable::create(&other, &cols, &row, None)?;
        o.push(
            &[vec![[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]],
            &row,
        )?;
        o.finish()?;
        let (names, _) = attribute_table(&other)?;
        assert_eq!(&names[..3], &["id", "uso", "sup"]);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
