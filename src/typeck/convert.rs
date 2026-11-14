//! `convert` (step 7 WP6): the calls the compiler types itself. The parsing functions
//! (`fromHex`, ...) are written in Drachm in `src/stdlib/convert.drh`.
//!
//! - `hex(n)`, `binary(n)`, `octal(n)`: any whole number as text in that base.
//! - `utf8(text)`: the bytes of the text, a `uint8[?]`.

use super::{Checker, Val};
use crate::ast;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, StrFn};
use crate::types::Type;

impl Checker<'_> {
    pub(super) fn convert_call(&mut self, name: &str, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        match name {
            "fromJson" => return self.from_json(targs, args, span),
            "jsonBuild" => return self.json_build(targs, args, span),
            "toJson" => return self.to_json(targs, args, span),
            _ => {}
        }
        if let Some(t) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, t.span, format!("`convert::{name}` takes no type arguments")));
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let [arg] = args else {
            self.diags.push(Diagnostic::at(&codes::E0103, span, format!("`convert::{name}` takes one argument, but {} given", args.len())));
            for a in args {
                self.expr(a);
            }
            return None;
        };
        match name {
            "hex" | "binary" | "octal" => {
                let base = match name {
                    "hex" => 16,
                    "binary" => 2,
                    _ => 8,
                };
                let v = self.expr(arg)?;
                let v = self.materialise(v, arg.span)?;
                if !v.ty.is_int() {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, arg.span, format!("`convert::{name}` needs a whole number, found `{}`", v.ty))
                            .note(format!("this is `{}`", v.ty)),
                    );
                    return None;
                }
                Some(Val::Expr(hir::Expr::new(hir::ExprKind::Radix { value: Box::new(v), base, upper: false }, Type::Str).at(span)))
            }
            "fromHex" | "fromBinary" | "fromOctal" => {
                let base = match name {
                    "fromHex" => 16,
                    "fromBinary" => 2,
                    _ => 8,
                };
                let text = self.coerce(arg, Type::Str)?;
                let fid = *self.g.funcs_by_name.get("convert::digitsValue").expect("convert.drh declares digitsValue");
                let ret = self.g.sigs[fid].ret?;
                let ty = Type::borrow(false, Type::Str);
                let borrow = hir::Expr::new(hir::ExprKind::Borrow { mutable: false, place: Box::new(text) }, ty);
                let call = hir::ExprKind::Call(fid, vec![borrow, super::int_lit(base, Type::Int { bits: 64, signed: false })]);
                Some(Val::Expr(hir::Expr::new(call, ret).at(span)))
            }
            "utf8" => {
                let text = self.coerce(arg, Type::Str)?;
                Some(Val::Expr(hir::Expr::new(hir::ExprKind::StrCall(StrFn::Utf8, vec![text]), StrFn::Utf8.ret()).at(span)))
            }
            _ => {
                self.diags.push(Diagnostic::at(&codes::E0201, span, format!("module `convert` has no function `{name}`")));
                for a in args {
                    self.expr(a);
                }
                None
            }
        }
    }

    /// `convert::toJson(value)`: the JSON text of a struct or a list.
    fn to_json(&mut self, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        if let Some(t) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, t.span, "`convert::toJson` takes no type arguments"));
        }
        let [arg] = args else {
            self.diags.push(Diagnostic::at(&codes::E0103, span, format!("`convert::toJson` takes one argument, but {} given", args.len())));
            for a in args {
                self.expr(a);
            }
            return None;
        };
        let v = self.expr(arg)?;
        let mut x = self.materialise(v, arg.span)?;
        if let Type::Ref { to, .. } = x.ty {
            x = hir::Expr::new(hir::ExprKind::Deref(Box::new(x)), *to);
        }
        let shape = self.json_shape_of("toJson", x.ty, arg.span, false)?;
        let args = vec![x, super::str_lit(shape)];
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Json { write: true, args }, Type::Str).at(span)))
    }

    /// `convert::fromJson<T>(text)`: calls the generic reader for `T`, with the text borrowed.
    fn from_json(&mut self, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        let [target] = targs else {
            self.diags.push(
                Diagnostic::at(&codes::E0103, span, "`convert::fromJson` needs the type to read")
                    .help("for example `convert::fromJson<Player>(text)`"),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        };
        let [arg] = args else {
            self.diags.push(Diagnostic::at(&codes::E0103, span, format!("`convert::fromJson` takes one argument, but {} given", args.len())));
            for a in args {
                self.expr(a);
            }
            return None;
        };
        let ty = self.g.resolve(target, self.diags)?;
        self.json_shape_of("fromJson", ty, target.span, true)?;
        let text = self.coerce(arg, Type::Str)?;
        let fid = self.instantiate_fn("convert::jsonRead", vec![ty], span)?;
        let ret = self.g.sigs[fid].ret?;
        let borrow = hir::Expr::new(hir::ExprKind::Borrow { mutable: false, place: Box::new(text) }, Type::borrow(false, Type::Str));
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Call(fid, vec![borrow]), ret).at(span)))
    }

    /// `jsonBuild<T>(text, kind, path, detail)`, inside `jsonRead`: the runtime reads the text.
    fn json_build(&mut self, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        let [target] = targs else { return None };
        let ty = self.g.resolve(target, self.diags)?;
        let shape = self.json_shape_of("fromJson", ty, span, true)?;
        let mut es = Vec::new();
        for a in args {
            es.push(self.coerce(a, Type::Str)?);
        }
        es.push(super::str_lit(shape));
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Json { write: false, args: es }, ty).at(span)))
    }

    /// The runtime's description of `ty` (see `runtime/strings/src/json.rs`), or E0901 for a
    /// type JSON cannot hold yet. `top` is a type being read, which must be a struct or a list.
    fn json_shape_of(&mut self, what: &str, ty: Type, span: Span, top: bool) -> Option<String> {
        let ty = ty.bare();
        let reason = if top && !(ty.struct_id().is_some() || ty.is_list()) {
            Err(format!("`{ty}`, which is not a struct or a list"))
        } else {
            self.json_shape(ty, &mut Vec::new())
        };
        match reason {
            Ok(s) => Some(s),
            Err(why) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, span, format!("`convert::{what}` cannot handle {why} yet"))
                        .help("JSON works for structs and lists of whole numbers, floats, `bool`, `string`, optionals, lists and structs of those"),
                );
                None
            }
        }
    }

    fn json_shape(&self, ty: Type, stack: &mut Vec<usize>) -> Result<String, String> {
        let ty = ty.bare();
        if let Some(inner) = super::optionals::inner_of(self.g, ty) {
            let (size, _) = self.g.size_align(Some(ty));
            let (_, align) = self.g.size_align(Some(inner));
            let off = hir::tag_size_for(2).next_multiple_of(align);
            return Ok(format!("o{size}.{off}({})", self.json_shape(inner, stack)?));
        }
        match ty {
            Type::Int { bits, signed } => Ok(format!("i{}{bits}", if signed { 's' } else { 'u' })),
            Type::Float(bits) => Ok(format!("f{bits}")),
            Type::Bool => Ok("b".to_string()),
            Type::Str => Ok("t".to_string()),
            _ if ty.is_list() => {
                let elem = ty.elem().expect("a list has an item type");
                let (size, align) = self.g.size_align(Some(elem));
                Ok(format!("l{size}.{align}({})", self.json_shape(elem, stack)?))
            }
            Type::Struct { id, name } => {
                let s = &self.g.structs[id];
                if s.union_layout.is_some() {
                    return Err(format!("the C union `{name}`"));
                }
                if stack.contains(&id) {
                    return Err(format!("`{name}`, which contains itself"));
                }
                stack.push(id);
                let sizes: Vec<(u32, u32)> = s.fields.iter().map(|f| self.g.size_align(f.ty)).collect();
                let (offsets, size, _) = hir::layout(&sizes);
                let mut out = format!("r{size}{{");
                for (f, off) in s.fields.iter().zip(offsets) {
                    let Some(ft) = f.ty else { return Err(format!("`{name}`, which has a field of unknown type")) };
                    out.push_str(&format!("{}@{off}={},", f.name, self.json_shape(ft, stack)?));
                }
                stack.pop();
                out.push('}');
                Ok(out)
            }
            Type::Map { .. } => Err("a map".to_string()),
            Type::Enum { name, .. } => Err(format!("the enum `{name}`")),
            _ if ty.is_array() => Err("a fixed-size array".to_string()),
            _ => Err(format!("`{ty}`")),
        }
    }
}
