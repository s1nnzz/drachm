//! Completion. The text under the cursor is usually unfinished (`p::` with
//! nothing after it), which the parser cannot make sense of, so the buffer is
//! patched first and then analysed again:
//!
//! - after `::`, `->` or `.` the operator and the word typed so far are
//!   removed, which leaves the receiver as an expression whose type the
//!   checker recorded;
//! - anywhere else the word typed so far is replaced by a placeholder name,
//!   and the checker's note of what type is expected there gives the bare
//!   enum variants.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp_types::CompletionItemKind as Kind;

use super::analysis::Analysis;
use super::query::word_at;
use crate::lexer::KEYWORDS;

/// What the placeholder is called in the patched text.
const PLACEHOLDER: &str = "zzcursor";

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: Kind,
    pub detail: String,
    /// Sorts before items with a later key.
    pub sort: u8,
}

fn item(label: impl Into<String>, kind: Kind, detail: impl Into<String>, sort: u8) -> Item {
    Item { label: label.into(), kind, detail: detail.into(), sort }
}

/// Where the cursor is, as far as completion cares.
#[derive(Debug, PartialEq)]
enum Context {
    /// After `::`, `->` or `.`: the operator starts at `op`, the word typed so far at `word`.
    Member { op: usize, word: usize, dot: bool },
    /// A name: the word typed so far starts at `word`.
    Name { word: usize },
    /// In a string or a comment.
    Nothing,
}

fn context(text: &str, offset: usize) -> Context {
    let offset = offset.min(text.len());
    let word = word_at(text, offset).map_or(offset, |(s, _)| s.min(offset));
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[line_start..offset];
    if line.contains("//") || line.matches('"').count() % 2 == 1 {
        return Context::Nothing;
    }
    let before = &text[..word];
    for op in ["::", "->"] {
        if before.ends_with(op) {
            return Context::Member { op: word - 2, word, dot: false };
        }
    }
    // `p.x`, `f().x`, `xs[0].x`; not `..` and not a number like `1.5`.
    if before.ends_with('.') && !before.ends_with("..") {
        let left = &before[..word - 1];
        let number = word_before(left, left.len()).is_some_and(|w| w.starts_with(|c: char| c.is_ascii_digit()));
        if left.ends_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | ')' | ']')) && !number {
            return Context::Member { op: word - 1, word, dot: true };
        }
    }
    Context::Name { word }
}

/// The type with borrows and pointers taken off.
fn bare(ty: &str) -> &str {
    let mut t = ty.trim();
    loop {
        let n = t.strip_prefix("&mut ").or_else(|| t.strip_prefix('&')).or_else(|| t.strip_prefix("*fixed ")).or_else(|| t.strip_prefix('*'));
        match n {
            Some(n) => t = n.trim(),
            None => return t,
        }
    }
}

const STRING_METHODS: &[&str] = &[
    "len", "isEmpty", "reverse", "sub", "upper", "lower", "trim", "find", "contains", "startsWith", "endsWith", "count", "join", "replace", "replaceLimit", "insert", "remove",
    "repeat", "padLeft", "padRight", "split", "lines", "chars", "text", "to",
];
const LIST_METHODS: &[&str] = &["len", "each", "push", "pop", "get", "first", "last", "insert", "remove", "clear", "join", "contains"];
const ARRAY_METHODS: &[&str] = &["len", "each", "get", "first", "last", "contains", "join"];
const MAP_METHODS: &[&str] = &["len", "get", "set", "remove", "each"];
const NUMBER_METHODS: &[&str] = &["text", "to"];
const IO_FUNCTIONS: &[&str] = &["out", "outln", "outf", "inln", "ended"];
const TYPES: &[&str] = &[
    "none", "bool", "string", "uint8", "uint16", "uint32", "uint64", "sint8", "sint16", "sint32", "sint64", "float32", "float64", "Map", "OrderedMap",
];

fn builtin(names: &[&str], sort: u8) -> Vec<Item> {
    names.iter().map(|n| item(*n, Kind::METHOD, "built in", sort)).collect()
}

impl Analysis {
    /// The completions at byte `offset` of `file`. `docs` are the open buffers
    /// (the text of `file` among them), which are analysed again with the
    /// text under the cursor patched.
    pub fn complete_at(file: &Path, docs: &HashMap<PathBuf, String>, offset: usize) -> Vec<Item> {
        let Some(text) = docs.get(file) else { return Vec::new() };
        let patch = |from: usize, to: usize, with: &str| {
            let mut d = docs.clone();
            d.insert(file.to_path_buf(), format!("{}{with}{}", &text[..from], &text[to..]));
            Analysis::run(file, &d)
        };
        match context(text, offset) {
            Context::Nothing => Vec::new(),
            Context::Member { op, word, dot } => {
                // The receiver stays, with a member that does not exist: the
                // checker looks at the receiver before it finds that out. A
                // line that ends here gets the `;` it will need.
                let rest = text[offset..].lines().next().unwrap_or("").trim();
                let member = if dot { PLACEHOLDER.to_string() } else { format!("{PLACEHOLDER}()") };
                let a = patch(word, offset.max(word), &format!("{member}{}", if rest.is_empty() || rest.starts_with('}') { ";" } else { "" }));
                // The receiver's own words, for `self`, struct names and modules.
                let recv = word_before(text, op);
                a.members(file, op, recv.as_deref(), dot)
            }
            Context::Name { word } => {
                // `case ` with nothing after it yet: give it a body so that it parses.
                let line_start = text[..word].rfind('\n').map_or(0, |i| i + 1);
                let line_end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
                let bare_case = text[line_start..word].trim_start().starts_with("case ") && !text[word..line_end].contains('{');
                let a = patch(word, offset.max(word), &if bare_case { format!("{PLACEHOLDER} {{ }}") } else { PLACEHOLDER.to_string() });
                a.names(file, word)
            }
        }
    }

