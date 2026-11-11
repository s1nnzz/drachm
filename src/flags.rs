//! File flags: `//--!$key: value` lines at the top of a source file. The lexer
//! sees them as ordinary comments; this module reads them before lexing.

use crate::diag::{codes, Diagnostic, Span};

/// What the flag lines of a file say.
pub struct FileFlags {
    /// Every well-formed flag line in the top block, in order.
    pub entries: Vec<FileFlag>,
    /// `//--!$unsafe: true` is present. Only a valid `true` sets it.
    pub unsafe_allowed: bool,
}

/// One well-formed `key: value` line.
pub struct FileFlag {
    pub key: String,
    pub value: String,
    /// The whole line, without its line break.
    pub span: Span,
}

const PREFIX: &str = "//--!$";
const KNOWN_KEYS: &[&str] = &["unsafe"];

/// Reads the flags at the top of `src`, reporting bad ones to `diags`. Never
/// fails. Flag lines after the top block are reported and not applied.
pub fn parse(src: &str, diags: &mut Vec<Diagnostic>) -> FileFlags {
    let lines = split_lines(src);
    let mut flags = FileFlags { entries: Vec::new(), unsafe_allowed: false };
    // A `#!` line may come first; the flags start on the line after it.
    let mut i = usize::from(lines.first().is_some_and(|l| l.text.starts_with("#!")));
    while let Some(line) = lines.get(i).filter(|l| l.text.starts_with(PREFIX)) {
        flag_line(line, &mut flags, diags);
        i += 1;
    }
    for line in &lines[i..] {
        if line.text.starts_with(PREFIX) {
            diags.push(
                Diagnostic::new(&codes::E0318, "file flags must be at the very top of the file")
                    .label(line.span(), "this flag comes after code")
                    .help("move it above the first line of code, with the other flags"),
            );
        }
    }
    flags
}

struct Line<'a> {
    start: usize,
    /// The text without its line break (and without a `\r` before it).
    text: &'a str,
}

impl Line<'_> {
    fn span(&self) -> Span {
        self.span_at(0, self.text.len())
    }

    /// A span of `len` bytes starting `at` bytes into the line.
    fn span_at(&self, at: usize, len: usize) -> Span {
        Span::new(self.start + at, self.start + at + len)
    }
}

fn split_lines(src: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for piece in src.split('\n') {
        lines.push(Line { start, text: piece.strip_suffix('\r').unwrap_or(piece) });
        start += piece.len() + 1;
    }
    lines
}

fn malformed(line: &Line) -> Diagnostic {
    Diagnostic::at(&codes::E0318, line.span(), "malformed file flag")
}

