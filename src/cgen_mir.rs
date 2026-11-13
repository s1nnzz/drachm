//! The optimising backend, from MIR: `mir::Program` → C, handed to the system
//! C compiler. Must behave exactly like `codegen_mir`.
//!
//! The C is the control-flow graph itself: every MIR local is a C local
//! declared at the top of its function (`lN`, as numbered in the MIR), blocks
//! come out in reverse post-order, and a `bbN:;` label is printed only for
//! blocks that something other than the block before jumps to. All arithmetic
//! is done in unsigned types so that overflow wraps as Drachm defines it.
//! Strings and structs are passed and returned by value; the output depends on
//! nothing but the MIR (no hash order).

use std::fmt::Write;

use crate::cgen_common::{bits, c_str_value, c_type, halves, main_fn, prelude, signed, struct_defs, work_type};
use crate::hir::{self, BinOp};
use crate::mir::drops::{copy_glue_types, drop_glue_types, glue_name, holds_string};
use crate::mir::MapFn;
use crate::codegen_common::{key_is_composite, key_tag, map_ops_type};
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
    out += &glue(hir);
    out += &vtables(hir);
    for &body in &bodies {
        let g = Gen { hir, mir, body, out: String::new(), jumped: vec![false; body.blocks.len()], branch_over: None, consts: Vec::new() };
        let _ = writeln!(out, "\n{} {{\n{}}}", signature(hir, body), g.function());
    }
    out += &main_fn(hir);
    out
}

/// The read-only table of each (type, interface) a borrow is converted at
/// (`hir::VTable`): `drachm_vt_N`, an array of words laid out as
/// `hir::DynIface` says. The tables of an interface's ancestors come first,
/// since a table points at them.
fn vtables(hir: &hir::Program) -> String {
    let mut out = String::new();
    let mut order: Vec<usize> = (0..hir.vtables.len()).collect();
    order.sort_by_key(|&i| hir.dyn_ifaces[hir.vtables[i].iface].ancestors.len());
    for i in order {
        let vt = &hir.vtables[i];
        let iface = &hir.dyn_ifaces[vt.iface];
        let (size, align) = hir.size_align(vt.ty);
        let mut words = vec![format!("(const void *)(uintptr_t){size}"), format!("(const void *)(uintptr_t){align}")];
        for &a in &iface.ancestors {
            words.push(format!("(const void *)drachm_vt_{}", hir.vtable_index(vt.ty, a)));
        }
        for m in &vt.methods {
            words.push(match m {
                Some(f) => format!("(const void *){}", hir.funcs[*f].symbol),
                None => "(const void *)0".to_string(),
            });
        }
        for &off in &vt.fields {
            words.push(format!("(const void *)(uintptr_t){off}"));
        }
        let _ = writeln!(out, "static const void *const drachm_vt_{i}[] = {{ {} }}; /* {} as {} */", words.join(", "), vt.ty, iface.name);
    }
    out
}

/// The drop and copy glue of the structs and enums that need it:
/// `drachm_drop_S(&value)` runs the struct's `drop()` (if it has one) and then
/// drops its fields in declaration order (an enum drops the fields of the
/// variant its tag names); `drachm_copy_S(&dest, &src)` copies the plain
/// parts by value and deep-copies the strings. Prototypes first, so the order
/// of the definitions does not matter. Structs by id, then enums.
fn glue(hir: &hir::Program) -> String {
    let mut protos = String::new();
    let mut defs = key_fns_c(hir);
    // The drop statement for the field at C path `access` (a value, not a pointer).
    let drop_of = |access: &str, ty: Type| -> Option<String> {
        match ty {
            Type::Str => Some(format!("if ({access}.cap != 0) drachm_str_drop(&{access});")),
            Type::Struct { .. } | Type::Enum { .. } | Type::List { .. } | Type::Array { .. } | Type::Map { .. } if crate::mir::needs_drop(hir, ty) => {
                Some(format!("drachm_drop_{}(&{access});", glue_name(ty)))
            }
            _ => None,
        }
    };
    let copy_of = |dst: &str, src: &str, ty: Type| -> Option<String> {
        match ty {
            Type::Str => Some(format!("drachm_str_copy(&{dst}, &{src});")),
            Type::Struct { .. } | Type::Enum { .. } | Type::List { .. } | Type::Array { .. } | Type::Map { .. } if holds_string(hir, ty) => {
                Some(format!("drachm_copy_{}(&{dst}, &{src});", glue_name(ty)))
            }
            _ => None,
        }
    };
    for t in drop_glue_types(hir) {
        let (n, c) = (glue_name(t), c_type(t));
        let _ = writeln!(protos, "static void drachm_drop_{n}({c} *p);");
        let _ = writeln!(defs, "\nstatic void drachm_drop_{n}({c} *p) {{");
        match t {
            Type::Struct { id, .. } => {
                let def = &hir.structs[id];
                if let Some(f) = def.has_drop {
                    let _ = writeln!(defs, "    {}(p);", hir.funcs[f].symbol);
                }
                for f in &def.fields {
                    if let Some(line) = drop_of(&format!("p->f_{}", f.name), f.ty) {
                        let _ = writeln!(defs, "    {line}");
                    }
                }
            }
            Type::List { elem } => {
                let f = match drop_of("(*x)", *elem) {
                    None => "NULL".to_string(),
                    Some(_) if *elem == Type::Str => "(drachm_drop_fn)drachm_str_drop".to_string(),
                    Some(_) => format!("(drachm_drop_fn)drachm_drop_{}", glue_name(*elem)),
                };
                let c = c_type(*elem);
                let _ = writeln!(defs, "    drachm_list_drop((drachm_list *)p, sizeof({c}), {f});");
            }
            Type::Array { elem, len } => {
                if let Some(line) = drop_of("p->a[i]", *elem) {
                    let _ = writeln!(defs, "    for (uint64_t i = 0; i < {len}; i++) {{ {line} }}");
                }
            }
            Type::Enum { id, .. } => {
                let _ = writeln!(defs, "    switch (p->tag) {{");
                for (vi, v) in hir.enums[id].variants.iter().enumerate() {
                    let lines: Vec<String> = v.fields.iter().filter_map(|f| drop_of(&format!("p->u.v{vi}.f_{}", f.name), f.ty)).collect();
                    if !lines.is_empty() {
                        let _ = writeln!(defs, "    case {vi}: {} break;", lines.join(" "));
                    }
                }
                let _ = writeln!(defs, "    default: break;\n    }}");
                if hir.enums[id].kind == hir::EnumKind::Failure {
                    let _ = writeln!(defs, "    drachm_cause_drop(p->u.v0.f_{});", hir::CAUSE_FIELD);
                }
            }
            Type::Map { .. } => {
                let _ = writeln!(defs, "    drachm_map_drop(p, {});", map_ops_c(t));
            }
            _ => unreachable!(),
        }
        defs += "}\n";
    }
    for t in copy_glue_types(hir) {
        let (n, c) = (glue_name(t), c_type(t));
        let _ = writeln!(protos, "static void drachm_copy_{n}({c} *dst, const {c} *src);");
        let _ = writeln!(defs, "\nstatic void drachm_copy_{n}({c} *dst, const {c} *src) {{\n    *dst = *src;");
        match t {
            Type::Struct { id, .. } => {
                for f in &hir.structs[id].fields {
                    if let Some(line) = copy_of(&format!("dst->f_{}", f.name), &format!("src->f_{}", f.name), f.ty) {
                        let _ = writeln!(defs, "    {line}");
                    }
                }
            }
            Type::List { elem } => {
                let f = match copy_of("(*d)", "(*s)", *elem) {
                    None => "NULL".to_string(),
                    Some(_) if *elem == Type::Str => "(drachm_copy_fn)drachm_str_copy".to_string(),
                    Some(_) => format!("(drachm_copy_fn)drachm_copy_{}", glue_name(*elem)),
                };
                let c = c_type(*elem);
                let _ = writeln!(defs, "    drachm_list_copy((drachm_list *)dst, (const drachm_list *)src, sizeof({c}), _Alignof({c}), {f});");
            }
            Type::Array { elem, len } => {
                if let Some(line) = copy_of("dst->a[i]", "src->a[i]", *elem) {
                    let _ = writeln!(defs, "    for (uint64_t i = 0; i < {len}; i++) {{ {line} }}");
                }
            }
            Type::Enum { id, .. } => {
                let _ = writeln!(defs, "    switch (src->tag) {{");
                for (vi, v) in hir.enums[id].variants.iter().enumerate() {
                    let lines: Vec<String> = v
                        .fields
                        .iter()
                        .filter_map(|f| copy_of(&format!("dst->u.v{vi}.f_{}", f.name), &format!("src->u.v{vi}.f_{}", f.name), f.ty))
                        .collect();
                    if !lines.is_empty() {
                        let _ = writeln!(defs, "    case {vi}: {} break;", lines.join(" "));
                    }
                }
                let _ = writeln!(defs, "    default: break;\n    }}");
            }
            Type::Map { .. } => {
                let _ = writeln!(defs, "    drachm_map_copy(dst, src, {});", map_ops_c(t));
            }
            _ => unreachable!(),
        }
        defs += "}\n";
    }
    let mut seen = std::collections::HashSet::new();
    for mt in hir.collections.iter().copied().filter(|t| t.is_map()).map(map_ops_type) {
        if seen.insert(mt) {
            let name = glue_name(mt);
            let _ = writeln!(protos, "static const drachm_map_ops drachm_ops_{name};");
            let _ = writeln!(defs, "static const drachm_map_ops drachm_ops_{name} = {};", map_ops_init(hir, mt));
        }
    }
    protos + &defs
}

