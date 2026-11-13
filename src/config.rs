//! `drachm.toml` and `srachm.toml`: what a project builds, what it depends on
//! and how its code behaves. Owner decisions (step 7 batch C):
//!
//! - `[project]` has `name` (required in a project), `kind = "program"` (the
//!   default) or `"library"`, and `entry = "src/main.drh"`. The source root is
//!   the folder of the entry file, so `in NAME.x.y;` starts there.
//! - `[dependencies]` takes `name = { path = "../utils" }` or
//!   `name = { git = "https://...", tag = "v0.4" }`. A registry version
//!   (`json = "1.2"`) is E0901 until the Agora exists. Fetching is in `deps.rs`.
//! - `[behaviour]` has six settings. The file flags `//--!$overflow`,
//!   `//--!$bounds` and `//--!$unsigned_negate` may set them for one file;
//!   the other three are project-wide and a file flag setting them is E0806.
//!
//! A project folder has a `drachm.toml`. A script may sit beside a
//! `srachm.toml` (same settings, script defaults); with neither, a script
//! has the script defaults. This module reads and checks the settings. Code
//! generation of `[behaviour]` is WP4: until it lands the values are stored,
//! not applied.

use std::path::Path;

use toml_edit::{ImDocument, Item, Table, Value};

use crate::diag::{codes, Diagnostic, SourceMap, Span};

pub const PROJECT_FILE: &str = "drachm.toml";
pub const SCRIPT_FILE: &str = "srachm.toml";
pub const LOCK_FILE: &str = "drachm.lock";
pub const DEFAULT_ENTRY: &str = "src/main.drh";

/// The build targets a `[build] target` may name (`os-cpu`).
pub const TARGETS: &[&str] = &["linux-x64", "macos-arm64", "windows-x64"];

/// What a project or script says about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub project: Project,
    pub dependencies: Vec<Dependency>,
    pub build: Build,
    pub warnings: Warnings,
    pub behaviour: Behaviour,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Program,
    Library,
}

impl Kind {
    pub const WORDS: &'static [(&'static str, Kind)] = &[("program", Kind::Program), ("library", Kind::Library)];
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Required in a `drachm.toml`; a script may have none.
    pub name: Option<String>,
    pub version: Option<String>,
    pub kind: Kind,
    /// The entry file, relative to the project folder.
    pub entry: String,
}

