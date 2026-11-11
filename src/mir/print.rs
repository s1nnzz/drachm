//! The text format of Appendix B. For tests and debugging; it may change.

use std::fmt::Write;

use super::ty::local_ty;
use super::*;
use crate::hir::BinOp;

pub fn program(p: &Program, hir: &hir::Program) -> String {
    let mut out = String::new();
    let mut first = true;
    for body in p.bodies.iter().flatten() {
        if !first {
            out.push('\n');
        }
        first = false;
        function(&mut out, p, hir, body);
    }
    out
}

fn function(out: &mut String, p: &Program, hir: &hir::Program, body: &Body) {
    let ty = |l: Local| local_ty(body, l).map_or_else(|e| format!("<{e}>"), |t| t.to_string());
    let params: Vec<String> = (1..=body.arg_count).map(|i| format!("_{i}: {}", ty(Local::new(i)))).collect();
    let _ = writeln!(out, "fn {}({}) -> {}", body.name, params.join(", "), ty(Local(0)));
    for (l, d) in body.locals.iter_enumerated() {
        if l.index() >= 1 && l.index() <= body.arg_count {
            continue;
        }
        let note = match &d.kind {
            LocalKind::Return => String::new(),
            LocalKind::User { name, .. } | LocalKind::Param { name, .. } => format!("// {name}"),
            LocalKind::Temp => "// temp".to_string(),
            LocalKind::DropFlag => "// drop flag".to_string(),
        };
        let _ = writeln!(out, "{}", format!("  {l}: {:<16}{note}", ty(l)).trim_end());
    }
    for (b, block) in body.blocks.iter_enumerated() {
        let _ = writeln!(out, "{b}:");
        for s in &block.statements {
            let _ = writeln!(out, "  {}", statement(p, hir, &s.kind));
        }
        let _ = writeln!(out, "  {}", terminator(p, hir, &block.terminator.kind));
    }
}

fn statement(p: &Program, hir: &hir::Program, k: &StatementKind) -> String {
    match k {
        StatementKind::Assign(dest, rv) => format!("{} = {}", place(dest), rvalue(p, hir, rv)),
        StatementKind::StorageLive(l) => format!("StorageLive({l})"),
        StatementKind::StorageDead(l) => format!("StorageDead({l})"),
        StatementKind::Drop { place: pl, flag, .. } => match flag {
            Some(f) => format!("Drop({}) if {f}", place(pl)),
            None => format!("Drop({})", place(pl)),
        },
        StatementKind::FakeRead(pl) => format!("FakeRead({})", place(pl)),
        StatementKind::Nop => "Nop".to_string(),
    }
}

fn terminator(p: &Program, hir: &hir::Program, k: &TerminatorKind) -> String {
    match k {
        TerminatorKind::Goto(b) => format!("goto -> {b}"),
        TerminatorKind::If { cond, then, els } => format!("If {} -> {then}, {els}", operand(p, cond)),
        TerminatorKind::Switch { discr, cases, otherwise } => {
            let mut arms: Vec<String> = cases.iter().map(|(v, b)| format!("{v}: {b}")).collect();
            arms.push(format!("otherwise: {otherwise}"));
            format!("Switch {} [{}]", operand(p, discr), arms.join(", "))
        }
        TerminatorKind::Call { callee: c, args, dest, target } => {
            let name = match c {
                Callee::Func(f) => super::build::display_name(&hir.funcs[*f].symbol).to_string(),
                Callee::Str(f) => format!("Str{f:?}"),
                Callee::Out { newline: false } => "Out".to_string(),
                Callee::Out { newline: true } => "Outln".to_string(),
                Callee::Text => "Text".to_string(),
                Callee::InLine => "InLine".to_string(),
                Callee::InEnded => "InEnded".to_string(),
                Callee::Parse(ty) => format!("Parse<{ty}>"),
                Callee::Radix { base, upper } => format!("Radix{base}{}", if *upper { "Upper" } else { "" }),
                Callee::Join => "Join".to_string(),
            };
            let args: Vec<String> = args.iter().map(|a| operand(p, &a.op)).collect();
            let call = format!("call {name}({}) -> {target}", args.join(", "));
            match dest {
                Some(d) => format!("{} = {call}", place(d)),
                None => call,
            }
        }
        TerminatorKind::Return => "return".to_string(),
        TerminatorKind::Panic(kind) => format!("panic({kind:?})"),
        TerminatorKind::Unreachable => "unreachable".to_string(),
    }
}

fn place(pl: &Place) -> String {
    let mut s = pl.local.to_string();
    for pr in &pl.proj {
        s = match pr {
            Proj::Field(f) => format!("{s}.{f}"),
            Proj::Deref => format!("(*{s})"),
        };
    }
    s
}

fn operand(p: &Program, op: &Operand) -> String {
    match op {
        Operand::Copy(pl) => format!("copy {}", place(pl)),
        Operand::Move(pl) => format!("move {}", place(pl)),
        Operand::Const(c) => format!("const {}", constant(p, c)),
    }
}

fn constant(p: &Program, c: &Constant) -> String {
    match c {
        Constant::Int(bits, ty) => match ty {
            Type::Int { bits: width, signed: true } => {
                // Stored zero-extended; show the signed value.
                let shift = 128 - u32::from(*width);
                let v = ((*bits << shift) as i128) >> shift;
                format!("{v}s{width}")
            }
            Type::Int { bits: width, signed: false } => format!("{bits}u{width}"),
            _ => format!("{bits}{ty}"),
        },
        Constant::Float(v, Type::Float(w)) => format!("{v:?}f{w}"),
        Constant::Float(v, ty) => format!("{v:?}{ty}"),
        Constant::Bool(b) => b.to_string(),
        Constant::Str(id) => format!("{:?}", p.strings[*id]),
    }
}

fn binop(op: BinOp) -> String {
    format!("{op:?}")
}

fn rvalue(p: &Program, hir: &hir::Program, rv: &Rvalue) -> String {
    let o = |x: &Operand| operand(p, x);
    match rv {
        Rvalue::Use(x) => o(x),
        Rvalue::Neg(x) => format!("Neg({})", o(x)),
        Rvalue::Not(x) => format!("Not({})", o(x)),
        Rvalue::BitNot(x) => format!("BitNot({})", o(x)),
        Rvalue::Binary(op, a, b) => format!("{}({}, {})", binop(*op), o(a), o(b)),
        Rvalue::Shift { left, value, amount } => {
            format!("{}({}, {})", if *left { "Shl" } else { "Shr" }, o(value), o(amount))
        }
        Rvalue::IntPow(a, b) => format!("IntPow({}, {})", o(a), o(b)),
        Rvalue::Convert(x, t) => format!("Convert({}, {t})", o(x)),
        Rvalue::Ref(BorrowKind::Shared, pl) => format!("&{}", place(pl)),
        Rvalue::Ref(BorrowKind::Mut { two_phase: false }, pl) => format!("&mut {}", place(pl)),
        Rvalue::Ref(BorrowKind::Mut { two_phase: true }, pl) => format!("&mut two_phase {}", place(pl)),
        Rvalue::RawRef(pl) => format!("&raw {}", place(pl)),
        Rvalue::Aggregate(sid, ops) => {
            let ops: Vec<String> = ops.iter().map(o).collect();
            format!("Aggregate {}({})", hir.structs[*sid].name, ops.join(", "))
        }
        Rvalue::DeepCopy(pl) => format!("DeepCopy({})", place(pl)),
    }
}
