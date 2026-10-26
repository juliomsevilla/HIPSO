//! The first sheet of an Excel workbook (`.xlsx`) as rows of text, enough to read a table of
//! field data: a workbook is a zip of XML files, of which only the sheet and the shared strings
//! are read. Formulas give their last stored value; dates come as the serial numbers Excel stores.

use std::io::Read;
use std::path::Path;

use crate::{Error, Result};

/// Largest XML part accepted once inflated (the files are hostile until proven otherwise).
const MAX_PART: u64 = 256 << 20;

fn bad(msg: &str) -> Error {
    Error::Format(format!("xlsx: {msg}"))
}

fn le16(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at + 2)?;
    Some(usize::from(u16::from_le_bytes([s[0], s[1]])))
}

fn le32(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at + 4)?;
    usize::try_from(u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok()
}

/// The inflated parts of the zip whose name satisfies `want`, as (name, bytes).
fn parts(zip: &[u8], want: impl Fn(&str) -> bool) -> Result<Vec<(String, Vec<u8>)>> {
    // the end-of-central-directory record: the last 22+ bytes, signature 0x06054b50
    let lo = zip.len().saturating_sub(22 + 65535);
    let eocd = (lo..zip.len().saturating_sub(21))
        .rev()
        .find(|&i| zip.get(i..i + 4) == Some(&[0x50, 0x4b, 0x05, 0x06][..]))
        .ok_or_else(|| bad("no es un libro de Excel (.xlsx)"))?;
    let count = le16(zip, eocd + 10).ok_or_else(|| bad("archivo truncado"))?;
    let mut at = le32(zip, eocd + 16).ok_or_else(|| bad("archivo truncado"))?;
    let mut out = Vec::new();
    for _ in 0..count {
        if zip.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02][..]) {
            return Err(bad("directorio del zip dañado"));
        }
        let method = le16(zip, at + 10).ok_or_else(|| bad("archivo truncado"))?;
        let csize = le32(zip, at + 20).ok_or_else(|| bad("archivo truncado"))?;
        let (nl, el, cl) = (
            le16(zip, at + 28).ok_or_else(|| bad("archivo truncado"))?,
            le16(zip, at + 30).ok_or_else(|| bad("archivo truncado"))?,
            le16(zip, at + 32).ok_or_else(|| bad("archivo truncado"))?,
        );
        let local = le32(zip, at + 42).ok_or_else(|| bad("archivo truncado"))?;
        let name = String::from_utf8_lossy(
            zip.get(at + 46..at + 46 + nl)
                .ok_or_else(|| bad("archivo truncado"))?,
        )
        .into_owned();
        at += 46 + nl + el + cl;
        if !want(&name) {
            continue;
        }
        let (lnl, lel) = (
            le16(zip, local + 26).ok_or_else(|| bad("archivo truncado"))?,
            le16(zip, local + 28).ok_or_else(|| bad("archivo truncado"))?,
        );
        let start = local + 30 + lnl + lel;
        let data = zip
            .get(start..start + csize)
            .ok_or_else(|| bad("archivo truncado"))?;
        let bytes = match method {
            0 => data.to_vec(),
            8 => {
                let mut v = Vec::new();
                flate2::read::DeflateDecoder::new(data)
                    .take(MAX_PART)
                    .read_to_end(&mut v)
                    .map_err(|_| bad("parte comprimida dañada"))?;
                v
            }
            _ => return Err(bad("compresión del zip no admitida")),
        };
        out.push((name, bytes));
    }
    Ok(out)
}

/// The text with the XML entities decoded.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(j) = rest.find(';') else { break };
        let ent = &rest[1..j];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let code = ent
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()));
                if let Some(c) = code.and_then(char::from_u32) {
                    out.push(c);
                }
            }
        }
        rest = &rest[j + 1..];
    }
    out.push_str(rest);
    out
}

/// The value of attribute `name` in the text of a start tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let i = tag.find(&key)? + key.len();
    let j = tag[i..].find('"')?;
    Some(&tag[i..i + j])
}

