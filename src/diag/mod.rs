//! Diagnostics: what every compiler phase reports instead of stopping at the
//! first problem. Rendering lives in `render`, the code table in `codes`.

pub mod codes;
mod render;

pub use codes::Code;
pub use render::{render, render_all, render_all_map};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    pub fn to(self, other: Span) -> Span {
        Span { start: self.start, end: other.end }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static Code,
    pub message: String,
    /// The first label is the primary one (`^^^`, and the `-->` location);
    /// the rest are secondary (`---`). May be empty for whole-file problems.
    pub labels: Vec<(Span, String)>,
    /// Extra facts, each printed on its own `note:` line.
    pub notes: Vec<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    /// A diagnostic with no location. Severity comes from the code (`E`/`W`).
    pub fn new(code: &'static Code, message: impl Into<String>) -> Self {
        let severity = if code.is_warning() { Severity::Warning } else { Severity::Error };
        Diagnostic { severity, code, message: message.into(), labels: Vec::new(), notes: Vec::new(), help: None }
    }

    /// A diagnostic whose primary label is at `span`, with no label text yet.
    pub fn at(code: &'static Code, span: Span, message: impl Into<String>) -> Self {
        Self::new(code, message).label(span, "")
    }

    /// Add a label. The first one added is the primary label.
    pub fn label(mut self, span: Span, text: impl Into<String>) -> Self {
        self.labels.push((span, text.into()));
        self
    }

    /// Set the text of the primary label.
    pub fn note(mut self, text: impl Into<String>) -> Self {
        if let Some(l) = self.labels.first_mut() {
            l.1 = text.into();
        }
        self
    }

    pub fn help(mut self, text: impl Into<String>) -> Self {
        self.help = Some(text.into());
        self
    }

    pub fn note_line(mut self, text: impl Into<String>) -> Self {
        self.notes.push(text.into());
        self
    }

    /// `help`, for a diagnostic that is already stored somewhere.
    pub fn set_help(&mut self, text: impl Into<String>) -> &mut Self {
        self.help = Some(text.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(Diagnostic::is_error)
}

/// A source file plus a line index, for turning byte offsets into positions.
pub struct Source {
    pub name: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl Source {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Source { name: name.into(), text, line_starts }
    }

    /// Zero-based line containing byte `offset`.
    pub fn line_of(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= offset.min(self.text.len())) - 1
    }

    /// Byte range of a zero-based line, without its line ending.
    pub fn line_range(&self, line: usize) -> (usize, usize) {
        let start = self.line_starts[line];
        let end = self.line_starts.get(line + 1).map_or(self.text.len(), |&e| e - 1);
        let end = if self.text[start..end].ends_with('\r') { end - 1 } else { end };
        (start, end)
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
}

/// Every source file of a build in one byte-offset space. File `k` occupies
/// `base(k) .. base(k) + len`, and the next file starts one byte later, so a
/// `Span` names its file by position alone and stays two words wide. The first
/// file starts at 0: a one-file build has the same spans as before.
#[derive(Default)]
pub struct SourceMap {
    files: Vec<(usize, Source)>,
}

impl SourceMap {
    pub fn new() -> Self {
        SourceMap::default()
    }

    /// A map of one file.
    pub fn single(name: impl Into<String>, text: impl Into<String>) -> Self {
        let mut m = SourceMap::new();
        m.add(name, text);
        m
    }

    /// The offset the next added file will start at.
    pub fn next_base(&self) -> usize {
        self.files.last().map_or(0, |(b, s)| b + s.text.len() + 1)
    }

    /// Add a file; returns its id and its base offset.
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> (usize, usize) {
        let base = self.next_base();
        self.files.push((base, Source::new(name, text)));
        (self.files.len() - 1, base)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn base(&self, file: usize) -> usize {
        self.files[file].0
    }

    pub fn source(&self, file: usize) -> &Source {
        &self.files[file].1
    }

    /// The file holding global offset `offset` (the last file for one past the end).
    pub fn file_of(&self, offset: usize) -> usize {
        self.files.partition_point(|(b, _)| *b <= offset).saturating_sub(1)
    }

    /// The file of a span and the span within that file.
    pub fn locate(&self, span: Span) -> (usize, Span) {
        let f = self.file_of(span.start);
        let b = self.base(f);
        let start = span.start - b;
        (f, Span::new(start, span.end.saturating_sub(b).max(start)))
    }

    /// `name:line` of a global offset (run-time messages, which print no column).
    pub fn file_line(&self, offset: usize) -> String {
        let f = self.file_of(offset);
        let src = self.source(f);
        format!("{}:{}", src.name, src.line_of(offset - self.base(f)) + 1)
    }

    /// `name:line:column` of a global offset.
    pub fn position(&self, offset: usize) -> String {
        let f = self.file_of(offset);
        let src = self.source(f);
        let local = offset - self.base(f);
        let line = src.line_of(local);
        let (ls, le) = src.line_range(line);
        let col = src.text[ls..local.min(le)].chars().count() + 1;
        format!("{}:{}:{col}", src.name, line + 1)
    }
}
