//! C declarations as Drachm declarations (step 7.1).
//!
//! The resolver gives every `alias::name` of a C import a program-wide name
//! (`alias::name`), asks this module whether the C declaration can be used,
//! and at the end asks it to write the declarations the program used as
//! ordinary syntax-tree items: functions with a C symbol and no body, and
//! structs with the C layout. The checker and both backends then see C names
//! as part of the program; what makes a call foreign is the symbol.
//!
//! Only what the program names is brought in, and the types it mentions. A
//! declaration the compiler cannot use yet is E0901 at its use, with the
//! reason found while reading the header.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::ast::{self, Dim, TypeMod, TypeName};
use crate::cimport::{CRecord, CSig, CType, CUnit, CValue, Decl};
use crate::diag::{codes, Diagnostic, Span};
use crate::types::CKind;

/// What `alias::name` stands for in an expression.
pub enum Member {
    /// A function or a type: used by its program-wide name.
    Item { global: String, func: bool },
    /// A constant macro: the value itself.
    Value(CValue),
    /// A named value of a C enum.
    Enumerator { enum_name: String, value: i128 },
}

#[derive(Default)]
struct State {
    wanted: Vec<(usize, String)>,
    seen: HashSet<(usize, String)>,
    macros: HashMap<(usize, String), Option<CValue>>,
}

pub struct CCtx {
    pub units: Vec<CUnit>,
    /// The program-wide prefix of each unit: the alias of its first import.
    pub prefix: Vec<String>,
    /// Where each unit was first imported.
    pub spans: Vec<Span>,
    state: RefCell<State>,
}

impl CCtx {
    pub fn new(units: Vec<CUnit>, prefix: Vec<String>, spans: Vec<Span>) -> CCtx {
        CCtx { units, prefix, spans, state: RefCell::new(State::default()) }
    }

    fn global(&self, k: usize, name: &str) -> String {
        format!("{}::{name}", self.prefix[k])
    }

    fn want(&self, k: usize, name: &str) {
        let mut st = self.state.borrow_mut();
        if st.seen.insert((k, name.to_string())) {
            st.wanted.push((k, name.to_string()));
        }
    }

    fn unusable(&self, k: usize, name: &str, why: &str, at: Span) -> Diagnostic {
        Diagnostic::at(&codes::E0901, at, format!("`{}::{name}` cannot be used yet: {why}", self.prefix[k]))
            .note(format!("declared in the C header `{}`", self.units[k].header))
    }

    /// Look `name` up in the unit and check it is usable.
    pub fn member(&self, k: usize, name: &str, at: Span, diags: &mut Vec<Diagnostic>) -> Option<Member> {
        let unit = &self.units[k];
        let Some(decl) = unit.decls.get(name) else {
            let known: Vec<&str> = unit.decls.keys().map(String::as_str).collect();
            let mut d = Diagnostic::at(&codes::E0703, at, format!("the C header `{}` has no `{name}`", unit.header)).note("no such name");
            if let Some(near) = crate::config::similar(name, &known) {
                d = d.help(format!("did you mean `{near}`?"));
            } else {
                d = d.help("C names are written exactly as the header spells them, and the header must declare them (or include a header that does)");
            }
            diags.push(d);
            return None;
        };
        match decl {
            Decl::Func { sig, inline } => {
                if *inline {
                    diags.push(self.unusable(k, name, "it is a `static inline` function, which has no symbol to call", at));
                    return None;
                }
                if let Err(why) = self.lower_sig(k, sig, &mut Vec::new()) {
                    diags.push(self.unusable(k, name, &why, at));
                    return None;
                }
                self.want(k, name);
                Some(Member::Item { global: self.global(k, name), func: true })
            }
            Decl::Record(r) => {
                if let Some(why) = &r.unsupported {
                    diags.push(self.unusable(k, name, why, at));
                    return None;
                }
                self.want(k, name);
                Some(Member::Item { global: self.global(k, name), func: false })
            }
            Decl::Alias(_) => Some(Member::Item { global: self.global(k, name), func: false }),
            Decl::Enum(_) => {
                self.want(k, name);
                Some(Member::Item { global: self.global(k, name), func: false })
            }
            // The values of an enum with no name are plain numbers.
            Decl::Enumerator { value, of } if of.is_empty() => Some(Member::Value(CValue::Int(*value))),
            Decl::Enumerator { value, of } => {
                self.want(k, of);
                Some(Member::Enumerator { enum_name: self.global(k, of), value: *value })
            }
            Decl::Macro(m) => {
                if m.function_like {
                    diags.push(
                        self.unusable(k, name, "it is a function-like macro", at)
                            .help(format!("declare its types first: `from c macro {name}(...) gives T;`")),
                    );
                    return None;
                }
                match self.macro_value(k, name) {
                    Some(v) => Some(Member::Value(v)),
                    None => {
                        diags.push(
                            self.unusable(k, name, "it is a macro that is not a constant number or string", at)
                                .help("only macros that evaluate to a number or a string come in as constants"),
                        );
                        None
                    }
                }
            }
            Decl::Unsupported(why) => {
                diags.push(self.unusable(k, name, why, at));
                None
            }
        }
    }

