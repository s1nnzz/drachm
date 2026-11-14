//! Dependencies: finding the folder of each one a project uses.
//!
//! A `path` dependency is a folder on this machine (relative to the folder
//! whose `drachm.toml` names it). A `git` dependency is cloned at its exact
//! `tag` into `deps/NAME@VERSION/` in the project's folder (owner: downloaded
//! packages live per project, not in a user cache; NAME is the last part of
//! the address, VERSION the tag without a leading `v`). Two versions of one
//! package sit side by side, and each package that asks for a dependency gets
//! the version it asked for: the folders found are `Root`s, one per
//! (declaring project, alias), and the module system looks imports up by them.
//!
//! A dependency is `{ git = URL, tag = "v0.4" }` (one exact tag) or
//! `{ git = URL, version = "^0.4" }` (the newest tag in the range; see
//! `version.rs`). The lock file `drachm.lock` records, for every package of the
//! build (transitively), the tag it was resolved to, the commit and a sha256
//! hash of its files (`hash_dir`). A build uses the locked entry: a range keeps
//! the locked tag while it still fits, `drachm update` asks the repository for
//! a newer one. If `deps/NAME@VERSION/` already holds the locked commit and
//! the hash agrees, nothing is fetched, even when the tag has moved since.
//! Only when the commit must be fetched and the tag now names another commit
//! is the build refused (E0809); contents that differ from the hash are E0816.
//!
//! Every package of the build is cloned into the `deps/` folder of the top
//! project. Each package's own `drachm.toml` is read and its own dependencies
//! are resolved in turn; a cycle is E0810. Git runs as the `git` program on
//! the `PATH` (owner/engineering choice: no libgit2, so no C library to build
//! and the user's own git configuration, credentials and proxies apply).
//! Prompts are off, so a private repository fails rather than waiting for a
//! password.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use toml_edit::{ImDocument, Item};

use crate::config::{self, Config, Source, LOCK_FILE, PROJECT_FILE};
use crate::diag::{codes, Diagnostic, SourceMap, Span};
use crate::sha256::Sha256;
use crate::version::{newest_in, Range, Version};

/// The folder under the project's folder where git dependencies are cloned.
pub const DEPS_DIR: &str = "deps";

/// One dependency, found as a folder.
#[derive(Debug, Clone)]
pub struct Root {
    /// The name the project gives it: `sdl` in `in sdl...`.
    pub name: String,
    /// Canonical folder of the project declaring this alias.
    pub declared_in: PathBuf,
    /// The folder with its `drachm.toml`.
    pub dir: PathBuf,
    /// The source root: the folder of the dependency's entry file.
    pub src: PathBuf,
    /// The dependency's own config: it obeys its own `drachm.toml`.
    pub config: Config,
    /// The commit a git dependency was fetched at; `None` for a path.
    pub commit: Option<String>,
}

/// One package as the lock file records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locked {
    /// The last part of the address (`sdl-drh`): the folder is `NAME@VERSION`.
    pub name: String,
    /// The address, as the manifest wrote it.
    pub url: String,
    pub tag: String,
    pub commit: String,
    /// `sha256:` and the hex digest of the package's files (`hash_dir`).
    pub hash: String,
}

impl Locked {
    /// The folder under `deps/`.
    pub fn folder(&self) -> String {
        folder_name(&self.name, &self.tag)
    }
}

/// What a resolution may take from the lock file.
#[derive(Debug, Clone, Default)]
pub enum Refresh {
    /// Keep everything the lock records.
    #[default]
    Nothing,
    /// Ask the repositories again for every range and every tag.
    Everything,
    /// Ask again for the packages from these addresses.
    Sources(Vec<String>),
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub refresh: Refresh,
    /// `--locked`: install exactly what the lock file lists, resolve nothing
    /// new and write nothing (see `resolve_with`). `refresh` is not used then.
    pub locked: bool,
}

/// What a resolution found.
pub struct Resolved {
    pub roots: Vec<Root>,
    /// The packages of the build, as written to `drachm.lock`.
    pub locks: Vec<Locked>,
}