    fn members(&self, file: &Path, op: usize, recv: Option<&str>, dot: bool) -> Vec<Item> {
        let ix = &self.index;
        if let Some(r) = recv {
            if r == "io" && !dot {
                return IO_FUNCTIONS.iter().map(|n| item(*n, Kind::FUNCTION, "io", 0)).collect();
            }
            if !dot && self.expr_type_ending_at(file, op).is_none() {
                if let Some(s) = ix.structs.iter().find(|s| s.name == r) {
                    return s.methods.iter().filter(|m| m.is_static).map(|m| item(&m.name, Kind::METHOD, &m.sig, 0)).collect();
                }
                if let Some(e) = ix.enums.iter().find(|e| e.name == r) {
                    return e.variants.iter().map(|v| item(&v.name, Kind::ENUM_MEMBER, &v.shape, 0)).collect();
                }
            }
        }
        let Some(ty) = self.expr_type_ending_at(file, op) else { return Vec::new() };
        let ty = bare(&ty).to_string();
        let own = recv == Some("self");
        if let Some(s) = ix.structs.iter().find(|s| s.name == ty) {
            if dot {
                return s.fields.iter().filter(|f| f.public || own).map(|f| item(&f.name, Kind::FIELD, &f.ty, 0)).collect();
            }
            return s.methods.iter().filter(|m| !m.is_static && (m.public || own) && m.name != "drop").map(|m| item(&m.name, Kind::METHOD, &m.sig, 0)).collect();
        }
        if dot {
            return Vec::new();
        }
        if ty == "string" {
            return builtin(STRING_METHODS, 0);
        }
        if ty.ends_with("[?]") || ty.starts_with("List<") {
            return builtin(LIST_METHODS, 0);
        }
        if ty.ends_with(']') {
            return builtin(ARRAY_METHODS, 0);
        }
        if ty.starts_with("Map<") || ty.starts_with("OrderedMap<") {
            return builtin(MAP_METHODS, 0);
        }
        if ty == "bool" || ty.starts_with("uint") || ty.starts_with("sint") || ty.starts_with("float") {
            return builtin(NUMBER_METHODS, 0);
        }
        Vec::new()
    }

    /// The type of the longest expression that ends at global... local `end` of `file`.
    fn expr_type_ending_at(&self, file: &Path, end: usize) -> Option<String> {
        let id = self.file_id(file)?;
        let g = self.source_map().base(id) + end;
        self.index.types.iter().filter(|(s, _)| s.end == g && s.end > s.start).min_by_key(|(s, _)| s.start).map(|(_, t)| t.clone())
    }

    /// Names that can be written at the placeholder at `word` (a local offset).
    fn names(&self, file: &Path, word: usize) -> Vec<Item> {
        let ix = &self.index;
        let Some(id) = self.file_id(file) else { return Vec::new() };
        let g = self.source_map().base(id) + word;
        let mut out: Vec<Item> = Vec::new();
        // The variants of the enum wanted here come first.
        if let Some((_, en)) = ix.expected.iter().filter(|(s, _)| s.start <= g && g <= s.end).min_by_key(|(s, _)| s.end - s.start)
            && let Some(e) = ix.enums.iter().find(|e| e.name == *en)
        {
            out.extend(e.variants.iter().map(|v| item(&v.name, Kind::ENUM_MEMBER, format!("{}::{}", e.name, v.shape), 0)));
        }
        for l in ix.locals.iter().filter(|l| l.decl.end <= g && g <= l.end && l.name != PLACEHOLDER) {
            if !out.iter().any(|o| o.label == l.name && o.kind == Kind::VARIABLE) {
                out.push(item(&l.name, Kind::VARIABLE, &l.ty, 1));
            }
        }
        out.extend(ix.funcs.iter().map(|f| item(&f.name, Kind::FUNCTION, &f.sig, 2)));
        out.extend(ix.structs.iter().map(|s| item(&s.name, Kind::STRUCT, "struct", 3)));
        out.extend(ix.enums.iter().map(|e| item(&e.name, Kind::ENUM, "enum", 3)));
        out.extend(TYPES.iter().map(|t| item(*t, Kind::KEYWORD, "type", 4)));
        out.extend(KEYWORDS.iter().filter(|k| k.chars().all(|c| c.is_ascii_lowercase()) && !TYPES.contains(k)).map(|k| item(*k, Kind::KEYWORD, "keyword", 5)));
        out.retain(|i| i.label != PLACEHOLDER);
        out
    }
}

/// The word just before `end` (a receiver such as `self` or `Point`).
fn word_before(text: &str, end: usize) -> Option<String> {
    let start = text[..end].char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_').last().map(|(i, _)| i)?;
    Some(text[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts() {
        assert_eq!(context("p::", 3), Context::Member { op: 1, word: 3, dot: false });
        assert_eq!(context("  p::su", 7), Context::Member { op: 3, word: 5, dot: false });
        assert_eq!(context("p->x", 4), Context::Member { op: 1, word: 3, dot: false });
        assert_eq!(context("p.x", 3), Context::Member { op: 1, word: 2, dot: true });
        assert_eq!(context("Dir d = No", 10), Context::Name { word: 8 });
        assert_eq!(context("s = \"ab::", 9), Context::Nothing);
        assert_eq!(context("// p::", 6), Context::Nothing);
    }
}
