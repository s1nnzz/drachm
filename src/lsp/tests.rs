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

#[test]
fn a_project_with_c_imports_is_analysed_without_noise() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/c_structs");
    let entry = dir.join("src/main.drh");
    let a = Analysis::run(&entry, &HashMap::new());
    assert_eq!(a.entry, entry);
    let all = a.lsp_diagnostics();
    assert!(all.iter().all(|(_, d)| d.is_empty()), "{all:?}");
    // A mistake in the file is still found, with the C names resolved.
    let text = std::fs::read_to_string(&entry).unwrap().replace("p.x += 10;", "p.x += \"ten\";");
    let a = Analysis::run(&entry, &HashMap::from([(entry.clone(), text)]));
    let all = a.lsp_diagnostics();
    assert!(all.iter().any(|(_, d)| d.iter().any(|d| d.code == Some(NumberOrString::String("E0101".into())))), "{all:?}");
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

/// The labels offered where `$` stands in `text`.
fn completions(text: &str) -> Vec<String> {
    let off = text.find('$').expect("a $ marks the cursor");
    let clean = text.replacen('$', "", 1);
    let path = PathBuf::from("/nonexistent-drachm-test/main.drh");
    let docs = HashMap::from([(path.clone(), clean)]);
    Analysis::complete_at(&path, &docs, off).into_iter().map(|i| i.label).collect()
}

#[test]
fn methods_after_the_colons() {
    let got = completions(&SHAPES.replace("p::sum()", "p::$"));
    assert!(got.contains(&"sum".to_string()), "{got:?}");
    assert!(!got.contains(&"origin".to_string()), "static methods are not offered on a value: {got:?}");
    assert!(!got.contains(&"add".to_string()), "{got:?}");
    // A word already typed does not change what is offered.
    assert!(completions(&SHAPES.replace("p::sum()", "p::su$")).contains(&"sum".to_string()));
}

#[test]
fn fields_after_the_dot() {
    let got = completions(&SHAPES.replace("p.x, p.y", "p.$, p.y"));
    assert_eq!(got, vec!["x".to_string(), "y".to_string()]);
}

#[test]
fn static_methods_after_a_struct_name_and_variants_after_an_enum_name() {
    let got = completions(&SHAPES.replace("p::sum()", "Point::$"));
    assert_eq!(got, vec!["origin".to_string()]);
    let got = completions(&SHAPES.replace("= North", "= Dir::$"));
    assert_eq!(got, vec!["North".to_string(), "East".to_string()]);
}

#[test]
fn string_list_and_map_methods() {
    let src = "none main() {\n    string s = \"a\";\n    sint32[?] xs;\n    s::$\n}\n";
    let got = completions(src);
    assert!(got.contains(&"upper".to_string()) && got.contains(&"split".to_string()), "{got:?}");
    let got = completions(&src.replace("s::$", "xs::$"));
    assert!(got.contains(&"push".to_string()) && !got.contains(&"upper".to_string()), "{got:?}");
}

#[test]
fn bare_variants_from_the_expected_type() {
    // In a declaration, an assignment, a comparison and an argument.
    let got = completions(&SHAPES.replace("= North", "= $"));
    assert_eq!(&got[..2], &["North".to_string(), "East".to_string()], "{got:?}");
    let got = completions(&SHAPES.replace("    Dir d = North;\n", "    Dir d = North;\n    d = $;\n"));
    assert_eq!(&got[..2], &["North".to_string(), "East".to_string()], "{got:?}");
    let got = completions(&SHAPES.replace("    Dir d = North;\n", "    Dir d = North;\n    if (d == $) { }\n"));
    assert_eq!(&got[..2], &["North".to_string(), "East".to_string()], "{got:?}");
}

#[test]
fn bare_variants_in_a_match() {
    let src = SHAPES.replace("    Dir d = North;\n", "    Dir d = North;\n    match (d) {\n        case $\n    }\n");
    let got = completions(&src);
    assert_eq!(&got[..2], &["North".to_string(), "East".to_string()], "{got:?}");
}

#[test]
fn names_in_scope_and_keywords() {
    let got = completions(&SHAPES.replace("    io::outln", "    $\n    io::outln"));
    for want in ["p", "total", "s", "d", "add", "Point", "Dir", "while", "uint32"] {
        assert!(got.contains(&want.to_string()), "{want} missing in {got:?}");
    }
    // A local is not offered before it is declared.
    let got = completions(&SHAPES.replace("    sint32 total", "    $\n    sint32 total"));
    assert!(got.contains(&"p".to_string()) && !got.contains(&"total".to_string()), "{got:?}");
}

