//! The field calculator: fills a field of a vector layer from an expression over the other
//! fields and the geometry.
//!
//! Expressions: numbers, `'text'`, `"field"` (or the bare name of a field), `$area`,
//! `$perimeter`, `$length`, `$x`, `$y`, `$id`, `$npoints`; `+ - * / % ^`, `||` to join texts,
//! comparisons (`= != < <= > >=`), `and`, `or`, parentheses and the functions `round`, `floor`,
//! `ceil`, `abs`, `sqrt`, `ln`, `log10`, `pow`, `min`, `max`, `upper`, `lower`, `trim`, `len`,
//! `concat`, `left`, `right`, `substr`, `replace`, `if` and `coalesce`. Areas and lengths are in
//! the units of the layer (square metres and metres in a projected system).

use std::collections::HashSet;

use geo::{Area, Centroid};

use crate::geomops::multipolygon_of;
use crate::vecops::{geometry_of, nice};
use crate::view::{GeomKind, VectorData};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Cat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Debug)]
enum E {
    Num(f64),
    Str(String),
    Field(String),
    Var(String),
    Neg(Box<E>),
    Bin(Op, Box<E>, Box<E>),
    Call(String, Vec<E>),
}

fn bad(msg: impl Into<String>) -> Error {
    Error::Param(msg.into())
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

impl Parser {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, text: &str) -> bool {
        self.ws();
        let t: Vec<char> = text.chars().collect();
        if self.s.get(self.i..self.i + t.len()) == Some(&t[..]) {
            self.i += t.len();
            true
        } else {
            false
        }
    }

    /// A word operator (`and`, `or`) that is not the start of a longer name.
    fn eat_word(&mut self, w: &str) -> bool {
        self.ws();
        let t: Vec<char> = w.chars().collect();
        let end = self.i + t.len();
        let same = self.s.get(self.i..end).is_some_and(|x| x.iter().zip(&t).all(|(a, b)| a.eq_ignore_ascii_case(b)));
        if same && !self.s.get(end).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            self.i = end;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<E> {
        let mut l = self.and()?;
        while self.eat_word("or") {
            l = E::Bin(Op::Or, Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<E> {
        let mut l = self.cmp()?;
        while self.eat_word("and") {
            l = E::Bin(Op::And, Box::new(l), Box::new(self.cmp()?));
        }
        Ok(l)
    }

    fn cmp(&mut self) -> Result<E> {
        let l = self.cat()?;
        for (t, op) in [("<=", Op::Le), (">=", Op::Ge), ("!=", Op::Ne), ("<>", Op::Ne), ("=", Op::Eq), ("<", Op::Lt), (">", Op::Gt)] {
            if self.eat(t) {
                return Ok(E::Bin(op, Box::new(l), Box::new(self.cat()?)));
            }
        }
        Ok(l)
    }

    fn cat(&mut self) -> Result<E> {
        let mut l = self.add()?;
        while self.eat("||") {
            l = E::Bin(Op::Cat, Box::new(l), Box::new(self.add()?));
        }
        Ok(l)
    }

    fn add(&mut self) -> Result<E> {
        let mut l = self.mul()?;
        loop {
            let op = if self.eat("+") {
                Op::Add
            } else if self.peek() == Some('-') {
                self.i += 1;
                Op::Sub
            } else {
                return Ok(l);
            };
            l = E::Bin(op, Box::new(l), Box::new(self.mul()?));
        }
    }

    fn mul(&mut self) -> Result<E> {
        let mut l = self.pow()?;
        loop {
            let op = if self.eat("*") {
                Op::Mul
            } else if self.eat("/") {
                Op::Div
            } else if self.eat("%") {
                Op::Rem
            } else {
                return Ok(l);
            };
            l = E::Bin(op, Box::new(l), Box::new(self.pow()?));
        }
    }

    fn pow(&mut self) -> Result<E> {
        let l = self.unary()?;
        if self.eat("^") {
            return Ok(E::Bin(Op::Pow, Box::new(l), Box::new(self.pow()?)));
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<E> {
        if self.peek() == Some('-') {
            self.i += 1;
            return Ok(E::Neg(Box::new(self.unary()?)));
        }
        if self.peek() == Some('+') {
            self.i += 1;
            return self.unary();
        }
        self.primary()
    }

    fn quoted(&mut self, q: char) -> Result<String> {
        // the opening quote is already read; a doubled quote is one inside the text
        let mut out = String::new();
        loop {
            match self.s.get(self.i) {
                None => return Err(bad("falta cerrar unas comillas")),
                Some(&c) if c == q => {
                    self.i += 1;
                    if self.s.get(self.i) == Some(&q) {
                        out.push(q);
                        self.i += 1;
                    } else {
                        return Ok(out);
                    }
                }
                Some(&c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn word(&mut self) -> String {
        let start = self.i;
        while self.s.get(self.i).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect()
    }

    fn primary(&mut self) -> Result<E> {
        let Some(c) = self.peek() else {
            return Err(bad("la expresión termina antes de tiempo"));
        };
        if c == '(' {
            self.i += 1;
            let e = self.expr()?;
            return if self.eat(")") { Ok(e) } else { Err(bad("falta un paréntesis de cierre")) };
        }
        if c == '\'' {
            self.i += 1;
            return Ok(E::Str(self.quoted('\'')?));
        }
        if c == '"' {
            self.i += 1;
            return Ok(E::Field(self.quoted('"')?));
        }
        if c == '$' {
            self.i += 1;
            let w = self.word().to_lowercase();
            return if w.is_empty() { Err(bad("«$» sin nombre")) } else { Ok(E::Var(w)) };
        }
        if c.is_ascii_digit() || c == '.' {
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
                self.i += 1;
            }
            if matches!(self.s.get(self.i), Some('e' | 'E'))
                && self.s.get(self.i + 1).is_some_and(|c| c.is_ascii_digit() || *c == '-' || *c == '+')
            {
                self.i += 2;
                while self.s.get(self.i).is_some_and(char::is_ascii_digit) {
                    self.i += 1;
                }
            }
            let t: String = self.s[start..self.i].iter().collect();
            return t.parse().map(E::Num).map_err(|_| bad(format!("«{t}» no es un número")));
        }
        if c.is_alphabetic() || c == '_' {
            let w = self.word();
            if self.peek() == Some('(') {
                self.i += 1;
                let mut args = Vec::new();
                if !self.eat(")") {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(",") {
                            continue;
                        }
                        if self.eat(")") {
                            break;
                        }
                        return Err(bad(format!("en la función {w}: faltan «,» o «)»")));
                    }
                }
                return Ok(E::Call(w.to_lowercase(), args));
            }
            if w.eq_ignore_ascii_case("pi") {
                return Ok(E::Num(std::f64::consts::PI));
            }
            return Ok(E::Field(w));
        }
        Err(bad(format!("carácter inesperado «{c}»")))
    }
}

fn parse(expr: &str) -> Result<E> {
    let mut p = Parser { s: expr.chars().collect(), i: 0 };
    let e = p.expr()?;
    if let Some(c) = p.peek() {
        return Err(bad(format!("sobra «{c}» en la expresión")));
    }
    Ok(e)
}

#[derive(Clone, Debug)]
enum V {
    N(f64),
    T(String),
}

impl V {
    fn num(&self) -> f64 {
        match self {
            V::N(x) => *x,
            V::T(s) => s.trim().parse().unwrap_or(f64::NAN),
        }
    }

    fn text(&self) -> String {
        match self {
            V::N(x) => nice(*x),
            V::T(s) => s.clone(),
        }
    }

    fn truth(&self) -> bool {
        let n = self.num();
        if n.is_nan() { !self.text().is_empty() } else { n != 0.0 }
    }
}

fn flag(b: bool) -> V {
    V::N(if b { 1.0 } else { 0.0 })
}

struct Ctx<'a> {
    d: &'a VectorData,
    kind: GeomKind,
    row: usize,
}

fn ring_length(r: &[[f64; 2]]) -> f64 {
    r.windows(2).map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1])).sum()
}

impl Ctx<'_> {
    fn parts(&self) -> &[Vec<[f64; 2]>] {
        &self.d.features[self.row].parts
    }

    fn var(&self, name: &str) -> Result<V> {
        let need = |k: GeomKind, what: &str| {
            if self.kind == k { Ok(()) } else { Err(bad(format!("{what} solo existe en capas de {}", if k == GeomKind::Polygon { "polígonos" } else { "líneas" }))) }
        };
        Ok(match name {
            "id" => V::N((self.row + 1) as f64),
            "npoints" => V::N(self.parts().iter().map(Vec::len).sum::<usize>() as f64),
            "area" => {
                need(GeomKind::Polygon, "$area")?;
                V::N(multipolygon_of(self.parts()).unsigned_area())
            }
            "perimeter" => {
                need(GeomKind::Polygon, "$perimeter")?;
                V::N(self.parts().iter().map(|r| ring_length(r)).sum())
            }
            "length" => {
                if self.kind == GeomKind::Point {
                    return Err(bad("$length no existe en capas de puntos"));
                }
                V::N(self.parts().iter().map(|r| ring_length(r)).sum())
            }
            "x" | "y" => {
                let p = if self.kind == GeomKind::Point {
                    self.parts().iter().flatten().next().map(|p| (p[0], p[1]))
                } else {
                    geometry_of(self.parts(), self.kind).and_then(|g| g.centroid()).map(|p| (p.x(), p.y()))
                };
                match p {
                    Some((x, y)) => V::N(if name == "x" { x } else { y }),
                    None => V::T(String::new()),
                }
            }
            other => return Err(bad(format!("no existe la variable ${other}"))),
        })
    }

    fn field(&self, name: &str) -> Result<V> {
        let c = self
            .d
            .fields
            .iter()
            .position(|f| f.trim().eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| bad(format!("no hay un campo «{name}»")))?;
        Ok(V::T(self.d.text.get(self.row).and_then(|r| r.get(c)).cloned().unwrap_or_default()))
    }

    fn eval(&self, e: &E) -> Result<V> {
        Ok(match e {
            E::Num(x) => V::N(*x),
            E::Str(s) => V::T(s.clone()),
            E::Field(n) => self.field(n)?,
            E::Var(n) => self.var(n)?,
            E::Neg(a) => V::N(-self.eval(a)?.num()),
            E::Bin(op, a, b) => {
                // `and` and `or` do not look at the right side when the left decides
                let l = self.eval(a)?;
                match op {
                    Op::And if !l.truth() => return Ok(flag(false)),
                    Op::Or if l.truth() => return Ok(flag(true)),
                    _ => {}
                }
                let r = self.eval(b)?;
                match op {
                    Op::Add => V::N(l.num() + r.num()),
                    Op::Sub => V::N(l.num() - r.num()),
                    Op::Mul => V::N(l.num() * r.num()),
                    Op::Div => V::N(l.num() / r.num()),
                    Op::Rem => V::N(l.num() % r.num()),
                    Op::Pow => V::N(l.num().powf(r.num())),
                    Op::Cat => V::T(format!("{}{}", l.text(), r.text())),
                    Op::And | Op::Or => flag(r.truth()),
                    Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                        let (x, y) = (l.num(), r.num());
                        let ord = if x.is_nan() || y.is_nan() { l.text().cmp(&r.text()) } else { x.total_cmp(&y) };
                        flag(match op {
                            Op::Eq => ord.is_eq(),
                            Op::Ne => ord.is_ne(),
                            Op::Lt => ord.is_lt(),
                            Op::Le => ord.is_le(),
                            Op::Gt => ord.is_gt(),
                            _ => ord.is_ge(),
                        })
                    }
                }
            }
            E::Call(f, args) => self.call(f, args)?,
        })
    }

    fn call(&self, f: &str, args: &[E]) -> Result<V> {
        let want = |n: usize| {
            if args.len() == n { Ok(()) } else { Err(bad(format!("{f} lleva {n} argumento(s) y se le dan {}", args.len()))) }
        };
        let a = |i: usize| self.eval(&args[i]);
        let nums = || -> Result<Vec<f64>> { args.iter().map(|e| self.eval(e).map(|v| v.num())).collect() };
        Ok(match f {
            "round" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(bad("round lleva 1 o 2 argumentos"));
                }
                let k = if args.len() == 2 { a(1)?.num() } else { 0.0 };
                let m = 10f64.powi(k as i32);
                V::N((a(0)?.num() * m).round() / m)
            }
            "floor" => {
                want(1)?;
                V::N(a(0)?.num().floor())
            }
            "ceil" => {
                want(1)?;
                V::N(a(0)?.num().ceil())
            }
            "abs" => {
                want(1)?;
                V::N(a(0)?.num().abs())
            }
            "sqrt" => {
                want(1)?;
                V::N(a(0)?.num().sqrt())
            }
            "ln" => {
                want(1)?;
                V::N(a(0)?.num().ln())
            }
            "log10" => {
                want(1)?;
                V::N(a(0)?.num().log10())
            }
            "pow" => {
                want(2)?;
                V::N(a(0)?.num().powf(a(1)?.num()))
            }
            "min" | "max" => {
                let v = nums()?;
                if v.is_empty() {
                    return Err(bad(format!("{f} necesita algún argumento")));
                }
                let pick = v.iter().copied().filter(|x| !x.is_nan());
                V::N(if f == "min" { pick.fold(f64::INFINITY, f64::min) } else { pick.fold(f64::NEG_INFINITY, f64::max) })
            }
            "upper" => {
                want(1)?;
                V::T(a(0)?.text().to_uppercase())
            }
            "lower" => {
                want(1)?;
                V::T(a(0)?.text().to_lowercase())
            }
            "trim" => {
                want(1)?;
                V::T(a(0)?.text().trim().to_string())
            }
            "len" => {
                want(1)?;
                V::N(a(0)?.text().chars().count() as f64)
            }
            "concat" => V::T(args.iter().map(|e| self.eval(e).map(|v| v.text())).collect::<Result<Vec<_>>>()?.concat()),
            "left" | "right" => {
                want(2)?;
                let (t, n) = (a(0)?.text(), a(1)?.num().max(0.0) as usize);
                let c: Vec<char> = t.chars().collect();
                let n = n.min(c.len());
                V::T(if f == "left" { c[..n].iter().collect() } else { c[c.len() - n..].iter().collect() })
            }
            "substr" => {
                want(3)?;
                let (t, from, n) = (a(0)?.text(), a(1)?.num().max(1.0) as usize, a(2)?.num().max(0.0) as usize);
                V::T(t.chars().skip(from - 1).take(n).collect())
            }
            "replace" => {
                want(3)?;
                let from = a(1)?.text();
                V::T(if from.is_empty() { a(0)?.text() } else { a(0)?.text().replace(&from, &a(2)?.text()) })
            }
            "if" => {
                want(3)?;
                if a(0)?.truth() { a(1)? } else { a(2)? }
            }
            "coalesce" => {
                for e in args {
                    let v = self.eval(e)?;
                    if !v.text().trim().is_empty() {
                        return Ok(v);
                    }
                }
                V::T(String::new())
            }
            other => return Err(bad(format!("no existe la función «{other}»"))),
        })
    }
}

