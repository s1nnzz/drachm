//! Dependencies (step 7, WP3): path and git dependencies, the `deps/` folder,
//! the lock file. No network: a git dependency is a local repository, reached
//! through a `file://` address, made in a temp folder by the test.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use drachm::config::{self, Config};
use drachm::deps::{self, Root};
use drachm::diag::{Diagnostic, SourceMap};

/// A folder of the test's own, removed when the test ends.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("drachm-deps-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn put(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Runs git in `dir` with a fixed identity; panics if git fails.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=drachm-test", "-c", "user.email=test@example.org", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .output()
        .expect("git must be installed to run these tests");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A library repository with one commit tagged `tag`. Returns its address.
fn library_repo(s: &Scratch, name: &str, tag: &str, body: &str) -> String {
    let dir = s.path(name);
    s.put(&format!("{name}/drachm.toml"), &format!("[project]\nname = \"{name}\"\nkind = \"library\"\n"));
    s.put(&format!("{name}/src/main.drh"), body);
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    git(&dir, &["tag", tag]);
    format!("file://{}", dir.display())
}

/// Commits a change to the library and moves its tag to it.
fn move_tag(s: &Scratch, name: &str, tag: &str, body: &str) {
    let dir = s.path(name);
    s.put(&format!("{name}/src/main.drh"), body);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "second"]);
    git(&dir, &["tag", "-f", tag]);
}

fn head(s: &Scratch, name: &str) -> String {
    git(&s.path(name), &["rev-parse", "HEAD"])
}

