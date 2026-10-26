//! Writes GeoPackage files (OGC GeoPackage 1.x, vector features) in pure Rust: a small SQLite
//! file writer (pages, table b-trees, overflow pages) and the GeoPackage tables on top of it.
//! The layout follows what GDAL and RustGIS write (`application_id` "GPKG", `user_version`
//! 10400, `gpkg_spatial_ref_sys`, `gpkg_contents`, `gpkg_geometry_columns`, geometry in
//! GeoPackage binary with WKB). There is no R-tree index and no primary-key indexes on the
//! metadata tables: every reader that follows the format opens it without them.

use std::path::Path;

use crate::geojson::signed_area;
use crate::view::{GeomKind, VectorData};
use crate::{Error, Result};

const PAGE: usize = 8192;

/// One feature table to write.
pub struct Table<'a> {
    pub name: &'a str,
    pub data: &'a VectorData,
    pub kind: GeomKind,
    pub epsg: Option<u16>,
}

// ------------------------------------------------------------------------------ SQLite pages

enum Val {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

fn varint(mut v: u64, out: &mut Vec<u8>) {
    if v > 0x00ff_ffff_ffff_ffff {
        let low = (v & 0xff) as u8;
        v >>= 8;
        let mut head = [0u8; 8];
        for b in head.iter_mut().rev() {
            *b = (v & 0x7f) as u8 | 0x80;
            v >>= 7;
        }
        out.extend(head);
        out.push(low);
        return;
    }
    let mut buf = [0u8; 9];
    let mut n = 0;
    loop {
        buf[n] = (v & 0x7f) as u8;
        v >>= 7;
        n += 1;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(if i == 0 { buf[i] } else { buf[i] | 0x80 });
    }
}

/// A row in SQLite's record format.
fn record(vals: &[Val]) -> Vec<u8> {
    let (mut types, mut body) = (Vec::new(), Vec::new());
    for v in vals {
        match v {
            Val::Null => varint(0, &mut types),
            Val::Int(0) => varint(8, &mut types),
            Val::Int(1) => varint(9, &mut types),
            Val::Int(i) => {
                let i = *i;
                let (t, n) = if i8::try_from(i).is_ok() {
                    (1, 1)
                } else if i16::try_from(i).is_ok() {
                    (2, 2)
                } else if (-(1 << 23)..1 << 23).contains(&i) {
                    (3, 3)
                } else if i32::try_from(i).is_ok() {
                    (4, 4)
                } else if (-(1 << 47)..1 << 47).contains(&i) {
                    (5, 6)
                } else {
                    (6, 8)
                };
                varint(t, &mut types);
                body.extend(&i.to_be_bytes()[8 - n..]);
            }
            Val::Real(f) => {
                varint(7, &mut types);
                body.extend(f.to_be_bytes());
            }
            Val::Text(s) => {
                varint(13 + 2 * s.len() as u64, &mut types);
                body.extend(s.as_bytes());
            }
            Val::Blob(b) => {
                varint(12 + 2 * b.len() as u64, &mut types);
                body.extend(b);
            }
        }
    }
    let mut head_len = types.len() + 1;
    if head_len > 127 {
        head_len += 1;
    }
    let mut out = Vec::new();
    varint(head_len as u64, &mut out);
    out.extend(types);
    out.extend(body);
    out
}

#[derive(Default)]
struct Pager {
    pages: Vec<Vec<u8>>,
}

impl Pager {
    fn alloc(&mut self) -> u32 {
        self.pages.push(vec![0; PAGE]);
        self.pages.len() as u32
    }

    /// The cell of a row of a table b-tree; what does not fit in the page goes to overflow pages.
    fn cell(&mut self, rowid: i64, payload: &[u8]) -> Vec<u8> {
        let (u, x) = (PAGE, PAGE - 35);
        let mut c = Vec::new();
        varint(payload.len() as u64, &mut c);
        varint(rowid as u64, &mut c);
        if payload.len() <= x {
            c.extend(payload);
            return c;
        }
        let m = (u - 12) * 32 / 255 - 23;
        let k = m + (payload.len() - m) % (u - 4);
        let local = if k <= x { k } else { m };
        c.extend(&payload[..local]);
        let (mut first, mut prev) = (0u32, 0u32);
        for chunk in payload[local..].chunks(u - 4) {
            let pg = self.alloc();
            if prev == 0 {
                first = pg;
            } else {
                self.pages[prev as usize - 1][..4].copy_from_slice(&pg.to_be_bytes());
            }
            self.pages[pg as usize - 1][4..4 + chunk.len()].copy_from_slice(chunk);
            prev = pg;
        }
        c.extend(first.to_be_bytes());
        c
    }

