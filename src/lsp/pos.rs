//! Byte offsets and LSP positions (zero-based line, UTF-16 column).

use lsp_types::{Position, Range, Uri};
use std::path::{Path, PathBuf};

use crate::diag::{Source, Span};

pub fn to_position(src: &Source, offset: usize) -> Position {
    let offset = offset.min(src.text.len());
    let line = src.line_of(offset);
    let (start, _) = src.line_range(line);
    let col: usize = src.text[start..offset.max(start)].chars().map(char::len_utf16).sum();
    Position { line: line as u32, character: col as u32 }
}

pub fn to_range(src: &Source, span: Span) -> Range {
    Range { start: to_position(src, span.start), end: to_position(src, span.end.max(span.start)) }
}

/// The byte offset of `pos` in `src` (clamped to the end of the line).
pub fn to_offset(src: &Source, pos: Position) -> usize {
    let line = (pos.line as usize).min(src.line_count() - 1);
    let (start, end) = src.line_range(line);
    let mut units = 0u32;
    for (i, c) in src.text[start..end].char_indices() {
        if units >= pos.character {
            return start + i;
        }
        units += c.len_utf16() as u32;
    }
    end
}

/// `file:///a/b%20c.drh` to a path.
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let s = uri.as_str().strip_prefix("file://")?;
    let s = s.strip_prefix("localhost").unwrap_or(s);
    let raw = s.as_bytes();
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && i + 2 < raw.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            bytes.push(b);
            i += 3;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let text = String::from_utf8(bytes).ok()?;
    #[cfg(windows)]
    let text = text.trim_start_matches('/').to_string();
    Some(PathBuf::from(text))
}

pub fn path_to_uri(path: &Path) -> Option<Uri> {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns() {
        let src = Source::new("t", "a\u{1F600}b\nxy");
        // The emoji is 4 bytes and 2 UTF-16 units.
        assert_eq!(to_position(&src, 5), Position { line: 0, character: 3 });
        assert_eq!(to_offset(&src, Position { line: 0, character: 3 }), 5);
        assert_eq!(to_position(&src, 8), Position { line: 1, character: 1 });
    }

    #[test]
    fn uris() {
        let p = Path::new("/tmp/a b/c.drh");
        let u = path_to_uri(p).unwrap();
        assert_eq!(u.as_str(), "file:///tmp/a%20b/c.drh");
        assert_eq!(uri_to_path(&u).unwrap(), p);
    }
}
