//! Tests of the language server's analysis, driven without an editor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp_types::{DiagnosticSeverity, NumberOrString};

use super::Analysis;

/// Analyse one in-memory file that does not exist on disk.
pub fn of_source(text: &str) -> (Analysis, PathBuf) {
    let path = PathBuf::from("/nonexistent-drachm-test/main.drh");
    let overlay = HashMap::from([(path.clone(), text.to_string())]);
    (Analysis::run(&path, &overlay), path)
}

#[test]
fn diagnostics_of_one_buffer() {
    let (a, _) = of_source("none main() {\n  uint32 x = \"hi\";\n}\n");
    let all = a.lsp_diagnostics();
    assert_eq!(all.len(), 1);
    let d = &all[0].1;
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, Some(NumberOrString::String("E0101".into())));
    assert_eq!(d[0].range.start.line, 1);
    assert_eq!(d[0].severity, Some(DiagnosticSeverity::ERROR));
}

#[test]
fn warnings_are_warnings() {
    let (a, _) = of_source("none main() {\n  uint8 x = 300;\n}\n");
    let d = &a.lsp_diagnostics()[0].1;
    assert!(d.iter().any(|d| d.severity == Some(DiagnosticSeverity::WARNING)), "{d:?}");
}

#[test]
fn a_project_file_is_checked_with_its_project() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/several_files");
    let entry = dir.join("src/main.drh");
    let util = dir.join("src/util.drh");
    // The buffer of the second file differs from the disk: it gets an error.
    let text = std::fs::read_to_string(&util).unwrap() + "\nnone broken() { uint32 bad = \"x\"; }\n";
    let overlay = HashMap::from([(util.clone(), text)]);
    let a = Analysis::run(&util, &overlay);
    assert!(a.files.len() > 1, "{:?}", a.files);
    assert_eq!(a.entry, entry);
    let all = a.lsp_diagnostics();
    assert!(all.iter().any(|(f, d)| f.ends_with("util.drh") && !d.is_empty()), "{all:?}");
    assert!(all.iter().any(|(f, d)| f.ends_with("main.drh") && d.is_empty()), "{all:?}");
}

fn parse(src: &str) -> (crate::ast::Program, Vec<crate::diag::Diagnostic>) {
    let mut diags = Vec::new();
    let tokens = crate::lexer::lex(src, &mut diags);
    let program = crate::parser::parse(tokens, &mut diags);
    (program, diags)
}

#[test]
fn a_half_typed_function_stays_in_the_tree() {
    let (p, diags) = parse("none f() {\n  uint32 x = 1;\n  uint32 y = 2;\n\nnone main() { }\n");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code.id, "E0004");
    let f = p.funcs.iter().find(|f| f.name == "f").expect("f is kept");
    assert_eq!(f.body.stmts.len(), 2);
    assert!(p.funcs.iter().any(|f| f.name == "main"));
}

#[test]
fn later_errors_still_show_after_a_bad_statement() {
    let (p, diags) = parse("none f() {\n  uint32 x = ;\n  uint32 y = 2;\n}\nnone g() {\n  uint32 z = ;\n}\nnone main() { }\n");
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert_eq!(p.funcs.len(), 3);
    assert_eq!(p.funcs[0].body.stmts.len(), 1);
}

#[test]
fn a_half_typed_struct_keeps_its_members() {
    let (p, diags) = parse("struct P {\n  public:\n  uint32 x;\n  uint32 y;\n  uint32 sum() { return self.x + self.y;\n");
    assert!(diags.iter().any(|d| d.code.id == "E0004"));
    let s = &p.structs[0];
    assert_eq!(s.fields.len(), 2);
    assert_eq!(s.methods.len(), 1);
}

/// Analyse `text` with a `$` marking the cursor; returns the analysis, the
/// path and the cursor's byte offset.
fn marked(text: &str) -> (Analysis, PathBuf, usize) {
    let off = text.find('$').expect("a $ marks the cursor");
    let clean = text.replacen('$', "", 1);
    let (a, p) = of_source(&clean);
    (a, p, off)
}

const SHAPES: &str = "in io;\n\
struct Point {\n    public:\n    sint32 x;\n    sint32 y;\n    sint32 sum() { return self.x + self.y; }\n    static Point origin() { return new Point of (0, 0); }\n}\n\
enum Dir { North, East(sint32 steps), }\n\
sint32 add(sint32 a, sint32 b) { return a + b; }\n\
none main() {\n    Point p = 3, 4;\n    sint32 total = add(p.x, p.y);\n    sint32 s = p::sum();\n    Dir d = North;\n    io::outln(total::text());\n}\n";

fn hover_text(src: &str) -> String {
    let (a, p, off) = marked(src);
    a.hover(&p, off).map(|h| h.markdown).unwrap_or_default()
}

