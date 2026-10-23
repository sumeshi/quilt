//! Small typed expression language used by `filter` and `derive`.
//!
//! This deliberately is not a SQL parser: identifiers are literal column
//! names, and values are never coerced through strings or lossy float casts.
use crate::controllers::nested::literal_col;
use crate::error::QuiltError;
use polars::prelude::*;

const MAX_EXPRESSION_BYTES: usize = 16_384;
const MAX_EXPRESSION_TOKENS: usize = 256;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    QuotedIdent(String),
    Str(String),
    Int(i128),
    Float(f64),
    LParen,
    RParen,
    Op(String),
    End,
}

fn lex(input: &str) -> Result<Vec<Tok>, QuiltError> {
    if input.len() > MAX_EXPRESSION_BYTES {
        return Err(parse_err("expression is too long (maximum 16384 bytes)"));
    }
    let b = input.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        if out.len() >= MAX_EXPRESSION_TOKENS {
            return Err(parse_err("expression is too complex (maximum 256 tokens)"));
        }
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        match b[i] {
            b'(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            b')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            b'`' => {
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < b.len() {
                    if b[i] == b'`' {
                        if i + 1 < b.len() && b[i + 1] == b'`' {
                            s.push('`');
                            i += 2;
                        } else {
                            i += 1;
                            closed = true;
                            break;
                        }
                    } else {
                        let c = input[i..].chars().next().unwrap();
                        s.push(c);
                        i += c.len_utf8();
                    }
                }
                if !closed || s.is_empty() {
                    return Err(parse_err("invalid backtick quoted column name"));
                }
                out.push(Tok::QuotedIdent(s));
            }
            b'\'' | b'"' => {
                let quote = b[i];
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < b.len() {
                    if b[i] == quote {
                        if i + 1 < b.len() && b[i + 1] == quote {
                            s.push(quote as char);
                            i += 2;
                        } else {
                            i += 1;
                            closed = true;
                            break;
                        }
                    } else if b[i] == b'\\' {
                        i += 1;
                        if i == b.len() {
                            break;
                        }
                        let c = match b[i] {
                            b'n' => {
                                i += 1;
                                '\n'
                            }
                            b'r' => {
                                i += 1;
                                '\r'
                            }
                            b't' => {
                                i += 1;
                                '\t'
                            }
                            _ => {
                                let c = input[i..].chars().next().unwrap();
                                i += c.len_utf8();
                                c
                            }
                        };
                        s.push(c);
                    } else {
                        let c = input[i..].chars().next().unwrap();
                        s.push(c);
                        i += c.len_utf8();
                    }
                }
                if !closed {
                    return Err(parse_err("unterminated string literal"));
                }
                out.push(Tok::Str(s));
            }
            b'0'..=b'9' => {
                i += 1;
                while i < b.len()
                    && (b[i].is_ascii_digit()
                        || b[i] == b'.'
                        || b[i] == b'e'
                        || b[i] == b'E'
                        || ((b[i] == b'+' || b[i] == b'-')
                            && i > start
                            && matches!(b[i - 1], b'e' | b'E')))
                {
                    i += 1;
                }
                let raw = &input[start..i];
                if raw.contains('.') || raw.contains('e') || raw.contains('E') {
                    let n = raw
                        .parse::<f64>()
                        .map_err(|_| parse_err("invalid number literal"))?;
                    if !n.is_finite() {
                        return Err(parse_err("float literals must be finite"));
                    }
                    out.push(Tok::Float(n));
                } else {
                    out.push(Tok::Int(raw.parse::<i128>().map_err(|_| {
                        parse_err("integer literal is outside the exact supported range")
                    })?));
                }
            }
            b'=' | b'!' | b'<' | b'>' | b'&' | b'|' | b'+' | b'-' | b'*' | b'/' => {
                i += 1;
                if i < b.len()
                    && ((b[start] == b'=' && b[i] == b'=')
                        || (b[start] == b'!' && b[i] == b'=')
                        || (b[start] == b'<' && b[i] == b'=')
                        || (b[start] == b'>' && b[i] == b'=')
                        || (b[start] == b'&' && b[i] == b'&')
                        || (b[start] == b'|' && b[i] == b'|'))
                {
                    i += 1;
                }
                let op = &input[start..i];
                if matches!(op, "=" | "&" | "|") {
                    return Err(parse_err("use ==, &&, or || for these operators"));
                }
                out.push(Tok::Op(op.into()));
            }
            _ => {
                let c = input[i..].chars().next().unwrap();
                if c.is_alphabetic() || c == '_' {
                    i += c.len_utf8();
                    while i < b.len() {
                        let c = input[i..].chars().next().unwrap();
                        if c.is_alphanumeric() || c == '_' || c == '.' {
                            i += c.len_utf8();
                        } else {
                            break;
                        }
                    }
                    out.push(Tok::Ident(input[start..i].into()));
                } else {
                    return Err(parse_err(&format!("unexpected character {c:?}")));
                }
            }
        }
    }
    out.push(Tok::End);
    Ok(out)
}

