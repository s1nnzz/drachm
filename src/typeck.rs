//! Name resolution, type checking and definite-assignment analysis.
//! Turns the syntax tree into `hir`, reporting every problem it finds.
//! Loops, jumps and `match` are in `control`; strings are in `strings`.

mod control;
mod strings;

use std::collections::HashMap;

use control::{Target, Test};

use crate::ast::{self, BinOp, ExprKind, OpClass, Stmt, UnOp};
use crate::consts::{Const, FoldError, Folded};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, FuncId, LocalId, StructId};
use crate::types::{self, Type, TypeError};

/// A function or method as callers see it.
struct Sig {
    /// How messages name it: `add`, `Point::move`.
    name: String,
    /// The declared parameters (not `self`). `None` where the written type
    /// did not resolve (already reported).
    params: Vec<Option<Type>>,
    ret: Option<Type>,
    span: Span,
}

struct StructInfo {
    name: String,
    name_span: Span,
    ty: Type,
    fields: Vec<FieldInfo>,
    methods: Vec<MethodInfo>,
    /// Holds a `string` somewhere inside (see `hir`'s module comment).
    owns: bool,
    /// Declared `stays`.
    stays: bool,
    /// Indexes of the borrow fields named by `lives`.
    lives: Vec<usize>,
    /// The `drop()` method.
    has_drop: Option<FuncId>,
}

struct FieldInfo {
    name: String,
    ty: Option<Type>,
    mutable: bool,
    public: bool,
    span: Span,
}

#[derive(Clone)]
struct MethodInfo {
    /// The operator's symbol for operator methods (`"+"`).
    name: String,
    operator: Option<BinOp>,
    func: FuncId,
    is_static: bool,
    public: bool,
}

/// What checking a method's body found out about how it uses `self`.
#[derive(Default)]
struct SelfUse {
    used: bool,
    /// The first place the body changes a non-`mut` part of `self` itself,
    /// and what it does there (`assigns \`self.x\``).
    changes: Option<(Span, String)>,
    /// Methods called on `self`, or on a part of it that is not `mut`: if
    /// one of them changes its `self`, so does this method.
    calls: Vec<(FuncId, Span)>,
}

/// A method call on a value that must not change. Whether the method
/// changes `self` is only known once every body has been checked.
struct FixedReceiver {
    method: FuncId,
    /// The method's name at the call.
    span: Span,
    /// The receiver as written (`p`, `a.b`).
    text: String,
    /// The binding the receiver belongs to; `None` for a temporary value.
    binding: Option<FixedBinding>,
}

struct FixedBinding {
    name: String,
    ty: Option<Type>,
    is_param: bool,
    span: Span,
}

/// Everything shared by all function bodies.
#[derive(Default)]
struct Globals {
    sigs: Vec<Sig>,
    funcs_by_name: HashMap<String, FuncId>,
    structs: Vec<StructInfo>,
    structs_by_name: HashMap<String, StructId>,
    /// Indexed by `FuncId`; only filled in for methods.
    self_use: Vec<SelfUse>,
    fixed_receivers: Vec<FixedReceiver>,
    /// The automatic `==` function of each struct that needs one.
    eq_funcs: HashMap<StructId, FuncId>,
    /// Automatic `==` functions asked for whose bodies are not built yet.
    eq_pending: Vec<StructId>,
    io_imported: bool,
    /// The file flag `//--!$unsafe: true`.
    unsafe_allowed: bool,
    /// A borrow, pointer or other step 3 node was seen (`hir::Program::uses_borrows`).
    uses_borrows: std::cell::Cell<bool>,
    /// Which structs copy automatically (`hir::StructDef::copy`).
    struct_copy: Vec<bool>,
}

impl Globals {
    fn add_sig(&mut self, sig: Sig) -> FuncId {
        self.sigs.push(sig);
        self.self_use.push(SelfUse::default());
        self.sigs.len() - 1
    }

    fn resolve_base_quiet(&self, t: &ast::TypeName) -> Option<Type> {
        if t.name == "none" {
            return Some(Type::None);
        }
        Type::from_name(&t.name).ok().or_else(|| self.structs_by_name.get(&t.name).map(|&id| self.structs[id].ty))
    }

    /// `t` with its `&`, `&mut` and `*` marks applied to the named type.
    fn resolve_quiet(&self, t: &ast::TypeName) -> Option<Type> {
        let base = self.resolve_base_quiet(t)?;
        Some(t.mods.iter().rev().fold(base, |ty, (m, _)| match m {
            ast::TypeMod::Ref { mutable } => Type::borrow(*mutable, ty),
            ast::TypeMod::Ptr => Type::ptr(ty),
        }))
    }

    fn resolve(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        if let Some((_, span)) = t.mods.first() {
            self.uses_borrows.set(true);
            if !self.unsafe_allowed && let Some((_, at)) = t.mods.iter().find(|(m, _)| *m == ast::TypeMod::Ptr) {
                diags.push(unsafe_disabled(*at, "raw pointer type here"));
            }
            if self.resolve_base_quiet(t) == Some(Type::None) {
                diags.push(Diagnostic::at(&codes::E0901, *span, "a borrow or pointer of `none` is not supported yet"));
                return None;
            }
        }
        if let Some(ty) = self.resolve_quiet(t) {
            return Some(ty);
        }
        match Type::from_name(&t.name) {
            Ok(ty) => Some(ty),
            Err(TypeError::NotYet) => {
                diags.push(Diagnostic::at(&codes::E0901, t.span, format!("the type `{}` is not supported yet", t.name)));
                None
            }
            Err(TypeError::Unknown) => {
                let mut d = Diagnostic::at(&codes::E0201, t.span, format!("unknown type `{}`", t.name));
                let structs: Vec<&str> = self.structs.iter().map(|s| s.name.as_str()).collect();
                if let Some(hint) = type_hint(&t.name) {
                    d = d.help(hint);
                } else if let Some(near) = similar(&t.name, &structs) {
                    d = d.help(format!("did you mean `{near}`?"));
                }
                diags.push(d);
                None
            }
        }
    }

    /// The type of a binding, parameter or field, which cannot be `none`.
    fn resolve_binding_type(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        match self.resolve(t, diags) {
            Some(Type::None) => {
                diags.push(
                    Diagnostic::at(&codes::E0104, t.span, "a binding cannot have type `none`").note("`none` holds no value"),
                );
                None
            }
            ty => ty,
        }
    }

    /// Does a value of type `t` own heap memory: a `string`, or a struct
    /// holding one?
    fn owns(&self, t: Type) -> bool {
        match t {
            Type::Str => true,
            Type::Struct { id, .. } => self.structs[id].owns,
            _ => false,
        }
    }

    /// Does a value of type `t` hold a borrow (so `lives` may name it)?
    fn holds_borrow(&self, t: Type) -> bool {
        match t {
            Type::Ref { .. } => true,
            Type::Struct { id, .. } => !self.structs[id].lives.is_empty(),
            _ => false,
        }
    }

    fn struct_of(&self, t: Type) -> Option<&StructInfo> {
        t.struct_id().map(|id| &self.structs[id])
    }

    fn method(&self, s: StructId, name: &str) -> Option<&MethodInfo> {
        self.structs[s].methods.iter().find(|m| m.name == name && m.operator.is_none())
    }

    fn operator(&self, s: StructId, op: BinOp) -> Option<&MethodInfo> {
        self.structs[s].methods.iter().find(|m| m.operator == Some(op))
    }

    /// The function behind the automatic `==` of struct `s`, created on first use.
    fn eq_func(&mut self, s: StructId) -> FuncId {
        if let Some(&f) = self.eq_funcs.get(&s) {
            return f;
        }
        let info = &self.structs[s];
        let sig = Sig { name: format!("{} ==", info.name), params: vec![Some(info.ty); 2], ret: Some(Type::Bool), span: info.name_span };
        let f = self.add_sig(sig);
        self.eq_funcs.insert(s, f);
        self.eq_pending.push(s);
        f
    }

    /// `a == b` for two values of struct type `s`: its `==` method if it has
    /// one, otherwise the automatic field-by-field comparison. `a` is the
    /// receiver (a place if the struct owns strings) and `b` an owned value.
    fn struct_eq(&mut self, s: StructId, a: hir::Expr, b: hir::Expr) -> hir::Expr {
        let f = match self.operator(s, BinOp::Eq) {
            Some(m) => m.func,
            None => self.eq_func(s),
        };
        hir::Expr::new(hir::ExprKind::Call(f, vec![a, b]), Type::Bool)
    }
}

/// Checks the whole program. The result is only fit for codegen when no
/// errors were reported.
pub fn check(prog: &ast::Program, src: &str, diags: &mut Vec<Diagnostic>) -> hir::Program {
    let mut g = Globals { unsafe_allowed: prog.unsafe_allowed, ..Globals::default() };
    for import in &prog.imports {
        if import.module == "io" {
            g.io_imported = true;
        } else {
            diags.push(
                Diagnostic::at(&codes::E0201, import.span, format!("unknown module `{}`", import.module))
                    .help("`io` is the only module so far"),
            );
        }
    }

    // Struct names first, so that any type can name any struct, wherever it is defined.
    let struct_ids: Vec<Option<StructId>> = prog.structs.iter().map(|s| declare_struct(&mut g, s, diags)).collect();
    for (s, id) in prog.structs.iter().zip(&struct_ids) {
        if let Some(id) = *id {
            declare_fields(&mut g, id, s, diags);
        }
    }
    check_struct_lives(&mut g, prog, &struct_ids, diags);
    let cyclic = report_cycles(&g, diags);
    mark_owning(&mut g, &cyclic);

    // Signatures: functions, then methods.
    let mut func_ids = Vec::new();
    for f in &prog.funcs {
        let sig = Sig {
            name: f.name.clone(),
            params: f.params.iter().map(|p| g.resolve_binding_type(&p.ty, diags)).collect(),
            ret: g.resolve(&f.ret, diags),
            span: f.span,
        };
        check_signature_extras(&g, f, &sig.params, diags);
        if let Some(&first) = g.funcs_by_name.get(&f.name) {
            diags.push(
                Diagnostic::at(&codes::E0202, f.name_span, format!("function `{}` is defined twice", f.name))
                    .note("defined again here")
                    .label(g.sigs[first].span, "first defined here"),
            );
            func_ids.push(None);
            continue;
        }
        let id = g.add_sig(sig);
        g.funcs_by_name.insert(f.name.clone(), id);
        func_ids.push(Some(id));
    }
    let method_ids: Vec<Vec<Option<FuncId>>> = prog
        .structs
        .iter()
        .zip(&struct_ids)
        .map(|(s, id)| match id {
            Some(id) => declare_methods(&mut g, *id, s, diags),
            None => vec![None; s.methods.len()],
        })
        .collect();

    // `copy` depends on which structs have a `drop()`, so layouts wait for the methods.
    let structs = lay_out(&g, &cyclic);
    g.struct_copy = structs.iter().map(|s| s.copy).collect();

    match g.funcs_by_name.get("main").map(|&id| &g.sigs[id]) {
        None => diags.push(
            Diagnostic::new(&codes::E0204, "this program has no `main` function")
                .help("add `none main() { }`; it is where the program starts"),
        ),
        Some(s) if !s.params.is_empty() || !matches!(s.ret, Some(Type::None | Type::S32) | None) => diags.push(
            Diagnostic::at(&codes::E0204, s.span, "`main` has the wrong shape")
                .note("must be `none main()` or `sint32 main()`")
                .help("`sint32 main()` returns the program's exit code"),
        ),
        _ => {}
    }

    // Bodies. Duplicates are still checked, but only the first definition is emitted.
    let mut funcs: Vec<Option<hir::Func>> = Vec::new();
    let emit = |funcs: &mut Vec<Option<hir::Func>>, id: FuncId, f: hir::Func| {
        funcs.resize_with(funcs.len().max(id + 1), || None);
        funcs[id] = Some(f);
    };
    for (f, id) in prog.funcs.iter().zip(&func_ids) {
        let checked = check_body(&mut g, src, diags, f, None, format!("drachm_f_{}", f.name));
        if let Some(id) = *id {
            emit(&mut funcs, id, checked);
        }
    }
    for ((s, sid), mids) in prog.structs.iter().zip(&struct_ids).zip(&method_ids) {
        for (m, mid) in s.methods.iter().zip(mids) {
            let (Some(sid), Some(mid)) = (*sid, *mid) else { continue };
            let word = m.operator.map_or(m.func.name.clone(), |op| format!("op_{}", op_word(op)));
            let symbol = format!("drachm_m_{}{}_{word}", s.name.len(), s.name);
            let ctx = MethodCtx { struct_id: sid, func: mid, static_span: m.is_static };
            let checked = check_body(&mut g, src, diags, &m.func, Some(ctx), symbol);
            emit(&mut funcs, mid, checked);
            if m.is_static.is_none() && m.operator.is_none() && !g.self_use[mid].used && m.func.name != "drop" {
                diags.push(
                    Diagnostic::at(&codes::W0208, m.func.name_span, format!("method `{}` never uses `self`", m.func.name))
                        .note("does not use `self`")
                        .help(format!(
                            "mark it `static` and call it on the struct, `{}::{}()`: write `static {}`",
                            s.name,
                            m.func.name,
                            short(src, m.func.span)
                        )),
                );
            }
        }
    }

    report_fixed_receivers(&g, diags);

    while let Some(s) = g.eq_pending.pop() {
        let f = auto_eq(&mut g, s);
        let id = g.eq_funcs[&s];
        emit(&mut funcs, id, f);
    }

    let main = g.funcs_by_name.get("main").copied().unwrap_or(0);
    hir::Program { funcs: funcs.into_iter().map(|f| f.unwrap_or_else(placeholder)).collect(), structs, main, uses_borrows: g.uses_borrows.get() }
}

/// Stands in for functions whose definition was never emitted (only ever
/// when errors were reported, so it never reaches a backend).
fn placeholder() -> hir::Func {
    let none = Span::new(0, 0);
    hir::Func {
        symbol: String::new(),
        params: 0,
        has_self: false,
        locals: Vec::new(),
        ret: Type::None,
        body: Vec::new(),
        name_span: none,
        span: none,
        body_end: none,
        is_unsafe: false,
        lives: None,
    }
}

/// The source text of `span`, for quoting a declaration in a fix-it.
fn short(src: &str, span: Span) -> &str {
    &src[span.start.min(src.len())..span.end.min(src.len())]
}

fn op_word(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::Div => "div",
        BinOp::Rem => "rem",
        BinOp::Pow => "pow",
        BinOp::Eq => "eq",
        BinOp::Ne => "ne",
        BinOp::Lt => "lt",
        BinOp::Le => "le",
        BinOp::Gt => "gt",
        BinOp::Ge => "ge",
        BinOp::Shl | BinOp::Shr | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::And | BinOp::Or => {
            unreachable!("not an operator method")
        }
    }
}

fn unsafe_disabled(span: Span, what: &str) -> Diagnostic {
    Diagnostic::at(&codes::E0317, span, "`unsafe` is not enabled in this file")
        .note(what)
        .help("add `//--!$unsafe: true` as the first line of the file")
}

/// An `unsafe` function needs the file flag; `lives` names parameters that
/// hold a borrow.
fn check_signature_extras(g: &Globals, f: &ast::Func, params: &[Option<Type>], diags: &mut Vec<Diagnostic>) {
    if let Some(at) = f.is_unsafe
        && !g.unsafe_allowed
    {
        diags.push(unsafe_disabled(at, "needs the file flag"));
    }
    for (name, span) in &f.lives {
        let pos = f.params.iter().position(|p| p.name == *name);
        if pos.is_some_and(|i| params[i].is_some_and(|t| g.holds_borrow(t))) {
            continue;
        }
        let holders: Vec<&str> = f
            .params
            .iter()
            .zip(params)
            .filter(|(_, t)| t.is_some_and(|t| g.holds_borrow(t)))
            .map(|(p, _)| p.name.as_str())
            .collect();
        let help = if holders.is_empty() {
            format!("`{}` has no parameter that holds a borrow, so it needs no `lives`", f.name)
        } else {
            format!("the parameters holding a borrow are {}", list(&holders))
        };
        diags.push(
            Diagnostic::at(&codes::E0309, *span, format!("`lives` names `{name}`, which is not a parameter holding a borrow"))
                .note("not a parameter holding a borrow")
                .help(help),
        );
    }
}

