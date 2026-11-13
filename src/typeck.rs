//! Name resolution, type checking and definite-assignment analysis.
//! Turns the syntax tree into `hir`, reporting every problem it finds.
//! Loops, jumps and `match` are in `control`; strings are in `strings`.

mod addresses;
mod constants;
mod construct;
mod control;
mod dynamic;
mod enums;
mod failures;
mod fill;
pub mod index;
mod generics;
mod interfaces;
mod iterable;
mod layout;
mod lambdas;
mod lists;
mod maps;
mod optionals;
mod overload;
mod strings;
mod ties;
mod units;

use std::collections::HashMap;

use control::{Target, Test};
use enums::EnumInfo;

use crate::ast::{self, BinOp, ExprKind, OpClass, Stmt, UnOp};
use crate::{lexer, parser};
use crate::consts::{Const, FoldError, Folded};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, FuncId, LocalId, StructId};
use crate::types::{self, CKind, Type, TypeError};

/// A function or method as callers see it.
struct Sig {
    /// How messages name it: `add`, `Point::move`.
    name: String,
    /// The declared parameters (not `self`). `None` where the written type
    /// did not resolve (already reported).
    params: Vec<Option<Type>>,
    ret: Option<Type>,
    span: Span,
    /// The failure types the function lists after `fails`.
    fails: Vec<hir::EnumId>,
    /// What it returns when it fails: an outcome of `ret` and the failure.
    outcome: Option<Type>,
}

struct StructInfo {
    name: String,
    name_span: Span,
    ty: Type,
    fields: Vec<FieldInfo>,
    methods: Vec<MethodInfo>,
    /// Declared `stays`.
    stays: bool,
    /// Indexes of the borrow fields named by `lives`.
    lives: Vec<usize>,
    /// The `drop()` method.
    has_drop: Option<FuncId>,
    /// Written constructors (`construct.rs`).
    ctors: Vec<construct::CtorInfo>,
    /// `Person(auto);`: the automatic constructor stays next to the written ones.
    auto_ctor: bool,
    /// The default `def`, an index into `Globals::defs`.
    default_def: Option<usize>,
    /// Written with a `lives` clause: it is a holder, even before its
    /// fields are checked.
    declared_lives: bool,
    /// Set for the stand-in of a type parameter while its generic is checked
    /// at its definition (`interfaces::placeholder`).
    placeholder: Option<interfaces::Placeholder>,
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
/// (Whether it changes `self` is inferred on the HIR, `ownership::self_mode`,
/// and checked on the MIR, `mir::mutability`.)
#[derive(Default)]
struct SelfUse {
    used: bool,
}

/// Everything shared by all function bodies.
#[derive(Default)]
struct Globals {
    settings: Vec<(usize, crate::config::Behaviour)>,
    sigs: Vec<Sig>,
    funcs_by_name: HashMap<String, FuncId>,
    structs: Vec<StructInfo>,
    structs_by_name: HashMap<String, StructId>,
    enums: Vec<EnumInfo>,
    enums_by_name: HashMap<String, hir::EnumId>,
    /// The optional enum of each `T`, for `T?` (`optionals.rs`).
    optional_ids: HashMap<Type, hir::EnumId>,
    /// Indexed by `FuncId`; only filled in for methods.
    self_use: Vec<SelfUse>,
    /// The automatic `==` function of each struct that needs one.
    eq_funcs: HashMap<StructId, FuncId>,
    /// Automatic `==` functions asked for whose bodies are not built yet.
    eq_pending: Vec<StructId>,
    /// List `==` and `contains` functions by kind and list type (S6C.2), and
    /// those whose bodies are not built yet.
    list_funcs: HashMap<(lists::ListFunc, Type), FuncId>,
    list_pending: Vec<(lists::ListFunc, Type, FuncId)>,
    /// Address compare functions by kind and type (`addresses.rs`), and those
    /// whose bodies are not built yet.
    addr_funcs: HashMap<(addresses::AddrFunc, Type), FuncId>,
    addr_pending: Vec<(addresses::AddrFunc, Type, FuncId)>,
    /// Region ties of instances whose type arguments hold borrows (`hir::Program::ties`).
    ties: std::collections::BTreeMap<FuncId, Vec<Vec<(Option<usize>, usize)>>>,
    tie_jobs: Vec<ties::Job>,
    /// C functions from header imports.
    foreign_funcs: std::collections::HashSet<FuncId>,
    io_imported: bool,
    time_imported: bool,
    fs_imported: bool,
    os_imported: bool,
    /// The file flag `//--!$unsafe: true`.
    unsafe_allowed: bool,
    /// Memoised layout, ownership and copy answers (`layout`).
    layouts: std::cell::RefCell<layout::Cache>,
    /// Map key checks that wait for the methods (`maps.rs`).
    keys: maps::KeyChecks,
    /// The units declared in the file (`typeck::units`).
    units: std::cell::RefCell<units::UnitEnv>,
    /// The `def`s of the file (`fill.rs`).
    defs: Vec<fill::DefInfo>,
    /// Failures, outcomes and the functions that report them (`failures.rs`).
    fail: failures::FailState,
    /// Generic functions, instances and the bodies still to check (`generics.rs`).
    mono: generics::State,
    /// Interfaces and who implements them (`interfaces.rs`).
    ifaces: interfaces::State,
    /// The tables of the borrows of interfaces (`dynamic.rs`).
    dynamic: dynamic::State,
    lambdas: lambdas::State,
    /// The type parameters of the instance being resolved or checked, and what
    /// they stand for here.
    tenv: HashMap<String, Type>,
    /// Named constants: top-level values with a constant initializer (`constants.rs`).
    consts: HashMap<String, constants::ConstInfo>,
    /// Top-level values that are only known while the program runs (already E0901).
    run_values: std::collections::HashSet<String>,
}

impl Globals {
    fn settings_at(&self, span: Span) -> crate::config::Behaviour {
        let end = self.settings.partition_point(|(base, _)| *base <= span.start);
        self.settings[..end].last().map_or_else(Default::default, |(_, settings)| *settings)
    }

    fn add_sig(&mut self, sig: Sig) -> FuncId {
        self.sigs.push(sig);
        self.self_use.push(SelfUse::default());
        self.sigs.len() - 1
    }

    fn resolve_base_quiet(&self, t: &ast::TypeName) -> Option<Type> {
        if t.self_ty && let Some(&ty) = self.tenv.get(generics::SELF) {
            return Some(ty);
        }
        if !t.dims.is_empty() {
            return None;
        }
        if !t.args.is_empty() {
            return self.instance_of(t);
        }
        if let Some(&ty) = self.tenv.get(&t.name) {
            // A unit parameter is no type (E0609, reported by `resolve`).
            return (ty.as_unit_arg().is_none() && ty.as_val().is_none()).then_some(ty);
        }
        if t.name == "none" {
            return Some(Type::None);
        }
        Type::from_name(&t.name)
            .ok()
            .or_else(|| self.structs_by_name.get(&t.name).map(|&id| self.structs[id].ty))
            .or_else(|| self.enums_by_name.get(&t.name).map(|&id| self.enums[id].ty))
    }

    /// Is `t` the built-in `List<X>` (no struct, enum or template of that name)?
    fn is_list_name(&self, t: &ast::TypeName) -> bool {
        t.name == "List"
            && !t.args.is_empty()
            && !self.structs_by_name.contains_key("List")
            && !self.enums_by_name.contains_key("List")
            && !self.mono.type_templates.contains_key("List")
    }

    /// The instance a type written with arguments names (`Stack<uint32>`,
    /// `Optional<uint32>`), if it was made.
    fn instance_of(&self, t: &ast::TypeName) -> Option<Type> {
        let args: Option<Vec<Type>> = match self.mono.type_templates.get(&t.name) {
            Some(tpl) => generics::resolve_targs(self, &t.name, &tpl.tparams, &t.args, true, &mut Vec::new()),
            None => t.args.iter().map(|a| if a.is_number { None } else { self.resolve_quiet(a) }).collect(),
        };
        let args = args?;
        if maps::is_map_name(&t.name) && t.args.len() == 2 {
            let [key, val] = args[..] else { return None };
            self.check_key(key, t.args[0].span).ok()?;
            self.check_map_value(val, t.args[1].span, &t.name).ok()?;
            return Some(self.make_map(key, val, t.name == "OrderedMap"));
        }
        if t.name == "Optional" {
            let [inner] = args[..] else { return None };
            return self.optional_ids.get(&inner).map(|&id| self.enums[id].ty);
        }
        if self.is_list_name(t) {
            let [elem] = args[..] else { return None };
            return (elem != Type::None).then(|| self.make_list(elem));
        }
        self.mono.type_instances.get(&(t.name.clone(), args)).copied()
    }

    /// `t` with its `&`, `&mut` and `*` marks applied to the named type.
    fn resolve_quiet(&self, t: &ast::TypeName) -> Option<Type> {
        if let Some(spec) = &t.unit_arg {
            return self.resolve_unit_arg(spec, t.span, &mut Vec::new());
        }
        if t.func.is_some() {
            return self.resolve_func(t, &mut Vec::new());
        }
        if let Some(fat) = dynamic::dyn_borrow(self, t) {
            return Some(fat);
        }
        if !t.dims.is_empty() {
            return self.resolve_dims(t, &mut Vec::new());
        }
        if let Some(spec) = &t.unit {
            return self.resolve_unit_type(t, spec, &mut Vec::new());
        }
        let base = optionals::wrap(self, t, self.resolve_base_quiet(t)?)?;
        Some(t.mods.iter().rev().fold(base, |ty, (m, _)| match m {
            ast::TypeMod::Ref { mutable } => Type::borrow(*mutable, ty),
            ast::TypeMod::Ptr => Type::ptr(ty),
            ast::TypeMod::FixedPtr => Type::fixed_ptr(ty),
                ast::TypeMod::ForeignPtr => Type::foreign(ty),
        }))
    }

