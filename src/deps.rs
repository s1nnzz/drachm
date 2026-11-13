//! Dependencies: finding the folder of each one a project uses.
//!
//! A `path` dependency is a folder on this machine (relative to the folder
//! whose `drachm.toml` names it). A `git` dependency is cloned at its exact
//! `tag` into `deps/NAME/` in the project's folder (owner, 2026-11-13: the
//! downloaded packages live per project, not in a user cache). The commit it
//! came from is written to `drachm.lock` beside the project's `drachm.toml`.
//!
//! Versions are git tags, pinned exactly: no ranges, and no `drachm add`
//! command yet (step 7.6). A build with a lock file uses the locked commit:
//! if `deps/NAME/` already holds it, nothing is fetched, even when the tag has
//! moved since. Only when the commit must be fetched and the tag now names
//! another commit is the build refused (E0809); deleting `drachm.lock` takes
//! the new commit.
//!
//! Every dependency of the build, transitively, is cloned into the same
//! `deps/` folder of the top project. Two different sources under one name are
//! E0804. Each dependency's own `drachm.toml` is read and its own dependencies
//! are resolved in turn; a cycle is E0810. The folders found are returned as
//! `Root`s. Wiring their modules into `in` paths is the module system's job
//! (step 7, WP2): this module only finds them.
//!
//! Git runs as the `git` program on the `PATH`. Prompts are off, so a private
//! repository fails rather than waiting for a password.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use toml_edit::{ImDocument, Item};

use crate::config::{self, Config, Source, LOCK_FILE, PROJECT_FILE};
use crate::diag::{codes, Diagnostic, SourceMap, Span};

/// The folder under the project's folder where git dependencies are cloned.
pub const DEPS_DIR: &str = "deps";

/// One dependency, found as a folder.
#[derive(Debug, Clone)]
pub struct Root {
    /// The name the project gives it: `sdl` in `in sdl...`.
    pub name: String,
    /// The folder with its `drachm.toml`.
    pub dir: PathBuf,
    /// The source root: the folder of the dependency's entry file.
    pub src: PathBuf,
    /// The dependency's own config: it obeys its own `drachm.toml`.
    pub config: Config,
    /// The commit a git dependency was fetched at; `None` for a path.
    pub commit: Option<String>,
}

/// One git dependency as the lock file records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locked {
    pub url: String,
    pub tag: String,
    pub commit: String,
}

/// Resolves every dependency of the project in `dir` (whose config is
/// `config`), fetching git ones into `dir/deps/` and writing `drachm.lock`.
/// Errors are the diagnostics; `map` gets every config file read, so they can
/// be shown.
pub fn resolve(dir: &Path, config: &Config, map: &mut SourceMap) -> Result<Vec<Root>, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let locked = read_lock(dir, map, &mut diags);
    if !diags.is_empty() {
        return Err(diags);
    }
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut r = Resolver { map, diags, deps_dir: dir.join(DEPS_DIR), locked, fetched: Vec::new(), stack: vec![canon], roots: Vec::new() };
    r.dependencies(dir, config);
    let Resolver { diags, fetched, roots, .. } = r;
    let mut diags = diags;
    if diags.is_empty() {
        let locks: Vec<Locked> = fetched.into_iter().map(|(l, _)| l).collect();
        if let Err(d) = write_lock(dir, &locks) {
            diags.push(d);
        }
    }
    if diags.is_empty() { Ok(roots) } else { Err(diags) }
}

struct Resolver<'a> {
    map: &'a mut SourceMap,
    diags: Vec<Diagnostic>,
    /// `<project>/deps`.
    deps_dir: PathBuf,
    /// What the lock file says, as read.
    locked: Vec<Locked>,
    /// The git dependencies of this build, with their folders.
    fetched: Vec<(Locked, PathBuf)>,
    /// The folders whose dependencies are being resolved, outermost first.
    stack: Vec<PathBuf>,
    roots: Vec<Root>,
}

