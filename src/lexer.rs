use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Num, Zero};

use crate::ast::NumLit;
use crate::consts::Const;
use crate::diag::{codes, Diagnostic, Span};
use crate::types::{Type, TypeError};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(NumLit),
    Str(String),
    Ident(String),
    Kw(&'static str),
    /// Punctuation or an operator.
    Sym(&'static str),
    Eof,
}

/// Every reserved word (SPEC section 1). `to` is not here: it only means
/// something directly after `skip`.
pub const KEYWORDS: &[&str] = &[
    "otherwise", "any", "failure", "because", "exit", "extends", "Self", "self", "none", "mut", "if", "else",
    "while", "for", "of", "match", "case", "return", "skip", "struct", "enum", "interface", "with", "static",
    "public", "private", "new", "in", "out", "from", "as", "fails", "fail", "maybe", "copy", "lives", "open",
    "break", "continue", "unsafe", "async", "await", "move", "test", "unit", "true", "false",
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
            Tok::Num(_) => "a number".into(),
            Tok::Str(_) => "a string".into(),
            Tok::Ident(s) => format!("name `{s}`"),
            Tok::Kw(s) => format!("keyword `{s}`"),
            Tok::Sym(s) => format!("`{s}`"),
            Tok::Eof => "end of file".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
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
                return Some((self.number(), start));
            } else if c.is_ascii_alphabetic() || c == b'_' {
                while is_word(self.at(0)) {
                    self.i += 1;
                }
                let word = &self.src[start..self.i];
                if let Some(tok) = self.word_base(word) {
                    return Some((tok, start));
                }
                let tok = match KEYWORDS.iter().find(|k| **k == word) {
                    Some(k) => Tok::Kw(k),
                    None => Tok::Ident(word.to_string()),
                };
                return Some((tok, start));
            } else if c == b'"' {
                return Some((self.string(), start));
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
        Tok::Num(NumLit { value: Const::Dec(value), suffix })
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
        Tok::Num(NumLit { value, suffix })
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

    fn string(&mut self) -> Tok {
        let start = self.i;
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(ch) = self.src[self.i..].chars().next().filter(|c| *c != '\n') else {
                self.error(&codes::E0004, start, "this string is never closed")
                    .set_help("end it with `\"` on the same line");
                break;
            };
            let at = self.i;
            self.i += ch.len_utf8();
            match ch {
                '"' => break,
                '\\' => {
                    let Some(esc) = self.src[self.i..].chars().next() else { continue };
                    self.i += esc.len_utf8();
                    match esc {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        '0' => out.push('\0'),
                        '\\' | '"' | '\'' | '`' | '$' => out.push(esc),
                        'u' if self.at(0) == b'{' => {
                            let close = self.src[self.i..].find(['}', '"', '\n']).map(|p| self.i + p);
                            let body = close.map(|c| &self.src[self.i + 1..c]).unwrap_or("");
                            let code = u32::from_str_radix(body, 16).ok().and_then(char::from_u32);
                            match (close, code) {
                                (Some(c), Some(ch)) if self.bytes[c] == b'}' => {
                                    out.push(ch);
                                    self.i = c + 1;
                                }
                                _ => {
                                    self.error(&codes::E0008, at, "`\\u{...}` needs a Unicode code point in hex")
                                        .set_help("for example `\\u{1F600}`; names such as `\\u{GREEK SMALL LETTER ALPHA}` are not supported yet");
                                    if let Some(c) = close.filter(|c| self.bytes[*c] == b'}') {
                                        self.i = c + 1;
                                    }
                                }
                            }
                        }
                        _ => {
                            self.error(&codes::E0008, at, format!("unknown escape `\\{}`", esc.escape_debug()))
                                .set_help("escapes are \\n \\t \\r \\0 \\\\ \\\" \\' \\` \\$ and \\u{...}");
                        }
                    }
                }
                _ => out.push(ch),
            }
        }
        Tok::Str(out)
    }
}

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
                Tok::Num(NumLit { value: Const::Int(v), suffix }) => format!("{v}{}", suffix.map_or(String::new(), |s| format!(":{s}"))),
                Tok::Num(NumLit { value: Const::Dec(v), suffix }) => format!("d{v}{}", suffix.map_or(String::new(), |s| format!(":{s}"))),
                Tok::Ident(s) => s,
                Tok::Sym(s) | Tok::Kw(s) => s.to_string(),
                Tok::Str(s) => format!("{s:?}"),
                Tok::Eof => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect()
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
    fn strings_and_symbols() {
        assert_eq!(nums(r#""a\n\"b\u{41}" **= ** * /* c */ // d"#), ["\"a\\n\\\"bA\"", "**=", "**", "*"]);
        assert_eq!(nums("i++ + --j"), ["i", "++", "+", "--", "j"]);
    }

    #[test]
    fn errors() {
        for bad in ["5x", "5u7", "2.5u8", "\"abc", "/* x", "@", "\"\\q\"", "5u256"] {
            let mut diags = Vec::new();
            lex(bad, &mut diags);
            assert_eq!(diags.len(), 1, "{bad}: {diags:?}");
        }
    }
}
