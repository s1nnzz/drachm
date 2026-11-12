//! Lists (`T[?]`) and arrays (`T[N]`, S6C.1): the types, literals, indexing
//! and the first list methods. The runtime behind them is `runtime/lists.h`.
//!
//! A list is a move type that owns a buffer (`Type::List`, 24 bytes like a
//! string). An array is `N` values side by side (`Type::Array`): it owns what
//! its elements own and copies by bits when the element is Copy and the whole
//! is at most 32 bytes (the struct rule). Neither can hold a borrow yet (Q3).

use num_traits::ToPrimitive;

use super::{Checker, Globals, PlaceInfo, Val, optionals, origin};
use crate::ast;
use crate::consts::Const;
use crate::diag::{Diagnostic, Span, codes};
use crate::hir::{self, ExprKind as K};
use crate::types::Type;

/// Arrays above this many bytes by value are refused (E0901).
const MAX_ARRAY_BYTES: u64 = 1 << 20;

impl Globals {
    /// `elem[?]`, noted for the backends.
    pub(super) fn make_list(&self, elem: Type) -> Type {
        let t = Type::list(elem);
        self.note_collection(t);
        t
    }

    /// `elem[len]`, noted for the backends.
    pub(super) fn make_array(&self, elem: Type, len: u64) -> Type {
        let t = Type::array(elem, len);
        self.note_collection(t);
        t
    }

    /// The bytes of an array of plain numbers, if `t` is one (a struct's size
    /// is not known while its fields are still being declared).
    fn scalar_bytes(t: Type) -> Option<u64> {
        match t {
            Type::Array { elem, len } => Some(Self::scalar_bytes(*elem)?.saturating_mul(len)),
            t => crate::hir::scalar_size(t).map(u64::from),
        }
    }

    /// `elem` with the written `dim` after it. Reports why not.
    fn collection_of(&self, elem: Type, dim: &ast::Dim, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let at = match dim {
            ast::Dim::List(s) | ast::Dim::Array(_, s) => *s,
        };
        let kind = if matches!(dim, ast::Dim::List(_)) { "list" } else { "array" };
        if elem == Type::None {
            diags.push(Diagnostic::at(&codes::E0104, at, format!("a {kind} cannot hold `none`")).note("`none` holds no value"));
            return None;
        }
        if elem.is_borrow() || self.holds_borrow(elem) {
            diags.push(
                Diagnostic::at(&codes::E0901, at, format!("a {kind} of borrows is not supported yet"))
                    .note(format!("this {kind} would hold a borrow"))
                    .help("keep the values themselves in the collection, and borrow them where you use them"),
            );
            return None;
        }
        match dim {
            ast::Dim::List(_) => Some(self.make_list(elem)),
            ast::Dim::Array(n, _) => {
                if Self::scalar_bytes(elem).is_some_and(|b| b.saturating_mul(*n) > MAX_ARRAY_BYTES) {
                    diags.push(
                        Diagnostic::at(&codes::E0901, at, "large arrays are not supported yet")
                            .note(format!("{n} items of `{elem}` is more than 1 MiB"))
                            .help("use a list instead: `[?]` grows on the heap"),
                    );
                    return None;
                }
                Some(self.make_array(elem, *n))
            }
        }
    }

    /// A type written with `[?]` / `[N]` after its name: the name (with its
    /// unit and type arguments), then each dimension in order, then the `?` of
    /// an optional, then the `&` / `*` marks.
    pub(super) fn resolve_dims(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let core = ast::TypeName { mods: Vec::new(), optional: None, dims: Vec::new(), ..t.clone() };
        let mut ty = self.resolve(&core, diags)?;
        for dim in &t.dims {
            ty = self.collection_of(ty, dim, diags)?;
        }
        let ty = optionals::wrap(self, t, ty)?;
        Some(t.mods.iter().rev().fold(ty, |ty, (m, _)| match m {
            ast::TypeMod::Ref { mutable } => Type::borrow(*mutable, ty),
            ast::TypeMod::Ptr => Type::ptr(ty),
            ast::TypeMod::FixedPtr => Type::fixed_ptr(ty),
        }))
    }
}

