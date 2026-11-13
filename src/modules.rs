//! The module graph: which files make up a build, found by following `in`.
//!
//! A module is one file. Starting from the entry file, every `in` path is
//! resolved to a standard module (`io`, `si`) or to a file of the project;
//! each file is read and parsed once, whoever imports it, and all files share
//! one `diag::SourceMap` so a `Span` names its file by position.
//!
//! Paths (owner, step 7 batch A): a path is relative to the folder of the file
//! that writes it, `in game.physics;` (or `in physics inside game;`) is the file
//! `game/physics.drh` beside this one. A path that starts with the project's
//! name starts at the project's source folder instead: `in mygame.game.physics;`.
//! Import cycles are allowed, so loading marks a file as known before it reads
//! its imports.
//!
//! This is WP1: loading, resolving and the checks that need only paths and
//! export lists. What a name means across files (scopes, visibility, `in *`,
//! shadowing) is WP2.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast;
use crate::diag::{codes, Diagnostic, SourceMap, Span};
use crate::{flags, lexer, parser};

/// The modules the compiler carries inside it. WP6 grows this list.
pub const STD_MODULES: &[&str] = &["io", "si"];

/// Where a project's files are and what its root path starts with.
#[derive(Debug, Clone)]
pub struct ProjectInfo {
    /// The project's name from `drachm.toml`: `in NAME.game.physics;` starts at
    /// `root`. A script has none.
    pub name: Option<String>,
    /// The source folder (`src/`), or for a script the folder of its file.
    pub root: PathBuf,
    /// File names in diagnostics are shown relative to this (the folder with
    /// `drachm.toml`); `None` shows them as they were given.
    pub display_base: Option<PathBuf>,
    pub requires_main: bool,
    /// Dependency aliases declared in this project.
    pub dependency_names: Vec<String>,
}

impl ProjectInfo {
    /// A script or a single file: its root is its own folder.
    pub fn script(file: &Path) -> ProjectInfo {
        let dir = file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        ProjectInfo { name: None, root: dir.to_path_buf(), display_base: None, requires_main: true, dependency_names: Vec::new() }
    }

}

/// What an `in` path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A project file: an index into `ModuleGraph::modules`.
    File(usize),
    /// A module built into the compiler.
    Std(&'static str),
}

#[derive(Debug)]
pub struct Module {
    /// The dotted path from the source root, `game.physics` (`main` for the entry).
    pub name: String,
    pub path: PathBuf,
    /// The file's id in `ModuleGraph::map`.
    pub file: usize,
    pub program: ast::Program,
    /// One entry per `program.imports`, in order; `None` where the import
    /// could not be resolved (an error was reported).
    pub targets: Vec<Option<Target>>,
}

pub struct ModuleGraph {
    /// The entry file first, then the others in the order they were found.
    pub modules: Vec<Module>,
    pub map: SourceMap,
}

impl ModuleGraph {
    /// Module indices with the files an import reaches before the importer
    /// (depth first from the entry, which comes last). A cycle is cut where it
    /// closes, so every module appears once.
    pub fn order(&self) -> Vec<usize> {
        fn visit(g: &ModuleGraph, i: usize, seen: &mut [bool], out: &mut Vec<usize>) {
            if std::mem::replace(&mut seen[i], true) {
                return;
            }
            for t in g.modules[i].targets.iter().flatten() {
                if let Target::File(j) = t {
                    visit(g, *j, seen, out);
                }
            }
            out.push(i);
        }
        let mut seen = vec![false; self.modules.len()];
        let mut out = Vec::new();
        for i in 0..self.modules.len() {
            visit(self, i, &mut seen, &mut out);
        }
        out
    }

    /// The files `module` imports (not the standard modules).
    pub fn file_imports(&self, module: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self.modules[module]
            .targets
            .iter()
            .filter_map(|t| match t {
                Some(Target::File(j)) => Some(*j),
                _ => None,
            })
            .collect();
        out.dedup();
        out
    }
}

/// Load `entry` and every file it imports. Syntax errors, unresolved imports
/// and export mistakes go to `diags`; the graph is returned either way, with
/// as much as could be loaded.
pub fn load(entry: &Path, info: &ProjectInfo, diags: &mut Vec<Diagnostic>) -> ModuleGraph {
    load_with_dependencies(entry, info, &[], diags)
}

