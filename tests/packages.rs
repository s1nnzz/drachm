//! The package manager (step 7.6): `drachm add`, `remove`, `update`,
//! `install`, `^` ranges, packages side by side, the lock file and its hashes,
//! C packages. No network: every repository is a local one in a temp folder,
//! reached through a `file://` address.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use drachm::pkg;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("drachm-pkg-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn put(&self, rel: &str, text: &str) {
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap()
    }

    /// The address of the repository at `rel`.
    fn url(&self, rel: &str) -> String {
        format!("file://{}", self.path(rel).display())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=drachm-test", "-c", "user.email=test@example.org", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", "-c", "init.defaultBranch=main"])
        .args(args)
        .output()
        .expect("git must be installed to run these tests");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// Commits the files (replacing the library's files of the same name) to the
/// repository at `rel`, making it first if needed, and tags the commit.
fn release(s: &Scratch, rel: &str, tag: &str, files: &[(&str, &str)]) {
    let dir = s.path(rel);
    let fresh = !dir.join(".git").exists();
    for (name, text) in files {
        s.put(&format!("{rel}/{name}"), text);
    }
    if fresh {
        git(&dir, &["init", "-q"]);
    }
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", &format!("release {tag}")]);
    git(&dir, &["tag", tag]);
}

/// A library with one module, `value`, that returns `n`.
fn value_library(s: &Scratch, rel: &str, tag: &str, n: i32) {
    release(
        s,
        rel,
        tag,
        &[
            ("drachm.toml", "[project]\nname = \"value\"\nkind = \"library\"\npublic_modules = [\"value\"]\n"),
            ("src/main.drh", "none unused() { }\n"),
            ("src/value.drh", &format!("out sint32 get() {{ return {n}; }}\n")),
        ],
    );
}

fn app(s: &Scratch) -> PathBuf {
    s.put("app/drachm.toml", "# my app\n[project]\nname = \"app\"   # the name\n\n[dependencies]\n# nothing yet\n");
    s.put("app/src/main.drh", "none main() { }\n");
    s.path("app")
}

fn drachm(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_drachm")).current_dir(cwd).env("NO_COLOR", "1").args(args).output().expect("could not start drachm")
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).replace("\r\n", "\n")
}

fn ok(r: Result<Vec<String>, drachm::config::Failure>) -> Vec<String> {
    r.unwrap_or_else(|f| panic!("{}", f.render(false)))
}

fn err_codes(r: Result<Vec<String>, drachm::config::Failure>) -> Vec<&'static str> {
    r.expect_err("should fail").diags.iter().map(|d| d.code.id).collect()
}

#[test]
fn add_an_exact_tag_edits_the_manifest_and_writes_the_lock() {
    let s = Scratch::new("add");
    value_library(&s, "repo/value-drh", "v0.4.1", 1);
    let dir = app(&s);
    let url = s.url("repo/value-drh");
    let out = ok(pkg::add(&dir, &format!("{url}@v0.4.1"), None));
    assert_eq!(out, [format!("added value_drh: {url} at v0.4.1")]);
    let toml = s.read("app/drachm.toml");
    // The owner's comments and layout stay; the entry is one inline table.
    assert!(toml.starts_with("# my app\n[project]\nname = \"app\"   # the name\n"), "{toml}");
    assert!(toml.contains("# nothing yet\n"), "{toml}");
    assert!(toml.contains(&format!("value_drh = {{ git = \"{url}\", tag = \"v0.4.1\" }}")), "{toml}");
    assert!(dir.join("deps/value-drh@0.4.1/drachm.toml").is_file());
    let lock = s.read("app/drachm.lock");
    assert!(lock.starts_with("# written by drachm, do not edit\n\n[[package]]\nname = \"value-drh\"\n"), "{lock}");
    assert!(lock.contains(&format!("source = \"{url}\"\ntag = \"v0.4.1\"\ncommit = \"")), "{lock}");
    assert!(lock.contains("hash = \"sha256:"), "{lock}");
    // Adding it again changes the entry in place.
    let again = ok(pkg::add(&dir, &format!("{url}@v0.4.1"), None));
    assert!(again[0].starts_with("changed value_drh"), "{again:?}");
}