#[derive(Clone, Debug)]
enum Node {
    Col(String),
    Str(String),
    Int(i128),
    Float(f64),
    Bool(bool),
    Null,
    Unary(String, Box<Node>),
    Binary(String, Box<Node>, Box<Node>),
    IsNull(Box<Node>, bool),
}
struct Parser {
    t: Vec<Tok>,
    i: usize,
}
impl Parser {
    fn peek(&self) -> &Tok {
        &self.t[self.i]
    }
    fn bump(&mut self) -> Tok {
        let x = self.t[self.i].clone();
        self.i += 1;
        x
    }
    fn op(&self, s: &str) -> bool {
        matches!(self.peek(),Tok::Op(x) if x==s)
    }
    fn word(&self, s: &str) -> bool {
        matches!(self.peek(),Tok::Ident(x) if x.eq_ignore_ascii_case(s))
    }
    fn eat_op(&mut self, s: &str) -> bool {
        if self.op(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn parse(mut self) -> Result<Node, QuiltError> {
        let n = self.or()?;
        if self.peek() != &Tok::End {
            return Err(parse_err("unexpected trailing input"));
        }
        Ok(n)
    }
    fn or(&mut self) -> Result<Node, QuiltError> {
        let mut n = self.and()?;
        loop {
            if self.word("or") || self.eat_op("||") {
                if self.word("or") {
                    self.bump();
                }
                n = Node::Binary("or".into(), Box::new(n), Box::new(self.and()?));
            } else {
                break;
            }
        }
        Ok(n)
    }
    fn and(&mut self) -> Result<Node, QuiltError> {
        let mut n = self.cmp()?;
        loop {
            if self.word("and") || self.eat_op("&&") {
                if self.word("and") {
                    self.bump();
                }
                n = Node::Binary("and".into(), Box::new(n), Box::new(self.cmp()?));
            } else {
                break;
            }
        }
        Ok(n)
    }
    fn cmp(&mut self) -> Result<Node, QuiltError> {
        let mut n = self.add()?;
        loop {
            if self.word("is") {
                self.bump();
                let neg = self.word("not");
                if neg {
                    self.bump();
                }
                if !self.word("null") {
                    return Err(parse_err("IS must be followed by NULL or NOT NULL"));
                }
                self.bump();
                n = Node::IsNull(Box::new(n), neg);
                continue;
            }
            let op = match self.peek() {
                Tok::Op(x) if matches!(x.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=") => {
                    x.clone()
                }
                _ => break,
            };
            self.bump();
            n = Node::Binary(op, Box::new(n), Box::new(self.add()?));
        }
        Ok(n)
    }
    fn add(&mut self) -> Result<Node, QuiltError> {
        let mut n = self.mul()?;
        loop {
            let op = if self.eat_op("+") {
                Some("+")
            } else if self.eat_op("-") {
                Some("-")
            } else {
                None
            };
            if let Some(o) = op {
                n = Node::Binary(o.into(), Box::new(n), Box::new(self.mul()?));
            } else {
                break;
            }
        }
        Ok(n)
    }
    fn mul(&mut self) -> Result<Node, QuiltError> {
        let mut n = self.unary()?;
        loop {
            let op = if self.eat_op("*") {
                Some("*")
            } else if self.eat_op("/") {
                Some("/")
            } else {
                None
            };
            if let Some(o) = op {
                n = Node::Binary(o.into(), Box::new(n), Box::new(self.unary()?));
            } else {
                break;
            }
        }
        Ok(n)
    }
    fn unary(&mut self) -> Result<Node, QuiltError> {
        if self.word("not") || self.eat_op("!") {
            if self.word("not") {
                self.bump();
            }
            return Ok(Node::Unary("not".into(), Box::new(self.unary()?)));
        }
        if self.eat_op("-") {
            return Ok(Node::Unary("-".into(), Box::new(self.unary()?)));
        }
        if self.eat_op("+") {
            return self.unary();
        }
        self.atom()
    }
    fn atom(&mut self) -> Result<Node, QuiltError> {
        match self.bump() {
            Tok::LParen => {
                let n = self.or()?;
                if self.bump() != Tok::RParen {
                    return Err(parse_err("expected ')'"));
                }
                Ok(n)
            }
            Tok::Ident(s) if s.eq_ignore_ascii_case("true") => Ok(Node::Bool(true)),
            Tok::Ident(s) if s.eq_ignore_ascii_case("false") => Ok(Node::Bool(false)),
            Tok::Ident(s) if s.eq_ignore_ascii_case("null") => Ok(Node::Null),
            Tok::Ident(s) => Ok(Node::Col(s)),
            Tok::QuotedIdent(s) => Ok(Node::Col(s)),
            Tok::Str(s) => Ok(Node::Str(s)),
            Tok::Int(n) => Ok(Node::Int(n)),
            Tok::Float(n) => Ok(Node::Float(n)),
            _ => Err(parse_err(
                "expected a column, literal, or parenthesized expression",
            )),
        }
    }
}

#[derive(Clone)]
enum Val {
    Col(String, DataType),
    Str(String),
    Int(i128),
    Float(f64),
    Bool(bool),
    Null,
    Expr(Expr, DataType),
}
fn parse_err(s: &str) -> QuiltError {
    QuiltError::usage(format!("filter expression: {s}"))
}
fn integer(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
    )
}
fn signed(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64
    )
}
fn numeric(dtype: &DataType) -> bool {
    integer(dtype)
        || matches!(
            dtype,
            DataType::Float32 | DataType::Float64 | DataType::Decimal(_, _)
        )
}
fn fail_type(a: &DataType, b: &DataType) -> QuiltError {
    QuiltError::usage(format!("cannot compare {a} with {b}; use a literal matching the column type (for example, quote strings)"))
}
fn resolve(n: &Node, s: &Schema) -> Result<Val, QuiltError> {
    Ok(match n {
        Node::Col(name) => Val::Col(
            name.clone(),
            s.get(name)
                .ok_or_else(|| {
                    QuiltError::schema(
                        "filter",
                        Some(name),
                        "column not found; quote unusual names with backticks",
                    )
                })?
                .clone(),
        ),
        Node::Str(x) => Val::Str(x.clone()),
        Node::Int(x) => Val::Int(*x),
        Node::Float(x) => Val::Float(*x),
        Node::Bool(x) => Val::Bool(*x),
        Node::Null => Val::Null,
        Node::IsNull(a, neg) => {
            let x = resolve(a, s)?;
            let e = as_expr(x);
            let e = if *neg { e.is_not_null() } else { e.is_null() };
            Val::Expr(e, DataType::Boolean)
        }
        Node::Unary(op, a) => {
            let x = resolve(a, s)?;
            match (op.as_str(), x) {
                ("not", Val::Bool(v)) => Val::Bool(!v),
                ("not", Val::Col(n, DataType::Boolean)) => {
                    Val::Expr(literal_col(&n).not(), DataType::Boolean)
                }
                ("not", Val::Expr(e, DataType::Boolean)) => Val::Expr(e.not(), DataType::Boolean),
                ("-", Val::Int(v)) => Val::Int(-v),
                ("-", Val::Float(v)) => Val::Float(-v),
                (_, Val::Col(n, d)) if op == "-" && integer(&d) => {
                    let dec = DataType::Decimal(38, 0);
                    Val::Expr(-literal_col(&n).cast(dec.clone()), dec)
                }
                (_, Val::Expr(e, d)) if op == "-" && integer(&d) => {
                    let dec = DataType::Decimal(38, 0);
                    Val::Expr(-e.cast(dec.clone()), dec)
                }
                (_, Val::Col(n, d)) if op == "-" && numeric(&d) => Val::Expr(-literal_col(&n), d),
                (_, Val::Expr(e, d)) if op == "-" && numeric(&d) => Val::Expr(-e, d),
                _ => return Err(parse_err("unary operator has an incompatible operand")),
            }
        }
        Node::Binary(op, a, b) => {
            let l = resolve(a, s)?;
            let r = resolve(b, s)?;
            if op == "and" || op == "or" {
                if !is_bool(&l) || !is_bool(&r) {
                    return Err(parse_err("and/or require Boolean operands"));
                }
                let le = as_expr(l);
                let re = as_expr(r);
                Val::Expr(
                    if op == "and" { le.and(re) } else { le.or(re) },
                    DataType::Boolean,
                )
            } else if matches!(op.as_str(), "+" | "-" | "*" | "/") {
                arithmetic(op, l, r)?
            } else {
                compare(op, l, r)?
            }
        }
    })
}
fn is_bool(v: &Val) -> bool {
    matches!(
        v,
        Val::Bool(_) | Val::Col(_, DataType::Boolean) | Val::Expr(_, DataType::Boolean)
    )
}
fn as_expr(v: Val) -> Expr {
    match v {
        Val::Col(n, _) => literal_col(&n),
        Val::Str(s) => lit(s),
        Val::Int(n) => lit(n as i64),
        Val::Float(n) => lit(n),
        Val::Bool(b) => lit(b),
        Val::Expr(e, _) => e,
        Val::Null => lit(NULL),
    }
}
fn compare(op: &str, l: Val, r: Val) -> Result<Val, QuiltError> {
    use Val::*;
    let (e1, e2) = match (&l, &r) {
        (Col(n, d), Int(v)) | (Int(v), Col(n, d)) if integer(d) => {
            let value = if signed(d) {
                i64::try_from(*v)
                    .map_err(|_| parse_err("integer literal is out of range for signed column"))?
                    as i128
            } else {
                if *v < 0 || *v > u64::MAX as i128 {
                    return Err(parse_err(
                        "integer literal is out of range for unsigned column",
                    ));
                }
                *v
            };
            let col = if signed(d) {
                literal_col(n).cast(DataType::Int64)
            } else {
                literal_col(n).cast(DataType::UInt64)
            };
            let litval = if signed(d) {
                lit(value as i64)
            } else {
                lit(value as u64)
            };
            if matches!(l, Int(_)) {
                (litval, col)
            } else {
                (col, litval)
            }
        }
        (Col(n, d), Float(v)) | (Float(v), Col(n, d))
            if matches!(d, DataType::Float32 | DataType::Float64) =>
        {
            let col = literal_col(n).cast(DataType::Float64);
            let litval = lit(*v);
            if matches!(l, Float(_)) {
                (litval, col)
            } else {
                (col, litval)
            }
        }
        (Col(n, d), Int(v)) | (Int(v), Col(n, d))
            if matches!(d, DataType::Float32 | DataType::Float64) =>
        {
            if !exact_f64_integer(*v) {
                return Err(parse_err(
                    "integer literal is not exactly representable as Float64; use a decimal or float literal",
                ));
            }
            let col = literal_col(n).cast(DataType::Float64);
            let litval = lit(*v as f64);
            if matches!(l, Int(_)) {
                (litval, col)
            } else {
                (col, litval)
            }
        }
        (Col(n, DataType::String), Str(v)) | (Str(v), Col(n, DataType::String)) => {
            let col = literal_col(n);
            let lv = lit(v.as_str());
            if matches!(l, Str(_)) {
                (lv, col)
            } else {
                (col, lv)
            }
        }
        (Col(n, DataType::Boolean), Bool(v)) | (Bool(v), Col(n, DataType::Boolean)) => {
            let col = literal_col(n);
            let lv = lit(*v);
            if matches!(l, Bool(_)) {
                (lv, col)
            } else {
                (col, lv)
            }
        }
        (Col(n, a), Col(m, b))
            if (a == b && matches!(a, DataType::String | DataType::Boolean))
                || (integer(a) && integer(b) && signed(a) == signed(b)) =>
        {
            let dtype = if integer(a) {
                if signed(a) {
                    DataType::Int64
                } else {
                    DataType::UInt64
                }
            } else {
                a.clone()
            };
            (
                literal_col(n).cast(dtype.clone()),
                literal_col(m).cast(dtype),
            )
        }
        (Col(n, a), Col(m, b)) if integer(a) && integer(b) && signed(a) != signed(b) => {
            // Decimal(20, 0) represents every signed or unsigned 64-bit
            // integer exactly, including UInt64::MAX.
            let exact = DataType::Decimal(20, 0);
            (
                literal_col(n).cast(exact.clone()),
                literal_col(m).cast(exact),
            )
        }
        (Col(n, a), Col(m, b))
            if numeric(a)
                && numeric(b)
                && matches!(a, DataType::Float32 | DataType::Float64)
                && matches!(b, DataType::Float32 | DataType::Float64) =>
        {
            (
                literal_col(n).cast(DataType::Float64),
                literal_col(m).cast(DataType::Float64),
            )
        }
        (Expr(e, DataType::Boolean), Bool(v)) if matches!(op, "==" | "!=") => (e.clone(), lit(*v)),
        (Bool(v), Expr(e, DataType::Boolean)) if matches!(op, "==" | "!=") => (lit(*v), e.clone()),
        (Null, _) | (_, Null) => {
            return Err(parse_err(
                "NULL comparisons are ambiguous; use IS NULL or IS NOT NULL",
            ))
        }
        _ => return Err(fail_type(&dtype(&l), &dtype(&r))),
    };
    let expr = match op {
        "==" => e1.eq(e2),
        "!=" => e1.neq(e2),
        "<" => e1.lt(e2),
        "<=" => e1.lt_eq(e2),
        ">" => e1.gt(e2),
        ">=" => e1.gt_eq(e2),
        _ => unreachable!(),
    };
    Ok(Val::Expr(expr, DataType::Boolean))
}
fn dtype(v: &Val) -> DataType {
    match v {
        Val::Col(_, d) => d.clone(),
        Val::Str(_) => DataType::String,
        Val::Int(_) => DataType::Int64,
        Val::Float(_) => DataType::Float64,
        Val::Bool(_) => DataType::Boolean,
        Val::Null => DataType::Null,
        Val::Expr(_, d) => d.clone(),
    }
}
fn arithmetic(op: &str, l: Val, r: Val) -> Result<Val, QuiltError> {
    let dl = dtype(&l);
    let dr = dtype(&r);
    if !numeric(&dl) || !numeric(&dr) {
        return Err(parse_err("arithmetic operators require numeric operands"));
    }
    let both_int = integer_or_decimal_integer(&dl) && integer_or_decimal_integer(&dr);
    if both_int && op != "/" {
        if op == "*" && (contains_decimal_expr(&l) || contains_decimal_expr(&r)) {
            return Err(parse_err(
                "multiplying a derived Decimal value could exceed Decimal precision; split or narrow the expression",
            ));
        }
        if op == "*"
            && ((matches!(l, Val::Col(_, DataType::UInt64)))
                || (matches!(r, Val::Col(_, DataType::UInt64))))
        {
            return Err(parse_err(
                "multiplication involving UInt64 columns can exceed Decimal precision; cast or narrow the input explicitly",
            ));
        }
        let dec = DataType::Decimal(38, 0);
        // Integer literals inherit a neighboring column's signedness before
        // the lossless Decimal arithmetic conversion.
        let signed_context = if matches!(l, Val::Col(_, _)) {
            signed(&dl)
        } else if matches!(r, Val::Col(_, _)) {
            signed(&dr)
        } else {
            true
        };
        let a = integer_decimal_expr(&l, signed_context)?.cast(dec.clone());
        let b = integer_decimal_expr(&r, signed_context)?.cast(dec.clone());
        let e = match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            _ => unreachable!(),
        };
        Ok(Val::Expr(e, dec))
    } else {
        // Any implicit integer-to-Float64 conversion can silently round
        // values above 2^53. This DSL has no explicit cast syntax, so reject
        // integer columns and inexact integer literals on this path.
        if contains_integer_column(&l)
            || contains_integer_column(&r)
            || contains_decimal_expr(&l)
            || contains_decimal_expr(&r)
            || large_integer_literal(&l)
            || large_integer_literal(&r)
        {
            return Err(parse_err(
                "Float64 arithmetic on integer inputs may lose precision above 2^53; use cast <column> float before derive to opt into floating-point arithmetic",
            ));
        }
        let e1 = as_expr(l);
        let e2 = as_expr(r);
        let e1 = e1.cast(DataType::Float64);
        let e2 = e2.cast(DataType::Float64);
        let e = match op {
            "+" => e1 + e2,
            "-" => e1 - e2,
            "*" => e1 * e2,
            "/" => e1 / e2,
            _ => unreachable!(),
        };
        Ok(Val::Expr(e, DataType::Float64))
    }
}

