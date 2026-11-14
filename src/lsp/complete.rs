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
    /// A module name in an import (`in io;`, or after `from`).
    Module,
    /// An item named in an import, taken from the module named after `from`.
    Item { module: String },
}

fn context(text: &str, offset: usize) -> Context {
    let offset = offset.min(text.len());
    let word = word_at(text, offset).map_or(offset, |(s, _)| s.min(offset));
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[line_start..offset];
    if line.contains("//") || line.matches('"').count() % 2 == 1 {
        return Context::Nothing;
    }
    if let Some(c) = import_context(text, word) {
        return c;
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

/// The context of an import line: `in mod;`, `in mod as x;`, `in a, b from mod;`
/// and `in * from mod;`. `word` is where the word typed so far starts. `None`
/// when the line is not an import.
fn import_context(text: &str, word: usize) -> Option<Context> {
    let line_start = text[..word].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[word..].find('\n').map_or(text.len(), |i| word + i);
    let stmt = text[line_start..word].trim_start().strip_prefix("in ")?;
    let shape = stmt.trim_end();
    if shape.contains([';', '"', '*']) || shape.ends_with(" as") || shape == "as" {
        return Some(Context::Nothing);
    }
    if shape == "from" || shape.ends_with(" from") {
        return Some(Context::Module);
    }
    // The names before `from` are items of the module after it, which may be typed later on the line.
    let rest = &text[word..line_end];
    if let Some((_, after)) = rest.split_once(" from ") {
        let module = after.split(';').next().unwrap_or("").trim();
        return Some(Context::Item { module: module.to_string() });
    }
    Some(Context::Module)
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

/// The built-in methods named in `names`, each with the signature `sig` gives.
fn builtin(names: &[&str], sig: impl Fn(&str) -> String, sort: u8) -> Vec<Item> {
    names.iter().map(|n| item(*n, Kind::METHOD, sig(n), sort)).collect()
}

/// The signature of `name` in a table of `(name, signature)`.
fn from_table(table: &[(&str, &str)], name: &str) -> String {
    table.iter().find(|(n, _)| *n == name).map_or_else(String::new, |(_, s)| s.to_string())
}

/// Methods of a string, as they are written (`T` is the number type of `to<T>()`).
const STRING_SIGS: &[(&str, &str)] = &[
    ("len", "uint64 len()"),
    ("isEmpty", "bool isEmpty()"),
    ("reverse", "string reverse()"),
    ("sub", "string sub(sint64 from, sint64 to)"),
    ("upper", "string upper()"),
    ("lower", "string lower()"),
    ("trim", "string trim()"),
    ("find", "sint64 find(string text)"),
    ("contains", "bool contains(string text)"),
    ("startsWith", "bool startsWith(string text)"),
    ("endsWith", "bool endsWith(string text)"),
    ("count", "uint64 count(string text)"),
    ("join", "string join(string text)"),
    ("replace", "string replace(string from, string to)"),
    ("replaceLimit", "string replaceLimit(string from, string to, uint64 limit)"),
    ("insert", "string insert(sint64 index, string text)"),
    ("remove", "string remove(sint64 from, sint64 to)"),
    ("repeat", "string repeat(uint64 count)"),
    ("padLeft", "string padLeft(uint64 width[, string fill])"),
    ("padRight", "string padRight(uint64 width[, string fill])"),
    ("split", "string[?] split(string sep)"),
    ("lines", "string[?] lines()"),
    ("chars", "string[?] chars()"),
    ("text", "string text()"),
    ("to", "T to<T>()"),
];

/// The item type of a list (`T[?]`, `List<T>`) or an array (`T[N]`).
fn item_type(ty: &str) -> String {
    if let Some(t) = ty.strip_suffix("[?]") {
        return t.to_string();
    }
    if let Some(t) = ty.strip_prefix("List<").and_then(|t| t.strip_suffix('>')) {
        return t.to_string();
    }
    match ty.rfind('[') {
        Some(i) if ty.ends_with(']') => ty[..i].to_string(),
        _ => "T".to_string(),
    }
}

/// The signature of a method of a list (`t` the item type) or an array.
fn list_sig(name: &str, t: &str) -> String {
    match name {
        "len" => "uint64 len()".into(),
        "each" => "none each(callback)".into(),
        "push" => format!("none push({t} item)"),
        "pop" => format!("{t}? pop()"),
        "get" => format!("{t}? get(sint64 index)"),
        "first" => format!("{t}? first()"),
        "last" => format!("{t}? last()"),
        "insert" => format!("none insert(sint64 index, {t} item)"),
        "remove" => format!("{t} remove(sint64 index)"),
        "clear" => "none clear()".into(),
        "join" => "string join(string sep)".into(),
        "contains" => format!("bool contains({t} item)"),
        _ => String::new(),
    }
}

/// The key and value types of a map (`Map<K, V>`).
fn key_value(ty: &str) -> (String, String) {
    let inner = ty.split_once('<').map_or("", |(_, r)| r.trim_end_matches('>'));
    match inner.split_once(", ") {
        Some((k, v)) => (k.to_string(), v.to_string()),
        None => ("K".to_string(), "V".to_string()),
    }
}

/// The signature of a method of a map.
fn map_sig(name: &str, k: &str, v: &str) -> String {
    match name {
        "len" => "uint64 len()".into(),
        "get" => format!("{v}? get({k} key)"),
        "set" => format!("none set({k} key, {v} value)"),
        "remove" => format!("none remove({k} key)"),
        "each" => "none each(callback)".into(),
        _ => String::new(),
    }
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
            Context::Module => Analysis::run(file, docs).import_modules(),
            Context::Item { module } => Analysis::run(file, docs).import_items(&module),
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

    /// The modules an import can name: the standard library's and the project's.
    fn import_modules(&self) -> Vec<Item> {
        let mut out: Vec<Item> = crate::stdlib::names().map(|m| item(m, Kind::MODULE, "standard library", 0)).collect();
        // The entry file comes first in the graph; it is not imported by name.
        if let Some(g) = &self.graph {
            out.extend(g.modules.iter().skip(1).map(|m| item(&m.name, Kind::MODULE, "module", 0)));
        }
        out
    }

    /// The names a module exports (`out`), for an import from it. The graph has
    /// taken the modules' trees by now, so the module's text is read again.
    fn import_items(&self, module: &str) -> Vec<Item> {
        let Some(g) = &self.graph else { return Vec::new() };
        let Some(m) = g.modules.iter().find(|m| m.name == module) else { return Vec::new() };
        let mut diags = Vec::new();
        let p = crate::parser::parse(crate::lexer::lex(&g.map.source(m.file).text, &mut diags), &mut diags);
        p.exports
            .iter()
            .map(|e| {
                let kind = if p.funcs.iter().any(|f| f.name == e.name) {
                    Kind::FUNCTION
                } else if p.structs.iter().any(|s| s.name == e.name) {
                    Kind::STRUCT
                } else if p.enums.iter().any(|x| x.name == e.name) {
                    Kind::ENUM
                } else if p.interfaces.iter().any(|x| x.name == e.name) {
                    Kind::INTERFACE
                } else if p.consts.iter().any(|c| c.name == e.name) {
                    Kind::CONSTANT
                } else {
                    Kind::VARIABLE
                };
                item(e.alias.as_deref().unwrap_or(&e.name), kind, format!("from {module}"), 0)
            })
            .collect()
    }

    fn members(&self, file: &Path, op: usize, recv: Option<&str>, dot: bool) -> Vec<Item> {
        let ix = &self.index;
        if let Some(r) = recv {
            if r == "time" && !dot {
                return vec![item("sleep", Kind::FUNCTION, "none sleep(float64<s> duration)", 0)];
            }
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
            return builtin(STRING_METHODS, |n| from_table(STRING_SIGS, n), 0);
        }
        if ty.ends_with("[?]") || ty.starts_with("List<") {
            let t = item_type(&ty);
            return builtin(LIST_METHODS, |n| list_sig(n, &t), 0);
        }
        if ty.ends_with(']') {
            let t = item_type(&ty);
            return builtin(ARRAY_METHODS, |n| list_sig(n, &t), 0);
        }
        if ty.starts_with("Map<") || ty.starts_with("OrderedMap<") {
            let (k, v) = key_value(&ty);
            return builtin(MAP_METHODS, |n| map_sig(n, &k, &v), 0);
        }
        if ty == "bool" || ty.starts_with("uint") || ty.starts_with("sint") || ty.starts_with("float") {
            return builtin(NUMBER_METHODS, |n| from_table(&[("text", "string text()"), ("to", "T to<T>()")], n), 0);
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

    #[test]
    fn import_lines() {
        assert_eq!(context("in ", 3), Context::Module);
        assert_eq!(context("in gam", 6), Context::Module);
        assert_eq!(context("in x from ", 10), Context::Module);
        assert_eq!(context("in gr", 5), Context::Module);
        assert_eq!(context("in gr from util;", 5), Context::Item { module: "util".into() });
        assert_eq!(context("in A as B, gr from util;", 13), Context::Item { module: "util".into() });
        assert_eq!(context("in io as ", 9), Context::Nothing);
        assert_eq!(context("in * from ", 10), Context::Nothing);
    }
}