impl Default for Project {
    fn default() -> Self {
        Project { name: None, version: None, kind: Kind::Program, entry: DEFAULT_ENTRY.to_string() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dependency {
    pub name: String,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A folder with its own `drachm.toml`, relative to the declaring project.
    Path(String),
    /// A git repository, cloned at `tag`.
    Git { url: String, tag: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Build {
    /// `None` means the machine the compiler runs on.
    pub target: Option<String>,
    /// `opt = true` makes `build` and `run` behave as if `-O` was given.
    pub opt: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Warnings {
    /// Warning names that become errors. The names are not defined yet, so
    /// they are stored and not applied (see the open questions in PLAN).
    pub as_errors: Vec<String>,
}

macro_rules! choice {
    ($(#[$m:meta])* $ty:ident, $key:literal, $default:ident { $($word:literal => $var:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $ty {
            $($var),+
        }

        impl $ty {
            /// The setting's name in `[behaviour]` (and in a file flag).
            pub const KEY: &'static str = $key;
            /// Each word the setting accepts, with its meaning.
            pub const WORDS: &'static [(&'static str, $ty)] = &[$(($word, $ty::$var)),+];

            pub fn parse(word: &str) -> Option<$ty> {
                Self::WORDS.iter().find(|(w, _)| *w == word).map(|(_, v)| *v)
            }

            pub fn word(self) -> &'static str {
                Self::WORDS.iter().find(|(_, v)| *v == self).map_or("", |(w, _)| w)
            }
        }

        impl Default for $ty {
            fn default() -> Self {
                $ty::$default
            }
        }
    };
}

choice! {
    /// What arithmetic overflow does: the amount wraps, saturates at the
    /// limits, or stops the program.
    Overflow, "overflow", Wrap { "wrap" => Wrap, "saturate" => Saturate, "panic" => Panic }
}
choice! {
    /// Index checks: `panic` checks every index, `unchecked` drops the checks.
    Bounds, "bounds", Panic { "panic" => Panic, "unchecked" => Unchecked }
}
choice! {
    /// What `m[k]` does when the key is missing: an optional, or a stop.
    MissingKey, "missing_key", Optional { "optional" => Optional, "panic" => Panic }
}
choice! {
    /// A call to a failing function with no handler: a compile error, or a
    /// program that stops with the failure's message.
    UnhandledFail, "unhandled_fail", Error { "error" => Error, "crash" => Crash }
}
choice! {
    /// A `in *` import whose names clash with another name: a warning or an error.
    ImportClash, "import_clash", Warn { "warn" => Warn, "error" => Error }
}
choice! {
    /// `-x` on an unsigned value: wraps, is an error, or becomes the next signed type.
    UnsignedNegate, "unsigned_negate", Wrap { "wrap" => Wrap, "error" => Error, "signed" => Signed }
}

/// The `[behaviour]` settings of a project.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Behaviour {
    pub overflow: Overflow,
    pub bounds: Bounds,
    pub missing_key: MissingKey,
    pub unhandled_fail: UnhandledFail,
    pub import_clash: ImportClash,
    pub unsigned_negate: UnsignedNegate,
}

/// The settings a file flag may set for one file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileBehaviour {
    pub overflow: Option<Overflow>,
    pub bounds: Option<Bounds>,
    pub unsigned_negate: Option<UnsignedNegate>,
}

/// The file flag keys that may set a behaviour for one file.
pub const FILE_KEYS: &[&str] = &[Overflow::KEY, Bounds::KEY, UnsignedNegate::KEY];
/// The behaviour keys that are project-wide: a file flag setting one is E0806.
pub const PROJECT_KEYS: &[&str] = &[MissingKey::KEY, UnhandledFail::KEY, ImportClash::KEY];

impl Behaviour {
    /// This project's behaviour as one file sees it: the file's own flags win.
    pub fn for_file(&self, flags: &FileBehaviour) -> Behaviour {
        Behaviour {
            overflow: flags.overflow.unwrap_or(self.overflow),
            bounds: flags.bounds.unwrap_or(self.bounds),
            unsigned_negate: flags.unsigned_negate.unwrap_or(self.unsigned_negate),
            ..*self
        }
    }
}

impl Config {
    /// The settings of a script with no `srachm.toml`: a script crashes on a
    /// failure nobody handles.
    pub fn script() -> Config {
        let mut c = Config::default();
        c.behaviour.unhandled_fail = UnhandledFail::Crash;
        c
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            project: Project::default(),
            dependencies: Vec::new(),
            build: Build::default(),
            warnings: Warnings::default(),
            behaviour: Behaviour::default(),
        }
    }
}

/// The target the compiler itself builds for, if it is one of `TARGETS`.
pub fn host_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x64"),
        ("macos", "aarch64") => Some("macos-arm64"),
        ("windows", "x86_64") => Some("windows-x64"),
        _ => None,
    }
}

/// Parses `text` (a `drachm.toml` when `script` is false, a `srachm.toml`
/// when it is true). `base` is the text's offset in its `SourceMap`, so the
/// spans of the diagnostics point into the right file.
pub fn parse(text: &str, base: usize, script: bool) -> Result<Config, Vec<Diagnostic>> {
    let doc = match ImDocument::parse(text) {
        Ok(d) => d,
        Err(e) => {
            let at = e.span().map_or(0..0, |r| r);
            let mut d = Diagnostic::at(&codes::E0800, Span::new(base + at.start, base + at.end), "this is not valid TOML");
            d = d.note_line(e.message().lines().next().unwrap_or("").to_string());
            return Err(vec![d]);
        }
    };
    let mut p = Reader { base, script, file: if script { SCRIPT_FILE } else { PROJECT_FILE }, diags: Vec::new() };
    let config = p.config(doc.as_table());
    if p.diags.is_empty() { Ok(config) } else { Err(p.diags) }
}

/// Reads one config file into `map` (so its diagnostics can be shown) and
/// parses it. Unreadable files are E0800.
pub fn load(map: &mut SourceMap, path: &Path, display: &str, script: bool) -> Result<Config, Vec<Diagnostic>> {
    let text = std::fs::read_to_string(path).map_err(|e| vec![Diagnostic::new(&codes::E0800, format!("cannot read `{display}`: {e}"))])?;
    let (_, base) = map.add(display, text.clone());
    parse(&text, base, script)
}

