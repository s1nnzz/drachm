//! Golden tests. To add a case, drop in a source file and its expectation:
//!
//! - `tests/run/NAME.drh` is compiled and run. Its stdout must equal
//!   `NAME.out` exactly. Its exit code must equal the number in `NAME.code`
//!   (0 if there is no such file). If `NAME.err` exists, each of its lines
//!   must appear somewhere in the program's stderr. The compiler must be
//!   silent, unless `NAME.warn` exists: then each of its lines must appear
//!   somewhere in the compiler's output (for testing warnings).
//! - `tests/fail/NAME.drh` must fail to compile, and each line of `NAME.err`
//!   must appear somewhere in the compiler's stderr. Match on error codes and
//!   messages, not on layout.
//! - Every file in `examples/` is run like a `tests/run` case, with its
//!   expectations in `tests/examples/`.
//! - Every file in `site/examples/` is built both ways, must compile without
//!   output, and must print exactly `NAME.out`. The documentation site
//!   includes these files, so they must stay real programs.
//!
//! Programs run with `DRACHM_CHECK_LEAKS=1`, and a string they leak, or free
//! twice, fails the case. A program that panics (exit code 101) is exempt: a
//! panic does not drop what is alive.
//!
//! `NAME.in` next to a test or example is that program's standard input
//! (`tests/run/NAME.in`, `tests/examples/NAME.in`, `site/examples/NAME.in`).
//! Without one, stdin is null: reading gives end of input at once, so a test
//! never waits for a terminal.
//!
//! `DRACHM_BLESS=1 cargo test` writes `.out`/`.code` for run cases from what
//! the program actually printed. Read the diff before committing it.

use std::env::consts::EXE_SUFFIX;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sources(dir: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(root().join(dir))
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "drh"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no test sources in {dir}");
    found
}

fn drachm(args: &[&str], file: &Path) -> Output {
    // Relative paths with `/` keep file names in diagnostics the same on every
    // machine (Windows accepts `/` too).
    let relative = file.strip_prefix(root()).unwrap().to_str().unwrap().replace('\\', "/");
    Command::new(env!("CARGO_BIN_EXE_drachm"))
        .current_dir(root())
        .args(args)
        .arg(relative)
        .output()
        .expect("could not start the drachm binary")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.replace("\r\n", "\n"))
}

/// The standard input for a program's source: `NAME.in` beside the source or,
/// for `examples/`, beside its expectations in `tests/examples/`.
fn input_of(src: &Path) -> Option<PathBuf> {
    let stem = src.file_stem().unwrap().to_str().unwrap();
    [src.with_extension("in"), root().join("tests/examples").join(format!("{stem}.in"))].into_iter().find(|p| p.exists())
}

/// Run a compiled program with the string runtime's leak check on. Its
/// standard input is `stdin`'s file, or null.
fn run_program(exe: &Path, stdin: Option<&Path>) -> Result<Output, String> {
    let stdin = match stdin {
        Some(p) => Stdio::from(File::open(p).map_err(|e| format!("{}: {e}", p.display()))?),
        None => Stdio::null(),
    };
    Command::new(exe).env("DRACHM_CHECK_LEAKS", "1").stdin(stdin).output().map_err(|e| format!("could not run: {e}"))
}

/// The runtime's leak or double-free report, if the program printed one and
/// did not end in a panic (a panic does not drop).
fn leak_report<'a>(run: &Output, stderr: &'a str) -> Option<&'a str> {
    if run.status.code() == Some(101) {
        return None;
    }
    stderr.lines().find(|l| l.starts_with("drachm: leaked") || l.starts_with("drachm: double free"))
}

/// Lines of `expected` (ignoring blank ones) that do not occur in `actual`.
fn missing_lines<'a>(expected: &'a str, actual: &str) -> Vec<&'a str> {
    expected.lines().map(str::trim).filter(|l| !l.is_empty() && !actual.contains(l)).collect()
}

