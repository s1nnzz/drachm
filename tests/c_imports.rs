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
        "[project]\nname = \"cdep\"\nkind = \"library\"\npublic_modules = [\"wrapped\"]\n\n[native]\ninclude = [\"inc\"]\nsources = [\"c/impl.c\"]\nunsafe = true\n",
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

#[test]
fn a_missing_libclang_is_reported_and_only_where_a_header_is_imported() {
    let with_c = root().join("tests/projects/c_scalars");
    let out = Command::new(env!("CARGO_BIN_EXE_drachm"))
        .current_dir(root())
        .env("NO_COLOR", "1")
        .env("LIBCLANG_PATH", "/nonexistent-drachm-libclang")
        .args(["check", with_c.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("E0700"), "{}", text(&out.stderr));
    // A program without a C import never loads libclang.
    let plain = root().join("tests/projects/single");
    let out = Command::new(env!("CARGO_BIN_EXE_drachm"))
        .current_dir(root())
        .env("LIBCLANG_PATH", "/nonexistent-drachm-libclang")
        .args(["check", plain.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
}

#[test]
fn the_manifest_allows_unsafe_for_its_project_and_for_its_dependencies() {
    // `[native] unsafe = true` in the dependency lets its wrapper use C with no
    // file flag; the app needs nothing, and its own files have no flag either.
    let s = Scratch::new("manifest_dep");
    s.put(
        "cdep/drachm.toml",
        "[project]\nname = \"cdep\"\nkind = \"library\"\npublic_modules = [\"wrapped\"]\n\n[native]\ninclude = [\"inc\"]\nsources = [\"c/impl.c\"]\nunsafe = true\n",
    );
    s.put("cdep/inc/dep.h", "#ifndef DEP_H\n#define DEP_H\nint dep_triple(int x);\n#endif\n");
    s.put("cdep/c/impl.c", "#include \"dep.h\"\nint dep_triple(int x) { return x * 3; }\n");
    s.put("cdep/src/main.drh", "none unused() { }\n");
    s.put(
        "cdep/src/wrapped.drh",
        "in * from c.\"dep.h\" as dep;\n\nout sint32 triple(sint32 x) {\n    unsafe {\n        return dep::dep_triple(x as c::int) as sint32;\n    }\n}\n",
    );
    s.put("app/drachm.toml", "[project]\nname = \"app\"\n\n[dependencies]\ncdep = { path = \"../cdep\" }\n");
    s.put("app/src/main.drh", "in io;\nin cdep.wrapped as w;\n\nnone main() {\n    io::outln(f\"triple: ${w::triple(14)}\");\n}\n");
    for flags in [&[][..], &["-O"][..]] {
        let mut args = vec!["run", "app"];
        args.extend(flags);
        let run = drachm(&s.dir, &args);
        assert!(run.status.success(), "{flags:?}: {}", text(&run.stderr));
        assert_eq!(text(&run.stdout), "triple: 42\n", "{flags:?}");
    }
}

#[test]
fn the_manifest_allows_unsafe_in_every_file_of_the_project() {
    let s = Scratch::new("manifest_own");
    s.put("drachm.toml", &format!("[project]\nname = \"own\"\n\n[native]\ninclude = [\"{}\"]\nsources = [\"{}/drachm_fixture.c\"]\nunsafe = true\n", fixture_dir(), fixture_dir()));
    s.put("src/main.drh", "in io;\nin other;\n\nnone main() {\n    io::outln(f\"${other::add(20, 22)}\");\n}\n");
    s.put("src/other.drh", "in * from c.\"drachm_fixture.h\" as fx;\n\nout sint32 add(sint32 a, sint32 b) {\n    unsafe { return fx::fx_add(a as c::int, b as c::int) as sint32; }\n}\n");
    let run = drachm(&s.dir, &["run", "."]);
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "42\n");
}

#[test]
fn the_file_flag_is_per_file_and_a_file_without_it_is_refused() {
    // A flag in one file does not allow `unsafe` in the others of the project.
    let s = Scratch::new("perfile");
    s.put("drachm.toml", &format!("[project]\nname = \"perfile\"\n\n[native]\ninclude = [\"{}\"]\nsources = [\"{}/drachm_fixture.c\"]\n", fixture_dir(), fixture_dir()));
    s.put("src/main.drh", "//--!$unsafe: true\nin io;\nin other;\n\nnone main() {\n    io::outln(f\"${other::add(20, 22)}\");\n}\n");
    s.put("src/other.drh", "in * from c.\"drachm_fixture.h\" as fx;\n\nout sint32 add(sint32 a, sint32 b) {\n    unsafe { return fx::fx_add(a as c::int, b as c::int) as sint32; }\n}\n");
    let check = drachm(&s.dir, &["check", "."]);
    assert!(!check.status.success());
    let err = text(&check.stderr);
    assert!(err.contains("E0317") && err.contains("other.drh"), "{err}");
    assert!(err.contains("[native]"), "{err}");
}

#[test]
fn a_dependency_file_may_carry_its_own_flag() {
    // The flag is read for each file, so a dependency's file works without the app's flag.
    let s = Scratch::new("depflag");
    s.put(
        "cdep/drachm.toml",
        "[project]\nname = \"cdep\"\nkind = \"library\"\npublic_modules = [\"wrapped\"]\n\n[native]\ninclude = [\"inc\"]\nsources = [\"c/impl.c\"]\n",
    );
    s.put("cdep/inc/dep.h", "#ifndef DEP_H\n#define DEP_H\nint dep_triple(int x);\n#endif\n");
    s.put("cdep/c/impl.c", "#include \"dep.h\"\nint dep_triple(int x) { return x * 3; }\n");
    s.put("cdep/src/main.drh", "none unused() { }\n");
    s.put(
        "cdep/src/wrapped.drh",
        "//--!$unsafe: true\nin * from c.\"dep.h\" as dep;\n\nout sint32 triple(sint32 x) {\n    unsafe {\n        return dep::dep_triple(x as c::int) as sint32;\n    }\n}\n",
    );
    s.put("app/drachm.toml", "[project]\nname = \"app\"\n\n[dependencies]\ncdep = { path = \"../cdep\" }\n");
    s.put("app/src/main.drh", "in io;\nin cdep.wrapped as w;\n\nnone main() {\n    io::outln(f\"triple: ${w::triple(14)}\");\n}\n");
    let run = drachm(&s.dir, &["run", "app"]);
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "triple: 42\n");
}

#[test]
fn the_unsafe_key_must_be_true_or_false() {
    let s = Scratch::new("badunsafe");
    s.put("drachm.toml", "[project]\nname = \"bad\"\n\n[native]\nunsafe = \"yes\"\n");
    s.put("src/main.drh", "none main() { }\n");
    let check = drachm(&s.dir, &["check", "."]);
    assert!(!check.status.success());
    let err = text(&check.stderr);
    assert!(err.contains("E0802") && err.contains("`unsafe` must be `true` or `false`"), "{err}");
}

#[test]
fn foreign_pointers_copy_and_a_handover_takes_as() {
    // A `foreign` pointer copies like a number: both names read the same text.
    let s = Scratch::new("copy");
    s.put("drachm.toml", &format!("[project]\nname = \"copy\"\n\n[native]\ninclude = [\"{}\"]\nsources = [\"{}/drachm_fixture.c\"]\n", fixture_dir(), fixture_dir()));
    s.put(
        "src/main.drh",
        "//--!$unsafe: true\nin io;\nin * from c.\"drachm_fixture.h\" as fx;\n\nnone main() {\n    unsafe {\n        foreign *uint8 p = fx::fx_greeting();\n        foreign *uint8 q = p;\n        string? a = p::text() maybe none;\n        string? b = q::text() maybe none;\n        io::outln(a maybe \"?\");\n        io::outln(b maybe \"?\");\n    }\n}\n",
    );
    let run = drachm(&s.dir, &["run", "."]);
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "hello from C\nhello from C\n");

    // A pointer is not a number: `as` takes a C pointer type only.
    let s = Scratch::new("handover");
    s.put("drachm.toml", &format!("[project]\nname = \"handover\"\n\n[native]\ninclude = [\"{}\"]\nsources = [\"{}/drachm_fixture.c\"]\n", fixture_dir(), fixture_dir()));
    s.put(
        "src/main.drh",
        "//--!$unsafe: true\nin io;\nin * from c.\"drachm_fixture.h\" as fx;\n\nnone main() {\n    unsafe {\n        foreign *uint8 p = fx::fx_greeting();\n        uint64 n = p as uint64;\n        io::outln(f\"${n}\");\n    }\n}\n",
    );
    let check = drachm(&s.dir, &["check", "."]);
    assert!(!check.status.success());
    let err = text(&check.stderr);
    assert!(err.contains("E0335") && err.contains("`as` turns a pointer into a C pointer"), "{err}");
}

