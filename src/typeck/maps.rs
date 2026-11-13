//! Maps (S6D): `Map<K, V>` and `OrderedMap<K, V>`, hash tables of owned keys
//! and values. The runtime is `runtime/lists.h` (`drachm_map_*`).
//!
//! A key is
//! a whole number, a float (hashed and compared by its bits, Q14), a `bool` or
//! a `string`. Other key types wait (E0901) until their hashing is built.

use super::{Checker, Globals, MethodInfo, PlaceInfo, StructInfo, Val, optionals};
use crate::ast::{self, BinOp};
use crate::diag::{Diagnostic, Span, codes};
use crate::hir;
use crate::types::Type;
use std::collections::HashMap;

/// A constant key, for finding a repeated one in a literal.
#[derive(PartialEq, Eq, Hash)]
enum Constant {
    Int(u128),
    Float(u64),
    Text(String),
    Bool(bool),
}

pub(super) fn is_map_name(name: &str) -> bool {
    matches!(name, "Map" | "OrderedMap")
}

impl Globals {
    /// `Map<key, val>`, noted for the backends.
    pub(super) fn make_map(&self, key: Type, val: Type, ordered: bool) -> Type {
        let t = Type::map(key, val, ordered);
        self.note_collection(t);
        t
    }
}

/// Checks that wait for every method to be declared (a composite key is known
/// to be hashable only once its `key()` and `==` are).
#[derive(Default)]
pub(super) struct KeyChecks {
    pending: std::cell::RefCell<Vec<(Type, Span)>>,
    ready: std::cell::Cell<bool>,
}

/// The `key()` method of a struct, if it has one of the right shape: not
/// `static`, no parameters, a result, and no failure.
pub(super) fn key_method(g: &Globals, s: &StructInfo) -> Option<MethodInfo> {
    let m = s.methods.iter().find(|m| m.name == "key" && m.operator.is_none())?;
    let sig = &g.sigs[m.func];
    (!m.is_static && sig.params.is_empty() && sig.ret.is_some_and(|r| r != Type::None) && sig.outcome.is_none()).then(|| m.clone())
}

/// Why a type is not a key, and where in it.
struct Unhashable {
    /// The path to the part, `Pos.tags`; the type's own name for the key itself.
    path: String,
    /// The part itself.
    part: Type,
    why: Why,
    /// The span of a method to point at.
    method: Option<Span>,
}

enum Why {
    /// Never hashed: lists, maps, borrows, pointers, failures.
    Kind { holder: Option<Type> },
    /// Has its own `==` and no `key()`.
    EqNoKey,
    /// `key()` has the wrong shape.
    KeyShape(String),
}

/// Run the key checks that were waiting for the methods, and stop waiting.
pub(super) fn check_pending_keys(g: &Globals, diags: &mut Vec<Diagnostic>) {
    g.keys.ready.set(true);
    let pending = std::mem::take(&mut *g.keys.pending.borrow_mut());
    for (key, at) in pending {
        if let Err(d) = g.check_key_now(key, at) {
            diags.push(d);
        }
    }
}

impl Globals {
    /// Can a value of type `key` be a map key? `Err` is the diagnostic to
    /// report. A composite key waits (`Ok`) until the methods are declared.
    pub(super) fn check_key(&self, key: Type, at: Span) -> Result<(), Diagnostic> {
        if !self.keys.ready.get() && matches!(key.bare(), Type::Struct { .. } | Type::Enum { .. } | Type::Array { .. }) {
            self.keys.pending.borrow_mut().push((key, at));
            return Ok(());
        }
        self.check_key_now(key, at)
    }