/// Run `case` on every source, in parallel, and fail with all the reports.
fn run_all(files: Vec<PathBuf>, case: impl Fn(&Path) -> Result<(), String> + Sync) {
    // A bounded pool: one worker per core (or DRACHM_TEST_JOBS), so several test
    // runs at once don't start hundreds of compilers.
    let jobs = std::env::var("DRACHM_TEST_JOBS").ok().and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
        .max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<Result<(), String>>>> =
        files.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..jobs.min(files.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(f) = files.get(i) else { break };
                *results[i].lock().unwrap() = Some(case(f));
            });
        }
    });
    let failures: Vec<String> = files
        .iter()
        .zip(results)
        .filter_map(|(f, r)| r.into_inner().unwrap().and_then(|r| r.err()).map(|e| format!("--- {}\n{e}", f.display())))
        .collect();
    assert!(failures.is_empty(), "{} of {} failed\n\n{}", failures.len(), files.len(), failures.join("\n\n"));
}

/// Compile and run `src`, comparing against the expectation files in `expect_dir`.
fn run_case(src: &Path, expect_dir: &Path) -> Result<(), String> {
    let stem = src.file_stem().unwrap().to_str().unwrap();
    let expect = |ext: &str| expect_dir.join(format!("{stem}.{ext}"));
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{}-{}-{stem}{EXE_SUFFIX}", std::process::id(), expect_dir.file_name().unwrap().to_str().unwrap()));

    let build = drachm(&["build", "-o", exe.to_str().unwrap()], src);
    let compiler_output = text(&build.stderr);
    if !build.status.success() {
        return Err(format!("did not compile:\n{compiler_output}"));
    }
    match read(&expect("warn")) {
        Some(want) => {
            let missing = missing_lines(&want, &compiler_output);
            if !missing.is_empty() {
                return Err(format!("compiler output lacks:\n  {}\nactual output:\n{compiler_output}", missing.join("\n  ")));
            }
        }
        None if !compiler_output.is_empty() => {
            return Err(format!("unexpected compiler output (expected warnings go in {stem}.warn):\n{compiler_output}"));
        }
        None => {}
    }
    let run = run_program(&exe, input_of(src).as_deref())?;
    let _ = fs::remove_file(&exe);
    let (stdout, stderr) = (text(&run.stdout), text(&run.stderr));
    let code = run.status.code().ok_or("killed by a signal")?;
    if let Some(leak) = leak_report(&run, &stderr) {
        return Err(leak.to_string());
    }

    if std::env::var_os("DRACHM_BLESS").is_some() {
        fs::write(expect("out"), &stdout).unwrap();
        if code != 0 {
            fs::write(expect("code"), format!("{code}\n")).unwrap();
        } else {
            let _ = fs::remove_file(expect("code"));
        }
        return Ok(());
    }

    let want_out = read(&expect("out"))
        .ok_or_else(|| format!("no {stem}.out (write it, or run with DRACHM_BLESS=1)"))?;
    let want_code: i32 = match read(&expect("code")) {
        Some(s) => s.trim().parse().map_err(|_| format!("{stem}.code is not a number"))?,
        None => 0,
    };

    let mut problems = Vec::new();
    if stdout != want_out {
        problems.push(format!("stdout differs\nexpected:\n{want_out}\nactual:\n{stdout}"));
    }
    if code != want_code {
        problems.push(format!("exit code {code}, expected {want_code}"));
    }
    if let Some(want_err) = read(&expect("err")) {
        let missing = missing_lines(&want_err, &stderr);
        if !missing.is_empty() {
            problems.push(format!("stderr lacks:\n  {}\nactual stderr:\n{stderr}", missing.join("\n  ")));
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems.join("\n")) }
}

