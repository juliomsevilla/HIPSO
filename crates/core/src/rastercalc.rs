//! Raster calculator: an expression over one or several named rasters, evaluated cell by cell.
//!
//! ```text
//! expr    := or
//! or      := and ('or' and)*
//! and     := cmp ('and' cmp)*
//! cmp     := sum (('<'|'<='|'>'|'>='|'=='|'!=') sum)?
//! sum     := term (('+'|'-') term)*
//! term    := unary (('*'|'/'|'%') unary)*
//! unary   := ('-'|'not') unary | power
//! power   := atom ('^' unary)?
//! atom    := number | variable | 'nan' | func '(' args ')' | '(' expr ')'
//! ```
//!
//! A variable is a name given to a raster (`A`, `chm`) or one band of it (`A.2`). True is 1 and
//! false is 0; anything but 0 counts as true. A value that is no number (an empty cell) makes
//! the result empty, except inside `isnan()`; so does a division by zero. `-2^2` is `-(2^2)`
//! and `2^3^2` is `2^(3^2)`; `not` binds like the minus sign, so `not A > 0` is `(not A) > 0`.
//!
//! The expression is compiled once to a short program for a small stack and run on strips of
//! rows, in parallel, so memory follows a strip and not the raster.

use std::path::PathBuf;

use rayon::prelude::*;

use crate::grid::NODATA;
use crate::pyramid::Pyramid;
use crate::rastertools::{Dtype, Resample};
use crate::{Error, Result};

/// A name an expression can use: a band of an input raster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    pub name: String,
    pub raster: usize,
    pub band: usize,
}

/// Where and why an expression is not valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Position of the error in the text, from 1.
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "columna {}: {}", self.column, self.message)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fn1 {
    Abs,
    Sqrt,
    Ln,
    Log10,
    Exp,
    Floor,
    Ceil,
    Round,
    Sin,
    Cos,
    Tan,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Const(f32),
    Var(usize),
    Neg,
    Not,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Cmp(Cmp),
    And,
    Or,
    F1(Fn1),
    Atan2,
    /// The smallest of the last `n` values.
    Min(usize),
    Max(usize),
    Clamp,
    If,
    IsNan,
}

/// A compiled expression.
#[derive(Clone, Debug)]
pub struct Calc {
    prog: Vec<Op>,
    /// Which of the variables the program reads.
    used: Vec<bool>,
    depth: usize,
}

// ---------------------------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f32),
    Name(String),
    Sym(&'static str),
    End,
}

struct Lexer {
    toks: Vec<(Tok, usize)>,
}

fn lex(text: &str) -> std::result::Result<Lexer, ParseError> {
    let chars: Vec<char> = text.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let col = i + 1;
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let s: String = chars[start..i].iter().collect();
            let v = s.parse::<f32>().map_err(|_| ParseError {
                column: col,
                message: format!("«{s}» no es un número"),
            })?;
            toks.push((Tok::Num(v), col));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            // a band: name.digits
            if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            toks.push((Tok::Name(chars[start..i].iter().collect()), col));
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            let sym = match two.as_str() {
                "<=" => Some("<="),
                ">=" => Some(">="),
                "==" => Some("=="),
                "!=" => Some("!="),
                _ => None,
            };
            if let Some(s) = sym {
                toks.push((Tok::Sym(s), col));
                i += 2;
                continue;
            }
            let s = match c {
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                '%' => "%",
                '^' => "^",
                '<' => "<",
                '>' => ">",
                '(' => "(",
                ')' => ")",
                ',' => ",",
                _ => {
                    return Err(ParseError {
                        column: col,
                        message: format!("carácter no válido «{c}»"),
                    });
                }
            };
            toks.push((Tok::Sym(s), col));
            i += 1;
        }
    }
    toks.push((Tok::End, chars.len() + 1));
    Ok(Lexer { toks })
}

// --------------------------------------------------------------------------------- parser

struct Parser<'a> {
    toks: Vec<(Tok, usize)>,
    at: usize,
    vars: &'a [Var],
    prog: Vec<Op>,
    used: Vec<bool>,
}