/// A struct's `lives` names exactly its borrow fields (E0314); fills in `StructInfo::lives`.
fn check_struct_lives(g: &mut Globals, prog: &ast::Program, ids: &[Option<StructId>], diags: &mut Vec<Diagnostic>) {
    let mut has_lives = vec![false; g.structs.len()];
    for (s, id) in prog.structs.iter().zip(ids) {
        if let Some(id) = *id {
            has_lives[id] = !s.lives.is_empty();
        }
    }
    for (s, id) in prog.structs.iter().zip(ids) {
        let Some(id) = *id else { continue };
        let fields = &g.structs[id].fields;
        let borrows: Vec<usize> = (0..fields.len()).filter(|&i| fields[i].ty.is_some_and(Type::is_borrow)).collect();
        let mut named = Vec::new();
        for (name, span) in &s.lives {
            match fields.iter().position(|f| f.name == *name).filter(|i| borrows.contains(i)) {
                Some(i) => named.push(i),
                None => diags.push(
                    Diagnostic::at(&codes::E0314, *span, format!("`lives {name}` names nothing `{}` borrows", s.name))
                        .note(format!("`{name}` is not a borrow field"))
                        .help("`lives` lists the fields of the struct that are borrows"),
                ),
            }
        }
        let names: Vec<&str> = borrows.iter().map(|&i| fields[i].name.as_str()).collect();
        if s.lives.is_empty() {
            if let Some(&first) = borrows.first() {
                diags.push(
                    Diagnostic::at(&codes::E0314, s.name_span, format!("`{}` holds a borrow, so it must say what it borrows from", s.name))
                        .label(fields[first].span, "this field is a borrow")
                        .help(format!("add `lives {}`: `struct {} lives {} {{ ... }}`", names.join(", "), s.name, names.join(", "))),
                );
            }
        } else {
            for &i in &borrows {
                if !named.contains(&i) && s.lives.iter().all(|(n, _)| *n != fields[i].name) {
                    diags.push(
                        Diagnostic::at(&codes::E0314, fields[i].span, format!("field `{}` is a borrow, but `lives` does not name it", fields[i].name))
                            .note("a borrow field")
                            .help(format!("add `{}` to the list: `lives {}`", fields[i].name, names.join(", "))),
                    );
                }
            }
        }
        for f in fields {
            let inner = f.ty.map(|t| t.target().filter(|_| t.is_borrow()).unwrap_or(t));
            if inner.and_then(Type::struct_id).is_some_and(|o| has_lives[o]) {
                diags.push(Diagnostic::at(&codes::E0901, f.span, "a field that borrows a struct with `lives` is not supported yet"));
            }
        }
        named.sort_unstable();
        named.dedup();
        g.structs[id].lives = named;
    }
}

fn declare_struct(g: &mut Globals, s: &ast::Struct, diags: &mut Vec<Diagnostic>) -> Option<StructId> {
    if s.name == "none" || !matches!(Type::from_name(&s.name), Err(TypeError::Unknown)) {
        diags.push(
            Diagnostic::at(&codes::E0202, s.name_span, format!("`{}` is the name of a built-in type", s.name))
                .note("already a type")
                .help("give the struct another name"),
        );
        return None;
    }
    if let Some(&first) = g.structs_by_name.get(&s.name) {
        diags.push(
            Diagnostic::at(&codes::E0202, s.name_span, format!("struct `{}` is defined twice", s.name))
                .note("defined again here")
                .label(g.structs[first].name_span, "first defined here"),
        );
        return None;
    }
    let id = g.structs.len();
    let ty = Type::Struct { id, name: types::intern(&s.name) };
    g.structs.push(StructInfo { name: s.name.clone(), name_span: s.name_span, ty, fields: Vec::new(), methods: Vec::new(), owns: false, stays: s.stays.is_some(), lives: Vec::new(), has_drop: None });
    g.structs_by_name.insert(s.name.clone(), id);
    Some(id)
}

fn declare_fields(g: &mut Globals, id: StructId, s: &ast::Struct, diags: &mut Vec<Diagnostic>) {
    for f in &s.fields {
        let ty = g.resolve_binding_type(&f.ty, diags);
        if let Some(first) = g.structs[id].fields.iter().find(|o| o.name == f.name) {
            diags.push(
                Diagnostic::at(&codes::E0202, f.name_span, format!("`{}` has two fields named `{}`", s.name, f.name))
                    .note("defined again here")
                    .label(first.span, "first defined here"),
            );
            continue;
        }
        let info = FieldInfo { name: f.name.clone(), ty, mutable: f.mutable, public: f.public, span: f.name_span };
        g.structs[id].fields.push(info);
    }
}

/// Signatures of struct `id`'s methods, checking operator methods' shapes.
fn declare_methods(g: &mut Globals, id: StructId, s: &ast::Struct, diags: &mut Vec<Diagnostic>) -> Vec<Option<FuncId>> {
    let self_ty = g.structs[id].ty;
    let mut ids = Vec::new();
    for m in &s.methods {
        let f = &m.func;
        let sig = Sig {
            name: format!("{}::{}", s.name, f.name),
            params: f.params.iter().map(|p| g.resolve_binding_type(&p.ty, diags)).collect(),
            ret: g.resolve(&f.ret, diags),
            span: f.span,
        };
        check_signature_extras(g, f, &sig.params, diags);
        let mut is_drop = false;
        if f.name == "drop" && m.operator.is_none() {
            if m.is_static.is_some() || !f.params.is_empty() || sig.ret != Some(Type::None) || f.is_unsafe.is_some() {
                diags.push(
                    Diagnostic::at(&codes::E0328, f.span, "`drop` must be written `none drop()`")
                        .note("`drop` runs by itself when the value is destroyed, so it takes nothing and returns nothing")
                        .help("write `none drop() { ... }`, not `static`, with no parameters"),
                );
            } else {
                is_drop = true;
            }
        }
        if let Some(op) = m.operator
            && !operator_shape_ok(op, m, &sig, self_ty, &s.name, diags)
        {
            ids.push(None);
            continue;
        }
        let info = &g.structs[id];
        let clash = if let Some(first) = info.methods.iter().find(|o| o.name == f.name) {
            let what = if m.operator.is_some() { format!("`{}` operators", f.name) } else { format!("methods named `{}`", f.name) };
            Some((format!("`{}` has two {what}", s.name), g.sigs[first.func].span, "first defined here"))
        } else {
            // Undecided by the SPEC, chosen here: a field and a method may not share a name.
            info.fields.iter().find(|o| o.name == f.name).map(|field| {
                (format!("`{}` has a field and a method both named `{}`", s.name, f.name), field.span, "the field")
            })
        };
        if let Some((msg, other, what)) = clash {
            diags.push(Diagnostic::at(&codes::E0202, f.name_span, msg).note("defined again here").label(other, what));
            ids.push(None);
            continue;
        }
        let func = g.add_sig(sig);
        let info = MethodInfo { name: f.name.clone(), operator: m.operator, func, is_static: m.is_static.is_some(), public: m.public };
        g.structs[id].methods.push(info);
        if is_drop {
            g.structs[id].has_drop = Some(func);
        }
        ids.push(Some(func));
    }
    ids
}

/// Operator methods take exactly one parameter (the right-hand side) and
/// are never static. Reports and returns false if `m` breaks the rules.
fn operator_shape_ok(op: BinOp, m: &ast::Method, sig: &Sig, self_ty: Type, sname: &str, diags: &mut Vec<Diagnostic>) -> bool {
    let f = &m.func;
    let sym = op.symbol();
    let problem = if !matches!(op.class(), OpClass::Arithmetic | OpClass::Comparison) {
        Some((f.name_span, format!("`{sym}` cannot be an operator method"), "operator methods are `+ - * / % **` and `== != < <= > >=`".to_string()))
    } else if let Some(st) = m.is_static {
        Some((st, format!("operator `{sym}` cannot be `static`"), format!("`a {sym} b` calls it on `a`, so it needs a `self`: remove `static`")))
    } else if f.params.len() != 1 {
        Some((f.span, format!("operator `{sym}` takes 1 parameter, but has {}", f.params.len()), format!("it takes the right-hand side: `{} {sym}({sname} o)`", f.ret.name)))
    } else if op.class() == OpClass::Comparison && sig.ret.is_some_and(|t| t != Type::Bool) {
        Some((f.ret.span, format!("operator `{sym}` must return `bool`"), format!("write `bool {sym}({} {})`", f.params[0].ty.name, f.params[0].name)))
    } else if matches!(op, BinOp::Eq | BinOp::Ne) && sig.params[0].is_some_and(|t| t != self_ty) {
        // Undecided by the SPEC, chosen here: `==` and `!=` compare a struct
        // with its own type only, so the automatic `==` of an outer struct can use them.
        Some((f.params[0].ty.span, format!("operator `{sym}` must take an argument of type `{sname}`"), format!("write `bool {sym}({sname} {})`", f.params[0].name)))
    } else {
        None
    };
    match problem {
        Some((span, msg, help)) => {
            diags.push(Diagnostic::at(&codes::E0113, span, msg).help(help));
            false
        }
        None => true,
    }
}

/// A struct that contains itself (directly or through other structs) would
/// be infinitely large. Reports each cycle once; returns which structs are in one.
fn report_cycles(g: &Globals, diags: &mut Vec<Diagnostic>) -> Vec<bool> {
    let n = g.structs.len();
    let mut cyclic = vec![false; n];
    let mut done = vec![false; n];
    // Depth-first, with the path from the start as (struct, field) pairs.
    fn visit(g: &Globals, s: StructId, path: &mut Vec<(StructId, usize)>, done: &mut [bool], cyclic: &mut [bool], diags: &mut Vec<Diagnostic>) {
        if done[s] {
            return;
        }
        if let Some(start) = path.iter().position(|&(p, _)| p == s) {
            let cycle = &path[start..];
            if cycle.iter().any(|&(p, _)| cyclic[p]) {
                return;
            }
            let name = &g.structs[s].name;
            let (first, first_field) = cycle[0];
            let f = &g.structs[first].fields[first_field];
            let mut d = Diagnostic::at(&codes::E0210, f.span, format!("struct `{name}` contains itself"))
                .note(format!("`{}` has type `{}`", f.name, f.ty.unwrap()));
            for &(p, i) in &cycle[1..] {
                let f = &g.structs[p].fields[i];
                d = d.label(f.span, format!("`{}` has type `{}`", f.name, f.ty.unwrap()));
            }
            let holds: Vec<String> = cycle
                .iter()
                .map(|&(p, i)| format!("`{}` holds `{}`", g.structs[p].name, g.structs[p].fields[i].ty.unwrap()))
                .collect();
            d = d
                .note_line(format!("{}: its size would never end", holds.join(", ")))
                .help("a struct cannot hold a value of its own type, directly or through other structs");
            diags.push(d);
            for &(p, _) in cycle {
                cyclic[p] = true;
            }
            return;
        }
        for (i, f) in g.structs[s].fields.iter().enumerate() {
            if let Some(inner) = f.ty.and_then(Type::struct_id) {
                path.push((s, i));
                visit(g, inner, path, done, cyclic, diags);
                path.pop();
            }
        }
        done[s] = true;
    }
    for s in 0..n {
        visit(g, s, &mut Vec::new(), &mut done, &mut cyclic, diags);
    }
    cyclic
}