/// The list methods that are decided (Q10) but not built yet.
const LATER: &[&str] = &["contains", "each", "swap"];

impl Checker<'_> {
    /// `[a, b, c]`. With a type wanted (`to`, a list or an array) the items
    /// convert to its item type; without one the first item decides.
    pub(super) fn list_literal(&mut self, items: &[ast::Expr], to: Option<Type>, span: Span) -> Option<hir::Expr> {
        let mut want = to.filter(|t| t.is_list() || t.is_array());
        if let Some(t) = to
            && want.is_none()
            && let Some(inner) = optionals::inner_of(self.g, t)
            && (inner.is_list() || inner.is_array())
        {
            let x = self.list_literal(items, Some(inner), span)?;
            return Some(optionals::some_value(t, x, span));
        }
        if let (Some(t), None) = (to, want) {
            self.diags.push(Diagnostic::at(&codes::E0101, span, format!("expected `{t}`, found a list")).note("a list literal here"));
            for i in items {
                self.expr(i);
            }
            return None;
        }
        let inferred = want.is_none();
        if want.is_none() {
            let Some(first) = items.first() else {
                self.diags.push(
                    Diagnostic::at(&codes::E0624, span, "cannot tell what this empty list holds")
                        .note("there is no item and no type to take it from")
                        .help("give it a type: `string[?] xs = [];`"),
                );
                return None;
            };
            let v = self.expr(first)?;
            let x = self.materialise(v, first.span)?;
            if x.ty == Type::None || x.ty.is_borrow() {
                self.diags.push(Diagnostic::at(&codes::E0901, first.span, "a list of borrows is not supported yet"));
                return None;
            }
            want = Some(self.g.make_list(x.ty));
        }
        let want = want?;
        let elem = want.elem()?;
        if let Type::Array { len, .. } = want
            && len as usize != items.len()
        {
            let n = items.len();
            self.diags.push(
                Diagnostic::at(
                    &codes::E0625,
                    span,
                    format!("`{want}` needs {len} value{}, but {n} {} given", if len == 1 { "" } else { "s" }, if n == 1 { "is" } else { "are" }),
                )
                .note(format!("{n} value{} here", if n == 1 { "" } else { "s" }))
                .help(if (n as u64) < len { "add the missing values, or change the length in the type" } else { "remove the extra values, or change the length in the type" }),
            );
            for i in items {
                self.expr(i);
            }
            return None;
        }
        let mut out = Vec::new();
        let mut ok = true;
        for item in items {
            let got = if inferred {
                match self.expr(item) {
                    Some(Val::Expr(x)) if x.ty != elem && !(x.ty.is_numeric() && elem.is_numeric()) => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0623, item.span, format!("this list holds `{elem}`, but this item is `{}`", x.ty))
                                .note(format!("this item is `{}`", x.ty))
                                .label(items[0].span, format!("the first item is `{elem}`"))
                                .help("convert the items to one type, or give the list a type: `string[?] xs = [...];`"),
                        );
                        None
                    }
                    Some(v) => self.into(v, elem, origin(item)),
                    None => None,
                }
            } else {
                self.coerce(item, elem)
            };
            match got {
                Some(x) => out.push(x),
                None => ok = false,
            }
        }
        if !ok {
            return None;
        }
        let kind = if want.is_list() { K::List(out) } else { K::Array(out) };
        Some(hir::Expr::new(kind, want).at(span))
    }

    /// A list literal given as the value of field `i` (or the field `name`) of
    /// struct `s` in a construction: the field's type is what the literal is
    /// checked against. `None` when `value` is not a literal or the field is
    /// not a list or array.
    pub(super) fn literal_for_field(&mut self, s: usize, i: usize, name: &Option<(String, Span)>, value: &ast::Expr) -> Option<Option<Val>> {
        let ast::ExprKind::ListLit(items) = &value.kind else { return None };
        let fields = &self.g.structs[s].fields;
        let want = match name {
            Some((n, _)) => fields.iter().find(|f| f.name == *n),
            None => fields.get(i),
        }
        .and_then(|f| f.ty)?;
        if !(want.is_list() || want.is_array()) {
            return None;
        }
        Some(self.list_literal(items, Some(want), value.span).map(Val::Expr))
    }

    /// The index inside `[ ]` of something of type `container` (a list or an
    /// array, `shown` as written). A constant index into an array is checked now.
    pub(super) fn index_arg(&mut self, index: &ast::Expr, container: Type, shown: Span) -> Option<hir::Expr> {
        let v = self.expr(index)?;
        match v {
            Val::Const(Const::Int(n)) => {
                let Some(n) = n.to_i128() else {
                    self.diags.push(Diagnostic::at(&codes::E0101, index.span, "this index is far too large").note("does not fit in 64 bits"));
                    return None;
                };
                match container {
                    Type::Array { len, .. } => {
                        let pos = if n < 0 { len as i128 + n } else { n };
                        if pos < 0 || pos >= len as i128 {
                            self.diags.push(
                                Diagnostic::at(&codes::E0620, index.span, format!("index {n} is outside `{container}`"))
                                    .note(format!("this index is {n}"))
                                    .label(shown, format!("an array of {len} value{}", if len == 1 { "" } else { "s" }))
                                    .help(format!("valid indexes are 0 to {}, or -1 to -{len} from the end", len - 1)),
                            );
                            return None;
                        }
                        Some(hir::Expr::new(K::Int(pos as u128), Type::U64).at(index.span))
                    }
                    _ if n >= 0 && n <= u64::MAX as i128 => Some(hir::Expr::new(K::Int(n as u128), Type::U64).at(index.span)),
                    _ if n < 0 && n >= i64::MIN as i128 => Some(hir::Expr::new(K::Int(n as i64 as u64 as u128), Type::S64).at(index.span)),
                    _ => {
                        self.diags.push(Diagnostic::at(&codes::E0101, index.span, "this index is far too large").note("does not fit in 64 bits"));
                        None
                    }
                }
            }
            Val::Expr(x) if x.ty.is_int() && !x.ty.mentions_unit() && matches!(x.ty, Type::Int { bits, .. } if bits <= 64) => Some(x),
            Val::Expr(x) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0101, index.span, format!("an index is a whole number, found `{}`", x.ty))
                        .note(format!("this is `{}`", x.ty))
                        .help("use an integer of at most 64 bits, without a unit"),
                );
                None
            }
            Val::Const(_) | Val::UConst(..) => {
                self.diags.push(Diagnostic::at(&codes::E0101, index.span, "an index is a whole number, found a decimal number").note("this is not a whole number"));
                None
            }
            Val::NoneLit(s) => {
                self.none_without_type(s);
                None
            }
        }
    }

    /// `recv[index]` as a value.
    pub(super) fn index_expr(&mut self, recv: &ast::Expr, index: &ast::Expr) -> Option<Val> {
        let base = match self.expr(recv)? {
            Val::Expr(x) => x,
            _ => {
                self.diags.push(Diagnostic::at(&codes::E0101, recv.span, "numbers cannot be indexed").note("this is a number"));
                self.expr(index);
                return None;
            }
        };
        if !(base.ty.is_list() || base.ty.is_array()) {
            self.not_indexable(recv, base.ty);
            self.expr(index);
            return None;
        }
        let idx = self.index_arg(index, base.ty, recv.span)?;
        let elem = base.ty.elem()?;
        Some(Val::Expr(hir::Expr::new(K::Index(Box::new(base), Box::new(idx)), elem)))
    }

    fn not_indexable(&mut self, recv: &ast::Expr, ty: Type) {
        self.diags.push(
            Diagnostic::at(&codes::E0101, recv.span, format!("`{ty}` cannot be indexed"))
                .note(format!("this is `{ty}`"))
                .help("only lists (`T[?]`) and arrays (`T[N]`) have items to pick with `[ ]`"),
        );
    }

    /// `recv[index]` as a place that is written or lent.
    pub(super) fn index_place(&mut self, recv: &ast::Expr, index: &ast::Expr) -> Option<PlaceInfo> {
        let mut p = self.lvalue(recv)?;
        Self::auto_deref(&mut p);
        if !(p.ty.is_list() || p.ty.is_array()) {
            self.not_indexable(recv, p.ty);
            self.expr(index);
            return None;
        }
        let idx = self.index_arg(index, p.ty, recv.span)?;
        p.ty = p.ty.elem()?;
        p.proj.push(hir::HirProj::Index(Box::new(idx)));
        Some(p)
    }

    /// The methods of a list or array (`v` is the receiver as a value).
    pub(super) fn list_method(&mut self, v: hir::Expr, recv: &ast::Expr, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr]) -> Option<Val> {
        let ty = v.ty;
        let elem = ty.elem()?;
        self.no_targs(name, targs);
        match name {
            "len" => {
                for a in args {
                    self.diags.push(Diagnostic::at(&codes::E0103, a.span, "`len` takes no arguments"));
                }
                Some(Val::Expr(hir::Expr::new(K::Len(Box::new(v)), Type::U64)))
            }
            "push" if ty.is_list() => {
                let [arg] = args else {
                    self.diags.push(
                        Diagnostic::at(&codes::E0103, name_span, format!("`push` takes 1 argument, but {} given", args.len()))
                            .help(format!("write the item to add: `{}::push(item)`", self.text(recv.span))),
                    );
                    for a in args {
                        self.expr(a);
                    }
                    return None;
                };
                let value = self.coerce(arg, elem)?;
                Some(Val::Expr(hir::Expr::new(K::ListPush { list: Box::new(v), value: Box::new(value) }, Type::None)))
            }
            "push" => {
                for a in args {
                    self.expr(a);
                }
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no method `push`"))
                        .label(recv.span, format!("this is `{ty}`, an array: its length is fixed"))
                        .help("use a list (`T[?]`) for something that grows"),
                );
                None
            }
            "pop" | "get" | "first" | "last" | "insert" | "remove" | "clear" | "join" => self.list_runtime_method(v, recv, name, name_span, args),
            _ => {
                for a in args {
                    self.expr(a);
                }
                if LATER.contains(&name) {
                    self.diags.push(Diagnostic::at(&codes::E0901, name_span, format!("`{name}` on a list is not supported yet")));
                } else {
                    self.diags.push(
                        Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no method `{name}`"))
                            .label(recv.span, format!("this is `{ty}`"))
                            .help("the methods so far are `len()` and, on a list, `push(item)`"),
                    );
                }
                None
            }
        }
    }
}