type PResult = std::result::Result<(), ParseError>;

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        &self.toks[self.at].0
    }

    fn col(&self) -> usize {
        self.toks[self.at].1
    }

    fn next(&mut self) {
        if self.at + 1 < self.toks.len() {
            self.at += 1;
        }
    }

    fn err<T>(&self, message: impl Into<String>) -> std::result::Result<T, ParseError> {
        Err(ParseError {
            column: self.col(),
            message: message.into(),
        })
    }

    fn sym(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sym(t) if *t == s)
    }

    fn word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Name(n) if n == w)
    }

    fn expect(&mut self, s: &str) -> PResult {
        if self.sym(s) {
            self.next();
            Ok(())
        } else if matches!(self.peek(), Tok::End) {
            self.err(format!("falta «{s}»"))
        } else {
            self.err(format!("se esperaba «{s}»"))
        }
    }

    fn or(&mut self) -> PResult {
        self.and()?;
        while self.word("or") {
            self.next();
            self.and()?;
            self.prog.push(Op::Or);
        }
        Ok(())
    }

    fn and(&mut self) -> PResult {
        self.cmp()?;
        while self.word("and") {
            self.next();
            self.cmp()?;
            self.prog.push(Op::And);
        }
        Ok(())
    }

    fn cmp(&mut self) -> PResult {
        self.sum()?;
        let op = match self.peek() {
            Tok::Sym("<") => Some(Cmp::Lt),
            Tok::Sym("<=") => Some(Cmp::Le),
            Tok::Sym(">") => Some(Cmp::Gt),
            Tok::Sym(">=") => Some(Cmp::Ge),
            Tok::Sym("==") => Some(Cmp::Eq),
            Tok::Sym("!=") => Some(Cmp::Ne),
            _ => None,
        };
        if let Some(op) = op {
            self.next();
            self.sum()?;
            self.prog.push(Op::Cmp(op));
        }
        Ok(())
    }

    fn sum(&mut self) -> PResult {
        self.term()?;
        loop {
            let op = if self.sym("+") {
                Op::Add
            } else if self.sym("-") {
                Op::Sub
            } else {
                return Ok(());
            };
            self.next();
            self.term()?;
            self.prog.push(op);
        }
    }

    fn term(&mut self) -> PResult {
        self.unary()?;
        loop {
            let op = if self.sym("*") {
                Op::Mul
            } else if self.sym("/") {
                Op::Div
            } else if self.sym("%") {
                Op::Rem
            } else {
                return Ok(());
            };
            self.next();
            self.unary()?;
            self.prog.push(op);
        }
    }

    fn unary(&mut self) -> PResult {
        if self.sym("-") {
            self.next();
            self.unary()?;
            self.prog.push(Op::Neg);
            Ok(())
        } else if self.word("not") {
            self.next();
            self.unary()?;
            self.prog.push(Op::Not);
            Ok(())
        } else {
            self.power()
        }
    }

    fn power(&mut self) -> PResult {
        self.atom()?;
        if self.sym("^") {
            self.next();
            self.unary()?;
            self.prog.push(Op::Pow);
        }
        Ok(())
    }

    fn args(&mut self) -> std::result::Result<usize, ParseError> {
        self.expect("(")?;
        let mut n = 0;
        if self.sym(")") {
            self.next();
            return Ok(0);
        }
        loop {
            self.or()?;
            n += 1;
            if self.sym(",") {
                self.next();
            } else {
                break;
            }
        }
        self.expect(")")?;
        Ok(n)
    }

    fn atom(&mut self) -> PResult {
        let col = self.col();
        match self.peek().clone() {
            Tok::Num(v) => {
                self.next();
                self.prog.push(Op::Const(v));
                Ok(())
            }
            Tok::Sym("(") => {
                self.next();
                self.or()?;
                self.expect(")")
            }
            Tok::Name(name) => {
                self.next();
                if name == "nan" {
                    self.prog.push(Op::Const(f32::NAN));
                    return Ok(());
                }
                if self.sym("(") {
                    return self.call(&name, col);
                }
                match self.vars.iter().position(|v| v.name == name) {
                    Some(k) => {
                        self.used[k] = true;
                        self.prog.push(Op::Var(k));
                        Ok(())
                    }
                    None => Err(ParseError {
                        column: col,
                        message: format!("no hay un ráster llamado «{name}»"),
                    }),
                }
            }
            Tok::End => self.err("la expresión se acaba antes de tiempo"),
            Tok::Sym(s) => self.err(format!("no se esperaba «{s}»")),
        }
    }

    fn call(&mut self, name: &str, col: usize) -> PResult {
        let n = self.args()?;
        let bad = |want: &str| {
            Err(ParseError {
                column: col,
                message: format!("{name}() pide {want}"),
            })
        };
        let (op, want): (Op, Option<usize>) = match name {
            "abs" => (Op::F1(Fn1::Abs), Some(1)),
            "sqrt" => (Op::F1(Fn1::Sqrt), Some(1)),
            "ln" => (Op::F1(Fn1::Ln), Some(1)),
            "log10" => (Op::F1(Fn1::Log10), Some(1)),
            "exp" => (Op::F1(Fn1::Exp), Some(1)),
            "floor" => (Op::F1(Fn1::Floor), Some(1)),
            "ceil" => (Op::F1(Fn1::Ceil), Some(1)),
            "round" => (Op::F1(Fn1::Round), Some(1)),
            "sin" => (Op::F1(Fn1::Sin), Some(1)),
            "cos" => (Op::F1(Fn1::Cos), Some(1)),
            "tan" => (Op::F1(Fn1::Tan), Some(1)),
            "isnan" => (Op::IsNan, Some(1)),
            "pow" => (Op::Pow, Some(2)),
            "atan2" => (Op::Atan2, Some(2)),
            "clamp" => (Op::Clamp, Some(3)),
            "if" => (Op::If, Some(3)),
            "min" => (Op::Min(n), None),
            "max" => (Op::Max(n), None),
            _ => {
                return Err(ParseError {
                    column: col,
                    message: format!("función desconocida «{name}»"),
                });
            }
        };
        match want {
            Some(w) if n != w => {
                return bad(&format!("{w} argumento{}", if w == 1 { "" } else { "s" }));
            }
            None if n < 2 => return bad("dos o más argumentos"),
            _ => {}
        }
        self.prog.push(op);
        Ok(())
    }
}

