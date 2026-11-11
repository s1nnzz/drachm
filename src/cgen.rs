//! The optimising backend: `hir` → C, handed to the system C compiler.
//!
//! The C is not meant to be read. Every sub-expression becomes its own
//! `const` temporary, in evaluation order, which pins down left-to-right
//! evaluation (C leaves it open) and costs nothing once the C compiler has
//! run. All arithmetic is done in unsigned types so that overflow wraps as
//! Drachm defines it instead of being undefined behaviour.
//!
//! Strings are `drachm_str` structs and Drachm structs are C structs (fields
//! `f_<name>`). Both are passed to and returned from Drachm functions by
//! value (the callee owns its parameters), except a method's `self`, which
//! is a pointer to the caller's value; strings go to the runtime by pointer.
//! Each struct that owns strings gets `drachm_copy_<Name>` (a deep copy) and
//! `drachm_drop_<Name>`.

use std::fmt::Write;

use crate::hir::*;
use crate::types::Type;

const RUNTIME_C: &str = include_str!("../runtime/runtime.c");
const STRINGS_H: &str = include_str!("../runtime/strings.h");

pub fn generate(prog: &Program) -> String {
    let mut out = String::from("#define DRACHM_INLINE 1\n");
    out += &RUNTIME_C.replace("#include \"strings.h\"\n", STRINGS_H);
    out += "\nstatic inline double drachm_f64_bits(uint64_t b) { double d; memcpy(&d, &b, 8); return d; }\n";
    out += "static inline float drachm_f32_bits(uint32_t b) { float f; memcpy(&f, &b, 4); return f; }\n\n";

    for s in struct_order(prog) {
        let def = &prog.structs[s];
        let name = struct_name(&def.name);
        let _ = writeln!(out, "typedef struct {{");
        for f in &def.fields {
            let _ = writeln!(out, "    {} f_{};", c_type(f.ty), f.name);
        }
        let _ = writeln!(out, "}} {name};\n");
        if def.owns {
            // The bits first, then a new copy of each string inside.
            let _ = writeln!(out, "static void drachm_copy_{}({name} *d, const {name} *s) {{\n    *d = *s;", def.name);
            for f in def.fields.iter().filter(|f| prog.owns(f.ty)) {
                let _ = writeln!(out, "    {}(&d->f_{n}, &s->f_{n});", copy_fn(f.ty), n = f.name);
            }
            let _ = writeln!(out, "}}\n\nstatic void drachm_drop_{}({name} *p) {{", def.name);
            for f in def.fields.iter().filter(|f| prog.owns(f.ty)) {
                let _ = writeln!(out, "    {}(&p->f_{});", drop_fn(f.ty), f.name);
            }
            let _ = writeln!(out, "}}\n");
        }
    }
    for f in &prog.funcs {
        let _ = writeln!(out, "{};", signature(f));
    }
    for f in &prog.funcs {
        let mut g = Gen { prog, func: f, out: String::new(), tmp: 0, depth: 1 };
        for (i, l) in f.locals.iter().enumerate().skip(f.params) {
            // Owning values start empty, which is what `Drop` relies on.
            // Other structs start out unset: the checker rejects reads before assignment.
            if prog.owns(l.ty) {
                g.line(format!("{} l{i} = {{0}};", c_type(l.ty)));
            } else if l.ty.struct_id().is_some() {
                g.line(format!("{} l{i};", c_type(l.ty)));
            } else {
                g.line(format!("{} l{i} = 0;", c_type(l.ty)));
            }
        }
        g.block(&f.body);
        let _ = writeln!(out, "\n{} {{\n{}}}", signature(f), g.out);
    }

    let main = &prog.funcs[prog.main];
    let call = format!("{}()", main.symbol);
    let body = if main.ret == Type::None { format!("{call}; return 0;") } else { format!("return (int){call};") };
    let _ = writeln!(out, "\nint main(void) {{ {body} }}");
    out
}

fn struct_name(name: &str) -> String {
    format!("drachm_s_{name}")
}