fn contains_integer_column(v: &Val) -> bool {
    matches!(v, Val::Col(_, d) | Val::Expr(_, d) if integer(d))
}

fn contains_decimal_expr(v: &Val) -> bool {
    matches!(v, Val::Expr(_, DataType::Decimal(_, _)))
}

fn integer_or_decimal_integer(dtype: &DataType) -> bool {
    integer(dtype) || matches!(dtype, DataType::Decimal(_, 0))
}

fn large_integer_literal(v: &Val) -> bool {
    matches!(v, Val::Int(n) if *n > (1i128 << 53) || *n < -(1i128 << 53))
}

fn exact_f64_integer(value: i128) -> bool {
    let magnitude = value.unsigned_abs();
    if magnitude == 0 {
        return true;
    }
    let bits = u128::BITS - magnitude.leading_zeros();
    let spacing_bits = bits.saturating_sub(53);
    magnitude.trailing_zeros() >= spacing_bits
}

fn integer_decimal_expr(value: &Val, signed_context: bool) -> Result<Expr, QuiltError> {
    match value {
        Val::Int(n) if signed_context => Ok(lit(i64::try_from(*n)
            .map_err(|_| parse_err("integer literal is out of range for signed arithmetic"))?)),
        Val::Int(n) => Ok(lit(u64::try_from(*n).map_err(|_| {
            parse_err("integer literal is out of range for unsigned arithmetic")
        })?)),
        Val::Expr(expr, DataType::Decimal(_, 0)) => Ok(expr.clone()),
        _ => Ok(as_expr(value.clone())),
    }
}

