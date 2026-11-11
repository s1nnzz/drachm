//! The optimising backend, from MIR: `mir::Program` → C, handed to the system
//! C compiler. Replaces the HIR walker in `cgen` for the programs
//! `mir::cleanup::eligible` accepts; must behave exactly like `codegen_mir`.
//!
//! The C is the control-flow graph itself: every MIR local is a C local
//! declared at the top of its function (`lN`, as numbered in the MIR), blocks
//! come out in reverse post-order, and a `bbN:;` label is printed only for
//! blocks that something other than the block before jumps to. All arithmetic
//! is done in unsigned types so that overflow wraps as Drachm defines it.
//! Strings and structs are passed and returned by value; the output depends on
//! nothing but the MIR (no hash order).

use std::fmt::Write;

use crate::cgen::{bits, c_str_value, c_type, halves, main_fn, prelude, signed, struct_defs, work_type};
use crate::hir::{self, BinOp};
use crate::mir::cleanup::moved_out_temps;
use crate::mir::traversal::reverse_postorder;
use crate::mir::ty::{operand_ty, place_ty};
use crate::mir::*;
use crate::types::Type;

pub fn generate(hir: &hir::Program, mir: &Program) -> String {
    let mut out = prelude();
    out += &struct_defs(hir, true);
    let bodies: Vec<&Body> = (0..hir.funcs.len()).map(|f| mir.bodies[f].as_ref().expect("a MIR body for every function")).collect();
    for &body in &bodies {
        let _ = writeln!(out, "{};", signature(hir, body));
    }
    for &body in &bodies {
        let g = Gen { hir, mir, body, out: String::new(), skip_drop: moved_out_temps(hir, body), jumped: vec![false; body.blocks.len()] };
        let _ = writeln!(out, "\n{} {{\n{}}}", signature(hir, body), g.function());
    }
    out += &main_fn(hir);
    out
}

/// `static T symbol(T l1, T l2)`. A method's `self` is an ordinary `&T`
/// parameter, so a pointer.
fn signature(hir: &hir::Program, body: &Body) -> String {
    let params: Vec<String> = (1..=body.arg_count).map(|i| format!("{} l{i}", c_type(body.locals[Local(i as u32)].ty))).collect();
    let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };
    format!("static {} {}({params})", c_type(hir.funcs[body.func].ret), hir.funcs[body.func].symbol)
}

struct Gen<'a> {
    hir: &'a hir::Program,
    mir: &'a Program,
    body: &'a Body,
    /// The text of the block being emitted.
    out: String,
    skip_drop: Vec<bool>,
    /// Blocks some emitted `goto` names.
    jumped: Vec<bool>,
}