/// The stack depth a program needs, or the position of the first op that pops too much.
fn depth_of(prog: &[Op]) -> usize {
    let (mut d, mut max) = (0usize, 0usize);
    for op in prog {
        let (pops, pushes) = match op {
            Op::Const(_) | Op::Var(_) => (0, 1),
            Op::Neg | Op::Not | Op::F1(_) | Op::IsNan => (1, 1),
            Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Rem
            | Op::Pow
            | Op::Cmp(_)
            | Op::And
            | Op::Or
            | Op::Atan2 => (2, 1),
            Op::Min(n) | Op::Max(n) => (*n, 1),
            Op::Clamp | Op::If => (3, 1),
        };
        d = d.saturating_sub(pops) + pushes;
        max = max.max(d);
    }
    max
}

impl Calc {
    /// Compiles `text`, whose names are `vars`.
    pub fn compile(text: &str, vars: &[Var]) -> std::result::Result<Calc, ParseError> {
        let lexer = lex(text)?;
        let mut p = Parser {
            toks: lexer.toks,
            at: 0,
            vars,
            prog: Vec::new(),
            used: vec![false; vars.len()],
        };
        if matches!(p.peek(), Tok::End) {
            return p.err("la expresión está vacía");
        }
        p.or()?;
        if !matches!(p.peek(), Tok::End) {
            return match p.peek() {
                Tok::Sym(")") => p.err("sobra «)»"),
                _ => p.err("no se esperaba esto aquí"),
            };
        }
        let depth = depth_of(&p.prog);
        Ok(Calc {
            prog: p.prog,
            used: p.used,
            depth,
        })
    }

    /// Like [`Calc::compile`], with the error as one of ours.
    pub fn parse(text: &str, vars: &[Var]) -> Result<Calc> {
        Self::compile(text, vars).map_err(|e| Error::Param(e.to_string()))
    }

    /// Which variables the expression reads (same order as the `vars` it was compiled with).
    pub fn used(&self) -> &[bool] {
        &self.used
    }

    /// The value for `inputs` (one per variable). Allocates its stack: use
    /// [`Calc::eval_with`] in a loop.
    pub fn eval(&self, inputs: &[f32]) -> f32 {
        let mut stack = Vec::with_capacity(self.depth);
        self.eval_with(inputs, &mut stack)
    }

