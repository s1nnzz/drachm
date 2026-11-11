//! The ownership checks that run on the finished MIR, body by body: moves and
//! initialisation (E0301, E0207, E0313, E0332), moves out of borrows
//! (E0310), copies of `&mut` holders (E0315), a missing `return` (E0105) and
//! the mutability checks of `mutability.rs` (E0205, E0311, E0312).

use super::dataflow::{BitSet, Results};
use super::moves::*;
use super::mutability;
use super::traversal::{in_cycle, predecessors, reverse_postorder};
use super::*;
use crate::diag::codes;

/// Run every ownership analysis that exists so far on every body, appending
/// diagnostics. Order per body: moves/uses, then mutability. Bodies go in
/// `FuncId` order and one body's diagnostics are sorted by position.
pub fn run(hir: &hir::Program, mir: &Program, diags: &mut Vec<Diagnostic>) {
    for body in mir.bodies.iter().flatten() {
        // The compiler's own functions (the automatic `==`) move fields as step 2 did.
        if hir.funcs[body.func].span == Span::new(0, 0) {
            continue;
        }
        let mut ds = Vec::new();
        check_body(hir, body, &mir.self_modes, &mut ds);
        ds.sort_by_key(|d| d.labels.first().map_or(0, |l| l.0.start));
        diags.extend(ds);
    }
}

/// Whether the symbol is a method, and the name as written.
pub fn source_name(symbol: &str) -> (bool, String) {
    if let Some(rest) = symbol.strip_prefix("drachm_m_") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse::<usize>() {
            let after = &rest[digits.len()..];
            if let Some(word) = after.get(n..).and_then(|w| w.strip_prefix('_')) {
                return (true, word.to_string());
            }
        }
        return (true, rest.to_string());
    }
    (false, symbol.strip_prefix("drachm_f_").unwrap_or(symbol).to_string())
}

const THIS_VALUE: &str = "this value";

fn quoted(text: &str) -> String {
    if text == THIS_VALUE { text.to_string() } else { format!("`{text}`") }
}

struct Cx<'a> {
    hir: &'a hir::Program,
    body: &'a Body,
    md: &'a MoveData,
}

impl Cx<'_> {
    fn local_name(&self, l: Local) -> Option<&str> {
        match &self.body.locals[l].kind {
            LocalKind::User { name, .. } | LocalKind::Param { name, .. } => Some(name),
            _ => None,
        }
    }

    /// The user's spelling of a place (`(*p).name` is `p.name`).
    fn place_text(&self, p: &Place) -> String {
        mutability::place_text(self.hir, self.body, p)
    }

    fn path_place(&self, p: MovePathId) -> Place {
        let path = &self.md.paths[p];
        Place { local: path.local, proj: path.fields.iter().map(|&f| Proj::Field(f)).collect() }
    }

    fn path_text(&self, p: MovePathId) -> String {
        self.place_text(&self.path_place(p))
    }

    fn related(&self, a: MovePathId, b: MovePathId) -> bool {
        self.md.is_ancestor(a, b) || self.md.is_ancestor(b, a)
    }
}

/// A move that makes a use fail.
struct Gone {
    path: MovePathId,
    loc: Location,
    span: Span,
    cause: MoveCause,
    /// Earlier in the same statement or call.
    here: bool,
}