    /// A table b-tree holding the rows (sorted by rowid); its root page.
    fn table(&mut self, rows: &[(i64, Vec<u8>)]) -> u32 {
        let mut level: Vec<(u32, i64)> = Vec::new();
        let mut cells: Vec<Vec<u8>> = Vec::new();
        let (mut used, mut last) = (0usize, 0i64);
        for (rowid, payload) in rows {
            let cell = self.cell(*rowid, payload);
            if !cells.is_empty() && 8 + 2 * (cells.len() + 1) + used + cell.len() > PAGE {
                level.push((self.leaf(&cells, 0), last));
                cells.clear();
                used = 0;
            }
            used += cell.len();
            cells.push(cell);
            last = *rowid;
        }
        level.push((self.leaf(&cells, 0), last));
        while level.len() > 1 {
            let mut next = Vec::new();
            let mut i = 0;
            while i < level.len() {
                // children whose cells (a pointer and a key) and the right pointer fit
                let (mut n, mut size) = (0usize, 12usize);
                while i + n < level.len() {
                    let mut key = Vec::new();
                    varint(level[i + n].1 as u64, &mut key);
                    let add = 2 + 4 + key.len();
                    if n >= 2 && size + add > PAGE {
                        break;
                    }
                    size += add;
                    n += 1;
                }
                let group = &level[i..i + n];
                next.push((self.interior(group), group[n - 1].1));
                i += n;
            }
            level = next;
        }
        level[0].0
    }

    /// A leaf page of a table b-tree; `at` is where its header starts (100 on page 1).
    fn leaf_into(page: &mut [u8], cells: &[Vec<u8>], at: usize) -> bool {
        let total: usize = cells.iter().map(Vec::len).sum();
        if at + 8 + 2 * cells.len() + total > PAGE {
            return false;
        }
        page[at] = 0x0d;
        page[at + 3..at + 5].copy_from_slice(&(cells.len() as u16).to_be_bytes());
        let mut pos = PAGE;
        for (k, c) in cells.iter().enumerate() {
            pos -= c.len();
            page[pos..pos + c.len()].copy_from_slice(c);
            page[at + 8 + 2 * k..at + 10 + 2 * k].copy_from_slice(&(pos as u16).to_be_bytes());
        }
        page[at + 5..at + 7].copy_from_slice(&(pos as u16).to_be_bytes());
        true
    }

    fn leaf(&mut self, cells: &[Vec<u8>], at: usize) -> u32 {
        let pg = self.alloc();
        Self::leaf_into(&mut self.pages[pg as usize - 1], cells, at);
        pg
    }

    fn interior(&mut self, children: &[(u32, i64)]) -> u32 {
        let pg = self.alloc();
        Self::interior_into(&mut self.pages[pg as usize - 1], children, 0);
        pg
    }

