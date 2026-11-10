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
//!
//! `DRACHM_BLESS=1 cargo test` writes `.out`/`.code` for run cases from what
//! the program actually printed. Read the diff before committing it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
    // Relative paths keep file names in diagnostics the same on every machine.
    Command::new(env!("CARGO_BIN_EXE_drachm"))
        .current_dir(root())
        .args(args)
        .arg(file.strip_prefix(root()).unwrap())
        .output()
        .expect("could not start the drachm binary")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.replace("\r\n", "\n"))
}

/// Lines of `expected` (ignoring blank ones) that do not occur in `actual`.
fn missing_lines<'a>(expected: &'a str, actual: &str) -> Vec<&'a str> {
    expected.lines().map(str::trim).filter(|l| !l.is_empty() && !actual.contains(l)).collect()
}

/// Run `case` on every source, in parallel, and fail with all the reports.
fn run_all(files: Vec<PathBuf>, case: impl Fn(&Path) -> Result<(), String> + Sync) {
    let failures: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = files.iter().map(|f| (f, s.spawn(|| case(f)))).collect();
        handles
            .into_iter()
            .filter_map(|(f, h)| h.join().unwrap().err().map(|e| format!("--- {}\n{e}", f.display())))
            .collect()
    });
    assert!(failures.is_empty(), "{} of {} failed\n\n{}", failures.len(), files.len(), failures.join("\n\n"));
}

/// Compile and run `src`, comparing against the expectation files in `expect_dir`.
fn run_case(src: &Path, expect_dir: &Path) -> Result<(), String> {
    let stem = src.file_stem().unwrap().to_str().unwrap();
    let expect = |ext: &str| expect_dir.join(format!("{stem}.{ext}"));
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{}-{stem}", expect_dir.file_name().unwrap().to_str().unwrap()));

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
    let run = Command::new(&exe).output().map_err(|e| format!("could not run: {e}"))?;
    let _ = fs::remove_file(&exe);
    let (stdout, stderr) = (text(&run.stdout), text(&run.stderr));
    let code = run.status.code().ok_or("killed by a signal")?;

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

/// Optimised builds must behave exactly like unoptimised ones.
#[test]
fn optimised_output_matches() {
    let mut files = sources("tests/run");
    files.extend(sources("examples"));
    run_all(files, |src| {
        let stem = src.file_stem().unwrap().to_str().unwrap();
        let mut outputs = Vec::new();
        for flags in [&[][..], &["-O"][..]] {
            let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("opt{}-{stem}", flags.len()));
            let mut args = vec!["build", "-o", exe.to_str().unwrap()];
            args.extend(flags);
            let build = drachm(&args, src);
            if !build.status.success() {
                return Err(format!("did not compile with {flags:?}:\n{}", text(&build.stderr)));
            }
            let run = Command::new(&exe).output().map_err(|e| format!("could not run: {e}"))?;
            let _ = fs::remove_file(&exe);
            outputs.push((text(&run.stdout), run.status.code()));
        }
        if outputs[0] == outputs[1] {
            Ok(())
        } else {
            Err(format!("plain build gave {:?}\n-O build gave {:?}", outputs[0], outputs[1]))
        }
    });
}