fn check_body(hir: &hir::Program, body: &Body, modes: &[Option<SelfMode>], out: &mut Vec<Diagnostic>) {
    let md = MoveData::build(body);
    let cx = Cx { hir, body, md: &md };
    let preds = predecessors(body);
    let rpo = reverse_postorder(body);
    let mut rpo_pos = vec![usize::MAX; body.blocks.len()];
    for (i, b) in rpo.iter().enumerate() {
        rpo_pos[b.index()] = i;
    }

    let mu = Results::compute(MaybeUninit(&md), body, &preds);
    let mi = Results::compute(MaybeInit(&md), body, &preds);
    let rm = Results::compute(ReachingMoves(&md), body, &preds);
    let ever = Results::compute(EverInit(&md), body, &preds);

    let mut reported = BitSet::new(md.paths.len());
    mu.visit(body, |loc, st, _| {
        let mut here: Vec<Gone> = Vec::new();
        for ev in events(body, loc) {
            match ev {
                Event::Use(u) => match u.kind {
                    UseKind::Read | UseKind::Assign => {
                        let m = md.prefix_path(&u.place);
                        let bad = st.contains(m.index())
                            || md.descendants(m).iter().any(|d| st.contains(d.index()))
                            || ancestors(&md, m).any(|a| st.contains(a.index()));
                        let intra = here.iter().any(|g| cx.related(g.path, m));
                        if bad || intra {
                            use_failed(&cx, &mi, &rm, &rpo_pos, loc, &u, m, &here, &mut reported, out);
                        }
                    }
                    UseKind::FieldWriteBase => {
                        if let Some(q) = md.path_of(&u.place) {
                            field_write(&cx, &rm, loc, &u, q, st, &mut reported, out);
                        }
                    }
                },
                Event::Move { place, span, cause } => {
                    if let Some(path) = md.path_of(&place) {
                        if cause.visible() {
                            here.push(Gone { path, loc, span, cause, here: true });
                        }
                    }
                }
                Event::Init(_) | Event::StorageLive(_) => {}
            }
        }
    });

    borrow_moves_and_copies(&cx, out);
    let cycles = in_cycle(body);
    mutability::check(hir, body, &md, &ever, &cycles, modes, out);

    if let Some(end) = body.fallthrough_end {
        if rpo.contains(&end) {
            let (method, name) = source_name(&hir.funcs[body.func].symbol);
            let what = if method { "method" } else { "function" };
            let ret = body.locals[Local(0)].ty;
            out.push(
                Diagnostic::at(&codes::E0105, body.span, format!("{what} `{name}` can finish without returning a value"))
                    .note(format!("promises to return `{ret}`"))
                    .help("add a `return` at the end, or to every branch"),
            );
        }
    }
}

fn ancestors(md: &MoveData, p: MovePathId) -> impl Iterator<Item = MovePathId> + '_ {
    std::iter::successors(md.paths[p].parent, move |&a| md.paths[a].parent)
}

/// The visible moves reaching `loc` whose path is related to `m`, by position.
fn reaching(cx: &Cx, rm: &Results<ReachingMoves>, loc: Location, m: MovePathId, here: &[Gone]) -> Vec<Gone> {
    let st = rm.state_at(cx.body, loc);
    let mut v: Vec<Gone> = st
        .iter()
        .map(|i| &cx.md.moves[MoveOutId::new(i)])
        .filter(|mo| mo.cause.visible() && cx.related(mo.path, m))
        .map(|mo| Gone { path: mo.path, loc: mo.loc, span: mo.span, cause: mo.cause, here: false })
        .collect();
    for g in here {
        if cx.related(g.path, m) {
            v.push(Gone { path: g.path, loc: g.loc, span: g.span, cause: g.cause, here: true });
        }
    }
    v.sort_by_key(|g| (g.span.start, g.span.end));
    v
}