/// The deep copy function for an owning type: `f(&dest, &src)`.
fn copy_fn(t: Type) -> String {
    match t {
        Type::Struct { name, .. } => format!("drachm_copy_{name}"),
        _ => "drachm_str_copy".to_string(),
    }
}

/// The drop function for an owning type: `f(&value)`.
fn drop_fn(t: Type) -> String {
    match t {
        Type::Struct { name, .. } => format!("drachm_drop_{name}"),
        _ => "drachm_str_drop".to_string(),
    }
}

/// Every struct, each after the structs it contains (C needs a struct's
/// definition before it is used as a field).
fn struct_order(prog: &Program) -> Vec<StructId> {
    fn visit(prog: &Program, s: StructId, done: &mut [bool], order: &mut Vec<StructId>) {
        if done[s] {
            return;
        }
        done[s] = true;
        for f in &prog.structs[s].fields {
            if let Some(inner) = f.ty.struct_id() {
                visit(prog, inner, done, order);
            }
        }
        order.push(s);
    }
    let mut done = vec![false; prog.structs.len()];
    let mut order = Vec::new();
    for s in 0..prog.structs.len() {
        visit(prog, s, &mut done, &mut order);
    }
    order
}

/// Parameters are passed by value, except a method's `self`, which is a
/// pointer to the caller's value. Results are returned by value.
fn signature(f: &Func) -> String {
    let params: Vec<String> = f.locals[..f.params]
        .iter()
        .enumerate()
        .map(|(i, l)| if i == 0 && f.has_self { format!("{} *l0", c_type(l.ty)) } else { format!("{} l{i}", c_type(l.ty)) })
        .collect();
    let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };
    format!("static {} {}({params})", c_type(f.ret), f.symbol)
}

fn c_type(t: Type) -> String {
    let name = match t {
        Type::Int { bits: 8, signed: false } => "uint8_t",
        Type::Int { bits: 16, signed: false } => "uint16_t",
        Type::Int { bits: 32, signed: false } => "uint32_t",
        Type::Int { bits: 64, signed: false } => "uint64_t",
        Type::Int { bits: _, signed: false } => "drachm_u128",
        Type::Int { bits: 8, signed: true } => "int8_t",
        Type::Int { bits: 16, signed: true } => "int16_t",
        Type::Int { bits: 32, signed: true } => "int32_t",
        Type::Int { bits: 64, signed: true } => "int64_t",
        Type::Int { bits: _, signed: true } => "drachm_s128",
        Type::Float(32) => "float",
        Type::Float(_) => "double",
        Type::Bool => "uint8_t",
        Type::Str => "drachm_str",
        Type::None => "void",
        Type::Struct { name, .. } => return struct_name(name),
        Type::Ref { to, .. } | Type::Ptr { to } => return format!("{} *", c_type(*to)),
    };
    name.to_string()
}

fn bits(t: Type) -> u16 {
    match t {
        Type::Int { bits, .. } => bits,
        _ => unreachable!("not an integer type"),
    }
}

fn signed(t: Type) -> bool {
    matches!(t, Type::Int { signed: true, .. })
}

/// The unsigned type integer arithmetic on `t` is carried out in: at least
/// 32 bits, so that C's promotion to (signed) `int` never gets involved.
fn work_type(t: Type) -> &'static str {
    match bits(t) {
        8 | 16 | 32 => "uint32_t",
        64 => "uint64_t",
        _ => "drachm_u128",
    }
}

/// `v`'s low and high 64-bit halves, sign- or zero-extended from its own type.
fn halves(v: &str, t: Type) -> String {
    let wide = if signed(t) { "drachm_s128" } else { "drachm_u128" };
    format!("(uint64_t)({v}), (uint64_t)((({wide})({v})) >> 64)")
}

/// A literal as a `drachm_str` initialiser: a not-owned view of read-only bytes.
fn c_str_value(s: &str) -> String {
    if s.is_empty() { "{0}".to_string() } else { format!("{{(uint8_t *){}, {}, 0}}", c_string(s), s.len()) }
}