/// Are the SDL3 headers and library installed (the window example needs both)?
fn sdl3_installed() -> bool {
    let header = ["/usr/include", "/usr/local/include"].iter().any(|d| Path::new(d).join("SDL3/SDL.h").exists());
    let lib = ["/usr/lib", "/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/local/lib"].iter().any(|d| Path::new(d).join("libSDL3.so").exists());
    header && lib
}

#[test]
fn the_sdl3_window_example_checks_and_runs_without_a_display() {
    if !sdl3_installed() {
        eprintln!("skipped: SDL3 is not installed");
        return;
    }
    let dir = root().join("examples/sdl3-window");
    let check = drachm(&root(), &["check", dir.to_str().unwrap()]);
    let err = text(&check.stderr);
    if err.contains("libclang") {
        eprintln!("skipped: libclang is not installed");
        return;
    }
    assert!(check.status.success(), "{err}");
    // SDL's dummy video driver has no display: a few frames, on both backends.
    for flags in [&["run"][..], &["run", "-O"][..]] {
        let mut args: Vec<&str> = flags.to_vec();
        args.push(dir.to_str().unwrap());
        let out = Command::new(env!("CARGO_BIN_EXE_drachm"))
            .current_dir(root())
            .env("NO_COLOR", "1")
            .env("SDL_VIDEO_DRIVER", "dummy")
            .env("WINDOW_FRAMES", "10")
            .args(&args)
            .output()
            .expect("could not start drachm");
        assert!(out.status.success(), "{:?}: {}", flags, text(&out.stderr));
        assert_eq!(text(&out.stdout), "closed after 10 frames, 0 clicks\n", "{flags:?}");
    }
}
