//! `build_unchecked`: the way tests build programs that use borrows, which
//! `build` refuses until the borrow checker exists (WP7). Every other program
//! is built and compared on both backends by `tests/run.rs`.

use std::env::consts::EXE_SUFFIX;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// User-written borrows are refused by `build` until the borrow checker exists
/// (WP7); `build_unchecked` builds them, on both backends.
#[test]
fn borrows_through_mir() {
    let programs = [
        (
            "none bump(&mut uint32 n) { n = n + 1; }\nnone main() { mut uint32 a = 1; bump(&mut a); io::outln(a::text()); }\n",
            "2\n",
        ),
        (
            "struct Point { public: sint64 x; sint64 y; }\n\
             sint64 sum(&Point p) { return p.x + p.y; }\n\
             none shift(&mut Point p) { p.x = p.x + 10; p.y = p.y * 2; }\n\
             none main() {\n\
                 mut Point p = 3, 4;\n\
                 io::outln(sum(&p)::text());\n\
                 shift(&mut p);\n\
                 io::outln(sum(&p)::text());\n\
                 io::outln(p.x::text());\n\
             }\n",
            "7\n21\n13\n",
        ),
    ];
    for (i, (body, want)) in programs.iter().enumerate() {
        let source = format!("in io;\n{body}");
        let (checked, diags) = drachm::check(&source);
        let hir = checked.unwrap_or_else(|| panic!("program {i} does not check: {:?}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>()));
        assert!(hir.uses_borrows, "program {i} was meant to use borrows");
        let strict = drachm::build(&hir, &Path::new(env!("CARGO_TARGET_TMPDIR")).join("be-no"), false);
        assert!(strict.is_err(), "build must still refuse user borrows");
        for optimise in [false, true] {
            let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("be-borrow-{i}-{}{EXE_SUFFIX}", optimise as u8));
            drachm::build_unchecked(&hir, &exe, optimise).unwrap_or_else(|e| panic!("program {i} (-O {optimise}): {e}"));
            let run = Command::new(&exe).stdin(Stdio::null()).output().unwrap();
            let _ = fs::remove_file(&exe);
            assert_eq!(text(&run.stdout), *want, "program {i} (-O {optimise})");
        }
    }
}