/// The values of `expr` for the rows asked for (all, or `only`), as text; checks the expression
/// without changing anything.
pub fn evaluate(d: &VectorData, kind: GeomKind, expr: &str, only: Option<&HashSet<usize>>) -> Result<Vec<(usize, String)>> {
    let e = parse(expr)?;
    let mut out = Vec::new();
    for row in 0..d.features.len() {
        if only.is_some_and(|s| !s.contains(&row)) {
            continue;
        }
        let v = Ctx { d, kind, row }.eval(&e)?;
        out.push((row, match v {
            V::N(x) if !x.is_finite() => String::new(),
            v => v.text(),
        }));
    }
    Ok(out)
}

/// The rows for which `expr` holds: a comparison that is true, a number other than zero, a text
/// that is not empty.
pub fn select(d: &VectorData, kind: GeomKind, expr: &str) -> Result<Vec<usize>> {
    let e = parse(expr)?;
    let mut out = Vec::new();
    for row in 0..d.features.len() {
        if (Ctx { d, kind, row }).eval(&e)?.truth() {
            out.push(row);
        }
    }
    Ok(out)
}

/// Writes the values of `expr` into `field` (made if the layer has none of that name) for all
/// the rows or only those in `only`. Nothing changes if the expression fails on any row.
/// Returns how many rows were set.
pub fn apply(d: &mut VectorData, kind: GeomKind, field: &str, expr: &str, only: Option<&HashSet<usize>>) -> Result<usize> {
    let name = field.trim();
    if name.is_empty() {
        return Err(bad("falta el nombre del campo"));
    }
    let values = evaluate(d, kind, expr, only)?;
    let col = match d.fields.iter().position(|f| f.trim().eq_ignore_ascii_case(name)) {
        Some(c) => c,
        None => {
            d.fields.push(name.to_string());
            for (t, n) in d.text.iter_mut().zip(d.num.iter_mut()) {
                t.push(String::new());
                n.push(f64::NAN);
            }
            d.fields.len() - 1
        }
    };
    let n = values.len();
    for (row, v) in values {
        d.num[row][col] = v.trim().parse().unwrap_or(f64::NAN);
        d.text[row][col] = v;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Feature;

    fn layer() -> VectorData {
        let sq = |s: f64| vec![[0.0, 0.0], [0.0, s], [s, s], [s, 0.0], [0.0, 0.0]];
        VectorData {
            features: vec![Feature { parts: vec![sq(10.0)] }, Feature { parts: vec![sq(20.0)] }],
            polygons: true,
            fields: vec!["especie".into(), "n".into()],
            text: vec![vec!["Pino".into(), "3".into()], vec!["Haya".into(), "".into()]],
            num: vec![vec![f64::NAN, 3.0], vec![f64::NAN, f64::NAN]],
            bounds: [0.0, 0.0, 20.0, 20.0],
            ..VectorData::default()
        }
    }

    fn one(expr: &str) -> Result<Vec<String>> {
        Ok(evaluate(&layer(), GeomKind::Polygon, expr, None)?.into_iter().map(|r| r.1).collect())
    }

    #[test]
    fn geometry_and_arithmetic() -> Result<()> {
        assert_eq!(one("$area / 10000")?, ["0.01", "0.04"]);
        assert_eq!(one("round($area / 3, 1)")?, ["33.3", "133.3"]);
        assert_eq!(one("$perimeter")?, ["40", "80"]);
        assert_eq!(one("2 + 3 * 4 ^ 2 - -1")?, ["51", "51"]);
        assert_eq!(one("$id")?, ["1", "2"]);
        assert_eq!(one("$x + $y")?, ["10", "20"]);
        Ok(())
    }

    #[test]
    fn fields_texts_and_conditions() -> Result<()> {
        assert_eq!(one("\"n\" * 2")?, ["6", ""]);
        assert_eq!(one("especie || '-' || $id")?, ["Pino-1", "Haya-2"]);
        assert_eq!(one("if(especie = 'Pino', 'conífera', 'frondosa')")?, ["conífera", "frondosa"]);
        assert_eq!(one("upper(left(especie, 2))")?, ["PI", "HA"]);
        assert_eq!(one("coalesce(n, 0)")?, ["3", "0"]);
        assert_eq!(one("$area > 150 or especie = 'Pino'")?, ["1", "1"]);
        Ok(())
    }

    #[test]
    fn rows_are_selected_by_an_expression() -> Result<()> {
        let d = layer();
        assert_eq!(select(&d, GeomKind::Polygon, "$area > 150")?, [1]);
        assert_eq!(select(&d, GeomKind::Polygon, "especie = 'Pino' or \"n\" > 5")?, [0]);
        assert_eq!(select(&d, GeomKind::Polygon, "n")?, [0]);
        assert!(select(&d, GeomKind::Polygon, "$area >").is_err());
        Ok(())
    }

    #[test]
    fn mistakes_are_reported_and_change_nothing() {
        let mut d = layer();
        assert!(apply(&mut d, GeomKind::Polygon, "z", "$area +", None).is_err());
        assert!(apply(&mut d, GeomKind::Polygon, "z", "falta + 1", None).is_err());
        assert!(apply(&mut d, GeomKind::Polygon, "z", "nada(1)", None).is_err());
        assert!(apply(&mut d, GeomKind::Point, "z", "$area", None).is_err());
        assert_eq!(d.fields.len(), 2);
    }

    #[test]
    fn a_new_field_is_made_and_only_the_selected_rows_are_set() -> Result<()> {
        let mut d = layer();
        let only: HashSet<usize> = [1].into_iter().collect();
        assert_eq!(apply(&mut d, GeomKind::Polygon, "ha", "$area / 10000", Some(&only))?, 1);
        assert_eq!(d.fields, ["especie", "n", "ha"]);
        assert_eq!(d.text[0][2], "");
        assert_eq!(d.text[1][2], "0.04");
        assert_eq!(d.num[1][2], 0.04);
        // an existing field is set, whatever the case of its name
        apply(&mut d, GeomKind::Polygon, "ESPECIE", "'x'", None)?;
        assert_eq!((d.fields.len(), d.text[0][0].as_str()), (3, "x"));
        Ok(())
    }
}