/// Which structs own strings: those with a `string` field, or a field of a
/// struct that does. Structs in a cycle (already reported) own nothing.
fn mark_owning(g: &mut Globals, cyclic: &[bool]) {
    loop {
        let mut grew = false;
        for (s, &in_cycle) in cyclic.iter().enumerate() {
            if g.structs[s].owns || in_cycle {
                continue;
            }
            let owns = g.structs[s].fields.iter().any(|f| match f.ty {
                Some(Type::Str) => true,
                Some(Type::Struct { id, .. }) => g.structs[id].owns,
                _ => false,
            });
            if owns {
                g.structs[s].owns = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
}

/// Which structs copy automatically, ignoring the size limit (`lay_out`
/// adds it): not `stays`, no `drop()`, and every field Copy. Found by
/// starting from "all" and removing, so a struct only needs its fields' answers.
fn copy_structs(g: &Globals, cyclic: &[bool], defs: &[hir::StructDef]) -> Vec<bool> {
    let mut copy: Vec<bool> = g.structs.iter().enumerate().map(|(i, s)| !s.stays && s.has_drop.is_none() && !cyclic[i]).collect();
    loop {
        let mut changed = false;
        for (i, s) in g.structs.iter().enumerate() {
            if !copy[i] {
                continue;
            }
            let ok = defs[i].size <= 32
                && s.fields.iter().all(|f| match f.ty {
                    Some(Type::Int { .. } | Type::Float(_) | Type::Bool | Type::Ref { mutable: false, .. }) => true,
                    Some(Type::Struct { id, .. }) => copy[id],
                    _ => false,
                });
            if !ok {
                copy[i] = false;
                changed = true;
            }
        }
        if !changed {
            return copy;
        }
    }
}

/// Layouts for every struct. Structs in a cycle (already reported) get an empty one.
fn lay_out(g: &Globals, cyclic: &[bool]) -> Vec<hir::StructDef> {
    fn size_of(g: &Globals, cyclic: &[bool], memo: &mut [Option<(u32, u32)>], t: Option<Type>) -> (u32, u32) {
        let Some(t) = t else { return (0, 1) };
        if t == Type::Str {
            return (hir::STR_SIZE, 8);
        }
        if let Some(n) = hir::scalar_size(t) {
            return (n, n);
        }
        let Some(s) = t.struct_id() else { return (0, 1) };
        if cyclic[s] {
            return (0, 1);
        }
        if let Some(m) = memo[s] {
            return m;
        }
        let sizes: Vec<(u32, u32)> = g.structs[s].fields.iter().map(|f| size_of(g, cyclic, memo, f.ty)).collect();
        let (_, size, align) = hir::layout(&sizes);
        memo[s] = Some((size, align));
        (size, align)
    }
    let mut memo = vec![None; g.structs.len()];
    let mut defs: Vec<hir::StructDef> = g
        .structs
        .iter()
        .enumerate()
        .map(|(id, s)| {
            let sizes: Vec<(u32, u32)> = if cyclic[id] {
                Vec::new()
            } else {
                s.fields.iter().map(|f| size_of(g, cyclic, &mut memo, f.ty)).collect()
            };
            let (offsets, size, align) = hir::layout(&sizes);
            let fields = s
                .fields
                .iter()
                .zip(offsets.into_iter().chain(std::iter::repeat(0)))
                .map(|(f, offset)| hir::FieldDef { name: f.name.clone(), ty: f.ty.unwrap_or(Type::None), offset, mutable: f.mutable })
                .collect();
            hir::StructDef {
                name: s.name.clone(),
                fields,
                size,
                align,
                owns: s.owns,
                copy: false,
                stays: s.stays,
                has_drop: s.has_drop,
                lives: s.lives.clone(),
            }
        })
        .collect();
    let copy = copy_structs(g, cyclic, &defs);
    for (d, c) in defs.iter_mut().zip(copy) {
        d.copy = c;
    }
    defs
}

/// After every body is checked: which methods change `self`, and the calls
/// of such methods on values that must not change.
fn report_fixed_receivers(g: &Globals, diags: &mut Vec<Diagnostic>) {
    // A method changes `self` if its body does, or if it calls such a method on `self`.
    let mut changes: Vec<Option<(Span, String)>> = g.self_use.iter().map(|u| u.changes.clone()).collect();
    loop {
        let mut grew = false;
        for (f, u) in g.self_use.iter().enumerate() {
            if changes[f].is_some() {
                continue;
            }
            if let Some(&(callee, at)) = u.calls.iter().find(|(c, _)| changes[*c].is_some()) {
                let name = g.sigs[callee].name.rsplit("::").next().unwrap_or_default();
                changes[f] = Some((at, format!("calls `{name}`, which changes `self`")));
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    for r in &g.fixed_receivers {
        let Some((at, why)) = &changes[r.method] else { continue };
        let m = g.sigs[r.method].name.rsplit("::").next().unwrap_or_default();
        let d = match &r.binding {
            Some(b) => {
                let ty = b.ty.map_or(String::new(), |t| format!("{t} "));
                let whose = if b.is_param { "parameter " } else { "" };
                Diagnostic::at(&codes::E0205, r.span, format!("cannot call `{m}` on `{}`: it changes the value, and {whose}`{}` is not `mut`", r.text, b.name))
                    .note(format!("`{m}` changes `{}`", r.text))
                    .label(*at, format!("`{m}` changes `self` here: it {why}"))
                    .label(b.span, "declared here without `mut`")
                    .help(format!("declare it as `mut {ty}{}`, or make the fields `{m}` changes `mut`", b.name))
            }
            None => Diagnostic::at(&codes::E0205, r.span, format!("cannot call `{m}` on a temporary value: it changes the value"))
                .note(format!("`{m}` changes `self`"))
                .label(*at, format!("`{m}` changes `self` here: it {why}"))
                .help(format!("store the value in a `mut` binding first, then call `{m}` on that")),
        };
        diags.push(d);
    }
}

/// The automatic `==` of struct `s`: every field equal, in order, with
/// strings compared by their text and struct fields by their own `==`.
/// It works like a method: `a` is the receiver, borrowed, and `b` is owned
/// (and dropped).
fn auto_eq(g: &mut Globals, s: StructId) -> hir::Func {
    let info = &g.structs[s];
    let (ty, name, owns) = (info.ty, info.name.clone(), info.owns);
    let fields: Vec<Type> = info.fields.iter().map(|f| f.ty.unwrap_or(Type::Bool)).collect();
    let side = |local, i, fty| {
        hir::Expr::new(hir::ExprKind::Field(Box::new(hir::Expr::new(hir::ExprKind::Local(local), ty)), i), fty)
    };
    let mut all: Option<hir::Expr> = None;
    for (i, fty) in fields.into_iter().enumerate() {
        let (a, b) = (side(0, i, fty), side(1, i, fty));
        let same = match fty.struct_id() {
            Some(inner) => {
                let b = if g.owns(fty) { copy(b) } else { b };
                g.struct_eq(inner, a, b)
            }
            None if fty == Type::Str => hir::Expr::new(hir::ExprKind::StrCall(hir::StrFn::Eq, vec![a, b]), Type::Bool),
            None => hir::Expr::new(hir::ExprKind::Binary(hir::BinOp::Eq, Box::new(a), Box::new(b)), Type::Bool),
        };
        all = Some(match all {
            None => same,
            Some(prev) => hir::Expr::new(hir::ExprKind::Logic { and: true, lhs: Box::new(prev), rhs: Box::new(same) }, Type::Bool),
        });
    }
    let result = all.unwrap_or(hir::Expr::new(hir::ExprKind::Bool(true), Type::Bool));
    let result = if owns { scoped(result, vec![1]) } else { result };
    let none = Span::new(0, 0);
    let param = |n: &str| hir::Local { name: n.into(), ty, span: none, mutability: hir::Mutability::Fixed, is_param: true };
    hir::Func {
        symbol: format!("drachm_eq_{name}"),
        params: 2,
        has_self: true,
        locals: vec![param("a"), param("b")],
        ret: Type::Bool,
        body: vec![hir::Stmt::Return(Some(result))],
        name_span: none,
        span: none,
        body_end: none,
        is_unsafe: false,
        lives: None,
    }
}

/// The method whose body is being checked.
#[derive(Clone, Copy)]
struct MethodCtx {
    struct_id: StructId,
    func: FuncId,
    /// The `static` keyword, if the method has one.
    static_span: Option<Span>,
}

fn check_body(g: &mut Globals, src: &str, diags: &mut Vec<Diagnostic>, f: &ast::Func, method: Option<MethodCtx>, symbol: String) -> hir::Func {
    let ret = g.resolve_quiet(&f.ret);
    let mut cx = Checker {
        src,
        g,
        diags,
        ret,
        method,
        scopes: vec![HashMap::new()],
        owned: vec![Vec::new()],
        temps: Vec::new(),
        locals: Vec::new(),
        targets: Vec::new(),
        labels: 0,
    };
    let has_self = method.is_some_and(|m| m.static_span.is_none());
    if let Some(m) = method.filter(|_| has_self) {
        let ty = cx.g.structs[m.struct_id].ty;
        let id = cx.declare("self", Some(ty), false, f.name_span, true);
        cx.locals[id].is_self = true;
        // `self` is the caller's value: borrowed, never dropped here.
        cx.owned[0].retain(|&o| o != id);
    }
    for p in &f.params {
        let ty = cx.g.resolve_quiet(&p.ty).filter(|t| *t != Type::None);
        let id = cx.declare(&p.name, ty, p.mutable, p.span, true);
        cx.locals[id].open = p.open;
        if p.mutable && p.open {
            let at = short(src, p.span).find("open").map_or(p.span, |i| Span::new(p.span.start + i, p.span.start + i + 4));
            cx.diags.push(
                Diagnostic::at(&codes::W0323, at, "`mut open` says the same as `mut`")
                    .note("`mut` already allows everything `open` does")
                    .help(format!("write `mut {} {}`", short(src, p.ty.span), p.name)),
            );
        }
    }
    let mut body = cx.block(&f.body);
    // Parameters belong to the function and are dropped at its end.
    body.extend(cx.owned[0].iter().rev().map(|&id| hir::Stmt::Drop(hir::Place::local(id))));
    let locals = cx
        .locals
        .into_iter()
        .map(|l| hir::Local {
            name: l.name,
            ty: l.ty.unwrap_or(Type::None),
            span: l.span,
            mutability: if l.open {
                hir::Mutability::Open
            } else if l.mutable {
                hir::Mutability::Mut
            } else {
                hir::Mutability::Fixed
            },
            is_param: l.is_param,
        })
        .collect();
    hir::Func {
        name_span: f.name_span,
        span: f.span,
        body_end: f.body.end,
        is_unsafe: f.is_unsafe.is_some(),
        lives: None,
        symbol,
        params: f.params.len() + has_self as usize,
        has_self,
        locals,
        ret: ret.unwrap_or(Type::None),
        body,
    }
}

/// People arriving from other languages reach for these names.
fn type_hint(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let suggestion = match lower.as_str() {
        "int" | "i32" | "integer" | "long" | "short" => "sint32",
        "i8" => "sint8",
        "i16" => "sint16",
        "i64" | "isize" => "sint64",
        "i128" => "sint128",
        "uint" | "u32" | "unsigned" => "uint32",
        "u8" | "byte" => "uint8",
        "char" | "character" | "rune" => {
            return Some("Drachm has no character type: a character is a one-character `string`".into());
        }
        "u16" => "uint16",
        "u64" | "usize" | "size_t" => "uint64",
        "u128" => "uint128",
        "float" | "double" | "f64" | "number" | "real" => "float64",
        "f32" => "float32",
        "boolean" => "bool",
        "string" | "text" => "string",
        "void" | "nothing" | "unit" => "none",
        "fn" | "func" | "function" | "def" | "fun" => {
            return Some("there is no function keyword: a function starts with its return type, like `none main() { }`".into());
        }
        _ if lower.starts_with("uint") || lower.starts_with("sint") => {
            return Some("integer types come in 8, 16, 32, 64 and 128 bits".into());
        }
        _ if lower.starts_with("float") => return Some("the float types are `float32` and `float64`".into()),
        _ => return None,
    };
    Some(format!("Drachm calls this `{suggestion}`"))
}

/// The candidate closest to `name`, if one is close enough to be a typo.
fn similar<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    fn distance(a: &str, b: &str) -> usize {
        let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.iter().enumerate() {
            let mut prev = row[0];
            row[0] = i + 1;
            for (j, cb) in b.iter().enumerate() {
                let cur = row[j + 1];
                row[j + 1] = (prev + (ca != cb) as usize).min(row[j] + 1).min(cur + 1);
                prev = cur;
            }
        }
        row[b.len()]
    }
    let limit = (name.chars().count() / 3).max(1);
    candidates.iter().copied().map(|c| (distance(name, c), c)).filter(|(d, _)| *d <= limit).min_by_key(|(d, _)| *d).map(|(_, c)| c)
}

/// "`a`, `b` and `c`"
fn list(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// An expression is either still an untyped constant or already has a type.
enum Val {
    Const(Const),
    Expr(hir::Expr),
}

/// One value of the construction shorthand, checked: its field name if
/// given, the value (`None` after an error), and where it came from.
type GivenValue = (Option<(String, Span)>, Option<Val>, Origin);

/// Where a value came from in the source, for messages about it.
#[derive(Clone, Copy)]
struct Origin {
    span: Span,
    /// Needs parentheses before `::to<T>()` can be appended.
    compound: bool,
}

fn origin(e: &ast::Expr) -> Origin {
    let compound = !e.paren && matches!(e.kind, ExprKind::Unary(..) | ExprKind::Binary(..) | ExprKind::IncDec { .. });
    Origin { span: e.span, compound }
}

struct LocalInfo {
    name: String,
    ty: Option<Type>,
    mutable: bool,
    is_param: bool,
    /// A method's `self`.
    is_self: bool,
    /// Declared `open` (stored; checked like `mut` until WP5).
    open: bool,
    span: Span,
}

impl PlaceInfo {
    fn hir_place(&self, span: Span) -> hir::Place {
        hir::Place { local: self.local, proj: self.proj.clone(), span }
    }
}

/// A local, then fields and dereferences inside it, resolved but not read.
struct PlaceInfo {
    local: LocalId,
    proj: Vec<hir::HirProj>,
    ty: Type,
    /// Some step on the path goes through a borrow or a pointer.
    through_deref: bool,
    /// Some field on the path is `mut`, so the place can change whatever its binding is.
    mut_field: bool,
}

/// Can the value a method is called on change?
enum Receiver {
    /// A `mut` binding, or behind a `mut` field.
    Changeable,
    /// `self` or a non-`mut` part of it, inside a method.
    OfSelf,
    /// A binding (or a non-`mut` part of one) that is not `mut`.
    Fixed(LocalId),
    Temporary,
}

struct Checker<'a> {
    src: &'a str,
    g: &'a mut Globals,
    diags: &'a mut Vec<Diagnostic>,
    /// `None` if the return type did not resolve.
    ret: Option<Type>,
    method: Option<MethodCtx>,
    scopes: Vec<HashMap<String, LocalId>>,
    /// The bindings of an owning type in each scope, in declaration order,
    /// to drop when it ends.
    owned: Vec<Vec<LocalId>>,
    /// Hidden locals holding the current statement's temporaries.
    temps: Vec<LocalId>,
    locals: Vec<LocalInfo>,
    /// The loops and `if`/`match` levels around the current statement, innermost last.
    targets: Vec<Target>,
    /// Labels handed out so far in this function.
    labels: hir::Label,
}

fn int_lit(bits: u128, ty: Type) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Int(bits), ty)
}

fn str_lit(text: String) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Str(text), Type::Str)
}

/// A deep copy of the place `e`.
fn copy(e: hir::Expr) -> hir::Expr {
    let ty = e.ty;
    hir::Expr::new(hir::ExprKind::Copy(Box::new(e)), ty)
}

/// `e`, followed by dropping `drops`.
fn scoped(e: hir::Expr, drops: Vec<LocalId>) -> hir::Expr {
    if drops.is_empty() {
        return e;
    }
    let ty = e.ty;
    hir::Expr::new(hir::ExprKind::Scoped(Box::new(e), drops), ty)
}

/// Does the place expression go through a `Deref`?
fn has_deref(e: &hir::Expr) -> bool {
    match &e.kind {
        hir::ExprKind::Deref(_) => true,
        hir::ExprKind::Field(inner, _) => has_deref(inner),
        _ => false,
    }
}

/// Is the written expression a place: a variable, `self`, a field of a
/// place, or a dereference?
fn ast_place(e: &ast::Expr) -> bool {
    match &e.kind {
        ExprKind::Var(_) | ExprKind::SelfValue => true,
        ExprKind::Field { recv, .. } => ast_place(recv),
        ExprKind::Deref(inner) => ast_place(inner),
        _ => false,
    }
}

/// A place with no effects to evaluate (no `Temp` in it): a statement that
/// is only this does nothing.
fn plain_place(e: &hir::Expr) -> bool {
    match &e.kind {
        hir::ExprKind::Local(_) | hir::ExprKind::Str(_) => true,
        hir::ExprKind::Field(inner, _) | hir::ExprKind::Deref(inner) => plain_place(inner),
        _ => false,
    }
}

impl Checker<'_> {
    fn lookup(&self, name: &str) -> Option<LocalId> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn declare(&mut self, name: &str, ty: Option<Type>, mutable: bool, span: Span, is_param: bool) -> LocalId {
        let id = self.locals.len();
        let info = LocalInfo { name: name.to_string(), ty, mutable, is_param, is_self: false, open: false, span };
        self.locals.push(info);
        self.scopes.last_mut().unwrap().insert(name.to_string(), id);
        if ty.is_some_and(|t| self.g.owns(t)) {
            self.owned.last_mut().unwrap().push(id);
        }
        id
    }

    /// A local the program cannot name, for a value the lowering needs to keep.
    fn hidden(&mut self, ty: Type) -> LocalId {
        let name = format!("${}", self.locals.len());
        self.hidden_named(&name, ty)
    }

    /// `hidden`, with a name that says what it holds (for reading the HIR).
    fn hidden_named(&mut self, name: &str, ty: Type) -> LocalId {
        let id = self.locals.len();
        let span = Span::new(0, 0);
        let info = LocalInfo { name: name.to_string(), ty: Some(ty), mutable: true, is_param: false, is_self: false, open: false, span };
        self.locals.push(info);
        id
    }

    /// A condition (`if`, `while`, `for`): a `bool` whose temporaries die
    /// as soon as it has been tested.
    fn cond(&mut self, e: &ast::Expr) -> Option<hir::Expr> {
        let c = self.coerce(e, Type::Bool);
        let temps = self.take_temps();
        c.map(|c| scoped(c, temps))
    }

    /// `e`, of an owning type, as a place: an owned value goes into a
    /// temporary that dies at the end of the statement.
    fn place(&mut self, e: hir::Expr) -> hir::Expr {
        if e.is_place() || matches!(e.kind, hir::ExprKind::Str(_)) {
            return e;
        }
        let id = self.hidden(e.ty);
        self.temps.push(id);
        let ty = e.ty;
        hir::Expr::new(hir::ExprKind::Temp(id, Box::new(e)), ty)
    }

    /// `e` as a value something else will own. Owning values are deep-copied
    /// on every assignment, pass and return (moves arrive with step 3).
    // OPEN: a struct holding a string is deep-copied like a string (the
    // step 2 rule); SPEC section 6 says it moves, which needs step 3.
    fn owned(&self, e: hir::Expr) -> hir::Expr {
        if e.is_place() && self.g.owns(e.ty) { copy(e) } else { e }
    }

    /// The by-value operand of an operator (`==`, an operator method): the
    /// callee gets a copy and the operand stays where it is, so the MIR must
    /// see a written copy (`Scoped` around `Copy`, see `copy_expr`).
    fn copied(&self, e: hir::Expr) -> hir::Expr {
        if e.is_place() && matches!(e.ty, Type::Str | Type::Struct { .. }) && !self.copies_anyway(e.ty) {
            return self.written_copy(e);
        }
        e
    }

    /// The copy of the place `e` that the program wrote: a legacy `Copy` for
    /// the backends if it owns what it copies (a plain struct is copied by
    /// using it), inside an empty `Scoped` that tells the MIR builder.
    fn written_copy(&self, e: hir::Expr) -> hir::Expr {
        let ty = e.ty;
        let inner = if self.g.owns(ty) { copy(e) } else { e };
        hir::Expr::new(hir::ExprKind::Scoped(Box::new(inner), Vec::new()), ty)
    }

    /// The receiver of a method: for a struct that owns strings, a place.
    fn receiver_value(&mut self, e: hir::Expr) -> hir::Expr {
        if self.g.owns(e.ty) { self.place(e) } else { e }
    }

    /// The temporaries made since the last call, in the order to drop them.
    fn take_temps(&mut self) -> Vec<LocalId> {
        let mut t = std::mem::take(&mut self.temps);
        t.reverse();
        t
    }

    /// Every owning binding in scope, innermost and latest first: what a
    /// `return` drops.
    fn all_owned(&self) -> Vec<LocalId> {
        self.owned_from(0)
    }

    /// The owning bindings of scope `depth` and every scope inside it,
    /// innermost and latest first: what a jump out of them drops.
    fn owned_from(&self, depth: usize) -> Vec<LocalId> {
        self.owned[depth..].iter().rev().flat_map(|s| s.iter().rev().copied()).collect()
    }

    /// Statements dropping `locals`.
    fn drop_all(locals: Vec<LocalId>) -> impl Iterator<Item = hir::Stmt> {
        locals.into_iter().map(|id| hir::Stmt::Drop(hir::Place::local(id)))
    }

    fn text(&self, span: Span) -> &str {
        short(self.src, span)
    }

    /// Source text of a value for use inside a message, if it is short.
    fn quote(&self, span: Span, fallback: &str) -> String {
        let t = self.text(span);
        if t.len() <= 32 && !t.contains('\n') { format!("`{t}`") } else { fallback.to_string() }
    }

    fn block(&mut self, b: &ast::Block) -> hir::Block {
        self.scopes.push(HashMap::new());
        self.owned.push(Vec::new());
        let mut out = Vec::new();
        for s in b {
            self.stmt(s, &mut out);
        }
        self.scopes.pop();
        let owned = self.owned.pop().unwrap();
        out.extend(Self::drop_all(owned.into_iter().rev().collect()));
        out
    }

    fn stmt(&mut self, s: &Stmt, out: &mut hir::Block) {
        match s {
            Stmt::Let { mutable, open, ty, name, name_span, value, span } => {
                if *mutable && *open {
                    let at = short(self.src, *span).find("open").map_or(*span, |i| Span::new(span.start + i, span.start + i + 4));
                    self.diags.push(
                        Diagnostic::at(&codes::W0323, at, "`mut open` says the same as `mut`")
                            .note("`mut` already allows everything `open` does")
                            .help(format!("write `mut {} {}`", short(self.src, ty.span), name)),
                    );
                }
                let ty = self.g.resolve_binding_type(ty, self.diags);
                // The value is checked before the name exists, so `uint32 x = x;`
                // refers to an outer `x`.
                let init = value.as_ref().map(|v| self.init(v, ty).map(|e| self.owned(e)));
                let temps = self.take_temps();
                let id = self.declare(name, ty, *mutable, *name_span, false);
                self.locals[id].open = *open;
                out.push(hir::Stmt::Decl(id));
                if let Some(init) = init {
                    if let Some(e) = init {
                        out.push(hir::Stmt::Assign(hir::Place::local(id), scoped(e, temps)));
                    }
                }
            }
            Stmt::Assign { target, op, value, span } => match &target.kind {
                ExprKind::Var(name) => self.assign_local(name, target.span, *op, value, *span, out),
                ExprKind::SelfValue => {
                    // Undecided by the SPEC, chosen here: `self` cannot be replaced as a whole.
                    if self.self_local(target.span).is_some() {
                        self.diags.push(
                            Diagnostic::at(&codes::E0005, target.span, "`self` cannot be assigned as a whole")
                                .note("assigned here")
                                .help("assign its fields one by one: `self.x = ...;`"),
                        );
                    }
                    self.expr(value);
                    self.take_temps();
                }
                _ => self.assign_field(target, *op, value, *span, out),
            },
            Stmt::If { branches, els, span } => {
                let label = self.new_label();
                let mut parts: Vec<(Test, &ast::Block)> = branches.iter().map(|(c, b)| (Test::Cond(c), b)).collect();
                if let Some(els) = els {
                    parts.push((Test::Always, els));
                }
                if let Some(branches) = self.chain(label, *span, false, parts, false) {
                    out.push(hir::Stmt::If { label, branches });
                }
            }
            Stmt::While { cond, body, span } => {
                let c = self.cond(cond);
                let label = self.new_label();
                let end = body.end;
                let (body, step) = self.looped(label, *span, body, None);
                if let Some(c) = c {
                    out.push(hir::Stmt::Loop { label, cond: Some(c), body, step, end });
                }
            }
            Stmt::For { init, cond, step, body, span } => {
                // The init binding belongs to the loop: it is dropped after it.
                self.scopes.push(HashMap::new());
                self.owned.push(Vec::new());
                if let Some(init) = init {
                    self.stmt(init, out);
                }
                let c = cond.as_ref().map(|c| self.cond(c));
                let label = self.new_label();
                let end = body.end;
                let (body, step) = self.looped(label, *span, body, step.as_deref());
                self.scopes.pop();
                let owned = self.owned.pop().unwrap();
                match c {
                    Some(None) => {}
                    c => out.push(hir::Stmt::Loop { label, cond: c.flatten(), body, step, end }),
                }
                out.extend(Self::drop_all(owned.into_iter().rev().collect()));
            }
            Stmt::ForEach { .. } => self.for_each(s, out),
            Stmt::Match { value, cases, span } => self.match_stmt(value, cases, *span, out),
            // A jump drops what is in scope between it and its target.
            Stmt::Break { cont, count, span } => {
                if let Some((j, depth)) = self.break_target(*cont, *count, *span) {
                    out.extend(Self::drop_all(self.owned_from(depth)));
                    out.push(hir::Stmt::Jump(j));
                }
            }
            Stmt::Skip { target, span } => {
                if let Some((j, depth)) = self.skip_target(*target, *span) {
                    out.extend(Self::drop_all(self.owned_from(depth)));
                    out.push(hir::Stmt::Jump(j));
                }
            }
            Stmt::Return { value, span } => {
                let v = match (value, self.ret) {
                    (Some(v), Some(Type::None)) => {
                        self.expr(v);
                        self.diags.push(
                            Diagnostic::at(&codes::E0101, v.span, "this function returns nothing, but a value is returned")
                                .note("remove this value")
                                .help("or change the function's return type from `none`"),
                        );
                        None
                    }
                    (Some(v), Some(t)) => self.coerce(v, t).map(|e| Some(self.owned(e))),
                    (Some(v), None) => {
                        self.expr(v);
                        None
                    }
                    (None, Some(t)) if t != Type::None => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0105, *span, format!("this function must return a `{t}`")).note("returns nothing"),
                        );
                        None
                    }
                    (None, _) => Some(None),
                };
                // The value is made first: it may read what is dropped.
                let mut drops = self.take_temps();
                drops.extend(self.all_owned());
                match v {
                    Some(Some(e)) => out.push(hir::Stmt::Return(Some(scoped(e, drops)))),
                    Some(None) => {
                        out.extend(Self::drop_all(drops));
                        out.push(hir::Stmt::Return(None));
                    }
                    None => {}
                }
            }
            Stmt::Unsafe { body, span } => {
                if !self.g.unsafe_allowed {
                    self.diags.push(unsafe_disabled(Span::new(span.start, span.start + 6), "needs the file flag"));
                }
                let block = self.block(body);
                out.push(hir::Stmt::Unsafe(block));
            }
            Stmt::Destroy { target, span } => {
                self.g.uses_borrows.set(true);
                let local = match &target.kind {
                    ExprKind::Var(n) => self.lookup(n),
                    _ => None,
                };
                let dropper = local.and_then(|id| self.locals[id].ty).and_then(Type::struct_id).and_then(|s| self.g.structs[s].has_drop);
                match local {
                    Some(id) if dropper.is_some() => {
                        self.read_local(id, target.span);
                        out.push(hir::Stmt::Destroy(hir::Place { local: id, proj: Vec::new(), span: target.span }));
                    }
                    _ => {
                        if matches!(target.kind, ExprKind::Var(_)) && local.is_none() {
                            self.expr(target);
                        } else {
                            self.diags.push(Diagnostic::at(
                                &codes::E0901,
                                *span,
                                "`destroy` of anything but a variable whose struct has a `drop()` is not supported yet",
                            ));
                        }
                    }
                }
            }
            Stmt::Expr(e) => {
                let v = self.expr(e);
                let temps = self.take_temps();
                match v {
                    // A place has no effects; there is nothing to do.
                    Some(Val::Expr(e)) if plain_place(&e) => {}
                    // An unused new owning value is made and dropped.
                    Some(Val::Expr(e)) if self.g.owns(e.ty) => {
                        let unused = self.hidden(e.ty);
                        out.push(hir::Stmt::Assign(hir::Place::local(unused), scoped(e, temps)));
                        out.push(hir::Stmt::Drop(hir::Place::local(unused)));
                    }
                    Some(Val::Expr(e)) => out.push(hir::Stmt::Expr(scoped(e, temps))),
                    _ => {}
                }
            }
        }
    }

    /// Store `e`, a new value for `place`, with the statement's temporaries.
    /// `compound` says it came from `x op= v`.
    fn store(&mut self, place: hir::Place, e: hir::Expr, compound: bool, out: &mut hir::Block) {
        let temps = self.take_temps();
        match e.kind {
            // `s += t` appends in place.
            hir::ExprKind::StrCall(hir::StrFn::Concat, args) if compound => {
                let append = hir::Expr::new(hir::ExprKind::StrCall(hir::StrFn::Append, args), Type::None);
                out.push(hir::Stmt::Expr(scoped(append, temps)));
            }
            // The new value is made before the old one is dropped, since
            // making it may read the old one.
            _ if self.g.owns(e.ty) => {
                let ty = e.ty;
                let new = self.hidden(ty);
                out.push(hir::Stmt::Assign(hir::Place::local(new), scoped(e, temps)));
                out.push(hir::Stmt::Drop(place.clone()));
                out.push(hir::Stmt::Assign(place, hir::Expr::new(hir::ExprKind::Move(new), ty)));
            }
            _ => out.push(hir::Stmt::Assign(place, scoped(e, temps))),
        }
    }

    /// `x = v;` or `x op= v;` on a local.
    fn assign_local(&mut self, name: &str, name_span: Span, op: Option<BinOp>, value: &ast::Expr, span: Span, out: &mut hir::Block) {
        let Some(id) = self.lookup(name) else {
            self.diags.push(Diagnostic::at(&codes::E0201, name_span, format!("unknown binding `{name}`")));
            self.expr(value);
            self.take_temps();
            return;
        };
        let ty = self.locals[id].ty;
        let mut place = hir::Place { local: id, proj: Vec::new(), span: name_span };
        let rhs = match op {
            None => match ty {
                // `r = rhs` re-points a borrow when `rhs` is one, and writes through it otherwise (R18).
                Some(t @ Type::Ref { to, .. }) => {
                    self.g.uses_borrows.set(true);
                    match self.expr_raw(value) {
                        Some(Val::Expr(x)) if x.ty.is_borrow() => {
                            self.check_assignable(id, name_span, false);
                            self.borrow_value(Val::Expr(x), value, t, None)
                        }
                        Some(v) => {
                            place.proj.push(hir::HirProj::Deref);
                            let to = *to;
                            self.into(v, to, origin(value)).map(|e| self.owned(e))
                        }
                        None => None,
                    }
                }
                Some(t) => {
                    self.check_assignable(id, name_span, false);
                    self.coerce(value, t).map(|e| self.owned(e))
                }
                None => {
                    self.check_assignable(id, name_span, false);
                    self.expr(value);
                    None
                }
            },
            Some(op) => {
                // `x += v` is `x = x + v`: it reads `x` first.
                let current = if ty.is_some_and(Type::is_borrow) {
                    // Through the borrow.
                    place.proj.push(hir::HirProj::Deref);
                    self.read_local(id, name_span).and_then(|x| match self.read(Val::Expr(x)) {
                        Val::Expr(x) => Some(x),
                        Val::Const(_) => None,
                    })
                } else {
                    self.check_assignable(id, name_span, true);
                    self.read_local(id, name_span)
                };
                self.compound(op, current, name_span, value, span)
            }
        };
        match rhs {
            Some(e) => self.store(place, e, op.is_some(), out),
            None => {
                self.take_temps();
            }
        }
    }

    /// `p.x = v;` or `p.x op= v;`.
    fn assign_field(&mut self, target: &ast::Expr, op: Option<BinOp>, value: &ast::Expr, span: Span, out: &mut hir::Block) {
        let Some(place) = self.lvalue(target) else {
            self.expr(value);
            self.take_temps();
            return;
        };
        let rhs = match op {
            None => {
                self.check_field_change(&place, target.span, "assigns");
                self.coerce(value, place.ty).map(|e| self.owned(e))
            }
            Some(op) => {
                self.read_local(place.local, target.span);
                self.check_field_change(&place, target.span, "changes");
                let current = self.place_expr(&place);
                self.compound(op, Some(current), target.span, value, span)
            }
        };
        match rhs {
            Some(e) => self.store(place.hir_place(target.span), e, op.is_some(), out),
            None => {
                self.take_temps();
            }
        }
    }

    /// The new value for `target op= value`, given the target's current value.
    fn compound(&mut self, op: BinOp, current: Option<hir::Expr>, target: Span, value: &ast::Expr, span: Span) -> Option<hir::Expr> {
        let rv = self.expr(value);
        let lo = Origin { span: target, compound: false };
        match self.binary_vals(op, Val::Expr(current?), lo, rv?, origin(value), span)? {
            Val::Expr(e) => Some(e),
            Val::Const(_) => None,
        }
    }

    /// Resolve an assignable expression (a name, `self`, or fields of those)
    /// without reading it.
    fn lvalue(&mut self, e: &ast::Expr) -> Option<PlaceInfo> {
        match &e.kind {
            ExprKind::Var(name) => {
                let Some(id) = self.lookup(name) else {
                    self.diags.push(Diagnostic::at(&codes::E0201, e.span, format!("unknown binding `{name}`")));
                    return None;
                };
                let ty = self.locals[id].ty?;
                Some(PlaceInfo { local: id, proj: Vec::new(), ty, through_deref: false, mut_field: false })
            }
            ExprKind::SelfValue => {
                let id = self.self_local(e.span)?;
                let ty = self.locals[id].ty?;
                Some(PlaceInfo { local: id, proj: Vec::new(), ty, through_deref: false, mut_field: false })
            }
            ExprKind::Field { recv, name, name_span } => {
                let mut p = self.lvalue(recv)?;
                Self::auto_deref(&mut p);
                let (i, ty) = self.field_of(p.ty, name, *name_span, recv.span)?;
                p.mut_field |= self.g.structs[p.ty.struct_id().unwrap()].fields[i].mutable;
                p.proj.push(hir::HirProj::Field(i));
                p.ty = ty;
                Some(p)
            }
            ExprKind::Deref(inner) => {
                self.g.uses_borrows.set(true);
                let mut p = self.lvalue(inner)?;
                match p.ty {
                    Type::Ptr { to } => {
                        p.proj.push(hir::HirProj::Deref);
                        p.ty = *to;
                        p.through_deref = true;
                        Some(p)
                    }
                    t => {
                        self.deref_error(t, inner.span, e.span);
                        None
                    }
                }
            }
            _ => unreachable!("the parser only lets places be assigned"),
        }
    }

    /// A borrow in the middle of a path is read through (R4).
    fn auto_deref(p: &mut PlaceInfo) {
        if let Type::Ref { to, .. } = p.ty {
            p.proj.push(hir::HirProj::Deref);
            p.ty = *to;
            p.through_deref = true;
        }
    }

    /// `*e` on something that is not a pointer.
    fn deref_error(&mut self, ty: Type, inner: Span, at: Span) {
        let d = if ty.is_borrow() {
            Diagnostic::at(&codes::E0901, at, "`*` on a borrow is not supported yet")
                .help("a borrow already means its target: use it without `*`")
        } else {
            Diagnostic::at(&codes::E0101, at, format!("`*` needs a pointer, found `{ty}`")).label(inner, format!("this is `{ty}`"))
        };
        self.diags.push(d);
    }

    /// The current value of a place (its local must certainly be assigned).
    fn place_expr(&self, p: &PlaceInfo) -> hir::Expr {
        let mut ty = self.locals[p.local].ty.unwrap();
        let mut e = hir::Expr::new(hir::ExprKind::Local(p.local), ty);
        for step in &p.proj {
            match *step {
                hir::HirProj::Field(i) => {
                    ty = self.g.structs[ty.struct_id().unwrap()].fields[i].ty.unwrap();
                    e = hir::Expr::new(hir::ExprKind::Field(Box::new(e), i), ty);
                }
                hir::HirProj::Deref => {
                    ty = ty.target().unwrap();
                    e = hir::Expr::new(hir::ExprKind::Deref(Box::new(e)), ty);
                }
            }
        }
        e
    }

    /// A field of a place is about to change (`does` is "assigns" or
    /// "changes", for messages). Fine behind a `mut` field or binding; on
    /// `self` it makes the method a changing one; otherwise an error.
    fn check_field_change(&mut self, p: &PlaceInfo, at: Span, does: &str) {
        let l = &self.locals[p.local];
        // WP5 checks writes through borrows.
        if p.mut_field || l.mutable || p.through_deref {
            return;
        }
        if l.is_self {
            let what = format!("{does} `{}`", self.text(at));
            let m = self.method.unwrap().func;
            self.g.self_use[m].changes.get_or_insert((at, what));
            return;
        }
        // The last field: the one that could be made `mut` instead.
        let mut ty = l.ty.unwrap();
        let fields: Vec<usize> = p
            .proj
            .iter()
            .filter_map(|s| match s {
                hir::HirProj::Field(i) => Some(*i),
                hir::HirProj::Deref => None,
            })
            .collect();
        for &i in &fields[..fields.len() - 1] {
            ty = self.g.structs[ty.struct_id().unwrap()].fields[i].ty.unwrap();
        }
        let s = &self.g.structs[ty.struct_id().unwrap()];
        let f = &s.fields[*fields.last().unwrap()];
        let (name, lty) = (&l.name, l.ty.unwrap());
        let whose = if l.is_param { "parameter " } else { "" };
        let d = Diagnostic::at(&codes::E0205, at, format!("cannot change `{}`: {whose}`{name}` is not `mut`, and neither is the field", self.text(at)))
            .note("changed here")
            .label(l.span, "declared here without `mut`")
            .help(format!(
                "declare it as `mut {lty} {name}`, or the field as `mut {} {};` in `{}`",
                f.ty.unwrap(),
                f.name,
                s.name
            ));
        self.diags.push(d);
    }

    /// Report if local `id` may not be assigned (or changed, for `++` and `+=`) here.
    fn check_assignable(&mut self, id: LocalId, at: Span, changes: bool) {
        let l = &self.locals[id];
        if l.mutable {
            return;
        }
        let name = &l.name;
        let ty = l.ty.map_or(String::new(), |t| format!("{t} "));
        let d = if l.is_param {
            Diagnostic::at(&codes::E0205, at, format!("cannot change parameter `{name}`: it is not `mut`"))
                .note("changed here")
                .label(l.span, "declared here without `mut`")
                .help(format!("declare it as `mut {ty}{name}` to allow changes"))
        } else if changes {
            Diagnostic::at(&codes::E0205, at, format!("cannot change `{name}`: it is not `mut`"))
                .note("changed here")
                .label(l.span, "declared here without `mut`")
                .help(format!("declare it as `mut {ty}{name}` to allow changes"))
        } else {
            return;
        };
        self.diags.push(d);
    }

    /// A read of local `id`. Whether it has a value is checked on the MIR.
    fn read_local(&mut self, id: LocalId, _at: Span) -> Option<hir::Expr> {
        let ty = self.locals[id].ty?;
        Some(hir::Expr::new(hir::ExprKind::Local(id), ty))
    }

    /// The `self` local, or an error if there is none here.
    fn self_local(&mut self, at: Span) -> Option<LocalId> {
        match self.method {
            None => {
                self.diags.push(Diagnostic::at(&codes::E0209, at, "`self` only exists inside a struct's methods").note("not in a method"));
                None
            }
            Some(MethodCtx { static_span: Some(st), .. }) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0209, at, "a `static` method has no `self`")
                        .note("used here")
                        .label(st, "the method is `static`")
                        .help("remove `static` to give the method a `self`"),
                );
                None
            }
            Some(m) => {
                self.g.self_use[m.func].used = true;
                Some(0)
            }
        }
    }

    /// Field `name` of a value of type `ty`: its index and type. Reports an
    /// unknown field, or a private one used outside the struct's methods.
    fn field_of(&mut self, ty: Type, name: &str, name_span: Span, recv_span: Span) -> Option<(usize, Type)> {
        let Some(s) = self.g.struct_of(ty) else {
            self.diags.push(
                Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no fields")).note("not a struct").label(recv_span, format!("this is `{ty}`")),
            );
            return None;
        };
        let Some(i) = s.fields.iter().position(|f| f.name == name) else {
            let fields: Vec<&str> = s.fields.iter().map(|f| f.name.as_str()).collect();
            let mut d = Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no field `{name}`")).note("unknown field").label(recv_span, format!("this is `{ty}`"));
            d = if s.methods.iter().any(|m| m.name == name) {
                d.help(format!("`{name}` is a method: call it with `{}::{name}()`", self.text(recv_span)))
            } else if let Some(near) = similar(name, &fields) {
                d.help(format!("did you mean `{near}`?"))
            } else if fields.is_empty() {
                d.help(format!("`{ty}` has no fields"))
            } else {
                d.help(format!("its fields are {}", list(&fields)))
            };
            self.diags.push(d);
            return None;
        };
        let f = &s.fields[i];
        if !f.public && !self.inside(ty) {
            let d = Diagnostic::at(&codes::E0212, name_span, format!("field `{name}` of `{ty}` is private"))
                .note("used outside the struct's methods")
                .label(f.span, "declared in a private section")
                .help(format!("only `{ty}`'s own methods can use it; put it under `public:` to use it here"));
            self.diags.push(d);
            return None;
        }
        Some((i, f.ty?))
    }

    /// Are we inside a method of struct type `ty` (so its private members are usable)?
    fn inside(&self, ty: Type) -> bool {
        self.method.is_some_and(|m| Some(m.struct_id) == ty.struct_id())
    }

    fn check_method_visible(&mut self, s: StructId, m: &MethodInfo, at: Span) -> bool {
        if m.public || self.inside(self.g.structs[s].ty) {
            return true;
        }
        let info = &self.g.structs[s];
        let what = if m.operator.is_some() { format!("operator `{}`", m.name) } else { format!("method `{}`", m.name) };
        self.diags.push(
            Diagnostic::at(&codes::E0212, at, format!("{what} of `{}` is private", info.name))
                .note("used outside the struct's methods")
                .label(self.g.sigs[m.func].span, "declared in a private section")
                .help(format!("only `{}`'s own methods can use it; put it under `public:` to use it here", info.name)),
        );
        false
    }

    /// Which kind of value a method is called on.
    fn receiver(&self, e: &hir::Expr) -> Receiver {
        let mut mut_field = false;
        let mut cur = e;
        let id = loop {
            match &cur.kind {
                hir::ExprKind::Field(inner, i) => {
                    mut_field |= self.g.structs[inner.ty.struct_id().unwrap()].fields[*i].mutable;
                    cur = inner;
                }
                hir::ExprKind::Local(id) => break *id,
                // WP5 checks changes through borrows.
                hir::ExprKind::Deref(_) => return Receiver::Changeable,
                _ => return Receiver::Temporary,
            }
        };
        let l = &self.locals[id];
        if mut_field || l.mutable {
            Receiver::Changeable
        } else if l.is_self {
            Receiver::OfSelf
        } else {
            Receiver::Fixed(id)
        }
    }

    /// Remember a call of `method` on `recv`, so it can be reported once it
    /// is known whether the method changes `self`.
    fn note_receiver(&mut self, recv: &hir::Expr, recv_span: Span, method: FuncId, span: Span) {
        let binding = match self.receiver(recv) {
            Receiver::Changeable => return,
            Receiver::OfSelf => {
                let m = self.method.unwrap().func;
                self.g.self_use[m].calls.push((method, span));
                return;
            }
            Receiver::Fixed(id) => {
                let l = &self.locals[id];
                Some(FixedBinding { name: l.name.clone(), ty: l.ty, is_param: l.is_param, span: l.span })
            }
            Receiver::Temporary => None,
        };
        let text = self.text(recv_span).to_string();
        self.g.fixed_receivers.push(FixedReceiver { method, span, text, binding });
    }

    /// Check `e` and convert it to `to`, reporting a mismatch or a lossy conversion.
    fn coerce(&mut self, e: &ast::Expr, to: Type) -> Option<hir::Expr> {
        if to.is_ptr() && matches!(e.kind, ExprKind::NoneValue) {
            self.g.uses_borrows.set(true);
            return Some(hir::Expr::new(hir::ExprKind::NullPtr, to).at(e.span));
        }
        if to.is_borrow() {
            return self.coerce_borrow(e, to, None);
        }
        let v = self.expr(e)?;
        self.into(v, to, origin(e))
    }

    /// `e` where a borrow of type `to` is wanted: the value must be a borrow
    /// of that kind. `call` is the function being called, for E0321.
    fn coerce_borrow(&mut self, e: &ast::Expr, to: Type, call: Option<FuncId>) -> Option<hir::Expr> {
        self.g.uses_borrows.set(true);
        let v = self.expr_raw(e)?;
        self.borrow_value(v, e, to, call)
    }

    /// `coerce_borrow` for a value already checked.
    fn borrow_value(&mut self, v: Val, e: &ast::Expr, to: Type, call: Option<FuncId>) -> Option<hir::Expr> {
        let to_target = to.target();
        match v {
            Val::Expr(x) if x.ty == to => Some(x),
            Val::Expr(x) if matches!((x.ty, to), (Type::Ref { mutable: true, to: a }, Type::Ref { mutable: false, to: b }) if a == b) => {
                if matches!(e.kind, ExprKind::Borrow { .. } | ExprKind::Lend(_)) || !ast_place(e) {
                    self.diags.push(
                        Diagnostic::at(&codes::E0901, e.span, "using a `&mut` where a `&` is expected is not supported yet")
                            .note(format!("this is `{}`", x.ty)),
                    );
                } else {
                    let name = self.text(e.span).to_string();
                    self.diags.push(
                        Diagnostic::at(&codes::E0330, e.span, format!("`{name}` is a `{}`, but a `{to}` is wanted here", x.ty))
                            .note("passes the `&mut` itself")
                            .help(format!("write `&{name}`: a shared borrow through the `&mut` you hold")),
                    );
                }
                None
            }
            Val::Expr(x) if x.ty.is_borrow() => {
                let o = origin(e);
                self.into(Val::Expr(x), to, o)
            }
            other => {
                let plain = match &other {
                    Val::Const(_) => true,
                    Val::Expr(x) => Some(x.ty) == to_target,
                };
                match call {
                    Some(f) if plain => {
                        let full = self.g.sigs[f].name.clone();
                        let short_name = full.rsplit("::").next().unwrap_or(&full).to_string();
                        let amp = if matches!(to, Type::Ref { mutable: true, .. }) { "&mut " } else { "&" };
                        let arg = self.text(e.span).to_string();
                        let help = if full.contains("::") {
                            format!("write `{amp}{arg}` as the argument")
                        } else {
                            format!("write `{short_name}({amp}{arg})`")
                        };
                        self.diags.push(
                            Diagnostic::at(&codes::E0321, e.span, format!("`{short_name}` takes `{to}` here, so the call must say so"))
                                .note(format!("this passes `{arg}` itself"))
                                .help(help),
                        );
                        None
                    }
                    _ => self.into(other, to, origin(e)),
                }
            }
        }
    }

    /// A declaration's value, for a binding of type `ty`.
    fn init(&mut self, v: &ast::Init, ty: Option<Type>) -> Option<hir::Expr> {
        match v {
            ast::Init::Expr(e) => {
                let Some(t) = ty else {
                    self.expr(e);
                    return None;
                };
                let Some(s) = t.struct_id() else { return self.coerce(e, t) };
                match self.expr_raw(e)? {
                    Val::Expr(x) if x.ty == t => Some(x),
                    Val::Expr(x) if x.ty.is_borrow() && x.ty.target() == Some(t) => match self.read(Val::Expr(x)) {
                        Val::Expr(x) => Some(x),
                        Val::Const(_) => None,
                    },
                    // One value of another type builds a one-field struct.
                    Val::Expr(x) if self.g.structs[s].fields.len() != 1 && (x.ty.struct_id().is_some() || x.ty == Type::None) => {
                        self.into(Val::Expr(x), t, origin(e))
                    }
                    other => self.construct(t, vec![(None, Some(other), origin(e))], e.span),
                }
            }
            ast::Init::Fields(values, span) => {
                let checked: Vec<_> = values.iter().map(|fv| (fv.name.clone(), self.expr_raw(&fv.value), origin(&fv.value))).collect();
                match ty {
                    Some(t) if t.struct_id().is_some() => self.construct(t, checked, *span),
                    Some(t) => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0114, *span, format!("a list of values can only build a struct, not a `{t}`"))
                                .note(format!("{} values for one `{t}`", values.len())),
                        );
                        None
                    }
                    None => None,
                }
            }
        }
    }

    /// The construction shorthand: struct `t` from values given in field
    /// order, or all by name. Every field must be given exactly once.
    fn construct(&mut self, t: Type, values: Vec<GivenValue>, span: Span) -> Option<hir::Expr> {
        let s = t.struct_id().unwrap();
        let fields: Vec<(String, Option<Type>)> = self.g.structs[s].fields.iter().map(|f| (f.name.clone(), f.ty)).collect();
        let names: Vec<&str> = fields.iter().map(|f| f.0.as_str()).collect();
        let in_order = || {
            let decls: Vec<String> = fields.iter().map(|(n, ty)| format!("{} {n}", ty.map_or("?".into(), |t| t.to_string()))).collect();
            list(&decls.iter().map(String::as_str).collect::<Vec<_>>())
        };
        let named = values.iter().filter(|v| v.0.is_some()).count();
        if named != 0 && named != values.len() {
            let odd = values.iter().find(|v| v.0.is_none() == (named * 2 > values.len())).unwrap();
            self.diags.push(
                Diagnostic::at(&codes::E0114, odd.2.span, format!("values for `{t}` are partly named and partly not"))
                    .note(if odd.0.is_some() { "named" } else { "not named" })
                    .help("name every value (`x: 1, y: 2`) or none (`1, 2`)"),
            );
            return None;
        }
        let mut order: Vec<Option<usize>> = Vec::new();
        if named == 0 {
            if values.len() != fields.len() {
                let n = fields.len();
                self.diags.push(
                    Diagnostic::at(
                        &codes::E0114,
                        span,
                        format!("`{t}` has {n} field{}, but {} value{} given", if n == 1 { "" } else { "s" }, values.len(), if values.len() == 1 { " is" } else { "s are" }),
                    )
                    .label(self.g.structs[s].name_span, format!("`{t}` is defined here"))
                    .help(if n == 0 { format!("`{t}` has no fields to give values for") } else { format!("give a value for each field, in order: {}", in_order()) }),
                );
                return None;
            }
            order.extend((0..fields.len()).map(Some));
        } else {
            let mut given: Vec<Option<Span>> = vec![None; fields.len()];
            for (name, _, _) in &values {
                let (name, at) = name.as_ref().unwrap();
                let Some(i) = names.iter().position(|n| n == name) else {
                    let mut d = Diagnostic::at(&codes::E0201, *at, format!("`{t}` has no field `{name}`"));
                    d = match similar(name, &names) {
                        Some(near) => d.help(format!("did you mean `{near}`?")),
                        None if names.is_empty() => d.help(format!("`{t}` has no fields")),
                        None => d.help(format!("its fields are {}", list(&names))),
                    };
                    self.diags.push(d);
                    order.push(None);
                    continue;
                };
                if let Some(first) = given[i] {
                    self.diags.push(
                        Diagnostic::at(&codes::E0114, *at, format!("field `{name}` is given twice"))
                            .note("given again here")
                            .label(first, "first given here"),
                    );
                    order.push(None);
                    continue;
                }
                given[i] = Some(*at);
                order.push(Some(i));
            }
            let missing: Vec<&str> = names.iter().zip(&given).filter(|(_, g)| g.is_none()).map(|(n, _)| *n).collect();
            if !missing.is_empty() {
                let what = if missing.len() == 1 { "field" } else { "fields" };
                self.diags.push(
                    Diagnostic::at(&codes::E0114, span, format!("{what} {} of `{t}` {} not given", list(&missing), if missing.len() == 1 { "is" } else { "are" }))
                        .note("missing here")
                        .help(format!("every field needs a value: {}", in_order())),
                );
                return None;
            }
        }
        let mut out = Vec::new();
        let mut ok = true;
        for ((_, v, o), i) in values.into_iter().zip(order) {
            let converted = match (v, i.and_then(|i| fields[i].1)) {
                (Some(v), Some(fty)) => self.into(v, fty, o).map(|e| self.owned(e)),
                _ => None,
            };
            match (converted, i) {
                (Some(e), Some(i)) => out.push((i, e)),
                _ => ok = false,
            }
        }
        ok.then_some(hir::Expr::new(hir::ExprKind::Struct(out), t))
    }

    fn materialise(&mut self, v: Val, span: Span) -> Option<hir::Expr> {
        match v {
            Val::Expr(e) => Some(e),
            Val::Const(c) => match c.default_type() {
                Some(ty) => Some(self.fit_const(&c, ty, span, true)),
                None => {
                    self.diags.push(
                        Diagnostic::at(&codes::E0002, span, "this constant is too large for any number type").note("does not fit in 128 bits"),
                    );
                    None
                }
            },
        }
    }

    fn into(&mut self, v: Val, to: Type, from: Origin) -> Option<hir::Expr> {
        match v {
            Val::Const(c) if to.is_numeric() => Some(self.fit_const(&c, to, from.span, false)),
            Val::Const(_) => {
                let mut d = Diagnostic::at(&codes::E0101, from.span, format!("expected `{to}`, found a number"))
                    .note("this is a number");
                if to == Type::Str {
                    d = d.help(self.text_fix(from));
                }
                self.diags.push(d);
                None
            }
            Val::Expr(e) if e.ty == to => Some(e),
            Val::Expr(e) if e.ty.is_borrow() && !to.is_borrow() => {
                let v = self.read(Val::Expr(e));
                self.into(v, to, from)
            }
            Val::Expr(e) if e.ty == Type::None => {
                self.diags.push(
                    Diagnostic::at(&codes::E0104, from.span, format!("expected `{to}`, but this produces no value"))
                        .note("returns `none`"),
                );
                None
            }
            Val::Expr(e) if e.ty.is_numeric() && to.is_numeric() => {
                if !e.ty.converts_losslessly_to(to) {
                    self.warn_lossy(e.ty, to, from);
                }
                Some(hir::Expr::new(hir::ExprKind::Convert(Box::new(e)), to))
            }
            Val::Expr(e) => {
                let mut d = Diagnostic::at(&codes::E0101, from.span, format!("expected `{to}`, found `{}`", e.ty))
                    .note(format!("this is `{}`", e.ty));
                if e.ty == Type::Bool && to.is_numeric() || e.ty.is_numeric() && to == Type::Bool {
                    d = d.help("`bool` is not a number and never converts to or from one");
                } else if to == Type::Str {
                    d = d.help(self.text_fix(from));
                }
                self.diags.push(d);
                None
            }
        }
    }

    /// "turn it into text with `::text()`: `x::text()`", for a number or bool
    /// found where a string belongs.
    fn text_fix(&self, at: Origin) -> String {
        let text = self.text(at.span);
        if text.len() > 40 || text.contains('\n') {
            "turn it into text with `::text()`".to_string()
        } else if at.compound {
            format!("turn it into text: `({text})::text()`")
        } else {
            format!("turn it into text: `{text}::text()`")
        }
    }


    fn warn_lossy(&mut self, from: Type, to: Type, at: Origin) {
        let note = match (from, to) {
            (Type::Int { .. }, Type::Int { .. }) => format!(
                "`{from}` holds {}; `{to}` holds {}; values outside that wrap around",
                from.describe_range(),
                to.describe_range()
            ),
            (Type::Float(_), Type::Int { .. }) => format!(
                "`{to}` holds only whole numbers from {}; the fraction is dropped and values outside that wrap around",
                to.describe_range()
            ),
            (Type::Int { .. }, Type::Float(_)) => {
                format!("`{to}` holds {}; `{from}` goes up to {}; larger values are rounded", to.describe_range(), from.int_range().unwrap().1)
            }
            _ => format!("`{to}` keeps about 7 significant digits, `{from}` about 16; the value is rounded"),
        };
        let text = self.text(at.span);
        let fix = if text.len() > 40 || text.contains('\n') {
            format!("add `::to<{to}>()` to say the conversion is intended")
        } else if at.compound {
            format!("write `({text})::to<{to}>()` to say the conversion is intended")
        } else {
            format!("write `{text}::to<{to}>()` to say the conversion is intended")
        };
        self.diags.push(
            Diagnostic::at(&codes::W0102, at.span, format!("converting `{from}` to `{to}` can lose information"))
                .note(format!("this is `{from}`"))
                .note_line(note)
                .help(fix),
        );
    }

    /// Give constant `c` the type `to`. Unless `explicit`, a value that does
    /// not fit is reported (W0102); either way it wraps.
    fn fit_const(&mut self, c: &Const, to: Type, span: Span, explicit: bool) -> hir::Expr {
        let e = match to {
            Type::Int { .. } => int_lit(c.wrap_to_int(to), to),
            _ => hir::Expr::new(hir::ExprKind::Float(c.to_float(to)), to),
        };
        if !explicit && !c.fits(to) {
            let what = self.quote(span, "this constant");
            let (becomes, note) = match (&e.kind, c) {
                (hir::ExprKind::Int(bits), Const::Dec(r)) if !r.is_integer() => (
                    match Const::from_int_bits(*bits, to) {
                        Const::Int(v) => v.to_string(),
                        Const::Dec(_) => unreachable!(),
                    },
                    format!("`{to}` holds only whole numbers, from {}; the fraction is dropped", to.describe_range()),
                ),
                (hir::ExprKind::Int(bits), _) => (
                    match Const::from_int_bits(*bits, to) {
                        Const::Int(v) => v.to_string(),
                        Const::Dec(_) => unreachable!(),
                    },
                    format!("`{to}` holds {}; a constant outside that wraps around", to.describe_range()),
                ),
                (hir::ExprKind::Float(f), _) if f.is_infinite() => ("infinity".to_string(), format!("it is beyond the largest `{to}`")),
                (hir::ExprKind::Float(f), _) => (format!("{f:?}"), format!("`{to}` holds {}; this one is rounded", to.describe_range())),
                _ => unreachable!(),
            };
            self.diags.push(
                Diagnostic::at(&codes::W0102, span, format!("{what} does not fit `{to}`"))
                    .note(format!("becomes {becomes}"))
                    .note_line(note)
                    .help(format!("use a value that fits or a bigger type; `::to<{to}>()` converts without this warning")),
            );
        }
        e
    }

    /// The value of `e`, or `None` if an error was already reported inside it.
    /// Every expression made from `e` carries its span.
    fn expr(&mut self, e: &ast::Expr) -> Option<Val> {
        let v = self.expr_raw(e)?;
        Some(self.read(v))
    }

    /// A borrow means its target (R4): reads through it. Contexts that want
    /// the borrow itself use `expr_raw`.
    fn read(&self, v: Val) -> Val {
        match v {
            Val::Expr(x) if x.ty.is_borrow() => {
                let (ty, span) = (x.ty.target().unwrap(), x.span);
                Val::Expr(hir::Expr::new(hir::ExprKind::Deref(Box::new(x)), ty).at(span))
            }
            v => v,
        }
    }

    fn expr_raw(&mut self, e: &ast::Expr) -> Option<Val> {
        match self.expr_inner(e)? {
            Val::Expr(mut x) => {
                if x.span == Span::new(0, 0) {
                    x.span = e.span;
                }
                Some(Val::Expr(x))
            }
            c => Some(c),
        }
    }

    fn expr_inner(&mut self, e: &ast::Expr) -> Option<Val> {
        let typed = |kind, ty| Some(Val::Expr(hir::Expr::new(kind, ty)));
        match &e.kind {
            ExprKind::Num(n) => match n.suffix {
                Some(ty) => Some(Val::Expr(self.fit_const(&n.value, ty, e.span, false))),
                None => Some(Val::Const(n.value.clone())),
            },
            ExprKind::Bool(b) => typed(hir::ExprKind::Bool(*b), Type::Bool),
            ExprKind::Str(parts) => self.string_literal(parts),
            ExprKind::Var(name) => match self.lookup(name) {
                Some(id) => self.read_local(id, e.span).map(Val::Expr),
                None => {
                    let mut d = Diagnostic::at(&codes::E0201, e.span, format!("unknown name `{name}`"));
                    if name == "io" {
                        d = d.help("`io` is a module; call its functions like `io::outln(...)`");
                    } else if self.g.structs_by_name.contains_key(name) {
                        d = d.help(format!("`{name}` is a struct; call its static methods like `{name}::name()`"));
                    }
                    self.diags.push(d);
                    None
                }
            },
            ExprKind::SelfValue => {
                let id = self.self_local(e.span)?;
                self.read_local(id, e.span).map(Val::Expr)
            }
            ExprKind::Field { recv, name, name_span } => {
                let v = match self.expr(recv)? {
                    Val::Const(_) => {
                        self.diags.push(Diagnostic::at(&codes::E0201, *name_span, "numbers have no fields").label(recv.span, "this is a number"));
                        return None;
                    }
                    Val::Expr(v) => v,
                };
                let (i, ty) = self.field_of(v.ty, name, *name_span, recv.span)?;
                // A field of a struct that owns strings is read in place.
                let v = self.receiver_value(v);
                typed(hir::ExprKind::Field(Box::new(v), i), ty)
            }
            ExprKind::Unary(op, inner) => self.unary(*op, inner, e.span),
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), l, r) => {
                let lhs = self.coerce(l, Type::Bool);
                let rhs = self.coerce(r, Type::Bool);
                typed(hir::ExprKind::Logic { and: *op == BinOp::And, lhs: Box::new(lhs?), rhs: Box::new(rhs?) }, Type::Bool)
            }
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne), l, r)
                if matches!(l.kind, ExprKind::NoneValue) || matches!(r.kind, ExprKind::NoneValue) =>
            {
                self.none_compare(*op, l, r, e.span)
            }
            ExprKind::Binary(op, l, r) => {
                let lv = self.expr(l);
                let rv = self.expr(r);
                self.binary_vals(*op, lv?, origin(l), rv?, origin(r), e.span)
            }
            ExprKind::IncDec { inc, prefix, target } => {
                let sym = if *inc { "++" } else { "--" };
                if !matches!(target.kind, ExprKind::Var(_) | ExprKind::SelfValue | ExprKind::Field { .. }) {
                    self.diags.push(
                        Diagnostic::at(&codes::E0005, target.span, format!("`{sym}` only works on a binding or a field"))
                            .note("this is not a binding"),
                    );
                    return None;
                }
                let mut p = self.lvalue(target)?;
                Self::auto_deref(&mut p);
                if !p.ty.is_numeric() {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, target.span, format!("`{sym}` needs a number, found `{}`", p.ty))
                            .note(format!("this is `{}`", p.ty)),
                    );
                    return None;
                }
                if p.through_deref {
                    // WP5 checks writes through borrows.
                } else if p.proj.is_empty() {
                    self.check_assignable(p.local, target.span, true);
                } else {
                    self.check_field_change(&p, target.span, "changes");
                }
                self.read_local(p.local, target.span)?;
                let ty = p.ty;
                typed(hir::ExprKind::IncDec { place: p.hir_place(e.span), inc: *inc, prefix: *prefix }, ty)
            }
            ExprKind::Call { name, args } => {
                let Some(&id) = self.g.funcs_by_name.get(name) else {
                    self.diags.push(Diagnostic::at(&codes::E0201, e.span, format!("unknown function `{name}`")));
                    for a in args {
                        self.expr(a);
                    }
                    return None;
                };
                let args = self.args(id, args, e.span);
                let ty = self.g.sigs[id].ret?;
                typed(hir::ExprKind::Call(id, args?), ty)
            }
            ExprKind::Method { recv, name, name_span, targs, args, .. } => {
                if let ExprKind::Var(n) = &recv.kind
                    && self.lookup(n).is_none()
                {
                    if n == "io" {
                        return self.io_call(name, *name_span, recv.span, targs, args, e.span);
                    }
                    if let Some(&s) = self.g.structs_by_name.get(n) {
                        return self.static_call(s, name, *name_span, targs, args, e.span);
                    }
                }
                self.method(recv, name, *name_span, targs, args, e.span)
            }
            ExprKind::Borrow { mutable, operand } => self.borrow_expr(*mutable, operand),
            ExprKind::Deref(inner) => {
                self.g.uses_borrows.set(true);
                let v = self.expr_raw(inner)?;
                let x = self.materialise(v, inner.span)?;
                match x.ty {
                    Type::Ptr { to } => typed(hir::ExprKind::Deref(Box::new(x)), *to),
                    t => {
                        self.deref_error(t, inner.span, e.span);
                        None
                    }
                }
            }
            ExprKind::Copy(inner) => self.copy_expr(e, inner),
            ExprKind::Take { place, leaving } => self.take_expr(e, place, leaving.as_deref()),
            ExprKind::Lend(inner) => self.lend_expr(e, inner),
            ExprKind::AddressOf(inner) => self.address_expr(e, inner),
            ExprKind::NoneValue => {
                self.diags.push(Diagnostic::at(&codes::E0901, e.span, "`none` as a value is not supported yet"));
                None
            }
            ExprKind::Range(..) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0010, e.span, "a range can only be used after `of` in a `for`, or as a `case` pattern")
                        .note("this is a range")
                        .help("to go through the numbers from a to b, write `for (uint32 i of a..b) { }`"),
                );
                None
            }
        }
    }

    /// `&e` / `&mut e`.
    fn borrow_expr(&mut self, mutable: bool, operand: &ast::Expr) -> Option<Val> {
        self.g.uses_borrows.set(true);
        let v = self.expr_raw(operand)?;
        let x = self.materialise(v, operand.span)?;
        let name = self.text(operand.span).to_string();
        match x.ty {
            Type::None => {
                self.diags.push(
                    Diagnostic::at(&codes::E0104, operand.span, "cannot borrow a value of type `none`").note("`none` holds no value"),
                );
                None
            }
            Type::Ref { mutable: held, to } => {
                if mutable && held {
                    self.diags.push(
                        Diagnostic::at(&codes::E0329, operand.span, format!("cannot borrow `{name}` with `&mut`: it is already a `&mut`"))
                            .note(format!("`{name}` is `{}`", x.ty))
                            .help(format!("write `lend {name}`")),
                    );
                    None
                } else if !mutable && held {
                    // The one allowed borrow of a borrow: a shared view through the `&mut` you hold.
                    let place = hir::Expr::new(hir::ExprKind::Deref(Box::new(x)), *to);
                    Some(Val::Expr(hir::Expr::new(hir::ExprKind::Borrow { mutable: false, place: Box::new(place) }, Type::borrow(false, *to))))
                } else {
                    self.diags.push(
                        Diagnostic::at(&codes::E0901, operand.span, "borrowing a borrow is not supported yet")
                            .help(format!("`{name}` is already a `&` borrow; use `{name}` itself")),
                    );
                    None
                }
            }
            t => Some(Val::Expr(hir::Expr::new(hir::ExprKind::Borrow { mutable, place: Box::new(x) }, Type::borrow(mutable, t)))),
        }
    }

    /// `copy e`.
    fn copy_expr(&mut self, e: &ast::Expr, inner: &ast::Expr) -> Option<Val> {
        if !ast_place(inner) {
            self.g.uses_borrows.set(true);
            let text = self.quote(inner.span, "this value");
            self.diags.push(
                Diagnostic::at(&codes::E0327, e.span, "`copy` needs a variable or a field, like `copy name`")
                    .note(format!("{text} is a temporary value; there is nothing to copy it from"))
                    .help("name the value first, then copy that"),
            );
            self.expr(inner);
            return None;
        }
        let v = self.expr_raw(inner)?;
        let x = self.materialise(v, inner.span)?;
        let x_ty = x.ty;
        let (value, ty) = match x.ty {
            Type::Ref { to, .. } => (hir::Expr::new(hir::ExprKind::Deref(Box::new(x)), *to), *to),
            t => (x, t),
        };
        // The workaround copy of step 2 is a deep copy too, and the backends
        // still build it: a `copy` of a place that owns what it copies (no
        // borrow in the way) is that node inside an empty `Scoped`, which
        // tells the MIR builder it was written (`mir::build`, WP6 removes
        // this with the workaround). Anything else is a step 3 node.
        let plain = !matches!(x_ty, Type::Ref { .. }) && !has_deref(&value);
        if plain && !self.copies_anyway(ty) {
            return Some(Val::Expr(self.written_copy(value)));
        }
        self.g.uses_borrows.set(true);
        if self.copies_anyway(ty) {
            let at = Span::new(e.span.start, e.span.start + 4);
            self.diags.push(
                Diagnostic::at(&codes::W0322, at, format!("`copy` does nothing for `{ty}`"))
                    .note("a copy happens anyway")
                    .help(format!("remove `copy`: `{ty}` always copies")),
            );
        }
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::DeepCopy(Box::new(value)), ty)))
    }

    /// Does a value of type `t` copy when used (R11)?
    fn copies_anyway(&self, t: Type) -> bool {
        match t {
            Type::Int { .. } | Type::Float(_) | Type::Bool | Type::Ref { mutable: false, .. } => true,
            Type::Struct { id, .. } => self.g.struct_copy.get(id).copied().unwrap_or(false),
            _ => false,
        }
    }

    /// `take p.f` / `take p.f leaving v`.
    fn take_expr(&mut self, e: &ast::Expr, place: &ast::Expr, leaving: Option<&ast::Expr>) -> Option<Val> {
        self.g.uses_borrows.set(true);
        let unsupported = |cx: &mut Self| {
            cx.diags.push(Diagnostic::at(&codes::E0901, place.span, "this form of `take` is not supported yet"));
        };
        if !ast_place(place) {
            let text = self.quote(place.span, "this value");
            self.diags.push(
                Diagnostic::at(&codes::E0327, e.span, "`take` needs a field of a variable, like `p.name`")
                    .note(format!("{text} is a temporary value; there is nothing to take it out of"))
                    .help("use the value directly, without `take`"),
            );
            self.expr(place);
            return None;
        }
        if !matches!(place.kind, ExprKind::Field { .. }) {
            unsupported(self);
            return None;
        }
        let p = self.lvalue(place)?;
        if p.through_deref || self.copies_anyway(p.ty) {
            unsupported(self);
            return None;
        }
        self.read_local(p.local, place.span)?;
        let leaving = match leaving {
            Some(v) => Some(Box::new(self.coerce(v, p.ty).map(|e| self.owned(e))?)),
            None => None,
        };
        let ty = p.ty;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Take { place: p.hir_place(place.span), leaving }, ty)))
    }

    /// `lend r`.
    fn lend_expr(&mut self, e: &ast::Expr, inner: &ast::Expr) -> Option<Val> {
        self.g.uses_borrows.set(true);
        let v = self.expr_raw(inner)?;
        let name = self.text(inner.span).to_string();
        match v {
            Val::Expr(x) if ast_place(inner) && matches!(x.ty, Type::Ref { mutable: true, .. }) => {
                let ty = x.ty;
                Some(Val::Expr(hir::Expr::new(hir::ExprKind::Lend(Box::new(x)), ty)))
            }
            Val::Expr(x) if ast_place(inner) && x.ty.is_borrow() => {
                self.diags.push(Diagnostic::at(&codes::E0901, inner.span, "`lend` on a `&` borrow is not supported yet"));
                None
            }
            other => {
                let found = match &other {
                    Val::Const(_) => "a number".to_string(),
                    Val::Expr(x) => format!("`{}`", x.ty),
                };
                let mut d = Diagnostic::at(&codes::E0326, e.span, format!("`lend` passes on a `&mut` borrow you hold, and `{name}` is not one"))
                    .note("nothing to pass on");
                if let ExprKind::Var(n) = &inner.kind
                    && let Some(id) = self.lookup(n)
                {
                    let l = &self.locals[id];
                    d = d.label(l.span, format!("`{name}` is {found}, not a `&mut`"));
                }
                self.diags.push(d.help(format!("to lend `{name}` itself, write `&mut {name}`")));
                None
            }
        }
    }

    /// `address of e`.
    fn address_expr(&mut self, e: &ast::Expr, inner: &ast::Expr) -> Option<Val> {
        self.g.uses_borrows.set(true);
        if !self.g.unsafe_allowed {
            self.diags.push(unsafe_disabled(Span::new(e.span.start, e.span.start + 7), "needs the file flag"));
        }
        if !ast_place(inner) {
            let text = self.quote(inner.span, "this value");
            self.diags.push(
                Diagnostic::at(&codes::E0327, e.span, "`address of` needs a variable or a field, like `address of n`")
                    .note(format!("{text} is a temporary value; it has no address"))
                    .help("name the value first"),
            );
            self.expr(inner);
            return None;
        }
        let p = self.lvalue(inner)?;
        let l = &self.locals[p.local];
        if !p.through_deref && !p.mut_field && !l.mutable && !l.open && !l.is_self {
            self.diags.push(Diagnostic::at(
                &codes::E0901,
                e.span,
                "`address of` an immutable binding (which gives a `*fixed T`) is not supported yet",
            ));
            return None;
        }
        self.read_local(p.local, inner.span)?;
        let ty = Type::ptr(p.ty);
        let place = self.place_expr(&p);
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::AddressOf(Box::new(place)), ty)))
    }

    /// `p == none` / `p != none` (either side).
    fn none_compare(&mut self, op: BinOp, l: &ast::Expr, r: &ast::Expr, span: Span) -> Option<Val> {
        let _ = span;
        let (other, none_left) = if matches!(l.kind, ExprKind::NoneValue) { (r, true) } else { (l, false) };
        if matches!(other.kind, ExprKind::NoneValue) {
            self.diags.push(Diagnostic::at(&codes::E0901, other.span, "`none` as a value is not supported yet"));
            return None;
        }
        let v = self.expr_raw(other)?;
        match v {
            Val::Expr(x) if x.ty.is_ptr() => {
                self.g.uses_borrows.set(true);
                let null = hir::Expr::new(hir::ExprKind::NullPtr, x.ty);
                let hop = if op == BinOp::Eq { hir::BinOp::Eq } else { hir::BinOp::Ne };
                let (a, b) = if none_left { (null, x) } else { (x, null) };
                Some(Val::Expr(hir::Expr::new(hir::ExprKind::Binary(hop, Box::new(a), Box::new(b)), Type::Bool)))
            }
            _ => {
                self.diags.push(Diagnostic::at(&codes::E0901, span, "`none` as a value is not supported yet"));
                None
            }
        }
    }

    /// Check a call's arguments against function `id`'s parameters.
    fn args(&mut self, id: FuncId, args: &[ast::Expr], span: Span) -> Option<Vec<hir::Expr>> {
        let params = self.g.sigs[id].params.clone();
        if params.len() != args.len() {
            let (name, def) = (&self.g.sigs[id].name, self.g.sigs[id].span);
            let n = params.len();
            self.diags.push(
                Diagnostic::at(&codes::E0103, span, format!("`{name}` takes {n} argument{}, but {} given", if n == 1 { "" } else { "s" }, args.len()))
                    .label(def, format!("`{name}` is defined here")),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let mut out = Vec::new();
        let mut ok = true;
        for (a, p) in args.iter().zip(&params) {
            let v = match p {
                // The callee owns its parameters.
                Some(p) if p.is_borrow() => self.coerce_borrow(a, *p, Some(id)),
                Some(p) => self.coerce(a, *p).map(|e| self.owned(e)),
                None => {
                    self.expr(a);
                    None
                }
            };
            match v {
                Some(v) => out.push(v),
                None => ok = false,
            }
        }
        ok.then_some(out)
    }

    fn no_targs(&mut self, name: &str, targs: &[ast::TypeName]) {
        if let Some(t) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, t.span, format!("`{name}` takes no type arguments")));
        }
    }

    /// `Name::method(args)`: a static method of struct `s`.
    fn static_call(&mut self, s: StructId, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        let sname = self.g.structs[s].name.clone();
        let Some(m) = self.g.method(s, name).cloned() else {
            self.unknown_method(s, name, name_span, "value");
            for a in args {
                self.expr(a);
            }
            return None;
        };
        let func = m.func;
        if !m.is_static {
            self.diags.push(
                Diagnostic::at(&codes::E0213, name_span, format!("`{sname}::{name}` is not static: it needs a `{sname}` value"))
                    .note("called on the struct")
                    .label(self.g.sigs[func].span, "declared without `static`")
                    .help(format!("call it on a value, like `p::{name}()` with `{sname} p`")),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let visible = self.check_method_visible(s, &m, name_span);
        self.no_targs(name, targs);
        let args = self.args(func, args, span);
        let ty = self.g.sigs[func].ret?;
        if !visible {
            return None;
        }
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Call(func, args?), ty)))
    }

    fn unknown_method(&mut self, s: StructId, name: &str, at: Span, shown: &str) {
        let info = &self.g.structs[s];
        let methods: Vec<&str> = info.methods.iter().filter(|m| m.operator.is_none()).map(|m| m.name.as_str()).collect();
        let mut d = Diagnostic::at(&codes::E0201, at, format!("`{}` has no method `{name}`", info.name));
        d = if info.fields.iter().any(|f| f.name == name) {
            d.help(format!("`{name}` is a field: write `{shown}.{name}`"))
        } else if let Some(near) = similar(name, &methods) {
            d.help(format!("did you mean `{near}`?"))
        } else if methods.is_empty() {
            d.help(format!("`{}` has no methods", info.name))
        } else {
            d.help(format!("its methods are {}", list(&methods)))
        };
        self.diags.push(d);
    }

    fn io_call(&mut self, name: &str, name_span: Span, module_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        if !self.g.io_imported {
            self.diags.push(
                Diagnostic::at(&codes::E0206, module_span, "module `io` is used but not imported")
                    .note("not imported")
                    .help("add `in io;` at the top of the file"),
            );
            // Say it once per file.
            self.g.io_imported = true;
        }
        let newline = match name {
            "out" => false,
            "outln" => true,
            "outf" => {
                if let Some(t) = targs.first() {
                    self.diags.push(Diagnostic::at(&codes::E0103, t.span, "`io::outf` takes no type arguments"));
                }
                return self.outf(args, span);
            }
            _ => {
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("module `io` has no function `{name}`"))
                        .help("it has `io::out`, `io::outln` and `io::outf`"),
                );
                for a in args {
                    self.expr(a);
                }
                return None;
            }
        };
        if let Some(t) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, t.span, format!("`io::{name}` takes no type arguments")));
        }
        if args.len() != 1 {
            self.diags.push(Diagnostic::at(
                &codes::E0103,
                span,
                format!("`io::{name}` takes 1 argument, but {} given", args.len()),
            ));
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let arg = match self.expr(&args[0])? {
            Val::Expr(a) if a.ty == Type::Str => a,
            other => {
                let found = match &other {
                    Val::Const(_) => "a number".to_string(),
                    Val::Expr(a) => format!("`{}`", a.ty),
                };
                let o = origin(&args[0]);
                let text = self.text(o.span);
                let fix = if o.compound { format!("({text})::text()") } else { format!("{text}::text()") };
                let mut d = Diagnostic::at(&codes::E0101, o.span, format!("`io::{name}` prints text, found {found}"))
                    .note(format!("this is {found}"));
                let has_text = match &other {
                    Val::Const(_) => true,
                    Val::Expr(a) => match a.ty.struct_id() {
                        Some(s) => self.g.method(s, "text").is_some(),
                        None => a.ty != Type::None,
                    },
                };
                if has_text && !text.contains('\n') {
                    d = d.help(format!("turn it into text: `io::{name}({fix})`"));
                }
                self.diags.push(d);
                return None;
            }
        };
        let arg = self.place(arg);
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Out { newline, arg: Box::new(arg) }, Type::None)))
    }

    fn method(&mut self, recv: &ast::Expr, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        match self.expr(recv) {
            Some(Val::Expr(v)) if v.ty.struct_id().is_some() => {
                let s = v.ty.struct_id().unwrap();
                self.struct_method(s, v, recv.span, name, name_span, targs, args, span)
            }
            Some(Val::Expr(v)) if v.ty == Type::Str && name != "to" => self.string_method(v, name, name_span, targs, args),
            value => self.value_method(value, recv, name, name_span, targs, args),
        }
    }

    /// `::text()` and `::to<T>()` on numbers and bools.
    fn value_method(&mut self, value: Option<Val>, recv: &ast::Expr, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr]) -> Option<Val> {
        for a in args {
            if !matches!(name, "text" | "to") {
                self.expr(a);
            }
        }
        let value = value?;
        let no_args = |cx: &mut Self| {
            if let Some(a) = args.first() {
                cx.diags.push(Diagnostic::at(&codes::E0103, a.span, format!("`{name}` takes no arguments")));
            }
        };
        match name {
            "text" => {
                no_args(self);
                if let Some(t) = targs.first() {
                    self.diags.push(Diagnostic::at(&codes::E0103, t.span, "`text` takes no type arguments"));
                }
                let v = self.materialise(value, recv.span)?;
                if !v.ty.is_numeric() && v.ty != Type::Bool {
                    self.diags.push(
                        Diagnostic::at(&codes::E0201, name_span, format!("`{}` has no method `text`", v.ty))
                            .label(recv.span, format!("this is `{}`", v.ty)),
                    );
                    return None;
                }
                Some(Val::Expr(hir::Expr::new(hir::ExprKind::Text(Box::new(v)), Type::Str)))
            }
            "to" => {
                no_args(self);
                if targs.len() != 1 {
                    self.diags.push(
                        Diagnostic::at(&codes::E0103, name_span, "`to` needs the type to convert to")
                            .help("for example `value::to<uint8>()`"),
                    );
                    return None;
                }
                let to = self.g.resolve(&targs[0], self.diags)?;
                if !to.is_numeric() {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, targs[0].span, format!("`to` converts between number types, not to `{to}`")),
                    );
                    return None;
                }
                match value {
                    Val::Const(c) => Some(Val::Expr(self.fit_const(&c, to, recv.span, true))),
                    Val::Expr(v) if v.ty == to => Some(Val::Expr(v)),
                    Val::Expr(v) if v.ty.is_numeric() => {
                        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Convert(Box::new(v)), to)))
                    }
                    Val::Expr(v) => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0101, recv.span, format!("`to` converts numbers, found `{}`", v.ty))
                                .note(format!("this is `{}`", v.ty)),
                        );
                        None
                    }
                }
            }
            _ => {
                let on = match &value {
                    Val::Const(_) => "numbers have".to_string(),
                    Val::Expr(v) => format!("`{}` has", v.ty),
                };
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("{on} no method `{name}`"))
                        .help("the methods so far are `text()` and `to<T>()`"),
                );
                None
            }
        }
    }

    /// `recv::name(args)` on a value of struct `s`.
    #[allow(clippy::too_many_arguments)]
    fn struct_method(
        &mut self,
        s: StructId,
        recv: hir::Expr,
        recv_span: Span,
        name: &str,
        name_span: Span,
        targs: &[ast::TypeName],
        args: &[ast::Expr],
        span: Span,
    ) -> Option<Val> {
        let Some(m) = self.g.method(s, name).cloned() else {
            let shown = self.text(recv_span).to_string();
            self.unknown_method(s, name, name_span, &shown);
            for a in args {
                self.expr(a);
            }
            return None;
        };
        let sname = self.g.structs[s].name.clone();
        if name == "drop" {
            let shown = self.text(recv_span).to_string();
            self.diags.push(
                Diagnostic::at(&codes::E0331, name_span, format!("`drop` runs by itself; write `destroy {shown};` to end `{shown}` now"))
                    .note("called here")
                    .help(format!("write `destroy {shown};`")),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        }
        if m.is_static {
            self.diags.push(
                Diagnostic::at(&codes::E0213, name_span, format!("`{name}` is a static method: it is called on the struct, not on a value"))
                    .note("called on a value")
                    .label(self.g.sigs[m.func].span, "declared `static`")
                    .help(format!("write `{sname}::{name}(...)`")),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let visible = self.check_method_visible(s, &m, name_span);
        self.no_targs(name, targs);
        let args = self.args(m.func, args, span);
        let ty = self.g.sigs[m.func].ret?;
        if !visible {
            return None;
        }
        self.note_receiver(&recv, recv_span, m.func, name_span);
        let mut all = vec![self.receiver_value(recv)];
        all.extend(args?);
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Call(m.func, all), ty)))
    }

    /// `lhs op rhs` where `lhs` is a struct value: the left side leads, so
    /// this is `lhs`'s operator method, or the automatic `==` / `!=`.
    fn struct_operator(&mut self, op: BinOp, lhs: hir::Expr, lo: Origin, rv: Val, ro: Origin, span: Span) -> Option<Val> {
        let s = lhs.ty.struct_id().unwrap();
        let t = lhs.ty;
        let not = |e: hir::Expr| hir::Expr::new(hir::ExprKind::Not(Box::new(e)), Type::Bool);
        if let Some(m) = self.g.operator(s, op).cloned() {
            return self.operator_call(s, &m, lhs, lo, rv, ro, span).map(Val::Expr);
        }
        match op {
            BinOp::Ne if self.g.operator(s, BinOp::Eq).is_some() => {
                let m = self.g.operator(s, BinOp::Eq).cloned().unwrap();
                self.operator_call(s, &m, lhs, lo, rv, ro, span).map(|e| Val::Expr(not(e)))
            }
            BinOp::Eq | BinOp::Ne => {
                let rhs = self.into(rv, t, ro)?;
                let rhs = self.copied(rhs);
                let lhs = self.receiver_value(lhs);
                let eq = self.g.struct_eq(s, lhs, rhs);
                Some(Val::Expr(if op == BinOp::Eq { eq } else { not(eq) }))
            }
            _ => {
                let sym = op.symbol();
                let ret = if op.class() == OpClass::Comparison { "bool".to_string() } else { t.to_string() };
                self.diags.push(
                    Diagnostic::at(&codes::E0112, span, format!("`{t}` has no `{sym}` operator"))
                        .label(lo.span, format!("this is `{t}`"))
                        .help(format!("give `{t}` an operator method: `{ret} {sym}({t} o) {{ ... }}`")),
                );
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn operator_call(&mut self, s: StructId, m: &MethodInfo, lhs: hir::Expr, lo: Origin, rv: Val, ro: Origin, span: Span) -> Option<hir::Expr> {
        if !self.check_method_visible(s, m, span) {
            return None;
        }
        let sig = &self.g.sigs[m.func];
        let (param, ty) = (sig.params[0]?, sig.ret?);
        let rhs = self.into(rv, param, ro)?;
        let rhs = self.copied(rhs);
        self.note_receiver(&lhs, lo.span, m.func, span);
        let lhs = self.receiver_value(lhs);
        Some(hir::Expr::new(hir::ExprKind::Call(m.func, vec![lhs, rhs]), ty))
    }

    fn unary(&mut self, op: UnOp, inner: &ast::Expr, span: Span) -> Option<Val> {
        if op == UnOp::Not {
            let v = self.coerce(inner, Type::Bool)?;
            return Some(Val::Expr(hir::Expr::new(hir::ExprKind::Not(Box::new(v)), Type::Bool)));
        }
        let (sym, wants) = if op == UnOp::Neg { ("-", "a number") } else { ("~", "a whole number") };
        match self.expr(inner)? {
            Val::Const(c) if op == UnOp::Neg => Some(Val::Const(c.neg())),
            Val::Const(c) => match c.bit_not() {
                Ok(c) => Some(Val::Const(c)),
                Err(_) => {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, span, "`~` needs a whole number, found a decimal").label(inner.span, "this is a decimal"),
                    );
                    None
                }
            },
            Val::Expr(v) if (op == UnOp::Neg && v.ty.is_numeric()) || (op == UnOp::BitNot && v.ty.is_int()) => {
                let ty = v.ty;
                let kind = if op == UnOp::Neg { hir::ExprKind::Neg(Box::new(v)) } else { hir::ExprKind::BitNot(Box::new(v)) };
                Some(Val::Expr(hir::Expr::new(kind, ty)))
            }
            Val::Expr(v) => {
                let mut d = Diagnostic::at(&codes::E0101, inner.span, format!("`{sym}` needs {wants}, found `{}`", v.ty)).note(format!("this is `{}`", v.ty));
                if v.ty.struct_id().is_some() {
                    d = d.help("operator methods are for two-sided operators only");
                }
                self.diags.push(d);
                None
            }
        }
    }

    fn fold_error(&mut self, err: FoldError, op: BinOp, span: Span, r: Origin) {
        let sym = op.symbol();
        let d = match err {
            FoldError::DivisionByZero => Diagnostic::at(&codes::E0107, span, "this constant expression divides by zero").label(r.span, "this is zero"),
            FoldError::TooLarge => Diagnostic::at(&codes::E0002, span, "this constant expression is too large to evaluate")
                .note("the result would not fit any number type"),
            FoldError::NeedsWhole => Diagnostic::at(&codes::E0101, span, format!("`{sym}` needs whole numbers, found a decimal")),
            FoldError::NegativeShift => Diagnostic::at(&codes::E0101, r.span, format!("cannot `{sym}` an untyped constant by a negative amount"))
                .note("negative")
                .help("give the left side a type (for example a `u32` suffix) for the amount to wrap around its width"),
            FoldError::NotANumber => Diagnostic::at(&codes::E0107, span, "this constant expression has no numeric result"),
        };
        self.diags.push(d);
    }

    /// A two-sided operator applied to already-checked operands.
    fn binary_vals(&mut self, op: BinOp, lv: Val, lo: Origin, rv: Val, ro: Origin, span: Span) -> Option<Val> {
        let sym = op.symbol();
        let class = op.class();

        if let (Val::Const(a), Val::Const(b)) = (&lv, &rv) {
            return match Const::binary(op, a, b) {
                Ok(Folded::Value(c)) => Some(Val::Const(c)),
                Ok(Folded::Bool(b)) => Some(Val::Expr(hir::Expr::new(hir::ExprKind::Bool(b), Type::Bool))),
                Err(err) => {
                    self.fold_error(err, op, span, ro);
                    None
                }
            };
        }

        // An untyped constant on the left takes the right side's type if it
        // fits there exactly; otherwise it gets its own default type and leads.
        let lv = match (lv, &rv) {
            (Val::Const(c), Val::Expr(r)) => {
                let counts = class == OpClass::Shift || (op == BinOp::Pow && !c.is_decimal() && r.ty.is_int());
                if !counts && r.ty.is_numeric() && c.fits(r.ty) {
                    Val::Expr(self.fit_const(&c, r.ty, lo.span, true))
                } else {
                    Val::Expr(self.materialise(Val::Const(c), lo.span)?)
                }
            }
            (lv, _) => lv,
        };
        let Val::Expr(lhs) = lv else { unreachable!() };
        let lead = lhs.ty;

        if lead == Type::Str {
            return self.string_binary(op, lhs, lo, rv, ro);
        }
        if lead.struct_id().is_some() && matches!(class, OpClass::Arithmetic | OpClass::Comparison) {
            return self.struct_operator(op, lhs, lo, rv, ro, span);
        }

        let wants = match class {
            OpClass::Arithmetic => "numbers",
            OpClass::Shift | OpClass::Bitwise => "whole numbers",
            OpClass::Comparison | OpClass::Logic => "numbers",
        };
        let lead_ok = match class {
            OpClass::Arithmetic => lead.is_numeric(),
            OpClass::Shift | OpClass::Bitwise => lead.is_int(),
            OpClass::Comparison => lead.is_numeric() || (lead == Type::Bool && matches!(op, BinOp::Eq | BinOp::Ne)),
            OpClass::Logic => unreachable!(),
        };
        if !lead_ok {
            let mut d = Diagnostic::at(&codes::E0101, lo.span, format!("`{sym}` needs {wants}, found `{lead}`")).note(format!("this is `{lead}`"));
            if lead == Type::Bool && class == OpClass::Comparison {
                d = d.help("`bool` values can only be compared with `==` and `!=`");
            }
            self.diags.push(d);
            return None;
        }

        // Counts: the right side of a shift or an integer power keeps its own type.
        if class == OpClass::Shift || (op == BinOp::Pow && lead.is_int()) {
            let count = match rv {
                Val::Const(c) if !c.is_decimal() => {
                    if class == OpClass::Shift {
                        self.check_shift_amount(&c, lead, sym, ro);
                    }
                    // Only the low bits of a shift amount matter, so any width will do.
                    match c.default_type() {
                        Some(ty) => self.fit_const(&c, ty, ro.span, true),
                        None if class == OpClass::Shift => int_lit(c.wrap_to_int(Type::U32), Type::U32),
                        None => self.materialise(Val::Const(c), ro.span)?,
                    }
                }
                Val::Expr(r) if r.ty.is_int() => r,
                // A fractional exponent on a whole-number base: the left side
                // leads, so the exponent becomes a whole number (with a warning).
                other if op == BinOp::Pow && !matches!(&other, Val::Expr(r) if !r.ty.is_numeric()) => self.into(other, lead, ro)?,
                other => {
                    let found = match &other {
                        Val::Const(_) => "a decimal".to_string(),
                        Val::Expr(r) => format!("`{}`", r.ty),
                    };
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, ro.span, format!("`{sym}` needs a whole number on the right, found {found}"))
                            .note(format!("this is {found}")),
                    );
                    return None;
                }
            };
            let kind = if class == OpClass::Shift {
                hir::ExprKind::Shift { left: op == BinOp::Shl, value: Box::new(lhs), amount: Box::new(count) }
            } else {
                hir::ExprKind::IntPow(Box::new(lhs), Box::new(count))
            };
            return Some(Val::Expr(hir::Expr::new(kind, lead)));
        }

        // Everything else: the left side leads and the right side converts to it.
        if let Val::Expr(r) = &rv
            && r.ty != lead
            && !(r.ty.is_numeric() && lead.is_numeric())
        {
            let mut d = Diagnostic::at(&codes::E0101, ro.span, format!("`{sym}` has `{lead}` on the left but `{}` on the right", r.ty))
                .note(format!("this is `{}`", r.ty))
                .label(lo.span, format!("this is `{lead}`"));
            if r.ty == Type::Str && op == BinOp::Add && lead != Type::None && lead.struct_id().is_none() {
                d = d.help(self.text_fix(lo));
            } else if r.ty == Type::Bool || lead == Type::Bool {
                d = d.help("`bool` is not a number and never converts to or from one");
            } else if r.ty.struct_id().is_some() {
                d = d.help(format!("the left side leads, so an operator method of `{}` is only found with the `{}` on the left", r.ty, r.ty));
            }
            self.diags.push(d);
            return None;
        }
        if let (Val::Const(c), BinOp::Div | BinOp::Rem, true) = (&rv, op, lead.is_int())
            && c.wrap_to_int(lead) == 0
            && c.fits(lead)
        {
            self.diags.push(
                Diagnostic::at(&codes::E0107, span, "this divides by a constant zero")
                    .label(ro.span, "this is zero")
                    .help("whole-number division by zero always panics"),
            );
            return None;
        }
        let rhs = self.into(rv, lead, ro)?;
        let hop = match op {
            BinOp::Add => hir::BinOp::Add,
            BinOp::Sub => hir::BinOp::Sub,
            BinOp::Mul => hir::BinOp::Mul,
            BinOp::Div => hir::BinOp::Div,
            BinOp::Rem => hir::BinOp::Rem,
            BinOp::Pow => hir::BinOp::Pow,
            BinOp::BitAnd => hir::BinOp::BitAnd,
            BinOp::BitOr => hir::BinOp::BitOr,
            BinOp::BitXor => hir::BinOp::BitXor,
            BinOp::Eq => hir::BinOp::Eq,
            BinOp::Ne => hir::BinOp::Ne,
            BinOp::Lt => hir::BinOp::Lt,
            BinOp::Le => hir::BinOp::Le,
            BinOp::Gt => hir::BinOp::Gt,
            BinOp::Ge => hir::BinOp::Ge,
            BinOp::Shl | BinOp::Shr | BinOp::And | BinOp::Or => unreachable!(),
        };
        let ty = if hop.is_comparison() { Type::Bool } else { lead };
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Binary(hop, Box::new(lhs), Box::new(rhs)), ty)))
    }

    /// A constant shift amount outside `0..width` wraps around the width (W0106).
    fn check_shift_amount(&mut self, amount: &Const, value_ty: Type, sym: &str, at: Origin) {
        let Type::Int { bits, .. } = value_ty else { return };
        let width_ty = Type::Int { bits: 128, signed: false };
        let wrapped = amount.wrap_to_int(width_ty) % bits as u128;
        if matches!(amount, Const::Int(v) if *v == wrapped.into()) {
            return;
        }
        let what = self.quote(at.span, "this amount");
        self.diags.push(
            Diagnostic::at(&codes::W0106, at.span, format!("shifting a `{value_ty}` by {what} wraps around to a shift by {wrapped}"))
                .note(format!("`{value_ty}` is {bits} bits wide"))
                .note_line(format!("a shift amount outside 0 to {} wraps around the width", bits - 1))
                .help(format!("write `{sym} {wrapped}` if that is what you mean, or use a wider type")),
        );
    }
}
