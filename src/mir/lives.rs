//! `lives` on functions (spec 3.6): working out which parameters a returned
//! borrow may come from, checking written `lives` clauses against the body,
//! and the signatures callers use.
//!
//! The call graph is split into strongly connected components, callees first.
//! A component with functions that need inference is solved by repeating the
//! borrow analysis, without reporting, until no signature grows; then one
//! reporting round keeps the diagnostics.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::borrowck::analyse_body;
use super::borrowck::regions::region_slots;
use super::borrowck::solve::{Constraint, ConstraintKind, Solution};
use super::check::source_name;
use super::index::Idx;
use super::*;
use crate::diag::codes;

/// The signature before inference: nothing is known beyond what was written.
pub fn initial(hir: &hir::Program, f: FuncId) -> LivesSig {
    let func = &hir.funcs[f];
    if region_slots(hir, func.ret) == 0 {
        LivesSig::NoBorrowResult
    } else if let Some(v) = &func.lives {
        LivesSig::Declared(v.clone())
    } else {
        LivesSig::Inferred(Vec::new())
    }
}

/// The strongly connected components of a graph given as adjacency lists,
/// callees (successors) first; each component sorted by node.
pub fn sccs(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct T<'a> {
        edges: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(t: &mut T, v: usize) {
        t.index[v] = Some(t.next);
        t.low[v] = t.next;
        t.next += 1;
        t.stack.push(v);
        t.on[v] = true;
        for i in 0..t.edges[v].len() {
            let w = t.edges[v][i];
            match t.index[w] {
                None => {
                    visit(t, w);
                    t.low[v] = t.low[v].min(t.low[w]);
                }
                Some(iw) if t.on[w] => t.low[v] = t.low[v].min(iw),
                _ => {}
            }
        }
        if Some(t.low[v]) == t.index[v] {
            let mut comp = Vec::new();
            while let Some(w) = t.stack.pop() {
                t.on[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            comp.sort();
            t.out.push(comp);
        }
    }
    let n = edges.len();
    let mut t = T { edges, index: vec![None; n], low: vec![0; n], on: vec![false; n], stack: Vec::new(), next: 0, out: Vec::new() };
    for v in 0..n {
        if t.index[v].is_none() {
            visit(&mut t, v);
        }
    }
    t.out
}

/// What the universal-region check finds in one solved body.
#[derive(Default, Debug)]
pub struct Findings {
    /// Parameters the result may borrow from, ascending.
    pub required: Vec<usize>,
    /// Pairs `(stored, into)` of universals: a borrow of one parameter flows
    /// into a slot of another.
    pub crossings: Vec<(usize, usize)>,
}

/// The universal region check (spec 3.6.2).
pub fn findings(sol: &Solution) -> Findings {
    let mut req = BTreeSet::new();
    let mut cross = BTreeSet::new();
    for (u, uni) in sol.universals.iter().enumerate() {
        let Some(p) = uni.param else { continue };
        for v in sol.values[RegionVid::new(u)].ends.iter() {
            if v == u {
                continue;
            }
            match sol.universals[v].param {
                None => {
                    req.insert(p);
                }
                Some(q) if q != p => {
                    // A flow in both directions is an invariant slot: report
                    // it once, as the shallower slot stored into the deeper.
                    let (a, b) = (&sol.universals[u], &sol.universals[v]);
                    if (a.slot, std::cmp::Reverse(p)) <= (b.slot, std::cmp::Reverse(q)) {
                        cross.insert((u, v));
                    }
                }
                _ => {}
            }
        }
    }
    Findings { required: req.into_iter().collect(), crossings: cross.into_iter().collect() }
}

use super::borrowck::solve::RegionVid;

fn loc_span(body: &Body, loc: Location) -> Span {
    let block = &body.blocks[loc.block];
    match block.statements.get(loc.index as usize) {
        Some(s) => s.span,
        None => block.terminator.span,
    }
}

/// The constraints from universal `from` down to a region satisfying `goal`
/// (shortest path along "contains" edges), in order.
fn path<'a>(sol: &'a Solution, from: usize, goal: impl Fn(usize) -> bool) -> Vec<&'a Constraint> {
    let mut prev: BTreeMap<usize, &Constraint> = BTreeMap::new();
    let mut seen = BTreeSet::from([from]);
    let mut q = VecDeque::from([from]);
    while let Some(r) = q.pop_front() {
        if r != from && goal(r) {
            let mut out = Vec::new();
            let mut at = r;
            while at != from {
                let c = prev[&at];
                out.push(c);
                at = c.sup.index();
            }
            out.reverse();
            return out;
        }
        for c in &sol.constraints {
            if c.sup.index() == r && seen.insert(c.sub.index()) {
                prev.insert(c.sub.index(), c);
                q.push_back(c.sub.index());
            }
        }
    }
    Vec::new()
}

