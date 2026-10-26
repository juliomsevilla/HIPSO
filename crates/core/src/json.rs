//! A small JSON reader for the files EasyLiDAR meets (GeoJSON layers): objects, arrays, numbers,
//! strings with their escapes, booleans and null. Nesting is limited, so a hostile file cannot
//! exhaust the stack.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Members in file order.
    Obj(Vec<(String, Json)>),
}

const MAX_DEPTH: usize = 64;

impl Json {
    pub fn parse(text: &str) -> Result<Json> {
        let mut p = Parser {
            s: text.as_bytes(),
            i: 0,
        };
        p.skip();
        let v = p.value(0)?;
        p.skip();
        if p.i != p.s.len() {
            return Err(p.err("hay texto después del documento"));
        }
        Ok(v)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> Error {
        Error::Format(format!("JSON: {what} (posición {})", self.i))
    }

    fn skip(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn value(&mut self, depth: usize) -> Result<Json> {
        if depth > MAX_DEPTH {
            return Err(self.err("demasiados niveles anidados"));
        }
        match self.peek() {
            Some(b'{') => {
                self.i += 1;
                let mut members = Vec::new();
                self.skip();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(members));
                }
                loop {
                    self.skip();
                    let key = self.string()?;
                    self.skip();
                    if self.peek() != Some(b':') {
                        return Err(self.err("falta ':'"));
                    }
                    self.i += 1;
                    self.skip();
                    members.push((key, self.value(depth + 1)?));
                    self.skip();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(members));
                        }
                        _ => return Err(self.err("se esperaba ',' o '}'")),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.skip();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    self.skip();
                    items.push(self.value(depth + 1)?);
                    self.skip();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return Err(self.err("se esperaba ',' o ']'")),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let from = self.i;
                while self
                    .peek()
                    .is_some_and(|c| matches!(c, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[from..self.i])
                    .ok()
                    .and_then(|t| t.parse::<f64>().ok())
                    .map(Json::Num)
                    .ok_or_else(|| self.err("número no válido"))
            }
            _ => Err(self.err("valor no válido")),
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(self.err("valor no válido"))
        }
    }

    fn hex4(&mut self) -> Result<u32> {
        let digits = self
            .s
            .get(self.i..self.i + 4)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .ok_or_else(|| self.err("escape \\u no válido"))?;
        self.i += 4;
        Ok(digits)
    }

    fn string(&mut self) -> Result<String> {
        if self.peek() != Some(b'"') {
            return Err(self.err("se esperaba una cadena"));
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or_else(|| self.err("cadena sin cerrar"))?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.err("escape incompleto"))?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi)
                                && self.s.get(self.i..self.i + 2) == Some(b"\\u")
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)
                            } else {
                                hi
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.err("escape no válido")),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("texto que no es UTF-8"))
    }
}

/// `s` as a JSON string, quotes included.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_documents_and_escapes() -> Result<()> {
        let j = Json::parse(
            r#" {"a": [1, -2.5e1, true, null], "b": {"c": "x\n\"y\" é 🌲"}, "d": []} "#,
        )?;
        let a = j.get("a").and_then(Json::as_array).unwrap_or(&[]);
        assert_eq!(a[0].as_f64(), Some(1.0));
        assert_eq!(a[1].as_f64(), Some(-25.0));
        assert_eq!(a[2], Json::Bool(true));
        assert_eq!(a[3], Json::Null);
        assert_eq!(
            j.get("b").and_then(|b| b.get("c")).and_then(Json::as_str),
            Some("x\n\"y\" é 🌲")
        );
        assert_eq!(j.get("d"), Some(&Json::Arr(Vec::new())));
        Ok(())
    }

    #[test]
    fn refuses_broken_and_too_deep_documents() {
        for bad in [
            "",
            "{",
            "[1,]",
            r#"{"a" 1}"#,
            "tru",
            "[1] 2",
            r#""\x""#,
            "1e",
        ] {
            assert!(Json::parse(bad).is_err(), "{bad}");
        }
        let deep = "[".repeat(200) + &"]".repeat(200);
        assert!(Json::parse(&deep).is_err());
    }

    #[test]
    fn quote_round_trips() -> Result<()> {
        let s = "línea\n\"comillas\" \\ \u{1}";
        assert_eq!(Json::parse(&quote(s))?, Json::Str(s.to_string()));
        Ok(())
    }
}