    /// An interior page of a table b-tree whose header starts at `at`; false when it does not fit.
    fn interior_into(page: &mut [u8], children: &[(u32, i64)], at: usize) -> bool {
        let n = children.len() - 1;
        let cells: Vec<Vec<u8>> = children[..n]
            .iter()
            .map(|(child, key)| {
                let mut c = child.to_be_bytes().to_vec();
                varint(*key as u64, &mut c);
                c
            })
            .collect();
        if at + 12 + 2 * n + cells.iter().map(Vec::len).sum::<usize>() > PAGE {
            return false;
        }
        page[at] = 0x05;
        page[at + 3..at + 5].copy_from_slice(&(n as u16).to_be_bytes());
        page[at + 8..at + 12].copy_from_slice(&children[n].0.to_be_bytes());
        let mut pos = PAGE;
        for (k, c) in cells.iter().enumerate() {
            pos -= c.len();
            page[pos..pos + c.len()].copy_from_slice(c);
            page[at + 12 + 2 * k..at + 14 + 2 * k].copy_from_slice(&(pos as u16).to_be_bytes());
        }
        page[at + 5..at + 7].copy_from_slice(&(pos as u16).to_be_bytes());
        true
    }
}

// --------------------------------------------------------------------------------- geometry

fn le_point(out: &mut Vec<u8>, p: &[f64; 2]) {
    out.extend(p[0].to_le_bytes());
    out.extend(p[1].to_le_bytes());
}

fn wkb_head(out: &mut Vec<u8>, ty: u32) {
    out.push(1);
    out.extend(ty.to_le_bytes());
}

fn ring(out: &mut Vec<u8>, r: &[[f64; 2]], reversed: bool) {
    let closed = r.first() == r.last();
    out.extend((r.len() as u32 + u32::from(!closed)).to_le_bytes());
    let pts: Box<dyn Iterator<Item = &[f64; 2]>> = if reversed {
        Box::new(r.iter().rev())
    } else {
        Box::new(r.iter())
    };
    for p in pts {
        le_point(out, p);
    }
    if !closed && let Some(f) = if reversed { r.last() } else { r.first() } {
        le_point(out, f);
    }
}

/// The polygons of a feature: each exterior ring (clockwise in the program's convention) starts
/// one and the holes that follow it belong to it.
fn polygons(parts: &[Vec<[f64; 2]>]) -> Vec<Vec<&Vec<[f64; 2]>>> {
    let mut out: Vec<Vec<&Vec<[f64; 2]>>> = Vec::new();
    for r in parts.iter().filter(|r| r.len() >= 3) {
        if signed_area(r) <= 0.0 || out.is_empty() {
            out.push(vec![r]);
        } else if let Some(last) = out.last_mut() {
            last.push(r);
        }
    }
    out
}

/// The geometry of a feature as GeoPackage binary, and the type name it needs.
fn geometry(parts: &[Vec<[f64; 2]>], kind: GeomKind, srs: i32) -> Option<Vec<u8>> {
    let mut b = vec![b'G', b'P', 0, 1];
    b.extend(srs.to_le_bytes());
    match kind {
        GeomKind::Point => {
            let pts: Vec<&[f64; 2]> = parts.iter().filter_map(|p| p.first()).collect();
            match pts.len() {
                0 => return None,
                1 => {
                    wkb_head(&mut b, 1);
                    le_point(&mut b, pts[0]);
                }
                n => {
                    wkb_head(&mut b, 4);
                    b.extend((n as u32).to_le_bytes());
                    for p in pts {
                        wkb_head(&mut b, 1);
                        le_point(&mut b, p);
                    }
                }
            }
        }
        GeomKind::Line => {
            let lines: Vec<&Vec<[f64; 2]>> = parts.iter().filter(|p| p.len() >= 2).collect();
            let one = |b: &mut Vec<u8>, l: &Vec<[f64; 2]>| {
                wkb_head(b, 2);
                b.extend((l.len() as u32).to_le_bytes());
                for p in l {
                    le_point(b, p);
                }
            };
            match lines.len() {
                0 => return None,
                1 => one(&mut b, lines[0]),
                n => {
                    wkb_head(&mut b, 5);
                    b.extend((n as u32).to_le_bytes());
                    for l in lines {
                        one(&mut b, l);
                    }
                }
            }
        }
        GeomKind::Polygon => {
            let polys = polygons(parts);
            let one = |b: &mut Vec<u8>, p: &Vec<&Vec<[f64; 2]>>| {
                wkb_head(b, 3);
                b.extend((p.len() as u32).to_le_bytes());
                // counter-clockwise exterior and clockwise holes, as the OGC wants them
                for r in p {
                    ring(b, r, true);
                }
            };
            match polys.len() {
                0 => return None,
                1 => one(&mut b, &polys[0]),
                n => {
                    wkb_head(&mut b, 6);
                    b.extend((n as u32).to_le_bytes());
                    for p in &polys {
                        one(&mut b, p);
                    }
                }
            }
        }
    }
    Some(b)
}

fn type_name(d: &VectorData, kind: GeomKind) -> &'static str {
    let multi = d.features.iter().any(|f| match kind {
        GeomKind::Point => f.parts.iter().filter(|p| !p.is_empty()).count() > 1,
        GeomKind::Line => f.parts.iter().filter(|p| p.len() >= 2).count() > 1,
        GeomKind::Polygon => polygons(&f.parts).len() > 1,
    });
    match (kind, multi) {
        (GeomKind::Point, false) => "POINT",
        (GeomKind::Point, true) => "MULTIPOINT",
        (GeomKind::Line, false) => "LINESTRING",
        (GeomKind::Line, true) => "MULTILINESTRING",
        (GeomKind::Polygon, false) => "POLYGON",
        (GeomKind::Polygon, true) => "MULTIPOLYGON",
    }
}

// --------------------------------------------------------------------------------- the file

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// `2026-10-26T14:30:00.000Z` from the clock (days since 1970 to a calendar date, Hinnant's
/// algorithm).
fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn srs_row(epsg: i64) -> Vec<Val> {
    let (name, org, definition, desc) = match epsg {
        -1 => (
            "Undefined Cartesian SRS".to_string(),
            "NONE",
            "undefined".to_string(),
            "undefined Cartesian coordinate reference system",
        ),
        0 => (
            "Undefined geographic SRS".to_string(),
            "NONE",
            "undefined".to_string(),
            "undefined geographic coordinate reference system",
        ),
        e => {
            let sr = rustlidar_crs::SpatialRef::from_epsg(e as u32).ok();
            (
                sr.as_ref()
                    .and_then(|s| s.name())
                    .unwrap_or_else(|| format!("EPSG:{e}")),
                "EPSG",
                sr.and_then(|s| s.to_wkt())
                    .unwrap_or_else(|| "undefined".into()),
                "",
            )
        }
    };
    vec![
        Val::Text(name),
        Val::Null,
        Val::Text(org.into()),
        Val::Int(if epsg > 0 { epsg } else { epsg.min(0) }),
        Val::Text(definition),
        Val::Text(desc.into()),
    ]
}

