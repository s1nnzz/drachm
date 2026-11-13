//! Renders diagnostics in the style of SPEC section 10:
//!
//! ```text
//! error[E0302]: cannot change `xs` while it is borrowed
//!   --> main.drh:5:5
//!    |
//!  4 |     &uint32 first = &xs[0];
//!    |                     ------ `xs` is borrowed here
//!  5 |     xs::push(9);
//!    |     ^^ change attempted here
//!    |
//!  help: finish using `first` before changing `xs`
//! ```

use super::{Diagnostic, Severity, Source, SourceMap};

const TAB: &str = "    ";

struct Paint(bool);

impl Paint {
    fn wrap(&self, code: &str, s: &str) -> String {
        if self.0 && !s.is_empty() { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
    }
    fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    fn gutter(&self, s: &str) -> String {
        self.wrap("1;34", s)
    }
    fn severity(&self, sev: Severity, s: &str) -> String {
        self.wrap(if sev == Severity::Error { "1;31" } else { "1;33" }, s)
    }
}

/// Display width of `s` as we print it (tabs become four spaces).
fn width(s: &str) -> usize {
    s.chars().map(|c| if c == '\t' { TAB.len() } else { 1 }).sum()
}

struct Mark<'a> {
    line: usize,
    /// Display column and width of the underline.
    col: usize,
    len: usize,
    text: &'a str,
    primary: bool,
}

pub fn render(d: &Diagnostic, src: &Source, color: bool) -> String {
    let p = Paint(color);
    let word = if d.severity == Severity::Error { "error" } else { "warning" };
    let mut out = format!(
        "{}{}\n",
        p.severity(d.severity, &format!("{word}[{}]", d.code.id)),
        p.bold(&format!(": {}", d.message))
    );

    let mut marks: Vec<Mark> = d
        .labels
        .iter()
        .enumerate()
        .map(|(i, (span, text))| {
            let start = span.start.min(src.text.len());
            let line = src.line_of(start);
            let (ls, le) = src.line_range(line);
            let start = start.min(le);
            // A span that runs past its first line is underlined to the line's end.
            let end = span.end.clamp(start, le);
            Mark {
                line,
                col: width(&src.text[ls..start]),
                len: width(&src.text[start..end]).max(1),
                text,
                primary: i == 0,
            }
        })
        .collect();

    // Gutter width follows the largest line number shown.
    let digits = marks.iter().map(|m| m.line + 1).max().unwrap_or(1).to_string().len();
    let pad = " ".repeat(digits + 1);
    let bar = p.gutter("|");

    match d.labels.first() {
        Some((span, _)) => {
            let start = span.start.min(src.text.len());
            let line = src.line_of(start);
            let (ls, le) = src.line_range(line);
            let col = src.text[ls..start.min(le)].chars().count() + 1;
            out += &format!("{pad}{} {}:{}:{col}\n", p.gutter("-->"), src.name, line + 1);
        }
        None => out += &format!("{pad}{} {}\n", p.gutter("-->"), src.name),
    }

    if !marks.is_empty() {
        out += &format!("{pad} {bar}\n");
        marks.sort_by_key(|m| (m.line, m.col));
        let mut prev: Option<usize> = None;
        for m in &marks {
            if prev != Some(m.line) {
                match prev {
                    // Show a single skipped line rather than an ellipsis for it.
                    Some(pl) if m.line == pl + 2 => out += &source_line(src, pl + 1, digits, &p),
                    Some(pl) if m.line > pl + 2 => out += &format!("{pad}{}\n", p.gutter("...")),
                    _ => {}
                }
                out += &source_line(src, m.line, digits, &p);
                prev = Some(m.line);
            }
            let underline = if m.primary { "^" } else { "-" }.repeat(m.len);
            let mut row = format!("{underline} {}", m.text);
            row.truncate(row.trim_end().len());
            let row = if m.primary { p.severity(d.severity, &row) } else { p.gutter(&row) };
            out += &format!("{pad} {bar} {}{row}\n", " ".repeat(m.col));
        }
    }

    if !marks.is_empty() && (d.help.is_some() || !d.notes.is_empty()) {
        out += &format!("{pad} {bar}\n");
    }
    for note in &d.notes {
        out += &format!(" {}: {note}\n", p.bold("note"));
    }
    if let Some(help) = &d.help {
        out += &format!(" {}: {help}\n", p.bold("help"));
    }
    out
}

fn source_line(src: &Source, line: usize, digits: usize, p: &Paint) -> String {
    let (ls, le) = src.line_range(line);
    let text = src.text[ls..le].replace('\t', TAB);
    let gutter = p.gutter(&format!(" {:>digits$} |", line + 1));
    format!("{gutter} {text}").trim_end().to_string() + "\n"
}

/// Every diagnostic in source order, then a one-line count.
pub fn render_all(diags: &[Diagnostic], src: &Source, color: bool) -> String {
    let mut order: Vec<&Diagnostic> = diags.iter().collect();
    order.sort_by_key(|d| d.labels.first().map_or(0, |l| l.0.start));
    let mut out = String::new();
    for d in &order {
        out += &render(d, src, color);
        out.push('\n');
    }
    let errors = diags.iter().filter(|d| d.is_error()).count();
    let warnings = diags.len() - errors;
    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(plural(errors, "error"));
    }
    if warnings > 0 {
        parts.push(plural(warnings, "warning"));
    }
    if !parts.is_empty() {
        out += &format!("{}: {}\n", src.name, parts.join(", "));
    }
    out
}