pub(crate) fn load_with_dependencies(entry: &Path, info: &ProjectInfo, deps: &[crate::deps::Root], diags: &mut Vec<Diagnostic>) -> ModuleGraph {
    let project_dir = info.display_base.as_deref().unwrap_or(&info.root);
    let project_dir = overlay_key(project_dir);
    let mut ld = Loader { info, deps, project_dir, graph: ModuleGraph { modules: Vec::new(), map: SourceMap::new() }, by_path: HashMap::new(), diags };
    ld.add_file(entry, None);
    let mut i = 0;
    while i < ld.graph.modules.len() {
        ld.resolve_imports(i);
        ld.check_exports(i);
        i += 1;
    }
    ld.graph
}

struct Loader<'a> {
    info: &'a ProjectInfo,
    deps: &'a [crate::deps::Root],
    project_dir: PathBuf,
    graph: ModuleGraph,
    by_path: HashMap<PathBuf, usize>,
    diags: &'a mut Vec<Diagnostic>,
}

thread_local! {
    /// Text that stands in for files on disk while loading (the language
    /// server's unsaved buffers), by canonical path.
    static OVERLAY: std::cell::RefCell<HashMap<PathBuf, String>> = std::cell::RefCell::new(HashMap::new());
}

/// The key `with_overlay` files are looked up by.
pub fn overlay_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Run `f` with `files` read instead of the files of the same path on disk.
pub fn with_overlay<R>(files: HashMap<PathBuf, String>, f: impl FnOnce() -> R) -> R {
    let old = OVERLAY.with(|o| std::mem::replace(&mut *o.borrow_mut(), files));
    let r = f();
    OVERLAY.with(|o| *o.borrow_mut() = old);
    r
}

fn read_source(path: &Path) -> std::io::Result<String> {
    if let Some(t) = OVERLAY.with(|o| o.borrow().get(&overlay_key(path)).cloned()) {
        return Ok(t);
    }
    std::fs::read_to_string(path)
}