/// How a field is stored: whole numbers, decimals or text.
#[derive(Clone, Copy, PartialEq)]
enum Col {
    Int,
    Real,
    Text,
}

fn column_kind(d: &VectorData, c: usize) -> Col {
    let mut kind = Col::Int;
    let mut any = false;
    for row in &d.text {
        let t = row.get(c).map_or("", |t| t.trim());
        if t.is_empty() {
            continue;
        }
        any = true;
        match t.parse::<f64>() {
            Ok(v) if v.is_finite() => {
                if kind == Col::Int
                    && (v.fract() != 0.0 || v.abs() > 9e15 || t.contains(['.', 'e', 'E']))
                {
                    kind = Col::Real;
                }
            }
            _ => return Col::Text,
        }
    }
    if any { kind } else { Col::Text }
}

/// Writes the tables to a new GeoPackage (an existing file is replaced).
pub fn write(path: &Path, tables: &[Table]) -> Result<()> {
    let mut pager = Pager::default();
    pager.alloc(); // page 1 is the schema; it is filled in last
    let mut master: Vec<(String, String, String, u32, String)> = Vec::new();
    let mut add = |kind: &str, name: &str, root: u32, sql: &str| {
        master.push((kind.into(), name.into(), name.into(), root, sql.into()));
    };

    // spatial reference systems
    let mut ids: Vec<i64> = vec![-1, 0, 4326];
    for t in tables {
        if let Some(e) = t.epsg
            && !ids.contains(&i64::from(e))
        {
            ids.push(i64::from(e));
        }
    }
    ids.sort_unstable();
    let rows: Vec<(i64, Vec<u8>)> = ids.iter().map(|e| (*e, record(&srs_row(*e)))).collect();
    let root = pager.table(&rows);
    add(
        "table",
        "gpkg_spatial_ref_sys",
        root,
        "CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT)",
    );

    // the feature tables
    let when = now_iso();
    let (mut contents, mut geom_cols) = (Vec::new(), Vec::new());
    let mut feature_tables = Vec::new();
    let mut extras: Vec<(String, String, String, u32, String)> = Vec::new();
    let mut ext_rows: Vec<Vec<u8>> = Vec::new();
    for t in tables {
        let d = t.data;
        let srs = t.epsg.map_or(-1, i64::from);
        // names: never `fid` or `geom`, never repeated
        let mut names: Vec<String> = Vec::new();
        // the columns written: a whole-number `fid` of the source is made again, not copied
        let src: Vec<usize> = (0..d.fields.len())
            .filter(|&c| {
                !(d.fields[c].trim().eq_ignore_ascii_case("fid") && column_kind(d, c) == Col::Int)
            })
            .collect();
        for &c in &src {
            let f = &d.fields[c];
            let mut n = f.trim().to_string();
            if n.is_empty() {
                n = "campo".into();
            }
            let base = n.clone();
            let mut k = 1;
            while names.iter().any(|o| o.eq_ignore_ascii_case(&n))
                || n.eq_ignore_ascii_case("fid")
                || n.eq_ignore_ascii_case("geom")
            {
                n = format!("{base}_{k}");
                k += 1;
            }
            names.push(n);
        }
        let kinds: Vec<Col> = src.iter().map(|&c| column_kind(d, c)).collect();
        let mut sql = format!(
            "CREATE TABLE {} (fid INTEGER PRIMARY KEY, geom BLOB",
            quote_ident(t.name)
        );
        for (n, k) in names.iter().zip(&kinds) {
            sql += &format!(
                ", {} {}",
                quote_ident(n),
                match k {
                    Col::Int => "INTEGER",
                    Col::Real => "REAL",
                    Col::Text => "TEXT",
                }
            );
        }
        sql.push(')');
        let mut rows = Vec::new();
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut fb: Vec<(i64, [f64; 4])> = Vec::new();
        for (i, f) in d.features.iter().enumerate() {
            let mut fe = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for p in f.parts.iter().flatten() {
                fe = [
                    fe[0].min(p[0]),
                    fe[1].min(p[1]),
                    fe[2].max(p[0]),
                    fe[3].max(p[1]),
                ];
            }
            b = [
                b[0].min(fe[0]),
                b[1].min(fe[1]),
                b[2].max(fe[2]),
                b[3].max(fe[3]),
            ];
            let geo = geometry(&f.parts, t.kind, srs as i32);
            if geo.is_some() && fe.iter().all(|v| v.is_finite()) {
                fb.push((i as i64 + 1, fe));
            }
            let mut vals = vec![Val::Null, geo.map_or(Val::Null, Val::Blob)];
            for (c, k) in src.iter().zip(&kinds) {
                let text = d
                    .text
                    .get(i)
                    .and_then(|r| r.get(*c))
                    .map_or("", |t| t.trim());
                vals.push(match (k, text.parse::<f64>()) {
                    _ if text.is_empty() => Val::Null,
                    (Col::Int, Ok(v)) => Val::Int(v as i64),
                    (Col::Real, Ok(v)) => Val::Real(v),
                    _ => Val::Text(text.to_string()),
                });
            }
            rows.push((i as i64 + 1, record(&vals)));
        }
        let root = pager.table(&rows);
        feature_tables.push((t.name.to_string(), root, sql));
        let tree = format!("rtree_{}_geom", t.name);
        let (n_root, p_root, r_root) = rtree_tables(&mut pager, &fb);
        let tq = quote_ident(&tree);
        extras.push((
            "table".into(),
            tree.clone(),
            tree.clone(),
            0,
            format!("CREATE VIRTUAL TABLE {tq} USING rtree(id, minx, maxx, miny, maxy)"),
        ));
        for (suffix, r, cols) in [
            ("node", n_root, "nodeno INTEGER PRIMARY KEY,data"),
            ("parent", p_root, "nodeno INTEGER PRIMARY KEY,parentnode"),
            ("rowid", r_root, "rowid INTEGER PRIMARY KEY,nodeno"),
        ] {
            let n = format!("{tree}_{suffix}");
            extras.push((
                "table".into(),
                n.clone(),
                n.clone(),
                r,
                format!("CREATE TABLE {}({cols})", quote_ident(&n)),
            ));
        }
        for (n, sql) in rtree_triggers(t.name, &tree) {
            extras.push(("trigger".into(), n, t.name.to_string(), 0, sql));
        }
        ext_rows.push(record(&[
            Val::Text(t.name.into()),
            Val::Text("geom".into()),
            Val::Text("gpkg_rtree_index".into()),
            Val::Text("http://www.geopackage.org/spec120/#extension_rtree".into()),
            Val::Text("write-only".into()),
        ]));
        let env = |v: f64| {
            if v.is_finite() {
                Val::Real(v)
            } else {
                Val::Null
            }
        };
        contents.push(record(&[
            Val::Text(t.name.into()),
            Val::Text("features".into()),
            Val::Text(t.name.into()),
            Val::Text(String::new()),
            Val::Text(when.clone()),
            env(b[0]),
            env(b[1]),
            env(b[2]),
            env(b[3]),
            Val::Int(srs),
        ]));
        geom_cols.push(record(&[
            Val::Text(t.name.into()),
            Val::Text("geom".into()),
            Val::Text(type_name(d, t.kind).into()),
            Val::Int(srs),
            Val::Int(0),
            Val::Int(0),
        ]));
    }
    let rows: Vec<(i64, Vec<u8>)> = contents
        .into_iter()
        .enumerate()
        .map(|(i, r)| (i as i64 + 1, r))
        .collect();
    let root = pager.table(&rows);
    add(
        "table",
        "gpkg_contents",
        root,
        "CREATE TABLE gpkg_contents (table_name TEXT NOT NULL, data_type TEXT NOT NULL, identifier TEXT, description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER)",
    );
    let rows: Vec<(i64, Vec<u8>)> = geom_cols
        .into_iter()
        .enumerate()
        .map(|(i, r)| (i as i64 + 1, r))
        .collect();
    let root = pager.table(&rows);
    add(
        "table",
        "gpkg_geometry_columns",
        root,
        "CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL)",
    );
    for (name, root, sql) in &feature_tables {
        add("table", name, *root, sql);
    }
    let rows: Vec<(i64, Vec<u8>)> = ext_rows
        .into_iter()
        .enumerate()
        .map(|(i, r)| (i as i64 + 1, r))
        .collect();
    let root = pager.table(&rows);
    add(
        "table",
        "gpkg_extensions",
        root,
        "CREATE TABLE gpkg_extensions (table_name TEXT, column_name TEXT, extension_name TEXT NOT NULL, definition TEXT NOT NULL, scope TEXT NOT NULL)",
    );
    master.extend(extras);

    // page 1: the schema table and the database header
    let cells: Vec<Vec<u8>> = master
        .iter()
        .enumerate()
        .map(|(i, (ty, name, tbl, root, sql))| {
            let rec = record(&[
                Val::Text(ty.clone()),
                Val::Text(name.clone()),
                Val::Text(tbl.clone()),
                Val::Int(i64::from(*root)),
                Val::Text(sql.clone()),
            ]);
            pager.cell(i as i64 + 1, &rec)
        })
        .collect();
    if !Pager::leaf_into(&mut pager.pages[0], &cells, 100) {
        // the schema takes several pages: page 1 points at the leaves that hold it
        let (mut leaves, mut cur, mut used) = (Vec::new(), Vec::new(), 0usize);
        for (i, c) in cells.into_iter().enumerate() {
            if !cur.is_empty() && 8 + 2 * (cur.len() + 1) + used + c.len() > PAGE {
                leaves.push((pager.leaf(&cur, 0), i as i64));
                cur = Vec::new();
                used = 0;
            }
            used += c.len();
            cur.push(c);
        }
        leaves.push((pager.leaf(&cur, 0), master.len() as i64));
        let mut page1 = std::mem::take(&mut pager.pages[0]);
        let fits = Pager::interior_into(&mut page1, &leaves, 100);
        pager.pages[0] = page1;
        if !fits {
            return Err(Error::Param(
                "demasiadas tablas o campos para el esquema de un GeoPackage".into(),
            ));
        }
    }
    let n_pages = pager.pages.len() as u32;
    let h = &mut pager.pages[0];
    h[..16].copy_from_slice(b"SQLite format 3\0");
    h[16..18].copy_from_slice(&(PAGE as u16).to_be_bytes());
    h[18] = 1;
    h[19] = 1;
    h[21] = 64;
    h[22] = 32;
    h[23] = 32;
    h[24..28].copy_from_slice(&1u32.to_be_bytes());
    h[28..32].copy_from_slice(&n_pages.to_be_bytes());
    h[40..44].copy_from_slice(&1u32.to_be_bytes());
    h[44..48].copy_from_slice(&4u32.to_be_bytes());
    h[56..60].copy_from_slice(&1u32.to_be_bytes());
    h[60..64].copy_from_slice(&10400u32.to_be_bytes());
    h[68..72].copy_from_slice(&1_196_444_487u32.to_be_bytes());
    h[92..96].copy_from_slice(&1u32.to_be_bytes());
    h[96..100].copy_from_slice(&3_045_000u32.to_be_bytes());
    let mut out = Vec::with_capacity(pager.pages.len() * PAGE);
    for p in &pager.pages {
        out.extend(p);
    }
    std::fs::write(path, out)?;
    Ok(())
}