struct Reader {
    base: usize,
    script: bool,
    file: &'static str,
    diags: Vec<Diagnostic>,
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `"a", "b" or "c"` for the words of a setting.
fn one_of<T>(words: &[(&str, T)]) -> String {
    let names: Vec<&str> = words.iter().map(|(w, _)| *w).collect();
    match names.split_last() {
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

impl Reader {
    fn span(&self, r: Option<std::ops::Range<usize>>) -> Span {
        match r {
            Some(r) => Span::new(self.base + r.start, self.base + r.end),
            None => Span::new(self.base, self.base),
        }
    }

    fn key_span(&self, t: &Table, key: &str) -> Span {
        self.span(t.key(key).and_then(|k| k.span()))
    }

    fn item_span(&self, item: &Item) -> Span {
        self.span(item.span())
    }

    /// Every key of `t` must be one of `known`; others are E0801.
    fn check_keys(&mut self, t: &Table, known: &[&str], section: &str) {
        for (k, _) in t.iter() {
            if known.contains(&k) {
                continue;
            }
            let d = Diagnostic::at(&codes::E0801, self.key_span(t, k), format!("unknown setting `{k}` in {section}")).note("no such setting");
            let d = match crate::config::similar(k, known) {
                Some(s) => d.help(format!("did you mean `{s}`?")),
                None => d.help(format!("the settings in {section} are {}", known.join(", "))),
            };
            self.diags.push(d);
        }
    }

    /// A string setting: `Some((text, span))`, or E0802 for another type.
    fn string(&mut self, item: &Item, what: &str) -> Option<(String, Span)> {
        match item.as_str() {
            Some(s) => Some((s.to_string(), self.item_span(item))),
            None => {
                self.diags.push(Diagnostic::at(&codes::E0802, self.item_span(item), format!("`{what}` must be text in quotes")));
                None
            }
        }
    }

    fn section<'a>(&mut self, root: &'a Table, name: &str) -> Option<&'a Table> {
        let item = root.get(name)?;
        match item.as_table() {
            Some(t) => Some(t),
            None => {
                self.diags.push(
                    Diagnostic::at(&codes::E0802, self.item_span(item), format!("`{name}` must be a table"))
                        .help(format!("write it as a section: `[{name}]`")),
                );
                None
            }
        }
    }

    fn config(&mut self, root: &Table) -> Config {
        self.check_keys(root, &["project", "dependencies", "build", "warnings", "behaviour"], self.file);
        let mut c = if self.script { Config::script() } else { Config::default() };
        if let Some(t) = self.section(root, "project") {
            c.project = self.project(t);
        } else if !self.script {
            self.missing_name(root);
        }
        if let Some(t) = self.section(root, "dependencies") {
            c.dependencies = self.dependencies(t);
        }
        if let Some(t) = self.section(root, "build") {
            c.build = self.build(t);
        }
        if let Some(t) = self.section(root, "warnings") {
            c.warnings = self.warnings(t);
        }
        if let Some(t) = self.section(root, "behaviour") {
            c.behaviour = self.behaviour(t, c.behaviour);
        }
        c
    }

    fn missing_name(&mut self, root: &Table) {
        let at = self.span(root.span());
        self.diags.push(
            Diagnostic::at(&codes::E0803, at, "`[project] name` is missing")
                .note("a project needs a name")
                .help("add a `[project]` section with `name = \"mygame\"`"),
        );
    }

    fn project(&mut self, t: &Table) -> Project {
        self.check_keys(t, &["name", "version", "kind", "entry"], "[project]");
        let mut p = Project::default();
        match t.get("name") {
            Some(item) => {
                if let Some((name, at)) = self.string(item, "name") {
                    if is_identifier(&name) {
                        p.name = Some(name);
                    } else {
                        self.diags.push(
                            Diagnostic::at(&codes::E0805, at, format!("`{name}` is not a valid project name"))
                                .note("a name is letters, digits and `_`, starting with a letter or `_`")
                                .help("the name starts paths like `in mygame.game.physics;`"),
                        );
                    }
                }
            }
            None if !self.script => {
                let at = self.span(t.span());
                self.diags.push(
                    Diagnostic::at(&codes::E0803, at, "`[project] name` is missing")
                        .note("a project needs a name")
                        .help("add `name = \"mygame\"` to `[project]`"),
                );
            }
            None => {}
        }
        if let Some(item) = t.get("version") {
            p.version = self.string(item, "version").map(|(v, _)| v);
        }
        if let Some(item) = t.get("kind") {
            if let Some((word, at)) = self.string(item, "kind") {
                match Kind::WORDS.iter().find(|(w, _)| *w == word) {
                    Some((_, k)) => p.kind = *k,
                    None => self.diags.push(
                        Diagnostic::at(&codes::E0802, at, format!("`kind` must be {}, found `{word}`", one_of(Kind::WORDS)))
                            .help("a program has `main()`; a library has no program to run"),
                    ),
                }
            }
        }
        if let Some(item) = t.get("entry") {
            if let Some((entry, at)) = self.string(item, "entry") {
                let relative = !entry.starts_with('/') && !entry.contains(':') && !entry.split(['/', '\\']).any(|s| s == "..");
                if relative && entry.ends_with(".drh") {
                    p.entry = entry;
                } else {
                    self.diags.push(
                        Diagnostic::at(&codes::E0802, at, format!("`entry` must be a `.drh` file inside the project, found `{entry}`"))
                            .help("for example `entry = \"src/main.drh\"`"),
                    );
                }
            }
        }
        p
    }

    fn dependencies(&mut self, t: &Table) -> Vec<Dependency> {
        let mut out = Vec::new();
        for (name, item) in t.iter() {
            let at = self.key_span(t, name);
            if !is_identifier(name) {
                self.diags.push(
                    Diagnostic::at(&codes::E0804, at, format!("`{name}` is not a valid dependency name"))
                        .help("a dependency is named like a module: letters, digits and `_`"),
                );
                continue;
            }
            if let Some(source) = self.source(name, item) {
                out.push(Dependency { name: name.to_string(), source });
            }
        }
        out
    }

    fn source(&mut self, name: &str, item: &Item) -> Option<Source> {
        let table = match item.as_value() {
            Some(Value::InlineTable(t)) => t,
            Some(Value::String(_)) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, self.item_span(item), format!("`{name}` names a version from the Agora, which is not supported yet"))
                        .note("registry dependencies come with the Agora")
                        .help("use a folder (`{ path = \"../name\" }`) or a git tag (`{ git = \"https://...\", tag = \"v0.4\" }`)"),
                );
                return None;
            }
            _ => {
                self.diags.push(
                    Diagnostic::at(&codes::E0804, self.item_span(item), format!("the dependency `{name}` must be a table"))
                        .help("write `name = { path = \"../name\" }` or `name = { git = \"https://...\", tag = \"v0.4\" }`"),
                );
                return None;
            }
        };
        let mut path = None;
        let mut git = None;
        let mut tag = None;
        for (k, v) in table.iter() {
            let at = self.span(table.key(k).and_then(|key| key.span()));
            let slot = match k {
                "path" => &mut path,
                "git" => &mut git,
                "tag" => &mut tag,
                _ => {
                    self.diags.push(
                        Diagnostic::at(&codes::E0804, at, format!("unknown key `{k}` in the dependency `{name}`"))
                            .help("a dependency has `path`, or `git` with `tag`"),
                    );
                    continue;
                }
            };
            match v.as_str() {
                Some(s) => *slot = Some((s.to_string(), self.span(v.span()))),
                None => self.diags.push(Diagnostic::at(&codes::E0802, self.span(v.span()), format!("`{k}` must be text in quotes"))),
            }
        }
        let whole = self.item_span(item);
        match (path, git, tag) {
            (Some((p, _)), None, None) => Some(Source::Path(p)),
            (None, Some((url, _)), Some((tag, _))) => Some(Source::Git { url, tag }),
            (None, Some((_, at)), None) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0803, at, format!("the git dependency `{name}` needs a `tag`"))
                        .note("a tag pins the version that is fetched")
                        .help("add `tag = \"v0.4\"` beside the `git` address"),
                );
                None
            }
            (None, None, Some((_, at))) => {
                self.diags.push(Diagnostic::at(&codes::E0804, at, "`tag` goes with `git`").help("a path dependency has no tag"));
                None
            }
            (Some((_, at)), Some(_), _) | (Some((_, at)), _, Some(_)) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0804, at, format!("the dependency `{name}` has both `path` and `git`"))
                        .help("pick one: a folder on this machine, or a git repository"),
                );
                None
            }
            (None, None, None) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0803, whole, format!("the dependency `{name}` needs `path` or `git`"))
                        .help("write `{ path = \"../name\" }` or `{ git = \"https://...\", tag = \"v0.4\" }`"),
                );
                None
            }
        }
    }

    fn build(&mut self, t: &Table) -> Build {
        self.check_keys(t, &["target", "opt"], "[build]");
        let mut b = Build::default();
        if let Some(item) = t.get("target") {
            if let Some((target, at)) = self.string(item, "target") {
                if TARGETS.contains(&target.as_str()) {
                    b.target = Some(target);
                } else {
                    self.diags.push(
                        Diagnostic::at(&codes::E0802, at, format!("`{target}` is not a build target"))
                            .help(format!("the targets are {}", TARGETS.join(", "))),
                    );
                }
            }
        }
        if let Some(item) = t.get("opt") {
            match item.as_bool() {
                Some(v) => b.opt = v,
                None => self.diags.push(Diagnostic::at(&codes::E0802, self.item_span(item), "`opt` must be `true` or `false`")),
            }
        }
        b
    }

    fn warnings(&mut self, t: &Table) -> Warnings {
        self.check_keys(t, &["as_errors"], "[warnings]");
        let mut w = Warnings::default();
        if let Some(item) = t.get("as_errors") {
            match item.as_array() {
                Some(list) => {
                    for v in list.iter() {
                        match v.as_str() {
                            Some(name) => w.as_errors.push(name.to_string()),
                            None => self.diags.push(Diagnostic::at(&codes::E0802, self.span(v.span()), "a warning name is text in quotes")),
                        }
                    }
                }
                None => self.diags.push(Diagnostic::at(&codes::E0802, self.item_span(item), "`as_errors` is a list of warning names")),
            }
        }
        w
    }

    fn behaviour(&mut self, t: &Table, mut b: Behaviour) -> Behaviour {
        let known = [Overflow::KEY, Bounds::KEY, MissingKey::KEY, UnhandledFail::KEY, ImportClash::KEY, UnsignedNegate::KEY];
        self.check_keys(t, &known, "[behaviour]");
        for (key, item) in t.iter() {
            if !known.contains(&key) {
                continue;
            }
            let Some((word, at)) = self.string(item, key) else { continue };
            // Each key has its own word list; a wrong word is E0802.
            macro_rules! set {
                ($ty:ident, $field:ident) => {
                    match $ty::parse(&word) {
                        Some(v) => b.$field = v,
                        None => self.diags.push(
                            Diagnostic::at(&codes::E0802, at, format!("`{key}` must be {}, found `{word}`", one_of($ty::WORDS)))
                                .help(format!("the default is `{}`", $ty::default().word())),
                        ),
                    }
                };
            }
            match key {
                Overflow::KEY => set!(Overflow, overflow),
                Bounds::KEY => set!(Bounds, bounds),
                MissingKey::KEY => set!(MissingKey, missing_key),
                UnhandledFail::KEY => set!(UnhandledFail, unhandled_fail),
                ImportClash::KEY => set!(ImportClash, import_clash),
                _ => set!(UnsignedNegate, unsigned_negate),
            }
        }
        b
    }
}

