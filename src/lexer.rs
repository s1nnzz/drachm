use crate::diag::{codes, Diagnostic, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Ident(String),
    // keywords
    Fn,
    Let,
    Var,
    If,
    Else,
    While,
    Return,
    True,
    False,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semi,
    Arrow,
    // operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Eof,
}

impl Tok {
    /// How the token is named in error messages.
    pub fn describe(&self) -> String {
        let text = match self {
            Tok::Int(n) => return format!("number `{n}`"),
            Tok::Ident(s) => return format!("name `{s}`"),
            Tok::Eof => return "end of file".into(),
            Tok::Fn => "fn",
            Tok::Let => "let",
            Tok::Var => "var",
            Tok::If => "if",
            Tok::Else => "else",
            Tok::While => "while",
            Tok::Return => "return",
            Tok::True => "true",
            Tok::False => "false",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::Comma => ",",
            Tok::Colon => ":",
            Tok::Semi => ";",
            Tok::Arrow => "->",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Assign => "=",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            Tok::Bang => "!",
        };
        format!("`{text}`")
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// Never fails: bad input is reported to `diags` and skipped, so the token
/// stream always ends in `Eof`.
pub fn lex(src: &str, diags: &mut Vec<Diagnostic>) -> Vec<Token> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        let start = i;

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // line comment
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                i += 1;
            }
            let span = Span::new(start, i);
            let text: String = src[start..i].chars().filter(|&c| c != '_').collect();
            let n = text.parse::<i64>().unwrap_or_else(|_| {
                diags.push(
                    Diagnostic::at(&codes::E0002, span, "number literal is out of range")
                        .note(format!("the largest i64 is {}", i64::MAX)),
                );
                0
            });
            out.push(Token { tok: Tok::Int(n), span });
            continue;
        }

        if c.is_ascii_alphabetic() || c == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let tok = match &src[start..i] {
                "fn" => Tok::Fn,
                "let" => Tok::Let,
                "var" => Tok::Var,
                "if" => Tok::If,
                "else" => Tok::Else,
                "while" => Tok::While,
                "return" => Tok::Return,
                "true" => Tok::True,
                "false" => Tok::False,
                s => Tok::Ident(s.to_string()),
            };
            out.push(Token { tok, span: Span::new(start, i) });
            continue;
        }

        let two = |a: u8, b: u8| c == a && bytes.get(i + 1) == Some(&b);
        let (tok, len) = if two(b'-', b'>') {
            (Tok::Arrow, 2)
        } else if two(b'=', b'=') {
            (Tok::EqEq, 2)
        } else if two(b'!', b'=') {
            (Tok::NotEq, 2)
        } else if two(b'<', b'=') {
            (Tok::Le, 2)
        } else if two(b'>', b'=') {
            (Tok::Ge, 2)
        } else if two(b'&', b'&') {
            (Tok::AndAnd, 2)
        } else if two(b'|', b'|') {
            (Tok::OrOr, 2)
        } else {
            let t = match c {
                b'(' => Tok::LParen,
                b')' => Tok::RParen,
                b'{' => Tok::LBrace,
                b'}' => Tok::RBrace,
                b',' => Tok::Comma,
                b':' => Tok::Colon,
                b';' => Tok::Semi,
                b'+' => Tok::Plus,
                b'-' => Tok::Minus,
                b'*' => Tok::Star,
                b'/' => Tok::Slash,
                b'%' => Tok::Percent,
                b'=' => Tok::Assign,
                b'<' => Tok::Lt,
                b'>' => Tok::Gt,
                b'!' => Tok::Bang,
                _ => {
                    let ch = src[i..].chars().next().unwrap();
                    i += ch.len_utf8();
                    diags.push(Diagnostic::at(
                        &codes::E0001,
                        Span::new(start, i),
                        format!("unexpected character `{}`", ch.escape_debug()),
                    ));
                    continue;
                }
            };
            (t, 1)
        };
        i += len;
        out.push(Token { tok, span: Span::new(start, i) });
    }

    out.push(Token { tok: Tok::Eof, span: Span::new(src.len(), src.len()) });
    out
}