    /// The value for `inputs`, with a stack to reuse.
    pub fn eval_with(&self, inputs: &[f32], s: &mut Vec<f32>) -> f32 {
        s.clear();
        for op in &self.prog {
            match *op {
                Op::Const(v) => s.push(v),
                Op::Var(k) => s.push(inputs.get(k).copied().unwrap_or(f32::NAN)),
                Op::Neg => un(s, |a| -a),
                Op::Not => un(s, |a| flag(a == 0.0)),
                Op::Add => bin(s, |a, b| a + b),
                Op::Sub => bin(s, |a, b| a - b),
                Op::Mul => bin(s, |a, b| a * b),
                Op::Div => bin(s, |a, b| if b == 0.0 { f32::NAN } else { a / b }),
                Op::Rem => bin(s, |a, b| if b == 0.0 { f32::NAN } else { a % b }),
                Op::Pow => bin(s, f32::powf),
                Op::Cmp(c) => bin(s, |a, b| {
                    flag(match c {
                        Cmp::Lt => a < b,
                        Cmp::Le => a <= b,
                        Cmp::Gt => a > b,
                        Cmp::Ge => a >= b,
                        Cmp::Eq => a == b,
                        Cmp::Ne => a != b,
                    })
                }),
                Op::And => bin(s, |a, b| flag(a != 0.0 && b != 0.0)),
                Op::Or => bin(s, |a, b| flag(a != 0.0 || b != 0.0)),
                Op::F1(f) => un(s, |a| match f {
                    Fn1::Abs => a.abs(),
                    Fn1::Sqrt => a.sqrt(),
                    Fn1::Ln => a.ln(),
                    Fn1::Log10 => a.log10(),
                    Fn1::Exp => a.exp(),
                    Fn1::Floor => a.floor(),
                    Fn1::Ceil => a.ceil(),
                    Fn1::Round => a.round(),
                    Fn1::Sin => a.sin(),
                    Fn1::Cos => a.cos(),
                    Fn1::Tan => a.tan(),
                }),
                Op::Atan2 => bin(s, f32::atan2),
                Op::Min(n) | Op::Max(n) => {
                    let from = s.len().saturating_sub(n);
                    let take = &s[from..];
                    let min = matches!(op, Op::Min(_));
                    let mut r = take.first().copied().unwrap_or(f32::NAN);
                    for &v in take {
                        if v.is_nan() {
                            r = f32::NAN;
                            break;
                        }
                        if (min && v < r) || (!min && v > r) {
                            r = v;
                        }
                    }
                    s.truncate(from);
                    s.push(r);
                }
                Op::Clamp => {
                    let hi = s.pop().unwrap_or(f32::NAN);
                    let lo = s.pop().unwrap_or(f32::NAN);
                    let x = s.pop().unwrap_or(f32::NAN);
                    s.push(if x.is_nan() || lo.is_nan() || hi.is_nan() {
                        f32::NAN
                    } else {
                        x.max(lo).min(hi)
                    });
                }
                Op::If => {
                    let b = s.pop().unwrap_or(f32::NAN);
                    let a = s.pop().unwrap_or(f32::NAN);
                    let c = s.pop().unwrap_or(f32::NAN);
                    s.push(if c.is_nan() {
                        f32::NAN
                    } else if c != 0.0 {
                        a
                    } else {
                        b
                    });
                }
                Op::IsNan => {
                    if let Some(a) = s.last_mut() {
                        *a = flag(a.is_nan());
                    }
                }
            }
        }
        s.pop().unwrap_or(f32::NAN)
    }
}

#[inline]
fn flag(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

/// Replaces the top value by `f` of it; a NaN stays NaN.
#[inline]
fn un(s: &mut [f32], f: impl Fn(f32) -> f32) {
    if let Some(a) = s.last_mut() {
        *a = if a.is_nan() { f32::NAN } else { f(*a) };
    }
}

/// Replaces the top two values by `f` of them; a NaN in either gives NaN.
#[inline]
fn bin(s: &mut Vec<f32>, f: impl Fn(f32, f32) -> f32) {
    let b = s.pop().unwrap_or(f32::NAN);
    if let Some(a) = s.last_mut() {
        *a = if a.is_nan() || b.is_nan() {
            f32::NAN
        } else {
            f(*a, b)
        };
    }
}

// ------------------------------------------------------------------------------------ job

/// What to compute.
#[derive(Clone, Debug)]
pub struct CalcJob {
    pub expr: String,
    /// The rasters and the names the expression gives them (`A`, `B`…).
    pub inputs: Vec<(String, PathBuf)>,
    pub output: PathBuf,
    pub dtype: Dtype,
    /// The overviews take the first valid pixel, not the mean: for classes.
    pub nearest: bool,
    /// Put the other rasters on the cells of the first one with this resampling when their
    /// cells differ; `None` refuses them.
    pub align: Option<Resample>,
    /// What an empty cell holds in the output; `None`: -9999 for decimals, 0 for integers.
    pub nodata: Option<f32>,
}

impl CalcJob {
    pub fn new(expr: String, inputs: Vec<(String, PathBuf)>, output: PathBuf) -> Self {
        Self {
            expr,
            inputs,
            output,
            dtype: Dtype::F32,
            nearest: false,
            align: None,
            nodata: None,
        }
    }
}

/// The names an expression can use for `inputs` (already opened): each raster by its name, each
/// band as `name.k`, and, when there is a single raster, `B1`…`Bn`.
pub fn variables(names: &[(String, usize)]) -> Vec<Var> {
    let mut v = Vec::new();
    for (i, (name, bands)) in names.iter().enumerate() {
        v.push(Var {
            name: name.clone(),
            raster: i,
            band: 0,
        });
        for b in 0..*bands {
            v.push(Var {
                name: format!("{name}.{}", b + 1),
                raster: i,
                band: b,
            });
        }
    }
    if let [(_, bands)] = names {
        for b in 0..*bands {
            v.push(Var {
                name: format!("B{}", b + 1),
                raster: 0,
                band: b,
            });
        }
    }
    v
}

fn same_cells(a: &Pyramid, b: &Pyramid) -> bool {
    let tol = a.cell * 1e-6;
    a.levels[0].cols == b.levels[0].cols
        && a.levels[0].rows == b.levels[0].rows
        && (a.cell - b.cell).abs() <= tol
        && (a.x0 - b.x0).abs() <= tol
        && (a.y_top - b.y_top).abs() <= tol
}

fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|f| f.is_alphabetic() || f == '_')
        && c.all(|f| f.is_alphanumeric() || f == '_')
        && !matches!(n, "and" | "or" | "not" | "nan")
}

