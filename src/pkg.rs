//! The package manager commands (step 7.6): `drachm add`, `remove`, `update`
//! and `install`. They edit the project's `drachm.toml` (with `toml_edit`, so
//! the owner's formatting and comments stay), then run the same resolution a
//! build runs (`deps::resolve_with`), which fetches into `deps/` and writes
//! `drachm.lock`. When a command fails, `drachm.toml` is put back as it was.
//!
//! Each command returns the lines to print; the CLI prints them.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

use crate::config::{self, Failure, Source, PROJECT_FILE};
use crate::deps::{self, Locked, Options, Refresh, Resolved, DEPS_DIR};
use crate::diag::{codes, Diagnostic, SourceMap};
use crate::version::{self, Range};

fn failure(d: Diagnostic) -> Failure {
    Failure { map: SourceMap::new(), diags: vec![d] }
}

fn cannot(why: impl Into<String>) -> Failure {
    failure(Diagnostic::new(&codes::E0817, why))
}

/// What `drachm add` was given: `ADDRESS`, `ADDRESS@TAG` or `ADDRESS@^RANGE`.
#[derive(Debug, PartialEq, Eq)]
pub struct Spec {
    pub url: String,
    /// `None` (newest version tag), `Some("v0.4")` or `Some("^0.4")`.
    pub version: Option<String>,
}

impl Spec {
    pub fn parse(text: &str) -> Result<Spec, Failure> {
        // The last `@` starts a version unless a `/` or `:` follows it
        // (`git@host:path`, `https://user@host/path`).
        let (raw, version) = match text.rfind('@') {
            Some(i) if i > 0 && !text[i + 1..].contains(['/', ':']) => (&text[..i], Some(text[i + 1..].to_string())),
            _ => (text, None),
        };
        if raw.is_empty() || version.as_deref() == Some("") {
            return Err(cannot(format!("`{text}` is not an address with an optional `@version`")).into_failure_help("write `github.com/someone/package@v1.2` or `...@^1.2`"));
        }
        if !raw.contains(['/', ':', '.']) {
            return Err(failure(
                Diagnostic::new(&codes::E0901, format!("`{raw}` looks like the name of a package in the Agora, which does not exist yet"))
                    .help("give a git address: `drachm add github.com/someone/package@^1.0`"),
            ));
        }
        Ok(Spec { url: normalise_url(raw), version })
    }
}

trait WithHelp {
    fn into_failure_help(self, help: &str) -> Failure;
}

impl WithHelp for Failure {
    fn into_failure_help(mut self, help: &str) -> Failure {
        if let Some(d) = self.diags.pop() {
            self.diags.push(d.help(help));
        }
        self
    }
}

/// `github.com/a/b` is `https://github.com/a/b`; addresses git already knows
/// (`https://`, `file://`, `git@host:path`, a folder path) stay as written.
pub fn normalise_url(raw: &str) -> String {
    let local = raw.starts_with(['/', '.', '~']);
    if raw.contains("://") || raw.starts_with("git@") || local {
        if local && Path::new(raw).exists() {
            if let Ok(p) = Path::new(raw).canonicalize() {
                return p.display().to_string();
            }
        }
        return raw.to_string();
    }
    format!("https://{raw}")
}

