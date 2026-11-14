//! Answers to hover and go-to-definition, from the index of an analysis.

use std::path::Path;

use super::analysis::Analysis;
use crate::diag::Span;
use crate::typeck::index::{EnumRec, FuncRec, Ref, StructRec};

/// What a hover shows: markdown, and the text it is about.
pub struct Hover {
    pub markdown: String,
    pub span: Span,
}

/// A place in a file of the program: the file's index and a span local to it.
pub struct Place {
    pub file: usize,
    pub span: Span,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The word touching `offset` in `text`.
pub fn word_at(text: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = offset.min(text.len());
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(offset, |(i, _)| i);
    let end = offset + text[offset..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len() - offset, |(i, _)| i);
    (start < end && !text[start..end].starts_with(|c: char| c.is_ascii_digit())).then_some((start, end))
}

fn code_block(code: &str) -> String {
    format!("```drachm\n{code}\n```")
}

impl StructRec {
    fn show(&self) -> String {
        let mut out = format!("struct {} {{", self.name);
        for f in &self.fields {
            out += &format!("\n    {}{} {};", if f.public { "public " } else { "" }, f.ty, f.name);
        }
        for m in &self.methods {
            out += &format!("\n    {}{};", if m.is_static { "static " } else { "" }, m.sig);
        }
        out + "\n}"
    }
}

impl EnumRec {
    fn show(&self) -> String {
        let vs: Vec<String> = self.variants.iter().map(|v| format!("    {},", v.shape)).collect();
        format!("enum {} {{\n{}\n}}", self.name, vs.join("\n"))
    }
}

impl Analysis {
    /// The global offset of `offset` in `file`, and the file's text.
    pub(super) fn locate_in(&self, file: &Path, offset: usize) -> Option<(usize, usize)> {
        let id = self.file_id(file)?;
        let map = self.source_map();
        Some((id, map.base(id) + offset.min(map.source(id).text.len())))
    }

    /// The reference at `g` that is the smallest span around it.
    pub(super) fn ref_at(&self, g: usize) -> Option<&Ref> {
        self.index.refs.iter().filter(|r| r.at.start <= g && g <= r.at.end && r.at.end > r.at.start).min_by_key(|r| r.at.end - r.at.start)
    }

    /// A declaration named by the word at `g`, when the checker has no reference there.
    pub(super) fn decl_at(&self, id: usize, g: usize) -> Option<(Span, String, Span)> {
        let map = self.source_map();
        let text = &map.source(id).text;
        let base = map.base(id);
        let (s, e) = word_at(text, g - base)?;
        let word = &text[s..e];
        let at = Span::new(base + s, base + e);
        let ix = &self.index;
        if let Some(st) = ix.structs.iter().find(|x| x.name == word) {
            return Some((at, st.show(), st.name_span));
        }
        if let Some(en) = ix.enums.iter().find(|x| x.name == word) {
            return Some((at, en.show(), en.name_span));
        }
        if let Some(FuncRec { sig, name_span, .. }) = ix.funcs.iter().find(|x| x.name == word) {
            return Some((at, sig.clone(), *name_span));
        }
        None
    }

    pub fn hover(&self, file: &Path, offset: usize) -> Option<Hover> {
        let (id, g) = self.locate_in(file, offset)?;
        let local = |s: Span| self.source_map().locate(s).1;
        // A use with no code of its own (a type name) shows the declaration below.
        if let Some(r) = self.ref_at(g).filter(|r| !r.code.is_empty()) {
            let mut markdown = code_block(&r.code);
            if !r.note.is_empty() {
                markdown += &format!("\n\n{}", r.note);
            }
            return Some(Hover { markdown, span: local(r.at) });
        }
        if let Some((at, code, _)) = self.decl_at(id, g) {
            return Some(Hover { markdown: code_block(&code), span: local(at) });
        }
        // The smallest expression around the cursor.
        let (span, ty) = self.index.types.iter().filter(|(s, _)| s.start <= g && g <= s.end && s.end > s.start).min_by_key(|(s, _)| s.end - s.start)?;
        let text = {
            let (f, l) = self.source_map().locate(*span);
            self.source_map().source(f).text[l.start..l.end].to_string()
        };
        let shown = if text.len() <= 40 && !text.contains('\n') { format!("{text}: {ty}") } else { ty.clone() };
        Some(Hover { markdown: code_block(&shown), span: local(*span) })
    }

    pub fn definition(&self, file: &Path, offset: usize) -> Option<Place> {
        let (id, g) = self.locate_in(file, offset)?;
        let def = match self.ref_at(g) {
            Some(r) => r.def?,
            None => self.decl_at(id, g)?.2,
        };
        let (file, span) = self.source_map().locate(def);
        Some(Place { file, span })
    }
}
