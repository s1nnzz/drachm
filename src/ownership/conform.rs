//! Struct methods against the interface methods they stand for: an interface
//! method that does not say `changes self` promises callers holding a shared
//! borrow that nothing changes, so the struct's method may not change `self`.
//! A method that writes only `mut` fields counts as changing: the interface
//! cannot promise less than what a `mut` field allows.

use super::self_mode::{MutKind, SelfMode};
use crate::diag::{codes, Diagnostic};
use crate::hir;

/// E0612 for each conforming method whose inferred mode changes `self`.
pub fn check(hir: &hir::Program, modes: &[Option<SelfMode>], diags: &mut Vec<Diagnostic>) {
    for c in &hir.conformances {
        let Some(Some(SelfMode::Mut(reason))) = modes.get(c.func) else { continue };
        let how = match &reason.kind {
            MutKind::IsDrop => continue,
            MutKind::Writes(place) => format!("changes `{place}` here"),
            MutKind::LendsMut(place) => format!("lends `{place}` to be changed here"),
            MutKind::Calls(_) => "calls a method that changes `self` here".to_string(),
        };
        let mut d = Diagnostic::at(&codes::E0612, reason.span, format!("`{}` changes `self`, but `{}` does not say `changes self`", c.method, c.promised)).note(how);
        if let Some(at) = c.promised_at {
            d = d.label(at, format!("`{}` promises not to change `self`", c.promised));
        }
        d = if c.signature.is_empty() {
            d.help("make the method stop changing `self`")
        } else {
            d.help(format!("add `changes self` to `{}`: `{} changes self;`, or stop changing `self`", c.promised, c.signature))
        };
        diags.push(d);
    }
}
