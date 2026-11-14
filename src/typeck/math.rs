//! stub
use super::{Checker, Val};
use crate::ast;
use crate::diag::Span;

impl Checker<'_> {
    pub(super) fn math_call(&mut self, _name: &str, _targs: &[ast::TypeName], _args: &[ast::Expr], _span: Span, _recv: Option<Val>) -> Option<Val> {
        None
    }
}