fn fail_case(src: &Path) -> Result<(), String> {
    let want = read(&src.with_extension("err")).ok_or("no .err file with the expected messages")?;
    if want.trim().is_empty() {
        return Err(".err file is empty".into());
    }
    let out = drachm(&["check"], src);
    let stderr = text(&out.stderr);
    if out.status.success() {
        return Err("compiled, but should have been rejected".into());
    }
    if out.status.code() != Some(1) {
        return Err(format!("the compiler crashed instead of reporting errors:\n{stderr}"));
    }
    let missing = missing_lines(&want, &stderr);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("stderr lacks:\n  {}\nactual stderr:\n{stderr}", missing.join("\n  ")))
    }
}

#[test]
fn run() {
    let dir = root().join("tests/run");
    run_all(sources("tests/run"), |f| run_case(f, &dir));
}

#[test]
fn fail() {
    run_all(sources("tests/fail"), fail_case);
}

#[test]
fn examples() {
    let dir = root().join("tests/examples");
    run_all(sources("examples"), |f| run_case(f, &dir));
}

/// The documentation site's programs: silent to compile, both backends print `NAME.out`.
#[test]
fn site_examples() {
    run_all(sources("site/examples"), site_example);
}

#[test]
fn script_entries() {
    run_all(
        [
            root().join("site/examples/script-entry.drh"),
            root().join("tests/scripts/configured/configured.drh"),
            root().join("tests/scripts/configured/status.drh"),
            root().join("examples/showcase/dispatch/dispatch.drh"),
        ].into(),
        |src| {
            let want = read(&src.with_extension("out")).ok_or("missing script output")?;
            let code = match read(&src.with_extension("code")) {
                Some(code) => code.trim().parse().map_err(|_| "invalid script exit code")?,
                None => 0,
            };
            for flags in [&[][..], &["-O"][..]] {
                let run = run_script(flags, src);
                let stderr = text(&run.stderr);
                if run.status.code() != Some(code) || !stderr.is_empty() || text(&run.stdout) != want {
                    return Err(format!("run with {flags:?} exited {:?}\nstdout:\n{}\nstderr:\n{stderr}", run.status.code(), text(&run.stdout)));
                }
            }
            Ok(())
        },
    );
}

fn run_script(flags: &[&str], file: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_drachm"))
        .current_dir(root())
        .env("DRACHM_CHECK_LEAKS", "1")
        .arg("run")
        .args(flags)
        .arg(file)
        .output()
        .expect("could not start the drachm binary")
}

/// Site examples of several files: a folder `site/examples/NAME/` that is a
/// project (see `projects`), with `expect.out` beside `drachm.toml`.
#[test]
fn site_example_projects() {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root().join("site/examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    if dirs.is_empty() {
        return;
    }
    run_all(dirs, project_case);
}

fn site_example(src: &Path) -> Result<(), String> {
    let stem = src.file_stem().unwrap().to_str().unwrap();
    let out_file = src.with_extension("out");
    let bless = std::env::var_os("DRACHM_BLESS").is_some();
    let want = if bless { None } else { Some(read(&out_file).ok_or_else(|| format!("no {stem}.out (write it, or run with DRACHM_BLESS=1)"))?) };
    let mut outputs = Vec::new();
    for flags in [&[][..], &["-O"][..]] {
        let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("site-{}-{}-{stem}{EXE_SUFFIX}", std::process::id(), flags.len()));
        let mut args = vec!["build", "-o", exe.to_str().unwrap()];
        args.extend(flags);
        let build = drachm(&args, src);
        let compiler_output = text(&build.stderr);
        let warnings = read(&src.with_extension("warn"));
        check_build_output(build.status.success(), &compiler_output, warnings.as_deref(), flags)?;
        let input = src.with_extension("in");
        let run = run_program(&exe, input.exists().then_some(input.as_path()))?;
        let _ = fs::remove_file(&exe);
        let stderr = text(&run.stderr);
        if let Some(leak) = leak_report(&run, &stderr) {
            return Err(format!("{leak} (with {flags:?})"));
        }
        if run.status.code() != Some(0) {
            return Err(format!("exit code {:?} with {flags:?}\n{stderr}", run.status.code()));
        }
        outputs.push(text(&run.stdout));
    }
    if outputs[0] != outputs[1] {
        return Err(format!("plain build gave:\n{}\n-O build gave:\n{}", outputs[0], outputs[1]));
    }
    if bless {
        fs::write(&out_file, &outputs[0]).unwrap();
        return Ok(());
    }
    let want = want.unwrap();
    if outputs[0] != want {
        return Err(format!("stdout differs\nexpected:\n{want}\nactual:\n{}", outputs[0]));
    }
    Ok(())
}

