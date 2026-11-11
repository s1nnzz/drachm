//! `build_unchecked`: the way tests build programs that use borrows, which
//! `build` refuses until the borrow checker exists (WP7). Every other program
//! is built and compared on both backends by `tests/run.rs`.

use std::env::consts::EXE_SUFFIX;
use std::fs;
use std::path::Path;
use std::process::Command;

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Programs with raw pointers: `build` refuses them until WP9, so they go
/// straight to the code generators. Each runs on both backends
/// with the string runtime's leak check on.
const PROGRAMS: &[(&str, &str, &str)] = &[
    (
        "b1",
        r####"//--!$unsafe: true
in io;
struct P { public: uint32 hp; }
unsafe none set(*uint32 p, uint32 v) { *p = v; }
unsafe uint32 peek(*uint32 p) { return *p; }
none main() {
    mut uint32 n = 1;
    mut P q = 3;
    *uint32 p;
    *uint32 h;
    unsafe {
        p = address of n;
        *p = 7;
        set(copy p, peek(copy p) + 1);
    }
    io::outln(n::text());
    unsafe {
        h = address of q.hp;
        *h = *h + 4;
    }
    io::outln(q.hp::text());
    unsafe { io::outln((p == none)::text()); }
}
"####,
        "8\n7\nfalse\n",
    ),
];

#[test]
fn raw_pointers_through_mir() {
    for (name, source, want) in PROGRAMS {
        let (checked, diags) = drachm::check(source);
        let hir = checked.unwrap_or_else(|| panic!("{name} does not check: {:?}", diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>()));
        for optimise in [false, true] {
            let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("be-{name}-{}{EXE_SUFFIX}", optimise as u8));
            drachm::build_unchecked(&hir, &exe, optimise).unwrap_or_else(|e| panic!("{name} (-O {optimise}): {e}"));
            let run = Command::new(&exe).env("DRACHM_CHECK_LEAKS", "1").output().unwrap();
            let _ = fs::remove_file(&exe);
            assert_eq!(text(&run.stdout), *want, "{name} (-O {optimise})");
            let err = text(&run.stderr);
            assert!(!err.contains("leaked") && !err.contains("double free"), "{name} (-O {optimise}): {err}");
        }
    }
}
