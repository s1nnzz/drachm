//! Facts the backends need that the builder does not record.

use super::*;

/// Decide which locals must live in memory (`LocalDecl.in_memory`): struct and
/// string locals, and every local whose address is taken by a borrow or a raw
/// reference. A place that goes through a `Deref` reads a pointer instead of
/// taking the local's address, so it does not count. Scalars never borrowed
/// stay `false`, which lets the backends keep them in registers.
pub fn mark_in_memory(hir: &hir::Program, body: &mut Body) {
    let _ = hir;
    let mut borrowed = vec![false; body.locals.len()];
    for block in body.blocks.iter() {
        for s in &block.statements {
            if let StatementKind::Assign(_, Rvalue::Ref(_, p) | Rvalue::RawRef(p)) = &s.kind
                && !p.proj.contains(&Proj::Deref)
            {
                borrowed[p.local.index()] = true;
            }
        }
    }
    for (l, d) in body.locals.iter_mut().enumerate() {
        d.in_memory = borrowed[l] || matches!(d.ty, Type::Str | Type::Struct { .. });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_scalars_and_structs_live_in_memory() {
        let src = "in io;
struct P { public: uint32 x; }
none bump(&mut uint32 n) { n = n + 1; }
none main() {
    mut uint32 a = 1;
    mut uint32 b = 2;
    bump(&mut a);
    P p = 3;
    io::outln((a + b + p.x)::text());
}
";
        let (hir, diags) = crate::check(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mut mir = build_program(&hir, &mut Vec::new());
        let body = mir.bodies.iter_mut().flatten().find(|b| b.name == "main").unwrap();
        mark_in_memory(&hir, body);
        let find = |body: &Body, name: &str| {
            body.locals
                .iter()
                .find(|d| matches!(&d.kind, LocalKind::User { name: n, .. } if n == name))
                .unwrap_or_else(|| panic!("no local {name}"))
                .in_memory
        };
        assert!(find(body, "a"), "a borrowed scalar is in memory");
        assert!(!find(body, "b"), "an unborrowed scalar is not");
        assert!(find(body, "p"), "a struct is");
        let bump = mir.bodies.iter_mut().flatten().find(|b| b.name == "bump").unwrap();
        mark_in_memory(&hir, bump);
        assert!(!bump.locals[Local(1)].in_memory, "a reference parameter is a plain pointer");
    }
}
