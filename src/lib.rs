//! The Drachm compiler as a library. The `drachm` binary, the tests and
//! (later) the language server all drive it through here.

pub mod ast;
pub mod cgen_common;
pub mod cgen_mir;
pub mod codegen_common;
pub mod codegen_mir;
pub mod config;
pub mod consts;
pub mod deps;
pub mod diag;
pub mod flags;
pub mod hir;
pub mod lexer;
pub mod link;
pub mod mir;
pub mod modules;
pub mod ownership;
pub mod parser;
pub mod typeck;
pub mod types;
pub mod units;

use diag::Diagnostic;

/// Lex, parse and type-check `src`, without the ownership checks on the MIR
/// (tests of those checks need programs that have ownership errors).
pub fn front(src: &str) -> (Option<hir::Program>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let flags = flags::parse(src, &mut diags);
    let tokens = lexer::lex(src, &mut diags);
    let mut program = parser::parse(tokens, &mut diags);
    program.unsafe_allowed = flags.unsafe_allowed;
    // A program with syntax errors has holes in it; type errors found in what
    // is left would mostly be noise.
    if diag::has_errors(&diags) {
        return (None, diags);
    }
    let checked = typeck::check(&program, src, &mut diags);
    if diag::has_errors(&diags) {
        return (None, diags);
    }
    (Some(checked), diags)
}

/// Lex, parse, type-check and ownership-check `src`. Returns every diagnostic
/// found (warnings included); the program is `None` if any of them is an error.
pub fn check(src: &str) -> (Option<hir::Program>, Vec<Diagnostic>) {
    let (checked, diags) = front(src);
    ownership_checks(checked, diags)
}

/// The ownership checks, which run on the MIR, after `front`.
fn ownership_checks(checked: Option<hir::Program>, mut diags: Vec<Diagnostic>) -> (Option<hir::Program>, Vec<Diagnostic>) {
    let Some(checked) = checked else { return (None, diags) };
    let mut mir = mir::build_program(&checked, &mut diags);
    ownership::conform::check(&checked, &mir.self_modes, &mut diags);
    if diag::has_errors(&diags) {
        return (None, diags);
    }
    mir::check::run(&checked, &mut mir, &mut diags);
    if diag::has_errors(&diags) {
        return (None, diags);
    }
    (Some(checked), diags)
}

/// `front` for a program that may be several files: load the module graph from
/// `entry`, then check it. The graph (with its source map) is returned for
/// printing the diagnostics. A graph of one file is checked exactly like `front`.
///
/// Step 7 is only started: a project with other files in it loads, parses and
/// has its imports and exports checked, but its names are not yet resolved
/// across files (WP2), so it ends in E0901 where the first file import is.
pub fn front_project(entry: &std::path::Path, info: &modules::ProjectInfo) -> (Option<hir::Program>, modules::ModuleGraph, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let graph = modules::load(entry, info, &mut diags);
    if diag::has_errors(&diags) || graph.modules.is_empty() {
        return (None, graph, diags);
    }
    if graph.modules.len() > 1 {
        let main = &graph.modules[0];
        // The first import of a project file, for the message.
        let at = main
            .targets
            .iter()
            .position(|t| matches!(t, Some(modules::Target::File(_))))
            .map(|i| main.program.imports[i].span);
        let mut d = Diagnostic::new(&diag::codes::E0901, "a program of several files cannot be built yet")
            .note_line(format!("{} files were found and read; naming things across files is the next part of step 7", graph.modules.len()));
        if let Some(at) = at {
            d = d.label(at, "this imports a project file");
        }
        diags.push(d);
        return (None, graph, diags);
    }
    let program = &graph.modules[0].program;
    let src = &graph.map.source(0).text;
    let checked = typeck::check(program, src, &mut diags);
    if diag::has_errors(&diags) {
        return (None, graph, diags);
    }
    (Some(checked), graph, diags)
}

/// `check` for a project: `front_project`, then the ownership checks.
pub fn check_project(entry: &std::path::Path, info: &modules::ProjectInfo) -> (Option<hir::Program>, modules::ModuleGraph, Vec<Diagnostic>) {
    let (checked, graph, diags) = front_project(entry, info);
    let (checked, diags) = ownership_checks(checked, diags);
    (checked, graph, diags)
}

/// Build an executable at `out`. Plain builds go through Cranelift, which is
/// fast to compile; optimised builds go through C, which is fast to run.
///
/// Both code generators read the same MIR: built, marked (which locals live in
/// memory), cleaned, drop-elaborated, cleaned again. The errors, if any, were
/// reported by `check`.
pub fn build(program: &hir::Program, out: &std::path::Path, optimise: bool) -> Result<(), String> {
    let mut mir = mir::build_program(program, &mut Vec::new());
    for body in mir.bodies.iter_mut().flatten() {
        mir::analysis::mark_in_memory(program, body);
    }
    mir::cleanup::cleanup_program(program, &mut mir);
    mir::drops::elaborate_program(program, &mut mir);
    mir::cleanup::cleanup_program(program, &mut mir);
    if optimise {
        link::compile_c(&cgen_mir::generate(program, &mir), out)
    } else {
        link::link_object(&codegen_mir::compile(program, &mir)?, out)
    }
}