fn flag_line(line: &Line, flags: &mut FileFlags, diags: &mut Vec<Diagnostic>) {
    let body = &line.text[PREFIX.len()..];
    let Some((key_part, value_part)) = body.split_once(':') else {
        diags.push(malformed(line));
        return;
    };
    let key = key_part.trim();
    if !valid_key(key) {
        diags.push(malformed(line));
        return;
    }
    // Byte offsets of the key and the value within the line.
    let key_at = PREFIX.len() + key_part.len() - key_part.trim_start().len();
    let value_at = PREFIX.len() + key_part.len() + 1 + value_part.len() - value_part.trim_start().len();
    let value = value_part.trim();
    flags.entries.push(FileFlag { key: key.into(), value: value.into(), span: line.span() });
    match key {
        "unsafe" => match value {
            "true" => flags.unsafe_allowed = true,
            "false" => flags.unsafe_allowed = false,
            _ => diags.push(
                Diagnostic::new(&codes::E0318, format!("`unsafe` must be `true` or `false`, found `{value}`"))
                    .label(line.span_at(value_at, value.len()), "expected `true` or `false`"),
            ),
        },
        _ => {
            let mut d = Diagnostic::at(&codes::W0319, line.span_at(key_at, key.len()), format!("unknown file flag `{key}`"));
            if let Some(k) = similar(key, KNOWN_KEYS) {
                d.set_help(format!("did you mean `{k}`?"));
            }
            diags.push(d);
        }
    }
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The known key closest to `key`, if it is within two edits.
fn similar<'a>(key: &str, known: &[&'a str]) -> Option<&'a str> {
    known.iter().copied().map(|k| (edit_distance(key, k), k)).filter(|(d, _)| *d <= 2).min_by_key(|(d, _)| *d).map(|(_, k)| k)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<u8> = b.bytes().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.bytes().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(src: &str) -> (FileFlags, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let flags = parse(src, &mut diags);
        (flags, diags)
    }

    fn ids(diags: &[Diagnostic]) -> Vec<&'static str> {
        diags.iter().map(|d| d.code.id).collect()
    }

    fn text_of(src: &str, span: Span) -> &str {
        &src[span.start..span.end]
    }

    #[test]
    fn shebang_then_flag() {
        let src = "#!/usr/bin/env drachm\n//--!$unsafe: true\nnone main() { }\n";
        let (flags, diags) = check(src);
        assert!(diags.is_empty());
        assert!(flags.unsafe_allowed);
        assert_eq!(flags.entries.len(), 1);
        assert_eq!(flags.entries[0].key, "unsafe");
        assert_eq!(flags.entries[0].value, "true");
        assert_eq!(text_of(src, flags.entries[0].span), "//--!$unsafe: true");
    }

    #[test]
    fn several_flags_in_a_row() {
        let src = "//--!$unsafe: false\n//--!$unsafe: true\n//--!$unsafe: true\nnone main() { }\n";
        let (flags, diags) = check(src);
        assert!(diags.is_empty());
        assert_eq!(flags.entries.len(), 3);
        assert!(flags.unsafe_allowed);
    }

    #[test]
    fn last_valid_value_wins() {
        let src = "//--!$unsafe: true\n//--!$unsafe: false\n//--!$unsafe: yes\n";
        let (flags, diags) = check(src);
        assert!(!flags.unsafe_allowed);
        assert_eq!(ids(&diags), ["E0318"]);
    }

    #[test]
    fn blank_line_ends_block() {
        let src = "//--!$unsafe: true\n\n//--!$unsafe: false\nnone main() { }\n";
        let (flags, diags) = check(src);
        assert_eq!(flags.entries.len(), 1);
        assert!(flags.unsafe_allowed);
        assert_eq!(ids(&diags), ["E0318"]);
        assert_eq!(diags[0].message, "file flags must be at the very top of the file");
    }

    #[test]
    fn blank_first_line_means_no_flags() {
        let (flags, diags) = check("\n//--!$unsafe: true\n");
        assert!(flags.entries.is_empty());
        assert!(!flags.unsafe_allowed);
        assert_eq!(ids(&diags), ["E0318"]);
    }

    #[test]
    fn flag_after_code() {
        let src = "none main() { }\n//--!$unsafe: true\n";
        let (flags, diags) = check(src);
        assert!(!flags.unsafe_allowed);
        assert!(flags.entries.is_empty());
        assert_eq!(ids(&diags), ["E0318"]);
        assert_eq!(diags[0].message, "file flags must be at the very top of the file");
        assert_eq!(diags[0].labels[0].1, "this flag comes after code");
        assert_eq!(diags[0].help.as_deref(), Some("move it above the first line of code, with the other flags"));
        assert_eq!(text_of(src, diags[0].labels[0].0), "//--!$unsafe: true");
    }

    #[test]
    fn bad_value() {
        let src = "//--!$unsafe: yes\nnone main() { }\n";
        let (flags, diags) = check(src);
        assert!(!flags.unsafe_allowed);
        assert_eq!(ids(&diags), ["E0318"]);
        assert_eq!(diags[0].message, "`unsafe` must be `true` or `false`, found `yes`");
        assert_eq!(text_of(src, diags[0].labels[0].0), "yes");
    }

    #[test]
    fn unknown_key_warns() {
        let src = "//--!$unsfe: true\n";
        let (flags, diags) = check(src);
        assert!(!flags.unsafe_allowed);
        assert_eq!(ids(&diags), ["W0319"]);
        assert!(!diags[0].is_error());
        assert_eq!(diags[0].message, "unknown file flag `unsfe`");
        assert_eq!(diags[0].help.as_deref(), Some("did you mean `unsafe`?"));
        assert_eq!(text_of(src, diags[0].labels[0].0), "unsfe");

        let (_, far) = check("//--!$zzzzzzz: 1\n");
        assert_eq!(ids(&far), ["W0319"]);
        assert!(far[0].help.is_none());
    }

    #[test]
    fn no_flags() {
        let (flags, diags) = check("none main() { }\n");
        assert!(flags.entries.is_empty());
        assert!(!flags.unsafe_allowed);
        assert!(diags.is_empty());
    }

    #[test]
    fn malformed_line() {
        let src = "//--!$unsafe true\n";
        let (flags, diags) = check(src);
        assert!(flags.entries.is_empty());
        assert_eq!(ids(&diags), ["E0318"]);
        assert_eq!(diags[0].message, "malformed file flag");
        assert_eq!(text_of(src, diags[0].labels[0].0), "//--!$unsafe true");
    }

    #[test]
    fn crlf_spans_and_values() {
        let src = "//--!$unsafe: true\r\n//--!$unsafe:   yes\r\nnone main() { }\r\n";
        let (flags, diags) = check(src);
        assert!(flags.unsafe_allowed);
        assert_eq!(flags.entries[0].value, "true");
        assert_eq!(flags.entries[1].value, "yes");
        assert_eq!(text_of(src, flags.entries[1].span), "//--!$unsafe:   yes");
        assert_eq!(text_of(src, diags[0].labels[0].0), "yes");
    }
}
