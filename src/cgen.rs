//! The optimising backend: `hir` → C, handed to the system C compiler.
//!
//! The C is not meant to be read. Every sub-expression becomes its own
//! `const` temporary, in evaluation order, which pins down left-to-right
//! evaluation (C leaves it open) and costs nothing once the C compiler has
//! run. All arithmetic is done in unsigned types so that overflow wraps as
//! Drachm defines it instead of being undefined behaviour.

use std::fmt::Write;

use crate::hir::*;
use crate::types::Type;

const RUNTIME_C: &str = include_str!("../runtime/runtime.c");

pub fn generate(prog: &Program) -> String {
    let mut out = String::from("#define DRACHM_INLINE 1\n");
    out += RUNTIME_C;
    out += "\nstatic inline double drachm_f64_bits(uint64_t b) { double d; memcpy(&d, &b, 8); return d; }\n";
    out += "static inline float drachm_f32_bits(uint32_t b) { float f; memcpy(&f, &b, 4); return f; }\n\n";

    for f in &prog.funcs {
        let _ = writeln!(out, "{};", signature(f));
    }
    for f in &prog.funcs {
        let mut g = Gen { prog, out: String::new(), tmp: 0, depth: 1 };
        for (i, l) in f.locals.iter().enumerate().skip(f.params) {
            g.line(format!("{} l{i} = 0;", c_type(l.ty)));
        }
        g.block(&f.body);
        let _ = writeln!(out, "\n{} {{\n{}}}", signature(f), g.out);
    }

    let main = &prog.funcs[prog.main];
    let call = format!("{}()", func_name(main));
    let body = if main.ret == Type::None { format!("{call}; return 0;") } else { format!("return (int){call};") };
    let _ = writeln!(out, "\nint main(void) {{ {body} }}");
    out
}

fn func_name(f: &Func) -> String {
    format!("drachm_f_{}", f.name)
}

fn signature(f: &Func) -> String {
    let params: Vec<String> = f.locals[..f.params].iter().enumerate().map(|(i, l)| format!("{} l{i}", c_type(l.ty))).collect();
    let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };
    format!("static {} {}({params})", c_type(f.ret), func_name(f))
}

fn c_type(t: Type) -> &'static str {
    match t {
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
        Type::None | Type::Str => "void",
    }
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
    out: String,
    tmp: usize,
    depth: usize,
}

impl Gen<'_> {
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

    /// Store `value` in a fresh temporary and return its name.
    fn temp(&mut self, ty: Type, value: impl AsRef<str>) -> String {
        self.tmp += 1;
        let name = format!("t{}", self.tmp);
        self.line(format!("const {} {name} = {};", c_type(ty), value.as_ref()));
        name
    }

    fn block(&mut self, b: &Block) {
        for s in b {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Assign(id, e) => {
                let v = self.expr(e);
                self.line(format!("l{id} = {v};"));
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
            Stmt::Loop { label, cond, body, step } => {
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
            ExprKind::Str(_) | ExprKind::Text(_) => unreachable!("text only exists as an argument of io::out"),
            // A copy, so that a later `++` cannot change what was read here.
            ExprKind::Local(id) => self.temp(ty, format!("l{id}")),
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
                let callee = &self.prog.funcs[*id];
                let args: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                let call = format!("{}({})", func_name(callee), args.join(", "));
                if ty == Type::None {
                    self.line(format!("{call};"));
                    String::new()
                } else {
                    self.temp(ty, call)
                }
            }
            ExprKind::IncDec { local, inc, prefix } => {
                let old = self.temp(ty, format!("l{local}"));
                let op = if *inc { '+' } else { '-' };
                if ty.is_float() {
                    self.line(format!("l{local} = {old} {op} 1;"));
                } else {
                    self.line(format!("l{local} = ({})(({}){old} {op} 1);", c_type(ty), work_type(ty)));
                }
                if *prefix { self.temp(ty, format!("l{local}")) } else { old }
            }
            ExprKind::Out { newline, arg } => {
                let nl = *newline as u8;
                match &arg.kind {
                    ExprKind::Str(s) => self.line(format!("drachm_out_str({}, {}, {nl});", c_string(s), s.len())),
                    ExprKind::Text(value) => {
                        let v = self.expr(value);
                        let call = match value.ty {
                            Type::Bool => format!("drachm_out_bool({v}, {nl})"),
                            Type::Float(32) => format!("drachm_out_f32({v}, {nl})"),
                            Type::Float(_) => format!("drachm_out_f64({v}, {nl})"),
                            Type::Int { bits: 128, signed } => {
                                format!("drachm_out_{}128({}, {nl})", if signed { 's' } else { 'u' }, halves(&v, value.ty))
                            }
                            Type::Int { signed: true, .. } => format!("drachm_out_s64((int64_t){v}, {nl})"),
                            Type::Int { signed: false, .. } => format!("drachm_out_u64((uint64_t){v}, {nl})"),
                            Type::None | Type::Str => unreachable!(),
                        };
                        self.line(format!("{call};"));
                    }
                    _ => unreachable!("checked by typeck"),
                }
                String::new()
            }
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
