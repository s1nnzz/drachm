//! The MIR backends (WP4): every program they accept builds, on both
//! backends, from MIR and from HIR, and behaves the same four ways.
//!
//! `DRACHM_BACKEND=hir|mir` picks the generator inside the `drachm` binary
//! (see `drachm::build_unchecked`). Programs `drachm::mir::cleanup::eligible`
//! rejects keep going through the HIR backends until WP6.

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
    found
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
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

/// Build the MIR of `src` the way `drachm::build` does, and ask whether the
/// MIR backends accept it. `Err` with the reason if not.
fn eligibility(src: &Path) -> Result<(), String> {
    let source = fs::read_to_string(src).map_err(|e| e.to_string())?;
    let (checked, diags) = drachm::check(&source);
    let hir = checked.ok_or_else(|| format!("does not type-check ({} diagnostics)", diags.len()))?;
    let mut mir = drachm::mir::build_program(&hir, &mut Vec::new());
    drachm::mir::cleanup::cleanup_program(&hir, &mut mir);
    match drachm::mir::cleanup::ineligible_reason(&hir, &mir) {
        None => {
            assert!(drachm::mir::cleanup::eligible(&hir, &mir));
            Ok(())
        }
        Some(why) => Err(why),
    }
}

fn all_programs() -> Vec<PathBuf> {
    let mut files = sources("tests/run");
    files.extend(sources("examples"));
    files
}

/// Build `src` with the `drachm` binary (`-O` or not, with the given
/// `DRACHM_BACKEND`), run it with the leak check on, and return stdout, exit
/// code and stderr.
fn build_and_run(src: &Path, optimise: bool, backend: &str, tag: &str) -> Result<(String, Option<i32>, String), String> {
    let stem = src.file_stem().unwrap().to_str().unwrap();
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("be-{tag}-{stem}"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_drachm"));
    cmd.current_dir(root()).env("DRACHM_BACKEND", backend).arg("build").arg("-o").arg(&exe);
    if optimise {
        cmd.arg("-O");
    }
    let build = cmd.arg(src.strip_prefix(root()).unwrap()).output().map_err(|e| e.to_string())?;
    if !build.status.success() {
        return Err(format!("did not compile (backend {backend}, -O {optimise}):\n{}", text(&build.stderr)));
    }
    let run: Output = Command::new(&exe).env("DRACHM_CHECK_LEAKS", "1").output().map_err(|e| format!("could not run: {e}"))?;
    let _ = fs::remove_file(&exe);
    let stderr = text(&run.stderr);
    if let Some(leak) = stderr.lines().find(|l| l.starts_with("drachm: leaked")) {
        return Err(format!("{leak} (backend {backend}, -O {optimise})"));
    }
    Ok((text(&run.stdout), run.status.code(), stderr))
}

/// Programs the MIR backends accept behave identically on the four paths:
/// {Cranelift, C} x {HIR, MIR}. Prints the eligible and ineligible lists.
#[test]
fn mir_backend_matches_hir() {
    let files = all_programs();
    let (mut eligible, mut ineligible) = (Vec::new(), Vec::new());
    for f in &files {
        match eligibility(f) {
            Ok(()) => eligible.push(f.clone()),
            Err(why) => ineligible.push((f.clone(), why)),
        }
    }
    let name = |f: &Path| f.strip_prefix(root()).unwrap().display().to_string();
    println!("eligible for the MIR backends ({} of {}):", eligible.len(), files.len());
    for f in &eligible {
        println!("  {}", name(f));
    }
    println!("ineligible ({}):", ineligible.len());
    for (f, why) in &ineligible {
        println!("  {}: {why}", name(f));
    }
    assert!(!eligible.is_empty());

    run_all(eligible, |src| {
        let mut results = Vec::new();
        for (optimise, backend) in [(false, "hir"), (false, "mir"), (true, "hir"), (true, "mir")] {
            let tag = format!("{backend}{}", optimise as u8);
            let (out, code, _) = build_and_run(src, optimise, backend, &tag)?;
            results.push((format!("{backend}{}", if optimise { " -O" } else { "" }), out, code));
        }
        let first = &results[0];
        for r in &results[1..] {
            if r.1 != first.1 || r.2 != first.2 {
                return Err(format!("{} gave {:?} / {:?}\n{} gave {:?} / {:?}", first.0, first.1, first.2, r.0, r.1, r.2));
            }
        }
        Ok(())
    });
}

/// The four numeric benchmarks are what stage 1 exists for.
#[test]
fn numeric_benchmarks_are_eligible() {
    for name in ["collatz", "fib", "mandel", "primes"] {
        let path = root().join(format!("bench/{name}.drh"));
        eligibility(&path).unwrap_or_else(|why| panic!("bench/{name}.drh is not eligible: {why}"));
    }
}

/// A program with a `string` local still builds (through the HIR backends),
/// unless the MIR backends are demanded.
#[test]
fn ineligible_falls_back() {
    let src = root().join("tests/run/strings.drh");
    assert!(eligibility(&src).is_err(), "strings.drh was expected to need the HIR backends");
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join("be-fallback");
    let rel = src.strip_prefix(root()).unwrap();
    let build = |backend: Option<&str>| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_drachm"));
        cmd.current_dir(root()).env_remove("DRACHM_BACKEND");
        if let Some(b) = backend {
            cmd.env("DRACHM_BACKEND", b);
        }
        cmd.arg("build").arg("-o").arg(&exe).arg(rel).output().unwrap()
    };
    let ok = build(None);
    assert!(ok.status.success(), "{}", text(&ok.stderr));
    let _ = fs::remove_file(&exe);
    let demanded = build(Some("mir"));
    assert!(!demanded.status.success());
    assert!(text(&demanded.stderr).contains("not supported by the MIR backend yet"), "{}", text(&demanded.stderr));
}

/// User-written borrows cannot go through the HIR backends; the MIR ones build
/// them (`build_unchecked`, until the borrow checker lets `build` do it).
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
            let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("be-borrow-{i}-{}", optimise as u8));
            drachm::build_unchecked(&hir, &exe, optimise).unwrap_or_else(|e| panic!("program {i} (-O {optimise}): {e}"));
            let run = Command::new(&exe).output().unwrap();
            let _ = fs::remove_file(&exe);
            assert_eq!(text(&run.stdout), *want, "program {i} (-O {optimise})");
        }
    }
}
