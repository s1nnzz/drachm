use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Num, Zero};

use crate::ast::NumLit;
use crate::consts::Const;
use crate::diag::{codes, Diagnostic, Span};
use crate::types::{Type, TypeError};
use crate::units;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A number literal, and the text of the unit written right after it
    /// (`9.6<s>` gives `"s"`) with the span of that text.
    Num(NumLit, Option<(String, Span)>),
    /// A string literal of any form, in pieces.
    Str(Vec<StrPiece>),
    Ident(String),
    Kw(&'static str),
    /// Punctuation or an operator.
    Sym(&'static str),
    Eof,
}

/// Every reserved word (SPEC section 1). `to` is not here: it only means
/// something directly after `skip`. `unit` is not here either: it is a
/// context word that declares only at the start of a top-level item.
pub const KEYWORDS: &[&str] = &[
    "otherwise", "any", "failure", "because", "exit", "extends", "Self", "self", "none", "mut", "if", "else",
    "while", "for", "of", "match", "case", "return", "skip", "struct", "enum", "interface", "with", "static",
    "public", "private", "new", "in", "out", "from", "as", "fails", "fail", "maybe", "copy", "lives", "open",
    "break", "continue", "take", "lend", "destroy", "unsafe", "async", "await", "move", "test", "true", "false",
];

/// Longest first, so that `**=` wins over `**` and `*`.
const SYMBOLS: &[&str] = &[
    "**=", "<<=", ">>=", "::", "->", "=>", "..", "**", "++", "--", "==", "!=", "<=", ">=", "&&", "||", "<<",
    ">>", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "(", ")", "{", "}", "[", "]", ",", ";", ":", ".",
    "?", "+", "-", "*", "/", "%", "=", "<", ">", "!", "&", "|", "^", "~",
];

impl Tok {
    /// How the token is named in error messages.
    pub fn describe(&self) -> String {
        match self {
            Tok::Num(..) => "a number".into(),
            Tok::Str(_) => "a string".into(),
            Tok::Ident(s) => format!("name `{s}`"),
            Tok::Kw(s) => format!("keyword `{s}`"),
            Tok::Sym(s) => format!("`{s}`"),
            Tok::Eof => "end of file".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// One piece of a string literal. Text written directly and text written as
/// escapes are kept apart, so that `io::outf` can tell a `%` placeholder from
/// an escaped `\%`, and point at the exact place of a problem.
#[derive(Debug, Clone, PartialEq)]
pub enum StrPiece {
    /// Source text exactly as written (it contains no escapes), at `span`.
    Raw(String, Span),
    /// The character an escape stands for; `span` covers the escape.
    Escape(char, Span),
    /// `${...}`: the tokens between the braces, then the closing `}` and
    /// `Eof`; `span` covers `${` to `}`.
    Interp(Vec<Token>, Span),
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    i: usize,
    diags: &'a mut Vec<Diagnostic>,
}

/// Never fails: bad input is reported to `diags` and skipped, so the token
/// stream always ends in `Eof`.
pub fn lex(src: &str, diags: &mut Vec<Diagnostic>) -> Vec<Token> {
    let mut lx = Lexer { src, bytes: src.as_bytes(), i: 0, diags };
    let mut out = Vec::new();
    while let Some((tok, start)) = lx.next() {
        out.push(Token { tok, span: Span::new(start, lx.i) });
    }
    out.push(Token { tok: Tok::Eof, span: Span::new(src.len(), src.len()) });
    out
}

impl Lexer<'_> {
    fn at(&self, offset: usize) -> u8 {
        self.bytes.get(self.i + offset).copied().unwrap_or(0)
    }

    fn error(&mut self, code: &'static codes::Code, start: usize, message: impl Into<String>) -> &mut Diagnostic {
        self.diags.push(Diagnostic::at(code, Span::new(start, self.i.max(start + 1)), message));
        self.diags.last_mut().unwrap()
    }

    fn next(&mut self) -> Option<(Tok, usize)> {
        loop {
            let start = self.i;
            let c = *self.bytes.get(self.i)?;

            if c.is_ascii_whitespace() {
                self.i += 1;
            } else if c == b'/' && self.at(1) == b'/' {
                while self.i < self.bytes.len() && self.bytes[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if c == b'/' && self.at(1) == b'*' {
                // Block comments do not nest.
                match self.src[start + 2..].find("*/") {
                    Some(end) => self.i = start + 2 + end + 2,
                    None => {
                        self.i = start + 2;
                        self.error(&codes::E0004, start, "this `/*` comment is never closed")
                            .set_help("end it with `*/`");
                        self.i = self.bytes.len();
                    }
                }
            } else if c.is_ascii_digit() {
                let tok = self.number();
                return Some((self.with_unit(tok), start));
            } else if c.is_ascii_alphabetic() || c == b'_' {
                while is_word(self.at(0)) {
                    self.i += 1;
                }
                let word = &self.src[start..self.i];
                if word == "f" && self.at(0) == b'"' {
                    return Some((self.string(start, true), start));
                }
                if let Some(tok) = self.word_base(word) {
                    return Some((self.with_unit(tok), start));
                }
                let tok = match KEYWORDS.iter().find(|k| **k == word) {
                    Some(k) => Tok::Kw(k),
                    None => Tok::Ident(word.to_string()),
                };
                return Some((tok, start));
            } else if c == b'"' {
                return Some((self.string(start, false), start));
            } else if c == b'`' {
                return Some((self.string(start, true), start));
            } else if let Some(sym) = SYMBOLS.iter().find(|s| self.src[start..].starts_with(**s)) {
                self.i += sym.len();
                return Some((Tok::Sym(sym), start));
            } else {
                let ch = self.src[start..].chars().next().unwrap();
                self.i += ch.len_utf8();
                self.error(&codes::E0001, start, format!("unexpected character `{}`", ch.escape_debug()));
            }
        }
    }

    /// `hex FF`, `bin 1010`, `oct 17`: a base word directly before digits of
    /// that base. Anything else leaves the word an ordinary name.
    fn word_base(&mut self, word: &str) -> Option<Tok> {
        let radix = match word {
            "hex" => 16,
            "bin" => 2,
            "oct" => 8,
            _ => return None,
        };
        let mut j = self.i;
        while matches!(self.bytes.get(j), Some(b' ' | b'\t')) {
            j += 1;
        }
        if j == self.i {
            return None;
        }
        let digits_start = j;
        while self.bytes.get(j).is_some_and(|b| (*b as char).is_digit(radix) || *b == b'_') {
            j += 1;
        }
        let digits_end = j;
        // An integer suffix may follow; anything else wordy means this was not a number.
        if matches!(self.bytes.get(j), Some(b'u' | b's')) {
            j += 1;
            while self.bytes.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
        }
        let has_digit = self.src[digits_start..digits_end].bytes().any(|b| b != b'_');
        if !has_digit || self.bytes.get(j).is_some_and(|b| is_word(*b)) {
            return None;
        }
        self.i = digits_end;
        Some(self.finish_int(digits_start, digits_start, radix))
    }

    /// The unit-literal rule (step 4.5, 2.4): a number literal directly
    /// followed by `<` with no space, then unit text up to the first `>`.
    /// Only the text's characters `A-Z a-z 0-9 _ / * ^ - ( )` are allowed,
    /// and a space only right after the word `delta`. If the text is not a
    /// valid unit expression the `<` is an ordinary less-than and the
    /// number is left as it was.
    fn with_unit(&mut self, tok: Tok) -> Tok {
        let Tok::Num(lit, _) = tok else { return tok };
        let plain = Tok::Num(lit.clone(), None);
        if self.at(0) != b'<' {
            return plain;
        }
        let from = self.i + 1;
        let mut j = from;
        loop {
            let Some(&b) = self.bytes.get(j) else { return plain };
            if b == b'>' {
                break;
            }
            let allowed = is_word(b) || matches!(b, b'/' | b'*' | b'^' | b'-' | b'(' | b')');
            if !allowed && !(b == b' ' && &self.src[from..j] == "delta") {
                return plain;
            }
            j += 1;
        }
        let text = &self.src[from..j];
        if !units::is_valid_unit_text(text) {
            return plain;
        }
        let unit = Some((text.to_string(), Span::new(from, j)));
        self.i = j + 1;
        Tok::Num(lit, unit)
    }

    fn number(&mut self) -> Tok {
        let start = self.i;
        let radix = match (self.at(0), self.at(1)) {
            (b'0', b'x') => 16,
            (b'0', b'b') => 2,
            (b'0', b'o') => 8,
            _ => 10,
        };
        if radix != 10 {
            self.i += 2;
            let digits = self.i;
            while (self.at(0) as char).is_digit(radix) || self.at(0) == b'_' {
                self.i += 1;
            }
            return self.finish_int(start, digits, radix);
        }

        while self.at(0).is_ascii_digit() || self.at(0) == b'_' {
            self.i += 1;
        }
        let mut decimal = false;
        // `1..9` is a range, and `1.x` is not a number either.
        if self.at(0) == b'.' && self.at(1).is_ascii_digit() {
            decimal = true;
            self.i += 1;
            while self.at(0).is_ascii_digit() || self.at(0) == b'_' {
                self.i += 1;
            }
        }
        let exp_digits = if self.at(1).is_ascii_digit() { 1 } else { 2 };
        if matches!(self.at(0), b'e' | b'E')
            && (exp_digits == 1 || matches!(self.at(1), b'+' | b'-'))
            && self.at(exp_digits).is_ascii_digit()
        {
            decimal = true;
            self.i += exp_digits;
            while self.at(0).is_ascii_digit() {
                self.i += 1;
            }
        }
        if !decimal {
            return self.finish_int(start, start, 10);
        }

        let text: String = self.src[start..self.i].chars().filter(|&c| c != '_').collect();
        let value = parse_decimal(&text).unwrap_or_else(|| {
            self.error(&codes::E0002, start, "this number's exponent is too large");
            BigRational::zero()
        });
        let suffix = self.suffix(start, true, true);
        Tok::Num(NumLit { value: Const::Dec(value), suffix, unit: None }, None)
    }

    /// Finish an integer literal whose digits run from `digits` to here.
    fn finish_int(&mut self, start: usize, digits: usize, radix: u32) -> Tok {
        let text: String = self.src[digits..self.i].chars().filter(|&c| c != '_').collect();
        let value = BigInt::from_str_radix(&text, radix).unwrap_or_else(|_| {
            self.error(&codes::E0002, start, "this number has no digits");
            BigInt::zero()
        });
        let suffix = self.suffix(start, radix == 10, false);
        // `5f` and `5f32` are floats written without a decimal point.
        let value = match suffix {
            Some(Type::Float(_)) => Const::Dec(BigRational::from_integer(value)),
            _ => Const::Int(value),
        };
        Tok::Num(NumLit { value, suffix, unit: None }, None)
    }

    /// A type suffix directly after a number: `u8`, `s`, `f32`, ...
    fn suffix(&mut self, start: usize, allow_float: bool, decimal: bool) -> Option<Type> {
        let from = self.i;
        while is_word(self.at(0)) {
            self.i += 1;
        }
        let text = &self.src[from..self.i];
        if text.is_empty() {
            return None;
        }
        let ty = match text.as_bytes()[0] {
            b'u' | b's' => Type::from_suffix(text),
            b'f' if allow_float => Type::from_suffix(text),
            _ => Err(TypeError::Unknown),
        };
        match ty {
            Ok(Type::Int { .. }) if decimal => {
                let text = text.to_string();
                self.error(&codes::E0002, start, format!("a decimal number cannot have the whole-number suffix `{text}`"))
                    .set_help("drop the decimal point, or convert with `::to<T>()`");
                None
            }
            Ok(ty) => Some(ty),
            Err(TypeError::NotYet) => {
                let text = text.to_string();
                self.error(&codes::E0901, start, format!("the `{text}` number type is not supported yet"));
                None
            }
            Err(TypeError::Unknown) => {
                let text = text.to_string();
                self.error(&codes::E0002, start, format!("`{text}` is not a number suffix"))
                    .set_help("suffixes are `u8` to `u128`, `s8` to `s128`, `f32`, `f64`, and bare `u`, `s`, `f`");
                None
            }
        }
    }

    /// A string literal. `self.i` is at the opening quote; `start` is where
    /// the token starts (at an `f` prefix, if any). `"` and backticks end at
    /// the end of the line, `"""` may span lines. Only `f` strings and
    /// backtick strings interpolate `${...}`.
    fn string(&mut self, start: usize, interpolate: bool) -> Tok {
        let close = if self.src[self.i..].starts_with("\"\"\"") {
            "\"\"\""
        } else if self.at(0) == b'`' {
            "`"
        } else {
            "\""
        };
        // OPEN (language): a `"""` string keeps its text exactly as written
        // between the delimiters, leading newline and indentation included.
        let multiline = close.len() == 3;
        self.i += close.len();
        let mut pieces = Vec::new();
        let mut raw = self.i;
        let mut interp_failed = false;
        loop {
            let rest = &self.src[self.i..];
            if rest.starts_with(close) {
                self.raw_piece(raw, &mut pieces);
                self.i += close.len();
                break;
            }
            let Some(ch) = rest.chars().next().filter(|c| multiline || *c != '\n') else {
                self.raw_piece(raw, &mut pieces);
                // An unclosed `${` already said what is wrong.
                if interp_failed {
                    break;
                }
                let help = match close {
                    "`" => "end it with a backtick on the same line",
                    "\"" => "end it with `\"` on the same line",
                    _ => "end it with `\"\"\"`",
                };
                self.error(&codes::E0004, start, "this string is never closed").set_help(help);
                break;
            };
            match ch {
                '\\' => {
                    self.raw_piece(raw, &mut pieces);
                    if let Some(piece) = self.escape() {
                        pieces.push(piece);
                    }
                    raw = self.i;
                }
                '$' if interpolate && self.at(1) == b'{' => {
                    self.raw_piece(raw, &mut pieces);
                    match self.interpolation(multiline) {
                        Some(piece) => pieces.push(piece),
                        None => interp_failed = true,
                    }
                    raw = self.i;
                }
                _ => self.i += ch.len_utf8(),
            }
        }
        Tok::Str(pieces)
    }

    /// The text from `from` to here, as written, if there is any.
    fn raw_piece(&self, from: usize, pieces: &mut Vec<StrPiece>) {
        if from < self.i {
            pieces.push(StrPiece::Raw(self.src[from..self.i].to_string(), Span::new(from, self.i)));
        }
    }

    /// An escape; `self.i` is at its `\\`. `None` if it was invalid (reported).
    fn escape(&mut self) -> Option<StrPiece> {
        let at = self.i;
        self.i += 1;
        let esc = self.src[self.i..].chars().next()?;
        if esc == '\n' || self.src[self.i..].starts_with("\r\n") {
            // Leave the newline to end (or continue) the string.
            self.error(&codes::E0008, at, "a `\\` at the end of a line is not an escape")
                .set_help(ESCAPES_HELP);
            return None;
        }
        self.i += esc.len_utf8();
        let ch = match esc {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '0' => '\0',
            '\\' | '"' | '\'' | '`' | '$' | '%' => esc,
            'u' if self.at(0) == b'{' => {
                let close = self.src[self.i..].find(['}', '"', '`', '\n']).map(|p| self.i + p);
                let body = close.map(|c| &self.src[self.i + 1..c]).unwrap_or("");
                let code = u32::from_str_radix(body, 16).ok().and_then(char::from_u32);
                match (close, code) {
                    (Some(c), Some(ch)) if self.bytes[c] == b'}' => {
                        self.i = c + 1;
                        ch
                    }
                    _ => {
                        self.error(&codes::E0008, at, "`\\u{...}` needs a Unicode code point in hex")
                            .set_help("for example `\\u{1F600}`; names such as `\\u{GREEK SMALL LETTER ALPHA}` are not supported yet");
                        if let Some(c) = close.filter(|c| self.bytes[*c] == b'}') {
                            self.i = c + 1;
                        }
                        return None;
                    }
                }
            }
            _ => {
                self.error(&codes::E0008, at, format!("unknown escape `\\{}`", esc.escape_debug())).set_help(ESCAPES_HELP);
                return None;
            }
        };
        Some(StrPiece::Escape(ch, Span::new(at, self.i)))
    }

    /// `${...}`; `self.i` is at the `$`. The expression inside is lexed as
    /// ordinary tokens up to the matching `}`, so it may hold anything,
    /// strings included. In a single-line string it must close on that line.
    fn interpolation(&mut self, multiline: bool) -> Option<StrPiece> {
        let open = self.i;
        self.i += 2;
        let mut tokens = Vec::new();
        let mut depth = 0u32;
        loop {
            let before = self.i;
            let next = self.next();
            let gap_end = next.as_ref().map_or(self.src.len(), |(_, s)| *s);
            let newline = self.src[before..gap_end].find('\n').filter(|_| !multiline);
            let (tok, s) = match (next, newline) {
                (Some(t), None) => t,
                (_, nl) => {
                    self.i = nl.map_or(self.src.len(), |n| before + n);
                    self.diags.push(
                        Diagnostic::at(&codes::E0004, Span::new(open, open + 2), "this `${` is never closed")
                            .help("end the interpolated expression with `}`"),
                    );
                    return None;
                }
            };
            match tok {
                Tok::Sym("{") => depth += 1,
                Tok::Sym("}") if depth == 0 => {
                    tokens.push(Token { tok, span: Span::new(s, self.i) });
                    tokens.push(Token { tok: Tok::Eof, span: Span::new(self.i, self.i) });
                    return Some(StrPiece::Interp(tokens, Span::new(open, self.i)));
                }
                Tok::Sym("}") => depth -= 1,
                _ => {}
            }
            tokens.push(Token { tok, span: Span::new(s, self.i) });
        }
    }
}

const ESCAPES_HELP: &str = "escapes are \\n \\t \\r \\0 \\\\ \\\" \\' \\` \\$ \\% and \\u{...}";

/// Exact value of `digits[.digits][e[+-]digits]`. `None` if the exponent is silly.
fn parse_decimal(text: &str) -> Option<BigRational> {
    let (mantissa, exp) = match text.find(['e', 'E']) {
        Some(p) => (&text[..p], text[p + 1..].parse::<i64>().ok()?),
        None => (text, 0),
    };
    let (whole, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = BigInt::from_str_radix(&format!("{whole}{frac}"), 10).ok()?;
    let exp = exp - frac.len() as i64;
    if exp.abs() > 1100 {
        return None;
    }
    let scale = num_traits::pow(BigInt::from(10), exp.unsigned_abs() as usize);
    Some(if exp >= 0 {
        BigRational::from_integer(digits * scale)
    } else {
        BigRational::new(digits, scale)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(src: &str) -> Vec<String> {
        let mut diags = Vec::new();
        let toks = lex(src, &mut diags);
        assert!(diags.is_empty(), "{diags:?}");
        toks.into_iter()
            .map(|t| match t.tok {
                Tok::Num(NumLit { value: Const::Int(v), suffix, .. }, unit) => {
                    format!("{v}{}{}", suffix.map_or(String::new(), |s| format!(":{s}")), unit_text(unit))
                }
                Tok::Num(NumLit { value: Const::Dec(v), suffix, .. }, unit) => {
                    format!("d{v}{}{}", suffix.map_or(String::new(), |s| format!(":{s}")), unit_text(unit))
                }
                Tok::Ident(s) => s,
                Tok::Sym(s) | Tok::Kw(s) => s.to_string(),
                Tok::Str(pieces) => format!("{:?}", show(&pieces)),
                Tok::Eof => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn unit_text(unit: Option<(String, Span)>) -> String {
        unit.map_or(String::new(), |(u, _)| format!("<{u}>"))
    }

    /// A string token's pieces: raw text as is, escapes in `[]`, interpolations
    /// as their tokens in `${}`.
    fn show(pieces: &[StrPiece]) -> String {
        pieces
            .iter()
            .map(|p| match p {
                StrPiece::Raw(t, _) => t.clone(),
                StrPiece::Escape(c, _) => format!("[{c}]"),
                StrPiece::Interp(toks, _) => format!("${{{}}}", toks.len() - 2),
            })
            .collect()
    }

    fn string(src: &str) -> String {
        let mut diags = Vec::new();
        let toks = lex(src, &mut diags);
        assert!(diags.is_empty(), "{src}: {diags:?}");
        assert_eq!(toks.len(), 2, "{src}: {toks:?}");
        match &toks[0].tok {
            Tok::Str(p) => show(p),
            t => panic!("{src}: {t:?}"),
        }
    }

    #[test]
    fn number_forms() {
        assert_eq!(nums("1_000 0xFF 0b1010 0o17 017"), ["1000", "255", "10", "15", "17"]);
        assert_eq!(nums("5u8 5s 5u 0xFFu8 7s128"), ["5:uint8", "5:sint32", "5:uint32", "255:uint8", "7:sint128"]);
        assert_eq!(nums("2.5 2.5f32 5.0f 5f 1e3 2.5e-3"), ["d5/2", "d5/2:float32", "d5:float64", "d5:float64", "d1000", "d1/400"]);
        assert_eq!(nums("hex FF bin 1010 oct 17 hex ffu8"), ["255", "10", "15", "255:uint8"]);
        // Not numbers: a name called hex, a range, a method on a number.
        assert_eq!(nums("hex = hex + bin;"), ["hex", "=", "hex", "+", "bin", ";"]);
        assert_eq!(nums("hex zz"), ["hex", "zz"]);
        assert_eq!(nums("1..9"), ["1", "..", "9"]);
        assert_eq!(nums("5::text()"), ["5", "::", "text", "(", ")"]);
    }

    #[test]
    fn unit_literals() {
        assert_eq!(
            nums("9.6<s> 5u8<m> 2.5f32<m/s> 1<1/s> 3.0<delta degC> 100<kg*m/s^2>"),
            ["d48/5<s>", "5:uint8<m>", "d5/2:float32<m/s>", "1<1/s>", "d3<delta degC>", "100<kg*m/s^2>"]
        );
        assert_eq!(nums("3<m>>2"), ["3<m>", ">", "2"]);
        assert_eq!(nums("9<s>>3 1<m>>=2"), ["9<s>", ">", "3", "1<m>", ">=", "2"]);
        // Not unit literals: a space, a bad unit, or no closing `>`.
        assert_eq!(nums("3 < m 3<m* 3<4 3<x && y>2"), ["3", "<", "m", "3", "<", "m", "*", "3", "<", "4", "3", "<", "x", "&&", "y", ">", "2"]);
        assert_eq!(nums("3.0<delta  degC>"), ["d3", "<", "delta", "degC", ">"]);
    }

    #[test]
    fn strings_and_symbols() {
        assert_eq!(nums(r#""a\n\"b\u{41}" **= ** * /* c */ // d"#), ["\"a[\\n][\\\"]b[A]\"", "**=", "**", "*"]);
        assert_eq!(nums("i++ + --j"), ["i", "++", "+", "--", "j"]);
    }

    #[test]
    fn string_forms() {
        assert_eq!(string(r#""100\% ${x} {}""#), "100[%] ${x} {}");
        assert_eq!(string(r#"f"a${x + 1}b${ {} }""#), "a${3}b${2}");
        assert_eq!(string("`say \"${ f\"${n}\" }\"`"), "say \"${1}\"");
        assert_eq!(string("\"\"\"line\n  \"quoted\"\\t\"\"\""), "line\n  \"quoted\"[\t]");
        assert_eq!(string("f\"\"\"\n${\nx\n}\"\"\""), "\n${1}");
        assert_eq!(string(r#""""#), "");
        assert_eq!(string(r#""\$\`""#), "[$][`]");
        // A name `f` before a space is not a prefix.
        assert_eq!(nums(r#"f "x""#), ["f", "\"x\""]);
    }

    #[test]
    fn errors() {
        for bad in ["5x", "5u7", "2.5u8", "\"abc", "/* x", "@", "\"\\q\"", "5u256", "f\"${x", "`abc", "\"\"\"abc"] {
            let mut diags = Vec::new();
            lex(bad, &mut diags);
            assert_eq!(diags.len(), 1, "{bad}: {diags:?}");
        }
    }

    /// A `\` before a Windows line ending is the end-of-line error, not `\r`.
    #[test]
    fn backslash_at_crlf() {
        for src in ["\"a \\\n\"", "\"a \\\r\n\""] {
            let mut diags = Vec::new();
            lex(src, &mut diags);
            assert_eq!(diags[0].code.id, "E0008", "{src:?}: {diags:?}");
            assert!(diags[0].message.contains("end of a line"), "{src:?}: {diags:?}");
        }
    }
}