/// The config of the project in `dir`, read as `config::load` reads it.
fn config_of(dir: &Path) -> (Config, SourceMap) {
    let mut map = SourceMap::new();
    let cfg = config::load(&mut map, &dir.join("drachm.toml"), "drachm.toml", false).unwrap_or_else(|d| panic!("{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>()));
    (cfg, map)
}

fn resolve(dir: &Path) -> Result<Vec<Root>, Vec<Diagnostic>> {
    let (cfg, mut map) = config_of(dir);
    deps::resolve(dir, &cfg, &mut map)
}

fn ok(dir: &Path) -> Vec<Root> {
    resolve(dir).unwrap_or_else(|d| panic!("{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>()))
}

fn codes(d: &[Diagnostic]) -> Vec<&'static str> {
    d.iter().map(|d| d.code.id).collect()
}

fn project_with_git(s: &Scratch, url: &str, tag: &str) -> PathBuf {
    s.put("app/drachm.toml", &format!("[project]\nname = \"app\"\n[dependencies]\nsdl = {{ git = \"{url}\", tag = \"{tag}\" }}\n"));
    s.path("app")
}

#[test]
fn git_dependency_is_cloned_into_deps_at_its_tag_and_locked() {
    let s = Scratch::new("fetch");
    let url = library_repo(&s, "sdl", "v0.4", "none helper() { }\n");
    let app = project_with_git(&s, &url, "v0.4");
    let roots = ok(&app);
    assert_eq!(roots.len(), 1);
    let root = &roots[0];
    assert_eq!(root.name, "sdl");
    assert_eq!(root.config.project.name.as_deref(), Some("sdl"));
    let commit = head(&s, "sdl");
    assert_eq!(root.commit.as_deref(), Some(commit.as_str()));
    assert_eq!(root.dir, app.join("deps").join("sdl"));
    assert_eq!(root.src, app.join("deps").join("sdl").join("src"));
    assert!(root.src.join("main.drh").is_file());
    let lock = fs::read_to_string(app.join("drachm.lock")).unwrap();
    assert!(lock.contains(&format!("commit = \"{commit}\"")), "{lock}");
    assert!(lock.contains("tag = \"v0.4\""), "{lock}");
}

#[test]
fn the_locked_commit_is_used_without_fetching_again() {
    let s = Scratch::new("lock");
    let url = library_repo(&s, "sdl", "v0.4", "none helper() { }\n");
    let app = project_with_git(&s, &url, "v0.4");
    let first = ok(&app);
    let lock_before = fs::read_to_string(app.join("drachm.lock")).unwrap();
    // The upstream is gone: the locked commit is already in deps/, so it works.
    fs::remove_dir_all(s.path("sdl")).unwrap();
    let second = ok(&app);
    assert_eq!(first[0].commit, second[0].commit);
    assert_eq!(fs::read_to_string(app.join("drachm.lock")).unwrap(), lock_before);
}

#[test]
fn a_moved_tag_keeps_the_locked_commit_and_is_refused_only_when_fetched() {
    let s = Scratch::new("moved");
    let url = library_repo(&s, "sdl", "v0.4", "none helper() { }\n");
    let app = project_with_git(&s, &url, "v0.4");
    let first = ok(&app);
    let old = first[0].commit.clone().unwrap();
    move_tag(&s, "sdl", "v0.4", "none helper() { }\nnone more() { }\n");
    let new = head(&s, "sdl");
    assert_ne!(old, new);
    // The locked commit is in deps/: the build keeps it, the lock is unchanged.
    let kept = ok(&app);
    assert_eq!(kept[0].commit.as_deref(), Some(old.as_str()));
    // With deps/sdl gone the commit must be fetched from the tag, which now
    // names another commit: E0809.
    fs::remove_dir_all(app.join("deps")).unwrap();
    let d = resolve(&app).unwrap_err();
    assert_eq!(codes(&d), ["E0809"]);
    assert!(d[0].message.contains("now points at another commit"), "{}", d[0].message);
    // Deleting the lock file takes the new commit, and records it.
    fs::remove_file(app.join("drachm.lock")).unwrap();
    let third = ok(&app);
    assert_eq!(third[0].commit.as_deref(), Some(new.as_str()));
    assert!(fs::read_to_string(app.join("drachm.lock")).unwrap().contains(&new));
}

#[test]
fn an_unknown_tag_is_e0808() {
    let s = Scratch::new("badtag");
    let url = library_repo(&s, "sdl", "v0.4", "none helper() { }\n");
    let app = project_with_git(&s, &url, "v9.9");
    let d = resolve(&app).unwrap_err();
    assert_eq!(codes(&d), ["E0808"]);
    assert_eq!(d[0].message, format!("cannot fetch the dependency `sdl` from `{url}` at tag `v9.9`"));
    assert!(!app.join("drachm.lock").exists());
}

#[test]
fn path_and_git_dependencies_are_transitive_and_found_once() {
    let s = Scratch::new("transitive");
    let url = library_repo(&s, "sdl", "v0.4", "none helper() { }\n");
    s.put("utils/drachm.toml", &format!("[project]\nname = \"utils\"\nkind = \"library\"\n[dependencies]\nsdl = {{ git = \"{url}\", tag = \"v0.4\" }}\n"));
    s.put("utils/src/main.drh", "none util() { }\n");
    s.put("app/drachm.toml", &format!("[project]\nname = \"app\"\n[dependencies]\nutils = {{ path = \"../utils\" }}\nsdl = {{ git = \"{url}\", tag = \"v0.4\" }}\n"));
    let app = s.path("app");
    let roots = ok(&app);
    let names: Vec<&str> = roots.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["utils", "sdl"]);
    assert_eq!(roots[0].commit, None);
    assert_eq!(roots[0].dir, s.path("app/../utils"));
    // The lock file lists the git dependency once, even though two lines name it.
    let lock = fs::read_to_string(app.join("drachm.lock")).unwrap();
    assert_eq!(lock.matches("[[git]]").count(), 1, "{lock}");
}

#[test]
fn two_sources_under_one_name_are_e0804() {
    let s = Scratch::new("clash");
    let url = library_repo(&s, "sdl", "v0.4", "none helper() { }\n");
    let other = library_repo(&s, "other", "v0.4", "none helper() { }\n");
    s.put("utils/drachm.toml", &format!("[project]\nname = \"utils\"\nkind = \"library\"\n[dependencies]\nsdl = {{ git = \"{other}\", tag = \"v0.4\" }}\n"));
    s.put("utils/src/main.drh", "\n");
    s.put("app/drachm.toml", &format!("[project]\nname = \"app\"\n[dependencies]\nutils = {{ path = \"../utils\" }}\nsdl = {{ git = \"{url}\", tag = \"v0.4\" }}\n"));
    let d = resolve(&s.path("app")).unwrap_err();
    assert_eq!(codes(&d), ["E0804"]);
    assert_eq!(d[0].message, "two dependencies are named `sdl`");
}

#[test]
fn a_dependency_obeys_its_own_config() {
    let s = Scratch::new("own");
    s.put("utils/drachm.toml", "[project]\nname = \"utils\"\nkind = \"library\"\n[behaviour]\noverflow = \"panic\"\n");
    s.put("utils/src/main.drh", "none util() { }\n");
    s.put("app/drachm.toml", "[project]\nname = \"app\"\n[dependencies]\nutils = { path = \"../utils\" }\n");
    let roots = ok(&s.path("app"));
    assert_eq!(roots[0].config.behaviour.overflow, config::Overflow::Panic);
}

#[test]
fn a_cycle_of_path_dependencies_is_e0810() {
    let s = Scratch::new("cycle");
    s.put("a/drachm.toml", "[project]\nname = \"a\"\nkind = \"library\"\n[dependencies]\nb = { path = \"../b\" }\n");
    s.put("a/src/main.drh", "\n");
    s.put("b/drachm.toml", "[project]\nname = \"b\"\nkind = \"library\"\n[dependencies]\na = { path = \"../a\" }\n");
    s.put("b/src/main.drh", "\n");
    let d = resolve(&s.path("a")).unwrap_err();
    assert_eq!(codes(&d), ["E0810"]);
    assert_eq!(d[0].message, "the dependency `a` is part of a cycle");
}

#[test]
fn a_dependency_must_have_a_drachm_toml_and_an_entry() {
    let s = Scratch::new("missing");
    s.put("plain/src/main.drh", "\n");
    s.put("app/drachm.toml", "[project]\nname = \"app\"\n[dependencies]\nplain = { path = \"../plain\" }\n");
    let d = resolve(&s.path("app")).unwrap_err();
    assert_eq!(codes(&d), ["E0804"]);

    s.put("nomain/drachm.toml", "[project]\nname = \"nomain\"\nkind = \"library\"\n");
    s.put("app/drachm.toml", "[project]\nname = \"app\"\n[dependencies]\nnomain = { path = \"../nomain\" }\n");
    let d = resolve(&s.path("app")).unwrap_err();
    assert_eq!(codes(&d), ["E0812"]);
}

#[test]
fn a_bad_lock_file_is_reported_not_ignored() {
    let s = Scratch::new("badlock");
    s.put("app/drachm.toml", "[project]\nname = \"app\"\n");
    s.put("app/drachm.lock", "[[git]\nurl = 1\n");
    let d = resolve(&s.path("app")).unwrap_err();
    assert_eq!(codes(&d), ["E0800"]);
}