struct Cx<'a> {
    hir: &'a hir::Program,
    body: &'a Body,
    sol: &'a Solution,
}

impl Cx<'_> {
    fn param_name(&self, p: usize) -> String {
        self.hir.funcs[self.body.func].locals[p].name.clone()
    }

    /// Labels along a path: where the borrow is returned or stored, and the calls it goes through.
    fn path_labels(&self, edges: &[&Constraint], text: &str, primary: Option<&mut Option<Span>>) -> Vec<(Span, String)> {
        let mut out = Vec::new();
        let mut found = primary;
        for c in edges {
            let span = loc_span(self.body, c.origin.loc);
            match c.origin.kind {
                ConstraintKind::Return => {
                    if let Some(f) = found.as_deref_mut() {
                        f.get_or_insert(span);
                    }
                    out.push((span, text.to_string()));
                }
                ConstraintKind::Call => {
                    if let TerminatorKind::Call { callee: Callee::Func(f), .. } = &self.body.blocks[c.origin.loc.block].terminator.kind
                        && c.origin.loc.index as usize == self.body.blocks[c.origin.loc.block].statements.len()
                    {
                        let (_, name) = source_name(&self.hir.funcs[*f].symbol);
                        let label = (self.body.blocks[c.origin.loc.block].terminator.span, format!("through the call to `{name}` here"));
                        if !out.contains(&label) {
                            out.push(label);
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The Return statement span of the path from parameter universals to the result.
    fn return_path(&self, param: usize) -> Vec<&Constraint> {
        let u = (0..self.sol.universals.len()).find(|&u| {
            self.sol.universals[u].param == Some(param)
                && self.sol.values[RegionVid::new(u)].ends.iter().any(|v| self.sol.universals[v].param.is_none())
        });
        let Some(u) = u else { return Vec::new() };
        path(self.sol, u, |r| self.sol.universals.get(r).is_some_and(|x| x.param.is_none()))
    }
}

fn quote_list(names: &[String], joiner: &str) -> String {
    let q: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    match q.len() {
        0 | 1 => q.join(""),
        _ => format!("{} {joiner} {}", q[..q.len() - 1].join(", "), q[q.len() - 1]),
    }
}

/// Check one solved body against its signature and report what is wrong.
fn report(hir: &hir::Program, body: &Body, sol: &Solution, sig: &LivesSig, out: &mut Vec<Diagnostic>) {
    let func = &hir.funcs[body.func];
    let f = findings(sol);
    let cx = Cx { hir, body, sol };
    let (_, fname) = source_name(&func.symbol);
    let mut seen_pairs = BTreeSet::new();
    for &(u, v) in &f.crossings {
        let stored = sol.universals[u].param.unwrap();
        let into = sol.universals[v].param.unwrap();
        if !seen_pairs.insert((stored, into)) {
            continue;
        }
        let (sn, intn) = (cx.param_name(stored), cx.param_name(into));
        let edges = path(sol, u, |r| r == v);
        let mut primary = None;
        let mut labels = Vec::new();
        if let Some(c) = edges.iter().rev().find(|c| c.origin.kind != ConstraintKind::Liveness).or(edges.last()) {
            let span = loc_span(body, c.origin.loc);
            primary = Some(span);
            labels.push((span, format!("the borrow of `{sn}` is stored in `{intn}` here")));
        }
        let at = primary.unwrap_or(func.name_span);
        let mut d = Diagnostic::at(&codes::E0320, at, format!("cannot store a borrow of `{sn}` inside `{intn}`"))
            .note(labels.first().map_or(String::new(), |l| l.1.clone()))
            .label(func.locals[into].span, format!("`{intn}` can only hold borrows that last as long as the caller's value"))
            .help(format!("the caller cannot promise that `{sn}` lives as long as what `{intn}` points to; store a copy: `copy {sn}`"));
        for (s, t) in cx.path_labels(&edges, "", None).into_iter().filter(|(_, t)| !t.is_empty()) {
            d = d.label(s, t);
        }
        out.push(d);
    }
    let names = |ps: &[usize]| -> Vec<String> { ps.iter().map(|&p| cx.param_name(p)).collect() };
    match sig {
        LivesSig::Declared(declared) => {
            let missing: Vec<usize> = f.required.iter().copied().filter(|p| !declared.contains(p)).collect();
            for p in missing {
                let n = cx.param_name(p);
                let edges = cx.return_path(p);
                let mut ret = None;
                let labels = cx.path_labels(&edges, &format!("returns a borrow of `{n}` here"), Some(&mut ret));
                let at = ret.unwrap_or(func.name_span);
                let declared_names = names(declared);
                let mut d = Diagnostic::at(
                    &codes::E0309,
                    at,
                    format!("`{fname}` returns a borrow of `{n}`, but its `lives` names only {}", quote_list(&declared_names, "and")),
                )
                .note(format!("returns a borrow of `{n}` here"));
                if let Some(ls) = func.lives_span {
                    d = d.label(ls, format!("`lives` names only {} here", quote_list(&declared_names, "and")));
                }
                for (s, t) in labels.into_iter().filter(|(s, _)| *s != at) {
                    d = d.label(s, t);
                }
                let mut all = declared_names.clone();
                all.push(n.clone());
                let from_form = declared.contains(&0) && func.has_self || n == "self";
                let list = if from_form { format!("lives from {}", all.join(" or ")) } else { format!("lives {}", all.join(", ")) };
                d = d.help(format!("add `{n}` to the list: `{list}`, or return a borrow of {}", quote_list(&declared_names, "or")));
                out.push(d);
            }
        }
        LivesSig::Inferred(_) if f.required.len() >= 2 => {
            let ns = names(&f.required);
            let mut d = Diagnostic::at(
                &codes::E0308,
                func.name_span,
                format!("`{fname}` returns a borrow, but which parameter it borrows from depends on the path"),
            )
            .note(format!("the result borrows from {}", quote_list(&ns, "or")));
            for &p in &f.required {
                let n = cx.param_name(p);
                let edges = cx.return_path(p);
                for (s, t) in cx.path_labels(&edges, &format!("returns a borrow of `{n}` here"), None) {
                    d = d.label(s, t);
                }
            }
            let help = if f.required.contains(&0) && func.has_self {
                format!("write `lives from {}` after the parameters", ns.join(" or "))
            } else {
                format!("write `lives {}` after the parameters", ns.join(", "))
            };
            out.push(d.help(help));
        }
        _ => {}
    }
}

fn callees(body: &Body) -> BTreeSet<FuncId> {
    body.blocks
        .iter()
        .filter_map(|b| match &b.terminator.kind {
            TerminatorKind::Call { callee: Callee::Func(f), .. } => Some(*f),
            _ => None,
        })
        .collect()
}

/// Work out every function's signature and append the borrow diagnostics.
/// Bodies come out in `FuncId` order, each sorted by position.
pub fn check_program(hir: &hir::Program, mir: &mut Program, diags: &mut Vec<Diagnostic>) {
    let n = hir.funcs.len();
    let real = |f: FuncId| mir.bodies[f].is_some() && hir.funcs[f].span != Span::new(0, 0);
    let checked: Vec<bool> = (0..n).map(real).collect();
    let graph: Vec<Vec<usize>> = (0..n)
        .map(|f| match (&mir.bodies[f], checked[f]) {
            (Some(b), true) => callees(b).into_iter().filter(|&c| c < n).collect(),
            _ => Vec::new(),
        })
        .collect();
    mir.lives = (0..n).map(|f| initial(hir, f)).collect();
    let mut by_func: Vec<Vec<Diagnostic>> = (0..n).map(|_| Vec::new()).collect();
    for comp in sccs(&graph) {
        let comp: Vec<FuncId> = comp.into_iter().filter(|&f| checked[f]).collect();
        let infer = comp.iter().any(|&f| matches!(mir.lives[f], LivesSig::Inferred(_)));
        if infer {
            let recursive = comp.len() > 1 || graph[comp[0]].contains(&comp[0]);
            loop {
                let mut changed = false;
                for &f in &comp {
                    if !matches!(mir.lives[f], LivesSig::Inferred(_)) {
                        continue;
                    }
                    let body = mir.bodies[f].as_ref().unwrap();
                    let sol = analyse_body(hir, &mir.lives, body, false).solution;
                    let LivesSig::Inferred(old) = &mir.lives[f] else { unreachable!() };
                    let mut set: BTreeSet<usize> = old.iter().copied().collect();
                    set.extend(findings(&sol).required);
                    let new: Vec<usize> = set.into_iter().collect();
                    if &new != old {
                        mir.lives[f] = LivesSig::Inferred(new);
                        changed = true;
                    }
                }
                if !changed || !recursive {
                    break;
                }
            }
        }
        for &f in &comp {
            let body = mir.bodies[f].as_ref().unwrap();
            let res = analyse_body(hir, &mir.lives, body, true);
            let mut ds = res.diags;
            report(hir, body, &res.solution, &mir.lives[f], &mut ds);
            ds.sort_by_key(|d| d.labels.first().map_or(0, |l| l.0.start));
            by_func[f] = ds;
        }
    }
    for ds in by_func {
        diags.extend(ds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_come_callees_first() {
        // 0 -> 1 -> 2 -> 1, 3 -> 0, 4 alone with a self loop.
        let g = vec![vec![1], vec![2], vec![1], vec![0], vec![4]];
        let c = sccs(&g);
        assert_eq!(c, vec![vec![1, 2], vec![0], vec![3], vec![4]]);
    }

    #[test]
    fn a_chain_is_ordered() {
        let c = sccs(&[vec![1], vec![2], vec![]]);
        assert_eq!(c, vec![vec![2], vec![1], vec![0]]);
    }

    fn sigs_of(src: &str) -> (hir::Program, Vec<LivesSig>, Vec<String>) {
        let (hir, d) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("{d:?}"));
        let mut diags = Vec::new();
        let mut mir = crate::mir::build_program(&hir, &mut diags);
        check_program(&hir, &mut mir, &mut diags);
        let codes = diags.iter().map(|d| d.code.id.to_string()).collect();
        let sigs = mir.lives.clone();
        (hir, sigs, codes)
    }

    fn sig(hir: &hir::Program, sigs: &[LivesSig], name: &str) -> LivesSig {
        let f = hir.funcs.iter().position(|f| f.symbol.ends_with(name)).unwrap();
        sigs[f].clone()
    }

    #[test]
    fn inference_and_fixed_point() {
        let (hir, sigs, codes) = sigs_of(
            "&string first(&string a, &string b) lives a { return a; }
&string one(&string a, uint32 n) { return a; }
&string via(&string a, &string b) lives a, b { return first(a, b); }
&string rec(&string a, uint32 n) { if (n == 0) { return a; } return rec(a, n - 1); }
&string ping(&string a, uint32 n) { if (n == 0) { return a; } return pong(a, n - 1); }
&string pong(&string a, uint32 n) { return ping(a, n); }
none main() { }
",
        );
        assert!(codes.is_empty(), "{codes:?}");
        assert_eq!(sig(&hir, &sigs, "one"), LivesSig::Inferred(vec![0]));
        assert_eq!(sig(&hir, &sigs, "rec"), LivesSig::Inferred(vec![0]));
        assert_eq!(sig(&hir, &sigs, "ping"), LivesSig::Inferred(vec![0]));
        assert_eq!(sig(&hir, &sigs, "pong"), LivesSig::Inferred(vec![0]));
        assert_eq!(sig(&hir, &sigs, "first"), LivesSig::Declared(vec![0]));
    }

    #[test]
    fn the_three_errors() {
        let (_, _, c) = sigs_of("&string longest(&string a, &string b) { if (a::len() > b::len()) { return a; } return b; }\nnone main() { }\n");
        assert_eq!(c, ["E0308"]);
        let (_, _, c) = sigs_of("&string pick(&string a, &string b) lives a { return b; }\nnone main() { }\n");
        assert_eq!(c, ["E0309"]);
        let (_, _, c) = sigs_of("struct Parser lives src { public: mut &string src; uint32 pos; }\nnone keep(&mut Parser a, &string b) { a.src = b; }\nnone main() { }\n");
        assert!(c.contains(&"E0320".to_string()), "{c:?}");
    }
}
