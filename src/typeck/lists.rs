//! Lists (`T[?]`) and arrays (`T[N]`, S6C.1): the types, literals, indexing
//! and the first list methods. The runtime behind them is `runtime/lists.h`.
//!
//! A list is a move type that owns a buffer (`Type::List`, 24 bytes like a
//! string). An array is `N` values side by side (`Type::Array`): it owns what
//! its elements own and copies by bits when the element is Copy and the whole
//! is at most 32 bytes (the struct rule). Neither can hold a borrow yet (Q3).

use std::collections::HashMap;

use num_traits::ToPrimitive;

use super::{Checker, Globals, Origin, PlaceInfo, Sig, Val, optionals, origin};
use crate::ast;
use crate::consts::Const;
use crate::diag::{Diagnostic, Span, codes};
use crate::hir::{self, ExprKind as K, FuncId};
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

    /// The length a value parameter stands for in `[N]`.
    fn length_param(&self, name: &str, span: Span, diags: &mut Vec<Diagnostic>) -> Option<u64> {
        match self.tenv.get(name) {
            Some(t) => match t.as_val() {
                Some((_, v)) if v >= 1 => Some(v as u64),
                Some((_, v)) => {
                    diags.push(
                        Diagnostic::at(&codes::E0636, span, format!("`{name}` is {v}, and an array needs at least one item"))
                            .note("an array length must be 1 or more")
                            .help("use a list, `[?]`, for something that may be empty"),
                    );
                    None
                }
                None => {
                    diags.push(
                        Diagnostic::at(&codes::E0602, span, format!("`{name}` is not a value, so it cannot be an array length"))
                            .help(format!("declare it as a value parameter: `uint32 {name}`")),
                    );
                    None
                }
            },
            None if let Some(info) = self.consts.get(name) => match &info.value {
                crate::consts::Const::Int(v) if *v >= num_bigint::BigInt::from(1) => num_traits::ToPrimitive::to_u64(v).or_else(|| {
                    diags.push(Diagnostic::at(&codes::E0901, span, "an array this long is not supported yet"));
                    None
                }),
                crate::consts::Const::Int(v) => {
                    diags.push(
                        Diagnostic::at(&codes::E0636, span, format!("`{name}` is {v}, and an array needs at least one item"))
                            .note("an array length must be 1 or more")
                            .help("use a list, `[?]`, for something that may be empty"),
                    );
                    None
                }
                _ => {
                    diags.push(Diagnostic::at(&codes::E0634, span, format!("`{name}` is not a whole number, so it cannot be an array length")));
                    None
                }
            },
            None => {
                diags.push(
                    Diagnostic::at(&codes::E0634, span, format!("`{name}` is not a constant"))
                        .note(self.run_value_note(name).unwrap_or_else(|| "an array length is a number, a constant or a value parameter".to_string()))
                        .help(format!("declare `{name}` as a constant at the top of the file (`uint32 {name} = 8;`) or as a value parameter of the struct or function: `uint32 {name}`")),
                );
                None
            }
        }
    }

    /// The length `[N + 1]` stands for here (the value parameters in scope are known).
    fn calc_length(&self, e: &ast::ConstExpr, span: Span, diags: &mut Vec<Diagnostic>) -> Option<u64> {
        let v = match super::generics::eval_value(self, e) {
            Ok((v, _)) => v,
            Err(d) => {
                diags.push(d);
                return None;
            }
        };
        if v < num_bigint::BigInt::from(1) {
            diags.push(
                Diagnostic::at(&codes::E0636, span, format!("this length is {v}, and an array needs at least one item"))
                    .note("an array length must be 1 or more")
                    .help("use a list, `[?]`, for something that may be empty"),
            );
            return None;
        }
        num_traits::ToPrimitive::to_u64(&v).or_else(|| {
            diags.push(Diagnostic::at(&codes::E0901, span, "an array this long is not supported yet"));
            None
        })
    }

    /// `elem` with the written `dim` after it. Reports why not.
    pub(super) fn collection_of(&self, elem: Type, dim: &ast::Dim, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let at = match dim {
            ast::Dim::List(s) | ast::Dim::Array(_, s) | ast::Dim::Param(_, s) | ast::Dim::Calc(_, s) => *s,
        };
        let kind = if matches!(dim, ast::Dim::List(_)) { "list" } else { "array" };
        if elem == Type::None {
            diags.push(Diagnostic::at(&codes::E0104, at, format!("a {kind} cannot hold `none`")).note("`none` holds no value"));
            return None;
        }
        let param_len;
        let dim = match dim {
            ast::Dim::Param(name, span) => {
                param_len = ast::Dim::Array(self.length_param(name, *span, diags)?, *span);
                &param_len
            }
            ast::Dim::Calc(e, span) => {
                param_len = ast::Dim::Array(self.calc_length(e, *span, diags)?, *span);
                &param_len
            }
            d => d,
        };
        match dim {
            ast::Dim::List(_) => Some(self.make_list(elem)),
            ast::Dim::Param(..) | ast::Dim::Calc(..) => unreachable!(),
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
const LATER: &[&str] = &["swap"];

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
            if x.ty.is_func() {
                self.diags.push(Diagnostic::at(&codes::E0652, first.span, "a function value cannot be stored in a list"));
                return None;
            }
            if x.ty == Type::None {
                self.diags.push(Diagnostic::at(&codes::E0104, first.span, "a list cannot hold `none`").note("`none` holds no value"));
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
        if base.ty.is_map() {
            return self.map_index(base, index);
        }
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
        if p.ty.is_map() {
            self.expr(index);
            self.diags.push(
                Diagnostic::at(&codes::E0901, recv.span, "a map item cannot be borrowed or moved yet")
                    .note("only reading it (`map[key]`) and writing it (`map[key] = value;`) are supported")
                    .help("copy the value out with `m[key]` and work with that"),
            );
            return None;
        }
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
            "each" => {
                let [arg] = args else {
                    self.diags.push(Diagnostic::at(&codes::E0103, name_span, format!("`each` takes 1 argument, but {} given", args.len())));
                    return None;
                };
                let callback = self.each_callback(arg, elem)?;
                let function = self.g.list_func(ListFunc::Each(callback.ty), ty);
                let list = hir::Expr::new(K::Borrow { mutable: false, place: Box::new(v) }, Type::borrow(false, ty)).at(recv.span);
                Some(Val::Expr(hir::Expr::new(K::Call(function, vec![list, callback]), Type::None).at(name_span)))
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
            "contains" => self.list_contains(v, recv, name_span, args),
            "containsRef" if ty.is_list() => self.list_contains_ref(v, name_span, args),
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
        // `copy` is only meant for the call it wraps, not the calls in its arguments.
        let copying = std::mem::replace(&mut self.copying_item, false);
        // Items that own memory are lent out, not copied: `get`, `first` and `last` give a
        // view, an optional holding a borrow of the item (`copy` asks for an owned item).
        let mut op = op;
        if op == ListOp::Get && !self.g.plain_copy(Some(elem)) && !copying {
            op = ListOp::View;
        }
        let mut all = vec![v];
        let result = match op {
            ListOp::Pop | ListOp::Get | ListOp::View => {
                if name == "first" || name == "last" {
                    let at = if name == "first" { 0 } else { -1i64 as u64 as u128 };
                    let ty_i = if name == "first" { Type::U64 } else { Type::S64 };
                    all.push(hir::Expr::new(K::Int(at), ty_i).at(name_span));
                } else if let Some(arg) = args.first() {
                    let idx = self.index_arg(arg, ty, recv.span)?;
                    all.push(idx);
                }
                optionals::optional_of(self.g, if op == ListOp::View { Type::borrow(false, elem) } else { elem })?
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
            ListOp::SwapRemove => elem,
            // Only written as `take xs[i] leaving v` (`take_item`); never through a method name.
            ListOp::TakeLeaving => unreachable!("`take xs[i] leaving v` is built by `take_item`"),
            ListOp::Join => {
                all.push(self.coerce(&args[0], Type::Str)?);
                Type::Str
            }
        };
        Some(Val::Expr(hir::Expr::new(K::ListCall { op, args: all }, result).at(name_span)))
    }
}

/// The functions the checker synthesises for lists and arrays: `==` of two
/// of them, and `contains`. Their bodies are loops over the items, built once
/// the other functions are checked (`list_func_body`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ListFunc {
    /// `(&L a, &L b) -> bool`: the same length and each item equal.
    Eq,
    /// `(&L a, E item) -> bool`: some item is equal to `item`.
    Contains,
    /// `(&L a, (E) => none callback)`: visit the items in index order.
    Each(Type),
}

impl Globals {
    /// The function for `kind` on the list or array type `list` (made now, its
    /// body later).
    pub(super) fn list_func(&mut self, kind: ListFunc, list: Type) -> FuncId {
        if let Some(&f) = self.list_funcs.get(&(kind, list)) {
            return f;
        }
        let elem = list.elem().expect("a list function is made for a list");
        let borrowed = Type::borrow(false, list);
        let (name, other, ret) = match kind {
            ListFunc::Eq => (format!("{list} =="), borrowed, Type::Bool),
            ListFunc::Contains => (format!("{list} contains"), if self.plain_copy(Some(elem)) { elem } else { Type::borrow(false, elem) }, Type::Bool),
            ListFunc::Each(callback) => (format!("{list} each"), callback, Type::None),
        };
        let sig = Sig { name, params: vec![Some(borrowed), Some(other)], ret: Some(ret), span: Span::new(0, 0), fails: Vec::new(), outcome: None };
        let f = self.add_sig(sig);
        self.list_funcs.insert((kind, list), f);
        self.list_pending.push((kind, list, f));
        f
    }

    /// `a == b` for two items of type `elem`, as a value of type `bool`.
    fn item_same(&mut self, a: hir::Expr, b: hir::Expr, elem: Type) -> hir::Expr {
        let eq = |a, b| hir::Expr::new(K::Binary(hir::BinOp::Eq, Box::new(a), Box::new(b)), Type::Bool);
        // Borrowed items are equal when what they point at is.
        if elem.is_borrow() && let Some(target) = elem.target() {
            let look = |e: hir::Expr| hir::Expr::new(K::Deref(Box::new(e)), target);
            return self.item_same(look(a), look(b), target);
        }
        if let Some(inner) = elem.struct_id() {
            // The comparison consumes its right side, so it gets a copy of a place.
            let b = if b.is_place() && !self.copy_of(elem) { hir::Expr::new(K::DeepCopy(Box::new(b)), elem) } else { b };
            return self.struct_eq(inner, a, b);
        }
        if elem == Type::Str {
            return hir::Expr::new(K::StrCall(hir::StrFn::Eq, vec![a, b]), Type::Bool);
        }
        if elem.is_list() || elem.is_array() {
            let f = self.list_func(ListFunc::Eq, elem);
            let borrow = |place: hir::Expr| hir::Expr::new(K::Borrow { mutable: false, place: Box::new(place) }, Type::borrow(false, elem));
            return hir::Expr::new(K::Call(f, vec![borrow(a), borrow(b)]), Type::Bool);
        }
        eq(a, b)
    }
}

/// The body of list function `f` (see `ListFunc`), with `list` the list type.
/// Hidden locals: `i`, the index. Loop labels are unique within the function.
pub(super) fn list_func_body(g: &mut Globals, kind: ListFunc, list: Type, f: FuncId) -> hir::Func {
    let elem = list.elem().expect("a list function is made for a list");
    let borrowed = Type::borrow(false, list);
    let none = Span::new(0, 0);
    let param = |name: &str, ty: Type| hir::Local { name: name.into(), ty, span: none, mutability: hir::Mutability::Fixed, is_param: true, written: None };
    let mut locals = vec![param("a", borrowed), match kind {
        ListFunc::Eq => param("b", borrowed),
        ListFunc::Contains => param("item", if g.plain_copy(Some(elem)) { elem } else { Type::borrow(false, elem) }),
        ListFunc::Each(callback) => param("callback", callback),
    }];
    let i = locals.len();
    locals.push(hir::Local { name: "i".into(), ty: Type::U64, span: none, mutability: hir::Mutability::Mut, is_param: false, written: None });

    // `*a`, `*b`, the list the parameter refers to, and the items and length of it.
    let whole = |l: usize| hir::Expr::new(K::Deref(Box::new(hir::Expr::new(K::Local(l), borrowed))), list);
    let len = |e: hir::Expr| hir::Expr::new(K::Len(Box::new(e)), Type::U64);
    let item = |e: hir::Expr| hir::Expr::new(K::Index(Box::new(e), Box::new(hir::Expr::new(K::Local(i), Type::U64))), elem);
    let bin = |op, a, b, ty| hir::Expr::new(K::Binary(op, Box::new(a), Box::new(b)), ty);
    let int = |v: u128| hir::Expr::new(K::Int(v), Type::U64);
    let (label_loop, label_if) = (0, 1);
    let step = vec![hir::Stmt::Assign(hir::Place::local(i), bin(hir::BinOp::Add, hir::Expr::new(K::Local(i), Type::U64), int(1), Type::U64))];
    let cond = bin(hir::BinOp::Lt, hir::Expr::new(K::Local(i), Type::U64), len(whole(0)), Type::Bool);
    let body = match kind {
        ListFunc::Eq => {
            let same = g.item_same(item(whole(0)), item(whole(1)), elem);
            let differs = hir::Expr::new(K::Not(Box::new(same)), Type::Bool);
            let lengths_differ = bin(hir::BinOp::Ne, len(whole(0)), len(whole(1)), Type::Bool);
            vec![
                hir::Stmt::If {
                    label: label_if + 1,
                    branches: vec![hir::Branch { test: Some(lengths_differ), body: vec![hir::Stmt::Return(Some(hir::Expr::new(K::Bool(false), Type::Bool)))], end: none }],
                },
                hir::Stmt::Decl(i),
                hir::Stmt::Assign(hir::Place::local(i), int(0)),
                hir::Stmt::Loop {
                    label: label_loop,
                    cond: Some(cond),
                    body: vec![hir::Stmt::If {
                        label: label_if,
                        branches: vec![hir::Branch { test: Some(differs), body: vec![hir::Stmt::Return(Some(hir::Expr::new(K::Bool(false), Type::Bool)))], end: none }],
                    }],
                    step,
                    end: none,
                },
                hir::Stmt::Return(Some(hir::Expr::new(K::Bool(true), Type::Bool))),
            ]
        }
        ListFunc::Contains => {
            // An item that owns memory is lent in (`&item`); look through the borrow.
            let wanted = if g.plain_copy(Some(elem)) {
                hir::Expr::new(K::Local(1), elem)
            } else {
                hir::Expr::new(K::Deref(Box::new(hir::Expr::new(K::Local(1), Type::borrow(false, elem)))), elem)
            };
            let same = g.item_same(item(whole(0)), wanted, elem);
            vec![
                hir::Stmt::Decl(i),
                hir::Stmt::Assign(hir::Place::local(i), int(0)),
                hir::Stmt::Loop {
                    label: label_loop,
                    cond: Some(cond),
                    body: vec![hir::Stmt::If {
                        label: label_if,
                        branches: vec![hir::Branch { test: Some(same), body: vec![hir::Stmt::Return(Some(hir::Expr::new(K::Bool(true), Type::Bool)))], end: none }],
                    }],
                    step,
                    end: none,
                },
                hir::Stmt::Return(Some(hir::Expr::new(K::Bool(false), Type::Bool))),
            ]
        }
        ListFunc::Each(callback) => {
            let Type::Func(sig) = callback else { unreachable!("an each callback") };
            let element = item(whole(0));
            let arg = if elem.is_borrow() && sig.params[0] == elem {
                // The items are borrows already: the callback gets the borrow itself.
                element
            } else if sig.params[0].is_borrow() {
                hir::Expr::new(K::Borrow { mutable: false, place: Box::new(element) }, sig.params[0])
            } else if g.plain_copy(Some(elem)) {
                element
            } else {
                hir::Expr::new(K::DeepCopy(Box::new(element)), elem)
            };
            let call = hir::Expr::new(K::FnCall(vec![hir::Expr::new(K::Local(1), callback), arg]), Type::None);
            vec![
                hir::Stmt::Decl(i),
                hir::Stmt::Assign(hir::Place::local(i), int(0)),
                hir::Stmt::Loop { label: label_loop, cond: Some(cond), body: vec![hir::Stmt::Expr(call)], step, end: none },
            ]
        }
    };
    hir::Func {
        symbol: format!("drachm_list_{}", f),
        params: 2,
        has_self: false,
        locals,
        ret: g.sigs[f].ret.unwrap(),
        body,
        name_span: none,
        span: none,
        body_end: none,
        is_unsafe: false,
        lives: None,
        lives_span: None,
    }
}

impl Checker<'_> {
    /// `a == b` / `a != b` for two lists (or arrays) of one type: the
    /// synthesised function compares the lengths and then each item. The
    /// items must have `==` (E0527 for an enum with values).
    pub(super) fn list_compare(&mut self, op: ast::BinOp, lhs: hir::Expr, lo: Origin, rv: Val, ro: Origin) -> Option<Val> {
        let t = lhs.ty;
        let elem = t.elem()?;
        if self.refuse_dyn_items(elem, lo.span.to(ro.span)) {
            return None;
        }
        if let Some(bad) = self.payload_enum_in(elem) {
            self.diags.push(
                Diagnostic::at(&codes::E0527, lo.span.to(ro.span), format!("`{t}` holds a `{bad}`, which has variants with values, so it cannot be compared with `{}`", op.symbol()))
                    .label(lo.span, format!("this is a `{t}`"))
                    .help("compare the items one by one with `match`, or give the enum no values"),
            );
            return None;
        }
        let rhs = self.into(rv, t, ro)?;
        let f = self.g.list_func(ListFunc::Eq, t);
        let borrow = |e: hir::Expr| hir::Expr::new(K::Borrow { mutable: false, place: Box::new(e) }, Type::borrow(false, t));
        let eq = hir::Expr::new(K::Call(f, vec![borrow(lhs), borrow(rhs)]), Type::Bool);
        let value = if op == ast::BinOp::Eq { eq } else { hir::Expr::new(K::Not(Box::new(eq)), Type::Bool) };
        Some(Val::Expr(value))
    }

    /// `take xs[i] reorders` (Q11): the item moves out, the last item fills its
    /// place, and the list is one shorter. `take xs[i] leaving v`: the item
    /// moves out and `v` takes its place, so the list keeps its length and order.
    pub(super) fn take_item(&mut self, place: &ast::Expr, recv: &ast::Expr, index: &ast::Expr, leaving: Option<&ast::Expr>, reorders: bool) -> Option<Val> {
        if leaving.is_none() && !reorders {
            self.diags.push(
                Diagnostic::at(&codes::E0901, place.span, "`take` of an item of a list is not supported yet")
                    .help("write `take xs[i] reorders`, which fills its place with the last item, or copy it with `copy`"),
            );
            return None;
        }
        let base = match self.expr(recv)? {
            Val::Expr(x) => x,
            _ => {
                self.expr(index);
                return None;
            }
        };
        if base.ty.is_array() {
            self.diags.push(Diagnostic::at(&codes::E0901, place.span, "`take` of an item of an array is not supported yet").help("an array has a fixed length: copy the item with `copy`"));
            self.expr(index);
            return None;
        }
        if !base.ty.is_list() {
            self.not_indexable(recv, base.ty);
            self.expr(index);
            return None;
        }
        let idx = self.index_arg(index, base.ty, recv.span)?;
        let elem = base.ty.elem()?;
        let Some(v) = leaving else {
            return Some(Val::Expr(hir::Expr::new(K::ListCall { op: hir::ListOp::SwapRemove, args: vec![base, idx] }, elem).at(place.span)));
        };
        let value = self.coerce(v, elem)?;
        Some(Val::Expr(hir::Expr::new(K::ListCall { op: hir::ListOp::TakeLeaving, args: vec![base, idx, value] }, elem).at(place.span)))
    }

    /// `xs::contains(item)`: some item of the list is equal to `item`. The
    /// items must be Copy for now (an owning item would be moved into the search).
    /// E0901 for comparing lists whose items are borrows of an interface: there is no `==` on one.
    fn refuse_dyn_items(&mut self, elem: Type, at: Span) -> bool {
        if elem.is_borrow() && matches!(elem.target(), Some(Type::Dyn { .. })) {
            self.diags.push(Diagnostic::at(&codes::E0901, at, format!("comparing items of type `{elem}` is not supported yet")).note("an interface has no `==`"));
            return true;
        }
        false
    }

    fn list_contains(&mut self, v: hir::Expr, recv: &ast::Expr, name_span: Span, args: &[ast::Expr]) -> Option<Val> {
        let ty = v.ty;
        let elem = ty.elem()?;
        if self.refuse_dyn_items(elem, name_span) {
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let [arg] = args else {
            self.diags.push(
                Diagnostic::at(&codes::E0103, name_span, format!("`contains` takes 1 argument, but {} given", args.len()))
                    .note("takes 1 argument"),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        };
        // Items that own memory: the value to look for is lent, `names::contains(&name)`.
        let item = if self.g.plain_copy(Some(elem)) {
            self.coerce(arg, elem)?
        } else {
            let want = Type::borrow(false, elem);
            if !matches!(arg.kind, ast::ExprKind::Borrow { mutable: false, .. }) {
                let text = self.text(arg.span).to_string();
                self.expr(arg);
                self.diags.push(
                    Diagnostic::at(&codes::E0101, arg.span, format!("`contains` on a list of `{elem}` takes the value with `&`"))
                        .note(format!("without `&`, `{text}` would move into the search"))
                        .help(format!("write `{}::contains(&{text})`", self.text(recv.span))),
                );
                return None;
            }
            self.coerce(arg, want)?
        };
        let f = self.g.list_func(ListFunc::Contains, ty);
        let borrow = hir::Expr::new(K::Borrow { mutable: false, place: Box::new(v) }, Type::borrow(false, ty));
        Some(Val::Expr(hir::Expr::new(K::Call(f, vec![borrow, item]), Type::Bool)))
    }
}

impl Checker<'_> {
    /// `for (T x of xs) { }` over a list or array (Q8, S6C.2). The loop holds
    /// a borrow of the list for its whole length (a hidden local), so the body
    /// cannot change it. Each time round, `x` is:
    /// - `T x` (`T` is the item type): a copy of the item (a plain read when
    ///   the item is Copy). The copy is made every time; skipping it when the
    ///   body never keeps `x` is not done yet.
    /// - `&T x`: a view of the item; `&mut T x`: a changeable view (the list
    ///   must be changeable: the loop then holds a `&mut` borrow).
    ///
    /// ```text
    /// owned = list;              // only when the list is not a place
    /// hidden = &list;            // or &mut list, the changeable views
    /// i = 0;
    /// loop while (i < len(*hidden)) {
    ///     x = item(*hidden, i);  // or copy, or a borrow of it
    ///     body
    /// } step { i = i + 1; }
    /// ```
    pub(super) fn for_list(&mut self, s: &ast::Stmt, ty: Option<Type>, list: hir::Expr, iter_span: Span, out: &mut hir::Block) {
        let ast::Stmt::ForEach { mutable, ty: ty_name, name, name_span, body, span, .. } = s else { unreachable!() };
        let list_ty = list.ty;
        let elem = list_ty.elem().expect("a list loop is over a list or array");
        let (view, changing_view) = (Type::borrow(false, elem), Type::borrow(true, elem));
        let kind = match ty {
            Some(t) if t == elem => Some(if self.g.plain_copy(Some(elem)) { Item::Read } else { Item::Copy }),
            Some(t) if t == view => Some(Item::View),
            Some(t) if t == changing_view => Some(Item::ChangingView),
            Some(t) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0630, ty_name.span, format!("the items are `{elem}`, but the variable is `{t}`"))
                        .label(iter_span, format!("this holds `{elem}`"))
                        .note(format!("this is `{t}`"))
                        .help(format!("write `{elem}` for a copy of each item, or `&{elem}` to look at each one")),
                );
                None
            }
            None => None,
        };
        self.scopes.push(HashMap::new());
        let var = self.declare(name, ty, *mutable, *name_span, false);
        let label = self.new_label();
        let body_end = body.end;
        let (body, _) = self.looped(label, *span, body, None);
        self.scopes.pop();

        let Some(kind) = kind else { return };
        let mut list = list;
        if !list.is_place() {
            let owned = self.hidden_named("for list", list_ty);
            out.push(hir::Stmt::Decl(owned));
            out.push(hir::Stmt::Assign(hir::Place::local(owned), list));
            list = hir::Expr::new(K::Local(owned), list_ty);
        }
        let changes = matches!(kind, Item::ChangingView);
        let hidden = self.hidden_named("for list borrow", Type::borrow(changes, list_ty));
        let index = self.hidden_named("for list index", Type::U64);
        out.push(hir::Stmt::Assign(hir::Place::local(hidden), hir::Expr::new(K::Borrow { mutable: changes, place: Box::new(list) }, Type::borrow(changes, list_ty))));
        out.push(hir::Stmt::Assign(hir::Place::local(index), hir::Expr::new(K::Int(0), Type::U64)));

        let whole = || hir::Expr::new(K::Deref(Box::new(hir::Expr::new(K::Local(hidden), Type::borrow(changes, list_ty)))), list_ty);
        let here = || hir::Expr::new(K::Local(index), Type::U64);
        let cond = hir::Expr::new(K::Binary(hir::BinOp::Lt, Box::new(here()), Box::new(hir::Expr::new(K::Len(Box::new(whole())), Type::U64))), Type::Bool);
        let item = hir::Expr::new(K::Index(Box::new(whole()), Box::new(here())), elem);
        let value = match kind {
            Item::Read => item,
            Item::Copy => hir::Expr::new(K::DeepCopy(Box::new(item)), elem),
            Item::View => hir::Expr::new(K::Borrow { mutable: false, place: Box::new(item) }, view),
            Item::ChangingView => hir::Expr::new(K::Borrow { mutable: true, place: Box::new(item) }, changing_view),
        };
        let mut inner = vec![hir::Stmt::Decl(var), hir::Stmt::Assign(hir::Place::local(var), value)];
        inner.extend(body);
        let step = vec![hir::Stmt::Assign(
            hir::Place::local(index),
            hir::Expr::new(K::Binary(hir::BinOp::Add, Box::new(here()), Box::new(hir::Expr::new(K::Int(1), Type::U64))), Type::U64),
        )];
        out.push(hir::Stmt::Loop { label, cond: Some(cond), body: inner, step, end: body_end });
    }
}

/// How a `for` over a list binds each item (see `for_list`).
#[derive(Clone, Copy)]
enum Item {
    /// The item is Copy: a plain read.
    Read,
    /// The item owns memory: a copy of it.
    Copy,
    /// `&T`: a view.
    View,
    /// `&mut T`: a changeable view.
    ChangingView,
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
