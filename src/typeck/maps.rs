//! Maps (S6D): `Map<K, V>` and `OrderedMap<K, V>`, hash tables of owned keys
//! and values. The runtime is `runtime/lists.h` (`drachm_map_*`).
//!
//! Step 1 (this file): the type names, the key rule and the layout. A key is
//! a whole number, a float (hashed and compared by its bits, Q14), a `bool` or
//! a `string`. Other key types wait (E0901) until their hashing is built.

use super::{Checker, Globals, Val, optionals};
use crate::ast::{self, BinOp};
use crate::diag::{Diagnostic, Span, codes};
use crate::hir;
use crate::types::Type;

/// A constant key, for finding a repeated one in a literal.
#[derive(PartialEq)]
enum Constant {
    Int(u128),
    Text(String),
    Bool(bool),
}

/// Is `t` written as `Map<K, V>` or `OrderedMap<K, V>` (and not a program's
/// own type of that name)?
pub(super) fn is_map_name(g: &Globals, t: &ast::TypeName) -> bool {
    matches!(t.name.as_str(), "Map" | "OrderedMap")
        && t.args.len() == 2
        && !g.structs_by_name.contains_key(&t.name)
        && !g.enums_by_name.contains_key(&t.name)
        && !g.mono.type_templates.contains_key(&t.name)
}

impl Globals {
    /// `Map<key, val>`, noted for the backends.
    pub(super) fn make_map(&self, key: Type, val: Type, ordered: bool) -> Type {
        let t = Type::map(key, val, ordered);
        self.note_collection(t);
        t
    }
}

/// Can a value of type `key` be a map key (step 1)? `Err` says why not, as
/// the diagnostic to report.
pub(super) fn check_key(key: Type, at: crate::diag::Span) -> Result<(), Diagnostic> {
    match key.bare() {
        Type::Int { .. } | Type::Float(_) | Type::Bool | Type::Str => Ok(()),
        Type::List { .. } | Type::Array { .. } | Type::Map { .. } | Type::Ref { .. } | Type::Ptr { .. } | Type::FixedPtr { .. } => Err(
            Diagnostic::at(&codes::E0626, at, format!("`{key}` cannot be a map key"))
                .note(format!("`{key}` is not hashed"))
                .help("use a whole number, a float, `bool` or `string` as the key"),
        ),
        _ => Err(Diagnostic::at(&codes::E0901, at, format!("`{key}` as a map key is not supported yet")).note("only numbers, `bool` and `string` are hashed so far")),
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
        if let Err(d) = check_key(key, at) {
            diags.push(d);
            return None;
        }
        if val == Type::None {
            diags.push(Diagnostic::at(&codes::E0104, t.args[1].span, "a map cannot hold `none`").note("`none` holds no value"));
            return None;
        }
        if val.is_borrow() || self.holds_borrow(val) || key.is_borrow() {
            diags.push(
                Diagnostic::at(&codes::E0901, t.span, format!("a `{}` of borrows is not supported yet", t.name))
                    .note("this map would hold a borrow")
                    .help("keep the values themselves in the map, and borrow them where you use them"),
            );
            return None;
        }
        Some(self.make_map(key, val, ordered))
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
                if k.ty == Type::None || v.ty == Type::None || k.ty.is_borrow() || v.ty.is_borrow() {
                    self.diags.push(Diagnostic::at(&codes::E0901, span, "a map of borrows is not supported yet"));
                    return None;
                }
                let want = self.g.make_map(k.ty, v.ty, false);
                let (kt, vt) = want.map_parts()?;
                if let Err(d) = check_key(kt, k0.span) {
                    self.diags.push(d);
                    return None;
                }
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
        if let Err(d) = check_key(kt, pairs.first().map_or(span, |p| p.0.span)) {
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
        let mut seen: Vec<(Constant, Span)> = Vec::new();
        for ((k, _), (ast_k, _)) in out.iter().zip(pairs) {
            let constant = match &k.kind {
                hir::ExprKind::Int(n) => Some(Constant::Int(*n)),
                hir::ExprKind::Str(s) => Some(Constant::Text(s.clone())),
                hir::ExprKind::Bool(b) => Some(Constant::Bool(*b)),
                _ => None,
            };
            let Some(c) = constant else { continue };
            if let Some((_, first)) = seen.iter().find(|(s, _)| *s == c) {
                let shown = self.text(ast_k.span);
                self.diags.push(
                    Diagnostic::at(&codes::E0629, ast_k.span, format!("this map literal has the key {shown} twice"))
                        .label(*first, "the first one is here")
                        .help("remove one of them, or the later value would replace the first"),
                );
                return None;
            }
            seen.push((c, ast_k.span));
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
            _ => {
                for a in args {
                    self.expr(a);
                }
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no method `{name}`"))
                        .label(recv.span, format!("this is `{ty}`"))
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
        }
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::MapCall { op, args: all }, result).at(name_span)))
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
        if op.is_some() {
            self.expr(index);
            self.expr(value);
            self.diags.push(
                Diagnostic::at(&codes::E0901, span, "changing a map item with `op=` is not supported yet")
                    .note("a missing key would have no value to add to")
                    .help("write `map[key] = value;` with the whole new value"),
            );
            return true;
        }
        let map = self.place_expr(&p);
        let (Some(k), Some(v)) = (self.coerce(index, kt), self.coerce(value, vt)) else { return true };
        let call = hir::Expr::new(hir::ExprKind::MapCall { op: hir::MapOp::Set, args: vec![map, k, v] }, Type::None).at(span);
        out.push(hir::Stmt::Expr(call));
        true
    }
}