/// The dependency name for an address: the package name with `-` and `.` as `_`.
fn alias_for(url: &str) -> String {
    deps::package_name(url).chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// Reads `dir/drachm.toml` as text.
fn manifest(dir: &Path) -> Result<(PathBuf, String), Failure> {
    let path = dir.join(PROJECT_FILE);
    match std::fs::read_to_string(&path) {
        Ok(t) => Ok((path, t)),
        Err(_) => Err(failure(
            Diagnostic::new(&codes::E0817, format!("`{}` has no {PROJECT_FILE}", dir.display())).help("run this command in a project folder; `drachm new NAME` makes one"),
        )),
    }
}

fn write_manifest(path: &Path, text: &str) -> Result<(), Failure> {
    std::fs::write(path, text).map_err(|e| cannot(format!("cannot write `{}`: {e}", path.display())))
}

fn edit(text: &str) -> Result<DocumentMut, Failure> {
    text.parse::<DocumentMut>().map_err(|e| failure(Diagnostic::new(&codes::E0800, format!("{PROJECT_FILE} is not valid TOML")).note_line(e.message().to_string())))
}

/// Reads the project's config and resolves its dependencies.
fn resolve(dir: &Path, options: &Options) -> Result<Resolved, Failure> {
    let mut map = SourceMap::new();
    let config = match config::load(&mut map, &dir.join(PROJECT_FILE), PROJECT_FILE, false) {
        Ok(c) => c,
        Err(diags) => return Err(Failure { map, diags }),
    };
    deps::resolve_with(dir, &config, &mut map, options).map_err(|diags| Failure { map, diags })
}

/// Removes what `deps/` holds beyond the packages of `locks`.
fn prune(dir: &Path, locks: &[Locked]) {
    let keep: Vec<String> = locks.iter().map(Locked::folder).collect();
    let Ok(entries) = std::fs::read_dir(dir.join(DEPS_DIR)) else { return };
    for e in entries.flatten() {
        if !keep.contains(&e.file_name().to_string_lossy().into_owned()) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(8)]
}

/// `drachm add SPEC [--name NAME]`.
pub fn add(dir: &Path, spec: &str, name: Option<&str>) -> Result<Vec<String>, Failure> {
    let spec = Spec::parse(spec)?;
    let alias = name.map(str::to_string).unwrap_or_else(|| alias_for(&spec.url));
    if !config::valid_name(&alias) {
        return Err(cannot(format!("`{alias}` is not a valid dependency name")).into_failure_help("a dependency is named like a module: letters, digits and `_`; choose one with `--name NAME`"));
    }
    let (path, old_text) = manifest(dir)?;
    // Which tag, or which range, goes into drachm.toml.
    let (key, value) = match spec.version.as_deref() {
        Some(v) if v.starts_with('^') => {
            if Range::parse(v).is_none() {
                return Err(cannot(format!("`{v}` is not a version range")).into_failure_help("a range is `^0.4` or `^1.2.3`; `@v0.4` names one exact tag"));
            }
            ("version", v.to_string())
        }
        Some(tag) => ("tag", tag.to_string()),
        None => {
            let tags = deps::list_tags(&spec.url).map_err(failure)?;
            match version::newest(tags.iter().map(String::as_str)) {
                Some(t) => ("tag", t.to_string()),
                None => {
                    return Err(failure(
                        Diagnostic::new(&codes::E0815, format!("`{}` has no version tags", spec.url))
                            .note_line("the newest tag that reads as a version (`v1.2.3`) is taken, and there is none")
                            .help("give a tag: `drachm add ADDRESS@TAG`"),
                    ))
                }
            }
        }
    };
    let mut doc = edit(&old_text)?;
    let deps_table = doc.entry("dependencies").or_insert(Item::Table(Table::new())).as_table_mut().ok_or_else(|| cannot("`dependencies` in drachm.toml is not a table"))?;
    let existed = deps_table.contains_key(&alias);
    let mut entry = InlineTable::new();
    entry.insert("git", Value::from(spec.url.clone()));
    entry.insert(key, Value::from(value.clone()));
    entry.fmt();
    deps_table.insert(&alias, Item::Value(Value::InlineTable(entry)));
    write_manifest(&path, &doc.to_string())?;
    let resolved = match resolve(dir, &Options::default()) {
        Ok(r) => r,
        Err(f) => {
            let _ = write_manifest(&path, &old_text);
            return Err(f);
        }
    };
    prune(dir, &resolved.locks);
    let shown = if key == "version" {
        let range = Range::parse(&value).expect("checked above");
        let tags = resolved.locks.iter().filter(|l| l.url == spec.url && range.allows_tag(&l.tag)).map(|l| l.tag.as_str());
        version::newest_in(tags, &range).unwrap_or(&value).to_string()
    } else {
        value.clone()
    };
    let verb = if existed { "changed" } else { "added" };
    Ok(vec![format!("{verb} {alias}: {} at {shown}", spec.url)])
}

/// The key of `[dependencies]` that `name` means (`sdl-drh` finds `sdl_drh`).
fn find_key(table: &Table, name: &str) -> Option<String> {
    let flat = |s: &str| s.replace(['-', '.'], "_");
    table.iter().map(|(k, _)| k.to_string()).find(|k| k == name || flat(k) == flat(name))
}

/// `drachm remove NAME`.
pub fn remove(dir: &Path, name: &str) -> Result<Vec<String>, Failure> {
    let (path, old_text) = manifest(dir)?;
    let mut doc = edit(&old_text)?;
    let table = doc.get_mut("dependencies").and_then(Item::as_table_mut);
    let key = table.as_deref().and_then(|t| find_key(t, name));
    let (Some(table), Some(key)) = (table, key) else { return Err(not_a_dependency(&old_text, name)) };
    table.remove(&key);
    write_manifest(&path, &doc.to_string())?;
    let resolved = match resolve(dir, &Options::default()) {
        Ok(r) => r,
        Err(f) => {
            let _ = write_manifest(&path, &old_text);
            return Err(f);
        }
    };
    prune(dir, &resolved.locks);
    Ok(vec![format!("removed {key}")])
}

fn not_a_dependency(text: &str, name: &str) -> Failure {
    let names: Vec<String> = config::parse(text, 0, false).map(|c| c.dependencies.into_iter().map(|d| d.name).collect()).unwrap_or_default();
    let d = Diagnostic::new(&codes::E0817, format!("`{name}` is not in [dependencies]"));
    failure(if names.is_empty() { d.note_line("the project has no dependencies") } else { d.note_line(format!("the dependencies are: {}", names.join(", "))) })
}

/// `drachm update [NAME]`: asks the repositories for newer tags in the
/// ranges, and for the commit a moved exact tag names now.
pub fn update(dir: &Path, name: Option<&str>) -> Result<Vec<String>, Failure> {
    let (_, text) = manifest(dir)?;
    let before = deps::read_lock_file(dir).map_err(|diags| Failure { map: SourceMap::new(), diags })?;
    let refresh = match name {
        None => Refresh::Everything,
        Some(n) => {
            let config = config::parse(&text, 0, false).map_err(|diags| Failure { map: SourceMap::new(), diags })?;
            let flat = |s: &str| s.replace(['-', '.'], "_");
            let Some(dep) = config.dependencies.iter().find(|d| d.name == n || flat(&d.name) == flat(n)) else { return Err(not_a_dependency(&text, n)) };
            match dep.source.url() {
                Some(url) => Refresh::Sources(vec![url.to_string()]),
                None => {
                    let Source::Path(_) = dep.source else { unreachable!() };
                    return Err(cannot(format!("`{}` is a path dependency, which has no version to update", dep.name)));
                }
            }
        }
    };
    let resolved = resolve(dir, &Options { refresh })?;
    prune(dir, &resolved.locks);
    let mut lines = Vec::new();
    for new in &resolved.locks {
        let same_source = |l: &&Locked| l.url == new.url;
        if before.iter().any(|o| o.url == new.url && o.tag == new.tag) {
            let old = before.iter().find(|o| o.url == new.url && o.tag == new.tag).unwrap();
            if old.commit != new.commit {
                lines.push(format!("updated {} {}: the tag moved, {} -> {}", new.name, new.tag, short(&old.commit), short(&new.commit)));
            }
            continue;
        }
        let gone: Vec<&str> = before.iter().filter(same_source).filter(|o| !resolved.locks.iter().any(|n| n.url == o.url && n.tag == o.tag)).map(|o| o.tag.as_str()).collect();
        if gone.is_empty() {
            lines.push(format!("added {} {}", new.name, new.tag));
        } else {
            lines.push(format!("updated {}: {} -> {}", new.name, gone.join(", "), new.tag));
        }
    }
    for old in &before {
        if !resolved.locks.iter().any(|n| n.url == old.url) {
            lines.push(format!("removed {} {}", old.name, old.tag));
        }
    }
    if lines.is_empty() {
        lines.push("everything is up to date".to_string());
    }
    Ok(lines)
}

/// `drachm install`: throws `deps/` away and fetches every package again, at
/// the commit `drachm.lock` records.
pub fn install(dir: &Path) -> Result<Vec<String>, Failure> {
    manifest(dir)?;
    let deps_dir = dir.join(DEPS_DIR);
    if deps_dir.exists() {
        std::fs::remove_dir_all(&deps_dir).map_err(|e| cannot(format!("cannot remove `{}`: {e}", deps_dir.display())))?;
    }
    let resolved = resolve(dir, &Options::default())?;
    let n = resolved.locks.len();
    Ok(vec![format!("installed {n} package{}", if n == 1 { "" } else { "s" })])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs() {
        let s = |t: &str| Spec::parse(t).ok().map(|s| (s.url, s.version));
        assert_eq!(s("github.com/a/b@v0.4"), Some(("https://github.com/a/b".into(), Some("v0.4".into()))));
        assert_eq!(s("github.com/a/b@^0.4"), Some(("https://github.com/a/b".into(), Some("^0.4".into()))));
        assert_eq!(s("github.com/a/b"), Some(("https://github.com/a/b".into(), None)));
        assert_eq!(s("git@github.com:a/b.git"), Some(("git@github.com:a/b.git".into(), None)));
        assert_eq!(s("git@github.com:a/b.git@v1"), Some(("git@github.com:a/b.git".into(), Some("v1".into()))));
        assert_eq!(s("file:///tmp/x/lib@v1.0"), Some(("file:///tmp/x/lib".into(), Some("v1.0".into()))));
        assert_eq!(s("https://user@host/a/b"), Some(("https://user@host/a/b".into(), None)));
        assert_eq!(s("a/b@"), None);
    }
}