fn parsed(df: &LazyFrame, expression: &str) -> Result<(Expr, DataType), QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("filter", None::<String>, e.to_string()))?;
    let ast = Parser {
        t: lex(expression)?,
        i: 0,
    }
    .parse()?;
    let value = resolve(&ast, &schema)?;
    let typ = dtype(&value);
    Ok((as_expr(value), typ))
}

/// Validate expression spelling and grammar without requiring a dataframe schema.
pub fn validate_syntax(expression: &str) -> Result<(), QuiltError> {
    Parser {
        t: lex(expression)?,
        i: 0,
    }
    .parse()
    .map(|_| ())
}

pub fn filter(df: &LazyFrame, expression: &str) -> Result<LazyFrame, QuiltError> {
    let (expr, dtype) = parsed(df, expression)?;
    if dtype != DataType::Boolean {
        return Err(QuiltError::usage(format!(
            "filter expression must produce Boolean, found {dtype}"
        )));
    }
    Ok(df.clone().filter(expr))
}

pub fn derive(df: &LazyFrame, output: &str, expression: &str) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("derive", None::<String>, e.to_string()))?;
    if schema.get(output).is_some() {
        return Err(QuiltError::schema(
            "derive",
            Some(output),
            "output column already exists",
        ));
    }
    let ast = Parser {
        t: lex(expression)?,
        i: 0,
    }
    .parse()?;
    let v = resolve(&ast, &schema)?;
    let typ = dtype(&v);
    if !numeric(&typ) {
        return Err(QuiltError::usage(format!(
            "derive expression must produce a numeric value, found {typ}"
        )));
    }
    if let Val::Int(n) = &v {
        i64::try_from(*n).map_err(|_| {
            QuiltError::usage("a derived integer literal must fit Int64; use a typed source column")
        })?;
    }
    let e = as_expr(v);
    // Integer-only arithmetic stays exact through Decimal intermediates and
    // then uses a checked Int64 result, so overflow is reported at collection.
    let e = if matches!(typ, DataType::Decimal(_, _)) {
        e.strict_cast(DataType::Int64)
    } else {
        e
    };
    Ok(df.clone().with_columns([e.alias(output)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_null_equality() {
        let ast = Parser {
            t: lex("x == null").unwrap(),
            i: 0,
        }
        .parse()
        .unwrap();
        let schema = Schema::from_iter([("x".into(), DataType::Int64)]);
        assert!(resolve(&ast, &schema).is_err());
    }

    #[test]
    fn escaped_quotes_and_mixed_integer_comparison_are_typed() {
        assert!(matches!(
            lex("label == 'it\\'s' ").unwrap()[2],
            Tok::Str(ref value) if value == "it's"
        ));
        let schema = Schema::from_iter([
            ("signed".into(), DataType::Int64),
            ("unsigned".into(), DataType::UInt64),
        ]);
        let ast = Parser {
            t: lex("signed < unsigned").unwrap(),
            i: 0,
        }
        .parse()
        .unwrap();
        assert!(matches!(
            resolve(&ast, &schema),
            Ok(Val::Expr(_, DataType::Boolean))
        ));
    }

    #[test]
    fn quoted_strings_and_reserved_columns_keep_their_contents() {
        for (source, expected) in [
            (r#"x == '\日'"#, "日"),
            (r#"x == '\😀'"#, "😀"),
            (r#"x == 'a\\b'"#, "a\\b"),
            (r#"x == '\n\t\r'"#, "\n\t\r"),
        ] {
            let tokens = lex(source).unwrap();
            assert!(matches!(&tokens[2], Tok::Str(value) if value == expected));
        }
        assert!(lex("x == 'unfinished\\").is_err());

        for name in ["true", "false", "null", "not"] {
            let frame =
                DataFrame::new_infer_height(vec![Series::new(name.into(), &[1i64, 2]).into()])
                    .unwrap();
            let filtered = filter(&frame.lazy(), &format!("`{name}` == 2"))
                .unwrap()
                .collect()
                .unwrap();
            assert_eq!(filtered.height(), 1, "quoted column {name}");
            assert_eq!(filtered[name].i64().unwrap().get(0), Some(2));
        }
    }

    #[test]
    fn float_comparisons_accept_exact_integer_literals_only() {
        let schema = Schema::from_iter([("count".into(), DataType::UInt64)]);
        let ast = Parser {
            t: lex("count / 2.0").unwrap(),
            i: 0,
        }
        .parse()
        .unwrap();
        assert!(resolve(&ast, &schema).is_err());

        assert!(exact_f64_integer(1i128 << 54));
        assert!(!exact_f64_integer((1i128 << 53) + 1));

        let frame = DataFrame::new_infer_height(vec![Series::new(
            "latency".into(),
            [999.0f64, 1000.0, 1001.0],
        )
        .into_column()])
        .unwrap()
        .lazy();
        let matched = filter(&frame, "latency >= 1000")
            .unwrap()
            .collect()
            .unwrap();
        assert_eq!(matched.height(), 2);
        assert!(filter(&frame, "latency == 9007199254740993").is_err());
        assert!(filter(&frame, "latency == 18014398509481984").is_ok());
    }

    #[test]
    fn integer_derivations_keep_decimal_chain_and_bound_unsigned_products() {
        let schema = Schema::from_iter([
            ("small".into(), DataType::Int64),
            ("wide".into(), DataType::UInt64),
        ]);
        let chain = Parser {
            t: lex("small * 2 + 1").unwrap(),
            i: 0,
        }
        .parse()
        .unwrap();
        assert!(matches!(
            resolve(&chain, &schema),
            Ok(Val::Expr(_, DataType::Decimal(_, 0)))
        ));

        let wide_product = Parser {
            t: lex("wide * 2").unwrap(),
            i: 0,
        }
        .parse()
        .unwrap();
        assert!(resolve(&wide_product, &schema).is_err());
    }

    #[test]
    fn rejects_pathologically_deep_expressions_before_recursion() {
        let input = format!("{}true", "!".repeat(MAX_EXPRESSION_TOKENS));
        assert!(lex(&input).is_err());
    }
}
