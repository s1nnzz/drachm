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
pub mod lsp;
pub mod mir;
pub mod modules;
pub mod ownership;
pub mod parser;
pub mod resolve;
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
    resolve::chain_slips_by_imports(&mut program, &mut diags);
    // A program with syntax errors has holes in it; type errors found in what
    // is left would mostly be noise.
    if diag::has_errors(&diags) {
        return (None, diags);
    }
    let settings = config::Behaviour::default().for_file(&flags.behaviour);
    let mut checked = typeck::check_with_settings(&program, src, &mut diags, vec![(0, settings)], true);
    if diag::has_errors(&diags) {
        return (None, diags);
    }
    locate_asserts(&mut checked, &diag::SourceMap::single("input", src));
    (Some(checked), diags)
}

/// Writes the message of each `assert`: its heading (the condition and the
/// message) and its place, which needs the files' names, so it is done once
/// the files are known.
fn locate_asserts(program: &mut hir::Program, map: &diag::SourceMap) {
    for a in &mut program.asserts {
        a.message = format!("{}\n  --> {}", a.heading, map.file_line(a.span.start));
    }
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
/// A project of several files has its names resolved by `resolve` and is then
/// checked as one program.
pub fn front_project(entry: &std::path::Path, info: &modules::ProjectInfo) -> (Option<hir::Program>, modules::ModuleGraph, Vec<Diagnostic>) {
    front_project_with_settings(entry, info, config::Behaviour::default(), &[])
}

fn front_project_with_settings(entry: &std::path::Path, info: &modules::ProjectInfo, behaviour: config::Behaviour, deps: &[deps::Root]) -> (Option<hir::Program>, modules::ModuleGraph, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut graph = modules::load_with_dependencies(entry, info, deps, &mut diags);
    if diag::has_errors(&diags) || graph.modules.is_empty() {
        // A chain on a name that is no module of its file is an error here too.
        for m in &mut graph.modules {
            resolve::chain_slips_by_imports(&mut m.program, &mut diags);
        }
        return (None, graph, diags);
    }
    let settings: Vec<_> = graph.modules.iter().map(|module| {
        let path = module.path.canonicalize().unwrap_or_else(|_| module.path.clone());
        let project = deps.iter().filter(|dep| path.starts_with(&dep.src))
            .max_by_key(|dep| dep.src.components().count())
            .map_or(behaviour, |dep| dep.config.behaviour);
        // Loading has already validated the flags and reported their diagnostics.
        let flags = flags::parse(&graph.map.source(module.file).text, &mut Vec::new());
        (graph.map.base(module.file), project.for_file(&flags.behaviour))
    }).collect();
    if graph.modules.len() > 1 {
        // Several files: give every name its program-wide meaning, join the
        // files into one program, and check that. The text is the files in
        // order with a newline between, which is how the source map numbers it.
        let mut graph = graph;
        let program = resolve::resolve(&mut graph, &mut diags);
        if diag::has_errors(&diags) {
            return (None, graph, diags);
        }
        let src = (0..graph.map.len()).map(|f| graph.map.source(f).text.as_str()).collect::<Vec<_>>().join("\n");
        let mut checked = typeck::check_with_settings(&program, &src, &mut diags, settings, info.requires_main);
        if diag::has_errors(&diags) {
            return (None, graph, diags);
        }
        locate_asserts(&mut checked, &graph.map);
        return (Some(checked), graph, diags);
    }
    let mut graph = graph;
    resolve::chain_slips_by_imports(&mut graph.modules[0].program, &mut diags);
    if diag::has_errors(&diags) {
        return (None, graph, diags);
    }
    let program = &graph.modules[0].program;
    let src = &graph.map.source(0).text;
    let mut checked = typeck::check_with_settings(program, src, &mut diags, settings, info.requires_main);
    if diag::has_errors(&diags) {
        return (None, graph, diags);
    }
    locate_asserts(&mut checked, &graph.map);
    (Some(checked), graph, diags)
}

/// `check` for a project: `front_project`, then the ownership checks.
pub fn check_project(entry: &std::path::Path, info: &modules::ProjectInfo) -> (Option<hir::Program>, modules::ModuleGraph, Vec<Diagnostic>) {
    { let (checked, graph, diags) = front_project(entry, info); let (checked, diags) = ownership_checks(checked, diags); (checked, graph, diags) }
}


/// Check an opened project with its configured behaviour.
pub fn front_opened(opened: &config::Opened, ownership: bool) -> (Option<hir::Program>, modules::ModuleGraph, Vec<Diagnostic>) {
    let (checked, graph, diags) = front_project_with_settings(
        &opened.entry, &opened.info, opened.config.behaviour, &opened.deps,
    );
    let (checked, diags) = if ownership { ownership_checks(checked, diags) } else { (checked, diags) };
    (checked, graph, diags)
}

/// Build an executable that runs one test: the test's function is its `main`,
/// so the program does nothing else. `drachm test` builds one per test.
pub fn build_test(program: &mut hir::Program, test: hir::FuncId, out: &std::path::Path, optimise: bool) -> Result<(), String> {
    program.main = test;
    build(program, out, optimise)
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
    // Test blocks are checked but not built: only the test that `drachm test`
    // makes `main` has a body here.
    for t in &program.tests {
        if t.func != program.main {
            mir.bodies[t.func] = None;
        }
    }
    if optimise {
        link::compile_c(&cgen_mir::generate(program, &mir), out)
    } else {
        link::link_object(&codegen_mir::compile(program, &mir)?, out)
    }
}