// ---------------------------------------------------------------------------- R-tree extension

/// A node holds the 51 cells SQLite allows; every node of a tree has the same size.
const NODE: usize = 4 + 24 * 51;
/// SQLite never puts more than 51 cells in a node (RTREE_MAXCELLS); more makes it loop.
const CAP: usize = 50;

/// An f32 not above `v`.
fn f32_down(v: f64) -> f32 {
    let f = v as f32;
    if f64::from(f) > v { f.next_down() } else { f }
}

/// An f32 not below `v`.
fn f32_up(v: f64) -> f32 {
    let f = v as f32;
    if f64::from(f) < v { f.next_up() } else { f }
}

fn node_blob(depth: u16, cells: &[(i64, [f64; 4])]) -> Vec<u8> {
    let mut b = vec![0u8; NODE];
    b[..2].copy_from_slice(&depth.to_be_bytes());
    b[2..4].copy_from_slice(&(cells.len() as u16).to_be_bytes());
    for (k, (id, e)) in cells.iter().enumerate() {
        let at = 4 + 24 * k;
        b[at..at + 8].copy_from_slice(&id.to_be_bytes());
        // minx, maxx, miny, maxy, rounded outward so that a box never loses a feature
        for (j, v) in [f32_down(e[0]), f32_up(e[2]), f32_down(e[1]), f32_up(e[3])]
            .iter()
            .enumerate()
        {
            b[at + 8 + 4 * j..at + 12 + 4 * j].copy_from_slice(&v.to_be_bytes());
        }
    }
    b
}

