//! Self-mode inference (`ownership::infer`, spec 3.7) on HIR from `check`.

use drachm::hir::{self, FuncId};
use drachm::ownership::{infer, reason_chain, MutKind, SelfMode};

const COUNTER: &str = r#"struct Counter {
    uint32 n;
    mut uint32 reads;
    none bump() { self.n = self.n + 1; }
    none bumpTwice() { self::bump(); self::bump(); }
    uint32 get() { self.reads = self.reads + 1; return self.n; }
    none ping(uint32 k) { if (k > 0) { self::pong(k - 1); } }
    none pong(uint32 k) { if (k > 0) { self::ping(k - 1); } }
    uint32 peek() { return self.n; }
    none inc() { self.n++; }
    none drop() { }
}
struct Outer {
    Counter c;
    none go() { self.c::bump(); }
    none look() { uint32 x = self.c::peek(); }
    none other(mut Counter k) { k::bump(); }
}
none main() { }
"#;

const BORROWS: &str = r#"none fill(&mut string s) { }
struct Buf {
    string data;
    none clear() { fill(&mut self.data); }
    none reread() { &string d = &self.data; }
}
none main() { }
"#;

fn checked(src: &str) -> hir::Program {
    let (program, diags) = drachm::check(src);
    let errors: Vec<String> = diags.iter().filter(|d| d.severity == drachm::diag::Severity::Error).map(|d| d.message.clone()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    program.expect("type-checks")
}

/// The `FuncId` of method `method` of struct `strct` (symbol `drachm_m_<len><struct>_<method>`).
fn method(hir: &hir::Program, strct: &str, method: &str) -> FuncId {
    let symbol = format!("drachm_m_{}{}_{method}", strct.len(), strct);
    hir.funcs.iter().position(|f| f.symbol == symbol).unwrap_or_else(|| panic!("no method {symbol}"))
}

fn mut_kind(modes: &[Option<SelfMode>], f: FuncId) -> MutKind {
    match &modes[f] {
        Some(SelfMode::Mut(r)) => r.kind.clone(),
        other => panic!("expected Mut, found {other:?}"),
    }
}

fn reason_span<'a>(src: &'a str, modes: &[Option<SelfMode>], f: FuncId) -> &'a str {
    match &modes[f] {
        Some(SelfMode::Mut(r)) => &src[r.span.start..r.span.end],
        other => panic!("expected Mut, found {other:?}"),
    }
}

#[test]
fn counter_modes() {
    let hir = checked(COUNTER);
    let modes = infer(&hir);
    let bump = method(&hir, "Counter", "bump");
    assert_eq!(mut_kind(&modes, bump), MutKind::Writes("self.n".into()));
    assert_eq!(reason_span(COUNTER, &modes, bump), "self.n");
    let bump_twice = method(&hir, "Counter", "bumpTwice");
    assert_eq!(mut_kind(&modes, bump_twice), MutKind::Calls(bump));
    // The `mut` field is still read-only through a borrow.
    assert_eq!(mut_kind(&modes, method(&hir, "Counter", "get")), MutKind::Writes("self.reads".into()));
    let (ping, pong, peek) = (method(&hir, "Counter", "ping"), method(&hir, "Counter", "pong"), method(&hir, "Counter", "peek"));
    assert_eq!(modes[ping], Some(SelfMode::Shared));
    assert_eq!(modes[pong], Some(SelfMode::Shared));
    assert_eq!(modes[peek], Some(SelfMode::Shared));
    let inc = method(&hir, "Counter", "inc");
    assert_eq!(mut_kind(&modes, inc), MutKind::Writes("self.n".into()));
    assert_eq!(mut_kind(&modes, method(&hir, "Counter", "drop")), MutKind::IsDrop);
    let go = method(&hir, "Outer", "go");
    assert_eq!(mut_kind(&modes, go), MutKind::Calls(bump));
    assert_eq!(modes[method(&hir, "Outer", "look")], Some(SelfMode::Shared));
    assert_eq!(modes[method(&hir, "Outer", "other")], Some(SelfMode::Shared));
    let main = hir.main;
    assert_eq!(modes[main], None);
    let chain = reason_chain(&modes, go);
    let got: Vec<(FuncId, MutKind)> = chain.iter().map(|(f, r)| (*f, r.kind.clone())).collect();
    assert_eq!(got, [(go, MutKind::Calls(bump)), (bump, MutKind::Writes("self.n".into()))]);
    assert_eq!(reason_span(COUNTER, &modes, go), "self.c::bump()");
    assert!(reason_chain(&modes, peek).is_empty());
}

#[test]
fn borrow_forms() {
    let hir = checked(BORROWS);
    let modes = infer(&hir);
    let clear = method(&hir, "Buf", "clear");
    assert_eq!(mut_kind(&modes, clear), MutKind::LendsMut("self.data".into()));
    assert_eq!(modes[method(&hir, "Buf", "reread")], Some(SelfMode::Shared));
}
