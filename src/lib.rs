//! The Drachm compiler as a library. The `drachm` binary, the tests and
//! (later) the language server all drive it through here.

pub mod ast;
pub mod codegen;
pub mod diag;
pub mod lexer;
pub mod link;
pub mod parser;
pub mod typeck;

use std::collections::HashMap;

use diag::Diagnostic;

/// A program that passed every front-end check.
pub struct Checked {
    pub program: ast::Program,
    pub sigs: HashMap<String, typeck::Sig>,
}

/// Lex, parse and type-check `src`. Returns every diagnostic found; the
/// program is `None` if any of them is an error.
pub fn check(src: &str) -> (Option<Checked>, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let tokens = lexer::lex(src, &mut diags);
    let program = parser::parse(tokens, &mut diags);
    // A program with syntax errors has holes in it; type errors found in what
    // is left would mostly be noise.
    if diag::has_errors(&diags) {
        return (None, diags);
    }
    let sigs = typeck::check(&program, &mut diags);
    if diag::has_errors(&diags) {
        return (None, diags);
    }
    (Some(Checked { program, sigs }), diags)
}

/// Compile a checked program to a native object file.
pub fn compile(checked: &Checked, opt: bool) -> Result<Vec<u8>, String> {
    codegen::compile(&checked.program, &checked.sigs, opt)
}