impl Resolver<'_> {
    /// The dependencies a config declares, relative to `base`.
    fn dependencies(&mut self, base: &Path, config: &Config) {
        for dep in &config.dependencies {
            match &dep.source {
                Source::Path(p) => self.enter(&dep.name, &base.join(p), None),
                Source::Git { url, tag } => {
                    // Already fetched for another dependency of this build?
                    if let Some((l, folder)) = self.fetched.iter().find(|(_, f)| f.file_name().is_some_and(|n| n == dep.name.as_str())) {
                        if l.url == *url && l.tag == *tag {
                            let (commit, folder) = (l.commit.clone(), folder.clone());
                            self.enter(&dep.name, &folder, Some(commit));
                        } else {
                            self.diags.push(
                                Diagnostic::new(&codes::E0804, format!("two dependencies are named `{}`", dep.name))
                                    .note_line(format!("one is `{}` at `{}`, the other `{url}` at `{tag}`", l.url, l.tag))
                                    .help("a dependency's name is its folder under deps/; give one of them another name"),
                            );
                        }
                        continue;
                    }
                    let locked = self.locked.iter().find(|l| l.url == *url && l.tag == *tag).map(|l| l.commit.clone());
                    match fetch(&self.deps_dir, &dep.name, url, tag, locked.as_deref()) {
                        Ok((commit, folder)) => {
                            let l = Locked { url: url.clone(), tag: tag.clone(), commit: commit.clone() };
                            self.fetched.push((l, folder.clone()));
                            self.enter(&dep.name, &folder, Some(commit));
                        }
                        Err(d) => self.diags.push(d),
                    }
                }
            }
        }
    }

    /// Reads the dependency in `folder` and resolves its own dependencies.
    fn enter(&mut self, name: &str, folder: &Path, commit: Option<String>) {
        let canon = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
        if self.stack.contains(&canon) {
            self.diags.push(
                Diagnostic::new(&codes::E0810, format!("the dependency `{name}` is part of a cycle"))
                    .note_line(format!("`{}` is already being resolved", folder.display()))
                    .help("a project's dependencies must form a tree"),
            );
            return;
        }
        if self.roots.iter().any(|r| r.dir.canonicalize().unwrap_or_else(|_| r.dir.clone()) == canon) {
            return;
        }
        let toml = folder.join(PROJECT_FILE);
        if !toml.is_file() {
            self.diags.push(
                Diagnostic::new(&codes::E0804, format!("the dependency `{name}` has no {PROJECT_FILE}"))
                    .note_line(format!("looked in `{}`", folder.display()))
                    .help("a dependency is a folder with a drachm.toml, like a project"),
            );
            return;
        }
        let shown = toml.display().to_string();
        let config = match config::load(self.map, &toml, &shown, false) {
            Ok(c) => c,
            Err(d) => {
                self.diags.extend(d);
                return;
            }
        };
        let entry = folder.join(&config.project.entry);
        if !entry.is_file() {
            self.diags.push(
                Diagnostic::new(&codes::E0812, format!("the entry file of the dependency `{name}` does not exist"))
                    .note_line(format!("looked for `{}`", entry.display()))
                    .help("a library needs an entry file too: `src/main.drh` unless `entry` says otherwise"),
            );
            return;
        }
        let src = entry.parent().map_or_else(|| folder.to_path_buf(), Path::to_path_buf);
        self.roots.push(Root { name: name.to_string(), dir: folder.to_path_buf(), src, config: config.clone(), commit });
        self.stack.push(canon);
        self.dependencies(folder, &config);
        self.stack.pop();
    }
}

fn fetch_error(name: &str, url: &str, tag: &str, why: &str) -> Diagnostic {
    Diagnostic::new(&codes::E0808, format!("cannot fetch the dependency `{name}` from `{url}` at tag `{tag}`"))
        .note_line(why.to_string())
        .help("check the address and the tag; `git ls-remote --tags URL` lists the tags")
}