#[allow(clippy::too_many_arguments)]
fn use_failed(
    cx: &Cx,
    mi: &Results<MaybeInit>,
    rm: &Results<ReachingMoves>,
    rpo_pos: &[usize],
    loc: Location,
    u: &Use,
    m: MovePathId,
    here: &[Gone],
    reported: &mut BitSet,
    out: &mut Vec<Diagnostic>,
) {
    let gone = reaching(cx, rm, loc, m, here);
    if gone.is_empty() {
        // Never given a value on some path: E0207.
        if reported.contains(m.index()) {
            return;
        }
        reported.insert(m.index());
        let name = cx.path_text(m);
        let local = &cx.body.locals[cx.md.paths[m].local];
        let maybe = mi.state_at(cx.body, loc).contains(m.index());
        let label = if maybe { "only some paths to here give it one" } else { "nothing has been assigned to it yet" };
        let mut d = Diagnostic::at(&codes::E0207, u.span, format!("{} is read before it certainly has a value", quoted(&name))).note(label);
        if name != THIS_VALUE {
            d = d.label(local.span, "declared here without a value");
            d = d.help(format!("give `{name}` a value on every path before this point"));
        }
        out.push(d);
        return;
    }
    if reported.contains(m.index()) || gone.iter().any(|g| reported.contains(g.path.index())) {
        return;
    }
    reported.insert(m.index());
    gone.iter().for_each(|g| {
        reported.insert(g.path.index());
    });

    let used = cx.path_text(m);
    let first = &gone[0];
    let first_text = cx.path_text(first.path);
    let message = if first.path == m {
        format!("{} is used after it moved", quoted(&used))
    } else {
        format!("{} is used after {} moved", quoted(&used), quoted(&first_text))
    };
    let mut d = Diagnostic::at(&codes::E0301, u.span, message).note("used here, after the move");

    let now_init = mi.state_at(cx.body, loc);
    let is_loop = |g: &Gone| !g.here && (rpo_pos[g.loc.block.index()], g.loc.index) >= (rpo_pos[loc.block.index()], loc.index);
    for g in gone.iter().take(3) {
        let text = cx.path_text(g.path);
        let ty = cx.md.path_ty(cx.body, cx.hir, g.path);
        let mut label = if g.cause == MoveCause::Destroy {
            format!("{} destroyed here", quoted(&text))
        } else if matches!(ty, Type::Ref { mutable: true, .. }) {
            format!("{} (a `&mut` borrow) moved here", quoted(&text))
        } else {
            format!("{} moved here", quoted(&text))
        };
        if is_loop(g) {
            label.push_str(", in the previous time round the loop");
        } else if !g.here && now_init.contains(g.path.index()) {
            label.push_str(", on some paths");
        }
        d = d.label(g.span, label);
    }

    let mty = cx.md.path_ty(cx.body, cx.hir, first.path);
    let partial = first.path != m && cx.md.is_ancestor(m, first.path);
    let help = if is_loop(first) {
        Some(format!("move it only once (for example after the loop), or pass `copy {first_text}`"))
    } else if matches!(mty, Type::Ref { mutable: true, .. }) {
        Some(format!("to keep using `{first_text}` afterwards, pass it on with `lend {first_text}`"))
    } else if partial || first.cause == MoveCause::Take {
        Some(format!("give `{first_text}` a new value first, or move a copy: `copy {first_text}`"))
    } else if let Some(def) = mty.struct_id().map(|id| &cx.hir.structs[id]).filter(|s| s.stays) {
        Some(format!("`{}` is marked `stays`, so it never copies automatically; write `copy {first_text}`", def.name))
    } else {
        move_help(cx, first, &first_text, mty)
    };
    if let Some(h) = help {
        d = d.help(h);
    }
    out.push(d);
}

