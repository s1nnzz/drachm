//! JSON for `convert::toJson` and `convert::fromJson<T>`.
//!
//! The compiler describes the type to convert in a text shape that it builds from the
//! field names and the layout it uses for them, so this code reads and writes a value in
//! memory without knowing the type:
//!
//! ```text
//! shape  = "iu32" | "is8" | ...     whole numbers: signedness and width
//!        | "f32" | "f64"            floats
//!        | "b"                      bool
//!        | "t"                      string
//!        | "o" SIZE "." OFF "(" shape ")"        optional: tag byte at 0, payload at OFF
//!        | "l" SIZE "." ALIGN "(" shape ")"      list of items of that size and alignment
//!        | "r" SIZE "{" (NAME "@" OFF "=" shape ",")* "}"   struct, fields in order
//! ```
//!
//! Reading checks the whole text against the shape before anything is built, so a
//! failure leaves nothing to free: the destination is zeroed (a valid empty value of every
//! shape) and the problem is described in `kind`, `path` and `detail`.

use crate::lists::{DrachmList, EMPTY_LIST, drachm_list_push};
use crate::{DrachmStr, bytes, drachm_str_drop, emit};

// ---- shapes -------------------------------------------------------------------

enum Shape {
    Int { bits: u32, signed: bool },
    Float(u32),
    Bool,
    Text,
    Opt { size: usize, off: usize, inner: Box<Shape> },
    List { size: usize, align: usize, elem: Box<Shape> },
    Struct { size: usize, fields: Vec<Field> },
}

struct Field {
    name: String,
    off: usize,
    shape: Shape,
}

impl Shape {
    /// Bytes the value takes in memory.
    fn size(&self) -> usize {
        match self {
            Shape::Int { bits, .. } | Shape::Float(bits) => *bits as usize / 8,
            Shape::Bool => 1,
            Shape::Text | Shape::List { .. } => 24,
            Shape::Opt { size, .. } | Shape::Struct { size, .. } => *size,
        }
    }

    fn is_optional(&self) -> bool {
        matches!(self, Shape::Opt { .. })
    }

    /// What a value of this shape reads as in text, for "should be ...".
    fn expects(&self) -> &'static str {
        match self {
            Shape::Int { .. } => "a whole number",
            Shape::Float(_) => "a number",
            Shape::Bool => "true or false",
            Shape::Text => "text",
            Shape::Opt { inner, .. } => inner.expects(),
            Shape::List { .. } => "a list",
            Shape::Struct { .. } => "an object",
        }
    }
}

struct ShapeReader<'a> {
    s: &'a [u8],
    at: usize,
}

impl ShapeReader<'_> {
    fn next(&mut self) -> u8 {
        let c = *self.s.get(self.at).expect("the shape is complete");
        self.at += 1;
        c
    }

    fn number(&mut self) -> usize {
        let mut n = 0;
        while let Some(&c) = self.s.get(self.at) {
            if !c.is_ascii_digit() {
                break;
            }
            n = n * 10 + (c - b'0') as usize;
            self.at += 1;
        }
        n
    }

    fn expect(&mut self, c: u8) {
        assert_eq!(self.next(), c, "the shape is well formed");
    }

    fn shape(&mut self) -> Shape {
        match self.next() {
            b'i' => {
                let signed = self.next() == b's';
                Shape::Int { bits: self.number() as u32, signed }
            }
            b'f' => Shape::Float(self.number() as u32),
            b'b' => Shape::Bool,
            b't' => Shape::Text,
            b'o' => {
                let size = self.number();
                self.expect(b'.');
                let off = self.number();
                self.expect(b'(');
                let inner = Box::new(self.shape());
                self.expect(b')');
                Shape::Opt { size, off, inner }
            }
            b'l' => {
                let size = self.number();
                self.expect(b'.');
                let align = self.number();
                self.expect(b'(');
                let elem = Box::new(self.shape());
                self.expect(b')');
                Shape::List { size, align, elem }
            }
            b'r' => {
                let size = self.number();
                self.expect(b'{');
                let mut fields = Vec::new();
                while self.s[self.at] != b'}' {
                    let start = self.at;
                    while self.s[self.at] != b'@' {
                        self.at += 1;
                    }
                    let name = String::from_utf8_lossy(&self.s[start..self.at]).into_owned();
                    self.expect(b'@');
                    let off = self.number();
                    self.expect(b'=');
                    let shape = self.shape();
                    self.expect(b',');
                    fields.push(Field { name, off, shape });
                }
                self.expect(b'}');
                Shape::Struct { size, fields }
            }
            other => panic!("bad shape letter {}", other as char),
        }
    }
}