/// Computes the expression over the rasters and writes the result; returns the summary.
pub fn run(job: &CalcJob) -> Result<String> {
    if job.inputs.is_empty() {
        return Err(Error::Param("no hay ráster de entrada".into()));
    }
    for (i, (n, _)) in job.inputs.iter().enumerate() {
        if !valid_name(n) {
            return Err(Error::Param(format!(
                "«{n}» no vale como nombre de ráster: letras, cifras y «_», sin empezar por cifra"
            )));
        }
        if job.inputs[..i].iter().any(|(m, _)| m == n) {
            return Err(Error::Param(format!("el nombre «{n}» está repetido")));
        }
    }
    let mut opened: Vec<Pyramid> = Vec::new();
    for (_, p) in &job.inputs {
        opened.push(Pyramid::open(p)?);
    }
    // the rasters must share cells; if asked, the others are put on those of the first
    // one folder per run: two calculations at once must not share (or delete) each other's copies
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp = std::env::temp_dir().join(format!(
        "hipso_calc_{}_{}",
        std::process::id(),
        RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut notes = Vec::new();
    let mut aligned: Vec<PathBuf> = Vec::new();
    for i in 1..opened.len() {
        if same_cells(&opened[0], &opened[i]) {
            continue;
        }
        let Some(resample) = job.align else {
            return Err(Error::Param(format!(
                "«{}» no tiene las mismas celdas que «{}» ({} x {} y {} x {}): igualarlos antes con «Igualar rásteres» o pedir que se alineen",
                job.inputs[i].0,
                job.inputs[0].0,
                opened[i].levels[0].cols,
                opened[i].levels[0].rows,
                opened[0].levels[0].cols,
                opened[0].levels[0].rows
            )));
        };
        std::fs::create_dir_all(&temp)?;
        let out = temp.join(format!("{}.tif", job.inputs[i].0));
        crate::rastertools::align(&crate::rastertools::AlignJob {
            reference: job.inputs[0].1.clone(),
            inputs: vec![(job.inputs[i].1.clone(), out.clone())],
            resample: Some(resample),
            dtype: Some(Dtype::F32),
            nodata: NODATA,
            src_nodata: None,
            compression: None,
        })?;
        opened[i] = Pyramid::open(&out)?;
        aligned.push(out);
        notes.push(format!(
            "«{}» alineado con «{}»",
            job.inputs[i].0, job.inputs[0].0
        ));
    }
    let names: Vec<(String, usize)> = job
        .inputs
        .iter()
        .zip(&opened)
        .map(|((n, _), p)| (n.clone(), p.bands))
        .collect();
    let vars = variables(&names);
    let calc = Calc::parse(&job.expr, &vars)?;

    let first = &opened[0];
    let e = first.extent();
    let cell = first.cell;
    let (cols, rows) = (first.levels[0].cols as usize, first.levels[0].rows as usize);
    let nodata_in: Vec<Option<f32>> = opened.iter().map(Pyramid::nodata).collect();
    let budget = crate::copc_cache::total_memory()
        .map_or(24_000_000, |m| (m / 512).clamp(16_000_000, 256_000_000));
    // a strip holds every raster it reads
    let step = (budget as usize / (cols.max(1) * opened.len().max(1))).clamp(8, 16_384);
    crate::paths::ensure_parent(&job.output)?;
    let nodata = job
        .nodata
        .unwrap_or(if job.dtype == Dtype::F32 { NODATA } else { 0.0 });
    let spec = crate::tiled::Spec {
        cols,
        rows,
        bands: 1,
        dtype: job.dtype,
        cell,
        x0: e[0],
        top: e[3],
        epsg: first.epsg,
        nodata,
        compression: crate::tiled::auto_compression(job.dtype, 1),
        nearest: job.nearest,
        bigtiff: false,
    };
    let started = std::time::Instant::now();
    let (mut valid, mut lo, mut hi, mut sum) = (0u64, f32::INFINITY, f32::NEG_INFINITY, 0.0f64);
    crate::progress::begin(rows as u64);
    let used = calc.used().to_vec();
    crate::tiled::write(&job.output, spec, step, |r0, r1| {
        crate::progress::advance((r1 - r0) as u64);
        // a quarter of a cell inside the strip, so that the pixel boundaries are the right ones
        let q = cell * 0.25;
        let win = [
            e[0] + q,
            e[3] - r1 as f64 * cell + q,
            e[2] - q,
            e[3] - r0 as f64 * cell - q,
        ];
        let mut imgs = Vec::with_capacity(opened.len());
        for (k, p) in opened.iter().enumerate() {
            let needed = vars.iter().zip(&used).any(|(v, u)| *u && v.raster == k);
            if !needed {
                imgs.push(None);
                continue;
            }
            let img = p.read(0, win)?;
            if img.cols != cols || img.rows != r1 - r0 {
                return Err(Error::Format(format!(
                    "la franja de «{}» mide {} x {} y se esperaba {} x {}",
                    job.inputs[k].0,
                    img.cols,
                    img.rows,
                    cols,
                    r1 - r0
                )));
            }
            imgs.push(Some(img));
        }
        let mut out = vec![f32::NAN; (r1 - r0) * cols];
        out.par_chunks_mut(cols).enumerate().for_each_init(
            || (Vec::with_capacity(8), vec![f32::NAN; vars.len()]),
            |(stack, vals), (row, line)| {
                for (c, cell_out) in line.iter_mut().enumerate() {
                    let i = row * cols + c;
                    for (k, v) in vars.iter().enumerate() {
                        if !used[k] {
                            continue;
                        }
                        let x = imgs[v.raster]
                            .as_ref()
                            .and_then(|m| m.bands.get(v.band))
                            .and_then(|b| b.get(i))
                            .copied()
                            .unwrap_or(f32::NAN);
                        vals[k] = if !x.is_finite() || nodata_in[v.raster].is_some_and(|nd| x == nd)
                        {
                            f32::NAN
                        } else {
                            x
                        };
                    }
                    let r = calc.eval_with(vals, stack);
                    *cell_out = if r.is_finite() { r } else { f32::NAN };
                }
            },
        );
        for &v in &out {
            if !v.is_nan() {
                valid += 1;
                lo = lo.min(v);
                hi = hi.max(v);
                sum += f64::from(v);
            }
        }
        Ok(out)
    })?;
    if !aligned.is_empty() {
        let _ = std::fs::remove_dir_all(&temp);
    }
    let mut lines = notes;
    lines.push(if valid == 0 {
        format!("sin celdas con valor -> {}", job.output.display())
    } else {
        format!(
            "{} celdas con valor, de {:.4} a {:.4}, media {:.4} ({:.1} s) -> {}",
            valid,
            lo,
            hi,
            sum / valid as f64,
            started.elapsed().as_secs_f64(),
            job.output.display()
        )
    });
    Ok(lines.join("\n"))
}

/// The expression with the names `A`, `B`… in the order of `paths`: for a form that assigns the
/// letters itself.
pub fn letters(paths: &[PathBuf]) -> Vec<(String, PathBuf)> {
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let n = if i < 26 {
                char::from(b'A' + i as u8).to_string()
            } else {
                format!("R{}", i + 1)
            };
            (n, p.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(names: &[&str]) -> Vec<Var> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Var {
                name: (*n).to_string(),
                raster: i,
                band: 0,
            })
            .collect()
    }

    fn calc(text: &str, names: &[&str], inputs: &[f32]) -> Result<f32> {
        Ok(Calc::parse(text, &vars(names))?.eval(inputs))
    }

    #[test]
    fn precedence_and_power() -> Result<()> {
        assert_eq!(calc("1+2*3", &[], &[])?, 7.0);
        assert_eq!(calc("(1+2)*3", &[], &[])?, 9.0);
        assert_eq!(calc("-2^2", &[], &[])?, -4.0);
        assert_eq!(calc("2^3^2", &[], &[])?, 512.0);
        assert_eq!(calc("2^-1", &[], &[])?, 0.5);
        assert_eq!(calc("7 % 4", &[], &[])?, 3.0);
        assert_eq!(calc("1e-3 * 1000", &[], &[])?, 1.0);
        Ok(())
    }

    #[test]
    fn conditions_and_functions() -> Result<()> {
        let n = ["A", "B", "C"];
        assert_eq!(calc("if(A>0, A, 0)", &n, &[5.0, 0.0, 0.0])?, 5.0);
        assert_eq!(calc("if(A>0, A, 0)", &n, &[-5.0, 0.0, 0.0])?, 0.0);
        assert_eq!(calc("clamp(A, 0, 10)", &n, &[15.0, 0.0, 0.0])?, 10.0);
        assert_eq!(calc("min(A,B,C)", &n, &[4.0, 2.0, 3.0])?, 2.0);
        assert_eq!(calc("max(A,B,C)", &n, &[4.0, 2.0, 3.0])?, 4.0);
        assert_eq!(calc("A > 1 and B > 1", &n, &[2.0, 0.0, 0.0])?, 0.0);
        assert_eq!(calc("A > 1 or B > 1", &n, &[2.0, 0.0, 0.0])?, 1.0);
        assert_eq!(calc("not 0", &n, &[])?, 1.0);
        assert_eq!(
            calc(
                "sqrt(16) + abs(-3) + floor(2.7) + ceil(2.1) + round(2.5)",
                &n,
                &[]
            )?,
            15.0
        );
        assert!((calc("atan2(1, 1)", &n, &[])? - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
        assert!((calc("log10(1000) + ln(1)", &n, &[])? - 3.0).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn nan_spreads_unless_asked() -> Result<()> {
        let n = ["A"];
        assert!(calc("A + 1", &n, &[f32::NAN])?.is_nan());
        assert!(calc("A > 1", &n, &[f32::NAN])?.is_nan());
        assert!(calc("min(A, 1)", &n, &[f32::NAN])?.is_nan());
        assert_eq!(calc("isnan(A)", &n, &[f32::NAN])?, 1.0);
        assert_eq!(calc("isnan(A)", &n, &[2.0])?, 0.0);
        assert_eq!(calc("if(isnan(A), 0, A)", &n, &[f32::NAN])?, 0.0);
        assert!(calc("1 / 0", &n, &[])?.is_nan());
        assert!(calc("nan", &n, &[])?.is_nan());
        Ok(())
    }

    #[test]
    fn errors_say_the_column() {
        let v = vars(&["A"]);
        let col = |t: &str| Calc::compile(t, &v).err().map(|e| e.column);
        assert_eq!(col("(A + 1"), Some(7));
        assert_eq!(col("A + 1)"), Some(6));
        assert_eq!(col("foo(A)"), Some(1));
        assert_eq!(col("A + Z"), Some(5));
        assert_eq!(col("A + "), Some(5));
        assert_eq!(col("A $ 1"), Some(3));
        assert_eq!(col(""), Some(1));
        assert_eq!(col("min(A)"), Some(1));
        assert_eq!(col("clamp(A, 1)"), Some(1));
        assert!(Calc::compile("A + 1", &v).is_ok());
    }

    #[test]
    fn bands_and_the_old_names() {
        let v = variables(&[("A".into(), 3)]);
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["A", "A.1", "A.2", "A.3", "B1", "B2", "B3"]);
        assert!(Calc::compile("(B3 - B2) / (B3 + B2)", &v).is_ok());
        assert!(Calc::compile("A.2 * 2", &v).is_ok());
        // with two rasters there are no B1…Bn
        let w = variables(&[("A".into(), 1), ("B".into(), 1)]);
        assert!(Calc::compile("B1", &w).is_err());
    }

    #[test]
    fn it_reads_only_what_it_uses() -> Result<()> {
        let c = Calc::parse("A * 2", &vars(&["A", "B"]))?;
        assert_eq!(c.used(), [true, false]);
        Ok(())
    }

    fn write(
        path: &std::path::Path,
        cols: usize,
        rows: usize,
        cell: f64,
        f: impl Fn(usize, usize) -> f32,
    ) -> Result<()> {
        let mut g = crate::grid::Grid::new(0.0, 0.0, cell, cols, rows)?;
        for r in 0..rows {
            for c in 0..cols {
                g.data[r * cols + c] = f(c, r);
            }
        }
        g.write_tiff(path, Some(25829))
    }

    #[test]
    fn it_computes_over_rasters_and_strips_agree_with_the_whole() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_calc_t_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let (a, b) = (dir.join("a.tif"), dir.join("b.tif"));
        write(&a, 40, 30, 1.0, |c, r| (c + r) as f32)?;
        write(&b, 40, 30, 1.0, |c, _| {
            if c % 7 == 0 { f32::NAN } else { c as f32 }
        })?;
        let mut job = CalcJob::new(
            "if(A > 20, A - B, 0) + 1".into(),
            vec![("A".into(), a), ("B".into(), b)],
            dir.join("out.tif"),
        );
        let msg = run(&job)?;
        assert!(msg.contains("celdas con valor"), "{msg}");
        let out = crate::grid::Grid::read(&dir.join("out.tif"))?;
        for r in 0..30 {
            for c in 0..40 {
                let (va, vb) = ((c + r) as f32, if c % 7 == 0 { f32::NAN } else { c as f32 });
                let want = if va > 20.0 { va - vb } else { 0.0 } + 1.0;
                let got = out.data[r * 40 + c];
                if want.is_nan() {
                    assert!(got.is_nan(), "{c},{r}: {got}");
                } else {
                    assert!((got - want).abs() < 1e-5, "{c},{r}: {got} / {want}");
                }
            }
        }
        // an integer output rounds and clamps
        job.dtype = Dtype::U8;
        job.expr = "A * 10 + 1".into();
        job.output = dir.join("u8.tif");
        run(&job)?;
        let u = crate::grid::Grid::read(&dir.join("u8.tif"))?;
        assert_eq!(u.data[0], 1.0);
        assert_eq!(u.data[5], 51.0);
        assert_eq!(u.data[39], 255.0);
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    /// Four rasters at once, one of them with a hole and another with other cells: every
    /// letter reads its own raster, an empty cell of any input empties the result, and the
    /// raster with other cells is put on those of the first.
    #[test]
    fn it_computes_over_four_rasters_at_once() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_calc_4_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let p = |n: &str| dir.join(n);
        write(&p("a.tif"), 30, 20, 1.0, |c, r| (c + 2 * r) as f32)?;
        write(&p("b.tif"), 30, 20, 1.0, |_, r| r as f32 + 0.5)?;
        write(&p("c.tif"), 30, 20, 1.0, |c, r| if c == 3 && r == 4 { f32::NAN } else { 2.0 })?;
        write(&p("d.tif"), 60, 40, 0.5, |_, _| 10.0)?;
        let mut job = CalcJob::new(
            "if(A > 10, (A + B) * C, B) - D / 10".into(),
            ["a", "b", "c", "d"]
                .iter()
                .zip(["A", "B", "C", "D"])
                .map(|(f, n)| (n.to_string(), p(&format!("{f}.tif"))))
                .collect(),
            p("out.tif"),
        );
        // D has other cells: refused unless asked
        assert!(run(&job).is_err());
        job.align = Some(Resample::Average);
        run(&job)?;
        let out = crate::grid::Grid::read(&p("out.tif"))?;
        for r in 0..20usize {
            for c in 0..30usize {
                let (a, b) = ((c + 2 * r) as f32, r as f32 + 0.5);
                let cv = if c == 3 && r == 4 { f32::NAN } else { 2.0 };
                let want = if a > 10.0 { (a + b) * cv } else { b } - 1.0;
                let got = out.data[r * 30 + c];
                if want.is_nan() {
                    // `b` is taken when A <= 10, so only the branch that reads C can be empty
                    assert!(got.is_nan() == (a > 10.0), "{c},{r}: {got}");
                } else {
                    assert!((got - want).abs() < 1e-4, "{c},{r}: {got} / {want}");
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn rasters_with_other_cells_need_to_be_aligned() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_calc_a_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let (a, b) = (dir.join("a.tif"), dir.join("b.tif"));
        write(&a, 20, 20, 1.0, |c, _| c as f32)?;
        write(&b, 40, 40, 0.5, |_, _| 1.0)?;
        let mut job = CalcJob::new(
            "A + B".into(),
            vec![("A".into(), a), ("B".into(), b)],
            dir.join("out.tif"),
        );
        let e = run(&job).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("las mismas celdas"), "{e}");
        job.align = Some(Resample::Bilinear);
        let msg = run(&job)?;
        assert!(msg.contains("alineado"), "{msg}");
        let out = crate::grid::Grid::read(&dir.join("out.tif"))?;
        assert!((out.data[3] - 4.0).abs() < 1e-4, "{}", out.data[3]);
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn names_are_checked() {
        let mut job = CalcJob::new(
            "A".into(),
            vec![("1a".into(), PathBuf::from("x.tif"))],
            PathBuf::from("o.tif"),
        );
        assert!(run(&job).is_err());
        job.inputs = vec![
            ("A".into(), PathBuf::from("x.tif")),
            ("A".into(), PathBuf::from("y.tif")),
        ];
        assert!(run(&job).is_err());
        assert_eq!(letters(&[PathBuf::from("a"), PathBuf::from("b")])[1].0, "B");
    }
}