fn move_help(cx: &Cx, g: &Gone, text: &str, ty: Type) -> Option<String> {
    let block = &cx.body.blocks[g.loc.block];
    match g.cause {
        MoveCause::CallArg { callee, index } => {
            let f = &cx.hir.funcs[callee];
            let (method, name) = source_name(&f.symbol);
            let borrowed = f.locals.get(index).is_some_and(|l| l.ty.is_borrow());
            if borrowed || method {
                return Some(format!("to keep using `{text}` afterwards, pass a copy: `copy {text}`"));
            }
            let single = matches!(&block.terminator.kind, TerminatorKind::Call { args, .. } if args.len() == 1);
            let call = if single { format!("{name}(copy {text})") } else { format!("copy {text}") };
            Some(format!("if `{name}` should not take ownership, give it a `&{ty}` parameter; or pass a copy: `{call}`"))
        }
        MoveCause::Assign => {
            if let Some(StatementKind::Assign(dest, Rvalue::Use(_))) = block.statements.get(g.loc.index as usize).map(|s| &s.kind) {
                if dest.proj.is_empty() {
                    if let Some(name) = cx.local_name(dest.local) {
                        let dty = cx.body.locals[dest.local].ty;
                        return Some(format!("to keep both, write `{dty} {name} = copy {text};`"));
                    }
                }
            }
            Some(format!("to keep both, write `copy {text}`"))
        }
        MoveCause::Destroy => None,
        _ => Some(format!("to keep using `{text}` afterwards, move a copy: `copy {text}`")),
    }
}

/// E0313 / E0332: `q.f = ...` needs the parents of `q.f` to have values.
#[allow(clippy::too_many_arguments)]
fn field_write(
    cx: &Cx,
    rm: &Results<ReachingMoves>,
    loc: Location,
    u: &Use,
    q: MovePathId,
    mu: &BitSet,
    reported: &mut BitSet,
    out: &mut Vec<Diagnostic>,
) {
    let Some(parent) = cx.md.paths[q].parent else { return };
    // The highest of the parents that has no value.
    let Some(off) = std::iter::once(parent).chain(ancestors(cx.md, parent)).filter(|a| mu.contains(a.index())).last() else {
        return;
    };
    if reported.contains(off.index()) || reported.contains(q.index()) {
        return;
    }
    reported.insert(off.index());
    reported.insert(q.index());
    let target = cx.path_text(q);
    let off_text = cx.path_text(off);
    let gone = reaching(cx, rm, loc, off, &[]);
    if let Some(first) = gone.first() {
        let mut d = Diagnostic::at(&codes::E0313, u.span, format!("cannot give `{target}` a value: {} has moved", quoted(&off_text)))
            .note("assigned here");
        d = d.label(first.span, format!("{} moved here", quoted(&cx.path_text(first.path))));
        out.push(d.help(format!("give `{off_text}` a whole new value instead: `{off_text} = ...;`")));
    } else {
        let local = &cx.body.locals[cx.md.paths[off].local];
        let mut d = Diagnostic::at(&codes::E0332, u.span, format!("cannot give `{target}` a value: {} has no value yet", quoted(&off_text)))
            .note("assigned here");
        if cx.local_name(cx.md.paths[off].local).is_some() {
            d = d.label(local.span, format!("`{off_text}` is declared here without a value"));
        }
        d = d.note_line(format!("a struct with no value is filled with `fill {off_text} {{ ... }}` (not supported yet)"));
        out.push(d.help(format!("give `{off_text}` a whole value first: `{off_text} = ...;`")));
    }
}

/// Every operand of the statement or terminator at `loc`.
fn operands_at(body: &Body, loc: Location) -> Vec<(&Operand, Span)> {
    let block = &body.blocks[loc.block];
    let mut v = Vec::new();
    match block.statements.get(loc.index as usize) {
        Some(s) => {
            if let StatementKind::Assign(_, rv) = &s.kind {
                match rv {
                    Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) => v.push((o, s.span)),
                    Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => v.extend([(a, s.span), (b, s.span)]),
                    Rvalue::Shift { value, amount, .. } => v.extend([(value, s.span), (amount, s.span)]),
                    Rvalue::Aggregate(_, ops) => v.extend(ops.iter().map(|o| (o, s.span))),
                    Rvalue::Ref(..) | Rvalue::RawRef(_) | Rvalue::DeepCopy(_) => {}
                }
            }
        }
        None => match &block.terminator.kind {
            TerminatorKind::If { cond, .. } => v.push((cond, block.terminator.span)),
            TerminatorKind::Switch { discr, .. } => v.push((discr, block.terminator.span)),
            TerminatorKind::Call { args, .. } => v.extend(args.iter().map(|a| (&a.op, a.span))),
            _ => {}
        },
    }
    v
}

