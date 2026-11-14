//! Constructors and building values from values (S5B.1).
//!
//! A constructor is a method named after its struct with no return type:
//! `Circle(float64 radius) { self.r = radius; }`. Its `self` starts with no
//! value and the body gives it one field at a time; the function returns it
//! (`check_body`, and the `Check` statements of `hir`). `new Circle of (...)`
//! and the declaration shorthand `Circle c = 5;` choose one with `overload`.
//! With no written constructor the automatic one (fields in order, or by
//! name) builds the value, as before; `Person(auto);` keeps it next to the
//! written ones.

use super::overload::{self, Pick, Rank};
use super::*;

/// A written constructor, as callers see it.
pub struct CtorInfo {
    pub func: FuncId,
    /// The parameter names: named values match these.
    pub names: Vec<String>,
    pub public: bool,
    pub span: Span,
}

/// One way to build a struct: a written constructor, or the automatic one
/// (`func` is `None`, its parameters are the fields in order).
struct Cand {
    func: Option<FuncId>,
    names: Vec<String>,
    params: Vec<Option<Type>>,
    public: bool,
    span: Span,
}

/// `(float64, uint32)`, for messages.
fn sig_text(params: &[Option<Type>]) -> String {
    let parts: Vec<String> = params.iter().map(|t| t.map_or("?".to_string(), |t| t.to_string())).collect();
    format!("({})", parts.join(", "))
}

/// Declares one written constructor of struct `id`.
pub(super) fn declare_ctor(g: &mut Globals, id: StructId, m: &ast::Method, diags: &mut Vec<Diagnostic>) -> Option<FuncId> {
    let f = &m.func;
    let params: Vec<Option<Type>> = f.params.iter().map(|p| g.resolve_binding_type(&p.ty, diags)).collect();
    let clash = g.structs[id].ctors.iter().find(|c| g.sigs[c.func].params == params).map(|c| c.span);
    if let Some(first) = clash {
        diags.push(
            Diagnostic::at(&codes::E0503, f.name_span, format!("`{}` has two constructors taking {}", f.name, sig_text(&params)))
                .note("defined again here")
                .label(first, "first defined here")
                .help("give one of them other parameter types"),
        );
        return None;
    }
    let ret = Some(g.structs[id].ty);
    let (fails, outcome) = failures::signature(g, f, ret, true, diags);
    let sig = Sig { name: f.name.clone(), params, ret, span: f.span, fails, outcome };
    let func = g.add_sig(sig);
    let names = f.params.iter().map(|p| p.name.clone()).collect();
    g.structs[id].ctors.push(CtorInfo { func, names, public: m.public, span: f.name_span });
    Some(func)
}

/// The order in which `values` fill candidate `c`'s parameters: `order[j]`
/// is the index of the value for parameter `j`. `None` if they do not fit
/// its names or count.
fn arrange(c: &Cand, values: &[GivenValue]) -> Option<Vec<usize>> {
    let n = c.params.len();
    if values.len() != n {
        return None;
    }
    if values.iter().all(|v| v.0.is_none()) {
        return Some((0..n).collect());
    }
    let mut order: Vec<Option<usize>> = vec![None; n];
    for (k, v) in values.iter().enumerate() {
        let (name, _) = v.0.as_ref()?;
        let i = c.names.iter().position(|x| x == name)?;
        if order[i].replace(k).is_some() {
            return None;
        }
    }
    order.into_iter().collect()
}