/// Optimised builds must behave exactly like unoptimised ones.
#[test]
fn optimised_output_matches() {
    let mut files = sources("tests/run");
    files.extend(sources("examples"));
    run_all(files, |src| {
        let stem = src.file_stem().unwrap().to_str().unwrap();
        let mut outputs = Vec::new();
        for flags in [&[][..], &["-O"][..]] {
            let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("opt-{}-{}-{stem}{EXE_SUFFIX}", std::process::id(), flags.len()));
            let mut args = vec!["build", "-o", exe.to_str().unwrap()];
            args.extend(flags);
            let build = drachm(&args, src);
            if !build.status.success() {
                return Err(format!("did not compile with {flags:?}:\n{}", text(&build.stderr)));
            }
            let run = run_program(&exe, input_of(src).as_deref())?;
            let _ = fs::remove_file(&exe);
            if let Some(leak) = leak_report(&run, &text(&run.stderr)) {
                return Err(format!("{leak} (with {flags:?})"));
            }
            outputs.push((text(&run.stdout), run.status.code()));
        }
        if outputs[0] == outputs[1] {
            Ok(())
        } else {
            Err(format!("plain build gave {:?}\n-O build gave {:?}", outputs[0], outputs[1]))
        }
    });
}

/// Every program builds to valid MIR (the builder also validates itself in
/// debug builds; this lists all the failures at once).
#[test]
fn mir_builds() {
    let mut files = sources("tests/run");
    files.extend(sources("examples"));
    files.extend(sources("tests/mir"));
    run_all(files, |src| {
        let text = fs::read_to_string(src).map_err(|e| e.to_string())?;
        let (checked, diags) = drachm::check(&text);
        let hir = checked.ok_or_else(|| format!("does not type-check ({} diagnostics)", diags.len()))?;
        let mut mir_diags = Vec::new();
        let mir = drachm::mir::build_program(&hir, &mut mir_diags);
        drachm::mir::validate(&mir, &hir).map_err(|errors| errors.join("\n"))
    });
}

/// `tests/mir/NAME.mir`: each non-blank line must occur in the output of
/// `check --dump-mir`. A line `!FN: TEXT` instead says that no line of
/// function FN's dump is TEXT.
#[test]
fn mir() {
    run_all(sources("tests/mir"), |src| {
        let want = read(&src.with_extension("mir")).ok_or("no .mir file with the expected lines")?;
        let out = drachm(&["check", "--dump-mir"], src);
        if !out.status.success() {
            return Err(format!("did not check:\n{}", text(&out.stderr)));
        }
        let dump = text(&out.stdout);
        let mut problems = Vec::new();
        for line in want.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(rest) = line.strip_prefix('!') {
                let (func, banned) = rest.split_once(':').ok_or("`!` lines are `!FN: TEXT`")?;
                let (func, banned) = (func.trim(), banned.trim());
                let header = format!("fn {func}(");
                let section = dump
                    .split("\n\n")
                    .find(|s| s.starts_with(&header))
                    .ok_or_else(|| format!("no function {func} in the dump:\n{dump}"))?;
                if section.lines().any(|l| l.trim() == banned) {
                    problems.push(format!("{func} must not contain `{banned}`"));
                }
            } else if !dump.contains(line) {
                problems.push(format!("missing `{line}`"));
            }
        }
        if problems.is_empty() { Ok(()) } else { Err(format!("{}\nactual dump:\n{dump}", problems.join("\n"))) }
    });
}

