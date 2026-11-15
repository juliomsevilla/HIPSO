//! La calculadora de campos: rellena un campo de una capa vectorial a partir de una expresión sobre
//! los demás campos y la geometría.
//!
//! Expresiones: números, `'texto'`, `"campo"` (o el nombre a secas de un campo), `$area`,
//! `$perimeter`, `$length`, `$x`, `$y`, `$id`, `$npoints`; `+ - * / % ^`, `||` para unir textos,
//! comparaciones (`= != < <= > >=`), `and`, `or`, `not`, paréntesis y las funciones `round`,
//! `floor`, `ceil`, `abs`, `sqrt`, `ln`, `log10`, `pow`, `min`, `max`, `upper`, `lower`, `trim`,
//! `len`, `concat`, `left`, `right`, `substr`, `replace`, `if` y `coalesce`; para las fechas
//! `date('2025-03-31')`, `today()`, `year`, `month` y `day` (una fecha más o menos días es una
//! fecha; dos fechas restadas dan días) y los valores `true` y `false`. Cada campo se lee según su
//! tipo: números como números, fechas como fechas, sí/no como sí/no y texto (y los hipervínculos)
//! como texto. Las áreas y longitudes están en las unidades de la capa (metros cuadrados y metros
//! en un sistema proyectado).

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

    /// Un operador de palabra (`and`, `or`) que no es el comienzo de un nombre más largo.
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
        let mut l = self.not()?;
        while self.eat_word("and") {
            l = E::Bin(Op::And, Box::new(l), Box::new(self.not()?));
        }
        Ok(l)
    }

    /// `not` delante de una comparación.
    fn not(&mut self) -> Result<E> {
        if self.eat_word("not") {
            return Ok(E::Call("not".into(), vec![self.not()?]));
        }
        self.cmp()
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
        // la comilla de apertura ya está leída; una comilla duplicada es una dentro del texto
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
    /// Texto sin tipo declarado (un literal, el resultado de una función): se lee como número
    /// cuando lo es.
    T(String),
    /// El valor de un campo de texto o de hipervínculo: se compara como texto, nunca como número.
    S(String),
    /// Un valor sí/no.
    B(bool),
    /// Una fecha, como días desde 1970-01-01.
    D(f64),
}

impl V {
    fn num(&self) -> f64 {
        match self {
            V::N(x) | V::D(x) => *x,
            V::B(b) => f64::from(u8::from(*b)),
            V::T(s) | V::S(s) => s.trim().parse().unwrap_or(f64::NAN),
        }
    }

    fn text(&self) -> String {
        match self {
            V::N(x) => nice(*x),
            V::T(s) | V::S(s) => s.clone(),
            V::B(b) => if *b { "Sí" } else { "No" }.to_string(),
            V::D(d) => crate::fieldtype::format_date(*d),
        }
    }

    fn truth(&self) -> bool {
        match self {
            V::B(b) => *b,
            V::D(_) => true,
            V::S(s) => !s.is_empty(),
            _ => {
                let n = self.num();
                if n.is_nan() { !self.text().is_empty() } else { n != 0.0 }
            }
        }
    }

    /// Como fecha, cuando lo es o es un texto que se lee como tal.
    fn date(&self) -> Option<f64> {
        match self {
            V::D(d) => Some(*d),
            V::T(s) | V::S(s) => crate::fieldtype::parse_date(s),
            V::N(x) if x.is_finite() => Some(*x),
            _ => None,
        }
    }

    /// Como sí/no, cuando lo es o es un texto que se lee como tal.
    fn yes_no(&self) -> Option<bool> {
        match self {
            V::B(b) => Some(*b),
            V::N(x) if x.is_finite() => Some(*x != 0.0),
            V::T(s) | V::S(s) => crate::fieldtype::FieldType::Bool.parse(s).ok().filter(|v| v.1.is_finite()).map(|v| v.1 != 0.0),
            V::D(_) | V::N(_) => None,
        }
    }
}