fn str_sort<T>(items: &mut [(T, [f64; 4])]) {
    let slabs = (items.len().div_ceil(CAP) as f64).sqrt().ceil().max(1.0) as usize;
    items.sort_unstable_by(|a, b| (a.1[0] + a.1[2]).total_cmp(&(b.1[0] + b.1[2])));
    for s in items.chunks_mut(slabs * CAP) {
        s.sort_unstable_by(|a, b| (a.1[1] + a.1[3]).total_cmp(&(b.1[1] + b.1[3])));
    }
}

/// The three shadow tables of an R-tree over `items` (feature id and extent): the b-tree roots
/// of `_node`, `_parent` and `_rowid`.
fn rtree_tables(p: &mut Pager, items: &[(i64, [f64; 4])]) -> (u32, u32, u32) {
    let mut sorted = items.to_vec();
    str_sort(&mut sorted);
    let mut nodes: Vec<Vec<(i64, [f64; 4])>> = Vec::new();
    let mut parent: Vec<usize> = Vec::new();
    let mut leaf: Vec<bool> = Vec::new();
    let mut level: Vec<usize> = Vec::new();
    for c in sorted.chunks(CAP) {
        nodes.push(c.to_vec());
        parent.push(usize::MAX);
        leaf.push(true);
        level.push(nodes.len() - 1);
    }
    if nodes.is_empty() {
        nodes.push(Vec::new());
        parent.push(usize::MAX);
        leaf.push(true);
        level.push(0);
    }
    let extent = |cells: &[(i64, [f64; 4])]| {
        cells.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |a, c| {
                [
                    a[0].min(c.1[0]),
                    a[1].min(c.1[1]),
                    a[2].max(c.1[2]),
                    a[3].max(c.1[3]),
                ]
            },
        )
    };
    let mut depth = 0u16;
    while level.len() > 1 {
        let mut keyed: Vec<(usize, [f64; 4])> =
            level.iter().map(|&k| (k, extent(&nodes[k]))).collect();
        str_sort(&mut keyed);
        let mut next = Vec::new();
        for c in keyed.chunks(CAP) {
            nodes.push(c.iter().map(|(k, e)| (*k as i64, *e)).collect());
            parent.push(usize::MAX);
            leaf.push(false);
            let me = nodes.len() - 1;
            for (k, _) in c {
                parent[*k] = me;
            }
            next.push(me);
        }
        level = next;
        depth += 1;
    }
    let root = level[0];
    // the root is node 1, the others follow in the order they were made
    let mut no = vec![0i64; nodes.len()];
    let mut n = 1;
    no[root] = 1;
    for k in (0..nodes.len()).filter(|&k| k != root) {
        n += 1;
        no[k] = n;
    }
    let mut node_rows: Vec<(i64, Vec<u8>)> = Vec::new();
    let (mut rowid_rows, mut parent_rows) = (Vec::new(), Vec::new());
    for k in 0..nodes.len() {
        let cells: Vec<(i64, [f64; 4])> = if leaf[k] {
            for (id, _) in &nodes[k] {
                rowid_rows.push((*id, record(&[Val::Null, Val::Int(no[k])])));
            }
            nodes[k].clone()
        } else {
            nodes[k]
                .iter()
                .map(|(c, e)| (no[*c as usize], *e))
                .collect()
        };
        if k != root {
            parent_rows.push((no[k], record(&[Val::Null, Val::Int(no[parent[k]])])));
        }
        node_rows.push((
            no[k],
            record(&[
                Val::Null,
                Val::Blob(node_blob(if k == root { depth } else { 0 }, &cells)),
            ]),
        ));
    }
    node_rows.sort_by_key(|r| r.0);
    parent_rows.sort_by_key(|r| r.0);
    rowid_rows.sort_by_key(|r| r.0);
    (
        p.table(&node_rows),
        p.table(&parent_rows),
        p.table(&rowid_rows),
    )
}

