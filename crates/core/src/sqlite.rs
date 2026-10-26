//! A read-only SQLite reader that scans whole tables, enough to read the vector layers of a
//! GeoPackage. The file format is public (https://www.sqlite.org/fileformat.html); the structure
//! of this reader follows the one in terrio (MIT, Qiusheng Wu), trimmed to what is needed here.

use rayon::prelude::*;

use crate::{Error, Result};

/// A value in a row.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Real(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(v) => Some(*v as f64),
            Value::Real(v) => Some(*v),
            Value::Text(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    /// The value as the text a table shows.
    pub fn display(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Int(v) => v.to_string(),
            Value::Real(v) => crate::paths::num(*v),
            Value::Text(s) => s.clone(),
            Value::Blob(b) => format!("[{} bytes]", b.len()),
        }
    }
}

/// A table's rows, with its column names and declared types.
pub struct Table {
    pub columns: Vec<String>,
    pub types: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    /// The index of a column (case-insensitive).
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
    }
}

/// A row of the b-tree of a table: its rowid and the bytes of its record.
type RawRow = (i64, Vec<u8>);

/// A schema entry: `(type, name, table, root page, sql)`.
pub type SchemaEntry = (String, String, String, u32, String);

/// An SQLite database file in memory.
pub struct Db {
    data: Vec<u8>,
    page_size: usize,
    usable: usize,
}

fn bad(msg: &str) -> Error {
    Error::Format(format!("SQLite: {msg}"))
}

/// Reads a varint at `i`: `(value, length)`.
fn varint(b: &[u8], i: usize) -> Option<(i64, usize)> {
    let mut v: u64 = 0;
    for k in 0..9 {
        let byte = *b.get(i + k)?;
        if k == 8 {
            v = (v << 8) | u64::from(byte);
            return Some((v as i64, 9));
        }
        v = (v << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Some((v as i64, k + 1));
        }
    }
    None
}

