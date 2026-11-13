//! The command line on project folders (step 7, WP3): `drachm new` makes a
//! project that `drachm run` runs, and the project-file errors come out with
//! their codes. Each test works in a temp folder of its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("drachm-cli-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Runs the compiler in `cwd` with `args`.
fn drachm(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_drachm")).current_dir(cwd).env("NO_COLOR", "1").args(args).output().expect("could not start drachm")
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).replace("\r\n", "\n")
}

#[test]
fn new_then_run_prints_the_greeting() {
    let s = Scratch::new("newrun");
    let out = drachm(&s.dir, &["new", "demo"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(s.dir.join("demo/drachm.toml").is_file());
    assert!(s.dir.join("demo/src/main.drh").is_file());
    assert!(s.dir.join("demo/tests").is_dir());
    let toml = fs::read_to_string(s.dir.join("demo/drachm.toml")).unwrap();
    assert!(toml.contains("name = \"demo\""), "{toml}");

    // From the folder above, and from inside it with no argument.
    let run = drachm(&s.dir, &["run", "demo"]);
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "Hello, world!\n");

    let inside = drachm(&s.dir.join("demo"), &["run"]);
    assert!(inside.status.success(), "{}", text(&inside.stderr));
    assert_eq!(text(&inside.stdout), "Hello, world!\n");

    let check = drachm(&s.dir.join("demo"), &["check"]);
    assert!(check.status.success(), "{}", text(&check.stderr));
}

#[test]
fn new_refuses_a_bad_name_and_an_existing_folder() {
    let s = Scratch::new("badnew");
    let bad = drachm(&s.dir, &["new", "2fast"]);
    assert!(!bad.status.success());
    assert!(text(&bad.stderr).contains("error[E0805]: `2fast` is not a valid project name"), "{}", text(&bad.stderr));
    assert!(!s.dir.join("2fast").exists());

    assert!(drachm(&s.dir, &["new", "demo"]).status.success());
    let again = drachm(&s.dir, &["new", "demo"]);
    assert!(!again.status.success());
    assert!(text(&again.stderr).contains("error[E0813]"), "{}", text(&again.stderr));
}

#[test]
fn a_library_is_checked_but_not_run_or_built() {
    let s = Scratch::new("library");
    assert!(drachm(&s.dir, &["new", "geo"]).status.success());
    let geo = s.dir.join("geo");
    fs::write(geo.join("drachm.toml"), "[project]\nname = \"geo\"\nkind = \"library\"\n").unwrap();
    fs::write(geo.join("src/main.drh"), "uint32 area(uint32 w, uint32 h) { return w * h; }\n").unwrap();
    let check = drachm(&s.dir, &["check", "geo"]);
    assert!(check.status.success(), "{}", text(&check.stderr));
    let run = drachm(&s.dir, &["run", "geo"]);
    assert!(!run.status.success());
    assert!(text(&run.stderr).contains("error[E0811]: `geo` is a library, so there is no program to run"), "{}", text(&run.stderr));
    let build = drachm(&s.dir, &["build", "geo"]);
    assert!(text(&build.stderr).contains("E0811"), "{}", text(&build.stderr));
}

#[test]
fn a_missing_entry_is_e0812_and_a_missing_toml_is_e0814() {
    let s = Scratch::new("entry");
    assert!(drachm(&s.dir, &["new", "demo"]).status.success());
    let demo = s.dir.join("demo");
    fs::write(demo.join("drachm.toml"), "[project]\nname = \"demo\"\nentry = \"app/start.drh\"\n").unwrap();
    let out = drachm(&s.dir, &["run", "demo"]);
    assert!(text(&out.stderr).contains("error[E0812]"), "{}", text(&out.stderr));

    let empty = s.dir.join("empty");
    fs::create_dir_all(&empty).unwrap();
    let out = drachm(&s.dir, &["check", "empty"]);
    assert!(text(&out.stderr).contains("error[E0814]"), "{}", text(&out.stderr));
}

#[test]
fn a_config_error_points_into_drachm_toml() {
    let s = Scratch::new("tomlerr");
    assert!(drachm(&s.dir, &["new", "demo"]).status.success());
    let demo = s.dir.join("demo");
    fs::write(demo.join("drachm.toml"), "[project]\nname = \"demo\"\n[behaviour]\noverflow = \"clamp\"\n").unwrap();
    let out = drachm(&s.dir, &["check", "demo"]);
    let err = text(&out.stderr);
    assert!(err.contains("error[E0802]: `overflow` must be wrap, saturate or panic, found `clamp`"), "{err}");
    assert!(err.contains("drachm.toml:4:12"), "{err}");
}

#[test]
fn a_file_flag_for_a_project_setting_is_e0806() {
    let s = Scratch::new("flag");
    assert!(drachm(&s.dir, &["new", "demo"]).status.success());
    let main = s.dir.join("demo/src/main.drh");
    let body = fs::read_to_string(&main).unwrap();
    fs::write(&main, format!("//--!$missing_key: panic\n{body}")).unwrap();
    let out = drachm(&s.dir, &["check", "demo"]);
    assert!(text(&out.stderr).contains("error[E0806]"), "{}", text(&out.stderr));
}

#[test]
fn a_git_dependency_is_fetched_by_check() {
    let s = Scratch::new("git");
    let lib = s.dir.join("sdl");
    fs::create_dir_all(lib.join("src")).unwrap();
    fs::write(lib.join("drachm.toml"), "[project]\nname = \"sdl\"\nkind = \"library\"\n").unwrap();
    fs::write(lib.join("src/main.drh"), "none helper() { }\n").unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&lib)
            .args(["-c", "user.name=drachm-test", "-c", "user.email=test@example.org", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("git must be installed");
        assert!(out.status.success(), "{}", text(&out.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    git(&["tag", "v0.4"]);

    assert!(drachm(&s.dir, &["new", "app"]).status.success());
    let url = format!("file://{}", lib.display());
    let toml = format!("[project]\nname = \"app\"\n[dependencies]\nsdl = {{ git = \"{url}\", tag = \"v0.4\" }}\n");
    fs::write(s.dir.join("app/drachm.toml"), toml).unwrap();
    let out = drachm(&s.dir, &["check", "app"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(s.dir.join("app/deps/sdl/drachm.toml").is_file());
    assert!(s.dir.join("app/drachm.lock").is_file());
}

#[test]
fn dependency_public_modules_gate_dotted_and_file_imports() {
    let s = Scratch::new("dependency-visibility");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("site/examples/dependency-modules");
    for name in [
        "drachm.toml", "src/main.drh", "library/drachm.toml",
        "library/src/main.drh", "library/src/calibration.drh", "library/src/survey/readings.drh",
    ] {
        let dest = s.dir.join(name);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(example.join(name), dest).unwrap();
    }
    let good = drachm(&s.dir, &["check"]);
    assert!(good.status.success(), "{}", text(&good.stderr));
    for import in ["in charts.calibration;", "in \"../library/src/calibration.drh\";"] {
        fs::write(s.dir.join("src/main.drh"), format!("{import}\nnone main() {{}}\n")).unwrap();
        let out = drachm(&s.dir, &["check"]);
        assert!(!out.status.success());
        assert!(text(&out.stderr).contains("E0236"), "{}", text(&out.stderr));
    }
}

#[test]
fn bounds_setting_is_applied_to_each_defining_file() {
    let s = Scratch::new("bounds");
    fs::create_dir(s.dir.join("src")).unwrap();
    fs::write(s.dir.join("drachm.toml"), "[project]\nname = \"demo\"\n[behaviour]\nbounds = \"unchecked\"\n").unwrap();
    fs::write(s.dir.join("src/main.drh"), r#"
in io;
in read from checked;
uint32 local_read(sint64 i) {
    uint32[2] values = [4, 8];
    return values[i];
}
none main() {
    io::outln(f"${local_read(-1)} ${read(-1)}");
}
"#).unwrap();
    fs::write(s.dir.join("src/checked.drh"), r#"//--!$bounds: panic
out uint32 read(sint64 i) {
    uint32[2] values = [15, 16];
    return values[i];
}
"#).unwrap();
    let dump = drachm(&s.dir, &["check", "--dump-mir"]);
    assert!(dump.status.success(), "{}", text(&dump.stderr));
    let mir = text(&dump.stdout);
    assert_eq!(mir.matches("IndexPanic").count(), 1, "{mir}");
    for args in [&["run"][..], &["run", "-O"][..]] {
        let out = drachm(&s.dir, args);
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert_eq!(text(&out.stdout), "8 16\n");
    }
}

#[test]
fn unsigned_negate_error_rejects_typed_values() {
    let s = Scratch::new("unsigned-negate");
    fs::write(s.dir.join("main.drh"), r#"//--!$unsigned_negate: error
none main() {
    uint8 amount = 240;
    uint8 wrapped = -amount;
}
"#).unwrap();
    let out = drachm(&s.dir, &["check", "main.drh"]);
    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert!(stderr.contains("E0101") && stderr.contains("unsigned_negate = error"), "{stderr}");
}

#[test]
fn missing_key_panic_is_identical_on_both_backends() {
    let s = Scratch::new("missing-key");
    fs::create_dir(s.dir.join("src")).unwrap();
    fs::write(s.dir.join("drachm.toml"), "[project]\nname = \"demo\"\n[behaviour]\nmissing_key = \"panic\"\n").unwrap();
    fs::write(s.dir.join("src/main.drh"), r#"
none main() {
    Map<string, string> cargo = { "A2": "beacons" };
    string contents = cargo["B2"];
}
"#).unwrap();
    let plain = drachm(&s.dir, &["run"]);
    let optimized = drachm(&s.dir, &["run", "-O"]);
    for out in [&plain, &optimized] {
        assert_eq!(out.status.code(), Some(101), "{}", text(&out.stderr));
        assert!(text(&out.stderr).contains("the key is not in the map"), "{}", text(&out.stderr));
    }
    assert_eq!(text(&plain.stderr), text(&optimized.stderr));
}

#[test]
fn sleep_blocks_and_converts_time_units_on_both_backends() {
    let s = Scratch::new("sleep");
    fs::write(s.dir.join("main.drh"), r#"
in si;
in time;
none main() {
    time::sleep(0);
    time::sleep(30<ms>);
}
"#).unwrap();
    for flags in [&[][..], &["-O"][..]] {
        let mut args = vec!["build", "main.drh"];
        args.extend(flags);
        let build = drachm(&s.dir, &args);
        assert!(build.status.success(), "{}", text(&build.stderr));
        let start = std::time::Instant::now();
        let run = std::process::Command::new(s.dir.join(format!("main{}", std::env::consts::EXE_SUFFIX)))
            .output().unwrap();
        assert!(run.status.success(), "{}", text(&run.stderr));
        assert!(start.elapsed() >= std::time::Duration::from_millis(30));
    }
}

/// A file of three tests, one failing: each runs on its own, the failure is
/// reported with its assertion, the others still run, and the exit code is 1.
const TESTS_FILE: &str = r#"
in io;

sint32 add(sint32 a, sint32 b) {
    return a + b;
}

test "adds small numbers" {
    assert(add(2, 3) == 5);
}

test "a wrong sum" {
    io::outln("this line prints before the check fails");
    assert(add(2, 3) == 6);
}

test "negative sums" {
    assert(add(-2, -3) == -5);
}

none main() {
    io::outln(f"add gives ${add(2, 3)}");
}
"#;

#[test]
fn test_runs_each_block_on_its_own_and_reports_the_failure() {
    let s = Scratch::new("testfail");
    fs::write(s.dir.join("sums.drh"), TESTS_FILE).unwrap();
    for flags in [&[][..], &["-O"][..]] {
        let mut args = vec!["test", "sums.drh"];
        args.extend(flags);
        let out = drachm(&s.dir, &args);
        let stdout = text(&out.stdout);
        assert_eq!(out.status.code(), Some(1), "{stdout}\n{}", text(&out.stderr));
        assert!(stdout.starts_with("running 3 tests\n"), "{stdout}");
        assert!(stdout.contains("test adds small numbers ... ok\n"), "{stdout}");
        assert!(stdout.contains("test a wrong sum ... FAILED\n"), "{stdout}");
        assert!(stdout.contains("    this line prints before the check fails\n"), "{stdout}");
        assert!(stdout.contains("    drachm: panic: assertion failed: add(2, 3) == 6\n"), "{stdout}");
        assert!(stdout.contains("      --> sums.drh:14\n"), "{stdout}");
        assert!(stdout.contains("    exit code 101\n"), "{stdout}");
        assert!(stdout.contains("test negative sums ... ok\n"), "{stdout}");
        assert!(stdout.ends_with("\ntest result: FAILED. 2 passed; 1 failed\n"), "{stdout}");
        // A normal run compiles the tests and runs only `main`.
        let run = drachm(&s.dir, &["run", "sums.drh"]);
        assert!(run.status.success(), "{}", text(&run.stderr));
        assert_eq!(text(&run.stdout), "add gives 5\n");
    }
}

#[test]
fn test_passes_with_exit_zero_and_handles_no_tests() {
    let s = Scratch::new("testpass");
    fs::write(s.dir.join("ok.drh"), "test \"one\" {\n    assert(1 + 1 == 2);\n}\n\nnone main() { }\n").unwrap();
    let out = drachm(&s.dir, &["test", "ok.drh"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).ends_with("\ntest result: ok. 1 passed; 0 failed\n"), "{}", text(&out.stdout));

    fs::write(s.dir.join("empty.drh"), "none main() { }\n").unwrap();
    let none = drachm(&s.dir, &["test", "empty.drh"]);
    assert!(none.status.success(), "{}", text(&none.stderr));
    assert_eq!(text(&none.stdout), "running 0 tests\n\ntest result: ok. 0 passed; 0 failed\n");
}

#[test]
fn test_name_filter_runs_only_the_tests_whose_name_contains_it() {
    let s = Scratch::new("testfilter");
    fs::write(s.dir.join("sums.drh"), TESTS_FILE).unwrap();
    for flags in [&[][..], &["-O"][..]] {
        // One test matches: it runs, and the other two are counted as filtered out.
        let mut args = vec!["test", "sums.drh", "wrong"];
        args.extend(flags);
        let out = drachm(&s.dir, &args);
        let stdout = text(&out.stdout);
        assert_eq!(out.status.code(), Some(1), "{stdout}\n{}", text(&out.stderr));
        assert!(stdout.starts_with("running 1 test\n"), "{stdout}");
        assert!(stdout.contains("test a wrong sum ... FAILED\n"), "{stdout}");
        assert!(!stdout.contains("adds small numbers"), "{stdout}");
        assert!(!stdout.contains("negative sums"), "{stdout}");
        assert!(stdout.ends_with("\ntest result: FAILED. 0 passed; 1 failed; 2 filtered out\n"), "{stdout}");

        // A name that matches a passing test: exit 0.
        let mut args = vec!["test", "sums.drh", "negative"];
        args.extend(flags);
        let out = drachm(&s.dir, &args);
        assert!(out.status.success(), "{}", text(&out.stdout));
        assert!(text(&out.stdout).ends_with("\ntest result: ok. 1 passed; 0 failed; 2 filtered out\n"), "{}", text(&out.stdout));
    }
}

#[test]
fn test_runs_the_tests_of_a_library_without_a_main() {
    let s = Scratch::new("testlib");
    fs::create_dir_all(s.dir.join("mathlib/src")).unwrap();
    fs::write(s.dir.join("mathlib/drachm.toml"), "[project]\nname = \"mathlib\"\nkind = \"library\"\n").unwrap();
    fs::write(s.dir.join("mathlib/src/main.drh"), "sint32 twice(sint32 n) { return n * 2; }\n\ntest \"doubles\" {\n    assert(twice(4) == 8);\n}\n").unwrap();
    let out = drachm(&s.dir, &["test", "mathlib"]);
    assert!(out.status.success(), "{}{}", text(&out.stdout), text(&out.stderr));
    assert!(text(&out.stdout).contains("test doubles ... ok\n"), "{}", text(&out.stdout));
}

#[test]
fn test_runs_the_showcase_project_tests() {
    let showcase = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/showcase");
    let out = drachm(&showcase, &["test"]);
    assert!(out.status.success(), "{}{}", text(&out.stdout), text(&out.stderr));
    assert!(text(&out.stdout).ends_with("\ntest result: ok. 3 passed; 0 failed\n"), "{}", text(&out.stdout));
}

#[test]
fn duplicate_test_names_are_e0239() {
    let s = Scratch::new("testdup");
    fs::write(s.dir.join("dup.drh"), "test \"same\" {\n    assert(true);\n}\n\ntest \"same\" {\n    assert(true);\n}\n\nnone main() { }\n").unwrap();
    let out = drachm(&s.dir, &["check", "dup.drh"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("error[E0239]"), "{}", text(&out.stderr));
}

#[test]
fn test_failure_prints_the_assert_message_after_the_condition() {
    let s = Scratch::new("testmsg");
    fs::write(s.dir.join("msg.drh"), "test \"sum\" {\n    assert(1 + 1 == 3, \"one and one make two\");\n}\n\nnone main() { }\n").unwrap();
    let out = drachm(&s.dir, &["test", "msg.drh"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(text(&out.stdout).contains("    drachm: panic: assertion failed: 1 + 1 == 3: one and one make two\n      --> msg.drh:2\n"), "{}", text(&out.stdout));
}

/// Four tests: two pass (an expected failure, and a text that is not a
/// number), two do not (no failure at all, and a different failure).
const EXPECT_FILE: &str = r#"
in io;

failure Boom { Bang "it went bang" }
failure Other { Odd "it went oddly" }

string named(uint32 n) fails Boom {
    if (n == 0) {
        fail Boom::Bang;
    }
    return "ok";
}

uint32 two_ways(uint32 n) fails Boom, Other {
    if (n == 0) {
        fail Boom::Bang;
    }
    if (n == 1) {
        fail Other::Odd;
    }
    return n;
}

test "the expected failure passes" {
    expect failure Boom from named(0);
    io::outln("this line is hidden when the test passes");
}

test "a text that is not a number" {
    expect failure ConvertError from "x"::to<uint32>();
}

test "no failure" {
    expect failure Boom from named(1);
}

test "another failure" {
    expect failure Boom from two_ways(1);
}

none main() {
    io::outln("main runs");
}
"#;

#[test]
fn test_expect_failure_passes_or_stops_the_test_with_a_message() {
    let s = Scratch::new("expectfail");
    fs::write(s.dir.join("expect.drh"), EXPECT_FILE).unwrap();
    let plain = drachm(&s.dir, &["test", "expect.drh"]);
    let optimised = drachm(&s.dir, &["test", "expect.drh", "-O"]);
    for out in [&plain, &optimised] {
        let stdout = text(&out.stdout);
        assert_eq!(out.status.code(), Some(1), "{stdout}\n{}", text(&out.stderr));
        assert!(stdout.starts_with("running 4 tests\n"), "{stdout}");
        assert!(stdout.contains("test the expected failure passes ... ok\n"), "{stdout}");
        assert!(!stdout.contains("this line is hidden"), "{stdout}");
        assert!(stdout.contains("test a text that is not a number ... ok\n"), "{stdout}");
        assert!(stdout.contains("test no failure ... FAILED\n"), "{stdout}");
        assert!(stdout.contains("    drachm: panic: expected a `Boom` failure from `named(1)`, but the call did not fail\n"), "{stdout}");
        assert!(stdout.contains("      --> expect.drh:34\n"), "{stdout}");
        assert!(stdout.contains("test another failure ... FAILED\n"), "{stdout}");
        assert!(stdout.contains("    drachm: panic: expected a `Boom` failure from `two_ways(1)`, but the call failed with a different failure\n"), "{stdout}");
        assert!(stdout.contains("      --> expect.drh:38\n"), "{stdout}");
        assert!(stdout.ends_with("\ntest result: FAILED. 2 passed; 2 failed\n"), "{stdout}");
    }
    assert_eq!(text(&plain.stdout), text(&optimised.stdout));
    // A normal run checks the tests and runs only `main`.
    let run = drachm(&s.dir, &["run", "expect.drh"]);
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "main runs\n");
}