    fn macro_value(&self, k: usize, name: &str) -> Option<CValue> {
        let key = (k, name.to_string());
        if let Some(v) = self.state.borrow().macros.get(&key) {
            return v.clone();
        }
        let value = crate::cimport::eval_macros(&self.units[k], &[name.to_string()]).ok().and_then(|mut m| m.remove(name));
        self.state.borrow_mut().macros.insert(key, value.clone());
        value
    }

    /// A typedef written as a type (`sdl::Uint32`): what it stands for.
    pub fn alias_type(&self, k: usize, name: &str, span: Span) -> Option<TypeName> {
        let Some(Decl::Alias(t)) = self.units[k].decls.get(name) else { return None };
        self.lower_type(k, t, span, &mut Vec::new()).ok()
    }

    // ---- types ----

    fn plain(&self, name: impl Into<String>, span: Span) -> TypeName {
        TypeName::plain(name, span)
    }

    /// The Drachm type written for a C type. `found` collects the records it names.
    fn lower_type(&self, k: usize, t: &CType, at: Span, found: &mut Vec<String>) -> Result<TypeName, String> {
        let synthetic = Span::new(at.start, at.start);
        match t {
            CType::Void => Ok(self.plain("none", synthetic)),
            CType::Scalar(kind) => Ok(self.plain(format!("c::{}", kind.word()), synthetic)),
            CType::Ptr(inner) => {
                let mut pointee = match &**inner {
                    CType::Void => self.plain("none", synthetic),
                    // Bytes behind a pointer are bytes: `char *` is `foreign *uint8`.
                    CType::Scalar(CKind::Char | CKind::UChar) => self.plain("uint8", synthetic),
                    CType::Scalar(CKind::SChar) => self.plain("sint8", synthetic),
                    CType::Record(name) => {
                        // A pointer to a record whose fields cannot be mapped is still
                        // a pointer: the record is then opaque.
                        found.push(name.clone());
                        self.plain(self.global(k, name), synthetic)
                    }
                    other => self.lower_type(k, other, at, found)?,
                };
                if !pointee.dims.is_empty() {
                    return Err("a pointer to an array".into());
                }
                pointee.mods.insert(0, (TypeMod::ForeignPtr, synthetic));
                Ok(pointee)
            }
            CType::Record(name) => match self.units[k].decls.get(name) {
                Some(Decl::Record(r)) if r.opaque => Err(format!("`{name}` is an incomplete type; only pointers to it exist")),
                Some(Decl::Record(r)) => match &r.unsupported {
                    Some(why) => Err(format!("`{name}`: {why}")),
                    None => {
                        found.push(name.clone());
                        Ok(self.plain(self.global(k, name), synthetic))
                    }
                },
                _ => Err(format!("`{name}` is not a struct the compiler could read")),
            },
            CType::Enum(name) => match self.units[k].decls.get(name) {
                Some(Decl::Enum(_)) => {
                    found.push(name.clone());
                    Ok(self.plain(self.global(k, name), synthetic))
                }
                _ => Err(format!("`{name}` is not an enum the compiler could read")),
            },
            CType::Array(elem, n) => {
                let mut e = self.lower_type(k, elem, at, found)?;
                if !e.mods.is_empty() || !e.dims.is_empty() {
                    return Err("an array of pointers or of arrays".into());
                }
                e.dims.push(Dim::Array(*n, synthetic));
                Ok(e)
            }
            CType::FuncPtr(_) => Err("function pointers are not supported yet".into()),
            CType::Unsupported(why) => Err(why.clone()),
        }
    }