fn c_string(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => out += "\\\"",
            b'\\' => out += "\\\\",
            // `?` is escaped to keep trigraphs out of it.
            0x20..=0x7e if b != b'?' => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push('"');
    out
}

struct Gen<'a> {
    prog: &'a Program,
    func: &'a Func,
    out: String,
    tmp: usize,
    depth: usize,
}

impl Gen<'_> {
    /// A local as a C lvalue (`self` is a pointer).
    fn local(&self, id: LocalId) -> String {
        if id == 0 && self.func.has_self { "(*l0)".to_string() } else { format!("l{id}") }
    }

    /// A place (local, then field indexes) as a C lvalue.
    fn place_lvalue(&self, place: &Place) -> String {
        let mut s = self.local(place.local);
        let mut ty = self.func.locals[place.local].ty;
        for i in place.fields() {
            let f = &self.prog.struct_def(ty).fields[i];
            let _ = write!(s, ".f_{}", f.name);
            ty = f.ty;
        }
        s
    }

    /// The type of a place.
    fn place_type(&self, place: &Place) -> Type {
        let mut ty = self.func.locals[place.local].ty;
        for i in place.fields() {
            ty = self.prog.struct_def(ty).fields[i].ty;
        }
        ty
    }

    /// A place-shaped expression (`Expr::is_place`) as a C lvalue, emitting
    /// what a `Temp` inside it makes.
    fn lvalue(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Local(id) => self.local(*id),
            ExprKind::Temp(id, inner) => {
                let dest = format!("l{id}");
                self.into(inner, &dest);
                dest
            }
            ExprKind::Field(inner, i) => {
                let base = self.lvalue(inner);
                format!("{base}.f_{}", self.prog.struct_def(inner.ty).fields[*i].name)
            }
            _ => unreachable!("not a place"),
        }
    }

    fn line(&mut self, text: impl AsRef<str>) {
        for _ in 0..self.depth {
            self.out += "    ";
        }
        self.out += text.as_ref();
        self.out.push('\n');
    }

    /// A jump target. The empty statement after it lets a declaration follow.
    fn label(&mut self, name: String) {
        self.line(format!("{name}: ;"));
    }

    /// Store `value` in a fresh temporary and return its name. Struct
    /// temporaries are not `const`, so that a method can be called on one.
    fn temp(&mut self, ty: Type, value: impl AsRef<str>) -> String {
        self.tmp += 1;
        let name = format!("t{}", self.tmp);
        let qualifier = if ty.struct_id().is_some() { "" } else { "const " };
        self.line(format!("{qualifier}{} {name} = {};", c_type(ty), value.as_ref()));
        name
    }

    fn block(&mut self, b: &Block) {
        for s in b {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Assign(place, e) if self.prog.owns(e.ty) => {
                let target = self.place_lvalue(place);
                self.into(e, &target);
            }
            Stmt::Assign(place, e) => {
                let v = self.expr(e);
                let target = self.place_lvalue(place);
                self.line(format!("{target} = {v};"));
            }
            Stmt::Decl(_) => {}
            Stmt::Unsafe(b) => self.block(b),
            Stmt::Destroy(_) => unreachable!("destroy cannot be built yet"),
            Stmt::Drop(place) => {
                let target = self.place_lvalue(place);
                let f = drop_fn(self.place_type(place));
                self.line(format!("{f}(&{target});"));
            }
            // Jumps are `goto`s to labels named after the HIR label:
            // `ifL_bN` is the test of branch N, `ifL_end` the way out.
            Stmt::If { label, branches } => {
                for (i, br) in branches.iter().enumerate() {
                    if i > 0 {
                        self.label(format!("if{label}_b{i}"));
                    }
                    let open = match &br.test {
                        Some(t) => {
                            let c = self.expr(t);
                            format!("if ({c}) {{")
                        }
                        None => "{".to_string(),
                    };
                    self.line(open);
                    self.depth += 1;
                    self.block(&br.body);
                    self.line(format!("goto if{label}_end;"));
                    self.depth -= 1;
                    self.line("}");
                }
                self.label(format!("if{label}_end"));
            }
            // `loopL_next` is the step (where `continue` goes), `loopL_end` the way out.
            Stmt::Loop { label, cond, body, step, .. } => {
                // The condition's temporaries must be recomputed each time round.
                self.line("for (;;) {");
                self.depth += 1;
                if let Some(cond) = cond {
                    let c = self.expr(cond);
                    self.line(format!("if (!{c}) break;"));
                }
                self.block(body);
                self.label(format!("loop{label}_next"));
                self.block(step);
                self.depth -= 1;
                self.line("}");
                self.label(format!("loop{label}_end"));
            }
            Stmt::Jump(j) => {
                let target = match *j {
                    Jump::Break(l) => format!("loop{l}_end"),
                    Jump::Continue(l) => format!("loop{l}_next"),
                    Jump::Leave(l) => format!("if{l}_end"),
                    Jump::Next(l, n) => format!("if{l}_b{n}"),
                };
                self.line(format!("goto {target};"));
            }
            Stmt::Return(None) => self.line("return;"),
            Stmt::Return(Some(e)) if self.prog.owns(e.ty) => {
                let r = self.fresh("r");
                self.line(format!("{} {r};", c_type(e.ty)));
                self.into(e, &r);
                self.line(format!("return {r};"));
            }
            Stmt::Return(Some(e)) => {
                let v = self.expr(e);
                self.line(format!("return {v};"));
            }
            Stmt::Expr(e) => {
                self.expr(e);
            }
        }
    }

    /// Emit the statements that compute `e`; returns a C expression for its
    /// value that is safe to use any number of times (a temporary or a literal).
    /// Not for strings or structs that own strings: those are made with
    /// `into` or used with `place`.
    fn expr(&mut self, e: &Expr) -> String {
        let ty = e.ty;
        match &e.kind {
            ExprKind::Int(v) => {
                let (lo, hi) = (*v as u64, (*v >> 64) as u64);
                let t = c_type(ty);
                match bits(ty) {
                    128 => format!("(({t})(((drachm_u128)UINT64_C({hi}) << 64) | UINT64_C({lo})))"),
                    _ => format!("(({t})UINT64_C({lo}))"),
                }
            }
            ExprKind::Float(v) => match ty {
                Type::Float(32) => format!("drachm_f32_bits(UINT32_C({}))", (*v as f32).to_bits()),
                _ => format!("drachm_f64_bits(UINT64_C({}))", v.to_bits()),
            },
            ExprKind::Bool(b) => format!("((uint8_t){})", *b as u8),
            ExprKind::Str(_)
            | ExprKind::Text(_)
            | ExprKind::Radix { .. }
            | ExprKind::Copy(_)
            | ExprKind::Move(_)
            | ExprKind::Temp(..)
            | ExprKind::Join(_) => unreachable!("owning values are made with `into` or used with `place`"),
            ExprKind::Scoped(inner, drops) => {
                let v = self.expr(inner);
                self.drops(drops);
                v
            }
            ExprKind::StrCall(f, args) => {
                let args = self.runtime_args(f.params(), args);
                let call = format!("{}({})", f.name(), args.join(", "));
                if ty == Type::None {
                    self.line(format!("{call};"));
                    String::new()
                } else {
                    self.temp(ty, call)
                }
            }
            // A copy, so that a later `++` or method call cannot change what was read here.
            ExprKind::Local(_) => {
                let v = self.lvalue(e);
                self.temp(ty, v)
            }
            ExprKind::Field(inner, i) => {
                let from = if inner.is_place() { self.lvalue(inner) } else { self.expr(inner) };
                let name = &self.prog.struct_def(inner.ty).fields[*i].name;
                self.temp(ty, format!("{from}.f_{name}"))
            }
            ExprKind::Struct(fields) => {
                let def = self.prog.struct_def(ty);
                let mut parts = Vec::new();
                for (i, value) in fields {
                    let v = self.expr(value);
                    parts.push(format!(".f_{} = {v}", def.fields[*i].name));
                }
                self.temp(ty, format!("{{ {} }}", parts.join(", ")))
            }
            ExprKind::Neg(inner) => {
                let v = self.expr(inner);
                if ty.is_float() {
                    self.temp(ty, format!("-{v}"))
                } else {
                    self.temp(ty, format!("({})(0 - ({}){v})", c_type(ty), work_type(ty)))
                }
            }
            ExprKind::Not(inner) => {
                let v = self.expr(inner);
                self.temp(ty, format!("!{v}"))
            }
            ExprKind::BitNot(inner) => {
                let v = self.expr(inner);
                self.temp(ty, format!("({})~({}){v}", c_type(ty), work_type(ty)))
            }
            ExprKind::Binary(op, l, r) => {
                let operand = l.ty;
                let a = self.expr(l);
                let b = self.expr(r);
                self.binary(*op, operand, ty, &a, &b)
            }
            ExprKind::Shift { left, value, amount } => {
                let v = self.expr(value);
                let n = self.expr(amount);
                let (t, w) = (c_type(ty), work_type(ty));
                let n = format!("((uint32_t)({n}) & {})", bits(ty) - 1);
                if *left {
                    self.temp(ty, format!("({t})(({w}){v} << {n})"))
                } else {
                    // On a signed value this is an arithmetic shift (gcc and clang both define it so).
                    self.temp(ty, format!("({t})({v} >> {n})"))
                }
            }
            ExprKind::IntPow(base, exp) => {
                let b = self.expr(base);
                let x = self.expr(exp);
                let negative = if signed(exp.ty) { format!("({x} < 0)") } else { "0".into() };
                let base_signed = signed(ty) as u8;
                if bits(ty) <= 64 && bits(exp.ty) <= 64 {
                    self.temp(ty, format!("({})drachm_ipow64((uint64_t){b}, (uint64_t){x}, {negative}, {base_signed})", c_type(ty)))
                } else {
                    self.tmp += 1;
                    let p = format!("p{}", self.tmp);
                    self.line(format!(
                        "const drachm_pair {p} = drachm_ipow128({}, {}, {negative}, {base_signed});",
                        halves(&b, ty),
                        halves(&x, exp.ty)
                    ));
                    self.temp(ty, format!("({})drachm_join({p}.lo, {p}.hi)", c_type(ty)))
                }
            }
            ExprKind::Logic { and, lhs, rhs } => {
                let a = self.expr(lhs);
                self.tmp += 1;
                let name = format!("t{}", self.tmp);
                self.line(format!("uint8_t {name} = {a};"));
                self.line(format!("if ({}{name}) {{", if *and { "" } else { "!" }));
                self.depth += 1;
                let b = self.expr(rhs);
                self.line(format!("{name} = {b};"));
                self.depth -= 1;
                self.line("}");
                name
            }
            ExprKind::Convert(inner) => {
                let from = inner.ty;
                let v = self.expr(inner);
                let t = c_type(ty);
                match (from, ty) {
                    (Type::Float(_), Type::Int { bits: 128, .. }) => {
                        self.tmp += 1;
                        let p = format!("p{}", self.tmp);
                        self.line(format!("const drachm_pair {p} = drachm_f64_to_u128((double){v});"));
                        self.temp(ty, format!("({t})drachm_join({p}.lo, {p}.hi)"))
                    }
                    (Type::Float(_), Type::Int { .. }) => self.temp(ty, format!("({t})drachm_f64_to_u64((double){v})")),
                    (Type::Int { bits: 128, signed }, Type::Float(fb)) => {
                        let f = format!("drachm_{}128_to_f{fb}", if signed { 's' } else { 'u' });
                        self.temp(ty, format!("{f}({})", halves(&v, from)))
                    }
                    _ => self.temp(ty, format!("({t}){v}")),
                }
            }
            ExprKind::Call(id, args) => {
                let call = self.call(*id, args);
                if ty == Type::None {
                    self.line(format!("{call};"));
                    String::new()
                } else {
                    self.temp(ty, call)
                }
            }
            ExprKind::IncDec { place, inc, prefix } => {
                let target = self.place_lvalue(place);
                let old = self.temp(ty, &target);
                let op = if *inc { '+' } else { '-' };
                if ty.is_float() {
                    self.line(format!("{target} = {old} {op} 1;"));
                } else {
                    self.line(format!("{target} = ({})(({}){old} {op} 1);", c_type(ty), work_type(ty)));
                }
                if *prefix { self.temp(ty, &target) } else { old }
            }
            ExprKind::Borrow { .. }
            | ExprKind::Deref(_)
            | ExprKind::DeepCopy(_)
            | ExprKind::Take { .. }
            | ExprKind::Lend(_)
            | ExprKind::AddressOf(_)
            | ExprKind::NullPtr => unreachable!("borrows and pointers cannot be built yet"),
            ExprKind::Out { newline, arg } => {
                let p = self.place(arg);
                self.line(format!("drachm_out_str((const char *){p}.ptr, {p}.len, {});", *newline as u8));
                String::new()
            }
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.tmp += 1;
        format!("{prefix}{}", self.tmp)
    }

    fn drops(&mut self, locals: &[LocalId]) {
        for &id in locals {
            let f = drop_fn(self.func.locals[id].ty);
            let l = self.local(id);
            self.line(format!("{f}(&{l});"));
        }
    }

    /// `f(args)` for a Drachm function. Owning arguments are owned values,
    /// each made in its own variable and handed over to the callee. A
    /// method's receiver is passed by address: a place directly, any other
    /// value through a temporary.
    fn call(&mut self, id: FuncId, args: &[Expr]) -> String {
        let callee = &self.prog.funcs[id];
        let mut vals = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let v = if i == 0 && callee.has_self {
                if a.is_place() { format!("&{}", self.lvalue(a)) } else { format!("&{}", self.expr(a)) }
            } else if self.prog.owns(a.ty) {
                let v = self.fresh("a");
                self.line(format!("{} {v};", c_type(a.ty)));
                self.into(a, &v);
                v
            } else {
                self.expr(a)
            };
            vals.push(v);
        }
        format!("{}({})", callee.symbol, vals.join(", "))
    }

    /// Arguments for a runtime function: strings by pointer, numbers by value.
    fn runtime_args(&mut self, params: &[Type], args: &[Expr]) -> Vec<String> {
        args.iter()
            .zip(params)
            .map(|(a, &p)| if p == Type::Str { format!("&{}", self.place(a)) } else { format!("({}){}", c_type(p), self.expr(a)) })
            .collect()
    }

    /// Emit what makes the string place `e`; returns a C lvalue for it.
    fn place(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Str(s) => {
                let t = self.fresh("t");
                self.line(format!("const drachm_str {t} = {};", c_str_value(s)));
                t
            }
            _ => self.lvalue(e),
        }
    }

    /// Emit what writes the owned value `e` (a string or a struct that owns
    /// strings) to the C lvalue `dest`.
    fn into(&mut self, e: &Expr, dest: &str) {
        match &e.kind {
            ExprKind::Str(s) => self.line(format!("{dest} = (drachm_str){};", c_str_value(s))),
            ExprKind::Copy(p) => {
                let p = self.place(p);
                self.line(format!("{}(&{dest}, &{p});", copy_fn(e.ty)));
            }
            ExprKind::Move(id) => {
                self.line(format!("{dest} = l{id};"));
                self.line(format!("l{id} = ({}){{0}};", c_type(e.ty)));
            }
            ExprKind::Struct(fields) => {
                let def = self.prog.struct_def(e.ty);
                for (i, value) in fields {
                    let field = format!("{dest}.f_{}", def.fields[*i].name);
                    if self.prog.owns(value.ty) {
                        self.into(value, &field);
                    } else {
                        let v = self.expr(value);
                        self.line(format!("{field} = {v};"));
                    }
                }
            }
            ExprKind::Text(value) => {
                let v = self.expr(value);
                let call = match value.ty {
                    Type::Bool => format!("drachm_text_bool(&{dest}, {v})"),
                    Type::Float(32) => format!("drachm_text_f32(&{dest}, {v})"),
                    Type::Float(_) => format!("drachm_text_f64(&{dest}, {v})"),
                    Type::Int { bits: 128, signed } => {
                        format!("drachm_text_{}128(&{dest}, {})", if signed { 's' } else { 'u' }, halves(&v, value.ty))
                    }
                    Type::Int { signed: true, .. } => format!("drachm_text_s64(&{dest}, (int64_t){v})"),
                    Type::Int { signed: false, .. } => format!("drachm_text_u64(&{dest}, (uint64_t){v})"),
                    Type::None | Type::Str | Type::Struct { .. } | Type::Ref { .. } | Type::Ptr { .. } => unreachable!("checked by typeck"),
                };
                self.line(format!("{call};"));
            }
            ExprKind::Radix { value, base, upper } => {
                let v = self.expr(value);
                let h = halves(&v, value.ty);
                self.line(format!("drachm_str_radix(&{dest}, {h}, {}, {base}, {});", signed(value.ty) as u8, *upper as u8));
            }
            ExprKind::StrCall(f, args) => {
                let args = self.runtime_args(f.params(), args);
                self.line(format!("{}(&{dest}, {});", f.name(), args.join(", ")));
            }
            ExprKind::Join(parts) => {
                let parts: Vec<String> = parts.iter().map(|p| self.place(p)).collect();
                self.line(format!("drachm_str_join(&{dest}, (const drachm_str[]){{{}}}, {});", parts.join(", "), parts.len()));
            }
            ExprKind::Call(id, args) => {
                let call = self.call(*id, args);
                self.line(format!("{dest} = {call};"));
            }
            ExprKind::Scoped(inner, drops) => {
                self.into(inner, dest);
                self.drops(drops);
            }
            _ => unreachable!("not an owned value"),
        }
    }

    /// `a op b` where both have type `operand`; the result has type `ty`.
    fn binary(&mut self, op: BinOp, operand: Type, ty: Type, a: &str, b: &str) -> String {
        let sym = match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Pow => "",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
        };
        if op.is_comparison() {
            return self.temp(ty, format!("{a} {sym} {b}"));
        }
        if operand.is_float() {
            let f = if operand == Type::Float(32) { "f" } else { "" };
            return match op {
                BinOp::Rem => self.temp(ty, format!("fmod{f}({a}, {b})")),
                BinOp::Pow => self.temp(ty, format!("pow{f}({a}, {b})")),
                _ => self.temp(ty, format!("{a} {sym} {b}")),
            };
        }
        let (t, w) = (c_type(ty), work_type(ty));
        match op {
            BinOp::Div | BinOp::Rem => {
                self.line(format!("if ({b} == 0) drachm_div_zero();"));
                let mut value = format!("({t})({a} {sym} {b})");
                if signed(ty) {
                    // The smallest value divided by -1 wraps instead of trapping.
                    let wrapped = if op == BinOp::Div { format!("({t})(0 - ({w}){a})") } else { "0".to_string() };
                    value = format!("{b} == -1 ? {wrapped} : {value}");
                }
                if bits(ty) == 64 {
                    // 64-bit division is several times slower than 32-bit on
                    // many CPUs, and most values are small: take the short one
                    // when both sides fit (which also rules out b == -1).
                    value = format!(
                        "(((uint64_t){a} | (uint64_t){b}) >> 32) == 0 ? ({t})((uint32_t){a} {sym} (uint32_t){b}) : ({value})"
                    );
                }
                self.temp(ty, value)
            }
            _ => self.temp(ty, format!("({t})(({w}){a} {sym} ({w}){b})")),
        }
    }
}