impl Checker<'_> {
    /// `t` built from checked values, when `t` has written constructors:
    /// the best candidate for them, or E0501 / E0502.
    pub(super) fn construct_with_ctors(&mut self, t: Type, mut values: Vec<GivenValue>, span: Span) -> Option<hir::Expr> {
        let s = t.struct_id()?;
        let name = self.g.structs[s].name.clone();
        let mut cands: Vec<Cand> = self.g.structs[s]
            .ctors
            .iter()
            .map(|c| Cand { func: Some(c.func), names: c.names.clone(), params: self.g.sigs[c.func].params.clone(), public: c.public, span: c.span })
            .collect();
        if self.g.structs[s].auto_ctor {
            let info = &self.g.structs[s];
            cands.push(Cand {
                func: None,
                names: info.fields.iter().map(|f| f.name.clone()).collect(),
                params: info.fields.iter().map(|f| f.ty).collect(),
                public: true,
                span: info.name_span,
            });
        }
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
        if values.iter().any(|v| v.1.is_none()) {
            // An error inside a value was reported already.
            return None;
        }
        let mut fitting: Vec<(usize, Vec<usize>, Vec<Rank>)> = Vec::new();
        for (ci, c) in cands.iter().enumerate() {
            let Some(order) = arrange(c, &values) else { continue };
            let ranks: Option<Vec<Rank>> = order
                .iter()
                .enumerate()
                .map(|(j, &k)| {
                    let p = c.params[j]?;
                    self.rank_val(values[k].1.as_ref()?, p)
                })
                .collect();
            if let Some(r) = ranks {
                fitting.push((ci, order, r));
            }
        }
        let ranks: Vec<Vec<Rank>> = fitting.iter().map(|f| f.2.clone()).collect();
        let chosen = match overload::pick(&ranks) {
            Pick::Fit(i) => i,
            Pick::NoFit => {
                self.no_constructor(&name, span, &cands, &values);
                return None;
            }
            Pick::Ambiguous(among) => {
                self.ambiguous_constructors(&name, &cands, &fitting, &among, &values, span);
                return None;
            }
        };
        let (ci, order, _) = fitting.swap_remove(chosen);
        let c = &cands[ci];
        if !c.public && !self.inside(t) {
            let what = if c.func.is_some() { "constructor" } else { "automatic constructor" };
            self.diags.push(
                Diagnostic::at(&codes::E0212, span, format!("the {what} of `{name}` is private"))
                    .note("used outside the struct's methods")
                    .label(c.span, "declared in a private section")
                    .help(self.trusted_help(t, format!("only `{name}`'s own methods can use it; put the constructor under `public:`"))),
            );
            return None;
        }
        let (func, params) = (c.func, c.params.clone());
        let mut exprs = Vec::new();
        let mut ok = true;
        for (j, &k) in order.iter().enumerate() {
            let (_, v, o) = &mut values[k];
            let v = v.take()?;
            match self.into(v, params[j]?, *o) {
                Some(e) => exprs.push(e),
                None => ok = false,
            }
        }
        if !ok {
            return None;
        }
        Some(match func {
            Some(f) => hir::Expr::new(hir::ExprKind::Call(f, exprs), t),
            None => hir::Expr::new(hir::ExprKind::Struct(exprs.into_iter().enumerate().collect()), t),
        })
    }

    fn no_constructor(&mut self, name: &str, span: Span, cands: &[Cand], values: &[GivenValue]) {
        let mut d = Diagnostic::at(&codes::E0501, span, format!("no constructor of `{name}` accepts these values"));
        for (_, v, o) in values {
            let what = match v {
                Some(Val::Expr(x)) => format!("`{}`", x.ty),
                _ => "a number".to_string(),
            };
            d = d.label(o.span, format!("this is {what}"));
        }
        for c in cands {
            let label = match c.func {
                Some(_) => format!("this constructor takes {}", sig_text(&c.params)),
                None => format!("the automatic constructor takes {}", sig_text(&c.params)),
            };
            d = d.label(c.span, label);
        }
        let takes: Vec<String> = cands.iter().map(|c| sig_text(&c.params)).collect();
        self.diags.push(d.help(format!("`{name}` takes {}", takes.join(" or "))));
    }

    fn ambiguous_constructors(&mut self, name: &str, cands: &[Cand], fitting: &[(usize, Vec<usize>, Vec<Rank>)], among: &[usize], values: &[GivenValue], span: Span) {
        let what = if values.len() == 1 { format!("`{}`", self.text(values[0].2.span)) } else { "these values".to_string() };
        let mut d = Diagnostic::at(&codes::E0502, span, format!("more than one constructor of `{name}` fits {what} equally well"));
        for &i in among {
            d = d.label(cands[fitting[i].0].span, "could take them");
        }
        let help = match (values, among.first()) {
            ([v], Some(&i)) => match cands[fitting[i].0].params.iter().flatten().next() {
                Some(t) => format!("convert the value yourself, for example `{}::to<{t}>()`", self.text(v.2.span)),
                None => "convert the value yourself".to_string(),
            },
            _ => "convert the values yourself so only one constructor fits".to_string(),
        };
        self.diags.push(d.help(help));
    }

    /// `new Type of (values)`.
    pub(super) fn new_expr(&mut self, name: &str, name_span: Span, values: &[ast::FieldValue], span: Span) -> Option<Val> {
        if self.g.mono.type_templates.contains_key(name) {
            self.generic_type_alone(name, name_span);
            for v in values {
                self.expr(&v.value);
            }
            return None;
        }
        self.g.note_type_use(name, name_span);
        let Some(&s) = self.g.structs_by_name.get(name) else {
            let mut d = Diagnostic::at(&codes::E0101, name_span, format!("`new` builds a struct, and `{name}` is not one"));
            let structs: Vec<&str> = self.g.structs.iter().map(|s| s.name.as_str()).collect();
            if let Some(near) = similar(name, &structs) {
                d = d.help(format!("did you mean `{near}`?"));
            }
            self.diags.push(d);
            for v in values {
                self.expr(&v.value);
            }
            return None;
        };
        self.new_struct(s, values, span)
    }

    /// `new Type of (values)` for struct `s`.
    pub(super) fn new_struct(&mut self, s: StructId, values: &[ast::FieldValue], span: Span) -> Option<Val> {
        let t = self.g.structs[s].ty;
        // `none` waits for the constructor: it may fit an optional or pointer parameter.
        let given: Vec<GivenValue> = values
            .iter()
            .enumerate()
            .map(|(i, fv)| {
                let v = match fv.value.kind {
                    ExprKind::NoneValue => Some(Val::NoneLit(fv.value.span)),
                    _ => match self.literal_for_field(s, i, &fv.name, &fv.value) {
                        Some(done) => done,
                        None => self.expr_raw(&fv.value),
                    },
                };
                (fv.name.clone(), v, origin(&fv.value))
            })
            .collect();
        self.construct(t, given, span).map(Val::Expr)
    }
}
