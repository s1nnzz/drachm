use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
}

#[derive(Debug)]
pub struct Error {
    pub span: Span,
    pub msg: String,
}

impl Error {
    pub fn new(span: Span, msg: impl Into<String>) -> Self {
        Error { span, msg: msg.into() }
    }

    /// Render as `file:line:col: error: msg` followed by the offending source line.
    pub fn render(&self, file: &str, src: &str) -> String {
        let start = self.span.start.min(src.len());
        let line_start = src[..start].rfind('\n').map_or(0, |p| p + 1);
        let line_end = src[start..].find('\n').map_or(src.len(), |p| start + p);
        let line_no = src[..start].matches('\n').count() + 1;
        let col = src[line_start..start].chars().count() + 1;
        let width = src[start..self.span.end.min(line_end).max(start)].chars().count().max(1);
        format!(
            "{file}:{line_no}:{col}: error: {}\n{:>4} | {}\n     | {}{}",
            self.msg,
            line_no,
            &src[line_start..line_end],
            " ".repeat(col - 1),
            "^".repeat(width)
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}