impl Checker<'_> {
    /// The methods that call the runtime (Q10, Q11): `pop()`, `get(i)`,
    /// `first()`, `last()` (`T?`), `insert(i, v)`, `remove(i)`, `clear()` and
    /// `join(sep)` of a `string[?]`. Negative indexes count from the end.
    fn list_runtime_method(&mut self, v: hir::Expr, recv: &ast::Expr, name: &str, name_span: Span, args: &[ast::Expr]) -> Option<Val> {
        use hir::ListOp;
        let ty = v.ty;
        let elem = ty.elem()?;
        let (op, takes) = match name {
            "pop" => (ListOp::Pop, 0),
            "get" => (ListOp::Get, 1),
            "first" | "last" => (ListOp::Get, 0),
            "insert" => (ListOp::Insert, 2),
            "remove" => (ListOp::Remove, 1),
            "clear" => (ListOp::Clear, 0),
            _ => (ListOp::Join, 1),
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
        if ty.is_array() && op.changes() {
            for a in args {
                self.expr(a);
            }
            self.diags.push(
                Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no method `{name}`"))
                    .label(recv.span, format!("this is `{ty}`, an array: its length is fixed"))
                    .help("use a list (`T[?]`) for something that changes its length"),
            );
            return None;
        }
        if ty.is_array() {
            for a in args {
                self.expr(a);
            }
            self.diags.push(Diagnostic::at(&codes::E0901, name_span, format!("`{name}` on an array is not supported yet")));
            return None;
        }
        if op == ListOp::Join && elem != Type::Str {
            for a in args {
                self.expr(a);
            }
            self.diags.push(
                Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no method `join`"))
                    .label(recv.span, format!("this list holds `{elem}`"))
                    .help("`join` puts a list of `string`s together, with the separator between them"),
            );
            return None;
        }
        if op == ListOp::Get && !self.g.plain_copy(Some(elem)) {
            for a in args {
                self.expr(a);
            }
            self.diags.push(
                Diagnostic::at(&codes::E0901, name_span, format!("`{name}` on a list of `{elem}` is not supported yet"))
                    .note("an item that owns memory cannot be copied or lent out by `get` yet"),
            );
            return None;
        }
        let mut all = vec![v];
        let result = match op {
            ListOp::Pop | ListOp::Get => {
                if name == "first" || name == "last" {
                    let at = if name == "first" { 0 } else { -1i64 as u64 as u128 };
                    let ty_i = if name == "first" { Type::U64 } else { Type::S64 };
                    all.push(hir::Expr::new(K::Int(at), ty_i).at(name_span));
                } else if let Some(arg) = args.first() {
                    let idx = self.index_arg(arg, ty, recv.span)?;
                    all.push(idx);
                }
                optionals::optional_of(self.g, elem)?
            }
            ListOp::Insert => {
                let idx = self.index_arg(&args[0], ty, recv.span)?;
                let value = self.coerce(&args[1], elem)?;
                all.push(idx);
                all.push(value);
                Type::None
            }
            ListOp::Remove => {
                all.push(self.index_arg(&args[0], ty, recv.span)?);
                elem
            }
            ListOp::Clear => Type::None,
            ListOp::Join => {
                all.push(self.coerce(&args[0], Type::Str)?);
                Type::Str
            }
        };
        Some(Val::Expr(hir::Expr::new(K::ListCall { op, args: all }, result).at(name_span)))
    }
}

