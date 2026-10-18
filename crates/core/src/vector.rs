//! Vector data: the polygons, circles and rectangles points are clipped with, and ESRI
//! shapefiles read and written for them (stands, inventory plots, tree tops, crowns, POI/AOI).

use std::collections::HashMap;
use std::path::Path;

use shapefile::dbase::{self, FieldName, FieldValue, TableWriterBuilder};
use shapefile::{Point as ShpPoint, PointZ, PolygonRing, Shape};

use crate::{Error, Result};

/// Vertices from which a polygon gets a lookup mask: a point far from every edge is then
/// answered from a table instead of by walking all the edges.
const MASK_FROM: usize = 200;

/// A polygon with holes and parts: a point is inside when it is inside an odd number of rings.
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
    /// 0 outside, 1 inside, 2 near an edge.
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

    /// Even-odd test over every ring.
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

    /// The area: the net of the rings' signed areas (outer rings run clockwise, holes the
    /// other way, as shapefiles keep them).
    pub fn area(&self) -> f64 {
        let net: f64 = self.rings.iter().map(|r| ring_signed_area(r)).sum();
        net.abs()
    }
}

/// Twice nothing: the signed area by the shoelace sum (counter-clockwise positive).
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
        // Inside: each row's centre line against every edge, filling between crossing pairs.
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
        // Edges: every cell an edge passes through, and its neighbours, is decided exactly.
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

/// The shape of a region.
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
}

/// A region to clip with: a stand, an inventory plot, an area of interest.
#[derive(Clone, Debug)]
pub struct Region {
    pub name: String,
    pub shape: Shape2,
    /// The attribute table's values, as text.
    pub attrs: Vec<(String, String)>,
}

impl Region {
    pub fn bbox(&self) -> [f64; 4] {
        match &self.shape {
            Shape2::Polygon(p) => p.bbox,
            Shape2::Circle { cx, cy, r } => [cx - r, cy - r, cx + r, cy + r],
            Shape2::Rect(b) => *b,
        }
    }

    #[inline]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match &self.shape {
            Shape2::Polygon(p) => p.contains(x, y),
            Shape2::Circle { cx, cy, r } => (x - cx).powi(2) + (y - cy).powi(2) <= r * r,
            Shape2::Rect(b) => x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3],
        }
    }

    pub fn area(&self) -> f64 {
        match &self.shape {
            Shape2::Polygon(p) => p.area(),
            Shape2::Circle { r, .. } => std::f64::consts::PI * r * r,
            Shape2::Rect(b) => (b[2] - b[0]) * (b[3] - b[1]),
        }
    }
}

/// Finds the regions holding a point without testing them all.
pub struct RegionIndex<'a> {
    regions: &'a [Region],
    bbox: [f64; 4],
    cell: f64,
    cols: usize,
    rows: usize,
    /// Per grid cell, the regions whose box reaches it; empty when there are few regions.
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

    /// The extent of all the regions.
    pub fn bbox(&self) -> [f64; 4] {
        self.bbox
    }

    /// Calls `f` with every region that holds the point.
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

    /// The first region (in file order) that holds the point.
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

/// How regions are read from a file.
#[derive(Clone, Debug, Default)]
pub struct ReadOptions {
    /// The attribute that names each region: a field name or its 1-based position.
    pub name_field: Option<String>,
    /// Radius for point shapes (plot centres), in map units.
    pub radius: Option<f64>,
    /// An attribute holding each point's own radius.
    pub radius_field: Option<String>,
    /// Plots given as points are squares (or rectangles) instead of circles.
    pub square: bool,
    /// Fixed side of the squares, or the attributes that hold the sides along x and y (metres).
    pub side: Option<f64>,
    pub side_x_field: Option<String>,
    pub side_y_field: Option<String>,
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

/// Reads regions from a shapefile (polygons, or points with a radius) or a text list.
pub fn read_regions(path: &Path, opts: &ReadOptions) -> Result<Vec<Region>> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "shp" {
        read_shapefile(path, opts)
    } else {
        read_text_regions(path, opts)
    }
}

fn read_shapefile(path: &Path, opts: &ReadOptions) -> Result<Vec<Region>> {
    // The table's fields in the file's own order (a record comes as an unordered map).
    let order: Vec<String> = dbase::Reader::from_path(path.with_extension("dbf"))
        .map(|r| r.fields().iter().map(|f| f.name().to_string()).collect())
        .unwrap_or_default();
    let mut reader = shapefile::Reader::from_path(path).map_err(shp_err)?;
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
                // A number is a 1-based position among the fields, as in FUSION's /shape.
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
                    "{}: shapes of type {} can't clip points (use polygons, or points with a radius)",
                    path.display(),
                    other.shapetype()
                )));
            }
        };
        out.push(Region { name, shape, attrs });
    }
    Ok(out)
}

/// A plot centred on a point: a circle, or a square or rectangle (sides from the attributes or
/// fixed, in metres).
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
            "{}: the plots are squares: give the side (or the field that holds it)",
            path.display()
        ))),
    }
}