/// Cómo se ordenan dos valores, según lo que son: fechas por día (un texto que se lee como fecha
/// cuenta como tal), sí/no como falso < verdadero, el texto de un campo como texto, el resto como
/// números cuando ambos lo son.
fn compare(l: &V, r: &V) -> std::cmp::Ordering {
    if (matches!(l, V::D(_)) || matches!(r, V::D(_)))
        && let (Some(a), Some(b)) = (l.date(), r.date())
    {
        return a.total_cmp(&b);
    }
    if (matches!(l, V::B(_)) || matches!(r, V::B(_)))
        && let (Some(a), Some(b)) = (l.yes_no(), r.yes_no())
    {
        return a.cmp(&b);
    }
    if matches!(l, V::S(_)) || matches!(r, V::S(_)) {
        return l.text().cmp(&r.text());
    }
    let (x, y) = (l.num(), r.num());
    if x.is_nan() || y.is_nan() { l.text().cmp(&r.text()) } else { x.total_cmp(&y) }
}

fn flag(b: bool) -> V {
    V::B(b)
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
        use crate::fieldtype::FieldType;
        let Some(c) = self
            .d
            .fields
            .iter()
            .position(|f| f.trim().eq_ignore_ascii_case(name.trim()))
        else {
            // `true` y `false` son valores cuando ningún campo tiene ese nombre
            return match name.trim().to_lowercase().as_str() {
                "true" | "verdadero" => Ok(V::B(true)),
                "false" | "falso" => Ok(V::B(false)),
                _ => Err(bad(format!("no hay un campo «{name}»"))),
            };
        };
        let t = self.d.text.get(self.row).and_then(|r| r.get(c)).cloned().unwrap_or_default();
        // un valor vacío está vacío sea cual sea el tipo: no es cero, ni falso, ni una fecha
        if t.trim().is_empty() {
            return Ok(V::T(String::new()));
        }
        Ok(match self.d.field_type(c) {
            FieldType::Integer | FieldType::Real => match t.trim().parse::<f64>() {
                Ok(x) => V::N(x),
                Err(_) => V::T(t),
            },
            FieldType::Bool => FieldType::Bool.parse(&t).map_or(V::T(t), |(_, n)| V::B(n != 0.0)),
            FieldType::Date => crate::fieldtype::parse_date(&t).map_or(V::T(t), V::D),
            FieldType::Text | FieldType::Link => V::S(t),
        })
    }

    fn eval(&self, e: &E) -> Result<V> {
        Ok(match e {
            E::Num(x) => V::N(*x),
            E::Str(s) => V::T(s.clone()),
            E::Field(n) => self.field(n)?,
            E::Var(n) => self.var(n)?,
            E::Neg(a) => V::N(-self.eval(a)?.num()),
            E::Bin(op, a, b) => {
                // `and` y `or` no miran el lado derecho cuando el izquierdo decide
                let l = self.eval(a)?;
                match op {
                    Op::And if !l.truth() => return Ok(flag(false)),
                    Op::Or if l.truth() => return Ok(flag(true)),
                    _ => {}
                }
                let r = self.eval(b)?;
                match op {
                    // una fecha más o menos días es una fecha; la diferencia de dos son días
                    Op::Add if matches!(l, V::D(_)) != matches!(r, V::D(_)) => V::D(l.num() + r.num()),
                    Op::Sub if matches!(l, V::D(_)) && !matches!(r, V::D(_)) => V::D(l.num() - r.num()),
                    Op::Add => V::N(l.num() + r.num()),
                    Op::Sub => V::N(l.num() - r.num()),
                    Op::Mul => V::N(l.num() * r.num()),
                    Op::Div => V::N(l.num() / r.num()),
                    Op::Rem => V::N(l.num() % r.num()),
                    Op::Pow => V::N(l.num().powf(r.num())),
                    Op::Cat => V::T(format!("{}{}", l.text(), r.text())),
                    Op::And | Op::Or => flag(r.truth()),
                    Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                        let ord = compare(&l, &r);
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
            "date" | "fecha" => {
                want(1)?;
                let v = a(0)?;
                match v.date() {
                    Some(d) if !matches!(v, V::N(_)) => V::D(d),
                    _ if v.text().trim().is_empty() => V::T(String::new()),
                    _ => return Err(bad(format!("«{}» no es una fecha (por ejemplo 2025-03-31)", v.text()))),
                }
            }
            "today" | "hoy" => {
                want(0)?;
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                V::D((secs / 86_400) as f64)
            }
            "year" | "month" | "day" | "año" | "mes" | "dia" | "día" => {
                want(1)?;
                match a(0)?.date() {
                    Some(d) => {
                        let (y, m, dd) = crate::fieldtype::civil_from_days(d as i64);
                        V::N(match f {
                            "year" | "año" => y,
                            "month" | "mes" => m,
                            _ => dd,
                        } as f64)
                    }
                    None => V::T(String::new()),
                }
            }
            "min" | "max" => {
                if args.iter().any(|e| matches!(self.eval(e), Ok(V::D(_)))) {
                    let d: Vec<f64> = args.iter().filter_map(|e| self.eval(e).ok().and_then(|v| v.date())).collect();
                    let pick = d.iter().copied();
                    return Ok(V::D(if f == "min" { pick.fold(f64::INFINITY, f64::min) } else { pick.fold(f64::NEG_INFINITY, f64::max) }));
                }
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
            "not" => {
                want(1)?;
                flag(!a(0)?.truth())
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

fn eval_rows(d: &VectorData, kind: GeomKind, expr: &str, only: Option<&HashSet<usize>>) -> Result<Vec<(usize, V)>> {
    let e = parse(expr)?;
    let mut out = Vec::new();
    for row in 0..d.features.len() {
        if only.is_some_and(|s| !s.contains(&row)) {
            continue;
        }
        out.push((row, Ctx { d, kind, row }.eval(&e)?));
    }
    Ok(out)
}

/// Los valores de `expr` para las filas pedidas (todas, o `only`), como texto; comprueba la
/// expresión sin cambiar nada.
pub fn evaluate(d: &VectorData, kind: GeomKind, expr: &str, only: Option<&HashSet<usize>>) -> Result<Vec<(usize, String)>> {
    Ok(eval_rows(d, kind, expr, only)?
        .into_iter()
        .map(|(row, v)| {
            (row, match v {
                V::N(x) if !x.is_finite() => String::new(),
                v => v.text(),
            })
        })
        .collect())
}

/// Las filas para las que se cumple `expr`: una comparación verdadera, un número distinto de cero,
/// un texto que no está vacío.
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

/// El tipo que recibe un campo hecho por una expresión: el de los valores que da.
fn type_of_values(values: &[(usize, V)]) -> crate::fieldtype::FieldType {
    use crate::fieldtype::FieldType;
    let some: Vec<&V> = values.iter().map(|v| &v.1).filter(|v| !v.text().is_empty()).collect();
    if some.is_empty() {
        FieldType::Text
    } else if some.iter().all(|v| matches!(v, V::D(_))) {
        FieldType::Date
    } else if some.iter().all(|v| matches!(v, V::B(_))) {
        FieldType::Bool
    } else if some.iter().all(|v| matches!(v, V::N(x) if x.is_finite())) {
        FieldType::infer(some.iter().map(|v| v.text()).collect::<Vec<_>>().iter().map(String::as_str))
    } else {
        FieldType::Text
    }
}

/// Escribe los valores de `expr` en `field` para todas las filas o solo las de `only`. Un campo que
/// existe conserva su tipo y los valores se comprueban contra él (un texto en un campo numérico es
/// un error); uno que no existe se hace con el tipo de los valores. No cambia nada si la expresión
/// falla en alguna fila o un valor no cabe. Devuelve cuántas filas se fijaron.
pub fn apply(d: &mut VectorData, kind: GeomKind, field: &str, expr: &str, only: Option<&HashSet<usize>>) -> Result<usize> {
    let name = field.trim();
    if name.is_empty() {
        return Err(bad("falta el nombre del campo"));
    }
    let values = eval_rows(d, kind, expr, only)?;
    let text = |v: &V| match v {
        V::N(x) if !x.is_finite() => String::new(),
        v => v.text(),
    };
    let existing = d.fields.iter().position(|f| f.trim().eq_ignore_ascii_case(name));
    // comprobar cada valor antes de tocar la capa
    if let Some(c) = existing {
        let ty = d.field_type(c);
        for (row, v) in &values {
            ty.parse(&text(v)).map_err(|m| bad(format!("fila {}: {m} (el campo «{}» es de tipo {})", row + 1, d.fields[c], ty.name())))?;
        }
    }
    let ty = type_of_values(&values);
    let col = match existing {
        Some(c) => c,
        None => d.add_field(name, ty),
    };
    let n = values.len();
    for (row, v) in &values {
        if d.set_value(*row, col, &text(v)).is_err() {
            return Err(bad(format!("fila {}: el valor no cabe en el campo", row + 1)));
        }
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
        assert_eq!(one("$area > 150 or especie = 'Pino'")?, ["Sí", "Sí"]);
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
        // un campo que existe se fija, sea cual sea la capitalización de su nombre
        apply(&mut d, GeomKind::Polygon, "ESPECIE", "'x'", None)?;
        assert_eq!((d.fields.len(), d.text[0][0].as_str()), (3, "x"));
        Ok(())
    }

    /// Una capa con un campo de cada tipo.
    fn typed() -> VectorData {
        use crate::fieldtype::FieldType::*;
        let mut d = layer();
        d.types = vec![Text, Integer];
        d.add_field("fecha", Date);
        d.add_field("activo", Bool);
        d.add_field("codigo", Text);
        for (row, (f, a, c)) in [("2025-03-31", "Sí", "007"), ("2024-12-01", "No", "10")].iter().enumerate() {
            d.set_value(row, 2, f).ok();
            d.set_value(row, 3, a).ok();
            d.set_value(row, 4, c).ok();
        }
        d
    }

    #[test]
    fn dates_yes_no_and_text_are_read_by_their_type() -> Result<()> {
        let d = typed();
        let sel = |e: &str| select(&d, GeomKind::Polygon, e);
        // las fechas se comparan como fechas, con un texto que se lee como fecha, y hacen
        // aritmética en días
        assert_eq!(sel("fecha > '2025-01-01'")?, [0]);
        assert_eq!(sel("fecha < date('01/01/2025')")?, [1]);
        assert_eq!(sel("year(fecha) = 2024")?, [1]);
        assert_eq!(sel("fecha - date('2025-03-01') = 30")?, [0]);
        assert_eq!(sel("fecha >= today() - 36500")?, [0, 1]);
        // sí/no
        assert_eq!(sel("activo")?, [0]);
        assert_eq!(sel("not activo = true")?, [1]);
        assert_eq!(sel("activo = false")?, [1]);
        // un campo de texto se compara como texto: «007» no es 7 y «10» va antes que «7»
        assert_eq!(sel("codigo = 7")?, Vec::<usize>::new());
        assert_eq!(sel("codigo = '007'")?, [0]);
        assert_eq!(sel("codigo < '2'")?, [0, 1]);
        // un campo numérico se compara como número
        assert_eq!(sel("n > 2")?, [0]);
        Ok(())
    }

    #[test]
    fn values_written_to_a_field_must_fit_its_type() -> Result<()> {
        let mut d = typed();
        // un texto en un campo numérico o de fecha se rechaza y no cambia nada
        assert!(apply(&mut d, GeomKind::Polygon, "n", "especie", None).is_err());
        assert!(apply(&mut d, GeomKind::Polygon, "fecha", "'mañana'", None).is_err());
        assert_eq!(d.text[0][1], "3");
        // un decimal en un campo de números enteros se rechaza, redondeado se acepta
        assert!(apply(&mut d, GeomKind::Polygon, "n", "$area / 7", None).is_err());
        apply(&mut d, GeomKind::Polygon, "n", "round($area / 7)", None)?;
        assert_eq!(d.text[0][1], "14");
        // un resultado sí/no en un campo sí/no es Sí / No
        apply(&mut d, GeomKind::Polygon, "activo", "$area > 150", None)?;
        assert_eq!((d.text[0][3].as_str(), d.text[1][3].as_str()), ("No", "Sí"));
        // un campo nuevo toma el tipo de los valores
        apply(&mut d, GeomKind::Polygon, "nuevo_f", "fecha + 10", None)?;
        apply(&mut d, GeomKind::Polygon, "nuevo_a", "$area", None)?;
        apply(&mut d, GeomKind::Polygon, "nuevo_s", "especie", None)?;
        apply(&mut d, GeomKind::Polygon, "nuevo_b", "$area > 5", None)?;
        use crate::fieldtype::FieldType::*;
        let t = |n: &str| d.fields.iter().position(|f| f == n).map(|c| d.field_type(c));
        assert_eq!(t("nuevo_f"), Some(Date));
        assert_eq!(t("nuevo_a"), Some(Integer));
        assert_eq!(t("nuevo_s"), Some(Text));
        assert_eq!(t("nuevo_b"), Some(Bool));
        assert_eq!(d.text[0][5], "2025-04-10");
        Ok(())
    }
}