impl Db {
    /// Opens a database file: the whole file is read, so very large ones (over 2 GB) are refused.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if std::fs::metadata(path)?.len() > (2u64 << 30) {
            return Err(Error::Unsupported(
                "GeoPackage de más de 2 GB: conviértelo antes a shapefile o divídelo".into(),
            ));
        }
        Self::new(std::fs::read(path)?)
    }

    pub fn new(data: Vec<u8>) -> Result<Self> {
        if data.len() < 100 || !data.starts_with(b"SQLite format 3\0") {
            return Err(bad("no es una base de datos SQLite"));
        }
        let page_size = match u16::from_be_bytes([data[16], data[17]]) {
            1 => 65536,
            p => usize::from(p),
        };
        if page_size < 512 {
            return Err(bad("tamaño de página no válido"));
        }
        let usable = page_size.saturating_sub(usize::from(data[20]));
        Ok(Self {
            data,
            page_size,
            usable,
        })
    }

    fn page(&self, n: u32) -> Result<&[u8]> {
        let start = (n as usize)
            .checked_sub(1)
            .ok_or_else(|| bad("página 0"))?
            .checked_mul(self.page_size)
            .ok_or_else(|| bad("página fuera de rango"))?;
        self.data
            .get(start..start + self.page_size)
            .ok_or_else(|| bad("página fuera de rango"))
    }

    /// Every row of the b-tree rooted at `root`, as `(rowid, payload)`: the interior pages are
    /// walked first, then the leaf pages are decoded in parallel.
    fn scan(&self, root: u32) -> Result<Vec<RawRow>> {
        let mut leaves = Vec::new();
        self.leaves(root, &mut leaves, 0)?;
        let rows: Vec<Result<Vec<RawRow>>> =
            leaves.par_iter().map(|n| self.leaf_rows(*n)).collect();
        let mut out = Vec::new();
        for r in rows {
            out.extend(r?);
        }
        Ok(out)
    }

    fn leaves(&self, root: u32, out: &mut Vec<u32>, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(bad("árbol demasiado profundo"));
        }
        let page = self.page(root)?;
        let h = if root == 1 { 100 } else { 0 };
        if page[h] != 0x05 {
            out.push(root);
            return Ok(());
        }
        let cells = usize::from(u16::from_be_bytes([page[h + 3], page[h + 4]]));
        for c in 0..cells {
            let p = h + 12 + 2 * c;
            let off = usize::from(u16::from_be_bytes([
                *page.get(p).ok_or_else(|| bad("celda"))?,
                *page.get(p + 1).ok_or_else(|| bad("celda"))?,
            ]));
            let child = u32::from_be_bytes(
                page.get(off..off + 4)
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| bad("celda"))?,
            );
            self.leaves(child, out, depth + 1)?;
        }
        let right = u32::from_be_bytes(
            page.get(h + 8..h + 12)
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| bad("página"))?,
        );
        self.leaves(right, out, depth + 1)
    }

    fn leaf_rows(&self, n: u32) -> Result<Vec<RawRow>> {
        let page = self.page(n)?;
        let h = if n == 1 { 100 } else { 0 };
        if page[h] != 0x0d {
            return Err(bad("tipo de página no admitido"));
        }
        let cells = usize::from(u16::from_be_bytes([page[h + 3], page[h + 4]]));
        let mut out = Vec::with_capacity(cells);
        for c in 0..cells {
            let p = h + 8 + 2 * c;
            let off = usize::from(u16::from_be_bytes([
                *page.get(p).ok_or_else(|| bad("celda"))?,
                *page.get(p + 1).ok_or_else(|| bad("celda"))?,
            ]));
            let (size, n1) = varint(page, off).ok_or_else(|| bad("celda"))?;
            let (rowid, n2) = varint(page, off + n1).ok_or_else(|| bad("celda"))?;
            out.push((rowid, self.payload(page, off + n1 + n2, size as usize)?));
        }
        Ok(out)
    }

    /// A cell's payload, following overflow pages.
    fn payload(&self, page: &[u8], start: usize, size: usize) -> Result<Vec<u8>> {
        let u = self.usable;
        let x = u.checked_sub(35).ok_or_else(|| bad("página"))?;
        if size <= x {
            return page
                .get(start..start + size)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| bad("contenido"));
        }
        let m = ((u - 12) * 32 / 255) - 23;
        let k = m + (size - m) % (u - 4);
        let local = if k <= x { k } else { m };
        let mut out = page
            .get(start..start + local)
            .ok_or_else(|| bad("contenido"))?
            .to_vec();
        let mut next = u32::from_be_bytes(
            page.get(start + local..start + local + 4)
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| bad("desbordamiento"))?,
        );
        while out.len() < size && next != 0 {
            let p = self.page(next)?;
            next = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
            let take = (size - out.len()).min(u - 4);
            out.extend_from_slice(p.get(4..4 + take).ok_or_else(|| bad("desbordamiento"))?);
        }
        Ok(out)
    }

    /// Decodes a value of serial type `t` at `pos`: the value and its length.
    fn value(payload: &[u8], t: i64, pos: usize) -> (Value, usize) {
        let int = |len: usize| -> i64 {
            let b = payload
                .get(pos..(pos + len).min(payload.len()))
                .unwrap_or(&[]);
            let mut v: i64 = if b.first().is_some_and(|x| x & 0x80 != 0) {
                -1
            } else {
                0
            };
            for x in b {
                v = (v << 8) | i64::from(*x);
            }
            v
        };
        match t {
            0 => (Value::Null, 0),
            1..=4 => (Value::Int(int(t as usize)), t as usize),
            5 => (Value::Int(int(6)), 6),
            6 => (Value::Int(int(8)), 8),
            7 => (Value::Real(f64::from_bits(int(8) as u64)), 8),
            8 => (Value::Int(0), 0),
            9 => (Value::Int(1), 0),
            t if t >= 12 && t % 2 == 0 => {
                let l = (t as usize - 12) / 2;
                (
                    Value::Blob(payload.get(pos..pos + l).unwrap_or_default().to_vec()),
                    l,
                )
            }
            t if t >= 13 => {
                let l = (t as usize - 13) / 2;
                (
                    Value::Text(
                        String::from_utf8_lossy(payload.get(pos..pos + l).unwrap_or_default())
                            .into_owned(),
                    ),
                    l,
                )
            }
            _ => (Value::Null, 0),
        }
    }

    /// Decodes a record (a row's payload).
    fn record(payload: &[u8]) -> Vec<Value> {
        let Some((header_len, n)) = varint(payload, 0) else {
            return Vec::new();
        };
        let mut types = Vec::new();
        let mut i = n;
        while i < header_len as usize {
            let Some((t, k)) = varint(payload, i) else {
                break;
            };
            types.push(t);
            i += k;
        }
        let mut pos = header_len as usize;
        let mut out = Vec::with_capacity(types.len());
        for t in types {
            let (v, len) = Self::value(payload, t, pos);
            pos += len;
            out.push(v);
        }
        out
    }

    /// The schema: `(type, name, table, root page, sql)` of every table, index, view and trigger.
    pub fn schema(&self) -> Result<Vec<SchemaEntry>> {
        Ok(self
            .scan(1)?
            .into_iter()
            .map(|(_, payload)| {
                let r = Self::record(&payload);
                let text = |i: usize| {
                    r.get(i)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                (
                    text(0),
                    text(1),
                    text(2),
                    r.get(3).and_then(Value::as_i64).unwrap_or(0) as u32,
                    text(4),
                )
            })
            .collect())
    }

    /// Reads a whole table by name.
    pub fn table(&self, name: &str) -> Result<Table> {
        for (kind, table, _, root, sql) in self.schema()? {
            if kind != "table" || !table.eq_ignore_ascii_case(name) {
                continue;
            }
            if root == 0 {
                return Err(bad(&format!("«{name}» es una tabla virtual")));
            }
            if sql
                .to_ascii_uppercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains("WITHOUT ROWID")
            {
                return Err(bad(&format!(
                    "«{name}» es una tabla WITHOUT ROWID, que no se admite"
                )));
            }
            let defs = column_types(&sql);
            let columns: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
            let types: Vec<String> = defs.into_iter().map(|(_, t)| t).collect();
            // an INTEGER PRIMARY KEY column is the rowid, stored as NULL
            let rowid_col = integer_primary_key(&sql, &columns);
            let raw = self.scan(root)?;
            let rows = raw
                .par_iter()
                .map(|(rowid, payload)| {
                    let mut v = Self::record(payload);
                    v.resize(columns.len().max(v.len()), Value::Null);
                    if let Some(c) = rowid_col
                        && v[c] == Value::Null
                    {
                        v[c] = Value::Int(*rowid);
                    }
                    v
                })
                .collect();
            return Ok(Table {
                columns,
                types,
                rows,
            });
        }
        Err(bad(&format!("no hay una tabla «{name}»")))
    }
}