/// The six triggers the extension asks for, which keep the index in step with the table.
fn rtree_triggers(t: &str, tree: &str) -> Vec<(String, String)> {
    let (tq, rq) = (quote_ident(t), quote_ident(tree));
    let put = format!(
        "INSERT OR REPLACE INTO {rq} VALUES (NEW.fid, ST_MinX(NEW.geom), ST_MaxX(NEW.geom), ST_MinY(NEW.geom), ST_MaxY(NEW.geom))"
    );
    let name = |s: &str| quote_ident(&format!("{tree}_{s}"));
    vec![
        (
            format!("{tree}_insert"),
            format!(
                "CREATE TRIGGER {} AFTER INSERT ON {tq} WHEN (NEW.geom NOT NULL AND NOT ST_IsEmpty(NEW.geom)) BEGIN {put}; END",
                name("insert")
            ),
        ),
        (
            format!("{tree}_update1"),
            format!(
                "CREATE TRIGGER {} AFTER UPDATE OF geom ON {tq} WHEN OLD.fid = NEW.fid AND (NEW.geom NOTNULL AND NOT ST_IsEmpty(NEW.geom)) BEGIN {put}; END",
                name("update1")
            ),
        ),
        (
            format!("{tree}_update2"),
            format!(
                "CREATE TRIGGER {} AFTER UPDATE OF geom ON {tq} WHEN OLD.fid = NEW.fid AND (NEW.geom IS NULL OR ST_IsEmpty(NEW.geom)) BEGIN DELETE FROM {rq} WHERE id = OLD.fid; END",
                name("update2")
            ),
        ),
        (
            format!("{tree}_update3"),
            format!(
                "CREATE TRIGGER {} AFTER UPDATE ON {tq} WHEN OLD.fid != NEW.fid AND (NEW.geom NOTNULL AND NOT ST_IsEmpty(NEW.geom)) BEGIN DELETE FROM {rq} WHERE id = OLD.fid; {put}; END",
                name("update3")
            ),
        ),
        (
            format!("{tree}_update4"),
            format!(
                "CREATE TRIGGER {} AFTER UPDATE ON {tq} WHEN OLD.fid != NEW.fid AND (NEW.geom IS NULL OR ST_IsEmpty(NEW.geom)) BEGIN DELETE FROM {rq} WHERE id IN (OLD.fid, NEW.fid); END",
                name("update4")
            ),
        ),
        (
            format!("{tree}_delete"),
            format!(
                "CREATE TRIGGER {} AFTER DELETE ON {tq} WHEN OLD.geom NOT NULL BEGIN DELETE FROM {rq} WHERE id = OLD.fid; END",
                name("delete")
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Feature;

    fn layer() -> VectorData {
        let sq =
            |x: f64, y: f64, s: f64| vec![[x, y], [x, y + s], [x + s, y + s], [x + s, y], [x, y]];
        // exterior clockwise and a hole counter-clockwise, as the program keeps them
        let mut hole = sq(2.0, 2.0, 2.0);
        hole.reverse();
        let features = vec![
            Feature {
                parts: vec![sq(0.0, 0.0, 10.0), hole],
            },
            Feature {
                parts: vec![sq(20.0, 0.0, 5.0)],
            },
        ];
        VectorData {
            features,
            polygons: true,
            points: false,
            fields: vec!["Nombre".into(), "Area".into(), "Entero".into()],
            text: vec![
                vec!["uno".into(), "96.5".into(), "7".into()],
                vec!["dos".into(), "25".into(), "8".into()],
            ],
            num: vec![vec![f64::NAN, 96.5, 7.0], vec![f64::NAN, 25.0, 8.0]],
            bounds: [0.0, 0.0, 25.0, 10.0],
        }
    }

    #[test]
    fn a_polygon_layer_reads_back_with_its_hole_and_attributes() -> Result<()> {
        let path = std::env::temp_dir().join(format!("rl_gpkgw_{}.gpkg", std::process::id()));
        let d = layer();
        write(
            &path,
            &[Table {
                name: "parcelas",
                data: &d,
                kind: GeomKind::Polygon,
                epsg: Some(25830),
            }],
        )?;
        let tables = crate::gpkg::layers(&path)?;
        assert_eq!(tables.len(), 1);
        assert_eq!(
            (
                tables[0].table.as_str(),
                tables[0].srs_id,
                tables[0].geometry_type.as_str()
            ),
            ("parcelas", 25830, "POLYGON")
        );
        assert_eq!(crate::gpkg::epsg_hint(&path), Some(25830));
        let back = crate::gpkg::read_vector(&path, Some("parcelas"))?;
        assert_eq!(back.features.len(), 2);
        assert_eq!(back.features[0].parts.len(), 2, "outer ring and hole");
        assert_eq!(back.fields, ["fid", "Nombre", "Area", "Entero"]);
        assert_eq!(back.text[0], ["1", "uno", "96.5", "7"]);
        assert_eq!(back.text[1][3], "8");
        std::fs::remove_file(&path).ok();
        Ok(())
    }

    #[test]
    fn many_rows_and_long_values_use_several_pages_and_overflow() -> Result<()> {
        let path = std::env::temp_dir().join(format!("rl_gpkgw_big_{}.gpkg", std::process::id()));
        let n = 5000;
        let features = (0..n)
            .map(|i| Feature {
                parts: vec![vec![[i as f64, 2.0 * i as f64]]],
            })
            .collect();
        let long = "x".repeat(20_000);
        let text: Vec<Vec<String>> = (0..n)
            .map(|i| {
                vec![if i == 7 {
                    long.clone()
                } else {
                    format!("p{i}")
                }]
            })
            .collect();
        let d = VectorData {
            features,
            polygons: false,
            points: true,
            fields: vec!["nombre".into()],
            num: vec![vec![f64::NAN]; n],
            text,
            bounds: [0.0; 4],
        };
        write(
            &path,
            &[Table {
                name: "puntos",
                data: &d,
                kind: GeomKind::Point,
                epsg: None,
            }],
        )?;
        let back = crate::gpkg::read_vector(&path, None)?;
        assert_eq!(back.features.len(), n);
        assert_eq!(back.features[4999].parts[0][0], [4999.0, 9998.0]);
        assert_eq!(back.text[7][1].len(), 20_000);
        assert_eq!(back.text[4321][1], "p4321");
        std::fs::remove_file(&path).ok();
        Ok(())
    }
}