/// The commit `dir` (a clone) is at, if git can say.
fn head_of(dir: &Path) -> Option<String> {
    let out = git(&["-C"]).arg(dir).args(["rev-parse", "HEAD"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Makes `deps/NAME` hold the commit of `url` at `tag`, and returns the commit
/// and the folder. With a locked commit, a folder already at that commit is
/// used as it is; otherwise the tag is cloned, and it must name the locked
/// commit if there is one (E0809).
fn fetch(deps_dir: &Path, name: &str, url: &str, tag: &str, locked: Option<&str>) -> Result<(String, PathBuf), Diagnostic> {
    let dest = deps_dir.join(name);
    if let Some(commit) = locked {
        if dest.is_dir() && head_of(&dest).as_deref() == Some(commit) {
            return Ok((commit.to_string(), dest));
        }
    }
    std::fs::create_dir_all(deps_dir).map_err(|e| fetch_error(name, url, tag, &format!("cannot make `{}`: {e}", deps_dir.display())))?;
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let tmp = deps_dir.join(format!(".{name}.tmp-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&tmp);
    match git(&["clone", "--quiet", "--depth", "1", "--branch", tag, "--", url]).arg(&tmp).output() {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(fetch_error(name, url, tag, &first_line(&String::from_utf8_lossy(&o.stderr), "git failed")));
        }
        Err(e) => return Err(fetch_error(name, url, tag, &format!("cannot run `git`: {e}; install git to use git dependencies"))),
    }
    let Some(commit) = head_of(&tmp) else {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(fetch_error(name, url, tag, "the clone has no commit to read"));
    };
    if let Some(old) = locked {
        if old != commit {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(Diagnostic::new(&codes::E0809, format!("the tag `{tag}` of `{url}` now points at another commit"))
                .note_line(format!("`drachm.lock` records {old}; the tag is at {commit}"))
                .help("if the new commit is wanted, delete drachm.lock and build again"));
        }
    }
    let _ = std::fs::remove_dir_all(&dest);
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(fetch_error(name, url, tag, &format!("cannot move the clone into `{}`: {e}", dest.display())));
    }
    Ok((commit, dest))
}

/// A `git` command with prompts off.
fn git(args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.args(args).env("GIT_TERMINAL_PROMPT", "0");
    c
}

fn first_line(text: &str, otherwise: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(otherwise);
    format!("git said: {line}")
}

/// Reads `drachm.lock` in `dir`. A missing file is no entries.
fn read_lock(dir: &Path, map: &mut SourceMap, diags: &mut Vec<Diagnostic>) -> Vec<Locked> {
    let path = dir.join(LOCK_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let (_, base) = map.add(path.display().to_string(), text.clone());
    let doc = match ImDocument::parse(text.as_str()) {
        Ok(d) => d,
        Err(e) => {
            let at = e.span().unwrap_or(0..0);
            diags.push(
                Diagnostic::at(&codes::E0800, Span::new(base + at.start, base + at.end), "this is not valid TOML")
                    .note_line(format!("in {LOCK_FILE}; delete it to make a new one")),
            );
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    if let Some(Item::ArrayOfTables(list)) = doc.as_table().get("git") {
        for t in list.iter() {
            let field = |k: &str| t.get(k).and_then(Item::as_str).map(str::to_string);
            match (field("url"), field("tag"), field("commit")) {
                (Some(url), Some(tag), Some(commit)) => out.push(Locked { url, tag, commit }),
                _ => diags.push(
                    Diagnostic::new(&codes::E0800, format!("an entry of {LOCK_FILE} lacks a `url`, `tag` or `commit`"))
                        .note_line("delete the file to make a new one"),
                ),
            }
        }
    }
    out
}

/// Writes the lock file for `locks` (sorted, one entry per URL and tag), or
/// removes it when there are none. Nothing is written when the file already
/// says the same.
fn write_lock(dir: &Path, locks: &[Locked]) -> Result<(), Diagnostic> {
    let path = dir.join(LOCK_FILE);
    let mut sorted: Vec<&Locked> = locks.iter().collect();
    sorted.sort_by(|a, b| (&a.url, &a.tag).cmp(&(&b.url, &b.tag)));
    sorted.dedup_by(|a, b| a.url == b.url && a.tag == b.tag);
    let existing = std::fs::read_to_string(&path).ok();
    if sorted.is_empty() {
        if existing.is_some() {
            std::fs::remove_file(&path).map_err(|e| lock_error(&path, &e.to_string()))?;
        }
        return Ok(());
    }
    let mut text = String::from("# Written by drachm: the commit each git dependency was fetched at. Commit this file.\n");
    for l in sorted {
        text += &format!("\n[[git]]\nurl = {}\ntag = {}\ncommit = {}\n", quote(&l.url), quote(&l.tag), quote(&l.commit));
    }
    if existing.as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    std::fs::write(&path, text).map_err(|e| lock_error(&path, &e.to_string()))
}

fn lock_error(path: &Path, why: &str) -> Diagnostic {
    Diagnostic::new(&codes::E0808, format!("cannot write `{}`", path.display())).note_line(why.to_string())
}

/// A TOML basic string.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_escapes_quotes_and_backslashes() {
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(quote("v0.4"), "\"v0.4\"");
    }
}