/// `static T symbol(T l1, T l2)`. A method's `self` is an ordinary `&T`
/// parameter, so a pointer.
fn signature(hir: &hir::Program, body: &Body) -> String {
    let params: Vec<String> = (1..=body.arg_count).map(|i| {
        if i == 1 && hir.closures.contains_key(&body.func) {
            "uint8_t *env".into()
        } else {
            format!("{} l{i}", c_type(body.locals[Local(i as u32)].ty))
        }
    }).collect();
    let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };
    format!("static {} {}({params})", c_type(hir.funcs[body.func].ret), hir.funcs[body.func].symbol)
}

struct Gen<'a> {
    hir: &'a hir::Program,
    mir: &'a Program,
    body: &'a Body,
    /// The text of the block being emitted.
    out: String,
    /// Blocks some emitted `goto` names.
    jumped: Vec<bool>,
    /// `if (!c) goto els;` for the block just emitted, left for `function` to
    /// print (or to fold into an early return).
    branch_over: Option<(String, BlockId)>,
    /// Scalars assigned exactly once, declared `const` where they are assigned.
    consts: Vec<bool>,
}

impl Gen<'_> {
    fn function(mut self) -> String {
        let body = self.body;
        self.consts = self.const_locals();
        let order = reverse_postorder(body);
        let mentions = mentions(body);
        let mut pieces = Vec::new();
        for (k, &b) in order.iter().enumerate() {
            self.out.clear();
            let block = &body.blocks[b];
            let mut stmts = &block.statements[..];
            // A comparison used only by the branch right after it goes into
            // the `if` itself: GCC's branch heuristics and inlining do
            // measurably better on `if (n < 2)` than on a flag variable.
            let mut cond = None;
            if let TerminatorKind::If { cond: Operand::Copy(p) | Operand::Move(p), .. } = &block.terminator.kind
                && p.proj.is_empty()
                && mentions[p.local.index()] == 2
                && matches!(body.locals[p.local].kind, LocalKind::Temp)
                && let Some((last, rest)) = stmts.split_last()
                && let StatementKind::Assign(d, Rvalue::Binary(op, a, b)) = &last.kind
                && d.local == p.local
                && d.proj.is_empty()
                && op.is_comparison()
            {
                let operand = self.op_ty(a);
                let (a, b) = (self.operand(a), self.operand(b));
                cond = Some(format!("({})", self.binary(*op, operand, &a, &b)));
                stmts = rest;
            }
            // Likewise `_0 = x; return _0;` becomes `return x;`.
            if matches!(block.terminator.kind, TerminatorKind::Return)
                && let Some((last, rest)) = stmts.split_last()
                && let StatementKind::Assign(d, rv) = &last.kind
                && d.local == Local(0)
                && d.proj.is_empty()
                && matches!(body.locals[Local(0)].ty, Type::Int { .. } | Type::Float(_) | Type::Bool)
            {
                let text = match rv {
                    Rvalue::Use(op) => Some(self.operand(op)),
                    Rvalue::Binary(op, a, b) if !matches!(op, BinOp::Div | BinOp::Rem) => {
                        let operand = self.op_ty(a);
                        let (a, b) = (self.operand(a), self.operand(b));
                        Some(self.binary(*op, operand, &a, &b))
                    }
                    _ => None,
                };
                if let Some(text) = text {
                    cond = Some(text);
                    stmts = rest;
                }
            }
            for s in stmts {
                self.statement(&s.kind);
            }
            self.terminator(&block.terminator.kind, order.get(k + 1).copied(), cond);
            pieces.push((b, std::mem::take(&mut self.out), self.branch_over.take()));
        }
        // `if (!c) goto bbE;` followed by a block that is just `return x;` and
        // then by bbE is written `if (c) { return x; }`: the shape GCC's
        // early-return heuristics were tuned on, and measurably faster on
        // small recursive functions.
        let n = pieces.len();
        let mut folded = vec![false; n];
        for k in 0..n {
            if let Some((_, els)) = &pieces[k].2
                && k + 2 < n
                && pieces[k + 2].0 == *els
                && pieces[k + 1].1.lines().count() == 1
                && pieces[k + 1].1.trim_start().starts_with("return")
            {
                folded[k] = true;
            }
        }
        loop {
            let mut jumped = self.jumped.clone();
            for k in 0..n {
                if let (Some((_, els)), false) = (&pieces[k].2, folded[k]) {
                    jumped[els.index()] = true;
                }
            }
            let mut changed = false;
            for k in 0..n {
                if folded[k] && jumped[pieces[k + 1].0.index()] {
                    folded[k] = false;
                    changed = true;
                }
            }
            if !changed {
                self.jumped = jumped;
                break;
            }
        }
        let mut body_text = String::new();
        let mut k = 0;
        while k < n {
            let (b, piece, branch) = &pieces[k];
            if self.jumped[b.index()] {
                let _ = writeln!(body_text, "bb{}:;", b.index());
            }
            body_text += piece;
            match branch {
                Some((c, _)) if folded[k] => {
                    let _ = writeln!(body_text, "    if ({c}) {{ {} }}", pieces[k + 1].1.trim());
                    k += 2;
                    continue;
                }
                Some((c, els)) => {
                    let _ = writeln!(body_text, "    if (!{c}) goto bb{};", els.index());
                }
                None => {}
            }
            k += 1;
        }
        // Declare only the locals the code names: a flag or a return slot that
        // was folded away would otherwise sit there unused, and GCC's choices
        // on small recursive functions change with such declarations.
        let named = named_locals(&body_text);
        let mut text = String::new();
        if self.hir.closures.contains_key(&body.func) {
            let ty = c_type(body.locals[Local(1)].ty);
            let _ = writeln!(text, "    {ty} l1 = ({ty})env;");
        }
        for (l, d) in body.locals.iter_enumerated() {
            if !named.contains(&l.index()) || (l.index() <= body.arg_count && l.index() != 0) || d.ty == Type::None || self.consts[l.index()] {
                continue;
            }
            // Strings start empty, which is what `Drop` relies on. Nothing
            // else is read before it is assigned (validated), so nothing
            // else is initialised: the C compiler would only have to remove it.
            if d.ty == Type::Str {
                let _ = writeln!(text, "    drachm_str l{} = {{0}};", l.index());
            } else if d.ty.is_list() {
                let _ = writeln!(text, "    drachm_list l{} = {{0}};", l.index());
            } else {
                let _ = writeln!(text, "    {} l{};", c_type(d.ty), l.index());
            }
        }
        text += &body_text;
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

    /// The left-hand side of an assignment to `dest`: a `const` declaration
    /// for a scalar assigned only here. GCC treats such single-assignment
    /// locals better than variables declared up front (measured on `fib`).
    fn decl(&self, dest: &Place, ty: Type) -> String {
        let lv = self.lvalue(dest);
        if dest.proj.is_empty() && self.consts[dest.local.index()] { format!("const {} {lv}", c_type(ty)) } else { lv }
    }

    /// Locals that can be declared `const` at their one assignment.
    fn const_locals(&self) -> Vec<bool> {
        let body = self.body;
        let n = body.locals.len();
        let mut defs = vec![0usize; n];
        let mut ok = vec![true; n];
        for block in body.blocks.iter() {
            for st in &block.statements {
                if let StatementKind::Assign(d, rv) = &st.kind {
                    defs[d.local.index()] += 1;
                    let wide = |t: Type| matches!(t, Type::Int { bits: 128, .. });
                    let simple = match rv {
                        // These two can need a helper block with its own scope.
                        Rvalue::IntPow(..) => false,
                        Rvalue::Convert(op, to) => !wide(*to) && !wide(self.op_ty(op)),
                        _ => true,
                    };
                    if !d.proj.is_empty() || !simple {
                        ok[d.local.index()] = false;
                    }
                }
            }
            if let TerminatorKind::Call { callee, dest: Some(d), .. } = &block.terminator.kind {
                defs[d.local.index()] += 1;
                if !d.proj.is_empty() || !matches!(callee, Callee::Func(_) | Callee::Str(_)) {
                    ok[d.local.index()] = false;
                }
            }
        }
        (0..n)
            .map(|i| {
                let d = &body.locals[Local(i as u32)];
                i > body.arg_count
                    && defs[i] == 1
                    && ok[i]
                    && !d.in_memory
                    && matches!(d.kind, LocalKind::Temp | LocalKind::User { .. })
                    && matches!(d.ty, Type::Int { .. } | Type::Float(_) | Type::Bool)
            })
            .collect()
    }

    // ---- places and operands -----------------------------------------------

    fn lvalue(&self, p: &Place) -> String {
        let mut s = format!("l{}", p.local.index());
        let mut ty = self.body.locals[p.local].ty;
        let mut variant = None;
        for pr in &p.proj {
            match *pr {
                Proj::Downcast(v) => variant = Some(v),
                Proj::Field(i) if variant.is_some() => {
                    let v = variant.take().unwrap();
                    let f = &self.hir.enum_def(ty).variants[v].fields[i];
                    let _ = write!(s, ".u.v{v}.f_{}", f.name);
                    ty = f.ty;
                }
                Proj::Field(i) if matches!(ty, Type::Dyn { .. }) => {
                    let Type::Dyn { id, .. } = ty else { unreachable!() };
                    let iface = &self.hir.dyn_ifaces[id];
                    let fty = iface.fields[i].ty;
                    s = format!("(*({} *)((uint8_t *){s}.data + (size_t)({s}.vt[{}])))", c_type(fty), iface.field_slot(i));
                    ty = fty;
                }
                Proj::Field(i) => {
                    let f = &self.hir.struct_def(ty).fields[i];
                    let _ = write!(s, ".f_{}", f.name);
                    ty = f.ty;
                }
                // The fat borrow itself stands for the interface behind it.
                Proj::Deref if ty.is_fat() => ty = ty.target().expect("a borrow"),
                Proj::Deref => {
                    s = format!("(*{s})");
                    ty = ty.target().expect("deref of a non-pointer");
                }
                Proj::Index(i) => {
                    let elem = ty.elem().expect("an item of a list or array");
                    s = if ty.is_list() { format!("(({} *){s}.ptr)[l{}]", c_type(elem), i.index()) } else { format!("{s}.a[l{}]", i.index()) };
                    ty = elem;
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
            Operand::Const(Constant::Int(v, ty)) if ty.is_ptr() => {
                debug_assert_eq!(*v, 0, "the only pointer constant is none");
                format!("(({})0)", c_type(*ty))
            }
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
                    let lhs = self.decl(dest, ty);
                    self.assign(&lhs, ty, rv);
                }
            }
            StatementKind::Drop { place, flag, .. } => {
                let ty = place_ty(self.body, self.hir, place).expect("validated MIR");
                let lv = self.lvalue(place);
                let call = match ty {
                    // `drachm_str_drop` starts with this test; doing it here skips the call.
                    Type::Str => format!("if ({lv}.cap != 0) drachm_str_drop(&{lv});"),
                    Type::Struct { .. } | Type::Enum { .. } | Type::List { .. } | Type::Array { .. } | Type::Map { .. } if crate::mir::needs_drop(self.hir, ty) => {
                        format!("drachm_drop_{}(&{lv});", glue_name(ty))
                    }
                    _ => return,
                };
                match flag {
                    Some(f) => self.line(format!("if (l{}) {call}", f.index())),
                    None => self.line(call),
                }
            }
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
            // A reborrow of the interface behind a fat borrow is the fat borrow.
            Rvalue::Ref(_, p) if matches!(place_ty(self.body, self.hir, p), Ok(Type::Dyn { .. })) => self.lvalue(p),
            Rvalue::Ref(_, p) | Rvalue::RawRef(p) => format!("&{}", self.lvalue(p)),
            Rvalue::MakeFn(id, _, env) => {
                let env = env.as_ref().map_or_else(|| "NULL".into(), |p| format!("(uint8_t *)&{}", self.lvalue(p)));
                format!("(drachm_fn){{ (void (*)(void)){}, {env} }}", self.hir.funcs[*id].symbol)
            }
            Rvalue::Unsize(op, to) => {
                let from = self.op_ty(op);
                let x = self.operand(op);
                let Some(Type::Dyn { id: k, .. }) = to.target() else { unreachable!("unsized to an interface") };
                match from.target() {
                    // An upcast: the table of the same type for the ancestor.
                    Some(Type::Dyn { id: j, .. }) => {
                        let slot = self.hir.dyn_ifaces[j].ancestor_slot(k).expect("an ancestor");
                        format!("(drachm_dyn){{ {x}.data, (const void *const *){x}.vt[{slot}] }}")
                    }
                    _ => {
                        let src = from.target().expect("a borrow");
                        format!("(drachm_dyn){{ (void *){x}, drachm_vt_{} }}", self.hir.vtable_index(src, k))
                    }
                }
            }
            Rvalue::Aggregate(sid, ops) => {
                let def = &self.hir.structs[*sid];
                let parts: Vec<String> = ops.iter().enumerate().map(|(i, op)| format!(".f_{} = {}", def.fields[i].name, self.operand(op))).collect();
                format!("({}){{ {} }}", crate::cgen_common::struct_name(&def.name), parts.join(", "))
            }
            Rvalue::Variant(eid, vi, ops) => {
                let v = &self.hir.enums[*eid].variants[*vi];
                let parts: Vec<String> = ops.iter().zip(&v.fields).map(|(op, f)| format!(".f_{} = {}", f.name, self.operand(op))).collect();
                let name = crate::cgen_common::enum_name(*eid);
                if parts.is_empty() {
                    format!("({name}){{ .tag = {vi} }}")
                } else {
                    format!("({name}){{ .tag = {vi}, .u.v{vi} = {{ {} }} }}", parts.join(", "))
                }
            }
            Rvalue::Discriminant(p) => format!("{}.tag", self.lvalue(p)),
            Rvalue::Len(p) => {
                let pty = place_ty(self.body, self.hir, p).expect("validated MIR");
                match pty {
                    Type::Array { len, .. } => format!("UINT64_C({len})"),
                    _ => format!("{}.len", self.lvalue(p)),
                }
            }
            Rvalue::MapNew(t) => {
                let _ = t;
                self.line(format!("drachm_map_new(&{lv});"));
                return;
            }
            Rvalue::ListNew(t, cap) => {
                if *cap == 0 {
                    format!("(drachm_list){{0}}")
                } else {
                    let c = c_type(t.elem().expect("a list"));
                    self.line(format!("drachm_list_with_cap(&{lv}, {cap}, sizeof({c}), _Alignof({c}));"));
                    return;
                }
            }
            Rvalue::Array(t, ops) => {
                let items: Vec<String> = ops.iter().map(|o| self.operand(o)).collect();
                format!("({}){{ {{ {} }} }}", c_type(*t), items.join(", "))
            }
            Rvalue::DeepCopy(p) => {
                let src = self.lvalue(p);
                match ty {
                    Type::Str => self.line(format!("drachm_str_copy(&{lv}, &{src});")),
                    Type::Struct { .. } | Type::Enum { .. } | Type::List { .. } | Type::Array { .. } | Type::Map { .. } if holds_string(self.hir, ty) => {
                        self.line(format!("drachm_copy_{}(&{lv}, &{src});", glue_name(ty)))
                    }
                    _ => self.line(format!("{lv} = {src};")),
                }
                return;
            }
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

    fn terminator(&mut self, k: &TerminatorKind, next: Option<BlockId>, cond_text: Option<String>) {
        match k {
            TerminatorKind::Goto(t) => self.goto(*t, next),
            TerminatorKind::If { cond, then, els } => {
                let c = cond_text.unwrap_or_else(|| self.operand(cond));
                if then == els {
                    self.goto(*then, next);
                } else if Some(*then) == next {
                    self.branch_over = Some((c, *els));
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
                if let Some(v) = cond_text {
                    self.line(format!("return {v};"));
                } else if self.body.locals[Local(0)].ty == Type::None {
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
            TerminatorKind::Switch { discr, cases, otherwise } => {
                let d = self.operand(discr);
                let mut text = format!("switch ({d}) {{");
                for (v, t) in cases {
                    self.jumped[t.index()] = true;
                    let _ = write!(text, " case {v}: goto bb{};", t.index());
                }
                self.jumped[otherwise.index()] = true;
                let _ = write!(text, " default: goto bb{}; }}", otherwise.index());
                self.line(text);
            }
            // `PanicKind` has no values.
            TerminatorKind::Panic(_) => unreachable!("the builder never emits {k:?}"),
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
                    (Some(_), t) if t != Type::None => {
                        let d = self.decl(dest.unwrap(), t);
                        self.line(format!("{d} = {call};"))
                    }
                    _ => self.line(format!("{call};")),
                }
            }
            Callee::Dyn { .. } | Callee::Fn(_) => {
                let fat = self.operand(&args[0].op);
                let (ret, args_ty, env_ty, env, code) = match callee {
                    Callee::Dyn { iface, method } => {
                        let iface = &self.hir.dyn_ifaces[iface];
                        let m = &iface.methods[method];
                        let slot = iface.method_slot(method);
                        (m.ret, m.params.as_slice(), "void *", format!("{fat}.data"), format!("{fat}.vt[{slot}]"))
                    }
                    Callee::Fn(Type::Func(sig)) => (sig.ret, sig.params.as_slice(), "uint8_t *", format!("{fat}.env"), format!("{fat}.code")),
                    _ => unreachable!("an indirect callee"),
                };
                let mut params = vec![env_ty.to_string()];
                params.extend(args_ty.iter().map(|&p| c_type(p)));
                let fn_ty = format!("{} (*)({})", c_type(ret), params.join(", "));
                let mut all = vec![env];
                all.extend(args[1..].iter().map(|a| self.operand(&a.op)));
                let call = format!("(({fn_ty}){code})({})", all.join(", "));
                match (&dest_lv, ret) {
                    (Some(_), t) if t != Type::None => {
                        let d = self.decl(dest.unwrap(), t);
                        self.line(format!("{d} = {call};"))
                    }
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
                    (Some(d), t) if crate::codegen_common::in_memory(t) => self.line(format!("{}(&{d}, {a});", f.name())),
                    (Some(_), t) if t != Type::None => {
                        let d = self.decl(dest.unwrap(), t);
                        self.line(format!("{d} = {}({a});", f.name()))
                    }
                    _ => self.line(format!("{}({a});", f.name())),
                }
            }
            Callee::Out { newline } => {
                let p = self.operand(&args[0].op);
                self.line(format!("drachm_out_str((const char *)(*{p}).ptr, (*{p}).len, {});", newline as u8));
            }
            Callee::InLine => {
                let dest = dest_lv.expect("a string result has a destination");
                self.line(format!("drachm_in_line(&{dest});"));
            }
            Callee::InEnded => {
                let dest = dest_lv.expect("a bool result has a destination");
                self.line(format!("{dest} = drachm_in_ended();"));
            }
            Callee::Parse(ty) => {
                let dest = dest_lv.expect("a number result has a destination");
                let text = self.operand(&args[0].op);
                match ty {
                    Type::Float(_) => {
                        let bits = if ty == Type::Float(32) { 32 } else { 64 };
                        self.line(format!("{dest} = ({})drachm_parse_float({text}, {bits});", c_type(ty)));
                    }
                    Type::Int { bits: 128, signed } => {
                        self.line(format!(
                            "{{ const drachm_pair p = drachm_parse_int({text}, {}, 128); {dest} = ({})drachm_join(p.lo, p.hi); }}",
                            signed as u8,
                            c_type(ty)
                        ));
                    }
                    Type::Int { bits, signed } => {
                        self.line(format!("{dest} = ({})drachm_parse_int({text}, {}, {bits}).lo;", c_type(ty), signed as u8));
                    }
                    _ => unreachable!("checked by typeck"),
                }
            }
            Callee::ParseStatus(ty) => {
                let dest = dest_lv.expect("a status has a destination");
                let text = self.operand(&args[0].op);
                let (kind, signed, bits) = match ty {
                    Type::Float(32) => (1, 0, 32),
                    Type::Float(_) => (2, 0, 64),
                    Type::Int { bits, signed } => (0, signed as u8, bits as u32),
                    _ => unreachable!("checked by typeck"),
                };
                self.line(format!("{dest} = drachm_parse_status({text}, {kind}, {signed}, {bits});"));
            }
            Callee::CauseWrap => {
                let dest = dest_lv.expect("a word result has a destination");
                let (msg, slot) = (self.operand(&args[0].op), self.operand(&args[1].op));
                self.line(format!("{dest} = drachm_cause_wrap({msg}, {slot});"));
            }
            Callee::ReportExit => {
                let (msg, slot, code) = (self.operand(&args[0].op), self.operand(&args[1].op), self.operand(&args[2].op));
                self.line(format!("drachm_report_exit({msg}, {slot}, {code});"));
            }
            Callee::Report => {
                let (msg, slot) = (self.operand(&args[0].op), self.operand(&args[1].op));
                self.line(format!("drachm_report({msg}, {slot});"));
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
                    _ => unreachable!("checked by typeck"),
                };
                self.line(format!("{call};"));
            }
            Callee::Radix { base, upper } => {
                let dest = dest_lv.expect("a string result has a destination");
                let ty = self.op_ty(&args[0].op);
                let v = self.operand(&args[0].op);
                self.line(format!("drachm_str_radix(&{dest}, {}, {}, {base}, {});", halves(&v, ty), signed(ty) as u8, upper as u8));
            }
            Callee::ListOp(op, elem) => self.list_op(op, elem, args, dest_lv.as_deref()),
            Callee::MapOp(f, mt) => self.map_call(f, mt, args, dest, dest_lv.as_deref()),
            Callee::ListHas(elem) => {
                let (list, idx) = (self.operand(&args[0].op), self.operand(&args[1].op));
                let d = self.decl(dest.unwrap(), Type::Bool);
                self.line(format!("{d} = drachm_list_get({list}, {idx}, sizeof({})) != NULL;", c_type(elem)));
            }
            Callee::ListPush(elem) => {
                let list = self.operand(&args[0].op);
                let c = c_type(elem);
                let value = self.operand(&args[1].op);
                // Room left: store in place; otherwise the runtime grows the list.
                self.line(format!(
                    "{{ drachm_list *L = {list}; {c} v = {value}; if (L->len < L->cap) (({c} *)L->ptr)[L->len++] = v; else drachm_list_push(L, &v, sizeof({c}), _Alignof({c})); }}"
                ));
            }
            Callee::IndexPanic { unsigned } => {
                let (i, len) = (self.operand(&args[0].op), self.operand(&args[1].op));
                let (f, t) = if unsigned { ("drachm_index_panic_u", "uint64_t") } else { ("drachm_index_panic", "int64_t") };
                self.line(format!("{f}(({t}){i}, {len});"));
            }
            Callee::Join => {
                let dest = dest_lv.expect("a string result has a destination");
                let parts: Vec<String> = args.iter().map(|a| format!("*{}", self.operand(&a.op))).collect();
                self.line(format!("drachm_str_join(&{dest}, (const drachm_str[]){{{}}}, {});", parts.join(", "), parts.len()));
            }
        }
    }
}

impl Gen<'_> {
    /// A list method with a runtime call (see `hir::ListOp`), as C.
    fn list_op(&mut self, op: hir::ListOp, elem: Type, args: &[Arg], dest_lv: Option<&str>) {
        use hir::ListOp;
        let list = self.operand(&args[0].op);
        let c = c_type(elem);
        match op {
            ListOp::Pop | ListOp::View => unreachable!("`pop` is a test and a `Remove`, a view is a borrow, not a call"),
            ListOp::Get => {
                let (idx, dest) = (self.operand(&args[1].op), dest_lv.expect("an item has a destination"));
                let copy = self.copy_entry(elem, dest, "g");
                self.line(format!("{{ void *g = drachm_list_get({list}, {idx}, sizeof({c})); if (g) {{ {copy} }} }}"));
            }
            ListOp::Remove | ListOp::SwapRemove => {
                let (idx, dest) = (self.operand(&args[1].op), dest_lv.expect("an item has a destination"));
                let name = if op == ListOp::Remove { "drachm_list_remove" } else { "drachm_list_swap_remove" };
                self.line(format!("{name}({list}, {idx}, &{dest}, sizeof({c}));"));
            }
            ListOp::TakeLeaving => {
                let (idx, value) = (self.operand(&args[1].op), self.operand(&args[2].op));
                let dest = dest_lv.expect("an item has a destination");
                self.line(format!("{{ {c} v = {value}; drachm_list_replace({list}, {idx}, &v, &{dest}, sizeof({c})); }}"));
            }
            ListOp::Insert => {
                let (idx, value) = (self.operand(&args[1].op), self.operand(&args[2].op));
                self.line(format!("{{ {c} v = {value}; drachm_list_insert({list}, {idx}, &v, sizeof({c}), _Alignof({c})); }}"));
            }
            ListOp::Clear => {
                let f = if !crate::mir::needs_drop(self.hir, elem) {
                    "NULL".to_string()
                } else if elem == Type::Str {
                    "(drachm_drop_fn)drachm_str_drop".to_string()
                } else {
                    format!("(drachm_drop_fn)drachm_drop_{}", glue_name(elem))
                };
                self.line(format!("drachm_list_clear({list}, sizeof({c}), {f});"));
            }
            ListOp::Join => {
                let dest = dest_lv.expect("a string result has a destination");
                let sep = self.operand(&args[1].op);
                self.line(format!("drachm_list_join(&{dest}, {list}, {sep});"));
            }
        }
    }
}

/// How many places of each local the body mentions (a definition counts).
fn mentions(body: &Body) -> Vec<usize> {
    let mut n = vec![0usize; body.locals.len()];
    let op = |o: &Operand, n: &mut Vec<usize>| {
        if let Some(p) = crate::mir::cleanup::operand_place(o) {
            n[p.local.index()] += 1;
        }
    };
    for block in body.blocks.iter() {
        for s in &block.statements {
            match &s.kind {
                StatementKind::Assign(d, rv) => {
                    n[d.local.index()] += 1;
                    for p in crate::mir::cleanup::rvalue_places(rv) {
                        n[p.local.index()] += 1;
                    }
                }
                StatementKind::Drop { place, flag, .. } => {
                    n[place.local.index()] += 1;
                    if let Some(f) = flag {
                        n[f.index()] += 1;
                    }
                }
                StatementKind::FakeRead(p) => n[p.local.index()] += 1,
                _ => {}
            }
        }
        match &block.terminator.kind {
            TerminatorKind::If { cond, .. } => op(cond, &mut n),
            TerminatorKind::Switch { discr, .. } => op(discr, &mut n),
            TerminatorKind::Call { args, dest, .. } => {
                for a in args {
                    op(&a.op, &mut n);
                }
                if let Some(d) = dest {
                    n[d.local.index()] += 1;
                }
            }
            _ => {}
        }
    }
    n
}

/// The numbers N of every `lN` in `text`.
fn named_locals(text: &str) -> std::collections::BTreeSet<usize> {
    let bytes = text.as_bytes();
    let mut found = std::collections::BTreeSet::new();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'l' && (i == 0 || !ident(bytes[i - 1])) {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && (j == bytes.len() || !ident(bytes[j])) {
                found.insert(text[i + 1..j].parse().unwrap());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

// ---- maps (S6D) ----

/// The C functions that hash and compare keys of each tag used by a map type.
fn key_fns_c(hir: &hir::Program) -> String {
    let mut tags: Vec<(String, Type)> = Vec::new();
    for t in &hir.collections {
        if let Some((k, _)) = t.map_parts() {
            let tag = key_tag(k);
            if !tags.iter().any(|(t, _)| *t == tag) {
                tags.push((tag, k.bare()));
            }
        }
    }
    let mut out = String::new();
    let mut walker = KeyC { hir, next: 0 };
    for (tag, key) in tags {
        let (hash, eq) = match tag.as_str() {
            _ if key_is_composite(key) => {
                let c = c_type(key);
                let (mut h, mut e) = (String::new(), String::new());
                walker.hash(&format!("(*(const {c} *)p)"), key, &mut h);
                walker.eq(&format!("(*(const {c} *)a)"), &format!("(*(const {c} *)b)"), key, "return 0;", &mut e);
                (format!("uint64_t h = 0;\n{h}return h;\n"), format!("{e}return 1;\n"))
            }
            "str" => (
                "return drachm_str_hash((const drachm_str *)p);".to_string(),
                "return drachm_str_eq((const drachm_str *)a, (const drachm_str *)b);".to_string(),
            ),
            "u128" => (
                "drachm_u128 n; memcpy(&n, p, sizeof n); return drachm_hash_combine(drachm_hash_u64((uint64_t)n), (uint64_t)(n >> 64));".to_string(),
                "return memcmp(a, b, sizeof(drachm_u128)) == 0;".to_string(),
            ),
            _ => {
                let c = match tag.as_str() {
                    "u8" => "uint8_t",
                    "u16" => "uint16_t",
                    "u32" => "uint32_t",
                    _ => "uint64_t",
                };
                (format!("{c} n; memcpy(&n, p, sizeof n); return drachm_hash_u64(n);"), format!("return memcmp(a, b, sizeof({c})) == 0;"))
            }
        };
        let _ = writeln!(out, "\nstatic uint64_t drachm_keyhash_{tag}(const void *p) {{ {hash} }}");
        let _ = writeln!(out, "static uint8_t drachm_keyeq_{tag}(const void *a, const void *b) {{ {eq} }}");
    }
    out
}

/// Hash and equality of composite keys, as C, walking the key's type.
struct KeyC<'a> {
    hir: &'a hir::Program,
    next: usize,
}

impl KeyC<'_> {
    fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    /// Statements that fold the value `a` (an lvalue of type `ty`) into `h`.
    fn hash(&mut self, a: &str, ty: Type, out: &mut String) {
        match ty.bare() {
            Type::Str => {
                let _ = writeln!(out, "h = drachm_hash_combine(h, drachm_str_hash(&{a}));");
            }
            Type::Struct { id, .. } => {
                let def = &self.hir.structs[id];
                if let Some(f) = def.key_fn {
                    let (n, ret) = (self.id(), self.hir.funcs[f].ret);
                    let _ = writeln!(out, "{{ {} r{n} = {}(({} *)&{a});", c_type(ret), self.hir.funcs[f].symbol, c_type(ty));
                    self.hash(&format!("r{n}"), ret, out);
                    out.push_str(&self.drop_result(&format!("r{n}"), ret));
                    out.push_str("}\n");
                } else {
                    for f in &def.fields {
                        self.hash(&format!("{a}.f_{}", f.name), f.ty, out);
                    }
                }
            }
            t @ Type::Enum { .. } => {
                let def = self.hir.enum_def(t);
                let _ = writeln!(out, "h = drachm_hash_combine(h, (uint64_t){a}.tag);\nswitch ({a}.tag) {{");
                for (vi, v) in def.variants.iter().enumerate() {
                    let mut body = String::new();
                    for f in &v.fields {
                        self.hash(&format!("{a}.u.v{vi}.f_{}", f.name), f.ty, &mut body);
                    }
                    if !body.is_empty() {
                        let _ = writeln!(out, "case {vi}: {{\n{body}break; }}");
                    }
                }
                out.push_str("default: break;\n}\n");
            }
            Type::Array { elem, len } => {
                let n = self.id();
                let mut body = String::new();
                self.hash(&format!("{a}.a[i{n}]"), *elem, &mut body);
                let _ = writeln!(out, "for (uint64_t i{n} = 0; i{n} < {len}; i{n}++) {{\n{body}}}");
            }
            t => match crate::hir::scalar_size(t) {
                Some(16) => {
                    let _ = writeln!(out, "{{ drachm_u128 n; memcpy(&n, &{a}, sizeof n); h = drachm_hash_combine(drachm_hash_combine(h, (uint64_t)n), (uint64_t)(n >> 64)); }}");
                }
                Some(size) => {
                    let _ = writeln!(out, "{{ uint{}_t n; memcpy(&n, &{a}, sizeof n); h = drachm_hash_combine(h, n); }}", size * 8);
                }
                None => unreachable!("checked by typeck"),
            },
        }
    }

    /// Statements that run `fail` when the values `a` and `b` (lvalues of type `ty`) differ.
    fn eq(&mut self, a: &str, b: &str, ty: Type, fail: &str, out: &mut String) {
        match ty.bare() {
            Type::Str => {
                let _ = writeln!(out, "if (!drachm_str_eq(&{a}, &{b})) {fail}");
            }
            Type::Struct { id, .. } => {
                let def = &self.hir.structs[id];
                if let Some(f) = def.key_fn {
                    let (n, ret) = (self.id(), self.hir.funcs[f].ret);
                    let (sym, c) = (&self.hir.funcs[f].symbol, c_type(ty));
                    let _ = writeln!(out, "{{ {r} ra{n} = {sym}(({c} *)&{a}); {r} rb{n} = {sym}(({c} *)&{b}); int same{n} = 1;", r = c_type(ret));
                    self.eq(&format!("ra{n}"), &format!("rb{n}"), ret, &format!("{{ same{n} = 0; goto end{n}; }}"), out);
                    let _ = writeln!(out, "end{n}:;");
                    out.push_str(&self.drop_result(&format!("ra{n}"), ret));
                    out.push_str(&self.drop_result(&format!("rb{n}"), ret));
                    let _ = writeln!(out, "if (!same{n}) {fail} }}");
                } else {
                    for f in &def.fields {
                        self.eq(&format!("{a}.f_{}", f.name), &format!("{b}.f_{}", f.name), f.ty, fail, out);
                    }
                }
            }
            t @ Type::Enum { .. } => {
                let def = self.hir.enum_def(t);
                let _ = writeln!(out, "if ({a}.tag != {b}.tag) {fail}\nswitch ({a}.tag) {{");
                for (vi, v) in def.variants.iter().enumerate() {
                    let mut body = String::new();
                    for f in &v.fields {
                        self.eq(&format!("{a}.u.v{vi}.f_{}", f.name), &format!("{b}.u.v{vi}.f_{}", f.name), f.ty, fail, &mut body);
                    }
                    if !body.is_empty() {
                        let _ = writeln!(out, "case {vi}: {{\n{body}break; }}");
                    }
                }
                out.push_str("default: break;\n}\n");
            }
            Type::Array { elem, len } => {
                let n = self.id();
                let mut body = String::new();
                self.eq(&format!("{a}.a[i{n}]"), &format!("{b}.a[i{n}]"), *elem, fail, &mut body);
                let _ = writeln!(out, "for (uint64_t i{n} = 0; i{n} < {len}; i{n}++) {{\n{body}}}");
            }
            _ => {
                let _ = writeln!(out, "if (memcmp(&{a}, &{b}, sizeof {a}) != 0) {fail}");
            }
        }
    }

    /// Drops the owned result of a `key()` call held in `r`.
    fn drop_result(&self, r: &str, ty: Type) -> String {
        match ty {
            Type::Str => format!("if ({r}.cap != 0) drachm_str_drop(&{r});\n"),
            _ if crate::mir::needs_drop(self.hir, ty) => format!("drachm_drop_{}(&{r});\n", glue_name(ty)),
            _ => String::new(),
        }
    }
}

/// The address of the static runtime operations table for this map.
fn map_ops_c(mt: Type) -> String {
    format!("&drachm_ops_{}", glue_name(map_ops_type(mt)))
}

fn map_ops_init(hir: &hir::Program, mt: Type) -> String {
    let (k, v) = mt.map_parts().expect("a map has key and value types");
    let (ks, ka) = hir.size_align(k);
    let (vs, va) = hir.size_align(v);
    let drop_of = |v: Type| match v {
        _ if !crate::mir::needs_drop(hir, v) => "NULL".to_string(),
        Type::Str => "(drachm_drop_fn)drachm_str_drop".to_string(),
        _ => format!("(drachm_drop_fn)drachm_drop_{}", glue_name(v)),
    };
    let copy_of = |v: Type| match v {
        _ if !holds_string(hir, v) => "NULL".to_string(),
        Type::Str => "(drachm_copy_fn)drachm_str_copy".to_string(),
        _ => format!("(drachm_copy_fn)drachm_copy_{}", glue_name(v)),
    };
    let (drop_k, copy_k, drop_v, copy_v) = (drop_of(k), copy_of(k), drop_of(v), copy_of(v));
    let tag = key_tag(k);
    format!(
        "{{ .ksize = {ks}, .kalign = {ka}, .vsize = {vs}, .valign = {va}, .hash = drachm_keyhash_{tag}, .eq = drachm_keyeq_{tag}, .drop_key = {drop_k}, .drop_val = {drop_v}, .copy_key = {copy_k}, .copy_val = {copy_v} }}"
    )
}

impl Gen<'_> {
    /// A map call (`mir::MapFn`), as C. `args[0]` is the map's address.
    fn map_call(&mut self, f: MapFn, mt: Type, args: &[Arg], dest: Option<&Place>, dest_lv: Option<&str>) {
        let (k, v) = mt.map_parts().expect("a map call is on a map");
        let map = self.operand(&args[0].op);
        let ops = map_ops_c(mt);
        match f {
            MapFn::Len => {
                let d = self.decl(dest.expect("a length has a destination"), Type::U64);
                self.line(format!("{d} = drachm_map_len({map});"));
            }
            MapFn::Get(result) => {
                let key = self.operand(&args[1].op);
                let dst = dest_lv.expect("a lookup has a destination");
                let field = &self.hir.enum_def(result).variants[1].fields[0].name;
                let out = format!("{dst}.u.v1.f_{field}");
                let copy = if self.hir_holds_string(v) {
                    match v {
                        Type::Str => format!("drachm_str_copy(&{out}, (const drachm_str *)at);"),
                        _ => format!("drachm_copy_{}(&{out}, (const {} *)at);", glue_name(v), c_type(v)),
                    }
                } else {
                    format!("memcpy(&{out}, at, sizeof {out});")
                };
                self.line(format!(
                    "{{ const void *at = drachm_map_find({map}, {key}, {ops}); if (at) {{ {copy} {dst}.tag = 1; }} else {{ {dst}.tag = 0; }} }}"
                ));
            }
            MapFn::Set => {
                let (key, val) = (self.operand(&args[1].op), self.operand(&args[2].op));
                self.line(format!("{{ {} drachm_k = {key}; {} drachm_v = {val}; drachm_map_insert({map}, &drachm_k, &drachm_v, {ops}); }}", c_type(k), c_type(v)));
            }
            MapFn::Remove => {
                let key = self.operand(&args[1].op);
                self.line(format!("drachm_map_remove({map}, {key}, NULL, {ops});"));
            }
            MapFn::Expect => {
                let key = self.operand(&args[1].op);
                let dst = dest_lv.expect("a lookup has a destination");
                let copy = self.copy_entry(v, dst, "at");
                self.line(format!("{{ const void *at = drachm_map_find({map}, {key}, {ops}); if (!at) drachm_map_key_panic(); {copy} }}"));
            }
            MapFn::More | MapFn::KeyAt | MapFn::ValAt | MapFn::After => {
                let cursor = self.operand(&args[1].op);
                let next = format!("uint64_t c = {cursor}; void *kp = NULL, *vp = NULL; uint8_t found = drachm_map_next({map}, &c, &kp, &vp, {ops}); (void)found; (void)kp; (void)vp;");
                match f {
                    MapFn::More => {
                        let d = self.decl(dest.expect("a test has a destination"), Type::Bool);
                        self.line(format!("{d} = ({{ {next} found; }});"));
                    }
                    MapFn::After => {
                        let d = self.decl(dest.expect("a cursor has a destination"), Type::U64);
                        self.line(format!("{d} = ({{ {next} c; }});"));
                    }
                    _ => {
                        let (ty, at) = if f == MapFn::KeyAt { (k, "kp") } else { (v, "vp") };
                        let dst = dest_lv.expect("an entry part has a destination");
                        let copy = self.copy_entry(ty, dst, at);
                        self.line(format!("{{ {next} {copy} }}"));
                    }
                }
            }
        }
    }

    /// A statement that copies the entry part of type `ty` that `at` points to into the lvalue `dst`.
    fn copy_entry(&self, ty: Type, dst: &str, at: &str) -> String {
        if !self.hir_holds_string(ty) {
            return format!("memcpy(&{dst}, {at}, sizeof {dst});");
        }
        match ty {
            Type::Str => format!("drachm_str_copy(&{dst}, (const drachm_str *){at});"),
            _ => format!("drachm_copy_{}(&{dst}, (const {} *){at});", glue_name(ty), c_type(ty)),
        }
    }

    fn hir_holds_string(&self, t: Type) -> bool {
        crate::mir::drops::holds_string(self.hir, t)
    }
}