#[test]
fn a_caret_range_takes_the_newest_matching_tag_and_a_bare_address_the_newest_tag() {
    let s = Scratch::new("range");
    for (tag, n) in [("v0.3.9", 1), ("v0.4.0", 2), ("v0.4.7", 3), ("v0.5.0", 4), ("v1.0.0", 5)] {
        value_library(&s, "repo/value", tag, n);
    }
    let dir = app(&s);
    let url = s.url("repo/value");
    let out = ok(pkg::add(&dir, &format!("{url}@^0.4"), Some("pinned")));
    assert_eq!(out, [format!("added pinned: {url} at v0.4.7")]);
    assert!(s.read("app/drachm.toml").contains(&format!("pinned = {{ git = \"{url}\", version = \"^0.4\" }}")));
    assert!(dir.join("deps/value@0.4.7").is_dir());
    ok(pkg::remove(&dir, "pinned"));
    let out = ok(pkg::add(&dir, &url, Some("newest")));
    assert_eq!(out, [format!("added newest: {url} at v1.0.0")]);
    assert!(s.read("app/drachm.toml").contains(&format!("newest = {{ git = \"{url}\", tag = \"v1.0.0\" }}")));
}

#[test]
fn remove_edits_the_manifest_and_forgets_the_package() {
    let s = Scratch::new("remove");
    value_library(&s, "repo/value", "v1.0.0", 1);
    let dir = app(&s);
    let url = s.url("repo/value");
    ok(pkg::add(&dir, &format!("{url}@v1.0.0"), None));
    assert!(dir.join("drachm.lock").is_file());
    let out = ok(pkg::remove(&dir, "value"));
    assert_eq!(out, ["removed value"]);
    let toml = s.read("app/drachm.toml");
    assert!(!toml.contains("git ="), "{toml}");
    assert!(toml.contains("# nothing yet"), "{toml}");
    assert!(!dir.join("drachm.lock").exists());
    assert!(!dir.join("deps/value@1.0.0").exists());
    assert_eq!(err_codes(pkg::remove(&dir, "value")), ["E0817"]);
}

#[test]
fn update_takes_newer_tags_inside_the_range_only() {
    let s = Scratch::new("update");
    value_library(&s, "repo/value", "v0.4.0", 1);
    value_library(&s, "repo/value", "v0.5.0", 9);
    let dir = app(&s);
    let url = s.url("repo/value");
    ok(pkg::add(&dir, &format!("{url}@^0.4"), None));
    assert!(dir.join("deps/value@0.4.0").is_dir());
    assert_eq!(ok(pkg::update(&dir, None)), ["everything is up to date"]);
    value_library(&s, "repo/value", "v0.4.3", 2);
    // A build keeps the locked tag even though a newer one fits.
    let before = s.read("app/drachm.lock");
    assert!(drachm(&dir, &["check", "."]).status.success());
    assert_eq!(s.read("app/drachm.lock"), before);
    let out = ok(pkg::update(&dir, Some("value")));
    assert_eq!(out, ["updated value: v0.4.0 -> v0.4.3"]);
    assert!(dir.join("deps/value@0.4.3").is_dir());
    assert!(!dir.join("deps/value@0.4.0").exists(), "the old version is pruned");
    assert!(s.read("app/drachm.lock").contains("tag = \"v0.4.3\""));
    assert_eq!(err_codes(pkg::update(&dir, Some("nothing"))), ["E0817"]);
}

