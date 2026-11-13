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