    fn check_key_now(&self, key: Type, at: Span) -> Result<(), Diagnostic> {
        let name = key.to_string();
        let Some(bad) = self.unhashable(key, name.clone(), &mut Vec::new()) else { return Ok(()) };
        let whole = bad.path == name;
        let part = bad.part;
        let mut d = match bad.why {
            Why::Kind { .. } if whole => Diagnostic::at(&codes::E0626, at, format!("`{key}` cannot be a map key"))
                .note(format!("`{key}` is not hashed"))
                .help("use a whole number, a float, `bool` or `string` as the key, or a struct or enum made of those"),
            Why::Kind { holder } => Diagnostic::at(&codes::E0626, at, format!("`{key}` cannot be a map key: `{}` is a `{part}`", bad.path))
                .note(format!("`{part}` is not hashed"))
                .help(match holder {
                    Some(h) => format!("give `{h}` a `key()` returning a whole number or text, which a map hashes instead"),
                    None => "use a whole number, a float, `bool` or `string` instead, or a struct that has a `key()`".to_string(),
                }),
            Why::EqNoKey if whole => Diagnostic::at(&codes::E0627, at, format!("`{key}` defines `==` but no `key()`, so a map cannot know how to hash it"))
                .help(format!("add a `key()` to `{key}` that returns what makes two of them equal: `string key() {{ ... }}`")),
            Why::EqNoKey => Diagnostic::at(&codes::E0627, at, format!("`{key}` cannot be a map key: `{}` is a `{part}`, which defines `==` but no `key()`", bad.path))
                .note("a map cannot know how to hash it")
                .help(format!("add a `key()` to `{part}` that returns what makes two of them equal")),
            Why::KeyShape(what) => Diagnostic::at(&codes::E0628, at, format!("the `key()` of `{part}` {what}"))
                .help("write it as `string key() { ... }`: no parameters, not `static`, and a result a map can hash"),
        };
        if let Some(m) = bad.method {
            d = d.label(m, "this one");
        }
        Err(d)
    }

    /// The first part of `t` that cannot be hashed, with the path to it.
    /// `seen` holds the types being walked, so a type that holds itself ends.
    fn unhashable(&self, t: Type, path: String, seen: &mut Vec<Type>) -> Option<Unhashable> {
        let t = t.bare();
        let holder = seen.iter().rev().copied().find(|s| matches!(s, Type::Struct { .. }));
        let kind = |path: String| Some(Unhashable { path, part: t, why: Why::Kind { holder }, method: None });
        match t {
            Type::Int { .. } | Type::Float(_) | Type::Bool | Type::Str => None,
            Type::Struct { id, .. } => {
                if seen.contains(&t) {
                    return None;
                }
                let s = &self.structs[id];
                if let Some(m) = s.methods.iter().find(|m| m.name == "key" && m.operator.is_none()) {
                    let sig = &self.sigs[m.func];
                    let wrong = |what: &str| Some(Unhashable { path: path.clone(), part: t, why: Why::KeyShape(what.into()), method: Some(sig.span) });
                    if m.is_static {
                        return wrong("must not be `static`");
                    }
                    if !sig.params.is_empty() {
                        return wrong("must take no parameters");
                    }
                    if sig.outcome.is_some() {
                        return wrong("must not fail");
                    }
                    let Some(ret) = sig.ret.filter(|r| *r != Type::None) else { return wrong("must return a value") };
                    seen.push(t);
                    let bad = self.unhashable(ret, ret.to_string(), seen);
                    seen.pop();
                    return bad.map(|_| Unhashable { path, part: t, why: Why::KeyShape(format!("returns `{ret}`, which a map cannot hash")), method: Some(sig.span) });
                }
                if let Some(op) = self.operator(id, BinOp::Eq) {
                    return Some(Unhashable { path, part: t, why: Why::EqNoKey, method: Some(self.sigs[op.func].span) });
                }
                seen.push(t);
                let found = s.fields.iter().filter_map(|f| f.ty.map(|ft| (f, ft))).find_map(|(f, ft)| self.unhashable(ft, format!("{path}.{}", f.name), seen));
                seen.pop();
                found
            }
            Type::Enum { id, .. } => {
                if seen.contains(&t) {
                    return None;
                }
                let e = &self.enums[id];
                if e.kind == hir::EnumKind::Failure {
                    return kind(path);
                }
                seen.push(t);
                let optional = matches!(e.kind, hir::EnumKind::Optional(_));
                let found = e.variants.iter().find_map(|v| {
                    v.fields.iter().filter_map(|f| f.ty.map(|ft| (f, ft))).find_map(|(f, ft)| {
                        let at = if optional { format!("{path}.value") } else { format!("{path}::{}.{}", v.name, f.name) };
                        self.unhashable(ft, at, seen)
                    })
                });
                seen.pop();
                found
            }
            Type::Array { elem, .. } => self.unhashable(*elem, format!("{path}[]"), seen),
            _ => kind(path),
        }
    }
}