    fn lower_sig(&self, k: usize, sig: &CSig, found: &mut Vec<String>) -> Result<(TypeName, Vec<TypeName>), String> {
        let at = self.spans[k];
        if sig.variadic {
            return Err("it takes a variable number of arguments, which are not supported yet".into());
        }
        let by_value = |t: &CType| matches!(t, CType::Record(_));
        if by_value(&sig.ret) || sig.params.iter().any(|(_, t)| by_value(t)) {
            return Err("passing a C struct by value is not supported yet".into());
        }
        let ret = self.lower_type(k, &sig.ret, at, found).map_err(|e| format!("its result: {e}"))?;
        let mut params = Vec::new();
        for (name, t) in &sig.params {
            params.push(self.lower_type(k, t, at, found).map_err(|e| format!("its parameter `{name}`: {e}"))?);
        }
        Ok((ret, params))
    }

    // ---- writing the declarations ----

    /// The declarations of everything the program used, as syntax-tree items.
    pub fn synthesize(&self, program: &mut ast::Program) {
        let mut i = 0;
        loop {
            let next = self.state.borrow().wanted.get(i).cloned();
            let Some((k, name)) = next else { break };
            i += 1;
            let at = self.spans[k];
            let synthetic = Span::new(at.start, at.start);
            match self.units[k].decls.get(&name) {
                Some(Decl::Func { sig, .. }) => {
                    let mut found = Vec::new();
                    let Ok((ret, params)) = self.lower_sig(k, sig, &mut found) else { continue };
                    for r in found {
                        self.want(k, &r);
                    }
                    program.funcs.push(ast::Func {
                        ret,
                        name: self.global(k, &name),
                        name_span: synthetic,
                        tparams: Vec::new(),
                        params: sig
                            .params
                            .iter()
                            .zip(params)
                            .map(|((n, _), ty)| ast::Param { mutable: false, open: false, ty, name: n.clone(), span: synthetic })
                            .collect(),
                        body: ast::Block { stmts: Vec::new(), end: synthetic },
                        is_unsafe: Some(synthetic),
                        lives: Vec::new(),
                        lives_span: None,
                        lives_from: false,
                        fails: Vec::new(),
                        fails_span: None,
                        span: synthetic,
                        foreign: Some(name.clone()),
                    });
                }
                Some(Decl::Record(r)) => {
                    let global = self.global(k, &name);
                    let mut fields = Vec::new();
                    if !r.opaque && r.unsupported.is_none() {
                        for f in &r.fields {
                            let mut found = Vec::new();
                            let Ok(ty) = self.lower_type(k, &f.ty, at, &mut found) else { continue };
                            for r in found {
                                self.want(k, &r);
                            }
                            fields.push(ast::Field { public: true, mutable: true, ty, name: f.name.clone(), name_span: synthetic });
                        }
                    }
                    program.structs.push(record_struct(r, global, fields, synthetic));
                }
                Some(Decl::Enum(e)) => program.c_enums.push((self.global(k, &name), e.underlying)),
                _ => {}
            }
        }
    }
}

fn record_struct(_r: &CRecord, name: String, fields: Vec<ast::Field>, span: Span) -> ast::Struct {
    ast::Struct {
        name,
        name_span: span,
        tparams: Vec::new(),
        span,
        with: Vec::new(),
        lives: Vec::new(),
        stays: None,
        defaults: None,
        auto_ctor: None,
        fields,
        methods: Vec::new(),
    }
}

/// The literal expression for a macro's value.
pub fn value_expr(v: &CValue, span: Span) -> ast::Expr {
    use num_bigint::BigInt;
    let kind = match v {
        CValue::Int(i) => ast::ExprKind::Num(ast::NumLit { value: crate::consts::Const::Int(BigInt::from(*i)), suffix: None, unit: None }),
        CValue::Float(f) => {
            let r = num_rational::BigRational::from_float(*f).unwrap_or_default();
            ast::ExprKind::Num(ast::NumLit { value: crate::consts::Const::Dec(r), suffix: None, unit: None })
        }
        CValue::Str(s) => ast::ExprKind::Str(vec![ast::StrPart::Raw(s.clone(), span)]),
    };
    ast::Expr { kind, span, paren: false }
}
