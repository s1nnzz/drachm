//! C header imports (step 7.1) beyond the golden projects in `tests/projects/c_*`:
//! libraries without `main()`, scripts with default header paths, link
//! requirements inherited from a dependency, and the messages for a missing
//! library. Each test works in a temp folder of its own; the C fixture is
//! `tests/c/drachm_fixture.{h,c}`, so no system library is needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("drachm-c-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn put(&self, rel: &str, text: &str) {
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn drachm(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_drachm")).current_dir(cwd).env("NO_COLOR", "1").args(args).output().expect("could not start drachm")
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).replace("\r\n", "\n")
}

/// The fixture's folder as a TOML string (absolute, `/` on every system).
fn fixture_dir() -> String {
    root().join("tests/c").display().to_string().replace('\\', "/")
}

#[test]
fn a_library_checks_without_main_and_cannot_be_run() {
    let dir = root().join("tests/c_projects/library");
    let check = drachm(&root(), &["check", dir.to_str().unwrap()]);
    assert!(check.status.success(), "{}", text(&check.stderr));
    let build = drachm(&root(), &["build", dir.to_str().unwrap()]);
    assert!(!build.status.success());
    assert!(text(&build.stderr).contains("E0811"), "{}", text(&build.stderr));
}

#[test]
fn a_script_checks_and_runs_with_the_default_header_paths() {
    let s = Scratch::new("script");
    s.put(
        "calc.drh",
        "//--!$unsafe: true\nin io;\nin * from c.\"stdlib.h\" as libc;\nin * from c.\"math.h\" as m;\n\nunsafe {\n    c::int a = libc::abs(-5);\n    io::outln(f\"abs: ${a as sint32}\");\n    io::outln(f\"pow: ${m::pow(2.0, 10.0) as float64}\");\n}\n",
    );
    let check = drachm(&s.dir, &["check", "calc.drh"]);
    assert!(check.status.success(), "{}", text(&check.stderr));
    for flags in [&[][..], &["-O"][..]] {
        let mut args = vec!["run", "calc.drh"];
        args.extend(flags);
        let run = drachm(&s.dir, &args);
        assert!(run.status.success(), "{flags:?}: {}", text(&run.stderr));
        assert_eq!(text(&run.stdout), "abs: 5\npow: 1024.0\n", "{flags:?}");
    }
}

#[test]
fn a_dependency_brings_its_native_link_requirements() {
    let s = Scratch::new("inherit");
    s.put(
        "cdep/drachm.toml",
        "[project]\nname = \"cdep\"\nkind = \"library\"\npublic_modules = [\"wrapped\"]\n\n[native]\ninclude = [\"inc\"]\nsources = [\"c/impl.c\"]\n",
    );
    s.put("cdep/inc/dep.h", "#ifndef DEP_H\n#define DEP_H\nint dep_triple(int x);\n#endif\n");
    s.put("cdep/c/impl.c", "#include \"dep.h\"\nint dep_triple(int x) { return x * 3; }\n");
    s.put("cdep/src/main.drh", "none unused() { }\n");
    s.put(
        "cdep/src/wrapped.drh",
        "in * from c.\"dep.h\" as dep;\n\nout sint32 triple(sint32 x) {\n    unsafe {\n        return dep::dep_triple(x as c::int) as sint32;\n    }\n}\n",
    );
    s.put("app/drachm.toml", "[project]\nname = \"app\"\n\n[dependencies]\ncdep = { path = \"../cdep\" }\n");
    s.put("app/src/main.drh", "//--!$unsafe: true\nin io;\nin cdep.wrapped as w;\n\nnone main() {\n    io::outln(f\"triple: ${w::triple(14)}\");\n}\n");
    for flags in [&[][..], &["-O"][..]] {
        let mut args = vec!["run", "app"];
        args.extend(flags);
        let run = drachm(&s.dir, &args);
        assert!(run.status.success(), "{flags:?}: {}", text(&run.stderr));
        assert_eq!(text(&run.stdout), "triple: 42\n", "{flags:?}");
    }
}

#[test]
fn a_missing_library_is_named_with_the_manifest_table() {
    let s = Scratch::new("nolib");
    s.put("drachm.toml", &format!("[project]\nname = \"nolib\"\n\n[native]\ninclude = [\"{}\"]\nsources = [\"{}/drachm_fixture.c\"]\nlibraries = [\"drachm_no_such_library\"]\n", fixture_dir(), fixture_dir()));
    s.put("src/main.drh", "//--!$unsafe: true\nin io;\nin * from c.\"drachm_fixture.h\" as fx;\n\nnone main() {\n    unsafe { io::outln(f\"${fx::fx_add(1, 2) as sint32}\"); }\n}\n");
    for flags in [&[][..], &["-O"][..]] {
        let mut args = vec!["build", "."];
        args.extend(flags);
        let build = drachm(&s.dir, &args);
        assert!(!build.status.success(), "{flags:?} should not link");
        let err = text(&build.stderr);
        assert!(err.contains("drachm_no_such_library"), "{err}");
        assert!(err.contains("[native]"), "{err}");
    }
}

#[test]
fn a_missing_c_source_is_reported() {
    let s = Scratch::new("nosource");
    s.put("drachm.toml", &format!("[project]\nname = \"nosource\"\n\n[native]\ninclude = [\"{}\"]\nsources = [\"missing.c\"]\n", fixture_dir()));
    s.put("src/main.drh", "none main() { }\n");
    let build = drachm(&s.dir, &["build", "."]);
    assert!(!build.status.success());
    assert!(text(&build.stderr).contains("missing.c"), "{}", text(&build.stderr));
}

#[test]
fn the_native_table_is_checked() {
    let s = Scratch::new("badnative");
    s.put("drachm.toml", "[project]\nname = \"bad\"\n\n[native]\nincludes = [\"x\"]\nlibraries = \"SDL2\"\n");
    s.put("src/main.drh", "none main() { }\n");
    let check = drachm(&s.dir, &["check", "."]);
    assert!(!check.status.success());
    let err = text(&check.stderr);
    assert!(err.contains("E0801") && err.contains("includes"), "{err}");
    assert!(err.contains("E0802") && err.contains("libraries"), "{err}");
}

#[test]
fn c_calls_do_not_hide_in_safe_code() {
    // The callee's unsafe call is inside an `unsafe` block of the wrapper; the caller needs none.
    let s = Scratch::new("safe");
    s.put("drachm.toml", &format!("[project]\nname = \"safe\"\n\n[native]\ninclude = [\"{}\"]\nsources = [\"{}/drachm_fixture.c\"]\n", fixture_dir(), fixture_dir()));
    s.put(
        "src/main.drh",
        "//--!$unsafe: true\nin io;\nin * from c.\"drachm_fixture.h\" as fx;\n\nsint32 add(sint32 a, sint32 b) {\n    unsafe { return fx::fx_add(a as c::int, b as c::int) as sint32; }\n}\n\nnone main() {\n    io::outln(f\"${add(20, 22)}\");\n}\n",
    );
    let run = drachm(&s.dir, &["run", "."]);
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "42\n");
}