#[test]
fn completion_in_half_typed_code() {
    // The block is never closed.
    let src = SHAPES.replace("p::sum()", "p::$").trim_end_matches("}\n").to_string();
    assert!(completions(&src).contains(&"sum".to_string()));
}

#[test]
fn nothing_inside_a_string_or_comment() {
    assert!(completions("none main() {\n    string s = \"p::$\";\n}\n").is_empty());
}

#[test]
fn references_of_locals_functions_methods_and_structs() {
    let (a, p, off) = marked(&SHAPES.replace("io::outln(total", "io::outln($total"));
    assert_eq!(a.references(&p, off, true).len(), 2);
    assert_eq!(a.references(&p, off, false).len(), 1);
    let (a, p, off) = marked(&SHAPES.replace("= add(", "= $add("));
    assert_eq!(a.references(&p, off, true).len(), 2);
    let (a, p, off) = marked(&SHAPES.replace("p::sum", "p::$sum"));
    assert_eq!(a.references(&p, off, true).len(), 2);
    // The struct's name: its declaration, the return type, `new Point`, and `Point p`.
    let (a, p, off) = marked(&SHAPES.replace("    Point p =", "    $Point p ="));
    let refs = a.references(&p, off, true);
    assert_eq!(refs.len(), 4, "{refs:?}");
}

#[test]
fn references_across_files_of_a_project() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/several_files");
    let main = dir.join("src/main.drh");
    let text = std::fs::read_to_string(&main).unwrap();
    let a = Analysis::run(&main, &HashMap::new());
    let off = text.find("greeting").unwrap() + 2;
    let refs = a.references(&main, off, true);
    assert_eq!(refs.len(), 2, "{refs:?}");
    let uris: Vec<String> = refs.iter().map(|r| r.uri.as_str().to_string()).collect();
    assert!(uris.iter().any(|f| f.ends_with("util.drh")) && uris.iter().any(|f| f.ends_with("main.drh")), "{uris:?}");
}

#[test]
fn rename_changes_the_declaration_and_every_use() {
    let (a, p, off) = marked(&SHAPES.replace("sint32 total", "sint32 tot$al"));
    let edits = a.rename(&p, off, "sum_all").expect("a local renames");
    let all: Vec<&lsp_types::TextEdit> = edits.values().flatten().collect();
    assert_eq!(all.len(), 2, "{all:?}");
    assert!(all.iter().all(|e| e.new_text == "sum_all"));
}

#[test]
fn rename_a_struct_renames_its_type_uses() {
    let (a, p, off) = marked(&SHAPES.replace("    Point p =", "    Poi$nt p ="));
    let edits = a.rename(&p, off, "Vec2").expect("a struct renames");
    let n: usize = edits.values().map(Vec::len).sum();
    assert_eq!(n, 4, "{edits:?}");
}

#[test]
fn rename_changes_every_file_of_the_project() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/several_files");
    let main = dir.join("src/main.drh");
    let text = std::fs::read_to_string(&main).unwrap();
    let a = Analysis::run(&main, &HashMap::new());
    let off = text.find("greeting").unwrap() + 2;
    let edits = a.rename(&main, off, "hello").expect("a function of the project renames");
    assert_eq!(edits.len(), 2, "{edits:?}");
    // The qualified use `util::greeting` changes its last word only.
    let (uri, main_edits) = edits.iter().find(|(u, _)| u.as_str().ends_with("main.drh")).unwrap();
    assert!(uri.as_str().ends_with("main.drh"));
    let line = text.lines().position(|l| l.contains("util::greeting")).unwrap();
    let col = text.lines().nth(line).unwrap().find("greeting").unwrap();
    assert_eq!(main_edits[0].range.start, lsp_types::Position { line: line as u32, character: col as u32 });
}

#[test]
fn rename_refuses_what_it_cannot_do() {
    let (a, p, off) = marked(&SHAPES.replace("sint32 total", "sint32 tot$al"));
    assert!(a.rename(&p, off, "while").unwrap_err().contains("keyword"));
    assert!(a.rename(&p, off, "2fast").unwrap_err().contains("not a name"));
    assert!(a.rename(&p, off, "uint32").unwrap_err().contains("type"));
    let (a, p, off) = marked(&SHAPES.replace("return self.x", "return $self.x"));
    assert!(a.rename(&p, off, "me").unwrap_err().contains("`self`"));
    // A name the program does not declare (a built-in) has nothing to rename.
    let (a, p, off) = marked(&SHAPES.replace("io::outln", "io::$outln"));
    assert!(a.rename(&p, off, "say").is_err());
}