fn parse_shape(shape: *const DrachmStr) -> Shape {
    let mut r = ShapeReader { s: unsafe { bytes(shape) }, at: 0 };
    r.shape()
}

// ---- writing ----------------------------------------------------------------------

unsafe fn read_uint(p: *const u8, bits: u32) -> u128 {
    unsafe {
        match bits {
            8 => p.read_unaligned() as u128,
            16 => (p as *const u16).read_unaligned() as u128,
            32 => (p as *const u32).read_unaligned() as u128,
            64 => (p as *const u64).read_unaligned() as u128,
            _ => (p as *const u128).read_unaligned(),
        }
    }
}

fn sign_extend(v: u128, bits: u32) -> i128 {
    if bits == 128 { v as i128 } else { ((v << (128 - bits)) as i128) >> (128 - bits) }
}

fn quote(out: &mut Vec<u8>, text: &[u8]) {
    out.push(b'"');
    for &b in text {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            0..=0x1f | 0x7f => out.extend_from_slice(format!("\\u{:04x}", b).as_bytes()),
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

unsafe fn write_value(shape: &Shape, p: *const u8, out: &mut Vec<u8>) {
    unsafe {
        match shape {
            Shape::Int { bits, signed } => {
                let v = read_uint(p, *bits);
                let text = if *signed { sign_extend(v, *bits).to_string() } else { v.to_string() };
                out.extend_from_slice(text.as_bytes());
            }
            Shape::Float(32) => {
                let v = (p as *const f32).read_unaligned();
                if v.is_finite() { out.extend_from_slice(format!("{v:?}").as_bytes()) } else { out.extend_from_slice(b"null") }
            }
            Shape::Float(_) => {
                let v = (p as *const f64).read_unaligned();
                if v.is_finite() { out.extend_from_slice(format!("{v:?}").as_bytes()) } else { out.extend_from_slice(b"null") }
            }
            Shape::Bool => out.extend_from_slice(if p.read() != 0 { b"true" } else { b"false" }),
            Shape::Text => quote(out, bytes(p as *const DrachmStr)),
            Shape::Opt { off, inner, .. } => {
                if p.read() == 0 {
                    out.extend_from_slice(b"null");
                } else {
                    write_value(inner, p.add(*off), out);
                }
            }
            Shape::List { size, elem, .. } => {
                let list = (p as *const DrachmList).read_unaligned();
                out.push(b'[');
                for i in 0..list.len as usize {
                    if i > 0 {
                        out.push(b',');
                    }
                    write_value(elem, list.ptr.add(i * size), out);
                }
                out.push(b']');
            }
            Shape::Struct { fields, .. } => {
                out.push(b'{');
                for (i, f) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    quote(out, f.name.as_bytes());
                    out.push(b':');
                    write_value(&f.shape, p.add(f.off), out);
                }
                out.push(b'}');
            }
        }
    }
}

/// `convert::toJson`: the JSON text of the value at `value`, described by `shape`.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_json_write(out: *mut DrachmStr, value: *const u8, shape: *const DrachmStr) {
    let shape = parse_shape(shape);
    let mut text = Vec::new();
    unsafe { write_value(&shape, value, &mut text) };
    unsafe { emit(out, &text) };
}

// ---- reading ------------------------------------------------------------------------

enum J {
    Null,
    Bool(bool),
    Num(String),
    Str(Vec<u8>),
    Arr(Vec<J>),
    Obj(Vec<(Vec<u8>, J)>),
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

/// A problem with the text or the shape: `kind` is `S` (syntax), `T` (wrong type), `M`
/// (missing field), `U` (unknown field) or `R` (out of range).
struct Problem {
    kind: u8,
    path: String,
    detail: String,
}

const MAX_DEPTH: usize = 256;

impl Parser<'_> {
    fn err<T>(&self, what: &str) -> Result<T, Problem> {
        let upto = &self.s[..self.at.min(self.s.len())];
        let line = upto.iter().filter(|&&b| b == b'\n').count() + 1;
        let col = upto.iter().rev().take_while(|&&b| b != b'\n').count() + 1;
        Err(Problem { kind: b'S', path: String::new(), detail: format!("{what} at line {line}, column {col}") })
    }