#[test]
fn update_follows_a_moved_exact_tag() {
    let s = Scratch::new("moved");
    value_library(&s, "repo/value", "v1.0.0", 1);
    let dir = app(&s);
    let url = s.url("repo/value");
    ok(pkg::add(&dir, &format!("{url}@v1.0.0"), None));
    let repo = s.path("repo/value");
    s.put("repo/value/src/value.drh", "out sint32 get() { return 2; }\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "again"]);
    git(&repo, &["tag", "-f", "v1.0.0"]);
    let out = ok(pkg::update(&dir, Some("value")));
    assert!(out[0].starts_with("updated value v1.0.0: the tag moved, "), "{out:?}");
}

#[test]
fn install_fetches_deps_again_from_the_lock() {
    let s = Scratch::new("install");
    value_library(&s, "repo/value", "v1.0.0", 1);
    let dir = app(&s);
    let url = s.url("repo/value");
    ok(pkg::add(&dir, &format!("{url}@v1.0.0"), None));
    let lock = s.read("app/drachm.lock");
    fs::remove_dir_all(dir.join("deps")).unwrap();
    assert_eq!(ok(pkg::install(&dir)), ["installed 1 package"]);
    assert!(dir.join("deps/value@1.0.0/src/value.drh").is_file());
    assert_eq!(s.read("app/drachm.lock"), lock);
    // Install also repairs a package that was edited by hand.
    fs::write(dir.join("deps/value@1.0.0/src/value.drh"), "out sint32 get() { return 99; }\n").unwrap();
    ok(pkg::install(&dir));
    assert!(fs::read_to_string(dir.join("deps/value@1.0.0/src/value.drh")).unwrap().contains("return 1;"));
}

#[test]
fn a_package_that_differs_from_the_lock_is_refused() {
    let s = Scratch::new("hash");
    value_library(&s, "repo/value", "v1.0.0", 1);
    let dir = app(&s);
    let url = s.url("repo/value");
    ok(pkg::add(&dir, &format!("{url}@v1.0.0"), None));
    // An edited file in deps/.
    fs::write(dir.join("deps/value@1.0.0/src/value.drh"), "out sint32 get() { return 99; }\n").unwrap();
    let check = drachm(&dir, &["check", "."]);
    assert!(!check.status.success());
    assert!(text(&check.stderr).contains("E0816"), "{}", text(&check.stderr));
    // A hash in the lock that the fetched files do not have.
    ok(pkg::install(&dir));
    let lock = s.read("app/drachm.lock");
    let hash = lock.lines().find_map(|l| l.strip_prefix("hash = \"")).unwrap().trim_end_matches('"').to_string();
    fs::write(dir.join("drachm.lock"), lock.replace(&hash, &format!("sha256:{}", "0".repeat(64)))).unwrap();
    let install = drachm(&dir, &["install"]);
    assert!(!install.status.success());
    assert!(text(&install.stderr).contains("E0816"), "{}", text(&install.stderr));
}

#[test]
fn a_missing_tag_or_range_is_reported_and_the_manifest_is_unchanged() {
    let s = Scratch::new("missing");
    value_library(&s, "repo/value", "v1.0.0", 1);
    let dir = app(&s);
    let url = s.url("repo/value");
    let before = s.read("app/drachm.toml");
    assert_eq!(err_codes(pkg::add(&dir, &format!("{url}@v9.9"), None)), ["E0815"]);
    assert_eq!(err_codes(pkg::add(&dir, &format!("{url}@^7"), None)), ["E0815"]);
    assert_eq!(err_codes(pkg::add(&dir, &format!("{url}@^x"), None)), ["E0817"]);
    assert_eq!(s.read("app/drachm.toml"), before);
    assert!(!dir.join("drachm.lock").exists());
    // A repository with no version tags: a bare address has nothing to take.
    release(&s, "repo/untagged", "nightly", &[("drachm.toml", "[project]\nname = \"u\"\nkind = \"library\"\n"), ("src/main.drh", "\n")]);
    assert_eq!(err_codes(pkg::add(&dir, &s.url("repo/untagged"), None)), ["E0815"]);
    // Not a repository at all.
    assert_eq!(err_codes(pkg::add(&dir, &s.url("repo/nonexistent"), None)), ["E0808"]);
    assert_eq!(s.read("app/drachm.toml"), before);
}

#[test]
fn two_versions_of_a_package_sit_side_by_side() {
    let s = Scratch::new("sidebyside");
    value_library(&s, "repo/json", "v1.4.0", 1);
    value_library(&s, "repo/json", "v2.1.0", 2);
    let json = s.url("repo/json");
    // `tools` asks for json 1.4.0; the app asks for json 2.
    release(
        &s,
        "repo/tools",
        "v1.0.0",
        &[
            (
                "drachm.toml",
                &format!("[project]\nname = \"tools\"\nkind = \"library\"\npublic_modules = [\"report\"]\n\n[dependencies]\njson = {{ git = \"{json}\", tag = \"v1.4.0\" }}\n"),
            ),
            ("src/main.drh", "none unused() { }\n"),
            ("src/report.drh", "in json.value as value;\n\nout sint32 seen() { return value::get(); }\n"),
        ],
    );
    let dir = app(&s);
    ok(pkg::add(&dir, &format!("{json}@^2"), Some("json")));
    ok(pkg::add(&dir, &format!("{}@v1.0.0", s.url("repo/tools")), None));
    s.put(
        "app/src/main.drh",
        "in io;\nin json.value as value;\nin tools.report as report;\n\nnone main() {\n    io::outln(f\"app: ${value::get()}, tools: ${report::seen()}\");\n}\n",
    );
    assert!(dir.join("deps/json@1.4.0").is_dir() && dir.join("deps/json@2.1.0").is_dir());
    let lock = s.read("app/drachm.lock");
    assert_eq!(lock.matches("[[package]]").count(), 3, "{lock}");
    for flags in [&["run", "."][..], &["run", ".", "-O"][..]] {
        let run = drachm(&dir, flags);
        assert!(run.status.success(), "{flags:?}: {}", text(&run.stderr));
        assert_eq!(text(&run.stdout), "app: 2, tools: 1\n", "{flags:?}");
    }
}

#[test]
fn a_range_reuses_a_version_the_build_already_has() {
    let s = Scratch::new("shared");
    value_library(&s, "repo/json", "v1.2.0", 1);
    value_library(&s, "repo/json", "v1.4.0", 2);
    let json = s.url("repo/json");
    release(
        &s,
        "repo/tools",
        "v1.0.0",
        &[
            ("drachm.toml", &format!("[project]\nname = \"tools\"\nkind = \"library\"\n\n[dependencies]\njson = {{ git = \"{json}\", version = \"^1.2\" }}\n")),
            ("src/main.drh", "none unused() { }\n"),
        ],
    );
    let dir = app(&s);
    ok(pkg::add(&dir, &format!("{json}@v1.2.0"), Some("json")));
    ok(pkg::add(&dir, &format!("{}@v1.0.0", s.url("repo/tools")), None));
    // tools' `^1.2` is satisfied by the 1.2.0 the app already uses: one json in deps/.
    assert!(dir.join("deps/json@1.2.0").is_dir());
    assert!(!dir.join("deps/json@1.4.0").exists());
}

#[test]
fn a_c_package_compiles_its_sources_and_shares_its_headers() {
    let s = Scratch::new("cpkg");
    release(
        &s,
        "repo/triple-drh",
        "v1.0.0",
        &[
            (
                "drachm.toml",
                "[project]\nname = \"triple\"\nkind = \"library\"\npublic_modules = [\"triple\"]\n\n[native]\ninclude = [\"c/include\"]\nsources = [\"c/triple.c\"]\nlink = [\"m\"]\nunsafe = true\n",
            ),
            ("c/include/triple.h", "#ifndef TRIPLE_H\n#define TRIPLE_H\nint triple_of(int x);\n#endif\n"),
            ("c/triple.c", "#include \"triple.h\"\nint triple_of(int x) { return x * 3; }\n"),
            ("src/main.drh", "none unused() { }\n"),
            (
                "src/triple.drh",
                "in * from c.\"triple.h\" as t;\n\nout sint32 times3(sint32 x) {\n    unsafe {\n        return t::triple_of(x as c::int) as sint32;\n    }\n}\n",
            ),
        ],
    );
    let dir = app(&s);
    ok(pkg::add(&dir, &format!("{}@^1", s.url("repo/triple-drh")), None));
    s.put("app/src/main.drh", "in io;\nin triple_drh.triple as triple;\n\nnone main() {\n    io::outln(f\"triple: ${triple::times3(14)}\");\n}\n");
    let check = drachm(&dir, &["check", "."]);
    if text(&check.stderr).contains("libclang") {
        eprintln!("skipped: libclang is not installed");
        return;
    }
    assert!(check.status.success(), "{}", text(&check.stderr));
    for flags in [&["run", "."][..], &["run", ".", "-O"][..]] {
        let run = drachm(&dir, flags);
        assert!(run.status.success(), "{flags:?}: {}", text(&run.stderr));
        assert_eq!(text(&run.stdout), "triple: 42\n", "{flags:?}");
    }
    // The app's own C imports find the header the package ships.
    s.put(
        "app/src/main.drh",
        "//--!$unsafe: true\nin io;\nin * from c.\"triple.h\" as t;\n\nnone main() {\n    unsafe { io::outln(f\"direct: ${t::triple_of(5) as sint32}\"); }\n}\n",
    );
    let check = drachm(&dir, &["check", "."]);
    assert!(check.status.success(), "{}", text(&check.stderr));
}

#[test]
fn the_commands_work_from_the_command_line() {
    let s = Scratch::new("cli");
    value_library(&s, "repo/value", "v0.2.0", 1);
    value_library(&s, "repo/value", "v0.2.5", 2);
    let url = s.url("repo/value");
    assert!(drachm(&s.dir, &["new", "demo"]).status.success());
    assert_eq!(s.read("demo/.gitignore"), "deps/\n");
    let dir = s.path("demo");
    let add = drachm(&dir, &["add", &format!("{url}@^0.2"), "--name", "val"]);
    assert!(add.status.success(), "{}", text(&add.stderr));
    assert_eq!(text(&add.stdout), format!("added val: {url} at v0.2.5\n"));
    let update = drachm(&dir, &["update"]);
    assert_eq!(text(&update.stdout), "everything is up to date\n");
    let bad = drachm(&dir, &["remove", "nothing"]);
    assert!(!bad.status.success());
    assert!(text(&bad.stderr).contains("error[E0817]: `nothing` is not in [dependencies]"), "{}", text(&bad.stderr));
    let install = drachm(&dir, &["install"]);
    assert_eq!(text(&install.stdout), "installed 1 package\n");
    let remove = drachm(&dir, &["remove", "val"]);
    assert_eq!(text(&remove.stdout), "removed val\n");
    // Outside a project there is nothing to change.
    let outside = drachm(&s.path("repo"), &["install"]);
    assert!(text(&outside.stderr).contains("E0817"), "{}", text(&outside.stderr));
}