/// E0310 (moving out through a borrow or pointer) and E0315 (copying a `&mut` holder).
fn borrow_moves_and_copies(cx: &Cx, out: &mut Vec<Diagnostic>) {
    let body = cx.body;
    for (b, block) in body.blocks.iter_enumerated() {
        for index in 0..=block.statements.len() {
            let loc = Location { block: b, index: index as u32 };
            let returning = matches!(block.statements.get(index).map(|s| &s.kind),
                Some(StatementKind::Assign(d, _)) if d.local == Local(0) && d.proj.is_empty());
            for (op, span) in operands_at(body, loc) {
                if let Operand::Move(p) = op {
                    if p.proj.contains(&Proj::Deref) {
                        out.push(move_through_deref(cx, p, span, returning));
                    }
                }
            }
            if let Some(StatementKind::Assign(dest, Rvalue::DeepCopy(p))) = block.statements.get(index).map(|s| &s.kind) {
                if let Ok(ty) = ty::place_ty(body, cx.hir, p) {
                    if holds_mut_borrow(cx.hir, ty) {
                        out.push(copy_of_mut_holder(cx, block, index, dest, p, ty));
                    }
                }
            }
        }
    }
}

fn move_through_deref(cx: &Cx, p: &Place, span: Span, returning: bool) -> Diagnostic {
    let body = cx.body;
    let first = p.proj.iter().position(|x| *x == Proj::Deref).unwrap();
    let mut t = body.locals[p.local].ty;
    for pr in &p.proj[..first] {
        if let Proj::Field(f) = pr {
            t = cx.hir.struct_def(t).fields[*f].ty;
        }
    }
    let text = cx.place_text(p);
    let (_, func) = source_name(&cx.hir.funcs[body.func].symbol);
    let local = &body.locals[p.local];
    let is_self = cx.local_name(p.local) == Some("self");
    if matches!(t, Type::Ptr { .. }) {
        return Diagnostic::at(&codes::E0310, span, format!("cannot move {} out through a raw pointer", quoted(&text)))
            .note("this would move out of a value the pointer does not own")
            .help(copy_help(&text, returning));
    }
    let mut d = Diagnostic::at(&codes::E0310, span, format!("cannot move {} out of a borrow", quoted(&text)))
        .note(format!("this would move {} out of a value `{func}` does not own", quoted(&text)));
    if is_self {
        d = d.label(local.span, format!("`self` is borrowed by `{func}`"));
        let help = if returning {
            format!("`self` is borrowed; write `return copy {text};`")
        } else {
            format!("`self` is borrowed; write `copy {text}`")
        };
        return d.help(help);
    }
    if let Some(name) = cx.local_name(p.local) {
        let kind = if matches!(t, Type::Ref { mutable: true, .. }) { "`&mut`" } else { "`&`" };
        d = d.label(local.span, format!("`{name}` is a {kind} borrow"));
    }
    d.help(copy_help(&text, returning))
}

fn copy_help(text: &str, returning: bool) -> String {
    if text == THIS_VALUE {
        return "make a copy with `copy` instead of moving the value out".to_string();
    }
    if returning { format!("return a copy: `return copy {text};`") } else { format!("write `copy {text}` to keep the original in place") }
}

