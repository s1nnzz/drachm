//! Find references and rename. Both work from the checker's table of uses
//! (`Index::refs`): every use of a name is recorded with the span of its
//! declaration, so the uses of one declaration are the refs with its span.
//! Rename changes the declaration and those uses across the program, which is
//! the whole project when the file is in one.

use std::collections::HashMap;
use std::path::Path;

use lsp_types::{Location, TextEdit, Uri};

use super::analysis::Analysis;
use super::pos::{path_to_uri, to_range};
use super::query::word_at;
use crate::diag::Span;
use crate::lexer::KEYWORDS;
use crate::types::Type;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Analysis {
    /// The global span of the declaration the name at `offset` of `file` refers to.
    fn target(&self, file: &Path, offset: usize) -> Option<Span> {
        let (id, g) = self.locate_in(file, offset)?;
        match self.ref_at(g) {
            Some(r) => r.def,
            None => self.decl_at(id, g).map(|d| d.2),
        }
    }

    /// The file of the program that holds the global span `s`, and the span
    /// in that file. `None` for text outside the program's files.
    fn in_program(&self, s: Span) -> Option<(usize, Span)> {
        let map = self.source_map();
        if map.is_empty() || s.end < s.start {
            return None;
        }
        let f = map.file_of(s.start);
        let base = map.base(f);
        let len = map.source(f).text.len();
        (s.start >= base && s.end <= base + len).then(|| map.locate(s))
    }

    /// The global spans of the uses of the declaration `def`, the declaration
    /// among them when `include_decl`, in source order.
    fn uses_of(&self, def: Span, include_decl: bool) -> Vec<Span> {
        let mut spans: Vec<Span> = self.index.refs.iter().filter(|r| r.def == Some(def) && r.at.end > r.at.start).map(|r| r.at).collect();
        if include_decl {
            spans.push(def);
        }
        spans.retain(|s| include_decl || *s != def);
        spans.sort_by_key(|s| (s.start, s.end));
        spans.dedup();
        spans
    }

    /// Every use of the name at the cursor, as locations (the declaration too
    /// when `include_decl`). Empty when there is no name there.
    pub fn references(&self, file: &Path, offset: usize, include_decl: bool) -> Vec<Location> {
        let Some(def) = self.target(file, offset) else { return Vec::new() };
        self.uses_of(def, include_decl)
            .into_iter()
            .filter_map(|s| {
                let (f, local) = self.in_program(s)?;
                let uri = path_to_uri(&self.files[f])?;
                Some(Location { uri, range: to_range(self.source_map().source(f), local) })
            })
            .collect()
    }

    /// Rename the name at `offset` of `file` to `new`: the edits per file, or
    /// the reason it cannot be done, shown to the user as it is.
    pub fn rename(&self, file: &Path, offset: usize, new: &str) -> Result<HashMap<Uri, Vec<TextEdit>>, String> {
        let valid = new.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && new.chars().all(is_word);
        if !valid {
            return Err(format!("`{new}` is not a name"));
        }
        if KEYWORDS.contains(&new) || Type::from_name(new).is_ok() {
            return Err(format!("`{new}` is a keyword or a type, so it cannot be a name"));
        }
        if self.word_under(file, offset).as_deref() == Some("self") {
            return Err("`self` cannot be renamed".into());
        }
        let def = self.target(file, offset).ok_or("there is no name to rename at the cursor")?;
        let (fid, decl) = self.in_program(def).ok_or("the name is declared in the standard library, so it cannot be renamed")?;
        let text = &self.source_map().source(fid).text;
        let old = text.get(decl.start..decl.end).unwrap_or_default().to_string();
        if old == "self" {
            return Err("`self` cannot be renamed".into());
        }
        // The declaration's span must be the name itself. If not, it is an
        // import, and the name comes from a C header or the standard library.
        let cursor_word = self.word_under(file, offset);
        if !old.chars().all(is_word) || cursor_word.as_deref() != Some(old.as_str()) {
            let (f, local) = self.source_map().locate(def);
            let src = self.source_map().source(f);
            let (a, b) = src.line_range(src.line_of(local.start));
            return Err(if src.text[a..b].contains("from c") {
                "the name comes from a C import, so it cannot be renamed".into()
            } else {
                "the name is declared in the standard library, so it cannot be renamed".into()
            });
        }
        // Only the project's own files: a name from a package it depends on is not ours to change.
        let root = self.entry.parent().unwrap_or(Path::new("."));
        if !self.files[fid].starts_with(root) {
            return Err(format!("`{old}` is declared in another package, so it cannot be renamed from here"));
        }
        let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
        for s in self.uses_of(def, true) {
            let Some((f, span)) = self.in_program(s) else { continue };
            let src = self.source_map().source(f);
            // A qualified use (`mod::name`) is renamed at its last word only.
            if !src.text.get(span.start..span.end).is_some_and(|t| t.ends_with(old.as_str())) {
                continue;
            }
            let start = span.end - old.len();
            if src.text[..start].chars().next_back().is_some_and(is_word) {
                continue;
            }
            let (Some(uri), range) = (path_to_uri(&self.files[f]), to_range(src, Span::new(start, span.end))) else { continue };
            changes.entry(uri).or_default().push(TextEdit { range, new_text: new.to_string() });
        }
        Ok(changes)
    }

    /// The word the cursor is on, in the file.
    fn word_under(&self, file: &Path, offset: usize) -> Option<String> {
        let (id, g) = self.locate_in(file, offset)?;
        let map = self.source_map();
        let text = &map.source(id).text;
        let (s, e) = word_at(text, g - map.base(id))?;
        Some(text[s..e].to_string())
    }
}