fn circle(x: f64, y: f64, radius: Option<f64>, path: &Path) -> Result<Shape2> {
    match radius.filter(|r| *r > 0.0) {
        Some(r) => Ok(Shape2::Circle { cx: x, cy: y, r }),
        None => Err(Error::Param(format!(
            "{}: the shapes are points: give a radius (or the field that holds it)",
            path.display()
        ))),
    }
}

/// A text list, one region a line: `name x y radius`, `x y radius`, or `name minx miny maxx
/// maxy`; separated by commas, semicolons, tabs or spaces; a first line that is not numbers is
/// a header.
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
            "{}: no regions found in it",
            path.display()
        )));
    }
    Ok(out)
}

fn shp_err(e: shapefile::Error) -> Error {
    Error::Format(format!("shapefile: {e}"))
}

/// A value in an attribute table.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Text(String),
    Num(f64),
    Int(i64),
}

/// A column of an attribute table.
#[derive(Clone, Debug, PartialEq)]
pub enum ColKind {
    /// Text of up to this many bytes.
    Text(u8),
    /// A number: total width and decimals.
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

/// At most `len` bytes of a string, cut on a character boundary.
fn truncate(s: &str, len: usize) -> String {
    let mut end = len.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Writes points as a shapefile, 3D when `with_z`; row `k` of `table` describes point `k`.
pub fn write_points(
    path: &Path,
    pts: &[[f64; 3]],
    with_z: bool,
    table: &Table,
    prj: Option<&str>,
) -> Result<()> {
    let mut w = shapefile::Writer::from_path(path, table.builder()?).map_err(shp_err)?;
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

/// Writes polygons (each ring an outer ring, clockwise) as a shapefile.
pub fn write_polygons(
    path: &Path,
    polys: &[Polygon],
    table: &Table,
    prj: Option<&str>,
) -> Result<()> {
    let mut w = shapefile::Writer::from_path(path, table.builder()?).map_err(shp_err)?;
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

/// Writes polylines (contours, profiles) as a shapefile.
pub fn write_polylines(
    path: &Path,
    lines: &[Vec<[f64; 3]>],
    with_z: bool,
    table: &Table,
    prj: Option<&str>,
) -> Result<()> {
    let mut w = shapefile::Writer::from_path(path, table.builder()?).map_err(shp_err)?;
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

/// What a shapefile holds that a viewer draws: polylines and polygons as rings of 3D points,
/// and points.
#[derive(Clone, Debug, Default)]
pub struct Drawing {
    pub points: Vec<[f64; 3]>,
    /// Open and closed parts; `true` for a ring of a polygon.
    pub lines: Vec<(Vec<[f64; 3]>, bool)>,
    /// Whether the shapes carry their own elevations.
    pub has_z: bool,
}

/// Reads any shapefile for display, keeping elevations when it has them.
pub fn read_drawing(path: &Path) -> Result<Drawing> {
    let mut reader = shapefile::ShapeReader::from_path(path).map_err(shp_err)?;
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

/// The EPSG code a `.prj` text names: from its authority, or from the name of a UTM zone of
/// ETRS89, WGS 84 or ED50.
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

/// A `.prj` text (OGC WKT with its EPSG identifiers, which every GIS reads the same way) for a
/// UTM code of ETRS89, WGS 84 or ED50.
pub fn prj_for_epsg(code: u16) -> Option<String> {
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
    #[test]
    fn prj_text_names_the_code_and_reads_back() {
        for code in [25829u16, 25830, 32630, 23030] {
            let t = prj_for_epsg(code).unwrap_or_default();
            assert!(
                t.ends_with(&format!("AUTHORITY[\"EPSG\",\"{code}\"]]")),
                "{t}"
            );
            assert_eq!(epsg_from_prj(&t), Some(code), "{t}");
        }
    }

    /// Writes one layer of each kind to the folder named by `RUSTLIDAR_MARKUP_OUT`, to open them in
    /// other software (`cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn write_layer_samples() -> Result<()> {
        use crate::view::{Feature, GeomKind, VectorData};
        let Some(dir) = std::env::var_os("RUSTLIDAR_MARKUP_OUT") else {
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
        // Clockwise, as shapefile outer rings.
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
        // A star with many vertices, so the mask is used.
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
                vec![Val::Text("parcela A".into()), Val::Num(100.0)],
                vec![Val::Text("parcela B".into()), Val::Num(100.0)],
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
        assert_eq!(regions[1].name, "parcela B");
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
// Attribute tables on polygons (metrics by cell or by plot) as shapefiles.

/// Words of FUSION's metric names and the short form used in a 10-character DBF field name.
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

/// DBF field names (10 characters at most, unique) for long column names, with the long names
/// they stand for. Strata columns become `E`/`I` + `S` + stratum number + statistic.
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

/// Records with polygon geometry written one at a time, so a table with a row per grid cell
/// never sits in memory. Column types come from the first row: whole numbers, decimals or text.
pub struct PolyTable {
    w: shapefile::Writer<std::io::BufWriter<std::fs::File>>,
    names: Vec<String>,
    kinds: Vec<FieldKind>,
    path: std::path::PathBuf,
    prj: Option<String>,
}

impl PolyTable {
    /// Creates `path` (and `<name>_campos.txt` with the long names when they were shortened).
    pub fn create(
        path: &Path,
        columns: &[String],
        first_row: &[String],
        prj: Option<&str>,
    ) -> Result<Self> {
        let named = dbf_names(columns);
        let kinds: Vec<FieldKind> = columns
            .iter()
            .enumerate()
            .map(|(k, _)| match first_row.get(k).map(|v| v.trim()) {
                Some(v) if v.parse::<i64>().is_ok() => FieldKind::Int,
                Some(v) if v.parse::<f64>().is_ok() => FieldKind::Float,
                _ => FieldKind::Text,
            })
            .collect();
        let mut b = TableWriterBuilder::new();
        for ((short, _), kind) in named.iter().zip(&kinds) {
            let n = FieldName::try_from(short.as_str())
                .map_err(|e| Error::Param(format!("field '{short}': {e}")))?;
            b = match kind {
                FieldKind::Int => b.add_numeric_field(n, 12, 0),
                FieldKind::Float => b.add_numeric_field(n, 18, 4),
                FieldKind::Text => b.add_character_field(n, 50),
            };
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let w = shapefile::Writer::from_path(path, b).map_err(shp_err)?;
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
            w,
            names: named.into_iter().map(|n| n.0).collect(),
            kinds,
            path: path.to_path_buf(),
            prj: prj.map(str::to_string),
        })
    }

    /// Adds a record: its rings (each an outer ring) and one value per column.
    pub fn push(&mut self, rings: &[Vec<[f64; 2]>], row: &[String]) -> Result<()> {
        let mut rec = dbase::Record::default();
        for (k, name) in self.names.iter().enumerate() {
            let v = row.get(k).map_or("", |v| v.trim());
            let value = match self.kinds[k] {
                FieldKind::Text => FieldValue::Character(Some(truncate(v, 50))),
                _ => FieldValue::Numeric(v.parse::<f64>().ok()),
            };
            rec.insert(name.clone(), value);
        }
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
        self.w
            .write_shape_and_record(&shapefile::Polygon::with_rings(parts), &rec)
            .map_err(shp_err)
    }

    pub fn finish(self) -> Result<()> {
        drop(self.w);
        write_prj(&self.path, self.prj.as_deref())
    }
}

/// Turns a metrics CSV into a shapefile of square cells. Each row names its cell by `row` and
/// `col`; `centres` gives the centre of every `(row, col)` (filled from the first file that has
/// `center X`/`center Y` columns, so the strata file, which has none, can use it too).
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
        let row: Vec<String> = line.split(',').map(str::to_string).collect();
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

/// Saves a layer made in the viewer: its features and its attribute table (a field is a number
/// when every value in it is one, else text), with the `.prj` text of the coordinate system.
pub fn write_layer(
    path: &Path,
    d: &crate::view::VectorData,
    kind: crate::view::GeomKind,
    prj: Option<&str>,
) -> Result<()> {
    use crate::view::GeomKind;
    let numeric: Vec<bool> = (0..d.fields.len())
        .map(|c| {
            d.text.iter().all(|row| {
                row.get(c)
                    .is_none_or(|t| t.trim().is_empty() || t.trim().parse::<f64>().is_ok())
            }) && !d.text.is_empty()
        })
        .collect();
    let names: Vec<String> = dbf_names(&d.fields).into_iter().map(|n| n.0).collect();
    let mut b = TableWriterBuilder::new();
    for (name, num) in names.iter().zip(&numeric) {
        let n = FieldName::try_from(name.as_str())
            .map_err(|e| Error::Param(format!("field '{name}': {e}")))?;
        b = if *num {
            b.add_numeric_field(n, 18, 6)
        } else {
            b.add_character_field(n, 80)
        };
    }
    let mut w = shapefile::Writer::from_path(path, b).map_err(shp_err)?;
    for (k, f) in d.features.iter().enumerate() {
        let mut rec = dbase::Record::default();
        for (c, name) in names.iter().enumerate() {
            let t = d
                .text
                .get(k)
                .and_then(|r| r.get(c))
                .map_or("", String::as_str)
                .trim();
            rec.insert(
                name.clone(),
                if numeric[c] {
                    FieldValue::Numeric(t.parse().ok())
                } else {
                    FieldValue::Character(Some(truncate(t, 80)))
                },
            );
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
                let rings: Vec<PolygonRing<ShpPoint>> = f
                    .parts
                    .iter()
                    .filter(|r| r.len() >= 3)
                    .map(|r| {
                        let area: f64 = r
                            .iter()
                            .zip(r.iter().cycle().skip(1))
                            .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
                            .sum();
                        let mut pts: Vec<ShpPoint> =
                            r.iter().map(|p| ShpPoint::new(p[0], p[1])).collect();
                        if area > 0.0 {
                            pts.reverse();
                        }
                        if pts.first() != pts.last() {
                            pts.push(pts[0]);
                        }
                        PolygonRing::Outer(pts)
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
    write_prj(path, prj)
}