impl Gen<'_> {
    fn function(mut self) -> String {
        let body = self.body;
        let mut text = String::new();
        for (l, d) in body.locals.iter_enumerated() {
            if l.index() > body.arg_count && d.ty != Type::None {
                // Strings start empty, which is what `Drop` relies on. Nothing
                // else is read before it is assigned (validated), so nothing
                // else is initialised: the C compiler would only have to remove it.
                if d.ty == Type::Str {
                    let _ = writeln!(text, "    drachm_str l{} = {{0}};", l.index());
                } else {
                    let _ = writeln!(text, "    {} l{};", c_type(d.ty), l.index());
                }
            } else if l.index() == 0 && d.ty != Type::None {
                let _ = writeln!(text, "    {} l0;", c_type(d.ty));
            }
        }
        let order = reverse_postorder(body);
        let mut pieces = Vec::new();
        for (k, &b) in order.iter().enumerate() {
            self.out.clear();
            let block = &body.blocks[b];
            for s in &block.statements {
                self.statement(&s.kind);
            }
            self.terminator(&block.terminator.kind, order.get(k + 1).copied());
            pieces.push((b, std::mem::take(&mut self.out)));
        }
        for (b, piece) in pieces {
            if self.jumped[b.index()] {
                let _ = writeln!(text, "bb{}:;", b.index());
            }
            text += &piece;
        }
        text
    }

    fn line(&mut self, s: impl AsRef<str>) {
        self.out += "    ";
        self.out += s.as_ref();
        self.out.push('\n');
    }

    fn goto(&mut self, target: BlockId, next: Option<BlockId>) {
        if Some(target) != next {
            self.jumped[target.index()] = true;
            self.line(format!("goto bb{};", target.index()));
        }
    }

    // ---- places and operands -----------------------------------------------

    fn lvalue(&self, p: &Place) -> String {
        let mut s = format!("l{}", p.local.index());
        let mut ty = self.body.locals[p.local].ty;
        for pr in &p.proj {
            match *pr {
                Proj::Field(i) => {
                    let f = &self.hir.struct_def(ty).fields[i];
                    let _ = write!(s, ".f_{}", f.name);
                    ty = f.ty;
                }
                Proj::Deref => {
                    s = format!("(*{s})");
                    ty = ty.target().expect("deref of a non-pointer");
                }
            }
        }
        s
    }

    fn op_ty(&self, op: &Operand) -> Type {
        operand_ty(self.body, self.hir, op).expect("validated MIR")
    }

    fn operand(&self, op: &Operand) -> String {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.lvalue(p),
            Operand::Const(Constant::Int(v, ty)) => {
                let (lo, hi) = (*v as u64, (*v >> 64) as u64);
                let t = c_type(*ty);
                match bits(*ty) {
                    128 => format!("(({t})(((drachm_u128)UINT64_C({hi}) << 64) | UINT64_C({lo})))"),
                    _ => format!("(({t})UINT64_C({lo}))"),
                }
            }
            Operand::Const(Constant::Float(v, Type::Float(32))) => format!("drachm_f32_bits(UINT32_C({}))", (*v as f32).to_bits()),
            Operand::Const(Constant::Float(v, _)) => format!("drachm_f64_bits(UINT64_C({}))", v.to_bits()),
            Operand::Const(Constant::Bool(b)) => format!("((uint8_t){})", *b as u8),
            Operand::Const(Constant::Str(id)) => format!("((drachm_str){})", c_str_value(&self.mir.strings[*id])),
        }
    }

    // ---- statements --------------------------------------------------------

    fn statement(&mut self, k: &StatementKind) {
        match k {
            StatementKind::Assign(dest, rv) => {
                let ty = place_ty(self.body, self.hir, dest).expect("validated MIR");
                if ty != Type::None {
                    self.assign(&self.lvalue(dest), ty, rv);
                }
            }
            StatementKind::Drop { place, flag: None, .. } => {
                if place.proj.is_empty() && self.skip_drop[place.local.index()] {
                    return;
                }
                let ty = place_ty(self.body, self.hir, place).expect("validated MIR");
                if ty == Type::Str {
                    // `drachm_str_drop` starts with this test; doing it here skips the call.
                    let lv = self.lvalue(place);
                    self.line(format!("if ({lv}.cap != 0) drachm_str_drop(&{lv});"));
                }
            }
            StatementKind::Drop { flag: Some(_), .. } => unreachable!("drop flags are not supported by the MIR backend yet"),
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::FakeRead(_) | StatementKind::Nop => {}
        }
    }

    /// `lv = rv;` where `lv` has type `ty`.
    fn assign(&mut self, lv: &str, ty: Type, rv: &Rvalue) {
        let value = match rv {
            Rvalue::Use(op) => self.operand(op),
            Rvalue::Neg(op) => {
                let v = self.operand(op);
                if ty.is_float() { format!("-{v}") } else { format!("({})(0 - ({}){v})", c_type(ty), work_type(ty)) }
            }
            Rvalue::Not(op) => format!("!{}", self.operand(op)),
            Rvalue::BitNot(op) => format!("({})~({}){}", c_type(ty), work_type(ty), self.operand(op)),
            Rvalue::Binary(op, l, r) => {
                let operand = self.op_ty(l);
                let (a, b) = (self.operand(l), self.operand(r));
                self.binary(*op, operand, &a, &b)
            }
            Rvalue::Shift { left, value, amount } => {
                let (v, n) = (self.operand(value), self.operand(amount));
                let (t, w) = (c_type(ty), work_type(ty));
                let n = format!("((uint32_t)({n}) & {})", bits(ty) - 1);
                if *left {
                    format!("({t})(({w}){v} << {n})")
                } else {
                    // On a signed value this is an arithmetic shift (gcc and clang both define it so).
                    format!("({t})({v} >> {n})")
                }
            }
            Rvalue::IntPow(base, exp) => {
                let (bty, xty) = (self.op_ty(base), self.op_ty(exp));
                let (b, x) = (self.operand(base), self.operand(exp));
                let negative = if signed(xty) { format!("({x} < 0)") } else { "0".into() };
                let base_signed = signed(bty) as u8;
                if bits(bty) <= 64 && bits(xty) <= 64 {
                    format!("({})drachm_ipow64((uint64_t){b}, (uint64_t){x}, {negative}, {base_signed})", c_type(bty))
                } else {
                    self.line(format!(
                        "{{ const drachm_pair p = drachm_ipow128({}, {}, {negative}, {base_signed}); {lv} = ({})drachm_join(p.lo, p.hi); }}",
                        halves(&b, bty),
                        halves(&x, xty),
                        c_type(bty)
                    ));
                    return;
                }
            }
            Rvalue::Convert(op, to) => {
                let from = self.op_ty(op);
                let v = self.operand(op);
                let t = c_type(*to);
                match (from, *to) {
                    (Type::Float(_), Type::Int { bits: 128, .. }) => {
                        self.line(format!(
                            "{{ const drachm_pair p = drachm_f64_to_u128((double){v}); {lv} = ({t})drachm_join(p.lo, p.hi); }}"
                        ));
                        return;
                    }
                    (Type::Float(_), Type::Int { .. }) => format!("({t})drachm_f64_to_u64((double){v})"),
                    (Type::Int { bits: 128, signed }, Type::Float(fb)) => {
                        format!("drachm_{}128_to_f{fb}({})", if signed { 's' } else { 'u' }, halves(&v, from))
                    }
                    _ => format!("({t}){v}"),
                }
            }
            Rvalue::Ref(_, p) => format!("&{}", self.lvalue(p)),
            Rvalue::Aggregate(sid, ops) => {
                let def = &self.hir.structs[*sid];
                let parts: Vec<String> = ops.iter().enumerate().map(|(i, op)| format!(".f_{} = {}", def.fields[i].name, self.operand(op))).collect();
                format!("({}){{ {} }}", crate::cgen::struct_name(&def.name), parts.join(", "))
            }
            Rvalue::RawRef(_) | Rvalue::DeepCopy(_) => unreachable!("not supported by the MIR backend yet: {rv:?}"),
        };
        self.line(format!("{lv} = {value};"));
    }

    /// `a op b` where both have type `operand`. A division first checks for zero.
    fn binary(&mut self, op: BinOp, operand: Type, a: &str, b: &str) -> String {
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
            return format!("{a} {sym} {b}");
        }
        if operand.is_float() {
            let f = if operand == Type::Float(32) { "f" } else { "" };
            return match op {
                BinOp::Rem => format!("fmod{f}({a}, {b})"),
                BinOp::Pow => format!("pow{f}({a}, {b})"),
                _ => format!("{a} {sym} {b}"),
            };
        }
        let (t, w) = (c_type(operand), work_type(operand));
        match op {
            BinOp::Div | BinOp::Rem => {
                self.line(format!("if ({b} == 0) drachm_div_zero();"));
                let mut value = format!("({t})({a} {sym} {b})");
                if signed(operand) {
                    // The smallest value divided by -1 wraps instead of trapping.
                    let wrapped = if op == BinOp::Div { format!("({t})(0 - ({w}){a})") } else { "0".to_string() };
                    value = format!("{b} == -1 ? {wrapped} : {value}");
                }
                if bits(operand) == 64 {
                    // 64-bit division is several times slower than 32-bit on
                    // many CPUs, and most values are small: take the short one
                    // when both sides fit (which also rules out b == -1).
                    value = format!(
                        "(((uint64_t){a} | (uint64_t){b}) >> 32) == 0 ? ({t})((uint32_t){a} {sym} (uint32_t){b}) : ({value})"
                    );
                }
                value
            }
            _ => format!("({t})(({w}){a} {sym} ({w}){b})"),
        }
    }

    // ---- terminators -------------------------------------------------------

    fn terminator(&mut self, k: &TerminatorKind, next: Option<BlockId>) {
        match k {
            TerminatorKind::Goto(t) => self.goto(*t, next),
            TerminatorKind::If { cond, then, els } => {
                let c = self.operand(cond);
                if then == els {
                    self.goto(*then, next);
                } else if Some(*then) == next {
                    self.jumped[els.index()] = true;
                    self.line(format!("if (!{c}) goto bb{};", els.index()));
                } else if Some(*els) == next {
                    self.jumped[then.index()] = true;
                    self.line(format!("if ({c}) goto bb{};", then.index()));
                } else {
                    self.jumped[then.index()] = true;
                    self.line(format!("if ({c}) goto bb{};", then.index()));
                    self.goto(*els, next);
                }
            }
            TerminatorKind::Return => {
                if self.body.locals[Local(0)].ty == Type::None {
                    self.line("return;");
                } else {
                    self.line("return l0;");
                }
            }
            TerminatorKind::Unreachable => self.line("__builtin_unreachable();"),
            TerminatorKind::Call { callee, args, dest, target } => {
                self.call(*callee, args, dest.as_ref());
                self.goto(*target, next);
            }
            TerminatorKind::Switch { .. } | TerminatorKind::Panic(_) => unreachable!("not supported by the MIR backend yet: {k:?}"),
        }
    }

    fn call(&mut self, callee: Callee, args: &[Arg], dest: Option<&Place>) {
        let dest_lv = dest.map(|d| self.lvalue(d));
        match callee {
            Callee::Func(id) => {
                let f = &self.hir.funcs[id];
                let a: Vec<String> = args.iter().map(|a| self.operand(&a.op)).collect();
                let call = format!("{}({})", f.symbol, a.join(", "));
                match (&dest_lv, f.ret) {
                    (Some(d), t) if t != Type::None => self.line(format!("{d} = {call};")),
                    _ => self.line(format!("{call};")),
                }
            }
            Callee::Str(f) => {
                let a: Vec<String> = args
                    .iter()
                    .zip(f.params())
                    .map(|(a, &p)| if p == Type::Str { self.operand(&a.op) } else { format!("({}){}", c_type(p), self.operand(&a.op)) })
                    .collect();
                let a = a.join(", ");
                match (&dest_lv, f.ret()) {
                    (Some(d), Type::Str) => self.line(format!("{}(&{d}, {a});", f.name())),
                    (Some(d), t) if t != Type::None => self.line(format!("{d} = {}({a});", f.name())),
                    _ => self.line(format!("{}({a});", f.name())),
                }
            }
            Callee::Out { newline } => {
                let p = self.operand(&args[0].op);
                self.line(format!("drachm_out_str((const char *)(*{p}).ptr, (*{p}).len, {});", newline as u8));
            }
            Callee::Text => {
                let dest = dest_lv.expect("a string result has a destination");
                let ty = self.op_ty(&args[0].op);
                let v = self.operand(&args[0].op);
                let call = match ty {
                    Type::Bool => format!("drachm_text_bool(&{dest}, {v})"),
                    Type::Float(32) => format!("drachm_text_f32(&{dest}, {v})"),
                    Type::Float(_) => format!("drachm_text_f64(&{dest}, {v})"),
                    Type::Int { bits: 128, signed } => {
                        format!("drachm_text_{}128(&{dest}, {})", if signed { 's' } else { 'u' }, halves(&v, ty))
                    }
                    Type::Int { signed: true, .. } => format!("drachm_text_s64(&{dest}, (int64_t){v})"),
                    Type::Int { signed: false, .. } => format!("drachm_text_u64(&{dest}, (uint64_t){v})"),
                    Type::None | Type::Str | Type::Struct { .. } | Type::Ref { .. } | Type::Ptr { .. } => unreachable!("checked by typeck"),
                };
                self.line(format!("{call};"));
            }
            Callee::Radix { base, upper } => {
                let dest = dest_lv.expect("a string result has a destination");
                let ty = self.op_ty(&args[0].op);
                let v = self.operand(&args[0].op);
                self.line(format!("drachm_str_radix(&{dest}, {}, {}, {base}, {});", halves(&v, ty), signed(ty) as u8, upper as u8));
            }
            Callee::Join => {
                let dest = dest_lv.expect("a string result has a destination");
                let parts: Vec<String> = args.iter().map(|a| format!("*{}", self.operand(&a.op))).collect();
                self.line(format!("drachm_str_join(&{dest}, (const drachm_str[]){{{}}}, {});", parts.join(", "), parts.len()));
            }
        }
    }
}
