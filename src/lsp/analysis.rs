//! One run of the front end over a program (a file, or a project), kept in
//! memory with what the editor asks about: diagnostics now; the side tables
//! for hover, definition and completion are added next to them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp_types::{DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString};

use super::pos::{path_to_uri, to_range};
use crate::config;
use crate::diag::{Diagnostic, Severity, SourceMap};
use crate::modules::{self, ModuleGraph};
use crate::typeck::index::{self, Index};

pub struct Analysis {
    /// The file the program starts at (the project's entry, or the file itself).
    pub entry: PathBuf,
    /// Files of the program, by file id of `map`.
    pub files: Vec<PathBuf>,
    pub map: SourceMap,
    pub graph: Option<ModuleGraph>,
    pub diags: Vec<Diagnostic>,
    /// What the checker found out about names and types.
    pub index: Index,
}

/// The nearest folder above `file` with a `drachm.toml`.
fn project_dir(file: &Path) -> Option<PathBuf> {
    let start = file.canonicalize().ok().or_else(|| Some(file.to_path_buf()))?;
    start.ancestors().skip(1).find(|d| d.join(config::PROJECT_FILE).is_file()).map(Path::to_path_buf)
}

/// The index of a program that did not pass the parser, from the tree the
/// parser's recovery left. A panic on such a tree gives an empty index.
fn recover_index(graph: &mut ModuleGraph) -> Index {
    let run = std::panic::AssertUnwindSafe(|| {
        index::collect(|| {
            let mut scratch = Vec::new();
            if graph.modules.len() > 1 {
                let program = crate::resolve::resolve(graph, &mut scratch);
                let src = (0..graph.map.len()).map(|f| graph.map.source(f).text.as_str()).collect::<Vec<_>>().join("\n");
                crate::typeck::check(&program, &src, &mut scratch);
            } else if let Some(m) = graph.modules.first() {
                crate::typeck::check(&m.program, &graph.map.source(0).text, &mut scratch);
            }
        })
        .1
    });
    std::panic::catch_unwind(run).unwrap_or_default()
}

impl Analysis {
    /// Check the program `file` belongs to. `overlay` holds the text of the
    /// open buffers, which wins over what is on disk.
    pub fn run(file: &Path, overlay: &HashMap<PathBuf, String>) -> Analysis {
        let files: HashMap<PathBuf, String> = overlay.iter().map(|(p, t)| (modules::overlay_key(p), t.clone())).collect();
        modules::with_overlay(files, || {
            // A file of a project is checked as part of it.
            if let Some(dir) = project_dir(file)
                && let Some(a) = Self::open_and_check(&dir, Some(file))
            {
                return a;
            }
            Self::open_and_check(file, None).unwrap_or_else(|| Analysis::empty(file))
        })
    }

    fn empty(file: &Path) -> Analysis {
        Analysis { entry: file.to_path_buf(), files: Vec::new(), map: SourceMap::new(), graph: None, diags: Vec::new(), index: Index::default() }
    }

    /// `None` when `member` is given and the program does not contain it.
    fn open_and_check(at: &Path, member: Option<&Path>) -> Option<Analysis> {
        let opened = match config::open(at) {
            Ok(o) => o,
            Err(f) => {
                // The config could not be read: its problems belong to the config files.
                let base = if at.is_dir() { at.to_path_buf() } else { at.parent().unwrap_or(Path::new(".")).to_path_buf() };
                let files = (0..f.map.len()).map(|i| base.join(&f.map.source(i).name)).collect();
                return Some(Analysis { entry: at.to_path_buf(), files, map: f.map, graph: None, diags: f.diags, index: Index::default() });
            }
        };
        let ((_, mut graph, diags), mut index) = index::collect(|| crate::check_project(&opened.entry, &opened.info));
        if !index.ran {
            // Syntax errors stopped the checker. Check what the parser could
            // keep, for the names and types only: its errors would be noise.
            index = recover_index(&mut graph);
        }
        let mut files = vec![PathBuf::new(); graph.map.len()];
        for m in &graph.modules {
            files[m.file] = m.path.clone();
        }
        if let Some(member) = member {
            let key = modules::overlay_key(member);
            if !files.iter().any(|f| modules::overlay_key(f) == key) {
                return None;
            }
        }
        Some(Analysis { entry: opened.entry, files, map: SourceMap::new(), graph: Some(graph), diags, index })
    }

    /// The file id of `path` in this program.
    pub fn file_id(&self, path: &Path) -> Option<usize> {
        let key = modules::overlay_key(path);
        self.files.iter().position(|f| modules::overlay_key(f) == key)
    }

    /// The diagnostics as LSP ones, by file. Every file of the program is a
    /// key, so a file whose problems were fixed gets its empty list.
    pub fn lsp_diagnostics(&self) -> Vec<(PathBuf, Vec<lsp_types::Diagnostic>)> {
        let mut out: Vec<(PathBuf, Vec<lsp_types::Diagnostic>)> = self.files.iter().map(|f| (f.clone(), Vec::new())).collect();
        if out.is_empty() {
            return out;
        }
        let map = self.source_map();
        for d in &self.diags {
            let (file, range) = match d.labels.first() {
                Some((span, _)) if !map.is_empty() => {
                    let (f, local) = map.locate(*span);
                    (f, to_range(map.source(f), local))
                }
                _ => (0, lsp_types::Range::default()),
            };
            let mut related = Vec::new();
            let here = |f: usize, r| Location { uri: path_to_uri(&self.files[f]).expect("a file path is a uri"), range: r };
            for (span, text) in &d.labels {
                if text.is_empty() {
                    continue;
                }
                let (f, local) = map.locate(*span);
                related.push(DiagnosticRelatedInformation { location: here(f, to_range(map.source(f), local)), message: text.clone() });
            }
            for n in &d.notes {
                related.push(DiagnosticRelatedInformation { location: here(file, range), message: format!("note: {n}") });
            }
            if let Some(h) = &d.help {
                related.push(DiagnosticRelatedInformation { location: here(file, range), message: format!("help: {h}") });
            }
            let lsp = lsp_types::Diagnostic {
                range,
                severity: Some(match d.severity {
                    Severity::Error => DiagnosticSeverity::ERROR,
                    Severity::Warning => DiagnosticSeverity::WARNING,
                }),
                code: Some(NumberOrString::String(d.code.id.to_string())),
                source: Some("drachm".to_string()),
                message: d.message.clone(),
                related_information: (!related.is_empty()).then_some(related),
                ..Default::default()
            };
            out[file].1.push(lsp);
        }
        out
    }

    pub fn source_map(&self) -> &SourceMap {
        match &self.graph {
            Some(g) => &g.map,
            None => &self.map,
        }
    }
}