/// Ownership errors the MIR builder reports (E0324, E0325): `check --dump-mir`
/// must fail and print each line of `NAME.err`. (They move to the fail tests
/// when `check` itself runs the MIR.)
#[test]
fn mir_fail() {
    run_all(sources("tests/mir_fail"), |src| {
        let want = read(&src.with_extension("err")).ok_or("no .err file with the expected messages")?;
        let out = drachm(&["check", "--dump-mir"], src);
        let stderr = text(&out.stderr);
        if out.status.code() != Some(1) {
            return Err(format!("expected an error exit, got {:?}:\n{stderr}", out.status.code()));
        }
        let missing = missing_lines(&want, &stderr);
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!("stderr lacks:\n  {}\nactual stderr:\n{stderr}", missing.join("\n  ")))
        }
    });
}

/// `tests/borrowck/NAME.bck`: each non-blank line must occur in the output of
/// `check --dump-borrowck` (loans with the points where they are in scope,
/// and the universal regions with what they reach).
#[test]
fn borrowck() {
    run_all(sources("tests/borrowck"), |src| {
        let want = read(&src.with_extension("bck")).ok_or("no .bck file with the expected lines")?;
        let out = drachm(&["check", "--dump-borrowck"], src);
        if !out.status.success() {
            return Err(format!("did not check:\n{}", text(&out.stderr)));
        }
        let dump = text(&out.stdout);
        let missing = missing_lines(&want, &dump);
        if missing.is_empty() { Ok(()) } else { Err(format!("missing:\n  {}\nactual dump:\n{dump}", missing.join("\n  "))) }
    });
}

/// Project folders, `tests/projects/NAME/` (and `examples/showcase/`): a `drachm.toml`, the files below
/// `src/` (entry `src/main.drh`), and the expectations beside them:
///
/// - `expect.out` (with optional `expect.code`, `expect.in`): the project is
///   built both ways, run with the leak check, and must print exactly that;
///   it must compile silently unless `expect.warn` lists the warnings (each
///   line must appear in the compiler's output);
/// - `expect.err` instead: `drachm check` must fail, each line of the file
///   must appear in the stderr. Diagnostics name their file relative to the
///   folder with `drachm.toml`, so the lines here are portable.
#[test]
fn projects() {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root().join("tests/projects"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no projects in tests/projects");
    // The showcase is a project folder too, with its expectations inside it.
    dirs.push(root().join("examples/showcase"));
    run_all(dirs, project_case);
}

fn check_build_output(success: bool, output: &str, warnings: Option<&str>, flags: &[&str]) -> Result<(), String> {
    if !success || (warnings.is_none() && !output.is_empty()) {
        return Err(format!("did not compile cleanly with {flags:?}:\n{output}"));
    }
    if let Some(warnings) = warnings {
        let missing = missing_lines(warnings, output);
        if !missing.is_empty() {
            return Err(format!("compiler output lacks:\n  {}\nactual output:\n{output}", missing.join("\n  ")));
        }
    }
    Ok(())
}

fn project_case(dir: &Path) -> Result<(), String> {
    let name = dir.file_name().unwrap().to_str().unwrap();
    let expect = |ext: &str| dir.join(format!("expect.{ext}"));
    if let Some(want) = read(&expect("out")) {
        let want_code: i32 = match read(&expect("code")) {
            Some(s) => s.trim().parse().map_err(|_| "expect.code is not a number".to_string())?,
            None => 0,
        };
        let stdin = expect("in");
        let mut outputs = Vec::new();
        for flags in [&[][..], &["-O"][..]] {
            let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("proj-{}-{}-{name}{EXE_SUFFIX}", std::process::id(), flags.len()));
            let mut args = vec!["build", "-o", exe.to_str().unwrap()];
            args.extend(flags);
            let build = drachm(&args, dir);
            let compiler_output = text(&build.stderr);
            let warnings = read(&expect("warn"));
            check_build_output(build.status.success(), &compiler_output, warnings.as_deref(), flags)?;
            let run = run_program(&exe, stdin.exists().then_some(stdin.as_path()))?;
            let _ = fs::remove_file(&exe);
            let stderr = text(&run.stderr);
            if let Some(leak) = leak_report(&run, &stderr) {
                return Err(format!("{leak} (with {flags:?})"));
            }
            if run.status.code() != Some(want_code) {
                return Err(format!("exit code {:?}, expected {want_code} (with {flags:?})\n{stderr}", run.status.code()));
            }
            outputs.push(text(&run.stdout));
        }
        if outputs[0] != outputs[1] {
            return Err(format!("plain build gave:\n{}\n-O build gave:\n{}", outputs[0], outputs[1]));
        }
        if outputs[0] != want {
            return Err(format!("stdout differs\nexpected:\n{want}\nactual:\n{}", outputs[0]));
        }
        return Ok(());
    }
    let want = read(&expect("err")).ok_or("neither expect.out nor expect.err")?;
    let out = drachm(&["check"], dir);
    let stderr = text(&out.stderr);
    if out.status.success() {
        return Err("checked, but should have been rejected".into());
    }
    if out.status.code() != Some(1) {
        return Err(format!("the compiler crashed instead of reporting errors:\n{stderr}"));
    }
    let missing = missing_lines(&want, &stderr);
    if missing.is_empty() { Ok(()) } else { Err(format!("stderr lacks:\n  {}\nactual stderr:\n{stderr}", missing.join("\n  "))) }
}