    fn resolve(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        if let Some(spec) = &t.unit_arg {
            return self.resolve_unit_arg(spec, t.span, diags);
        }
        if t.func.is_some() {
            return self.resolve_func(t, diags);
        }
        if t.args.is_empty() && t.unit.is_none() && self.tenv.get(&t.name).is_some_and(|ty| ty.as_unit_arg().is_some()) {
            diags.push(
                Diagnostic::at(&codes::E0609, t.span, format!("`{}` is a unit, so it cannot be a type", t.name))
                    .note("a unit parameter stands for a unit")
                    .help(format!("use it inside a number type: `float64<{}>`", t.name)),
            );
            return None;
        }
        if t.args.is_empty() && t.unit.is_none() && self.tenv.get(&t.name).is_some_and(|ty| ty.as_val().is_some()) {
            diags.push(
                Diagnostic::at(&codes::E0609, t.span, format!("`{}` is a value, so it cannot be a type", t.name))
                    .note("a value parameter stands for a number")
                    .help(format!("use it where a value goes, or as an array length: `T[{}]`", t.name)),
            );
            return None;
        }
        if let Some((_, span)) = t.mods.first() {
            if !self.unsafe_allowed && let Some((_, at)) = t.mods.iter().find(|(m, _)| matches!(m, ast::TypeMod::Ptr | ast::TypeMod::FixedPtr) || (matches!(m, ast::TypeMod::ForeignPtr) && !is_synthetic(t.span))) {
                diags.push(unsafe_disabled(*at, "raw pointer type here"));
            }
            if self.resolve_base_quiet(t) == Some(Type::None) && !matches!(t.mods.first(), Some((ast::TypeMod::ForeignPtr, _))) {
                diags.push(Diagnostic::at(&codes::E0901, *span, "a borrow or pointer of `none` is not supported yet"));
                return None;
            }
        }
        if interfaces::as_value_type(self, t, diags) {
            return None;
        }
        if !t.dims.is_empty() {
            return self.resolve_dims(t, diags);
        }
        if maps::is_map_name(&t.name) {
            if t.args.len() != 2 {
                diags.push(
                    Diagnostic::at(&codes::E0602, t.span, format!("`{}` takes 2 type arguments, but {} given", t.name, t.args.len()))
                        .help(format!("write `{}<string, uint32>`", t.name)),
                );
                return None;
            }
            let base = self.resolve_map(t, diags)?;
            let base = optionals::wrap(self, t, base)?;
            return Some(t.mods.iter().rev().fold(base, |ty, (m, _)| match m {
                ast::TypeMod::Ref { mutable } => Type::borrow(*mutable, ty),
                ast::TypeMod::Ptr => Type::ptr(ty),
                ast::TypeMod::FixedPtr => Type::fixed_ptr(ty),
                ast::TypeMod::ForeignPtr => Type::foreign(ty),
            }));
        }
        if self.is_list_name(t) {
            // `List<&string>` is `string[?]` spelled out; the only way to write a list of borrows.
            if t.args.len() != 1 || t.args[0].is_number {
                diags.push(Diagnostic::at(&codes::E0602, t.span, "`List` takes 1 type argument").help("write `List<&string>`, or `string[?]`"));
                return None;
            }
            let elem = self.resolve_binding_type(&t.args[0], diags)?;
            let base = self.collection_of(elem, &ast::Dim::List(t.span), diags)?;
            let base = optionals::wrap(self, t, base)?;
            return Some(t.mods.iter().rev().fold(base, |ty, (m, _)| match m {
                ast::TypeMod::Ref { mutable } => Type::borrow(*mutable, ty),
                ast::TypeMod::Ptr => Type::ptr(ty),
                ast::TypeMod::FixedPtr => Type::fixed_ptr(ty),
                ast::TypeMod::ForeignPtr => Type::foreign(ty),
            }));
        }
        if !t.args.is_empty() {
            match generics::check_type_args(self, t, diags) {
                Some(true) => {}
                Some(false) => return None,
                None => return self.type_args_error(t, diags),
            }
        } else if self.mono.type_templates.contains_key(&t.name) && !t.self_ty && !self.tenv.contains_key(&t.name) {
            generics::type_by_name(self, t, diags);
            return None;
        }
        if t.optional.is_some() {
            if let Some(ty) = self.resolve_quiet(t) {
                return Some(ty);
            }
            // The optional was not made, so its `T` is unknown: report that name.
            self.resolve(&ast::TypeName { mods: Vec::new(), optional: None, ..t.clone() }, diags);
            return None;
        }
        if let Some(spec) = &t.unit {
            return self.resolve_unit_type(t, spec, diags);
        }
        if let Some(ty) = self.resolve_quiet(t) {
            return Some(ty);
        }
        if !t.args.is_empty() {
            generics::missing_instance(self, t, diags);
            return None;
        }
        match Type::from_name(&t.name) {
            Ok(ty) => Some(ty),
            Err(TypeError::NotYet) => {
                diags.push(Diagnostic::at(&codes::E0901, t.span, format!("the type `{}` is not supported yet", t.name)));
                None
            }
            Err(TypeError::Unknown) if t.name == "Self" => {
                diags.push(
                    Diagnostic::at(&codes::E0616, t.span, "`Self` only means something inside an interface")
                        .note("not inside an interface")
                        .help("write the struct's own name"),
                );
                None
            }
            Err(TypeError::Unknown) => {
                let mut d = Diagnostic::at(&codes::E0201, t.span, format!("unknown type `{}`", t.name));
                let structs: Vec<&str> = self.structs.iter().map(|s| s.name.as_str()).chain(self.enums.iter().map(|e| e.name.as_str())).collect();
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

    /// A type written with `<..>` arguments. Nothing generic exists as a type
    /// yet, so this explains why: E0602 for a type that takes none, E0901 for
    /// the library types that are still to come.
    fn type_args_error(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let at = t.span.to(t.args.last().map_or(t.span, |a| a.span));
        let plain = ast::TypeName::plain(t.name.clone(), t.span);
        let unit_arg = t.args.len() == 1 && self.resolve_quiet(&t.args[0]).is_none() && units::is_unit_name(self, &t.args[0].name);
        if unit_arg && matches!(self.resolve_base_quiet(&plain), Some(Type::Str | Type::Bool)) {
            // `string<m>`: a unit on something that is not a number.
            diags.push(
                Diagnostic::at(&codes::E0405, t.span, format!("`{}` cannot have a unit", t.name))
                    .note("units go only on number types")
                    .help("for example `float64<m>`"),
            );
        } else if matches!(t.name.as_str(), "Array" | "Iterable") {
            diags.push(Diagnostic::at(&codes::E0901, t.span, format!("`{}` is not supported yet", t.name)));
        } else if self.tenv.contains_key(&t.name) {
            diags.push(
                Diagnostic::at(&codes::E0602, at, format!("`{}` is a type parameter, so it takes no type arguments", t.name))
                    .note("type arguments written here"),
            );
        } else if self.resolve_base_quiet(&plain).is_some() {
            diags.push(
                Diagnostic::at(&codes::E0602, at, format!("`{}` takes no type arguments", t.name))
                    .note("type arguments written here")
                    .help(format!("`{}` is not generic: write just `{}`", t.name, t.name)),
            );
        } else {
            self.resolve(&plain, diags);
        }
        None
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

    /// Does a value of type `t` hold a borrow (so `lives` may name it)?
    fn holds_borrow(&self, t: Type) -> bool {
        self.holds_borrow_deep(t, 0)
    }

    fn holds_borrow_deep(&self, t: Type, depth: usize) -> bool {
        if depth > 8 {
            return false;
        }
        match t {
            Type::Ref { .. } => true,
            Type::Struct { id, .. } => self.structs[id].declared_lives || !self.structs[id].lives.is_empty(),
            Type::List { elem } | Type::Array { elem, .. } => self.holds_borrow_deep(*elem, depth + 1),
            Type::Map { key, val, .. } => self.holds_borrow_deep(*key, depth + 1) || self.holds_borrow_deep(*val, depth + 1),
            Type::Unit { of, .. } => self.holds_borrow_deep(*of, depth + 1),
            Type::Enum { id, .. } => self.enums[id].variants.iter().flat_map(|v| &v.fields).any(|f| f.ty.is_some_and(|t| self.holds_borrow_deep(t, depth + 1))),
            _ => false,
        }
    }

    fn enum_of(&self, t: Type) -> Option<&EnumInfo> {
        t.enum_id().map(|id| &self.enums[id])
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
        let sig = Sig { name: format!("{} ==", info.name), params: vec![Some(info.ty); 2], ret: Some(Type::Bool), span: info.name_span, fails: Vec::new(), outcome: None };
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

/// The units of `in si;`, by name (the resolver checks that a file importing
/// them has its own `in si;`).
pub fn si_unit_names() -> Vec<String> {
    units::si_unit_names()
}

/// Checks the whole program. The result is only fit for codegen when no
/// errors were reported.
pub fn check(prog: &ast::Program, src: &str, diags: &mut Vec<Diagnostic>) -> hir::Program {
    check_with_settings(prog, src, diags, Vec::new(), true)
}

pub(crate) fn check_with_settings(
    prog: &ast::Program,
    src: &str,
    diags: &mut Vec<Diagnostic>,
    settings: Vec<(usize, crate::config::Behaviour)>,
    requires_main: bool,
) -> hir::Program {
    let mut g = Globals { settings, unsafe_allowed: prog.unsafe_allowed, ..Globals::default() };
    let mut si = false;
    for import in &prog.imports {
        match import.plain_name() {
            Some("io") => g.io_imported = true,
            Some("fs") => g.fs_imported = true,
            Some("os") => g.os_imported = true,
            Some("time") => g.time_imported = true,
            Some("si") => si = true,
            // Project files are loaded by `modules`; checking names across
            // files is step 7's WP2.
            _ if import.path.len() > 1 || !import.items.is_empty() || import.glob || import.alias.is_some() => {
                diags.push(
                    Diagnostic::at(&codes::E0901, import.span, "this form of `in` is not supported yet")
                        .help("import standard modules whole, for example `in time;`"),
                );
            }
            _ => {
                diags.push(
                    Diagnostic::at(&codes::E0201, import.span, format!("unknown module `{}`", import.path.join(".")))
                        .help("standard modules include `io`, `si` and `time`"),
                );
            }
        }
    }

    // Units first: a type may name any unit, and a unit may name none.
    // `in si;` declares the SI prelude before the file's own units, so a
    // file that declares one of those names again is an error.
    if si {
        let mut prelude_diags = Vec::new();
        let tokens = lexer::lex(units::SI_PRELUDE, &mut prelude_diags);
        let prelude = parser::parse(tokens, &mut prelude_diags);
        debug_assert!(prelude_diags.is_empty(), "the SI prelude must parse");
        units::declare_units(&mut g, &prelude.units, &mut prelude_diags);
        debug_assert!(prelude_diags.is_empty(), "the SI prelude must check");
    }
    units::declare_units(&mut g, &prog.units, diags);
    constants::declare(&mut g, &prog.consts, diags);

    // Struct names first, so that any type can name any struct, wherever it is defined.
    generics::split_written(&mut g, prog);
    let struct_ids: Vec<Option<StructId>> = prog
        .structs
        .iter()
        .enumerate()
        .map(|(i, s)| if s.tparams.is_empty() { declare_struct(&mut g, s, diags) } else { generics::declare_struct_template(&mut g, i, s, diags) })
        .collect();
    let enum_ids: Vec<Option<hir::EnumId>> = prog
        .enums
        .iter()
        .enumerate()
        .map(|(i, e)| if e.tparams.is_empty() { enums::declare_enum(&mut g, e, diags) } else { generics::declare_enum_template(&mut g, i, e, diags) })
        .collect();
    // The failures every program has come first, so a program that declares
    // one of the same names is the one reported.
    let mut core_diags = Vec::new();
    let mut core_text = if g.fs_imported { [failures::PRELUDE, failures::FS_PRELUDE].concat() } else { failures::PRELUDE.into() };
    if prog.funcs.iter().any(|f| f.foreign.is_some()) {
        core_text += failures::C_PRELUDE;
    }
    let core = parser::parse(lexer::lex(&core_text, &mut core_diags), &mut core_diags);
    debug_assert!(core_diags.is_empty(), "the failure prelude must parse");
    let all_failures: Vec<&ast::Failure> = core.failures.iter().chain(&prog.failures).collect();
    let failure_ids: Vec<Option<hir::EnumId>> = all_failures.iter().map(|f| failures::declare_failure(&mut g, f, diags)).collect();
    g.fail.convert_error = g.enums_by_name.get("ConvertError").copied();
    interfaces::declare(&mut g, src, &prog.interfaces, diags);
    generics::validate_type_templates(&mut g, diags);
    // Every `T?` and every `Name<args>` the file writes outside a template.
    g.mono.written_optionals = prog.optionals.clone();
    g.mono.scope = g.mono.global.clone();
    generics::ensure_written(&mut g, diags);
    for (s, id) in prog.structs.iter().zip(&struct_ids) {
        if let Some(id) = *id {
            declare_fields(&mut g, id, s, diags);
        }
    }
    for (e, id) in prog.enums.iter().zip(&enum_ids) {
        if let Some(id) = *id {
            enums::declare_variants(&mut g, id, e, diags);
        }
    }
    for (f, id) in all_failures.iter().zip(&failure_ids) {
        if let Some(id) = *id {
            failures::declare_variants(&mut g, id, f, diags);
        }
    }
    check_struct_lives(&mut g, prog, &struct_ids, diags);
    report_cycles(&g, diags);
    interfaces::validate(&mut g, diags);

    // Signatures: functions, then methods.
    let mut func_ids = Vec::new();
    for (index, f) in prog.funcs.iter().enumerate() {
        if !f.tparams.is_empty() {
            generics::declare_template(&mut g, index, f, diags);
            func_ids.push(None);
            continue;
        }
        let params = f.params.iter().map(|p| g.resolve_binding_type(&p.ty, diags)).collect();
        let ret = g.resolve_return(&f.ret, diags);
        let (fails, outcome) = failures::signature(&mut g, f, ret, true, diags);
        let sig = Sig { name: f.name.clone(), params, ret, span: f.span, fails, outcome };
        check_signature_extras(&g, f, &sig.params, false, diags);
        if let Some(first) = g.funcs_by_name.get(&f.name).map(|&id| g.sigs[id].span).or_else(|| g.mono.templates.get(&f.name).map(|t| t.head.span)) {
            diags.push(
                Diagnostic::at(&codes::E0202, f.name_span, format!("function `{}` is defined twice", f.name))
                    .note("defined again here")
                    .label(first, "first defined here"),
            );
            func_ids.push(None);
            continue;
        }
        let id = g.add_sig(sig);
        g.funcs_by_name.insert(f.name.clone(), id);
        if f.is_unsafe.is_some() {
            g.lambdas.unsafe_named.insert(id);
        }
        if f.foreign.is_some() {
            g.foreign_funcs.insert(id);
        }
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

    maps::check_pending_keys(&g, diags);
    fill::declare_defs(&mut g, src, prog, diags);

    let entry = g.funcs_by_name.get("main").or_else(|| g.funcs_by_name.get(ast::SCRIPT_ENTRY)).copied();
    match entry.filter(|_| requires_main).map(|id| &g.sigs[id]) {
        None if requires_main => diags.push(
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

    let impl_targets = interfaces::declare_impls(&mut g, prog, diags);
    interfaces::finish(&mut g, prog, &struct_ids, &impl_targets, diags);
    generics::definition_pass(&mut g, src, prog, diags);

    // Bodies. Duplicates are still checked, but only the first definition is emitted.
    let mut funcs: Vec<Option<hir::Func>> = Vec::new();
    let emit = |funcs: &mut Vec<Option<hir::Func>>, id: FuncId, f: hir::Func| {
        funcs.resize_with(funcs.len().max(id + 1), || None);
        funcs[id] = Some(f);
    };
    for (f, id) in prog.funcs.iter().zip(&func_ids) {
        // A generic function is checked once per instance, below.
        if !f.tparams.is_empty() {
            continue;
        }
        let symbol = if f.name == ast::SCRIPT_ENTRY { "drachm_script_entry".into() } else { format!("drachm_f_{}", types::mangle(&f.name)) };
        let checked = check_body(&mut g, src, diags, f, None, symbol);
        if let Some(id) = *id {
            emit(&mut funcs, id, checked);
        }
    }
    for ((s, sid), mids) in prog.structs.iter().zip(&struct_ids).zip(&method_ids) {
        for (j, (m, mid)) in s.methods.iter().zip(mids).enumerate() {
            let (Some(sid), Some(mid)) = (*sid, *mid) else { continue };
            let symbol = method_symbol(&s.name, m, j, &s.methods);
            let ctx = MethodCtx {
                struct_id: sid,
                func: mid,
                static_span: m.is_static,
                ctor: m.ctor,
                no_fails: false,
                ext: None,
                trusted: true,
            };
            let checked = check_body(&mut g, src, diags, &m.func, Some(ctx), symbol);
            emit(&mut funcs, mid, checked);
            if m.is_static.is_none() && m.operator.is_none() && !m.ctor && !g.self_use[mid].used && m.func.name != "drop" && !g.ifaces.implementing.contains(&mid) {
                diags.push(never_uses_self(src, &s.name, m));
            }
        }
    }

    for (decl, target) in prog.impls.iter().zip(&impl_targets) {
        let Some(target) = target else { continue };
        for (j, (m, mid)) in decl.methods.iter().zip(&target.ids).enumerate() {
            let Some(mid) = *mid else { continue };
            let sname = target.ty.to_string();
            let symbol = method_symbol(&sname, m, 1000 + j, &decl.methods);
            let ctx = MethodCtx {
                struct_id: target.struct_id.unwrap_or(usize::MAX),
                func: mid,
                static_span: m.is_static,
                ctor: m.ctor,
                no_fails: false,
                ext: target.struct_id.is_none().then_some(target.ty),
                trusted: decl.trusted,
            };
            let checked = check_body(&mut g, src, diags, &m.func, Some(ctx), symbol);
            emit(&mut funcs, mid, checked);
            if m.is_static.is_none() && m.operator.is_none() && !m.ctor && !g.self_use[mid].used && !g.ifaces.implementing.contains(&mid) {
                diags.push(never_uses_self(src, &sname, m));
            }
        }
    }

    generics::drain(&mut g, src, prog, diags, &mut |id, f| emit(&mut funcs, id, f));
    generics::warn_unused(&g, diags);
    // Instances made while bodies were checked can hold themselves too.
    report_cycles(&g, diags);

    // A list function may ask for struct `==` and the other way round: take both until neither is left.
    loop {
        if let Some(s) = g.eq_pending.pop() {
            let f = auto_eq(&mut g, s);
            let id = g.eq_funcs[&s];
            emit(&mut funcs, id, f);
        } else if let Some((kind, list, id)) = g.list_pending.pop() {
            let f = lists::list_func_body(&mut g, kind, list, id);
            emit(&mut funcs, id, f);
        } else if let Some((kind, ty, id)) = g.addr_pending.pop() {
            let f = addresses::addr_func_body(&mut g, kind, ty, id);
            emit(&mut funcs, id, f);
        } else if let Some((callback, map, id)) = g.keys.each_pending.pop() {
            let f = maps::map_each_body(&g, callback, map, id);
            emit(&mut funcs, id, f);
        } else {
            break;
        }
    }

    for (id, f) in std::mem::take(&mut g.lambdas.bodies) {
        emit(&mut funcs, id, f);
    }
    for (id, f) in failures::finish(&mut g) {
        emit(&mut funcs, id, f);
    }
    for (wrapper, drop, ty) in std::mem::take(&mut g.fail.drop_wrappers) {
        let f = failures::drop_wrapper(&g, wrapper, drop, ty);
        emit(&mut funcs, wrapper, f);
    }
    let mut main = entry.unwrap_or(0);
    if requires_main && let Some((id, f)) = failures::main_wrapper(&mut g, main) {
        emit(&mut funcs, id, f);
        main = id;
    }
    // `copy` depends on which structs have a `drop()`; the queries answer for
    // every type that exists now, including those made while bodies were checked.
    g.compute_ties();
    let (structs, enum_defs) = layout::lay_out(&g);
    let mut collections: Vec<Type> = Vec::new();
    for t in g.collections() {
        if !collections.contains(&t.bare()) {
            collections.push(t.bare());
        }
    }
    let conformances = std::mem::take(&mut g.ifaces.records);
    let (dyn_ifaces, vtables) = dynamic::finish(&mut g, &structs);
    let foreign: std::collections::BTreeMap<FuncId, String> = prog
        .funcs
        .iter()
        .zip(&func_ids)
        .filter_map(|(f, id)| Some(((*id)?, f.foreign.clone()?)))
        .collect();
    index::finish(&g, src);
    let mut program = hir::Program { settings: Vec::new(), funcs: funcs.into_iter().map(|f| f.unwrap_or_else(placeholder)).collect(), structs, enums: enum_defs, main, collections, conformances, dyn_ifaces, vtables, closures: std::mem::take(&mut g.lambdas.closures), ties: std::mem::take(&mut g.ties), foreign, link: Default::default() };
    program.settings = program.funcs.iter().map(|f| g.settings_at(f.name_span)).collect();
    if !diags.iter().any(Diagnostic::is_error) {
        lambdas::infer_captures(&mut program, diags);
    }
    units::erase(&mut program);
    if cfg!(debug_assertions) {
        units::assert_erased(&mut program);
    }
    program
}

/// The symbol of method `j` of struct `sname`: `drachm_m_6Player_nameOf`.
/// A name with `<`, `,` and so on (an instance) is mangled first.
fn method_symbol(sname: &str, m: &ast::Method, j: usize, siblings: &[ast::Method]) -> String {
    let mut word = if m.ctor { format!("new{j}") } else { m.operator.map_or(m.func.name.clone(), |op| format!("op_{}", op_word(op))) };
    // Methods that differ only in their result share a name.
    if !m.ctor && m.operator.is_none() && siblings.iter().take(j % 1000).any(|o| !o.ctor && o.operator.is_none() && o.func.name == m.func.name) {
        word = format!("{word}_{}", j % 1000);
    }
    let sname = types::mangle(sname);
    format!("drachm_m_{}{}_{word}", sname.len(), sname)
}

/// W0208: a method that never uses `self` of the struct called `sname`.
fn never_uses_self(src: &str, sname: &str, m: &ast::Method) -> Diagnostic {
    Diagnostic::at(&codes::W0208, m.func.name_span, format!("method `{}` never uses `self`", m.func.name))
        .note("does not use `self`")
        .help(format!(
            "mark it `static` and call it on the struct, `{}::{}()`: write `static {}`",
            sname,
            m.func.name,
            short(src, m.func.span)
        ))
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
        lives_span: None,
    }
}

/// The source text of `span`, for quoting a declaration in a fix-it.
fn short(src: &str, span: Span) -> &str {
    &src[span.start.min(src.len())..span.end.min(src.len())]
}

/// A declared type as the user wrote it, unit and `?` included (`&float64<m>`).
fn written_type(src: &str, ty: &ast::TypeName) -> String {
    let start = ty.mods.first().map_or(ty.span.start, |(_, s)| s.start.min(ty.span.start));
    let mut end = ty.span.end;
    if ty.unit.is_some()
        && let Some(i) = src.get(end..).and_then(|rest| rest.find('>'))
    {
        end += i + 1;
    }
    if let Some(q) = ty.optional {
        end = end.max(q.end);
    }
    short(src, Span::new(start, end)).to_string()
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
        BinOp::Shl | BinOp::Shr | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::And | BinOp::Or | BinOp::Same | BinOp::NotSame => {
            unreachable!("not an operator method")
        }
    }
}

/// Types the compiler writes itself for a C declaration carry an empty span.
fn is_synthetic(span: Span) -> bool {
    span.start == span.end
}

fn unsafe_disabled(span: Span, what: &str) -> Diagnostic {
    Diagnostic::at(&codes::E0317, span, "`unsafe` is not enabled in this file")
        .note(what)
        .help("add `//--!$unsafe: true` as the first line of the file")
}

/// An `unsafe` function needs the file flag; `lives` names parameters that
/// hold a borrow.
fn check_signature_extras(g: &Globals, f: &ast::Func, params: &[Option<Type>], is_method: bool, diags: &mut Vec<Diagnostic>) {
    if let Some(at) = f.is_unsafe
        && !g.unsafe_allowed
        && f.foreign.is_none()
    {
        diags.push(unsafe_disabled(at, "needs the file flag"));
    }
    for (name, span) in &f.lives {
        if name == "self" {
            if !is_method {
                diags.push(
                    Diagnostic::at(&codes::E0309, *span, "`lives` names `self`, but this is not a method")
                        .note("only a method has `self`")
                        .help("name a parameter holding a borrow instead"),
                );
            } else if !f.lives_from {
                diags.push(
                    Diagnostic::at(&codes::E0309, *span, "`self` can only be named after `lives from`")
                        .note("`lives a, b` lists parameters")
                        .help("write `lives from self or ...`, with the sources joined by `or`"),
                );
            }
            continue;
        }
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
    for (s, id) in prog.structs.iter().zip(ids) {
        if let Some(id) = *id {
            check_one_struct_lives(g, id, s, diags);
        }
    }
}

/// `check_struct_lives` for one struct (or one instance of a generic struct).
fn check_one_struct_lives(g: &mut Globals, id: StructId, s: &ast::Struct, diags: &mut Vec<Diagnostic>) {
    let sname = g.structs[id].name.clone();
    let fields = &g.structs[id].fields;
    // A generic struct made with a borrow type argument holds borrows in fields
    // written `T`: those are not written as borrows, so `lives` need not name them.
    let written_borrow = |i: usize| -> bool {
        g.tenv.is_empty() || s.fields.iter().find(|f| f.name == fields[i].name).is_none_or(|f| f.ty.mods.iter().any(|(m, _)| matches!(m, ast::TypeMod::Ref { .. })))
    };
    let borrows: Vec<usize> = (0..fields.len()).filter(|&i| fields[i].ty.is_some_and(Type::is_borrow) && written_borrow(i)).collect();
    let held: Vec<usize> = (0..fields.len()).filter(|&i| !borrows.contains(&i) && fields[i].ty.is_some_and(|t| g.holds_borrow(t))).collect();
    let mut named = Vec::new();
    for (name, span) in &s.lives {
        match fields.iter().position(|f| f.name == *name).filter(|i| borrows.contains(i)) {
            Some(i) => named.push(i),
            None => diags.push(
                Diagnostic::at(&codes::E0314, *span, format!("`lives {name}` names nothing `{sname}` borrows"))
                    .note(format!("`{name}` is not a borrow field"))
                    .help("`lives` lists the fields of the struct that are borrows"),
            ),
        }
    }
    let names: Vec<&str> = borrows.iter().map(|&i| fields[i].name.as_str()).collect();
    if s.lives.is_empty() {
        if let Some(&first) = borrows.first() {
            diags.push(
                Diagnostic::at(&codes::E0314, s.name_span, format!("`{sname}` holds a borrow, so it must say what it borrows from"))
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
        if inner.and_then(Type::struct_id).is_some_and(|o| g.structs[o].declared_lives) {
            diags.push(Diagnostic::at(&codes::E0901, f.span, "a field that borrows a struct with `lives` is not supported yet"));
        }
    }
    if g.tenv.is_empty() {
        // Only an instance of a generic struct may hold a borrow inside a container or optional.
        for &i in &held {
            let f = &fields[i];
            if f.ty.is_some_and(|t| t.struct_id().is_none_or(|o| !g.structs[o].declared_lives)) {
                diags.push(
                    Diagnostic::at(&codes::E0901, f.span, "a field that holds borrows inside a container is not supported yet")
                        .note("this field would hold a borrow")
                        .help("hold the borrow in its own field: `&T`, named by `lives`"),
                );
            }
        }
    } else {
        named.extend(held.iter().copied());
    }
    named.sort_unstable();
    named.dedup();
    g.structs[id].lives = named;
}

fn declare_struct(g: &mut Globals, s: &ast::Struct, diags: &mut Vec<Diagnostic>) -> Option<StructId> {
    if s.name == "none" || maps::is_map_name(&s.name) || !matches!(Type::from_name(&s.name), Err(TypeError::Unknown)) {
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
    if generics::type_name_taken(g, &s.name, s.name_span, "struct", diags) {
        return None;
    }
    if let Some(&first) = g.enums_by_name.get(&s.name) {
        diags.push(
            Diagnostic::at(&codes::E0202, s.name_span, format!("`{}` is both a struct and an enum", s.name))
                .note("defined again here")
                .label(g.enums[first].name_span, "first defined here"),
        );
        return None;
    }
    let id = new_struct_info(g, &s.name, s);
    g.structs_by_name.insert(s.name.clone(), id);
    Some(id)
}

/// Adds a struct called `name` (its display name) with nothing declared yet.
fn new_struct_info(g: &mut Globals, name: &str, s: &ast::Struct) -> StructId {
    let id = g.structs.len();
    let ty = Type::Struct { id, name: types::intern(name) };
    g.structs.push(StructInfo {
        name: name.to_string(),
        name_span: s.name_span,
        ty,
        fields: Vec::new(),
        methods: Vec::new(),
        stays: s.stays.is_some(),
        lives: Vec::new(),
        has_drop: None,
        ctors: Vec::new(),
        auto_ctor: s.auto_ctor.is_some(),
        default_def: None,
        declared_lives: !s.lives.is_empty(),
        placeholder: None,
    });
    id
}

fn declare_fields(g: &mut Globals, id: StructId, s: &ast::Struct, diags: &mut Vec<Diagnostic>) {
    for f in &s.fields {
        let ty = g.resolve_storable(&f.ty, diags);
        if let Some(first) = g.structs[id].fields.iter().find(|o| o.name == f.name) {
            diags.push(
                Diagnostic::at(&codes::E0202, f.name_span, format!("`{}` has two fields named `{}`", g.structs[id].name, f.name))
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
    declare_method_list(g, id, &s.methods, diags)
}

/// Signatures of `methods`, which belong to struct `id` (its own, or added by `T with I { }`).
fn declare_method_list(g: &mut Globals, id: StructId, methods: &[ast::Method], diags: &mut Vec<Diagnostic>) -> Vec<Option<FuncId>> {
    let self_ty = g.structs[id].ty;
    let sname = g.structs[id].name.clone();
    let mut ids = Vec::new();
    for m in methods {
        if m.ctor {
            ids.push(construct::declare_ctor(g, id, m, diags));
            continue;
        }
        let f = &m.func;
        let params = f.params.iter().map(|p| g.resolve_binding_type(&p.ty, diags)).collect();
        let ret = g.resolve_return(&f.ret, diags);
        let (fails, outcome) = failures::signature(g, f, ret, true, diags);
        let sig = Sig { name: format!("{sname}::{}", f.name), params, ret, span: f.span, fails, outcome };
        check_signature_extras(g, f, &sig.params, m.is_static.is_none(), diags);
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
            && !operator_shape_ok(op, m, &sig, self_ty, &sname, diags)
        {
            ids.push(None);
            continue;
        }
        let info = &g.structs[id];
        // Two methods may share a name and parameters when only their results differ:
        // a type that is `Iterable` over two item types has two `at`. Calling one by
        // name is then ambiguous (E0632); an interface picks its own by result.
        let by_result = |o: &MethodInfo| {
            m.operator.is_none() && o.operator.is_none() && m.is_static.is_none() && !o.is_static && g.sigs[o.func].params == sig.params && g.sigs[o.func].ret != sig.ret
        };
        let clash = if let Some(first) = info.methods.iter().find(|o| o.name == f.name && !by_result(o)) {
            let what = if m.operator.is_some() { format!("`{}` operators", f.name) } else { format!("methods named `{}`", f.name) };
            Some((format!("`{sname}` has two {what}"), g.sigs[first.func].span, "first defined here"))
        } else {
            // Undecided by the SPEC, chosen here: a field and a method may not share a name.
            info.fields.iter().find(|o| o.name == f.name).map(|field| {
                (format!("`{sname}` has a field and a method both named `{}`", f.name), field.span, "the field")
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
            // A `drop()` that fails is run through a function that reports the failure.
            let wrapper = if g.sigs[func].outcome.is_some() {
                let sig = Sig { name: format!("{sname}::drop"), params: Vec::new(), ret: Some(Type::None), span: f.span, fails: Vec::new(), outcome: None };
                let w = g.add_sig(sig);
                g.fail.drop_wrappers.push((w, func, self_ty));
                w
            } else {
                func
            };
            g.structs[id].has_drop = Some(wrapper);
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

/// A struct or enum that contains itself (directly or through others) would
/// be infinitely large. Reports each cycle once; returns which types are in
/// one: the structs by id, then the enums (`structs.len() + id`).
fn report_cycles(g: &Globals, diags: &mut Vec<Diagnostic>) {
    let ns = g.structs.len();
    let n = ns + g.enums.len();
    let mut cyclic: Vec<bool> = (0..n).map(|i| g.was_cyclic(i)).collect();
    let mut done = vec![false; n];
    /// One field (or payload value) that holds another type by value.
    struct Edge {
        field: String,
        ty: Type,
        span: Span,
        to: usize,
    }
    fn node_name(g: &Globals, node: usize) -> &str {
        let ns = g.structs.len();
        if node < ns { &g.structs[node].name } else { &g.enums[node - ns].name }
    }
    fn edges(g: &Globals, node: usize) -> Vec<Edge> {
        let ns = g.structs.len();
        let mut out = Vec::new();
        let mut add = |f: &FieldInfo, label: String| {
            // An array holds its items by value; a list holds them on the heap.
            let mut held = f.ty;
            while let Some(Type::Array { elem, .. }) = held {
                held = Some(*elem);
            }
            let to = match held {
                Some(Type::Struct { id, .. }) => id,
                Some(Type::Enum { id, .. }) => ns + id,
                _ => return,
            };
            out.push(Edge { field: label, ty: f.ty.unwrap(), span: f.span, to });
        };
        if node < ns {
            for f in &g.structs[node].fields {
                add(f, f.name.clone());
            }
        } else {
            for v in &g.enums[node - ns].variants {
                for f in &v.fields {
                    add(f, f.name.clone());
                }
            }
        }
        out
    }
    // Depth-first, with the path from the start as (type, edge) pairs.
    fn visit(g: &Globals, s: usize, path: &mut Vec<(usize, usize)>, done: &mut [bool], cyclic: &mut [bool], diags: &mut Vec<Diagnostic>) {
        if done[s] {
            return;
        }
        if let Some(start) = path.iter().position(|&(p, _)| p == s) {
            let cycle = &path[start..];
            if cycle.iter().any(|&(p, _)| cyclic[p]) {
                return;
            }
            let ns = g.structs.len();
            let name = node_name(g, s);
            let with_enum = cycle.iter().any(|&(p, _)| p >= ns);
            let (first, first_edge) = cycle[0];
            let e = &edges(g, first)[first_edge];
            let title = if with_enum { format!("`{name}` contains itself") } else { format!("struct `{name}` contains itself") };
            let mut d = Diagnostic::at(&codes::E0210, e.span, title).note(format!("`{}` has type `{}`", e.field, e.ty));
            for &(p, i) in &cycle[1..] {
                let e = &edges(g, p)[i];
                d = d.label(e.span, format!("`{}` has type `{}`", e.field, e.ty));
            }
            let holds: Vec<String> = cycle
                .iter()
                .map(|&(p, i)| format!("`{}` holds `{}`", node_name(g, p), edges(g, p)[i].ty))
                .collect();
            d = d.note_line(format!("{}: its size would never end", holds.join(", ")));
            d = if with_enum {
                d.help("a variant cannot hold its own enum by value; boxes come with generics")
            } else {
                d.help("a struct cannot hold a value of its own type, directly or through other structs")
            };
            diags.push(d);
            for &(p, _) in cycle {
                cyclic[p] = true;
            }
            return;
        }
        for (i, e) in edges(g, s).iter().enumerate() {
            path.push((s, i));
            visit(g, e.to, path, done, cyclic, diags);
            path.pop();
        }
        done[s] = true;
    }
    for s in 0..n {
        visit(g, s, &mut Vec::new(), &mut done, &mut cyclic, diags);
    }
    g.mark_cyclic(&cyclic);
}

/// The automatic `==` of struct `s`: every field equal, in order, with
/// strings compared by their text and struct fields by their own `==`.
/// It works like a method: `a` is the receiver, borrowed, and `b` is owned
/// (and dropped).
fn auto_eq(g: &mut Globals, s: StructId) -> hir::Func {
    let info = &g.structs[s];
    let (ty, name) = (info.ty, info.name.clone());
    let fields: Vec<Type> = info.fields.iter().map(|f| f.ty.unwrap_or(Type::Bool)).collect();
    let side = |local, i, fty| {
        hir::Expr::new(hir::ExprKind::Field(Box::new(hir::Expr::new(hir::ExprKind::Local(local), ty)), i), fty)
    };
    let mut all: Option<hir::Expr> = None;
    for (i, fty) in fields.into_iter().enumerate() {
        let (a, b) = (side(0, i, fty), side(1, i, fty));
        let same = match fty.struct_id() {
            Some(inner) => g.struct_eq(inner, a, b),
            None if fty == Type::Str => hir::Expr::new(hir::ExprKind::StrCall(hir::StrFn::Eq, vec![a, b]), Type::Bool),
            None => hir::Expr::new(hir::ExprKind::Binary(hir::BinOp::Eq, Box::new(a), Box::new(b)), Type::Bool),
        };
        all = Some(match all {
            None => same,
            Some(prev) => hir::Expr::new(hir::ExprKind::Logic { and: true, lhs: Box::new(prev), rhs: Box::new(same) }, Type::Bool),
        });
    }
    let result = all.unwrap_or(hir::Expr::new(hir::ExprKind::Bool(true), Type::Bool));
    let none = Span::new(0, 0);
    let param = |n: &str| hir::Local { name: n.into(), ty, span: none, mutability: hir::Mutability::Fixed, is_param: true, written: None };
    hir::Func {
        symbol: format!("drachm_eq_{}", types::mangle(&name)),
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
        lives_span: None,
    }
}

/// The method whose body is being checked.
#[derive(Clone, Copy)]
struct MethodCtx {
    struct_id: StructId,
    func: FuncId,
    /// The `static` keyword, if the method has one.
    static_span: Option<Span>,
    /// A constructor: `self` starts with no value and is returned at the end.
    ctor: bool,
    /// Constructors, operators and `drop` may not say `fails` yet.
    no_fails: bool,
    /// A method added to a built-in type (`uint32 with Zero { }`): the type of `self`.
    ext: Option<Type>,
    /// May use the type's private members (false in a plain `TYPE with I { }` block).
    trusted: bool,
}

fn check_body(g: &mut Globals, src: &str, diags: &mut Vec<Diagnostic>, f: &ast::Func, method: Option<MethodCtx>, symbol: String) -> hir::Func {
    let ret = g.resolve_return(&f.ret, &mut Vec::new());
    let (fails, outcome) = failures::signature(g, f, ret, !method.is_some_and(|m| m.no_fails), &mut Vec::new());
    let mark = index::mark();
    let mut cx = Checker::new(g, src, diags, &symbol, ret, method);
    cx.script_entry = f.name == ast::SCRIPT_ENTRY;
    cx.fails = fails;
    cx.outcome = outcome;
    let has_self = method.is_some_and(|m| m.static_span.is_none() && !m.ctor);
    if let Some(m) = method.filter(|_| has_self) {
        let ty = m.ext.unwrap_or_else(|| cx.g.structs[m.struct_id].ty);
        let id = cx.declare("self", Some(ty), false, f.name_span, true);
        cx.locals[id].is_self = true;
    }
    for p in &f.params {
        let ty = cx.g.resolve_quiet(&p.ty).filter(|t| *t != Type::None);
        let id = cx.declare(&p.name, ty, p.mutable, p.span, true);
        cx.locals[id].open = p.open;
        cx.locals[id].written = Some(written_type(src, &p.ty));
        if p.mutable && p.open {
            let at = short(src, p.span).find("open").map_or(p.span, |i| Span::new(p.span.start + i, p.span.start + i + 4));
            cx.diags.push(
                Diagnostic::at(&codes::W0323, at, "`mut open` says the same as `mut`")
                    .note("`mut` already allows everything `open` does")
                    .help(format!("write `mut {} {}`", short(src, p.ty.span), p.name)),
            );
        }
    }
    // A constructor's `self` comes after the parameters, with no value yet; the
    // body fills it and the function returns it.
    let ctor_self = method.filter(|m| m.ctor).map(|m| {
        let ty = cx.g.structs[m.struct_id].ty;
        let id = cx.declare("self", Some(ty), true, f.name_span, false);
        cx.locals[id].is_self = true;
        cx.ctor_self = Some(id);
        cx.filling.push(id);
        (id, ty)
    });
    // A C function has no body: the symbol is all there is.
    let mut body = if f.foreign.is_some() { Vec::new() } else { cx.block(&f.body) };
    index::close(mark, f.body.end.end);
    // A constructor that can fail leaves `self` half built on a `fail`: it owns what it has.
    if let Some((id, _)) = ctor_self
        && cx.outcome.is_some()
    {
        body.insert(0, hir::Stmt::Decl(id));
    }
    // A failing function that returns nothing succeeds when it reaches its end.
    if cx.outcome.is_some() && cx.ret == Some(Type::None) {
        body.push(hir::Stmt::Return(cx.wrap_ok(None)));
    }
    if let Some((id, ty)) = ctor_self {
        body.push(hir::Stmt::Check { local: id, kind: hir::CheckKind::CtorEnd, span: f.body.end });
        body.push(hir::Stmt::Return(cx.wrap_ok(Some(hir::Expr::new(hir::ExprKind::Local(id), ty)))));
    }
    let locals = cx.lower_locals();
    hir::Func {
        name_span: f.name_span,
        span: f.span,
        body_end: f.body.end,
        is_unsafe: f.is_unsafe.is_some(),
        // Parameter indexes with `self` as 0 (names that are no parameter were reported as E0309).
        lives: (!f.lives.is_empty()).then(|| {
            let mut v: Vec<usize> = f
                .lives
                .iter()
                .filter_map(|(n, _)| {
                    if n == "self" {
                        has_self.then_some(0)
                    } else {
                        f.params.iter().position(|p| p.name == *n).map(|i| i + has_self as usize)
                    }
                })
                .collect();
            v.sort();
            v.dedup();
            v
        }),
        lives_span: f.lives_span,
        symbol,
        params: f.params.len() + has_self as usize,
        has_self,
        locals,
        ret: outcome.or(ret).unwrap_or(Type::None),
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
    /// `none` among constructor values: it fits an optional or pointer
    /// parameter, which only the chosen constructor says (see `construct`).
    NoneLit(Span),
    Const(Const),
    /// An untyped constant with a unit (`9.6<s>`): it gets its number type
    /// when it meets one (`typeck::units`).
    UConst(Const, &'static crate::units::Unit),
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

#[derive(Clone)]
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
    /// The declared type as written, for help text.
    written: Option<String>,
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
    /// Some step on the path reads through a `&` borrow or a `*fixed` pointer:
    /// the place cannot be changed whatever its binding says.
    through_shared: bool,
    /// Some step on the path is a `*fixed` pointer: writing to the place is E0333.
    through_fixed: bool,
}

struct Checker<'a> {
    src: &'a str,
    g: &'a mut Globals,
    diags: &'a mut Vec<Diagnostic>,
    symbol: &'a str,
    /// The generated body of a script's top-level statements.
    script_entry: bool,
    /// Present when checking a lambda whose result comes from its body.
    lambda: Option<lambdas::Frame>,
    /// `None` if the return type did not resolve.
    ret: Option<Type>,
    method: Option<MethodCtx>,
    scopes: Vec<HashMap<String, LocalId>>,
    locals: Vec<LocalInfo>,
    /// The loops and `if`/`match` levels around the current statement, innermost last.
    targets: Vec<Target>,
    /// Labels handed out so far in this function.
    labels: hir::Label,
    /// Locals that do not have all their fields yet: a constructor's `self`,
    /// and a value inside its `fill` block.
    filling: Vec<LocalId>,
    /// `copy xs::first()` is being checked: an item that owns memory is copied out.
    copying_item: bool,
    /// Inside `now e`, where reading a field already set is allowed.
    now_depth: u32,
    /// The `self` of the constructor being checked.
    ctor_self: Option<LocalId>,
    /// The failure types this function lists after `fails`.
    fails: Vec<hir::EnumId>,
    /// What this function returns: an outcome, when it fails.
    outcome: Option<Type>,
    /// The call a `maybe` or `fails` is being checked for (an `ast::Expr` address).
    suffix_ok: Option<usize>,
    /// The failure handlers around the statement being checked, innermost last.
    handlers: Vec<failures::HandlerCtx>,
}

/// How the hover shows a local: `mut uint32 count`.
fn local_code(l: &index::LocalRec) -> String {
    format!("{}{} {}", if l.mutable { "mut " } else { "" }, l.ty, l.name)
}

fn int_lit(bits: u128, ty: Type) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Int(bits), ty)
}

fn str_lit(text: String) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Str(text), Type::Str)
}

/// Does the place expression go through a `Deref`?
fn has_deref(e: &hir::Expr) -> bool {
    match &e.kind {
        hir::ExprKind::Deref(_) | hir::ExprKind::Index(..) => true,
        hir::ExprKind::Field(inner, _) => has_deref(inner),
        _ => false,
    }
}

/// Is the written expression a place: a variable, `self`, a field of a
/// place, or a dereference?
fn ast_place(e: &ast::Expr) -> bool {
    match &e.kind {
        ExprKind::Var(_) | ExprKind::SelfValue => true,
        ExprKind::Field { recv, .. } | ExprKind::Index { recv, .. } => ast_place(recv),
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
        hir::ExprKind::Index(inner, idx) => plain_place(inner) && matches!(idx.kind, hir::ExprKind::Int(_) | hir::ExprKind::Local(_)),
        _ => false,
    }
}

impl<'a> Checker<'a> {
    fn new(g: &'a mut Globals, src: &'a str, diags: &'a mut Vec<Diagnostic>, symbol: &'a str, ret: Option<Type>, method: Option<MethodCtx>) -> Self {
        Self {
            src,
            g,
            diags,
            symbol,
            script_entry: false,
            lambda: None,
            ret,
            fails: Vec::new(),
            outcome: None,
            suffix_ok: None,
            handlers: Vec::new(),
            method,
            scopes: vec![HashMap::new()],
            locals: Vec::new(),
            targets: Vec::new(),
            labels: 0,
            filling: Vec::new(),
            copying_item: false,
            now_depth: 0,
            ctor_self: None,
        }
    }

    fn lower_locals(&mut self) -> Vec<hir::Local> {
        std::mem::take(&mut self.locals)
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
                written: l.written,
            })
            .collect()
    }

    fn lookup(&self, name: &str) -> Option<LocalId> {
        let id = self.scopes.iter().rev().find_map(|s| s.get(name).copied())?;
        self.use_capture(id);
        Some(id)
    }

    fn declare(&mut self, name: &str, ty: Option<Type>, mutable: bool, span: Span, is_param: bool) -> LocalId {
        let id = self.locals.len();
        let info = LocalInfo { name: name.to_string(), ty, mutable, is_param, is_self: false, open: false, span, written: None };
        self.locals.push(info);
        self.scopes.last_mut().unwrap().insert(name.to_string(), id);
        self.note_declared(id);
        id
    }

    /// Tell the language server about the local `id` (see `index`).
    pub(super) fn note_declared(&self, id: LocalId) {
        let l = &self.locals[id];
        index::with(|ix| {
            let ty = l.ty.map_or("?".to_string(), |t| index::demangle(&t.to_string()));
            let rec = index::LocalRec { name: l.name.clone(), ty, decl: l.span, mutable: l.mutable, is_param: l.is_param, end: usize::MAX };
            if l.name != "self" {
                ix.refs.push(index::Ref { at: l.span, def: Some(l.span), code: local_code(&rec), note: String::new() });
            }
            ix.locals.push(rec);
        });
    }

    /// Tell the language server that a call at `span` names the function `name`.
    pub(super) fn note_func(&self, name: &str, span: Span) {
        let sig = self.g.funcs_by_name.get(name).map(|&id| self.g.sigs[id].span).or_else(|| self.g.mono.templates.get(name).map(|t| t.head.span));
        let Some(sig) = sig else { return };
        let at = Span::new(span.start, span.start + index::written_len(self.src, span.start));
        let code = short(self.src, sig).to_string();
        index::with(|ix| ix.refs.push(index::Ref { at, def: Some(index::name_in(self.src, sig, name)), code, note: String::new() }));
    }

    /// Tell the language server that `at` names method `m` of struct `s`.
    pub(super) fn note_method(&self, s: StructId, m: &MethodInfo, at: Span) {
        let sig = self.g.sigs[m.func].span;
        let code = short(self.src, sig).to_string();
        let note = format!("method of `{}`", index::demangle(&self.g.structs[s].name));
        index::with(|ix| ix.refs.push(index::Ref { at, def: Some(index::name_in(self.src, sig, &m.name)), code, note }));
    }

    /// Tell the language server that a value of type `ty` is wanted at `at`.
    pub(super) fn note_expected(&self, at: Span, ty: Type) {
        if ty.enum_id().is_some() {
            index::with(|ix| ix.expected.push((at, index::demangle(&ty.to_string()))));
        }
    }

    /// Tell the language server that `at` names the local `id`.
    pub(super) fn note_use(&self, id: LocalId, at: Span) {
        let l = &self.locals[id];
        if l.span == Span::new(0, 0) {
            return;
        }
        index::with(|ix| {
            let ty = l.ty.map_or("?".to_string(), |t| index::demangle(&t.to_string()));
            let rec = index::LocalRec { name: l.name.clone(), ty: ty.clone(), decl: l.span, mutable: l.mutable, is_param: l.is_param, end: 0 };
            ix.types.push((at, ty));
            ix.refs.push(index::Ref { at, def: Some(l.span), code: local_code(&rec), note: String::new() });
        });
    }

    /// A local the program cannot name, for a value the lowering needs to keep;
    /// the name says what it holds (for reading the HIR).
    fn hidden_named(&mut self, name: &str, ty: Type) -> LocalId {
        let id = self.locals.len();
        let span = Span::new(0, 0);
        let info = LocalInfo { name: name.to_string(), ty: Some(ty), mutable: true, is_param: false, is_self: false, open: false, span, written: None };
        self.locals.push(info);
        id
    }

    /// A condition (`if`, `while`, `for`): a `bool`. Its temporaries die as
    /// soon as it has been tested (the MIR builder sees to that).
    fn cond(&mut self, e: &ast::Expr) -> Option<hir::Expr> {
        self.coerce(e, Type::Bool)
    }

    /// The by-value operand of an operator (`==`, an operator method): the
    /// callee gets a copy and the operand stays where it is.
    fn copied(&self, e: hir::Expr) -> hir::Expr {
        if e.is_place() && matches!(e.ty, Type::Str | Type::Struct { .. } | Type::Enum { .. }) && !self.copies_anyway(e.ty) {
            let ty = e.ty;
            return hir::Expr::new(hir::ExprKind::DeepCopy(Box::new(e)), ty);
        }
        e
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
        let mark = index::mark();
        let mut out = Vec::new();
        for s in b {
            self.stmt(s, &mut out);
        }
        self.scopes.pop();
        index::close(mark, b.end.end);
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
                let written = written_type(self.src, ty);
                let ty = self.g.resolve_binding_type(ty, self.diags);
                // The value is checked before the name exists, so `uint32 x = x;`
                // refers to an outer `x`.
                let init = value.as_ref().map(|v| self.init(v, ty));
                let id = self.declare(name, ty, *mutable, *name_span, false);
                self.locals[id].open = *open;
                self.locals[id].written = Some(written);
                out.push(hir::Stmt::Decl(id));
                if let Some(init) = init {
                    if let Some(e) = init {
                        out.push(hir::Stmt::Assign(hir::Place::local(id), e));
                    }
                }
                // A list declared without a value starts empty and is usable at once (Q20).
                if value.is_none() && let Some(t) = ty && t.is_list() {
                    out.push(hir::Stmt::Assign(hir::Place::local(id), hir::Expr::new(hir::ExprKind::List(Vec::new()), t).at(*span)));
                }
                // A map declared without a value starts empty too (Q20).
                if value.is_none() && let Some(t) = ty && t.is_map() {
                    out.push(hir::Stmt::Assign(hir::Place::local(id), hir::Expr::new(hir::ExprKind::Map(Vec::new()), t).at(*span)));
                }
                // `uint32? x;` starts as `none`.
                if value.is_none() && let Some(t) = ty && optionals::is_optional(self.g, t) {
                    out.push(hir::Stmt::Assign(hir::Place::local(id), optionals::none_value(t, *span)));
                }
            }
            Stmt::Assign { target, op, value, span } => match &target.kind {
                ExprKind::Var(name) => self.assign_local(name, target.span, *op, value, *span, out),
                ExprKind::SelfValue if self.ctor_self.is_some() => {
                    // Delegation (one constructor building through another) is not supported yet.
                    self.diags.push(
                        Diagnostic::at(&codes::E0901, *span, "a constructor cannot give `self` a value from another constructor yet")
                            .help("write the fields one by one: `self.x = ...;`"),
                    );
                    self.expr(value);
                }
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
                if let Some(init) = init {
                    self.stmt(init, out);
                }
                let c = cond.as_ref().map(|c| self.cond(c));
                let label = self.new_label();
                let end = body.end;
                let (body, step) = self.looped(label, *span, body, step.as_deref());
                self.scopes.pop();
                match c {
                    Some(None) => {}
                    c => out.push(hir::Stmt::Loop { label, cond: c.flatten(), body, step, end }),
                }
            }
            Stmt::ForEach { .. } => self.for_each(s, out),
            Stmt::ForEntries { key, value, iter, body, span } => self.for_entries(key, value, iter, body, *span, out),
            Stmt::Match { value, cases, span } => self.match_stmt(value, cases, *span, out),
            Stmt::Break { cont, count, span } => {
                if let Some(j) = self.break_target(*cont, *count, *span) {
                    out.push(hir::Stmt::Jump(j));
                }
            }
            Stmt::Skip { target, span } => {
                if let Some(j) = self.skip_target(*target, *span) {
                    out.push(hir::Stmt::Jump(j));
                }
            }
            Stmt::Return { span, .. } | Stmt::Fail { span, .. } if self.script_entry => {
                self.diags.push(Diagnostic::at(&codes::E0901, *span, "returning or propagating a failure from script statements is not supported yet"));
            }
            Stmt::Return { value, span } if self.ctor_self.is_some() => {
                let id = self.ctor_self.unwrap();
                if let Some(v) = value {
                    self.expr(v);
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, v.span, "a constructor gives the value itself")
                            .note("returned here")
                            .help("write `return;`: the constructor returns the value it has built when it ends"),
                    );
                }
                let ty = self.locals[id].ty.unwrap_or(Type::None);
                out.push(hir::Stmt::Check { local: id, kind: hir::CheckKind::CtorEnd, span: *span });
                out.push(hir::Stmt::Return(self.wrap_ok(Some(hir::Expr::new(hir::ExprKind::Local(id), ty)))));
            }
            Stmt::Fill { partial, target, with, body, span } => self.fill(*partial, target, with.as_ref(), body, *span, out),
            Stmt::Return { value, span } if self.lambda.is_some() => {
                if let Some(v) = self.lambda_return(value.as_ref(), *span) {
                    out.push(hir::Stmt::Return(v));
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
                    (Some(v), Some(t)) => self.coerce(v, t).map(Some),
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
                match v {
                    Some(v) => out.push(hir::Stmt::Return(self.wrap_ok(v))),
                    None => {}
                }
            }
            Stmt::Resume { value, span } => self.resume_stmt(value.as_ref(), *span, out),
            Stmt::Fail { value, because, span } => self.fail_stmt(value, because.as_ref(), *span, out),
            Stmt::Unsafe { body, span } => {
                if !self.g.unsafe_allowed {
                    self.diags.push(unsafe_disabled(Span::new(span.start, span.start + 6), "needs the file flag"));
                }
                let block = self.block(body);
                out.push(hir::Stmt::Unsafe(block));
            }
            Stmt::Destroy { target, span } => {
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
            Stmt::Expr(e) => match self.expr(e) {
                // A place has no effects; there is nothing to do.
                Some(Val::Expr(e)) if plain_place(&e) => {}
                Some(Val::Expr(e)) => out.push(hir::Stmt::Expr(e)),
                _ => {}
            },
        }
    }

    /// Store `e`, a new value for `place`. `compound` says it came from `x op= v`.
    fn store(&mut self, place: hir::Place, e: hir::Expr, compound: bool, out: &mut hir::Block) {
        if compound {
            // `x += v` calls the operator method: it may not fail unnoticed.
            let at = e.span;
            self.report_unhandled(at, &e);
        }
        // A field of a value being filled (or built by a constructor) is an initialisation.
        if !compound && !place.proj.is_empty() && self.filling.contains(&place.local) {
            out.push(hir::Stmt::InitField(place, e));
            return;
        }
        match e.kind {
            // `s += t` appends in place.
            hir::ExprKind::StrCall(hir::StrFn::Concat, args) if compound => {
                let append = hir::Expr::new(hir::ExprKind::StrCall(hir::StrFn::Append, args), Type::None);
                out.push(hir::Stmt::Expr(append));
            }
            _ => out.push(hir::Stmt::Assign(place, e)),
        }
    }

    /// `x = v;` or `x op= v;` on a local.
    fn assign_local(&mut self, name: &str, name_span: Span, op: Option<BinOp>, value: &ast::Expr, span: Span, out: &mut hir::Block) {
        let Some(id) = self.lookup(name) else {
            self.diags.push(Diagnostic::at(&codes::E0201, name_span, format!("unknown binding `{name}`")));
            self.expr(value);
            return;
        };
        self.note_use(id, name_span);
        if self.filling.contains(&id) {
            self.diags.push(
                Diagnostic::at(&codes::E0507, name_span, format!("`{name}` is still being filled, so it cannot be given a whole value"))
                    .note("assigned here")
                    .help(format!("give its fields values one at a time: `{name}.field = value;`")),
            );
            self.expr(value);
            return;
        }
        let ty = self.locals[id].ty;
        let mut place = hir::Place { local: id, proj: Vec::new(), span: name_span };
        let rhs = match op {
            None => match ty {
                // `r = rhs` re-points a borrow when `rhs` is one, and writes through it otherwise (R18).
                Some(t @ Type::Ref { to, .. }) => {
                    match self.expr_raw(value) {
                        Some(Val::Expr(x)) if x.ty.is_borrow() => {
                            self.borrow_value(Val::Expr(x), value, t, None)
                        }
                        Some(v) => {
                            place.proj.push(hir::HirProj::Deref);
                            let to = *to;
                            self.into(v, to, origin(value))
                        }
                        None => None,
                    }
                }
                Some(t) => {
                    self.coerce(value, t)
                }
                None => {
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
                        Val::Const(_) | Val::UConst(..) | Val::NoneLit(_) => None,
                    })
                } else {
                    self.read_local(id, name_span)
                };
                self.compound(op, current, name_span, value, span)
            }
        };
        match rhs {
            Some(e) => self.store(place, e, op.is_some(), out),
            None => {}
        }
    }

    /// `p.x = v;` or `p.x op= v;`.
    fn assign_field(&mut self, target: &ast::Expr, op: Option<BinOp>, value: &ast::Expr, span: Span, out: &mut hir::Block) {
        if let ExprKind::Index { recv, index } = &target.kind && self.map_assign(recv, index, op, value, span, out) {
            return;
        }
        let Some(place) = self.lvalue(target) else {
            self.expr(value);
            return;
        };
        if place.through_fixed {
            self.fixed_write(target.span);
            self.expr(value);
            return;
        }
        let rhs = match op {
            None => {
                self.coerce(value, place.ty)
            }
            Some(op) => {
                self.read_local(place.local, target.span);
                let current = self.place_expr(&place);
                self.compound(op, Some(current), target.span, value, span)
            }
        };
        match rhs {
            Some(e) => self.store(place.hir_place(target.span), e, op.is_some(), out),
            None => {}
        }
    }

    /// E0333: writing through a `*fixed T`, which can only be read.
    fn fixed_write(&mut self, at: Span) {
        let text = self.text(at);
        self.diags.push(
            Diagnostic::at(&codes::E0333, at, format!("cannot write through `{text}`: it is a `*fixed` pointer, which can only be read"))
                .note("written through a `*fixed` pointer here")
                .help("only a pointer made from a `mut` or `open` binding (`*T`) can be written through"),
        );
    }

    /// The new value for `target op= value`, given the target's current value.
    fn compound(&mut self, op: BinOp, current: Option<hir::Expr>, target: Span, value: &ast::Expr, span: Span) -> Option<hir::Expr> {
        let rv = self.expr(value);
        let lo = Origin { span: target, compound: false };
        let current = current?;
        let cur_ty = current.ty;
        match self.binary_vals(op, Val::Expr(current), lo, rv?, origin(value), span)? {
            // `d *= 2` stays in `d`'s unit; anything else is converted to it.
            Val::Expr(e) if cur_ty.mentions_unit() && e.ty != cur_ty => self.unit_into(Val::Expr(e), cur_ty, origin(value)),
            Val::Expr(e) => Some(e),
            Val::Const(_) | Val::UConst(..) | Val::NoneLit(_) => None,
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
                self.note_use(id, e.span);
                let ty = self.locals[id].ty?;
                Some(PlaceInfo { local: id, proj: Vec::new(), ty, through_deref: false, mut_field: false, through_shared: false, through_fixed: false })
            }
            ExprKind::SelfValue => {
                let id = self.self_local(e.span)?;
                let ty = self.locals[id].ty?;
                Some(PlaceInfo { local: id, proj: Vec::new(), ty, through_deref: false, mut_field: false, through_shared: false, through_fixed: false })
            }
            ExprKind::Field { recv, name, name_span } => {
                let mut p = self.lvalue(recv)?;
                Self::auto_deref(&mut p);
                if matches!(p.ty, Type::Dyn { .. }) {
                    self.dyn_field_write(name, *name_span);
                    return None;
                }
                let (i, ty) = self.field_of(p.ty, name, *name_span, recv.span)?;
                p.mut_field |= self.g.structs[p.ty.struct_id().unwrap()].fields[i].mutable;
                p.proj.push(hir::HirProj::Field(i));
                p.ty = ty;
                Some(p)
            }
            ExprKind::Deref(inner) => {
                let mut p = self.lvalue(inner)?;
                match p.ty {
                    Type::Ptr { to } | Type::FixedPtr { to } => {
                        if p.ty.is_fixed_ptr() {
                            p.through_fixed = true;
                            p.through_shared = true;
                        }
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
            ExprKind::Index { recv, index } => self.index_place(recv, index),
            _ => unreachable!("the parser only lets places be assigned"),
        }
    }

    /// A borrow in the middle of a path is read through (R4).
    fn auto_deref(p: &mut PlaceInfo) {
        if let Type::Ref { to, mutable } = p.ty {
            p.through_shared |= !mutable;
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
            match step {
                hir::HirProj::Downcast(_) => unreachable!("places the program writes have no payload steps"),
                hir::HirProj::Index(idx) => {
                    ty = ty.elem().unwrap();
                    e = hir::Expr::new(hir::ExprKind::Index(Box::new(e), idx.clone()), ty);
                }
                hir::HirProj::Field(i) => {
                    let i = *i;
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

    /// A read of local `id`. Whether it has a value is checked on the MIR.
    fn read_local(&mut self, id: LocalId, at: Span) -> Option<hir::Expr> {
        if self.filling.contains(&id) && self.now_depth == 0 {
            self.incomplete_use(id, at);
            return None;
        }
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
                let id = self.lookup("self");
                if let Some(id) = id {
                    self.note_use(id, at);
                }
                id
            }
        }
    }

    /// Field `name` of a value of type `ty`: its index and type. Reports an
    /// unknown field, or a private one used outside the struct's methods.
    fn field_of(&mut self, ty: Type, name: &str, name_span: Span, recv_span: Span) -> Option<(usize, Type)> {
        if let Type::Dyn { id, .. } = ty {
            return self.dyn_field(id, name, name_span, recv_span);
        }
        if let Some(sid) = ty.struct_id()
            && self.g.structs[sid].placeholder.is_some()
            && !self.g.structs[sid].fields.iter().any(|f| f.name == name)
            && self.beyond_constraint(sid, &format!("field `{name}`"), name_span)
        {
            return None;
        }
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
                .help(self.trusted_help(ty, format!("only `{ty}`'s own methods can use it; put it under `public:` to use it here")));
            self.diags.push(d);
            return None;
        }
        index::with(|ix| {
            let ty = f.ty.map_or("?".to_string(), |t| index::demangle(&t.to_string()));
            ix.types.push((recv_span.to(name_span), ty.clone()));
            ix.refs.push(index::Ref { at: name_span, def: Some(f.span), code: format!("{ty} {name}"), note: format!("field of `{}`", index::demangle(&ty.to_string())) });
        });
        Some((i, f.ty?))
    }

    /// Are we inside a method of struct type `ty` (so its private members are usable)?
    fn inside(&self, ty: Type) -> bool {
        self.method.is_some_and(|m| m.trusted && Some(m.struct_id) == ty.struct_id())
    }

    /// The extra help for a private member used in a `TYPE with I { }` block.
    fn trusted_help(&self, ty: Type, help: String) -> String {
        if self.method.is_some_and(|m| !m.trusted && Some(m.struct_id) == ty.struct_id()) {
            format!("{help}, or write `with trusted` before the interface name: a plain `with` block only sees public members")
        } else {
            help
        }
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
                .help(self.trusted_help(self.g.structs[s].ty, format!("only `{}`'s own methods can use it; put it under `public:` to use it here", info.name))),
        );
        false
    }

    /// Check `e` and convert it to `to`, reporting a mismatch or a lossy conversion.
    fn coerce(&mut self, e: &ast::Expr, to: Type) -> Option<hir::Expr> {
        self.note_expected(e.span, to);
        if let Type::Func(sig) = to {
            return self.coerce_func(e, to, sig);
        }
        if let Some(made) = self.bare_variant(e, to) {
            return made;
        }
        if matches!(e.kind, ExprKind::NoneValue) && optionals::is_optional(self.g, to) {
            return Some(optionals::none_value(to, e.span));
        }
        if to.is_ptr() && matches!(e.kind, ExprKind::NoneValue) {
            return Some(hir::Expr::new(hir::ExprKind::NullPtr, to).at(e.span));
        }
        if to.is_borrow() {
            return self.coerce_borrow(e, to, None);
        }
        if let ExprKind::MapLit(pairs) = &e.kind {
            return self.map_literal(pairs, Some(to), e.span);
        }
        if let ExprKind::ListLit(items) = &e.kind {
            return self.list_literal(items, Some(to), e.span);
        }
        let v = self.expr(e)?;
        self.into(v, to, origin(e))
    }

    /// `e` where a borrow of type `to` is wanted: the value must be a borrow
    /// of that kind. `call` is the function being called, for E0321.
    fn coerce_borrow(&mut self, e: &ast::Expr, to: Type, call: Option<FuncId>) -> Option<hir::Expr> {
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
            Val::Expr(x) if to.is_fat() && x.ty.is_borrow() => self.unsize(x, e, to),
            Val::Expr(x) if x.ty.is_borrow() => {
                let o = origin(e);
                self.into(Val::Expr(x), to, o)
            }
            other => {
                let plain = match &other {
                    Val::Const(_) | Val::UConst(..) => true,
                    Val::NoneLit(_) => false,
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
    fn init_inner(&mut self, v: &ast::Init, ty: Option<Type>) -> Option<hir::Expr> {
        match v {
            ast::Init::Expr(e) => {
                let Some(t) = ty else {
                    self.expr(e);
                    return None;
                };
                let Some(s) = t.struct_id() else { return self.coerce(e, t) };
                // One value for a struct with one enum field: a bare variant takes that enum.
                if let [only] = &self.g.structs[s].fields[..]
                    && let Some(fty) = only.ty
                    && let Some(made) = self.bare_variant(e, fty)
                {
                    return self.construct(t, vec![(None, made.map(Val::Expr), origin(e))], e.span);
                }
                match self.expr_raw(e)? {
                    Val::Expr(x) if x.ty == t => Some(x),
                    Val::Expr(x) if x.ty.is_borrow() && x.ty.target() == Some(t) => match self.read(Val::Expr(x)) {
                        Val::Expr(x) => Some(x),
                        Val::Const(_) | Val::UConst(..) | Val::NoneLit(_) => None,
                    },
                    // One value of another type builds a one-field struct.
                    Val::Expr(x) if self.g.structs[s].fields.len() != 1 && (x.ty.struct_id().is_some() || x.ty == Type::None) => {
                        self.into(Val::Expr(x), t, origin(e))
                    }
                    other => self.construct(t, vec![(None, Some(other), origin(e))], e.span),
                }
            }
            ast::Init::Fields(values, span) => {
                let sid = ty.and_then(Type::struct_id);
                let checked: Vec<_> = values
                    .iter()
                    .enumerate()
                    .map(|(i, fv)| {
                        let want = sid.and_then(|s| {
                            let fields = &self.g.structs[s].fields;
                            match &fv.name {
                                Some((n, _)) => fields.iter().find(|f| f.name == *n),
                                None => fields.get(i),
                            }
                            .and_then(|f| f.ty)
                        });
                        let literal = sid.and_then(|s| self.literal_for_field(s, i, &fv.name, &fv.value));
                        let v = match want.and_then(|w| self.bare_variant(&fv.value, w)) {
                            Some(made) => made.map(Val::Expr),
                            None => match literal {
                                Some(done) => done,
                                None => self.expr_raw(&fv.value),
                            },
                        };
                        (fv.name.clone(), v, origin(&fv.value))
                    })
                    .collect();
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

    /// What follows `=` in a declaration of type `ty`.
    fn init(&mut self, v: &ast::Init, ty: Option<Type>) -> Option<hir::Expr> {
        if let (ast::Init::Expr(e), Some(t)) = (v, ty)
            && let Some(done) = self.failing_shorthand(e, t)
        {
            return done;
        }
        let before = self.diags.len();
        let x = self.init_inner(v, ty)?;
        // A constructor that can fail, chosen by the shorthand, with nothing after it.
        if matches!(&x.kind, hir::ExprKind::Call(fid, _) if !self.g.sigs[*fid].fails.is_empty())
            && !self.diags[before..].iter().any(|d| d.code.id == "E0544")
        {
            let span = match v {
                ast::Init::Expr(e) => e.span,
                ast::Init::Fields(_, span) => *span,
            };
            self.report_unhandled(span, &x);
            return None;
        }
        Some(x)
    }

    /// `Port p = 8080 maybe fallback;` (also `fails` and `fails with`): the
    /// shorthand builds the struct with a constructor that can fail, and the
    /// suffix says what happens when it does. `None` when `e` is not that.
    fn failing_shorthand(&mut self, e: &ast::Expr, t: Type) -> Option<Option<hir::Expr>> {
        let s = t.struct_id()?;
        if !self.g.structs[s].ctors.iter().any(|c| !self.g.sigs[c.func].fails.is_empty()) {
            return None;
        }
        let value = match &e.kind {
            ExprKind::Maybe { value, .. } | ExprKind::PassUp(value) => value,
            ExprKind::Handled { call, .. } => call,
            _ => return None,
        };
        match &value.kind {
            ExprKind::Call { .. } | ExprKind::Method { .. } | ExprKind::New { .. } | ExprKind::Path { .. } => return None,
            ExprKind::Var(n) => {
                let own = self.lookup(n).and_then(|id| self.locals[id].ty);
                if own.is_none_or(|lt| lt == t || optionals::is_optional(self.g, lt)) {
                    return None;
                }
            }
            _ => {}
        }
        let v = self.expr_raw(value);
        let Some(made) = self.construct(t, vec![(None, v, origin(value))], value.span) else { return Some(None) };
        let word = if matches!(e.kind, ExprKind::Maybe { .. }) { "maybe" } else { "fails" };
        let f = match self.classify(made) {
            Ok(f) => f,
            Err(_) => {
                self.cannot_fail(value, word);
                return Some(None);
            }
        };
        let done = match &e.kind {
            ExprKind::Maybe { fallback, .. } => self.maybe_failing(f, fallback),
            ExprKind::PassUp(_) => self.pass_up_failing(e, f),
            ExprKind::Handled { handlers, .. } => self.handled_failing(e, f, handlers),
            _ => unreachable!(),
        };
        Some(match done {
            Some(Val::Expr(x)) => Some(x),
            _ => None,
        })
    }

    /// The construction shorthand: struct `t` from values given in field
    /// order, or all by name. Every field must be given exactly once.
    fn construct(&mut self, t: Type, values: Vec<GivenValue>, span: Span) -> Option<hir::Expr> {
        let s = t.struct_id().unwrap();
        if !self.g.structs[s].ctors.is_empty() {
            return self.construct_with_ctors(t, values, span);
        }
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
                (Some(v), Some(fty)) => self.into(v, fty, o),
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
            Val::NoneLit(s) => {
                self.none_without_type(s);
                None
            }
            Val::UConst(c, u) => self.unit_materialise(&c, u, span),
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
        if let Val::NoneLit(span) = v {
            if optionals::is_optional(self.g, to) {
                return Some(optionals::none_value(to, span));
            }
            if to.is_ptr() {
                return Some(hir::Expr::new(hir::ExprKind::NullPtr, to).at(span));
            }
            self.none_without_type(span);
            return None;
        }
        // A `T` goes to a `T?` by itself (`uint32? x = 5;`, `return 5;`).
        if let Some(inner) = optionals::inner_of(self.g, to) && !matches!(&v, Val::Expr(e) if e.ty == to) {
            let x = self.into(v, inner, from)?;
            return Some(optionals::some_value(to, x, from.span));
        }
        // An optional where its value is wanted is E0531.
        if let Val::Expr(e) = &v
            && optionals::is_optional(self.g, e.ty)
            && !optionals::is_optional(self.g, to)
        {
            let ty = e.ty;
            self.optional_as_value(ty, Some(to), from);
            return None;
        }
        if to.mentions_unit() || units::has_unit(&v) {
            return self.unit_into(v, to, from);
        }
        match v {
            Val::Const(c) if to.is_numeric() => Some(self.fit_const(&c, to, from.span, false)),
            Val::UConst(..) => unreachable!("units are handled first"),
            Val::NoneLit(_) => unreachable!("`none` is handled first"),
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
            // A pointer made on the Drachm side goes to the C side as it is, and any
            // pointer goes where C wants a `void *` (`foreign *none`).
            Val::Expr(e)
                if e.ty != to
                    && matches!(to, Type::Foreign { to: target } if matches!(e.ty, Type::Ptr { to: from } | Type::FixedPtr { to: from } | Type::Foreign { to: from } if from == target || *target == Type::None)) =>
            {
                Some(hir::Expr::new(hir::ExprKind::PtrConv(Box::new(e)), to))
            }
            Val::Expr(e) if e.ty.is_ptr() && to.is_ptr() && e.ty != to => {
                self.diags.push(
                    Diagnostic::at(&codes::E0334, from.span, format!("expected `{to}`, found `{}`", e.ty))
                        .note(format!("this is `{}`", e.ty))
                        .note_line("`*T` and `*fixed T` do not convert to each other")
                        .help("inside `unsafe { }`, `p::freeze()` gives a `*fixed T` from a `*T`, and `p::thaw()` gives a `*T` from a `*fixed T`"),
                );
                None
            }
            Val::Expr(e) if e.ty == Type::None => {
                self.diags.push(
                    Diagnostic::at(&codes::E0104, from.span, format!("expected `{to}`, but this produces no value"))
                        .note("returns `none`"),
                );
                None
            }
            Val::Expr(e) if e.ty.is_numeric() && to.is_numeric() && e.ty != to && (e.ty.is_c() || to.is_c()) => {
                self.c_types_apart(e.ty, to, from.span);
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

    /// E0704: two number types of which one is a C type.
    fn c_types_apart(&mut self, from: Type, to: Type, at: Span) {
        let (c, other) = if from.is_c() { (from, to) } else { (to, from) };
        self.diags.push(
            Diagnostic::at(&codes::E0704, at, format!("`{from}` does not become `{to}` by itself"))
                .note(format!("this is `{from}`"))
                .note_line(format!("`{c}` is a C type of its own, not a synonym of `{other}`"))
                .help(format!("convert it: `x::to<{to}>()`")),
        );
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
        } else if at.compound || text.contains("::") || text.contains("->") {
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
    fn fit_const(&mut self, c: &Const, written: Type, span: Span, explicit: bool) -> hir::Expr {
        // A C number type holds what its representation holds; the value is built
        // in the representation and given the C type afterwards.
        let to = written.bare();
        let mut e = match to {
            Type::Int { .. } => int_lit(c.wrap_to_int(to), to),
            _ => hir::Expr::new(hir::ExprKind::Float(c.to_float(to)), to),
        };
        if !explicit && !c.fits(to) {
            let what = self.quote(span, "this constant");
            let (becomes, note) = match (&e.kind, c) {
                (hir::ExprKind::Int(bits), Const::Dec(r)) if !r.is_integer() => (
                    match Const::from_int_bits(*bits, to) {
                        Const::Int(v) => v.to_string(),
                        Const::Dec(_) | Const::NegZero => unreachable!(),
                    },
                    format!("`{to}` holds only whole numbers, from {}; the fraction is dropped", to.describe_range()),
                ),
                (hir::ExprKind::Int(bits), _) => (
                    match Const::from_int_bits(*bits, to) {
                        Const::Int(v) => v.to_string(),
                        Const::Dec(_) | Const::NegZero => unreachable!(),
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
        e.ty = written;
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
                self.check_unhandled(e, &x);
                index::with(|ix| ix.types.push((e.span, index::demangle(&x.ty.to_string()))));
                Some(Val::Expr(x))
            }
            c => Some(c),
        }
    }

    fn expr_inner(&mut self, e: &ast::Expr) -> Option<Val> {
        let typed = |kind, ty| Some(Val::Expr(hir::Expr::new(kind, ty)));
        match &e.kind {
            ExprKind::PassUp(call) => self.pass_up_expr(e, call),
            ExprKind::Handled { call, handlers } => self.handled_expr(e, call, handlers),
            ExprKind::Num(n) if n.unit.is_some() => self.unit_literal(n, e.span),
            ExprKind::Num(n) => match n.suffix {
                Some(ty) => Some(Val::Expr(self.fit_const(&n.value, ty, e.span, false))),
                None => Some(Val::Const(n.value.clone())),
            },
            ExprKind::Bool(b) => typed(hir::ExprKind::Bool(*b), Type::Bool),
            ExprKind::Str(parts) => self.string_literal(parts),
            ExprKind::Lambda { moves, params, body } => self.lambda_expr(*moves, params, body, e.span).map(Val::Expr),
            ExprKind::Var(name) => match self.lookup(name) {
                Some(id) => {
                    self.note_use(id, e.span);
                    self.read_local(id, e.span).map(Val::Expr)
                }
                // A value parameter is a constant of its type.
                None if let Some((ty, v)) = self.g.tenv.get(name).and_then(|t| t.as_val()) => {
                    let c = Const::Int(num_bigint::BigInt::from(v));
                    Some(Val::Expr(self.fit_const(&c, ty, e.span, true)))
                }
                None if let Some(info) = self.g.consts.get(name).cloned() => Some(Val::Expr(self.fit_const(&info.value, info.ty, e.span, true))),
                None if self.ambiguous_bare(name, e.span) => None,
                None if self.g.mono.templates.contains_key(name) && !self.g.funcs_by_name.contains_key(name) => {
                    self.generic_by_name(name, e.span);
                    None
                }
                None if self.g.mono.type_templates.contains_key(name) => {
                    self.generic_type_alone(name, e.span);
                    None
                }
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
            ExprKind::Path { owner, owner_span, name, name_span } => self.path_expr(owner, *owner_span, name, *name_span, e.span),
            ExprKind::SelfValue => {
                let id = self.self_local(e.span)?;
                self.read_local(id, e.span).map(Val::Expr)
            }
            ExprKind::Field { recv, name, name_span } => {
                let v = match self.expr(recv)? {
                    Val::NoneLit(s) => {
                        self.none_without_type(s);
                        return None;
                    }
                    Val::Const(_) | Val::UConst(..) => {
                        self.diags.push(Diagnostic::at(&codes::E0201, *name_span, "numbers have no fields").label(recv.span, "this is a number"));
                        return None;
                    }
                    Val::Expr(v) => v,
                };
                let (i, ty) = self.field_of(v.ty, name, *name_span, recv.span)?;
                typed(hir::ExprKind::Field(Box::new(v), i), ty)
            }
            ExprKind::ListLit(items) => self.list_literal(items, None, e.span).map(Val::Expr),
            ExprKind::MapLit(pairs) => self.map_literal(pairs, None, e.span).map(Val::Expr),
            ExprKind::Index { recv, index } => self.index_expr(recv, index),
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
            ExprKind::Binary(BinOp::Same, l, r) => self.same_as(l, r, e.span, "same as"),
            ExprKind::Binary(BinOp::NotSame, l, r) => match self.same_as(l, r, e.span, "not same as")? {
                Val::Expr(x) => Some(Val::Expr(hir::Expr::new(hir::ExprKind::Not(Box::new(x)), Type::Bool))),
                other => Some(other),
            },
            ExprKind::Binary(op, l, r) => {
                let lv = self.expr(l);
                // `d == North`: a bare variant on the right takes its enum from the left.
                let rv = match &lv {
                    Some(Val::Expr(x)) if matches!(op, BinOp::Eq | BinOp::Ne) && x.ty.enum_id().is_some() => self.coerce(r, x.ty).map(Val::Expr),
                    _ => self.expr(r),
                };
                self.binary_vals(*op, lv?, origin(l), rv?, origin(r), e.span)
            }
            ExprKind::IncDec { inc, prefix, target } => {
                let sym = if *inc { "++" } else { "--" };
                if !matches!(target.kind, ExprKind::Var(_) | ExprKind::SelfValue | ExprKind::Field { .. } | ExprKind::Index { .. }) {
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
                self.read_local(p.local, target.span)?;
                let ty = p.ty;
                typed(hir::ExprKind::IncDec { place: p.hir_place(e.span), inc: *inc, prefix: *prefix }, ty)
            }
            ExprKind::Call { name, targs, args } => {
                if let Some(id) = self.lookup(name) {
                    self.note_use(id, Span::new(e.span.start, e.span.start + name.len()));
                    return self.call_value(id, name, targs, args, e.span);
                }
                self.note_func(name, e.span);
                if !self.g.funcs_by_name.contains_key(name) && self.g.mono.templates.contains_key(name) {
                    return self.generic_call(name, targs, args, e.span);
                }
                if !targs.is_empty() {
                    self.not_generic(name, targs, args.iter().collect(), e.span);
                    return None;
                }
                let Some(&id) = self.g.funcs_by_name.get(name) else {
                    if !self.ambiguous_bare(name, e.span) {
                        let mut d = Diagnostic::at(&codes::E0201, e.span, format!("unknown function `{name}`"));
                        if self.g.structs_by_name.contains_key(name) {
                            d = d.help(format!("build a `{name}` with `new {name} of (...)`"));
                        }
                        self.diags.push(d);
                    }
                    for a in args {
                        self.expr(a);
                    }
                    return None;
                };
                let args = self.args(id, args, e.span);
                let ty = self.g.sigs[id].ret?;
                typed(hir::ExprKind::Call(id, args?), ty)
            }
            ExprKind::Method { recv, unit: Some(spec), name_span, args, .. } => self.unit_in(recv, spec, args, *name_span, e.span),
            ExprKind::Method { recv, name, name_span, targs, args, .. } => {
                if let ExprKind::Var(n) = &recv.kind
                    && self.lookup(n).is_none()
                {
                    if matches!(n.as_str(), "fs" | "os") {
                        return self.system_call(n, name, *name_span, recv.span, targs, args, e.span);
                    }
                    if n == "time" {
                        return self.time_call(name, *name_span, recv.span, targs, args, e.span);
                    }
                    if n == "io" {
                        return self.io_call(name, *name_span, recv.span, targs, args, e.span);
                    }
                    if self.g.mono.type_templates.contains_key(n) {
                        self.generic_type_alone(n, recv.span);
                        for a in args {
                            self.expr(a);
                        }
                        return None;
                    }
                    if let Some(&ty) = self.g.tenv.get(n) {
                        return self.static_on_type(ty, n, name, *name_span, targs, args, e.span);
                    }
                    if let Some(&s) = self.g.structs_by_name.get(n) {
                        return self.static_call(s, name, *name_span, targs, args, e.span);
                    }
                    if let Ok(ty) = Type::from_name(n) {
                        return self.static_on_type(ty, n, name, *name_span, targs, args, e.span);
                    }
                    if let Some(&en) = self.g.enums_by_name.get(n) {
                        return self.qualified_variant(en, name, *name_span, args, e.span);
                    }
                }
                if let ExprKind::TypeArgs { name: tn, targs: tt } = &recv.kind {
                    return self.generic_static(tn, tt, recv.span, name, *name_span, targs, args, e.span);
                }
                self.method(recv, name, *name_span, targs, args, e.span)
            }
            ExprKind::Borrow { mutable, operand } => self.borrow_expr(*mutable, operand),
            ExprKind::Deref(inner) => {
                let v = self.expr_raw(inner)?;
                let x = self.materialise(v, inner.span)?;
                match x.ty {
                    Type::Ptr { to } | Type::FixedPtr { to } => {
                        typed(hir::ExprKind::Deref(Box::new(x)), *to)
                    }
                    t => {
                        self.deref_error(t, inner.span, e.span);
                        None
                    }
                }
            }
            ExprKind::CopyOf(inner) => self.copy_expr(e, inner),
            ExprKind::Take { place, leaving, reorders } => self.take_expr(e, place, leaving.as_deref(), *reorders),
            ExprKind::Lend(inner) => self.lend_expr(e, inner),
            ExprKind::CString(_) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, e.span, "`as c string` is only allowed as an argument of a C function")
                        .note("the text is valid for that one call")
                        .help("write it in the call: `unsafe { sdl::SDL_SetWindowTitle(w, &title as c string); }`"),
                );
                None
            }
            ExprKind::AddressOf(inner) => self.address_expr(e, inner),
            ExprKind::New { name, name_span, targs, values } => {
                if !targs.is_empty() && self.g.mono.type_templates.contains_key(name) {
                    return self.new_generic(name, *name_span, targs, values, e.span);
                }
                if !targs.is_empty() {
                    self.not_generic(name, targs, values.iter().map(|v| &v.value).collect(), e.span);
                    return None;
                }
                self.new_expr(name, *name_span, values, e.span)
            }
            ExprKind::TypeArgs { name, targs } => {
                if self.g.mono.type_templates.contains_key(name) {
                    self.generic_type_alone(name, e.span);
                    return None;
                }
                self.not_generic(name, targs, Vec::new(), e.span);
                None
            }
            ExprKind::Now(inner) => {
                self.now_depth += 1;
                let v = self.expr(inner);
                self.now_depth -= 1;
                v
            }
            ExprKind::NoneValue => {
                self.none_without_type(e.span);
                None
            }
            ExprKind::Maybe { value, fallback } => self.maybe_expr(value, fallback),
            ExprKind::OptionalTest { value, has } => self.optional_test(value, *has),
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
        let v = self.expr_raw(operand)?;
        let x = self.materialise(v, operand.span)?;
        let name = self.text(operand.span).to_string();
        match x.ty {
            Type::Func(_) => {
                self.diags.push(Diagnostic::at(&codes::E0652, operand.span, "a function value cannot be stored behind a borrow"));
                None
            }
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
        // `copy names::first()`: the item of the list, copied out as an owned `T?`.
        if let ExprKind::Method { name, .. } = &inner.kind
            && matches!(name.as_str(), "get" | "first" | "last")
        {
            let before = std::mem::replace(&mut self.copying_item, true);
            let v = self.expr(inner);
            self.copying_item = before;
            return v;
        }
        if !ast_place(inner) {
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
        // A `copy` of a place with no borrow in the way builds without the
        // borrow checker; any other needs it.
        let plain = !matches!(x_ty, Type::Ref { .. }) && !has_deref(&value);
        if !(plain && !self.copies_anyway(ty)) {
        }
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
            // A number with a unit is a number (`of` is always Int or Float).
            Type::Int { .. } | Type::Float(_) | Type::Bool | Type::Func(_) | Type::Unit { .. } | Type::Ref { mutable: false, .. } => true,
            Type::Struct { .. } | Type::Enum { .. } | Type::Array { .. } => self.g.copy_of(t),
            _ => false,
        }
    }

    /// `take p.f` / `take p.f leaving v`.
    fn take_expr(&mut self, e: &ast::Expr, place: &ast::Expr, leaving: Option<&ast::Expr>, reorders: bool) -> Option<Val> {
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
        if let ExprKind::Index { recv, index } = &place.kind {
            return self.take_item(place, recv, index, leaving, reorders);
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
            Some(v) => Some(Box::new(self.coerce(v, p.ty)?)),
            None => None,
        };
        let ty = p.ty;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Take { place: p.hir_place(place.span), leaving }, ty)))
    }

    /// `lend r`.
    fn lend_expr(&mut self, e: &ast::Expr, inner: &ast::Expr) -> Option<Val> {
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
                    Val::Const(_) | Val::UConst(..) => "a number".to_string(),
                    Val::Expr(x) => format!("`{}`", x.ty),
                    Val::NoneLit(_) => "`none`".to_string(),
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
        if p.ty.is_func() {
            self.diags.push(Diagnostic::at(&codes::E0652, inner.span, "a function value cannot be stored behind a pointer"));
            return None;
        }
        let l = &self.locals[p.local];
        // A place that can be changed gives `*T`; anything else (an immutable
        // binding, a place behind `&` or `*fixed`) gives the read-only `*fixed T` (#9).
        let writable = if p.through_shared {
            false
        } else if p.through_deref {
            true
        } else {
            p.mut_field || l.mutable || l.open || l.is_self
        };
        self.read_local(p.local, inner.span)?;
        let ty = if writable { Type::ptr(p.ty) } else { Type::fixed_ptr(p.ty) };
        let place = self.place_expr(&p);
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::AddressOf(Box::new(place)), ty)))
    }

    /// `p == none` / `p != none` (either side).
    fn none_compare(&mut self, op: BinOp, l: &ast::Expr, r: &ast::Expr, span: Span) -> Option<Val> {
        let _ = span;
        let (other, none_left) = if matches!(l.kind, ExprKind::NoneValue) { (r, true) } else { (l, false) };
        if matches!(other.kind, ExprKind::NoneValue) {
            self.none_without_type(span);
            return None;
        }
        let v = self.expr_raw(other)?;
        match v {
            Val::Expr(x) if x.ty.is_ptr() => {
                let null = hir::Expr::new(hir::ExprKind::NullPtr, x.ty);
                let hop = if op == BinOp::Eq { hir::BinOp::Eq } else { hir::BinOp::Ne };
                let (a, b) = if none_left { (null, x) } else { (x, null) };
                Some(Val::Expr(hir::Expr::new(hir::ExprKind::Binary(hop, Box::new(a), Box::new(b)), Type::Bool)))
            }
            Val::Expr(x) if optionals::is_optional(self.g, x.ty) => {
                self.optional_vs_none(op, other, span);
                None
            }
            _ => {
                self.none_without_type(span);
                None
            }
        }
    }

    /// `&s as c string` given to a C function: a NUL-terminated copy of `s`,
    /// which lives to the end of the statement, as the pointer the parameter wants.
    fn c_string_arg(&mut self, a: &ast::Expr, param: Type) -> Option<hir::Expr> {
        let ExprKind::CString(inner) = &a.kind else { unreachable!() };
        if param != Type::foreign(Type::U8) && param != Type::foreign(Type::Int { bits: 8, signed: true }) {
            self.diags.push(
                Diagnostic::at(&codes::E0101, a.span, format!("expected `{param}`, found text for C"))
                    .note("`&s as c string` is a `foreign *uint8`")
                    .help("only a parameter that takes C text (`const char *`) accepts it"),
            );
            return None;
        }
        let text = self.coerce(inner, Type::Str)?;
        let terminated = hir::Expr::new(hir::ExprKind::StrCall(hir::StrFn::Concat, vec![text, str_lit("\0".into())]), Type::Str).at(a.span);
        Some(hir::Expr::new(hir::ExprKind::StrCall(hir::StrFn::CPtr, vec![terminated]), param).at(a.span))
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
                Some(p) if matches!(a.kind, ExprKind::CString(_)) && self.g.foreign_funcs.contains(&id) => self.c_string_arg(a, *p),
                // The callee owns its parameters.
                Some(p) if p.is_borrow() => self.coerce_borrow(a, *p, Some(id)),
                Some(p) => self.coerce(a, *p),
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
        self.note_method(s, &m, name_span);
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
        if self.beyond_constraint(s, &format!("method `{name}`"), at) {
            return;
        }
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

    fn system_call(&mut self, module: &str, name: &str, name_span: Span, module_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        use hir::SystemOp::*;
        let imported = if module == "fs" { self.g.fs_imported } else { self.g.os_imported };
        if !imported {
            self.diags.push(Diagnostic::at(&codes::E0206, module_span, format!("module `{module}` is used but not imported")));
            return None;
        }
        let op = match (module, name) {
            ("fs", "read") => Read,
            ("fs", "write") => Write,
            ("os", "platform") => Platform,
            ("os", "getEnv") => GetEnv,
            ("fs", "create" | "open" | "readLine" | "writeLine") | ("os", "cwd") => {
                self.diags.push(Diagnostic::at(&codes::E0901, name_span, format!("`{module}::{name}` is not implemented yet")));
                return None;
            }
            _ => {
                self.diags.push(Diagnostic::at(&codes::E0201, name_span, format!("module `{module}` has no function `{name}`")));
                return None;
            }
        };
        if args.len() != op.inputs() || !targs.is_empty() {
            self.diags.push(Diagnostic::at(&codes::E0103, span, format!("`{}` takes {} string arguments and no type arguments", op.name(), op.inputs())));
            return None;
        }
        let args = args.iter().map(|arg| self.coerce(arg, Type::Str)).collect::<Option<Vec<_>>>()?;
        let mut fails = Vec::new();
        let ty = match op {
            Read | Write => {
                let names = if op == Read { &["FileMissing", "AccessDenied", "InvalidPath", "InvalidFileText", "FileIoError"][..] }
                    else { &["FileMissing", "AccessDenied", "InvalidPath", "FileIoError"][..] };
                fails.extend(names.iter().map(|name| self.g.enums_by_name[*name]));
                failures::outcome_type(self.g, op.fs_result(), &fails);
                op.fs_result()
            }
            Platform | CText => Type::Str,
            GetEnv => optionals::optional_of(self.g, Type::Str)?,
        };
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::System(op, args, fails), ty).at(span)))
    }

    fn time_call(&mut self, name: &str, name_span: Span, module_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        if !self.g.time_imported {
            self.diags.push(
                Diagnostic::at(&codes::E0206, module_span, "module `time` is used but not imported")
                    .help("add `in time;` at the top of this file"),
            );
            self.g.time_imported = true;
        }
        if name != "sleep" {
            self.diags.push(Diagnostic::at(&codes::E0201, name_span, format!("module `time` has no function `{name}`"))
                .help("use `time::sleep(duration)` to pause"));
            return None;
        }
        if let Some(arg) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, arg.span, "`time::sleep` takes no type arguments"));
        }
        let [duration] = args else {
            self.diags.push(Diagnostic::at(&codes::E0103, span, "`time::sleep` takes one duration"));
            return None;
        };
        let duration = self.coerce(duration, units::seconds_type())?;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Sleep(Box::new(duration)), Type::None).at(span)))
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
        if matches!(name, "inln" | "ended") {
            if let Some(t) = targs.first() {
                self.diags.push(Diagnostic::at(&codes::E0103, t.span, format!("`io::{name}` takes no type arguments")));
            }
            if let Some(a) = args.first() {
                self.diags.push(Diagnostic::at(&codes::E0103, a.span, format!("`io::{name}` takes no arguments")));
                for a in args {
                    self.expr(a);
                }
                return None;
            }
            return Some(Val::Expr(if name == "inln" {
                hir::Expr::new(hir::ExprKind::InLine, Type::Str)
            } else {
                hir::Expr::new(hir::ExprKind::InEnded, Type::Bool)
            }));
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
                        .help("it has `io::out`, `io::outln`, `io::outf`, `io::inln` and `io::ended`"),
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
                    Val::Const(_) | Val::UConst(..) => "a number".to_string(),
                    Val::Expr(a) => format!("`{}`", a.ty),
                    Val::NoneLit(_) => "`none`".to_string(),
                };
                let o = origin(&args[0]);
                let text = self.text(o.span);
                let fix = if o.compound { format!("({text})::text()") } else { format!("{text}::text()") };
                let mut d = Diagnostic::at(&codes::E0101, o.span, format!("`io::{name}` prints text, found {found}"))
                    .note(format!("this is {found}"));
                let has_text = match &other {
                    Val::Const(_) | Val::UConst(..) => true,
                    Val::NoneLit(_) => false,
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
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Out { newline, arg: Box::new(arg) }, Type::None)))
    }

    fn method(&mut self, recv: &ast::Expr, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        match self.expr(recv) {
            Some(Val::Expr(v)) if self.g.ifaces.ext.get(&v.ty).is_some_and(|ms| ms.iter().any(|m| m.name == name)) => {
                self.ext_method(v, recv.span, name, name_span, targs, args, span)
            }
            Some(Val::Expr(v)) if v.ty.struct_id().is_some() => {
                let s = v.ty.struct_id().unwrap();
                self.struct_method(s, v, recv.span, name, name_span, targs, args, span)
            }
            Some(Val::Expr(v)) if matches!(v.ty, Type::Dyn { .. }) => self.dyn_method(v, recv.span, name, name_span, targs, args, span),
            Some(Val::Expr(v)) if v.ty.is_ptr() && matches!(name, "freeze" | "thaw") => {
                self.pointer_convert(v, recv.span, name, name_span, args)
            }
            Some(Val::Expr(v)) if v.ty == Type::foreign(Type::U8) && name == "text" => self.foreign_text(v, recv.span, name_span, targs, args, span),
            Some(Val::Expr(v)) if v.ty == Type::Str => self.string_method(v, name, name_span, targs, args),
            Some(Val::Expr(v)) if v.ty.is_map() => self.map_method(v, recv, name, name_span, args),
            Some(Val::Expr(v)) if v.ty.is_list() || v.ty.is_array() => self.list_method(v, recv, name, name_span, targs, args),
            Some(Val::Expr(v)) if self.is_failure_type(v.ty) => self.failure_method(v, recv.span, name, name_span, args),
            value => self.value_method(value, recv, name, name_span, targs, args),
        }
    }

    /// `p::text()` on a `foreign *uint8`: the text up to the NUL, checked as
    /// UTF-8. It fails (`InvalidText`) on bad text and on a null pointer.
    fn foreign_text(&mut self, v: hir::Expr, recv: Span, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        let _ = recv;
        for a in args {
            self.expr(a);
        }
        if let Some(a) = args.first().map(|a| a.span).or(targs.first().map(|t| t.span)) {
            self.diags.push(Diagnostic::at(&codes::E0103, a, "`text` takes no arguments"));
            return None;
        }
        let _ = name_span;
        let fail = *self.g.enums_by_name.get("InvalidText")?;
        let fails = vec![fail];
        failures::outcome_type(self.g, Type::Str, &fails);
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::System(hir::SystemOp::CText, vec![v], fails), Type::Str).at(span)))
    }

    /// `p::freeze()` (`*T` to `*fixed T`) and `p::thaw()` (the reverse).
    fn pointer_convert(&mut self, v: hir::Expr, recv: Span, name: &str, name_span: Span, args: &[ast::Expr]) -> Option<Val> {
        for a in args {
            self.expr(a);
        }
        if let Some(a) = args.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, a.span, format!("`{name}` takes no arguments")));
            return None;
        }
        let (want_fixed, to) = match (name, v.ty) {
            ("freeze", Type::Ptr { to }) => (true, *to),
            ("thaw", Type::FixedPtr { to }) => (false, *to),
            _ => {
                let (other, why) = if name == "freeze" {
                    ("`thaw`", "`freeze` turns a `*T` into a `*fixed T`")
                } else {
                    ("`freeze`", "`thaw` turns a `*fixed T` into a `*T`")
                };
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("`{}` has no method `{name}`", v.ty))
                        .label(recv, format!("this is `{}`", v.ty))
                        .help(format!("{why}; {other} is the other way")),
                );
                return None;
            }
        };
        let ty = if want_fixed { Type::fixed_ptr(to) } else { Type::ptr(to) };
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::PtrConv(Box::new(v)), ty)))
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
                if to.mentions_unit() || units::has_unit(&value) {
                    return self.unit_to(value, to, recv.span, targs[0].span);
                }
                // `c::bool` and `bool` are the same byte: `to` renames one as the other.
                if let Val::Expr(v) = &value
                    && (v.ty == Type::C(CKind::Bool) && to == Type::Bool || v.ty == Type::Bool && to == Type::C(CKind::Bool))
                {
                    let Val::Expr(mut v) = value else { unreachable!() };
                    v.ty = to;
                    return Some(Val::Expr(v));
                }
                if !to.is_numeric() {
                    let mut d = Diagnostic::at(&codes::E0101, targs[0].span, format!("`to` converts between number types, not to `{to}`"));
                    if to == Type::Str {
                        d = d.help("`::text()` makes a string from a number");
                    }
                    self.diags.push(d);
                    return None;
                }
                match value {
                    Val::Const(c) => Some(Val::Expr(self.fit_const(&c, to, recv.span, true))),
                    Val::UConst(..) => unreachable!("handled by unit_to"),
                    Val::NoneLit(_) => unreachable!("`none` is not a number"),
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
            "unitText" => {
                no_args(self);
                self.unit_text(Some(value), recv.span, name_span)
            }
            _ => {
                let on = match &value {
                    Val::Const(_) | Val::UConst(..) => "numbers have".to_string(),
                    Val::NoneLit(_) => "`none` has".to_string(),
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
        self.note_method(s, &m, name_span);
        let sname = self.g.structs[s].name.clone();
        let same: Vec<FuncId> = self.g.structs[s].methods.iter().filter(|o| o.name == name && o.operator.is_none()).map(|o| o.func).collect();
        if same.len() > 1 {
            let shown = self.text(recv_span).to_string();
            let results: Vec<String> = same.iter().map(|&f| self.g.sigs[f].ret.map_or("?".to_string(), |t| t.to_string())).collect();
            let mut d = Diagnostic::at(&codes::E0632, name_span, format!("`{sname}` has {} `{name}` methods that differ only in their result, so `{shown}::{name}` cannot choose", same.len()))
                .note(format!("they return {}", results.iter().map(|r| format!("`{r}`")).collect::<Vec<_>>().join(", ")))
                .help("such methods exist to implement interfaces; `for (T x of thing)` picks the one for `T`");
            for &f in &same {
                d = d.label(self.g.sigs[f].span, "declared here");
            }
            self.diags.push(d);
            for a in args {
                self.expr(a);
            }
            return None;
        }
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
        if self.g.ifaces.changing.contains(&m.func) {
            self.changing_call(&recv, name, name_span);
        }
        let visible = self.check_method_visible(s, &m, name_span);
        self.no_targs(name, targs);
        let args = self.args(m.func, args, span);
        let ty = self.g.sigs[m.func].ret?;
        if !visible {
            return None;
        }
        let mut all = vec![recv];
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
            return self.operator_call(s, &m, lhs, rv, ro, span).map(Val::Expr);
        }
        match op {
            BinOp::Ne if self.g.operator(s, BinOp::Eq).is_some() => {
                let m = self.g.operator(s, BinOp::Eq).cloned().unwrap();
                self.operator_call(s, &m, lhs, rv, ro, span).map(|e| Val::Expr(not(e)))
            }
            _ if self.beyond_constraint(s, &format!("`{}` operator", op.symbol()), lo.span.to(span)) => None,
            BinOp::Eq | BinOp::Ne => {
                if let Some(bad) = self.payload_enum_in(t) {
                    self.diags.push(
                        Diagnostic::at(&codes::E0527, span, format!("`{t}` holds a `{bad}`, which has variants with values, so it cannot be compared with `{}`", op.symbol()))
                            .label(lo.span, format!("this is a `{t}`"))
                            .help("compare the parts with `match`, or give the struct its own `==` method"),
                    );
                    return None;
                }
                let rhs = self.into(rv, t, ro)?;
                let rhs = self.copied(rhs);
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

    /// An enum with values somewhere inside `t` (a struct, at any depth), if there is one.
    fn payload_enum_in(&self, t: Type) -> Option<Type> {
        match t {
            Type::Enum { id, .. } => self.g.enums[id].variants.iter().any(|v| !v.fields.is_empty()).then_some(t),
            Type::Struct { id, .. } => self.g.structs[id].fields.iter().filter_map(|f| f.ty).find_map(|ft| self.payload_enum_in(ft)),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn operator_call(&mut self, s: StructId, m: &MethodInfo, lhs: hir::Expr, rv: Val, ro: Origin, span: Span) -> Option<hir::Expr> {
        if !self.check_method_visible(s, m, span) {
            return None;
        }
        let sig = &self.g.sigs[m.func];
        let (param, ty) = (sig.params[0]?, sig.ret?);
        let rhs = self.into(rv, param, ro)?;
        let rhs = self.copied(rhs);
        Some(hir::Expr::new(hir::ExprKind::Call(m.func, vec![lhs, rhs]), ty))
    }

    fn unary(&mut self, op: UnOp, inner: &ast::Expr, span: Span) -> Option<Val> {
        if op == UnOp::Not {
            let v = self.coerce(inner, Type::Bool)?;
            return Some(Val::Expr(hir::Expr::new(hir::ExprKind::Not(Box::new(v)), Type::Bool)));
        }
        let (sym, wants) = if op == UnOp::Neg { ("-", "a number") } else { ("~", "a whole number") };
        let value = self.expr(inner)?;
        let value = match self.unit_unary(op, value, inner.span) {
            Ok(v) => v,
            Err(done) => return done,
        };
        match value {
            Val::UConst(..) => unreachable!(),
            Val::NoneLit(_) => unreachable!("`none` is not a number"),
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
                let mut v = v;
                if let (UnOp::Neg, Type::Int { bits, signed: false }) = (op, v.ty.bare()) {
                    use crate::config::UnsignedNegate;
                    match self.g.settings_at(span).unsigned_negate {
                        UnsignedNegate::Wrap => {}
                        UnsignedNegate::Error => {
                            self.diags.push(Diagnostic::at(&codes::E0101, span, format!("cannot negate `{}` with unsigned_negate = error", v.ty)));
                            return None;
                        }
                        UnsignedNegate::Signed if bits == 128 => {
                            self.diags.push(Diagnostic::at(&codes::E0901, span, "negating uint128 in signed mode needs sint256"));
                            return None;
                        }
                        UnsignedNegate::Signed => {
                            let signed = Type::Int { bits: bits * 2, signed: true };
                            let ty = v.ty.unit().map_or(signed, |unit| signed.with_unit(unit));
                            v = hir::Expr::new(hir::ExprKind::Convert(Box::new(v)), ty).at(span);
                        }
                    }
                }
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

        if units::involved(&lv, &rv) {
            return self.unit_binary(op, lv, lo, rv, ro, span);
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
        // An optional is not an operand: its value may be missing (E0531).
        let rhs_ty = match &rv {
            Val::Expr(r) => Some(r.ty),
            _ => None,
        };
        if optionals::is_optional(self.g, lead) {
            self.optional_as_value(lead, None, lo);
            return None;
        }
        if let Some(r) = rhs_ty.filter(|&r| optionals::is_optional(self.g, r)) {
            self.optional_as_value(r, None, ro);
            return None;
        }

        if lead == Type::Str {
            return self.string_binary(op, lhs, lo, rv, ro);
        }
        if lead.struct_id().is_some() && matches!(class, OpClass::Arithmetic | OpClass::Comparison) {
            return self.struct_operator(op, lhs, lo, rv, ro, span);
        }
        if lead.enum_id().is_some() && matches!(op, BinOp::Eq | BinOp::Ne) {
            return self.enum_compare(op, lhs, lo, rv, ro);
        }
        if (lead.is_list() || lead.is_array()) && matches!(op, BinOp::Eq | BinOp::Ne) {
            return self.list_compare(op, lhs, lo, rv, ro);
        }

        // Two pointers compare their addresses (owner answers, Q14). `*T` and
        // `*fixed T` of one pointee type may be mixed: a comparison writes nothing,
        // so it needs no freeze or thaw. Different pointee types fall through to E0101.
        if matches!(op, BinOp::Eq | BinOp::Ne) && lead.is_ptr() {
            if let Val::Expr(r) = &rv
                && r.ty.is_ptr()
                && r.ty.target() == lead.target()
            {
                let Val::Expr(r) = rv else { unreachable!() };
                let hop = if op == BinOp::Eq { hir::BinOp::Eq } else { hir::BinOp::Ne };
                return Some(Val::Expr(hir::Expr::new(hir::ExprKind::Binary(hop, Box::new(lhs), Box::new(r)), Type::Bool)));
            }
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
                        Val::Const(_) | Val::UConst(..) => "a decimal".to_string(),
                        Val::NoneLit(_) => "`none`".to_string(),
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
            && r.ty.is_numeric()
            && lead.is_numeric()
            && (r.ty.is_c() || lead.is_c())
        {
            let rt = r.ty;
            self.c_types_apart(rt, lead, ro.span);
            return None;
        }
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
            BinOp::Shl | BinOp::Shr | BinOp::And | BinOp::Or | BinOp::Same | BinOp::NotSame => unreachable!(),
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