/// The column definitions of a `CREATE TABLE` statement.
fn column_defs(sql: &str) -> Vec<String> {
    let Some(open) = sql.find('(') else {
        return Vec::new();
    };
    let body = &sql[open + 1..sql.rfind(')').unwrap_or(sql.len())];
    let mut out = Vec::new();
    let mut depth = 0;
    let mut quote: Option<char> = None;
    let mut current = String::new();
    for ch in body.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(ch),
            (None, '[') => quote = Some(']'),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    out.push(current);
    out.into_iter()
        .map(|d| d.trim().to_string())
        .filter(|d| {
            let first = d
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            !d.is_empty()
                && !matches!(
                    first.as_str(),
                    "CONSTRAINT" | "PRIMARY" | "UNIQUE" | "CHECK" | "FOREIGN"
                )
        })
        .collect()
}

/// Splits a column definition into its (unquoted) name and the rest.
fn split_column(def: &str) -> (String, &str) {
    let def = def.trim();
    let close = match def.chars().next() {
        Some('"') => Some('"'),
        Some('`') => Some('`'),
        Some('[') => Some(']'),
        Some('\'') => Some('\''),
        _ => None,
    };
    match close {
        Some(c) => {
            let mut name = String::new();
            let mut chars = def.char_indices().skip(1).peekable();
            while let Some((i, ch)) = chars.next() {
                if ch == c {
                    if c != ']' && chars.peek().is_some_and(|&(_, n)| n == c) {
                        chars.next();
                        name.push(c);
                        continue;
                    }
                    return (name, &def[i + ch.len_utf8()..]);
                }
                name.push(ch);
            }
            (name, "")
        }
        None => {
            let end = def.find(char::is_whitespace).unwrap_or(def.len());
            (def[..end].to_string(), &def[end..])
        }
    }
}

/// The columns of a `CREATE TABLE` statement: names and declared types.
pub fn column_types(sql: &str) -> Vec<(String, String)> {
    const CONSTRAINTS: [&str; 9] = [
        "PRIMARY",
        "NOT",
        "NULL",
        "UNIQUE",
        "CHECK",
        "DEFAULT",
        "COLLATE",
        "REFERENCES",
        "CONSTRAINT",
    ];
    column_defs(sql)
        .iter()
        .map(|d| {
            let (name, rest) = split_column(d);
            let mut ty = String::new();
            let mut depth = 0;
            for word in rest.split_whitespace() {
                if depth == 0 && CONSTRAINTS.contains(&word.to_ascii_uppercase().as_str()) {
                    break;
                }
                depth += word.matches('(').count() as i32 - word.matches(')').count() as i32;
                if !ty.is_empty() {
                    ty.push(' ');
                }
                ty.push_str(word);
            }
            (name, ty)
        })
        .collect()
}

/// The column that aliases the rowid (`INTEGER PRIMARY KEY`).
fn integer_primary_key(sql: &str, columns: &[String]) -> Option<usize> {
    let types = column_types(sql);
    let pk_in_def = column_defs(sql).iter().zip(&types).position(|(d, (_, t))| {
        t.eq_ignore_ascii_case("INTEGER")
            && split_column(d)
                .1
                .to_ascii_uppercase()
                .contains("PRIMARY KEY")
    });
    pk_in_def.or_else(|| {
        let upper = sql.to_ascii_uppercase();
        let at = upper.rfind("PRIMARY KEY")?;
        let rest = &sql[at + 11..];
        let inner = rest.trim_start().strip_prefix('(')?.split(')').next()?;
        if inner.contains(',') {
            return None;
        }
        let (name, _) = split_column(inner);
        let i = columns.iter().position(|c| c.eq_ignore_ascii_case(&name))?;
        types[i].1.eq_ignore_ascii_case("INTEGER").then_some(i)
    })
}