#[test]
fn system_modules() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("system-modules-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("system.drh");
    fs::write(&source, r#"
in fs;
in io;
in os;

string load(string path) fails FileMissing, AccessDenied, InvalidPath, InvalidFileText, FileIoError {
    return fs::read(path) fails;
}

none main() fails FileMissing, AccessDenied, InvalidPath, FileIoError {
    fs::write("茶.txt", "short") fails;
    io::outln(load("茶.txt") maybe "unexpected error");
    string missing = fs::read("missing.txt")
        fails with (FileMissing e) => { maybe "missing"; }
        otherwise (any e) => { maybe "wrong cause"; };
    io::outln(missing);
    string invalid = fs::read("binary.txt")
        fails with (InvalidFileText e) => { maybe "invalid text"; }
        otherwise (any e) => { maybe "wrong cause"; };
    io::outln(invalid);
    io::outln(os::platform());
    io::outln(os::getEnv("DRACHM_SYSTEM_EMPTY") maybe "not empty");
    io::outln(os::getEnv("DRACHM_SYSTEM_MISSING") maybe "unset");
}
"#).unwrap();
    for flags in [&[][..], &["-O"][..]] {
        fs::write(dir.join("茶.txt"), "original long text").unwrap();
        fs::write(dir.join("binary.txt"), [0xff]).unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_drachm"))
            .current_dir(&dir)
            .env("DRACHM_CHECK_LEAKS", "1")
            .env("DRACHM_SYSTEM_EMPTY", "")
            .env_remove("DRACHM_SYSTEM_MISSING")
            .arg("run")
            .args(flags)
            .arg(&source)
            .output()
            .unwrap();
        assert!(run.status.success(), "{flags:?}: {}", text(&run.stderr));
        assert_eq!(text(&run.stderr), "");
        assert_eq!(text(&run.stdout), format!("short\nmissing\ninvalid text\n{}\n\nunset\n", std::env::consts::OS));
        assert_eq!(fs::read(dir.join("茶.txt")).unwrap(), b"short");
    }
    fs::remove_dir_all(dir).unwrap();
}