/// The closest of `known` to `key`, if it is within two edits.
pub(crate) fn similar<'a>(key: &str, known: &[&'a str]) -> Option<&'a str> {
    known.iter().copied().map(|k| (edit_distance(key, k), k)).filter(|(d, _)| *d <= 2).min_by_key(|(d, _)| *d).map(|(_, k)| k)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<u8> = b.bytes().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.bytes().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Config {
        match parse(text, 0, false) {
            Ok(c) => c,
            Err(d) => panic!("unexpected: {:?}", d.iter().map(|d| d.message.clone()).collect::<Vec<_>>()),
        }
    }

    fn codes_of(text: &str, script: bool) -> Vec<&'static str> {
        match parse(text, 0, script) {
            Ok(_) => Vec::new(),
            Err(d) => d.iter().map(|d| d.code.id).collect(),
        }
    }

    const NAMED: &str = "[project]\nname = \"mygame\"\n";

    #[test]
    fn defaults() {
        let c = ok(NAMED);
        assert_eq!(c.project.name.as_deref(), Some("mygame"));
        assert_eq!(c.project.kind, Kind::Program);
        assert_eq!(c.project.entry, "src/main.drh");
        assert!(c.dependencies.is_empty());
        assert_eq!(c.behaviour, Behaviour::default());
        assert_eq!(c.behaviour.overflow, Overflow::Wrap);
        assert_eq!(c.behaviour.bounds, Bounds::Panic);
        assert_eq!(c.behaviour.unhandled_fail, UnhandledFail::Error);
        assert!(!c.build.opt);
    }

    #[test]
    fn kind_and_entry() {
        let c = ok("[project]\nname = \"geo\"\nkind = \"library\"\nentry = \"lib/geo.drh\"\n");
        assert_eq!(c.project.kind, Kind::Library);
        assert_eq!(c.project.entry, "lib/geo.drh");
        assert_eq!(codes_of("[project]\nname = \"geo\"\nkind = \"lib\"\n", false), ["E0802"]);
        assert_eq!(codes_of("[project]\nname = \"geo\"\nentry = \"../x.drh\"\n", false), ["E0802"]);
        assert_eq!(codes_of("[project]\nname = \"geo\"\nentry = \"main.txt\"\n", false), ["E0802"]);
    }

    #[test]
    fn name_is_required_in_a_project_but_not_a_script() {
        assert_eq!(codes_of("[project]\nversion = \"1\"\n", false), ["E0803"]);
        assert_eq!(codes_of("", false), ["E0803"]);
        assert!(parse("", 0, true).is_ok());
        assert_eq!(codes_of("[project]\nname = \"2fast\"\n", false), ["E0805"]);
        assert_eq!(codes_of("[project]\nname = \"my-game\"\n", true), ["E0805"]);
    }

    #[test]
    fn unknown_settings_and_tables() {
        let d = parse("[project]\nname = \"a\"\nnmae = \"b\"\n", 0, false).unwrap_err();
        assert_eq!(d[0].code.id, "E0801");
        assert_eq!(d[0].message, "unknown setting `nmae` in [project]");
        assert_eq!(d[0].help.as_deref(), Some("did you mean `name`?"));
        assert_eq!(codes_of("[project]\nname = \"a\"\n[profile]\nx = 1\n", false), ["E0801"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\nname2 = \"a\"\n", false), ["E0801"]);
    }

    #[test]
    fn behaviour_values() {
        let c = ok("[project]\nname = \"a\"\n[behaviour]\noverflow = \"saturate\"\nbounds = \"unchecked\"\nmissing_key = \"panic\"\nunhandled_fail = \"crash\"\nimport_clash = \"error\"\nunsigned_negate = \"signed\"\n");
        assert_eq!(c.behaviour.overflow, Overflow::Saturate);
        assert_eq!(c.behaviour.bounds, Bounds::Unchecked);
        assert_eq!(c.behaviour.missing_key, MissingKey::Panic);
        assert_eq!(c.behaviour.unhandled_fail, UnhandledFail::Crash);
        assert_eq!(c.behaviour.import_clash, ImportClash::Error);
        assert_eq!(c.behaviour.unsigned_negate, UnsignedNegate::Signed);
        assert_eq!(c.behaviour.overflow.word(), "saturate");
    }

    #[test]
    fn bad_behaviour_value_lists_the_words() {
        let d = parse("[project]\nname = \"a\"\n[behaviour]\noverflow = \"clamp\"\n", 0, false).unwrap_err();
        assert_eq!(d[0].code.id, "E0802");
        assert_eq!(d[0].message, "`overflow` must be wrap, saturate or panic, found `clamp`");
        assert_eq!(d[0].help.as_deref(), Some("the default is `wrap`"));
        assert_eq!(codes_of("[project]\nname = \"a\"\n[behaviour]\noverflw = \"wrap\"\n", false), ["E0801"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[behaviour]\nbounds = 3\n", false), ["E0802"]);
    }

    #[test]
    fn scripts_crash_on_unhandled_failures_by_default() {
        let c = parse("", 0, true).unwrap();
        assert_eq!(c.behaviour.unhandled_fail, UnhandledFail::Crash);
        let c = parse("[behaviour]\nunhandled_fail = \"error\"\n", 0, true).unwrap();
        assert_eq!(c.behaviour.unhandled_fail, UnhandledFail::Error);
        assert_eq!(Config::script().behaviour.unhandled_fail, UnhandledFail::Crash);
    }

    #[test]
    fn file_flags_override_only_the_three() {
        let c = ok("[project]\nname = \"a\"\n[behaviour]\noverflow = \"panic\"\nbounds = \"unchecked\"\n");
        let file = FileBehaviour { overflow: Some(Overflow::Saturate), ..FileBehaviour::default() };
        let b = c.behaviour.for_file(&file);
        assert_eq!(b.overflow, Overflow::Saturate);
        assert_eq!(b.bounds, Bounds::Unchecked);
        assert_eq!(b.unsigned_negate, UnsignedNegate::Wrap);
        assert_eq!(FILE_KEYS, ["overflow", "bounds", "unsigned_negate"]);
        assert_eq!(PROJECT_KEYS, ["missing_key", "unhandled_fail", "import_clash"]);
    }

    #[test]
    fn path_dependencies() {
        let c = ok("[project]\nname = \"a\"\n[dependencies]\nutils = { path = \"../utils\" }\n");
        assert_eq!(c.dependencies, [Dependency { name: "utils".into(), source: Source::Path("../utils".into()) }]);
    }

    #[test]
    fn git_dependencies() {
        let c = ok("[project]\nname = \"a\"\n[dependencies]\nsdl = { git = \"https://example.org/sdl.git\", tag = \"v0.4\" }\n");
        assert_eq!(
            c.dependencies,
            [Dependency { name: "sdl".into(), source: Source::Git { url: "https://example.org/sdl.git".into(), tag: "v0.4".into() } }]
        );
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\nsdl = { git = \"https://x\" }\n", false), ["E0803"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\nsdl = { tag = \"v1\" }\n", false), ["E0804"]);
    }

    #[test]
    fn bad_dependencies() {
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\njson = \"1.2\"\n", false), ["E0901"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\nx = { path = \"a\", git = \"b\", tag = \"c\" }\n", false), ["E0804"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\nx = {}\n", false), ["E0803"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\nx = { path = \"a\", version = \"1\" }\n", false), ["E0804"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\nmy-lib = { path = \"a\" }\n", false), ["E0804"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\nx = 3\n", false), ["E0804"]);
    }

    #[test]
    fn build_and_warnings() {
        let c = ok("[project]\nname = \"a\"\n[build]\ntarget = \"linux-x64\"\nopt = true\n[warnings]\nas_errors = [\"lossy-conversion\"]\n");
        assert_eq!(c.build, Build { target: Some("linux-x64".into()), opt: true });
        assert_eq!(c.warnings.as_errors, ["lossy-conversion"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[build]\ntarget = \"plan9\"\n", false), ["E0802"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[build]\nopt = 1\n", false), ["E0802"]);
        assert_eq!(codes_of("[project]\nname = \"a\"\n[warnings]\nas_errors = \"x\"\n", false), ["E0802"]);
    }

    #[test]
    fn syntax_errors_are_e0800_with_a_span() {
        let d = parse("[project\nname = \"a\"\n", 10, false).unwrap_err();
        assert_eq!(d[0].code.id, "E0800");
        assert!(d[0].labels[0].0.start >= 10);
    }

    #[test]
    fn a_table_where_a_value_goes() {
        assert_eq!(codes_of("[project]\nname = \"a\"\n[dependencies]\n", false), Vec::<&str>::new());
        assert_eq!(codes_of("dependencies = 3\n[project]\nname = \"a\"\n", false), ["E0802"]);
    }

    #[test]
    fn similar_words() {
        assert_eq!(similar("overlow", &["overflow", "bounds"]), Some("overflow"));
        assert_eq!(similar("zzzzzzz", &["overflow", "bounds"]), None);
    }

    #[test]
    fn host_is_a_known_target_or_none() {
        if let Some(t) = host_target() {
            assert!(TARGETS.contains(&t));
        }
    }
}