    fn ws(&mut self) {
        while matches!(self.s.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, word: &[u8]) -> bool {
        if self.s[self.at..].starts_with(word) {
            self.at += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<J, Problem> {
        if depth > MAX_DEPTH {
            return self.err("nested too deeply");
        }
        self.ws();
        match self.s.get(self.at) {
            None => self.err("the text ends before a value"),
            Some(b'n') => if self.eat(b"null") { Ok(J::Null) } else { self.err("expected a value") },
            Some(b't') => if self.eat(b"true") { Ok(J::Bool(true)) } else { self.err("expected a value") },
            Some(b'f') => if self.eat(b"false") { Ok(J::Bool(false)) } else { self.err("expected a value") },
            Some(b'"') => Ok(J::Str(self.string()?)),
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(J::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(J::Arr(items));
                        }
                        _ => return self.err("expected `,` or `]`"),
                    }
                }
            }
            Some(b'{') => {
                self.at += 1;
                let mut fields: Vec<(Vec<u8>, J)> = Vec::new();
                self.ws();
                if self.s.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(J::Obj(fields));
                }
                loop {
                    self.ws();
                    if self.s.get(self.at) != Some(&b'"') {
                        return self.err("expected a field name in quotes");
                    }
                    let key = self.string()?;
                    self.ws();
                    if self.s.get(self.at) != Some(&b':') {
                        return self.err("expected `:` after the field name");
                    }
                    self.at += 1;
                    let v = self.value(depth + 1)?;
                    if fields.iter().any(|(k, _)| *k == key) {
                        return self.err(&format!("the field `{}` is given twice", String::from_utf8_lossy(&key)));
                    }
                    fields.push((key, v));
                    self.ws();
                    match self.s.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(J::Obj(fields));
                        }
                        _ => return self.err("expected `,` or `}`"),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => self.err("expected a value"),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.s.get(self.at), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<J, Problem> {
        let start = self.at;
        if self.s[self.at] == b'-' {
            self.at += 1;
        }
        match self.s.get(self.at) {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return self.err("expected digits"),
        }
        if self.s.get(self.at) == Some(&b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return self.err("expected digits after the point");
            }
        }
        if matches!(self.s.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.s.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return self.err("expected digits in the exponent");
            }
        }
        Ok(J::Num(String::from_utf8_lossy(&self.s[start..self.at]).into_owned()))
    }

    fn hex4(&mut self) -> Result<u32, Problem> {
        let digits = self.s.get(self.at..self.at + 4).and_then(|d| std::str::from_utf8(d).ok()).filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()));
        match digits {
            Some(d) => {
                self.at += 4;
                Ok(u32::from_str_radix(d, 16).unwrap())
            }
            None => self.err("expected four hex digits after `\\u`"),
        }
    }

    fn string(&mut self) -> Result<Vec<u8>, Problem> {
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.at) else { return self.err("the text ends inside a string") };
            self.at += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(&e) = self.s.get(self.at) else { return self.err("the text ends inside a string") };
                    self.at += 1;
                    match e {
                        b'"' | b'\\' | b'/' => out.push(e),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xd800..0xdc00).contains(&code) {
                                if !self.eat(b"\\u") {
                                    return self.err("a high surrogate needs a low one after it");
                                }
                                let low = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&low) {
                                    return self.err("a high surrogate needs a low one after it");
                                }
                                code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                            } else if (0xdc00..0xe000).contains(&code) {
                                return self.err("a low surrogate without a high one");
                            }
                            let ch = char::from_u32(code).expect("a valid scalar value");
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        _ => {
                            self.at -= 1;
                            return self.err("unknown escape");
                        }
                    }
                }
                0..=0x1f => {
                    self.at -= 1;
                    return self.err("a control character inside a string");
                }
                _ => out.push(c),
            }
        }
    }
}

fn problem(kind: u8, path: &str, detail: &str) -> Problem {
    Problem { kind, path: path.to_string(), detail: detail.to_string() }
}