fn copy_of_mut_holder(cx: &Cx, block: &BasicBlock, index: usize, dest: &Place, p: &Place, ty: Type) -> Diagnostic {
    let text = cx.place_text(p);
    let span = block.statements[index].span;
    let (strukt, field, fty) = find_mut_borrow(cx.hir, ty).unwrap();
    // The user's variable receiving the copy: the destination, or the local the temp is moved into next.
    let mut target = dest.clone();
    if cx.local_name(target.local).is_none() {
        for s in &block.statements[index + 1..] {
            if let StatementKind::Assign(d, Rvalue::Use(Operand::Move(src))) = &s.kind {
                if src.local == dest.local && src.proj.is_empty() && cx.local_name(d.local).is_some() {
                    target = d.clone();
                    break;
                }
            }
        }
    }
    let help = match cx.local_name(target.local) {
        Some(name) if target.proj.is_empty() => format!("borrow `{text}` instead: `&{ty} {name} = &{text};`"),
        _ => format!("borrow `{text}` instead: `&{text}`"),
    };
    Diagnostic::at(&codes::E0315, span, format!("cannot copy `{text}`: it holds a `&mut` borrow"))
        .note("copy attempted here")
        .note_line(format!("`{strukt}.{field}` is `{fty}`; a copy would be a second `&mut` to the same {}", fty.target().map_or("value".to_string(), |t| t.to_string())))
        .help(help)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_on(src: &str) -> Vec<Diagnostic> {
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mut d = Vec::new();
        let mir = crate::mir::build_program(&hir, &mut d);
        assert!(d.is_empty(), "builder: {d:?}");
        let mut out = Vec::new();
        run(&hir, &mir, &mut out);
        out
    }

    fn assert_clean(src: &str) {
        let out = run_on(src);
        assert!(out.is_empty(), "unexpected: {:?}", out.iter().map(|d| (d.code.id, &d.message)).collect::<Vec<_>>());
    }

    #[test]
    fn reassigning_after_a_move_is_fine() {
        assert_clean(
            "none consume(string s) { }
none main() {
    mut string s = \"a\";
    consume(s);
    s = \"b\";
    consume(s);
}
",
        );
    }

    #[test]
    fn take_leaving_keeps_the_field_usable() {
        assert_clean(
            "struct P { public: string name; string title; }
none consume(string s) { }
none main() {
    mut P p = \"a\", \"b\";
    consume(take p.name leaving \"x\");
    consume(copy p.name);
    P q = p;
}
",
        );
    }

    #[test]
    fn partial_take_keeps_other_fields_usable() {
        assert_clean(
            "struct P { public: string name; string title; }
none consume(string s) { }
none main() {
    mut P p = \"a\", \"b\";
    consume(take p.name);
    consume(copy p.title);
    p.name = \"again\";
    P q = p;
}
",
        );
    }

    #[test]
    fn a_loop_may_move_what_it_declares() {
        assert_clean(
            "none consume(string s) { }
none main() {
    mut uint32 i = 0;
    while (i < 3) {
        string s = \"x\";
        consume(s);
        i++;
    }
}
",
        );
    }

    #[test]
    fn a_moving_branch_that_returns_does_not_reach_the_next_use() {
        assert_clean(
            "none consume(string s) { }
bool flip() { return true; }
none main() {
    string s = \"x\";
    if (flip()) {
        consume(s);
        return;
    }
    consume(s);
}
",
        );
    }

    #[test]
    fn dropping_before_assigning_a_moved_string_is_fine() {
        assert_clean(
            "none consume(string s) { }
none main() {
    mut string s = \"a\";
    consume(s);
    s = \"b\";
    s = \"c\";
}
",
        );
    }

    #[test]
    fn a_use_after_a_move_is_reported_once() {
        let out = run_on(
            "none consume(string s) { }
none main() {
    string s = \"a\";
    consume(s);
    consume(s);
    consume(s);
}
",
        );
        let ids: Vec<&str> = out.iter().map(|d| d.code.id).collect();
        assert_eq!(ids, ["E0301"]);
    }

    #[test]
    fn method_names_come_back_from_symbols() {
        assert_eq!(source_name("drachm_f_main"), (false, "main".to_string()));
        assert_eq!(source_name("drachm_m_6Player_nameOf"), (true, "nameOf".to_string()));
    }
}