impl Globals {
    /// `Map<K, V>` or `OrderedMap<K, V>` as written (the arguments resolved
    /// here; the `?`, `&` and `*` marks are applied by the caller).
    pub(super) fn resolve_map(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let ordered = t.name == "OrderedMap";
        let key = self.resolve(&t.args[0], diags)?;
        let val = self.resolve(&t.args[1], diags)?;
        let at = t.args[0].span;
        if let Err(d) = self.check_key(key, at) {
            diags.push(d);
            return None;
        }
        if let Err(d) = self.check_map_value(val, t.args[1].span, &t.name) {
            diags.push(d);
            return None;
        }
        Some(self.make_map(key, val, ordered))
    }

    /// Map storage owns its entries. Check even when resolving quietly for
    /// generic arguments, before recording a type for the backends.
    pub(super) fn check_map_value(&self, val: Type, at: Span, map_name: &str) -> Result<(), Diagnostic> {
        if val == Type::None {
            return Err(Diagnostic::at(&codes::E0104, at, "a map cannot hold `none`").note("`none` holds no value"));
        }
        if self.holds_borrow(val) {
            return Err(
                Diagnostic::at(&codes::E0901, at, format!("a `{map_name}` of borrows is not supported yet"))
                    .note("this map would hold a borrow")
                    .help("keep the values themselves in the map, and borrow them where you use them"),
            );
        }
        Ok(())
    }
}