/// Checks `j` against `shape`; the problem names the path of the first mismatch.
fn check(shape: &Shape, j: &J, path: &str) -> Result<(), Problem> {
    let wrong = || problem(b'T', path, shape.expects());
    match (shape, j) {
        (Shape::Opt { .. }, J::Null) => Ok(()),
        (Shape::Opt { inner, .. }, other) => check(inner, other, path),
        (Shape::Int { bits, signed }, J::Num(n)) => {
            if n.contains(['.', 'e', 'E']) {
                return Err(wrong());
            }
            let fits = if *signed {
                n.parse::<i128>().is_ok_and(|v| *bits == 128 || (-(1i128 << (bits - 1))..(1i128 << (bits - 1))).contains(&v))
            } else {
                !n.starts_with('-') && n.parse::<u128>().is_ok_and(|v| *bits == 128 || v < (1u128 << bits))
            };
            if fits { Ok(()) } else { Err(problem(b'R', path, "")) }
        }
        (Shape::Float(bits), J::Num(n)) => {
            let finite = if *bits == 32 { n.parse::<f32>().is_ok_and(f32::is_finite) } else { n.parse::<f64>().is_ok_and(f64::is_finite) };
            if finite { Ok(()) } else { Err(problem(b'R', path, "")) }
        }
        (Shape::Bool, J::Bool(_)) | (Shape::Text, J::Str(_)) => Ok(()),
        (Shape::List { elem, .. }, J::Arr(items)) => {
            for (i, item) in items.iter().enumerate() {
                check(elem, item, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (Shape::Struct { fields, .. }, J::Obj(given)) => {
            for (key, _) in given {
                let name = String::from_utf8_lossy(key);
                if !fields.iter().any(|f| f.name.as_bytes() == key.as_slice()) {
                    return Err(problem(b'U', &format!("{path}.{name}"), ""));
                }
            }
            for f in fields {
                let here = format!("{path}.{}", f.name);
                match given.iter().find(|(k, _)| k.as_slice() == f.name.as_bytes()) {
                    Some((_, v)) => check(&f.shape, v, &here)?,
                    None if f.shape.is_optional() => {}
                    None => return Err(problem(b'M', &here, "")),
                }
            }
            Ok(())
        }
        _ => Err(wrong()),
    }
}

/// Builds the value `j` (already checked) at `p`, which holds `shape.size()` zero bytes.
unsafe fn build(shape: &Shape, j: &J, p: *mut u8) {
    unsafe {
        match (shape, j) {
            (Shape::Opt { .. }, J::Null) => {}
            (Shape::Opt { off, inner, .. }, other) => {
                p.write(1);
                build(inner, other, p.add(*off));
            }
            (Shape::Int { bits, signed }, J::Num(n)) => {
                let v: u128 = if *signed { n.parse::<i128>().unwrap() as u128 } else { n.parse::<u128>().unwrap() };
                match bits {
                    8 => p.write(v as u8),
                    16 => (p as *mut u16).write_unaligned(v as u16),
                    32 => (p as *mut u32).write_unaligned(v as u32),
                    64 => (p as *mut u64).write_unaligned(v as u64),
                    _ => (p as *mut u128).write_unaligned(v),
                }
            }
            (Shape::Float(32), J::Num(n)) => (p as *mut f32).write_unaligned(n.parse().unwrap()),
            (Shape::Float(_), J::Num(n)) => (p as *mut f64).write_unaligned(n.parse().unwrap()),
            (Shape::Bool, J::Bool(b)) => p.write(*b as u8),
            (Shape::Text, J::Str(s)) => emit(p as *mut DrachmStr, s),
            (Shape::List { size, align, elem }, J::Arr(items)) => {
                let list = p as *mut DrachmList;
                list.write_unaligned(EMPTY_LIST);
                for item in items {
                    let mut buf = vec![0u8; *size];
                    build(elem, item, buf.as_mut_ptr());
                    drachm_list_push(list, buf.as_ptr(), *size as u64, *align as u64);
                }
            }
            (Shape::Struct { fields, .. }, J::Obj(given)) => {
                for f in fields {
                    if let Some((_, v)) = given.iter().find(|(k, _)| k.as_slice() == f.name.as_bytes()) {
                        build(&f.shape, v, p.add(f.off));
                    }
                }
            }
            _ => unreachable!("the value was checked against the shape"),
        }
    }
}

unsafe fn set_text(out: *mut DrachmStr, text: &str) {
    unsafe {
        drachm_str_drop(out);
        emit(out, text.as_bytes());
    }
}

/// `convert::fromJson`: reads `text` as a value of `shape` into `out`. On a problem `out`
/// is zeroed and `kind`, `path` and `detail` say what went wrong (all empty when fine).
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn drachm_json_read(
    out: *mut u8,
    text: *const DrachmStr,
    kind: *mut DrachmStr,
    path: *mut DrachmStr,
    detail: *mut DrachmStr,
    shape: *const DrachmStr,
) {
    let shape = parse_shape(shape);
    unsafe { std::ptr::write_bytes(out, 0, shape.size()) };
    let mut parser = Parser { s: unsafe { bytes(text) }, at: 0 };
    let parsed = parser.value(0).and_then(|j| {
        parser.ws();
        if parser.at < parser.s.len() { parser.err("unexpected text after the value") } else { Ok(j) }
    });
    let result = parsed.and_then(|j| check(&shape, &j, "$").map(|()| j));
    match result {
        Ok(j) => unsafe { build(&shape, &j, out) },
        Err(p) => unsafe {
            set_text(kind, std::str::from_utf8(&[p.kind]).unwrap());
            set_text(path, &p.path);
            set_text(detail, &p.detail);
        },
    }
}
