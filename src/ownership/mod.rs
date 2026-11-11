//! Ownership and borrow analyses that run on the HIR, before MIR is built.
//! See `docs/impl/step3-ownership.md` section 3.7.

pub mod self_mode;

pub use self_mode::{infer, reason_chain, MutKind, MutReason, SelfMode};
