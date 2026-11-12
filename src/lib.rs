//! The Drachm compiler as a library. The `drachm` binary, the tests and
//! (later) the language server all drive it through here.

pub mod ast;
pub mod cgen_common;
pub mod cgen_mir;
pub mod codegen_common;
pub mod codegen_mir;
pub mod consts;
pub mod diag;
pub mod flags;
pub mod hir;
pub mod lexer;
pub mod link;
pub mod mir;
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
    let (checked, mut diags) = front(src);
    let Some(checked) = checked else { return (None, diags) };
    // The ownership checks run on the MIR.
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