fn goto(src: &str) -> Option<(usize, usize)> {
    let (a, p, off) = marked(src);
    let d = a.definition(&p, off)?;
    Some((d.span.start, d.span.end))
}

#[test]
fn the_sample_program_checks() {
    let (a, _) = of_source(SHAPES);
    assert!(a.diags.iter().all(|d| !d.is_error()), "{:?}", a.diags.iter().map(|d| &d.message).collect::<Vec<_>>());
}

#[test]
fn hover_shows_local_types() {
    let h = hover_text(&SHAPES.replace("add(p.x", "add($p.x"));
    assert!(h.contains("Point p"), "{h}");
    let h = hover_text(&SHAPES.replace("sint32 total", "sint32 tot$al"));
    assert!(h.contains("sint32 total"), "{h}");
}

#[test]
fn hover_shows_fields_methods_and_functions() {
    assert!(hover_text(&SHAPES.replace("p.x, p.y", "p.$x, p.y")).contains("sint32 x"));
    assert!(hover_text(&SHAPES.replace("p::sum", "p::s$um")).contains("sint32 sum()"));
    let h = hover_text(&SHAPES.replace("= add(", "= ad$d("));
    assert!(h.contains("sint32 add(sint32 a, sint32 b)"), "{h}");
}

#[test]
fn hover_shows_expression_types() {
    let h = hover_text(&SHAPES.replace("return a + b", "return a $+ b"));
    assert!(h.contains("a + b: sint32"), "{h}");
}

#[test]
fn hover_on_a_type_name_shows_the_struct() {
    let h = hover_text(&SHAPES.replace("    Point p =", "    Poi$nt p ="));
    assert!(h.contains("struct Point") && h.contains("sint32 x"), "{h}");
}

#[test]
fn hover_shows_a_variant() {
    let h = hover_text(&SHAPES.replace("= North", "= Nor$th"));
    assert!(h.contains("Dir::North"), "{h}");
}

#[test]
fn definition_of_locals_functions_fields_methods_variants_and_structs() {
    let clean = SHAPES.to_string();
    // A local: the declaration.
    let (s, _) = goto(&SHAPES.replace("add(p.x", "add($p.x")).unwrap();
    assert_eq!(s, clean.find("p = 3, 4").unwrap());
    // A function.
    let (s, e) = goto(&SHAPES.replace("= add(", "= ad$d(")).unwrap();
    assert_eq!(&clean[s..e], "add");
    assert_eq!(s, clean.find("add(sint32 a").unwrap());
    // A field.
    let (s, e) = goto(&SHAPES.replace("p.x, p.y", "p.$x, p.y")).unwrap();
    assert_eq!(&clean[s..e], "x");
    assert_eq!(s, clean.find("x;").unwrap());
    // A method.
    let (s, _) = goto(&SHAPES.replace("p::sum", "p::s$um")).unwrap();
    assert_eq!(s, clean.find("sum()").unwrap());
    // A variant.
    let (s, e) = goto(&SHAPES.replace("= North", "= Nor$th")).unwrap();
    assert_eq!(&clean[s..e], "North");
    // A struct, from its type.
    let (s, e) = goto(&SHAPES.replace("    Point p =", "    Poi$nt p =")).unwrap();
    assert_eq!(&clean[s..e], "Point");
    assert_eq!(s, clean.find("Point {").unwrap());
}

#[test]
fn hover_works_in_half_typed_code() {
    // The line under the cursor is unfinished and the function never closes.
    let src = SHAPES.replace("    sint32 s = p::sum();\n", "    sint32 s = p::sum();\n    sint32 q = \n").replace("add(p.x", "add($p.x");
    let src = src.trim_end_matches("}\n").to_string();
    let (a, p, off) = marked(&src);
    assert!(a.diags.iter().any(|d| d.is_error()));
    let h = a.hover(&p, off).map(|h| h.markdown).unwrap_or_default();
    assert!(h.contains("Point p"), "{h}");
}

#[test]
fn hover_and_definition_across_files() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/several_files");
    let main = dir.join("src/main.drh");
    let text = std::fs::read_to_string(&main).unwrap();
    let a = Analysis::run(&main, &HashMap::new());
    assert!(a.diags.iter().all(|d| !d.is_error()));
    let off = text.find("greeting").unwrap() + 2;
    let h = a.hover(&main, off).expect("hover on greeting");
    assert!(h.markdown.contains("greeting("), "{}", h.markdown);
    let d = a.definition(&main, off).expect("definition of greeting");
    assert!(a.files[d.file].ends_with("util.drh"), "{:?}", a.files[d.file]);
    let util = std::fs::read_to_string(dir.join("src/util.drh")).unwrap();
    assert_eq!(d.span.start, util.find("greeting").unwrap());
}
