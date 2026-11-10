//! Diagnostics: what every compiler phase reports instead of stopping at the
//! first problem. Rendering lives in `render`, the code table in `codes`.

pub mod codes;
mod render;

pub use codes::Code;
pub use render::{render, render_all};

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
