//! The Drachm compiler as a library. The `drachm` binary, the tests and
//! (later) the language server all drive it through here.

pub mod ast;
pub mod cgen;
pub mod cgen_mir;
pub mod codegen;
pub mod codegen_mir;
pub mod consts;
pub mod diag;
pub mod flags;
pub mod hir;
pub mod lexer;
pub mod link;
pub mod mir;
pub mod parser;
pub mod typeck;
pub mod types;

use diag::Diagnostic;

/// Lex, parse and type-check `src`. Returns every diagnostic found (warnings
/// included); the program is `None` if any of them is an error.
pub fn check(src: &str) -> (Option<hir::Program>, Vec<Diagnostic>) {
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

/// Build an executable at `out`. Plain builds go through Cranelift, which is
/// fast to compile; optimised builds go through C, which is fast to run.
pub fn build(program: &hir::Program, out: &std::path::Path, optimise: bool) -> Result<(), String> {
    if program.uses_borrows {
        return Err("borrows and pointers cannot be built yet (step 3 is in progress)".to_string());
    }
    build_unchecked(program, out, optimise)
}

/// `build` without the gate on user-written borrows and pointers (which have
/// no borrow checker yet). Tests only. Such programs can only be built from
/// MIR, so one the MIR backends do not accept yet is an error.
///
/// Programs `mir::cleanup::eligible` accepts are built from MIR; the others
/// still go through the HIR backends (WP6 removes them). The environment
/// variable `DRACHM_BACKEND` overrides the choice: `hir` always uses the HIR
/// backends, `mir` makes a program the MIR backends cannot build an error
/// instead of falling back.
pub fn build_unchecked(program: &hir::Program, out: &std::path::Path, optimise: bool) -> Result<(), String> {
    let backend = std::env::var("DRACHM_BACKEND").unwrap_or_default();
    if backend != "hir" {
        // The errors, if any, were reported by `check`.
        let mut mir = mir::build_program(program, &mut Vec::new());
        mir::cleanup::cleanup_program(program, &mut mir);
        for body in mir.bodies.iter_mut().flatten() {
            mir::analysis::mark_in_memory(program, body);
        }
        match mir::cleanup::ineligible_reason(program, &mir) {
            None => {
                return if optimise {
                    link::compile_c(&cgen_mir::generate(program, &mir), out)
                } else {
                    link::link_object(&codegen_mir::compile(program, &mir)?, out)
                };
            }
            Some(why) if backend == "mir" || program.uses_borrows => {
                return Err(format!("not supported by the MIR backend yet: {why}"));
            }
            Some(_) => {}
        }
    }
    if optimise {
        link::compile_c(&cgen::generate(program), out)
    } else {
        link::link_object(&codegen::compile(program)?, out)
    }
}