/// The last part of a git address: `https://github.com/a/sdl-drh.git` is
/// `sdl-drh`. Characters that do not belong in a folder name become `_`.
pub fn package_name(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let last = trimmed.rsplit(['/', ':', '\\']).next().unwrap_or("");
    let name: String = last.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    if name.is_empty() || name.starts_with('.') { "package".to_string() } else { name }
}

/// `sdl-drh@0.4.1`: the folder of a package under `deps/`.
pub fn folder_name(name: &str, tag: &str) -> String {
    let version = tag.strip_prefix('v').filter(|v| v.starts_with(|c: char| c.is_ascii_digit())).unwrap_or(tag);
    let version: String = version.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+') { c } else { '_' }).collect();
    format!("{name}@{version}")
}

/// Resolves every dependency of the project in `dir` (whose config is
/// `config`), fetching git ones into `dir/deps/` and writing `drachm.lock`.
/// Errors are the diagnostics; `map` gets every config file read, so they can
/// be shown.
pub fn resolve(dir: &Path, config: &Config, map: &mut SourceMap) -> Result<Vec<Root>, Vec<Diagnostic>> {
    resolve_with(dir, config, map, &Options::default()).map(|r| r.roots)
}

/// `resolve` with a say in what the lock file may decide.
///
/// With `options.locked` nothing is resolved anew: every git dependency must
/// have an entry in the lock file (E0818 otherwise), every locked package must
/// be needed by the manifests (E0818 otherwise), and the lock file is not
/// written. Locked packages are fetched as usual, so a hash that differs is
/// still E0816.
pub fn resolve_with(dir: &Path, config: &Config, map: &mut SourceMap, options: &Options) -> Result<Resolved, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let mut locked = read_lock(dir, map, &mut diags);
    if !diags.is_empty() {
        return Err(diags);
    }
    if options.locked {
        if !dir.join(LOCK_FILE).is_file() {
            if config.dependencies.is_empty() {
                return Ok(Resolved { roots: Vec::new(), locks: Vec::new() });
            }
            return Err(vec![Diagnostic::new(&codes::E0818, format!("{LOCK_FILE} is missing, but {PROJECT_FILE} has dependencies"))
                .note_line(format!("`--locked` builds exactly what {LOCK_FILE} lists, and this folder has none"))
                .help("run `drachm install` without `--locked` to resolve the dependencies and write the lock file, then commit it")]);
        }
    } else {
        match &options.refresh {
            Refresh::Nothing => {}
            Refresh::Everything => locked.clear(),
            Refresh::Sources(urls) => locked.retain(|l| !urls.contains(&l.url)),
        }
    }
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut r = Resolver {
        map,
        diags,
        deps_dir: dir.join(DEPS_DIR),
        locked,
        fetched: Vec::new(),
        stack: vec![canon],
        roots: Vec::new(),
        remote: Vec::new(),
        locked_only: options.locked,
        wanted: Vec::new(),
    };
    r.dependencies(dir, config);
    let Resolver { diags, fetched, roots, locked, wanted, .. } = r;
    let mut diags = diags;
    if options.locked {
        for l in locked.iter().filter(|l| !wanted.iter().any(|(u, t)| *u == l.url && *t == l.tag)) {
            diags.push(
                Diagnostic::new(&codes::E0818, format!("{LOCK_FILE} lists `{}` at `{}`, which {PROJECT_FILE} does not need", l.name, l.tag))
                    .note_line(format!("the source is `{}`", l.url))
                    .help("run `drachm install` without `--locked` to drop it from the lock file"),
            );
        }
    }
    let locks: Vec<Locked> = fetched.into_iter().map(|(l, _)| l).collect();
    if diags.is_empty() && !options.locked {
        if let Err(d) = write_lock(dir, &locks) {
            diags.push(d);
        }
    }
    if diags.is_empty() { Ok(Resolved { roots, locks }) } else { Err(diags) }
}