impl Checker<'_> {
    /// `{ k: v, ... }`. With a map type wanted (`to`) the pairs convert to its
    /// key and value types; without one the first pair decides, and `{}` has
    /// nothing to say what it holds (E0624).
    pub(super) fn map_literal(&mut self, pairs: &[(ast::Expr, ast::Expr)], to: Option<Type>, span: Span) -> Option<hir::Expr> {
        let want = match to {
            Some(t) if t.is_map() => t,
            Some(t) => {
                self.diags.push(Diagnostic::at(&codes::E0101, span, format!("expected `{t}`, found a map")).note("a map literal here"));
                for (k, v) in pairs {
                    self.expr(k);
                    self.expr(v);
                }
                return None;
            }
            None => {
                let Some((k0, v0)) = pairs.first() else {
                    self.diags.push(
                        Diagnostic::at(&codes::E0624, span, "cannot tell what this empty map holds")
                            .note("there is no entry and no type to take it from")
                            .help("give it a type: `Map<string, uint32> ages = {};`"),
                    );
                    return None;
                };
                let kv = self.expr(k0)?;
                let k = self.materialise(kv, k0.span)?;
                let vv = self.expr(v0)?;
                let v = self.materialise(vv, v0.span)?;
                let (kt, vt) = (k.ty, v.ty);
                if let Err(d) = self.g.check_map_value(vt, v0.span, "Map") {
                    self.diags.push(d);
                    return None;
                }
                if let Err(d) = self.g.check_key(kt, k0.span) {
                    self.diags.push(d);
                    return None;
                }
                let want = self.g.make_map(kt, vt, false);
                let mut out = vec![(k, v)];
                for (k, v) in &pairs[1..] {
                    let k = self.coerce(k, kt);
                    let v = self.coerce(v, vt);
                    out.push((k?, v?));
                }
                return self.finish_map_literal(out, pairs, want, span);
            }
        };
        let (kt, vt) = want.map_parts()?;
        if let Err(d) = self.g.check_key(kt, pairs.first().map_or(span, |p| p.0.span)) {
            self.diags.push(d);
            return None;
        }
        let mut out = Vec::new();
        let mut ok = true;
        for (k, v) in pairs {
            match (self.coerce(k, kt), self.coerce(v, vt)) {
                (Some(k), Some(v)) => out.push((k, v)),
                _ => ok = false,
            }
        }
        if !ok {
            return None;
        }
        self.finish_map_literal(out, pairs, want, span)
    }

    /// The literal's pairs in order, with E0629 for a constant key written twice.
    fn finish_map_literal(&mut self, out: Vec<(hir::Expr, hir::Expr)>, pairs: &[(ast::Expr, ast::Expr)], want: Type, span: Span) -> Option<hir::Expr> {
        let mut seen = HashMap::new();
        for ((k, _), (ast_k, _)) in out.iter().zip(pairs) {
            let constant = match &k.kind {
                hir::ExprKind::Int(n) => Some(Constant::Int(*n)),
                hir::ExprKind::Float(n) => Some(Constant::Float(n.to_bits())),
                hir::ExprKind::Str(s) => Some(Constant::Text(s.clone())),
                hir::ExprKind::Bool(b) => Some(Constant::Bool(*b)),
                _ => None,
            };
            let Some(c) = constant else { continue };
            if let Some(first) = seen.insert(c, ast_k.span) {
                let shown = self.text(ast_k.span);
                self.diags.push(
                    Diagnostic::at(&codes::E0629, ast_k.span, format!("this map literal has the key {shown} twice"))
                        .label(first, "the first one is here")
                        .help("remove one of them, or the later value would replace the first"),
                );
                return None;
            }
        }
        Some(hir::Expr::new(hir::ExprKind::Map(out), want).at(span))
    }

    /// `map[key]` as a value: `V?`, the value under `key` if there is one.
    pub(super) fn map_index(&mut self, base: hir::Expr, index: &ast::Expr) -> Option<Val> {
        let (kt, vt) = base.ty.map_parts()?;
        let key = self.coerce(index, kt)?;
        let result = optionals::optional_of(self.g, vt)?;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::MapCall { op: hir::MapOp::Get, args: vec![base, key] }, result)))
    }

    /// `map::get(k)`, `map::set(k, v)`, `map::remove(k)`, `map::len()`.
    pub(super) fn map_method(&mut self, v: hir::Expr, recv: &ast::Expr, name: &str, name_span: Span, args: &[ast::Expr]) -> Option<Val> {
        use hir::MapOp;
        let ty = v.ty;
        let (kt, vt) = ty.map_parts()?;
        let (op, takes, result) = match name {
            "len" => (MapOp::Len, 0, Type::U64),
            "get" => (MapOp::Get, 1, optionals::optional_of(self.g, vt)?),
            "set" => (MapOp::Set, 2, Type::None),
            "remove" => (MapOp::Remove, 1, Type::None),
            "each" => {
                self.diags.push(Diagnostic::at(&codes::E0901, name_span, "walking a map with `each` is not supported yet"));
                return None;
            }
            _ => {
                for a in args {
                    self.expr(a);
                }
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no method `{name}`"))
                        .label(recv.span, format!("this is `{ty}`"))
                        .note("other map methods are not decided yet")
                        .help("the methods so far are `len()`, `get(key)`, `set(key, value)` and `remove(key)`"),
                );
                return None;
            }
        };
        if args.len() != takes {
            let s = if takes == 1 { "" } else { "s" };
            self.diags.push(
                Diagnostic::at(&codes::E0103, name_span, format!("`{name}` takes {takes} argument{s}, but {} given", args.len()))
                    .note(format!("takes {takes} argument{s}")),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let mut all = vec![v];
        match op {
            MapOp::Len => {}
            MapOp::Set => {
                all.push(self.coerce(&args[0], kt)?);
                all.push(self.coerce(&args[1], vt)?);
            }
            MapOp::Get | MapOp::Remove => all.push(self.coerce(&args[0], kt)?),
            MapOp::Expect | MapOp::More | MapOp::KeyAt | MapOp::ValAt | MapOp::After => unreachable!("made by the compiler, not named"),
        }
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::MapCall { op, args: all }, result).at(name_span)))
    }

    /// `map[key] op= value;`: the value under `key` is read (the program stops
    /// when there is none), combined with `value` and put back. The map place
    /// and the key are evaluated once, the new value before the map is changed:
    ///
    /// ```text
    /// r = &mut map;  k = key;
    /// n = (*r)[k] op value;       // MapOp::Expect, not an optional
    /// (*r)::set(k, n);
    /// ```
    fn map_update(&mut self, p: &PlaceInfo, op: BinOp, index: &ast::Expr, value: &ast::Expr, span: Span, out: &mut hir::Block) {
        use hir::ExprKind as K;
        let map_ty = p.ty;
        let (kt, vt) = map_ty.map_parts().expect("a map");
        let Some(key_value) = self.coerce(index, kt) else {
            self.expr(value);
            return;
        };
        let place = self.place_expr(p);
        let ref_ty = Type::borrow(true, map_ty);
        let r = self.hidden_named("map update borrow", ref_ty);
        let key = self.hidden_named("map update key", kt);
        let new = self.hidden_named("map update value", vt);
        let local = |id, ty| hir::Expr::new(K::Local(id), ty);
        let whole = || hir::Expr::new(K::Deref(Box::new(local(r, ref_ty))), map_ty);
        out.push(hir::Stmt::Decl(r));
        out.push(hir::Stmt::Assign(hir::Place::local(r), hir::Expr::new(K::Borrow { mutable: true, place: Box::new(place) }, ref_ty)));
        out.push(hir::Stmt::Decl(key));
        out.push(hir::Stmt::Assign(hir::Place::local(key), key_value));
        let current = hir::Expr::new(K::MapCall { op: hir::MapOp::Expect, args: vec![whole(), local(key, kt)] }, vt).at(span);
        let Some(updated) = self.compound(op, Some(current), index.span, value, span) else { return };
        out.push(hir::Stmt::Decl(new));
        out.push(hir::Stmt::Assign(hir::Place::local(new), updated));
        let set = hir::Expr::new(K::MapCall { op: hir::MapOp::Set, args: vec![whole(), local(key, kt), local(new, vt)] }, Type::None).at(span);
        out.push(hir::Stmt::Expr(set));
    }

    /// `map[key] = value;` (`value` as a statement): adds or replaces the entry.
    /// Returns false when the target is not an item of a map (nothing was
    /// reported then, so the caller checks it the usual way).
    pub(super) fn map_assign(&mut self, recv: &ast::Expr, index: &ast::Expr, op: Option<BinOp>, value: &ast::Expr, span: Span, out: &mut hir::Block) -> bool {
        let n = self.diags.len();
        let Some(mut p) = self.lvalue(recv) else {
            self.diags.truncate(n);
            return false;
        };
        Self::auto_deref(&mut p);
        if !p.ty.is_map() {
            self.diags.truncate(n);
            return false;
        }
        let (kt, vt) = p.ty.map_parts().expect("a map");
        if let Some(op) = op {
            self.map_update(&p, op, index, value, span, out);
            return true;
        }
        let map = self.place_expr(&p);
        let (Some(k), Some(v)) = (self.coerce(index, kt), self.coerce(value, vt)) else { return true };
        let call = hir::Expr::new(hir::ExprKind::MapCall { op: hir::MapOp::Set, args: vec![map, k, v] }, Type::None).at(span);
        out.push(hir::Stmt::Expr(call));
        true
    }
}
