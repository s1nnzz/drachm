//! The Drachm compiler as a library. The `drachm` binary, the tests and
//! (later) the language server all drive it through here.

pub mod ast;
pub mod cgen;
pub mod codegen;
pub mod consts;
pub mod diag;
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
    let tokens = lexer::lex(src, &mut diags);
    let program = parser::parse(tokens, &mut diags);
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
    if optimise {
        link::compile_c(&cgen::generate(program), out)
    } else {
        link::link_object(&codegen::compile(program)?, out)
    }
}