/// A path as text, with `/` on every system.
fn shown(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// A path with `.` and `..` folded away (without touching the disk).
fn normalise(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The file an import path means below `dir`: `a.b.c` is `dir/a/b/c.drh`.
fn file_for(dir: &Path, segs: &[String]) -> PathBuf {
    let mut p = dir.to_path_buf();
    for s in &segs[..segs.len() - 1] {
        p.push(s);
    }
    p.push(format!("{}.drh", segs[segs.len() - 1]));
    p
}

impl Loader<'_> {
    fn dependency_owner(&self, path: &Path) -> Option<&crate::deps::Root> {
        let path = overlay_key(path);
        self.deps.iter().filter(|dep| path.starts_with(&dep.src))
            .max_by_key(|dep| dep.src.components().count())
    }

    fn visible(&mut self, importer: &Path, target: &Path, at: Span) -> bool {
        let Some(dep) = self.dependency_owner(target) else { return true };
        if self.dependency_owner(importer).is_some_and(|owner| owner.src == dep.src) {
            return true;
        }
        let importer_dir = self.dependency_owner(importer).map_or_else(|| self.project_dir.clone(), |owner| overlay_key(&owner.dir));
        let module = overlay_key(target).strip_prefix(&dep.src).ok()
            .map(|path| path.with_extension("").components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("."));
        if self.deps.iter().any(|edge| edge.src == dep.src && edge.declared_in == importer_dir)
            && module.as_ref().is_some_and(|name| dep.config.project.public_modules.contains(name))
        {
            return true;
        }
        self.diags.push(Diagnostic::at(&codes::E0236, at, format!("dependency module `{}` is private", self.display(target)))
            .help("declare the dependency and list the module in its `[project] public_modules`"));
        false
    }

    fn display(&self, path: &Path) -> String {
        match &self.info.display_base {
            Some(base) => shown(path.strip_prefix(base).unwrap_or(path)),
            None => shown(path),
        }
    }

    fn module_name(&self, path: &Path) -> String {
        if let Some(dep) = self.dependency_owner(path) {
            let path = overlay_key(path);
            if let Ok(relative) = path.strip_prefix(&dep.src) {
                let module = relative.with_extension("").components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join(".");
                return format!("{}.{module}", dep.config.project.name.as_deref().unwrap_or(&dep.name));
            }
        }
        match path.strip_prefix(&self.info.root) {
            Ok(rel) => rel.with_extension("").components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("."),
            Err(_) => path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        }
    }

    /// Read and parse `path` unless it is known already. `from` is the import
    /// that led here (for the error if the file cannot be read).
    fn add_file(&mut self, path: &Path, from: Option<Span>) -> Option<usize> {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(&i) = self.by_path.get(&key) {
            return Some(i);
        }
        let text = match read_source(path) {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("cannot read `{}`: {e}", self.display(path));
                self.diags.push(match from {
                    Some(span) => Diagnostic::at(&codes::E0235, span, msg),
                    None => Diagnostic::new(&codes::E0235, msg),
                });
                return None;
            }
        };
        let (file, base) = self.graph.map.add(self.display(path), text.clone());
        let fl = flags::parse_at(&text, base, self.diags);
        let tokens = lexer::lex_at(&text, base, self.diags);
        let mut program = parser::parse(tokens, self.diags);
        program.unsafe_allowed = fl.unsafe_allowed;
        let index = self.graph.modules.len();
        self.by_path.insert(key, index);
        let mut name = self.module_name(path);
        while self.graph.modules.iter().any(|m| m.name == name) {
            name = format!("{name}_{index}");
        }
        self.graph.modules.push(Module { name, path: path.to_path_buf(), file, program, targets: Vec::new() });
        Some(index)
    }

    fn resolve_imports(&mut self, m: usize) {
        let imports: Vec<(Vec<String>, Span, Option<String>, Span, Option<String>)> = self.graph.modules[m]
            .program
            .imports
            .iter()
            .map(|i| (i.path.clone(), i.path_span, i.binding().map(String::from), i.span, i.file.clone()))
            .collect();
        let importer = self.graph.modules[m].path.clone();
        let mut targets = Vec::new();
        // The name each module import gives, for the same-name check.
        let mut bound: Vec<(String, Target)> = Vec::new();
        for (path, path_span, binding, whole, file) in imports {
            let target = match &file {
                Some(f) => self.resolve_file(&importer, f, path_span),
                None => self.resolve(&importer, &path, path_span),
            };
            if let (Some(name), Some(t)) = (binding, target) {
                match bound.iter().find(|(n, _)| *n == name) {
                    Some((_, first)) if *first != t => self.diags.push(same_name(&name, &path, whole)),
                    Some(_) => {}
                    None => bound.push((name, t)),
                }
            }
            targets.push(target);
        }
        self.graph.modules[m].targets = targets;
    }

    /// An import by file path: relative to the importing file's folder, `..` allowed.
    fn resolve_file(&mut self, importer: &Path, file: &str, at: Span) -> Option<Target> {
        let dir = importer.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let path = normalise(&dir.join(file));
        if !path.is_file() {
            self.diags.push(
                Diagnostic::at(&codes::E0230, at, format!("module file `{file}` not found"))
                    .note("no such file")
                    .note_line(format!("looked for `{}`", self.display(&path)))
                    .help("a file path is relative to this file's folder and may climb with `..`: `in \"../runtime/scheduler.drh\";`"),
            );
            return None;
        }
        if !self.visible(importer, &path, at) {
            return None;
        }
        self.add_file(&path, Some(at)).map(Target::File)
    }

    fn resolve(&mut self, importer: &Path, segs: &[String], at: Span) -> Option<Target> {
        let written = segs.join(".");
        let dir = importer.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let std = (segs.len() == 1).then(|| STD_MODULES.iter().find(|m| **m == segs[0])).flatten().copied();
        let beside = file_for(dir, segs);
        let owner = self.dependency_owner(importer);
        let (project_name, root, project_dir) = owner.map_or(
            (self.info.name.as_ref(), &self.info.root, self.project_dir.clone()),
            |dep| (dep.config.project.name.as_ref(), &dep.src, overlay_key(&dep.dir)),
        );
        let rooted = (project_name == segs.first() && segs.len() > 1)
            .then(|| file_for(root, &segs[1..]));
        let dependency = self.deps.iter().find(|dep| dep.declared_in == project_dir && dep.name == segs[0])
            .filter(|_| segs.len() > 1)
            .map(|dep| file_for(&dep.src, &segs[1..]));
        // The places this path could mean; two different ones is an error.
        let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let mut files: Vec<PathBuf> = Vec::new();
        for f in [Some(beside.clone()), rooted.clone(), dependency.clone()].into_iter().flatten() {
            if f.is_file() && !files.iter().any(|g| canon(g) == canon(&f)) {
                files.push(f);
            }
        }
        let mut places: Vec<String> = Vec::new();
        if let Some(s) = std {
            places.push(format!("the standard module `{s}`"));
        }
        places.extend(files.iter().map(|f| format!("the file `{}`", self.display(f))));
        if places.len() > 1 {
            self.diags.push(
                Diagnostic::at(&codes::E0231, at, format!("`{written}` could mean {}", places.join(" or ")))
                    .note("ambiguous")
                    .help("rename the file or folder: a standard module is never replaced by a project file silently"),
            );
            return None;
        }
        if let Some(s) = std {
            return Some(Target::Std(s));
        }
        let Some(file) = files.pop() else {
            if self.info.dependency_names.contains(&segs[0]) {
                self.diags.push(Diagnostic::at(
                    &codes::E0230,
                    at,
                    format!("importing from dependency `{}` requires a public module path", segs[0]),
                ));
                return None;
            }
            let mut d = Diagnostic::at(&codes::E0230, at, format!("module `{written}` not found")).note("no such module");
            d = d.note_line(format!("looked for the file `{}`", self.display(&beside)));
            if let Some(r) = &rooted {
                d = d.note_line(format!("and, from the project root, `{}`", self.display(r)));
            }
            if let Some(path) = &dependency {
                d = d.note_line(format!("and, in the dependency, `{}`", self.display(path)));
            }
            d = d.help(match &self.info.name {
                Some(n) => format!("a path is relative to this file's folder; to start at the project's source folder, begin with `{n}`: `in {n}.{written};`"),
                None => "a path is relative to this file's folder: `in game.physics;` is the file `game/physics.drh`".to_string(),
            });
            self.diags.push(d);
            return None;
        };
        if !self.visible(importer, &file, at) {
            return None;
        }
        self.add_file(&file, Some(at)).map(Target::File)
    }

    /// The export list of module `m`: unique names, all declared in the file.
    fn check_exports(&mut self, m: usize) {
        let prog = &self.graph.modules[m].program;
        let declared: Vec<&str> = prog
            .funcs
            .iter()
            .map(|f| f.name.as_str())
            .chain(prog.structs.iter().map(|s| s.name.as_str()))
            .chain(prog.enums.iter().map(|e| e.name.as_str()))
            .chain(prog.interfaces.iter().map(|i| i.name.as_str()))
            .chain(prog.failures.iter().map(|f| f.name.as_str()))
            .chain(prog.units.iter().map(|u| u.name.as_str()))
            .chain(prog.consts.iter().map(|c| c.name.as_str()))
            .collect();
        let mut imported: Vec<&str> = Vec::new();
        for i in &prog.imports {
            imported.extend(i.binding());
            imported.extend(i.items.iter().map(|it| it.alias.as_deref().unwrap_or(&it.name)));
        }
        let has_glob = prog.imports.iter().any(|i| i.glob);
        let mut shown_names: Vec<(&str, Span)> = Vec::new();
        let mut out = Vec::new();
        for e in &prog.exports {
            if let Some((_, first)) = shown_names.iter().find(|(n, _)| *n == e.shown_as()) {
                out.push(
                    Diagnostic::at(&codes::E0233, e.span, format!("`{}` is exported twice", e.shown_as()))
                        .note("exported again here")
                        .label(*first, "first exported here")
                        .help("export each name once, or rename one with `as`"),
                );
                continue;
            }
            shown_names.push((e.shown_as(), e.span));
            if declared.contains(&e.name.as_str()) {
                continue;
            }
            if imported.contains(&e.name.as_str()) || has_glob {
                out.push(
                    Diagnostic::at(&codes::E0901, e.span, format!("`{}` comes from an import; exporting an imported name requires a public module path", e.name))
                        .note("re-export")
                        .help("export names the file declares itself"),
                );
            } else {
                out.push(
                    Diagnostic::at(&codes::E0234, e.span, format!("`{}` is exported but not declared in this file", e.name))
                        .note("no such declaration")
                        .help("only the functions, structs, enums, interfaces, failures and units written in this file can be exported"),
                );
            }
        }
        self.diags.extend(out);
    }
}

/// E0232 with the fix-it `in engine.audio as engineAudio;`.
fn same_name(name: &str, path: &[String], whole: Span) -> Diagnostic {
    let fix = match path {
        [.., folder, last] => {
            let mut cap = last.chars();
            let cap: String = cap.next().map(|c| c.to_uppercase().collect::<String>() + cap.as_str()).unwrap_or_default();
            format!("{folder}{cap}")
        }
        _ => format!("{name}2"),
    };
    Diagnostic::at(&codes::E0232, whole, format!("`{name}` already names another module in this file"))
        .note("second module with this name")
        .help(format!("give it a name of its own: `in {} as {fix};`", path.join(".")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_paths() {
        let segs: Vec<String> = ["game", "physics"].iter().map(|s| s.to_string()).collect();
        assert_eq!(shown(&file_for(Path::new("src"), &segs)), "src/game/physics.drh");
    }
}
