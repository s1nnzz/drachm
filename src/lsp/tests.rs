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