/// The text of every `<t>` element in `xml`, joined (a shared string may be in several runs).
fn texts(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<t") {
        let after = &rest[i + 2..];
        // `<t>` or `<t xml:space="preserve">`, and not `<tr` or `<tab`
        if !(after.starts_with('>') || after.starts_with(' ')) {
            rest = after;
            continue;
        }
        let Some(open_end) = after.find('>') else {
            break;
        };
        if after[..open_end].ends_with('/') {
            rest = &after[open_end..];
            continue;
        }
        let body = &after[open_end + 1..];
        let Some(close) = body.find("</t>") else {
            break;
        };
        out.push_str(&unescape(&body[..close]));
        rest = &body[close + 4..];
    }
    out
}

/// The zero-based column of a cell reference such as `AB12`.
fn column_of(r: &str) -> usize {
    let mut c = 0usize;
    for ch in r.bytes().take_while(u8::is_ascii_alphabetic) {
        c = c * 26 + usize::from(ch.to_ascii_uppercase() - b'A' + 1);
    }
    c.saturating_sub(1)
}

/// Reads the first sheet of the workbook; short rows are padded to the widest one.
pub fn read(path: &Path) -> Result<Vec<Vec<String>>> {
    let zip = std::fs::read(path).map_err(|e| {
        Error::Io(std::io::Error::new(
            e.kind(),
            format!(
                "{}: {e}{}",
                path.display(),
                crate::cloud_hint(&e.to_string())
            ),
        ))
    })?;
    let found = parts(&zip, |n| {
        n == "xl/sharedStrings.xml" || (n.starts_with("xl/worksheets/") && n.ends_with(".xml"))
    })?;
    let strings: Vec<String> = found
        .iter()
        .find(|(n, _)| n == "xl/sharedStrings.xml")
        .map(|(_, b)| {
            let xml = String::from_utf8_lossy(b);
            xml.split("<si")
                .skip(1)
                .filter(|s| s.starts_with('>') || s.starts_with(' '))
                .map(|si| texts(si.split("</si>").next().unwrap_or("")))
                .collect()
        })
        .unwrap_or_default();
    // sheet1 is the first in a workbook that was not reordered; take the lowest number
    let mut sheets: Vec<&(String, Vec<u8>)> = found
        .iter()
        .filter(|(n, _)| n.starts_with("xl/worksheets/"))
        .collect();
    sheets.sort_by_key(|(n, _)| {
        let digits: String = n.chars().filter(char::is_ascii_digit).collect();
        (digits.parse::<u32>().unwrap_or(u32::MAX), n.clone())
    });
    let Some((_, sheet)) = sheets.first() else {
        return Err(bad("el libro no tiene hojas"));
    };
    let xml = String::from_utf8_lossy(sheet);
    let mut rows: Vec<Vec<String>> = Vec::new();
    for row in xml.split("<row").skip(1) {
        let row = row.split("</row>").next().unwrap_or("");
        let mut cells: Vec<String> = Vec::new();
        for cell in row.split("<c ").skip(1) {
            let head_end = cell.find('>').unwrap_or(cell.len());
            let head = format!(" {}", &cell[..head_end]);
            let empty = head.ends_with('/');
            let col = attr(&head, "r").map_or(cells.len(), column_of);
            let body = if empty { "" } else { &cell[head_end..] };
            let value = match attr(&head, "t") {
                Some("s") => body
                    .split("<v>")
                    .nth(1)
                    .and_then(|v| v.split("</v>").next())
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .and_then(|i| strings.get(i).cloned())
                    .unwrap_or_default(),
                Some("inlineStr") => texts(body),
                _ => body
                    .split("<v>")
                    .nth(1)
                    .and_then(|v| v.split("</v>").next())
                    .map(|v| unescape(v.trim()))
                    .unwrap_or_default(),
            };
            if cells.len() <= col {
                cells.resize(col + 1, String::new());
            }
            cells[col] = value;
        }
        rows.push(cells);
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for r in &mut rows {
        r.resize(width, String::new());
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_and_columns() {
        assert_eq!(unescape("a &amp; b &lt;c&gt; &#65;&#x42;"), "a & b <c> AB");
        assert_eq!(column_of("A1"), 0);
        assert_eq!(column_of("AB12"), 27);
    }

    #[test]
    fn runs_of_a_shared_string_join() {
        assert_eq!(
            texts("<r><t>Par</t></r><r><t xml:space=\"preserve\">cela </t></r>"),
            "Parcela "
        );
    }

    #[test]
    fn rejects_other_files() {
        let p = std::env::temp_dir().join("easylidar_not_xlsx.xlsx");
        assert!(std::fs::write(&p, b"hello").is_ok());
        assert!(read(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }
}