struct Resolver<'a> {
    map: &'a mut SourceMap,
    diags: Vec<Diagnostic>,
    /// `<project>/deps`.
    deps_dir: PathBuf,
    /// What the lock file says, as read (less what an update asked again for).
    locked: Vec<Locked>,
    /// The git packages of this build, with their folders.
    fetched: Vec<(Locked, PathBuf)>,
    /// The folders whose dependencies are being resolved, outermost first.
    stack: Vec<PathBuf>,
    roots: Vec<Root>,
    /// Tags asked of repositories during this resolution, by address.
    remote: Vec<(String, Vec<String>)>,
    /// `--locked`: only the lock file may name a tag; nothing is asked of a repository.
    locked_only: bool,
    /// The (address, tag) pairs the manifests reach, to find lock entries they do not need.
    wanted: Vec<(String, String)>,
}

impl Resolver<'_> {
    /// The dependencies a config declares, relative to `base`.
    fn dependencies(&mut self, base: &Path, config: &Config) {
        for dep in &config.dependencies {
            match &dep.source {
                Source::Path(p) => self.enter(base, &dep.name, &base.join(p), None),
                Source::Git { url, tag } => self.git(base, &dep.name, url, tag),
                Source::GitRange { url, range } => match self.pick_tag(&dep.name, url, range) {
                    Ok(tag) => self.git(base, &dep.name, url, &tag),
                    Err(d) => self.diags.push(d),
                },
            }
        }
    }

    /// The tag a range means: one this build already uses, else one the lock
    /// file has, else the newest in the repository.
    fn pick_tag(&mut self, alias: &str, url: &str, range_text: &str) -> Result<String, Diagnostic> {
        let Some(range) = Range::parse(range_text) else {
            return Err(Diagnostic::new(&codes::E0802, format!("`{range_text}` is not a version range")).help("a range starts with `^`, like `^0.4`"));
        };
        let known = |tags: Vec<&str>| newest_in(tags, &range).map(str::to_string);
        if let Some(t) = known(self.fetched.iter().filter(|(l, _)| l.url == url).map(|(l, _)| l.tag.as_str()).collect()) {
            return Ok(t);
        }
        if let Some(t) = known(self.locked.iter().filter(|l| l.url == url).map(|l| l.tag.as_str()).collect()) {
            return Ok(t);
        }
        if self.locked_only {
            let locked: Vec<&str> = self.locked.iter().filter(|l| l.url == url).map(|l| l.tag.as_str()).collect();
            let listed = if locked.is_empty() { format!("the lock file has no entry for `{url}`") } else { format!("the lock file has `{url}` at {}", locked.join(", ")) };
            return Err(Diagnostic::new(&codes::E0818, format!("no entry of drachm.lock fits `{range_text}` (the dependency `{alias}`)"))
                .note_line(listed)
                .help("run `drachm install` without `--locked` to resolve the range and write the lock file"));
        }
        let tags = self.remote_tags(url)?;
        known(tags.iter().map(String::as_str).collect()).ok_or_else(|| {
            let versions: Vec<&str> = tags.iter().map(String::as_str).filter(|t| Version::from_tag(t).is_some()).collect();
            Diagnostic::new(&codes::E0815, format!("no tag of `{url}` fits `{range_text}` (the dependency `{alias}`)"))
                .note_line(if versions.is_empty() { "the repository has no version tags".to_string() } else { format!("version tags there: {}", versions.join(", ")) })
                .help("change the range, or ask the owner of the package for a release")
        })
    }

    fn remote_tags(&mut self, url: &str) -> Result<Vec<String>, Diagnostic> {
        if let Some((_, tags)) = self.remote.iter().find(|(u, _)| u == url) {
            return Ok(tags.clone());
        }
        let tags = list_tags(url)?;
        self.remote.push((url.to_string(), tags.clone()));
        Ok(tags)
    }

    /// One git dependency at an exact tag.
    fn git(&mut self, base: &Path, alias: &str, url: &str, tag: &str) {
        self.wanted.push((url.to_string(), tag.to_string()));
        // Already fetched for another dependency of this build?
        if let Some((l, folder)) = self.fetched.iter().find(|(l, _)| l.url == url && l.tag == tag) {
            let (commit, folder) = (l.commit.clone(), folder.clone());
            self.enter(base, alias, &folder, Some(commit));
            return;
        }
        let name = package_name(url);
        let wanted = folder_name(&name, tag);
        if let Some((l, _)) = self.fetched.iter().find(|(l, _)| l.folder() == wanted) {
            self.diags.push(
                Diagnostic::new(&codes::E0804, format!("two different packages would be stored in `deps/{wanted}`"))
                    .note_line(format!("one is `{}` at `{}`, the other `{url}` at `{tag}`", l.url, l.tag))
                    .help("the folder is named after the last part of the address and the tag; fork one of them under another name"),
            );
            return;
        }
        let locked = self.locked.iter().find(|l| l.url == url && l.tag == tag).cloned();
        if self.locked_only && locked.is_none() {
            self.diags.push(
                Diagnostic::new(&codes::E0818, format!("{LOCK_FILE} has no entry for `{url}` at `{tag}` (the dependency `{alias}`)"))
                    .note_line(format!("{PROJECT_FILE} asks for it, and `--locked` does not resolve anything new"))
                    .help("run `drachm install` without `--locked` to resolve it and write the lock file, or `drachm update` for newer tags in a range"),
            );
            return;
        }
        match fetch(&self.deps_dir, alias, &name, url, tag, locked.as_ref()) {
            Ok((l, folder)) => {
                let commit = l.commit.clone();
                self.fetched.push((l, folder.clone()));
                self.enter(base, alias, &folder, Some(commit));
            }
            Err(d) => self.diags.push(d),
        }
    }

    /// Reads the dependency in `folder` and resolves its own dependencies.
    fn enter(&mut self, base: &Path, name: &str, folder: &Path, commit: Option<String>) {
        let canon = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
        if self.stack.contains(&canon) {
            self.diags.push(
                Diagnostic::new(&codes::E0810, format!("the dependency `{name}` is part of a cycle"))
                    .note_line(format!("`{}` is already being resolved", folder.display()))
                    .help("a project's dependencies must form a tree"),
            );
            return;
        }
        let declared_in = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
        if let Some(root) = self.roots.iter().find(|r| r.dir.canonicalize().ok().as_ref() == Some(&canon)) {
            let mut alias = root.clone();
            alias.name = name.to_owned();
            alias.declared_in = declared_in;
            self.roots.push(alias);
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
        let src = entry.parent().and_then(|p| p.canonicalize().ok()).unwrap_or_else(|| canon.clone());
        self.roots.push(Root { name: name.to_string(), declared_in, dir: folder.to_path_buf(), src, config: config.clone(), commit });
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

fn git_missing(e: std::io::Error) -> String {
    format!("cannot run `git`: {e}; install git to use git dependencies")
}

/// The tags of the repository at `url`.
pub fn list_tags(url: &str) -> Result<Vec<String>, Diagnostic> {
    let fail = |why: &str| Diagnostic::new(&codes::E0808, format!("cannot list the tags of `{url}`")).note_line(why.to_string()).help("check the address; it must be one `git clone` can use");
    let out = git(&["ls-remote", "--tags", "--refs", "--", url]).output().map_err(|e| fail(&git_missing(e)))?;
    if !out.status.success() {
        return Err(fail(&first_line(&String::from_utf8_lossy(&out.stderr), "git failed")));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().filter_map(|l| l.split_once("refs/tags/").map(|(_, t)| t.trim().to_string())).filter(|t| !t.is_empty()).collect())
}

/// The commit `dir` (a clone) is at, if git can say.
fn head_of(dir: &Path) -> Option<String> {
    let out = git(&["-C"]).arg(dir).args(["rev-parse", "HEAD"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The sha256 hash of a package's files, as written in `drachm.lock`:
/// `sha256:` and 64 hex digits. The digest runs over every file under `dir`
/// except anything named `.git`, in the order of their `/`-separated relative
/// paths (compared as bytes). For each file it takes the relative path, a zero
/// byte, `f` (or `l` for a symbolic link, whose "contents" are its target), the
/// length of the contents in decimal, a zero byte, and the contents. Folders
/// (empty ones too), permissions and times do not count.
pub fn hash_dir(dir: &Path) -> std::io::Result<String> {
    fn walk(base: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(base)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(&entry.path(), &rel, out)?;
            } else {
                out.push((rel, entry.path()));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut sha = Sha256::default();
    for (rel, path) in files {
        let (kind, bytes) = if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            (b'l', std::fs::read_link(&path)?.to_string_lossy().into_owned().into_bytes())
        } else {
            (b'f', std::fs::read(&path)?)
        };
        sha.update(rel.as_bytes());
        sha.update(&[0, kind]);
        sha.update(bytes.len().to_string().as_bytes());
        sha.update(&[0]);
        sha.update(&bytes);
    }
    Ok(format!("sha256:{}", sha.finish()))
}

fn hash_error(dir: &Path, locked: &str, found: &str, why: &str) -> Diagnostic {
    Diagnostic::new(&codes::E0816, format!("`{}` is not what drachm.lock recorded", dir.display()))
        .note_line(format!("{why}: the lock has {locked}, the files hash to {found}"))
        .help("`drachm install` fetches `deps/` again from the lock; do not edit packages in `deps/`")
}

/// Makes `deps/NAME@VERSION` hold the commit of `url` at `tag`, and returns
/// its lock entry and the folder. With a locked entry, a folder already at
/// that commit is used as it is (its files must hash as recorded, else
/// E0816); otherwise the tag is cloned, and it must name the locked commit
/// (E0809) and give the locked hash (E0816).
fn fetch(deps_dir: &Path, alias: &str, name: &str, url: &str, tag: &str, locked: Option<&Locked>) -> Result<(Locked, PathBuf), Diagnostic> {
    let dest = deps_dir.join(folder_name(name, tag));
    let entry = |commit: String, hash: String| Locked { name: name.to_string(), url: url.to_string(), tag: tag.to_string(), commit, hash };
    if let Some(l) = locked {
        if dest.is_dir() && head_of(&dest).as_deref() == Some(l.commit.as_str()) {
            let found = hash_dir(&dest).map_err(|e| fetch_error(alias, url, tag, &format!("cannot read `{}`: {e}", dest.display())))?;
            if found != l.hash {
                return Err(hash_error(&dest, &l.hash, &found, "the files in deps/ changed"));
            }
            return Ok((entry(l.commit.clone(), l.hash.clone()), dest));
        }
    }
    std::fs::create_dir_all(deps_dir).map_err(|e| fetch_error(alias, url, tag, &format!("cannot make `{}`: {e}", deps_dir.display())))?;
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let tmp = deps_dir.join(format!(".{name}.tmp-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&tmp);
    match git(&["clone", "--quiet", "--depth", "1", "--branch", tag, "--", url]).arg(&tmp).output() {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let _ = std::fs::remove_dir_all(&tmp);
            let said = String::from_utf8_lossy(&o.stderr);
            if said.contains("not found in upstream") {
                return Err(Diagnostic::new(&codes::E0815, format!("`{url}` has no tag `{tag}` (the dependency `{alias}`)"))
                    .note_line(first_line(&said, "git failed"))
                    .help("`git ls-remote --tags URL` lists the tags; `drachm add URL` takes the newest"));
            }
            return Err(fetch_error(alias, url, tag, &first_line(&said, "git failed")));
        }
        Err(e) => return Err(fetch_error(alias, url, tag, &git_missing(e))),
    }
    let Some(commit) = head_of(&tmp) else {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(fetch_error(alias, url, tag, "the clone has no commit to read"));
    };
    if let Some(old) = locked.map(|l| &l.commit) {
        if *old != commit {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(Diagnostic::new(&codes::E0809, format!("the tag `{tag}` of `{url}` now points at another commit"))
                .note_line(format!("`drachm.lock` records {old}; the tag is at {commit}"))
                .help(format!("if the new commit is wanted, run `drachm update {alias}`")));
        }
    }
    let hash = hash_dir(&tmp).map_err(|e| fetch_error(alias, url, tag, &format!("cannot read the clone: {e}")))?;
    if let Some(l) = locked {
        if l.hash != hash {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(hash_error(&dest, &l.hash, &hash, "the fetched files differ"));
        }
    }
    let _ = std::fs::remove_dir_all(&dest);
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(fetch_error(alias, url, tag, &format!("cannot move the clone into `{}`: {e}", dest.display())));
    }
    Ok((entry(commit, hash), dest))
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
    if let Some(Item::ArrayOfTables(list)) = doc.as_table().get("package") {
        for t in list.iter() {
            let field = |k: &str| t.get(k).and_then(Item::as_str).map(str::to_string);
            match (field("name"), field("source"), field("tag"), field("commit"), field("hash")) {
                (Some(name), Some(url), Some(tag), Some(commit), Some(hash)) => out.push(Locked { name, url, tag, commit, hash }),
                _ => diags.push(
                    Diagnostic::new(&codes::E0800, format!("an entry of {LOCK_FILE} lacks a `name`, `source`, `tag`, `commit` or `hash`"))
                        .note_line("delete the file and run `drachm install` to make a new one"),
                ),
            }
        }
    }
    out
}

/// The packages `drachm.lock` in `dir` records (none when there is no file).
pub fn read_lock_file(dir: &Path) -> Result<Vec<Locked>, Vec<Diagnostic>> {
    let mut map = SourceMap::new();
    let mut diags = Vec::new();
    let locks = read_lock(dir, &mut map, &mut diags);
    if diags.is_empty() { Ok(locks) } else { Err(diags) }
}

/// The text of a lock file for `locks`: sorted, one entry per address and tag.
fn lock_text(locks: &[Locked]) -> String {
    let mut sorted: Vec<&Locked> = locks.iter().collect();
    sorted.sort_by(|a, b| (&a.name, &a.url, &a.tag).cmp(&(&b.name, &b.url, &b.tag)));
    sorted.dedup_by(|a, b| a.url == b.url && a.tag == b.tag);
    let mut text = String::from("# written by drachm, do not edit\n");
    for l in sorted {
        text += &format!(
            "\n[[package]]\nname = {}\nsource = {}\ntag = {}\ncommit = {}\nhash = {}\n",
            quote(&l.name),
            quote(&l.url),
            quote(&l.tag),
            quote(&l.commit),
            quote(&l.hash)
        );
    }
    text
}

/// Writes the lock file for `locks`, or removes it when there are none.
/// Nothing is written when the file already says the same.
fn write_lock(dir: &Path, locks: &[Locked]) -> Result<(), Diagnostic> {
    let path = dir.join(LOCK_FILE);
    let existing = std::fs::read_to_string(&path).ok();
    if locks.is_empty() {
        if existing.is_some() {
            std::fs::remove_file(&path).map_err(|e| lock_error(&path, &e.to_string()))?;
        }
        return Ok(());
    }
    let text = lock_text(locks);
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

    #[test]
    fn package_and_folder_names() {
        assert_eq!(package_name("https://github.com/someone/sdl-drh.git"), "sdl-drh");
        assert_eq!(package_name("https://github.com/someone/sdl-drh/"), "sdl-drh");
        assert_eq!(package_name("git@github.com:someone/json-drh.git"), "json-drh");
        assert_eq!(package_name("file:///tmp/x/lib"), "lib");
        assert_eq!(folder_name("json-drh", "v1.4.0"), "json-drh@1.4.0");
        assert_eq!(folder_name("json-drh", "2.1.0"), "json-drh@2.1.0");
        assert_eq!(folder_name("x", "vnext"), "x@vnext");
        assert_eq!(folder_name("x", "rel/1"), "x@rel_1");
    }
}
