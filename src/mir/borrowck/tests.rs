//! The false-positive net: every program the compiler accepts today must pass
//! the borrow checker, and a few programs it must reject.

use std::path::Path;

fn borrow_errors(src: &str) -> Option<Vec<String>> {
    let (hir, _) = crate::front(src);
    let hir = hir?;
    let mut diags = Vec::new();
    let mir = crate::mir::build_program(&hir, &mut diags);
    if diags.iter().any(|d| d.is_error()) {
        return None;
    }
    let mut out = Vec::new();
    super::check_program(&hir, &mir, &mut out);
    Some(out.iter().map(|d| format!("{}: {}", d.code.id, d.message)).collect())
}

fn files(dir: &str) -> Vec<std::path::PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut v: Vec<_> = std::fs::read_dir(root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "drh")).collect();
    v.sort();
    v
}

#[test]
fn accepted_programs_have_no_borrow_errors() {
    let mut bad = Vec::new();
    let mut seen = 0;
    for dir in ["tests/run", "examples", "site/examples", "tests/mir", "bench"] {
        for f in files(dir) {
            let src = std::fs::read_to_string(&f).unwrap();
            if let Some(errors) = borrow_errors(&src) {
                seen += 1;
                if !errors.is_empty() {
                    bad.push(format!("{}: {errors:?}", f.display()));
                }
            }
        }
    }
    assert!(seen > 100, "only {seen} programs were checked");
    assert!(bad.is_empty(), "false borrow errors:\n{}", bad.join("\n"));
}

#[test]
fn rejected_programs_have_borrow_errors() {
    let cases = [
        ("none main() { mut string n = \"a\"; &string f = &n; n = \"b\"; io::outln(f); }", "E0302"),
        ("none main() { mut uint32 n = 1; &mut uint32 a = &mut n; &mut uint32 b = &mut n; a = 2; b = 3; }", "E0303"),
        ("&string f() { string s = \"a\"; return &s; }\nnone main() { }", "E0307"),
    ];
    for (body, code) in cases {
        let src = format!("in io;\n{body}\n");
        let errors = borrow_errors(&src).unwrap_or_else(|| panic!("{body} does not build"));
        assert!(errors.iter().any(|e| e.starts_with(code)), "{body}: {errors:?}");
    }
}