#[cfg(test)]
mod tests {
    use crate::mir::{is_copy, needs_drop};
    use crate::types::Type;

    #[test]
    fn sizes_copy_and_ownership() {
        let src = "
struct Row { public: uint8[4] px; string[?] names; }
struct Tree { public: Tree[?] kids; }
struct Big { public: uint64[5] a; }
struct Wide { public: uint128[2] a; }
none main() { }
";
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        assert_eq!(hir.size_align(Type::list(Type::U32)), (24, 8));
        assert_eq!(hir.size_align(Type::array(Type::U8, 4)), (4, 1));
        assert_eq!(hir.size_align(Type::array(Type::U64, 3)), (24, 8));
        assert_eq!(hir.size_align(Type::array(Type::Int { bits: 128, signed: false }, 2)), (32, 16));
        assert!(is_copy(&hir, Type::array(Type::U8, 4)));
        assert!(!is_copy(&hir, Type::array(Type::U64, 5)), "40 bytes is over the Copy limit");
        assert!(!is_copy(&hir, Type::list(Type::U8)));
        assert!(!is_copy(&hir, Type::array(Type::Str, 2)));
        assert!(needs_drop(&hir, Type::list(Type::U8)));
        assert!(needs_drop(&hir, Type::array(Type::Str, 2)));
        assert!(!needs_drop(&hir, Type::array(Type::U8, 2)));
        let row = hir.structs.iter().position(|s| s.name == "Row").unwrap();
        assert!(hir.structs[row].owns && !hir.structs[row].copy);
        assert_eq!(hir.structs[row].size, 32);
        let tree = hir.structs.iter().position(|s| s.name == "Tree").unwrap();
        assert_eq!(hir.structs[tree].size, 24, "a list inside its own element type is fine");
        assert!(!hir.structs.iter().any(|s| s.name == "Big" && s.copy));
        assert!(hir.collections.contains(&Type::array(Type::U8, 4)));
    }
}