/// `render_all` for a build of several files. Each diagnostic is shown in the
/// file of its primary label; a label in another file becomes a note that
/// names the place (`also src/a.drh:3:1: the first import`).
pub fn render_all_map(diags: &[Diagnostic], map: &SourceMap, color: bool) -> String {
    let mut order: Vec<&Diagnostic> = diags.iter().collect();
    order.sort_by_key(|d| d.labels.first().map_or(0, |l| l.0.start));
    let mut out = String::new();
    for d in order {
        let file = d.labels.first().map_or(0, |l| map.file_of(l.0.start));
        let mut local = Diagnostic {
            severity: d.severity,
            code: d.code,
            message: d.message.clone(),
            labels: Vec::new(),
            notes: d.notes.clone(),
            help: d.help.clone(),
        };
        for (span, text) in &d.labels {
            if map.file_of(span.start) == file {
                local.labels.push((map.locate(*span).1, text.clone()));
            } else {
                let at = map.position(span.start);
                local.notes.push(if text.is_empty() { format!("also {at}") } else { format!("also {at}: {text}") });
            }
        }
        out += &render(&local, map.source(file), color);
        out.push('\n');
    }
    let errors = diags.iter().filter(|d| d.is_error()).count();
    let warnings = diags.len() - errors;
    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(plural(errors, "error"));
    }
    if warnings > 0 {
        parts.push(plural(warnings, "warning"));
    }
    if !parts.is_empty() && !map.is_empty() {
        out += &format!("{}: {}\n", map.source(0).name, parts.join(", "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::{codes, Span};

    fn span_of(src: &str, needle: &str) -> Span {
        let start = src.find(needle).unwrap();
        Span::new(start, start + needle.len())
    }

    #[test]
    fn spec_layout() {
        let text = "none main() {\n    mut uint32[?] xs = [1];\n\n    &uint32 first = &xs[0];\n    xs::push(9);\n    io::outln(first::text());\n}\n";
        let src = Source::new("main.drh", text);
        let push = span_of(text, "xs::push");
        let d = Diagnostic::at(&codes::E0205, Span::new(push.start, push.start + 2), "cannot change `xs` while it is borrowed")
            .note("change attempted here")
            .label(span_of(text, "&xs[0]"), "`xs` is borrowed here")
            .label(Span::new(text.find("first::").unwrap(), text.find("first::").unwrap() + 5), "the borrow is still used here")
            .help("finish using `first` before changing `xs`");
        let want = "\
error[E0205]: cannot change `xs` while it is borrowed
  --> main.drh:5:5
   |
 4 |     &uint32 first = &xs[0];
   |                     ------ `xs` is borrowed here
 5 |     xs::push(9);
   |     ^^ change attempted here
 6 |     io::outln(first::text());
   |               ----- the borrow is still used here
   |
 help: finish using `first` before changing `xs`
";
        assert_eq!(render(&d, &src, false), want);
    }

    #[test]
    fn gaps_tabs_and_no_labels() {
        let text = "a\n\tbb\nc\nd\ne\nf\ng\nh\ni\nlast";
        let src = Source::new("t.drh", text);
        let d = Diagnostic::at(&codes::E0003, span_of(text, "bb"), "m")
            .label(span_of(text, "d"), "near")
            .label(span_of(text, "last"), "far");
        let want = "\
error[E0003]: m
   --> t.drh:2:2
    |
  2 |     bb
    |     ^^
  3 | c
  4 | d
    | - near
   ...
 10 | last
    | ---- far
";
        assert_eq!(render(&d, &src, false), want);

        let d = Diagnostic::new(&codes::E0204, "no `main` function").help("add one");
        assert_eq!(render(&d, &src, false), "error[E0204]: no `main` function\n  --> t.drh\n help: add one\n");
    }

    #[test]
    fn source_map_two_files() {
        let mut map = SourceMap::new();
        let (a, abase) = map.add("src/main.drh", "in util;\nnone main() { }\n");
        let (b, bbase) = map.add("src/util.drh", "none f() { }\n");
        assert_eq!((a, abase, b, bbase), (0, 0, 1, 26));
        assert_eq!(map.file_of(25), 0);
        assert_eq!(map.file_of(26), 1);
        assert_eq!(map.position(bbase + 5), "src/util.drh:1:6");
        let d = Diagnostic::at(&codes::E0201, Span::new(abase + 3, abase + 7), "unknown module").label(Span::new(bbase + 5, bbase + 6), "defined here");
        let want = "\
error[E0201]: unknown module
  --> src/main.drh:1:4
   |
 1 | in util;
   |    ^^^^
   |
 note: also src/util.drh:1:6: defined here
";
        assert_eq!(render_all_map(&[d], &map, false), format!("{want}\nsrc/main.drh: 1 error\n"));
    }

    #[test]
    fn span_at_end_of_file() {
        let src = Source::new("t.drh", "fn main() {");
        let d = Diagnostic::at(&codes::E0004, Span::new(11, 11), "m");
        assert!(render(&d, &src, false).contains(" 1 | fn main() {\n   |            ^\n"));
    }
}
