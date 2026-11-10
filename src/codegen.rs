//! The fast backend: `hir` → Cranelift IR → object file. Used for plain
//! (non `-O`) builds. Must behave exactly like the C backend in `cgen`.
//!
//! Strings live in 24-byte stack slots laid out as `drachm_str` (pointer,
//! length, capacity) and go to the runtime by pointer. Between Drachm
//! functions a string parameter is passed as a pointer to a value the callee
//! takes over (it copies the 24 bytes into its own slot and drops it later),
//! and a string result is written through a hidden first parameter that
//! points at where the caller wants it.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    types, AbiParam, InstBuilder, MemFlagsData, Signature, StackSlot, StackSlotData, StackSlotKind, TrapCode, UserFuncName, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::hir::*;
use crate::types::Type;

type ClType = types::Type;
type ClBlock = cranelift_codegen::ir::Block;

fn cl_type(t: Type) -> Option<ClType> {
    match t {
        Type::Int { bits, .. } => ClType::int(bits),
        Type::Float(32) => Some(types::F32),
        Type::Float(_) => Some(types::F64),
        Type::Bool => Some(types::I8),
        Type::None | Type::Str => None,
    }
}

/// Size of a `drachm_str`, and the offsets of its fields.
const STR_SIZE: u32 = 24;
const STR_LEN: i32 = 8;
const STR_CAP: i32 = 16;

fn is_signed(t: Type) -> bool {
    matches!(t, Type::Int { signed: true, .. })
}

fn err(e: impl std::fmt::Debug) -> String {
    format!("{e:?}")
}

pub fn compile(prog: &Program) -> Result<Vec<u8>, String> {
    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(err)?;
    flags.set("opt_level", "speed").map_err(err)?;
    // Lets 128-bit integers appear in (Drachm-to-Drachm) function signatures.
    flags.set("enable_llvm_abi_extensions", "true").map_err(err)?;
    let isa = cranelift_native::builder().map_err(err)?.finish(settings::Flags::new(flags)).map_err(err)?;
    let builder = ObjectBuilder::new(isa, "drachm", cranelift_module::default_libcall_names()).map_err(err)?;
    let mut module = ObjectModule::new(builder);

    // Declare every function up front so calls can be emitted in any order.
    let ptr = module.target_config().pointer_type();
    let mut ids = Vec::new();
    for f in &prog.funcs {
        let mut s = module.make_signature();
        if f.ret == Type::Str {
            s.params.push(AbiParam::new(ptr));
        }
        s.params.extend(f.locals[..f.params].iter().map(|l| AbiParam::new(cl_type(l.ty).unwrap_or(ptr))));
        s.returns.extend(cl_type(f.ret).map(AbiParam::new));
        ids.push(module.declare_function(&format!("drachm_f_{}", f.name), Linkage::Local, &s).map_err(err)?);
    }

    let mut ctx = module.make_context();
    let mut fctx = FunctionBuilderContext::new();
    let mut runtime = HashMap::new();
    let mut strings = HashMap::new();

    for (f, &id) in prog.funcs.iter().zip(&ids) {
        ctx.func.signature = module.declarations().get_function_decl(id).signature.clone();
        ctx.func.name = UserFuncName::user(0, id.as_u32());
        let cfg = module.isa().frontend_config();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            b.seal_block(entry);

            let params = b.block_params(entry).to_vec();
            let hidden = (f.ret == Type::Str) as usize;
            let mut g = Gen {
                b,
                module: &mut module,
                ids: &ids,
                runtime: &mut runtime,
                strings: &mut strings,
                ptr,
                vars: Vec::new(),
                ret_ptr: (hidden == 1).then(|| params[0]),
                loops: HashMap::new(),
                ifs: HashMap::new(),
                terminated: false,
            };
            for (i, l) in f.locals.iter().enumerate() {
                let incoming = (i < f.params).then(|| params[hidden + i]);
                let local = match cl_type(l.ty) {
                    // Parameters arrive as block parameters; other locals start at
                    // zero (never observed: the checker rejects reads before assignment).
                    Some(ty) => {
                        let var = g.b.declare_var(ty);
                        let v = match incoming {
                            Some(v) => v,
                            None => g.zero(ty),
                        };
                        g.b.def_var(var, v);
                        Slot::Var(var)
                    }
                    // A string parameter is taken over from the caller; other
                    // strings start empty, which is what `Drop` relies on.
                    None => {
                        let slot = g.new_slot(STR_SIZE);
                        let addr = g.b.ins().stack_addr(ptr, slot, 0);
                        match incoming {
                            Some(from) => g.copy_str(addr, from),
                            None => {
                                let zero = g.b.ins().iconst(types::I64, 0);
                                g.store_str(addr, zero, zero, zero);
                            }
                        }
                        Slot::Str(slot)
                    }
                };
                g.vars.push(local);
            }
            g.block(&f.body);
            if !g.terminated {
                if f.ret == Type::None {
                    g.b.ins().return_(&[]);
                } else {
                    // The checker proved every path returns, so this is dead.
                    g.b.ins().trap(TrapCode::unwrap_user(1));
                }
            }
            g.b.finalize(cfg);
        }
        module.define_function(id, &mut ctx).map_err(|e| format!("in `{}`: {e:?}", f.name))?;
        module.clear_context(&mut ctx);
    }

    emit_c_main(&mut module, &mut ctx, &mut fctx, ids[prog.main], prog.funcs[prog.main].ret)?;
    module.finish().emit().map_err(err)
}

/// `int main(void)` that calls the Drachm `main` and turns its result into an exit code.
fn emit_c_main(
    module: &mut ObjectModule,
    ctx: &mut cranelift_codegen::Context,
    fctx: &mut FunctionBuilderContext,
    drachm_main: FuncId,
    ret: Type,
) -> Result<(), String> {
    let mut sig = Signature::new(module.isa().default_call_conv());
    sig.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("main", Linkage::Export, &sig).map_err(err)?;
    ctx.func.signature = sig;
    ctx.func.name = UserFuncName::user(0, id.as_u32());
    let cfg = module.isa().frontend_config();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, fctx);
        let entry = b.create_block();
        b.switch_to_block(entry);
        b.seal_block(entry);
        let callee = module.declare_func_in_func(drachm_main, b.func);
        let call = b.ins().call(callee, &[]);
        let code = if ret == Type::None { b.ins().iconst(types::I32, 0) } else { b.inst_results(call)[0] };
        b.ins().return_(&[code]);
        b.finalize(cfg);
    }
    module.define_function(id, ctx).map_err(err)?;
    module.clear_context(ctx);
    Ok(())
}

/// Where a local lives: numbers in SSA variables, strings in stack slots.
#[derive(Clone, Copy)]
enum Slot {
    Var(Variable),
    Str(StackSlot),
}

struct Gen<'a, 'b> {
    b: FunctionBuilder<'b>,
    module: &'a mut ObjectModule,
    ids: &'a [FuncId],
    /// Runtime functions imported so far.
    runtime: &'a mut HashMap<&'static str, FuncId>,
    /// The read-only data of each string literal emitted so far.
    strings: &'a mut HashMap<String, DataId>,
    ptr: ClType,
    vars: Vec<Slot>,
    /// Where a function that returns a string writes it.
    ret_ptr: Option<Value>,
    /// Jump targets of each `Loop`: (step, exit).
    loops: HashMap<Label, (ClBlock, ClBlock)>,
    /// Jump targets of each `If`: (the test of each branch, exit).
    ifs: HashMap<Label, (Vec<ClBlock>, ClBlock)>,
    /// The current block already ended in a terminator (e.g. after `return`).
    terminated: bool,
}

impl Gen<'_, '_> {
    /// Continue emitting into a fresh block once the current one is terminated,
    /// so code after `return` still has somewhere (unreachable) to go.
    fn ensure_open(&mut self) {
        if self.terminated {
            let dead = self.b.create_block();
            self.b.switch_to_block(dead);
            self.b.seal_block(dead);
            self.terminated = false;
        }
    }

    fn jump_to(&mut self, target: ClBlock) {
        if !self.terminated {
            self.b.ins().jump(target, &[]);
        }
    }

    fn zero(&mut self, ty: ClType) -> Value {
        match ty {
            types::F32 => self.b.ins().f32const(0.0),
            types::F64 => self.b.ins().f64const(0.0),
            _ => self.int_const(ty, 0),
        }
    }

    /// An integer constant from its (zero-extended) bits.
    fn int_const(&mut self, ty: ClType, bits: u128) -> Value {
        if ty == types::I128 {
            let lo = self.b.ins().iconst(types::I64, bits as u64 as i64);
            let hi = self.b.ins().iconst(types::I64, (bits >> 64) as u64 as i64);
            return self.b.ins().iconcat(lo, hi);
        }
        // Cranelift wants narrow immediates sign-extended.
        let shift = 64 - ty.bits();
        self.b.ins().iconst(ty, ((bits as u64 as i64) << shift) >> shift)
    }

    /// Call a runtime function, importing it on first use.
    fn rt(&mut self, name: &'static str, params: &[ClType], returns: &[ClType], args: &[Value]) -> Vec<Value> {
        let id = match self.runtime.get(name) {
            Some(id) => *id,
            None => {
                let mut sig = self.module.make_signature();
                // C expects narrow arguments widened by the caller.
                sig.params.extend(params.iter().map(|&t| if t.bits() < 32 { AbiParam::new(t).uext() } else { AbiParam::new(t) }));
                sig.returns.extend(returns.iter().copied().map(AbiParam::new));
                let id = self.module.declare_function(name, Linkage::Import, &sig).expect("runtime import");
                self.runtime.insert(name, id);
                id
            }
        };
        let callee = self.module.declare_func_in_func(id, self.b.func);
        let call = self.b.ins().call(callee, args);
        self.b.inst_results(call).to_vec()
    }

    /// Widen an integer to 64 bits, by its own signedness.
    fn widen64(&mut self, v: Value, ty: Type) -> Value {
        match self.b.func.dfg.value_type(v) {
            types::I64 => v,
            types::I128 => self.b.ins().ireduce(types::I64, v),
            _ if is_signed(ty) => self.b.ins().sextend(types::I64, v),
            _ => self.b.ins().uextend(types::I64, v),
        }
    }

    /// An integer as (low, high) 64-bit halves, extended by its own signedness.
    fn halves(&mut self, v: Value, ty: Type) -> [Value; 2] {
        let wide = match self.b.func.dfg.value_type(v) {
            types::I128 => v,
            _ if is_signed(ty) => self.b.ins().sextend(types::I128, v),
            _ => self.b.ins().uextend(types::I128, v),
        };
        let (lo, hi) = self.b.ins().isplit(wide);
        [lo, hi]
    }

    /// The low bits of a (low, high) pair as an integer of type `ty`.
    fn join_halves(&mut self, pair: &[Value], ty: ClType) -> Value {
        match ty {
            types::I128 => self.b.ins().iconcat(pair[0], pair[1]),
            types::I64 => pair[0],
            _ => self.b.ins().ireduce(ty, pair[0]),
        }
    }

    fn block(&mut self, stmts: &Block) {
        for s in stmts {
            self.ensure_open();
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Assign(id, value) => match self.vars[*id] {
                Slot::Var(var) => {
                    let v = self.value(value);
                    self.b.def_var(var, v);
                }
                Slot::Str(_) => {
                    let dest = self.addr(*id);
                    self.into(value, dest);
                }
            },
            Stmt::Drop(id) => {
                let a = self.addr(*id);
                self.rt("drachm_str_drop", &[self.ptr], &[], &[a]);
            }
            Stmt::If { label, branches } => {
                // One block per branch test; `skip to next` jumps to the next one.
                let tests: Vec<ClBlock> = branches.iter().map(|_| self.b.create_block()).collect();
                let end = self.b.create_block();
                self.ifs.insert(*label, (tests.clone(), end));
                self.b.ins().jump(tests[0], &[]);
                for (i, br) in branches.iter().enumerate() {
                    let body_b = self.b.create_block();
                    let next = tests.get(i + 1).copied().unwrap_or(end);
                    self.b.switch_to_block(tests[i]);
                    match &br.test {
                        Some(t) => {
                            let c = self.value(t);
                            self.b.ins().brif(c, body_b, &[], next, &[]);
                        }
                        None => {
                            self.b.ins().jump(body_b, &[]);
                        }
                    }
                    self.b.seal_block(body_b);
                    self.b.switch_to_block(body_b);
                    self.block(&br.body);
                    self.jump_to(end);
                    self.terminated = false;
                }
                for t in tests {
                    self.b.seal_block(t);
                }
                self.b.seal_block(end);
                self.b.switch_to_block(end);
            }
            Stmt::Loop { label, cond, body, step } => {
                let header = self.b.create_block();
                let body_b = self.b.create_block();
                let cont = self.b.create_block();
                let exit = self.b.create_block();
                self.loops.insert(*label, (cont, exit));
                self.b.ins().jump(header, &[]);

                self.b.switch_to_block(header);
                match cond {
                    Some(cond) => {
                        let c = self.value(cond);
                        self.b.ins().brif(c, body_b, &[], exit, &[]);
                    }
                    None => {
                        self.b.ins().jump(body_b, &[]);
                    }
                }
                self.b.seal_block(body_b);

                self.b.switch_to_block(body_b);
                self.block(body);
                self.jump_to(cont);
                self.terminated = false;
                self.b.seal_block(cont);

                self.b.switch_to_block(cont);
                self.block(step);
                self.jump_to(header);
                self.terminated = false;
                self.b.seal_block(header);

                self.b.seal_block(exit);
                self.b.switch_to_block(exit);
            }
            Stmt::Jump(j) => {
                let target = match *j {
                    Jump::Break(l) => self.loops[&l].1,
                    Jump::Continue(l) => self.loops[&l].0,
                    Jump::Leave(l) => self.ifs[&l].1,
                    Jump::Next(l, n) => self.ifs[&l].0[n],
                };
                self.b.ins().jump(target, &[]);
                self.terminated = true;
            }
            Stmt::Return(Some(value)) if value.ty == Type::Str => {
                let dest = self.ret_ptr.expect("a string function has an out pointer");
                self.into(value, dest);
                self.b.ins().return_(&[]);
                self.terminated = true;
            }
            Stmt::Return(value) => {
                let vals: Vec<Value> = value.iter().map(|v| self.value(v)).collect();
                self.b.ins().return_(&vals);
                self.terminated = true;
            }
            Stmt::Expr(e) => {
                self.expr(e);
            }
        }
    }

    fn value(&mut self, e: &Expr) -> Value {
        self.expr(e).expect("expression has a value")
    }

    /// Returns `None` for expressions of type `none`.
    fn expr(&mut self, e: &Expr) -> Option<Value> {
        let ty = e.ty;
        let cl = cl_type(ty);
        Some(match &e.kind {
            ExprKind::Int(bits) => self.int_const(cl.unwrap(), *bits),
            ExprKind::Float(v) => match ty {
                Type::Float(32) => self.b.ins().f32const(*v as f32),
                _ => self.b.ins().f64const(*v),
            },
            ExprKind::Bool(v) => self.b.ins().iconst(types::I8, *v as i64),
            ExprKind::Str(_)
            | ExprKind::Text(_)
            | ExprKind::Radix { .. }
            | ExprKind::Copy(_)
            | ExprKind::Move(_)
            | ExprKind::Temp(..)
            | ExprKind::Join(_) => unreachable!("strings are made with `into` or used with `place`"),
            ExprKind::Local(id) => self.b.use_var(self.var(*id)),
            ExprKind::Scoped(inner, drops) => {
                let v = self.expr(inner);
                self.drops(drops);
                return v;
            }
            ExprKind::StrCall(f, args) => {
                let (params, vals) = self.runtime_args(f.params(), args);
                let returns: Vec<ClType> = cl_type(f.ret()).into_iter().collect();
                return self.rt(f.name(), &params, &returns, &vals).first().copied();
            }
            ExprKind::Neg(inner) => {
                let v = self.value(inner);
                if ty.is_float() { self.b.ins().fneg(v) } else { self.b.ins().ineg(v) }
            }
            ExprKind::Not(inner) => {
                let v = self.value(inner);
                self.b.ins().bxor_imm_u(v, 1)
            }
            ExprKind::BitNot(inner) => {
                let v = self.value(inner);
                self.b.ins().bnot(v)
            }
            ExprKind::Binary(op, l, r) => {
                let operand = l.ty;
                let a = self.value(l);
                let b = self.value(r);
                self.binary(*op, operand, a, b)
            }
            ExprKind::Shift { left, value, amount } => {
                let v = self.value(value);
                let n = self.value(amount);
                // Only the low bits of the amount matter; Cranelift wraps it
                // around the value's width, which is Drachm's rule too.
                let n = match self.b.func.dfg.value_type(n).bits() {
                    32 => n,
                    0..32 => self.b.ins().uextend(types::I32, n),
                    _ => self.b.ins().ireduce(types::I32, n),
                };
                match (*left, is_signed(ty)) {
                    (true, _) => self.b.ins().ishl(v, n),
                    (false, true) => self.b.ins().sshr(v, n),
                    (false, false) => self.b.ins().ushr(v, n),
                }
            }
            ExprKind::IntPow(base, exp) => {
                let bv = self.value(base);
                let xv = self.value(exp);
                let exp_cl = self.b.func.dfg.value_type(xv);
                let negative = if is_signed(exp.ty) {
                    let zero = self.zero(exp_cl);
                    let lt = self.b.ins().icmp(IntCC::SignedLessThan, xv, zero);
                    self.b.ins().uextend(types::I32, lt)
                } else {
                    self.b.ins().iconst(types::I32, 0)
                };
                let base_signed = self.b.ins().iconst(types::I32, is_signed(ty) as i64);
                let (i32t, i64t) = (types::I32, types::I64);
                if cl.unwrap().bits() <= 64 && exp_cl.bits() <= 64 {
                    let b64 = self.widen64(bv, ty);
                    let x64 = self.widen64(xv, exp.ty);
                    let r = self.rt("drachm_ipow64", &[i64t, i64t, i32t, i32t], &[i64t], &[b64, x64, negative, base_signed]);
                    self.join_halves(&[r[0], r[0]], cl.unwrap())
                } else {
                    let [blo, bhi] = self.halves(bv, ty);
                    let [xlo, xhi] = self.halves(xv, exp.ty);
                    let r = self.rt(
                        "drachm_ipow128",
                        &[i64t, i64t, i64t, i64t, i32t, i32t],
                        &[i64t, i64t],
                        &[blo, bhi, xlo, xhi, negative, base_signed],
                    );
                    self.join_halves(&r, cl.unwrap())
                }
            }
            ExprKind::Logic { and, lhs, rhs } => {
                // Short-circuit: only evaluate `rhs` when it decides the result.
                let lv = self.value(lhs);
                let rhs_b = self.b.create_block();
                let merge = self.b.create_block();
                self.b.append_block_param(merge, types::I8);
                if *and {
                    self.b.ins().brif(lv, rhs_b, &[], merge, &[lv.into()]);
                } else {
                    self.b.ins().brif(lv, merge, &[lv.into()], rhs_b, &[]);
                }
                self.b.seal_block(rhs_b);
                self.b.switch_to_block(rhs_b);
                let rv = self.value(rhs);
                self.b.ins().jump(merge, &[rv.into()]);
                self.b.seal_block(merge);
                self.b.switch_to_block(merge);
                self.b.block_params(merge)[0]
            }
            ExprKind::Convert(inner) => {
                let v = self.value(inner);
                self.convert(v, inner.ty, ty)
            }
            ExprKind::Call(id, args) => return self.call(*id, args, None),
            ExprKind::IncDec { local, inc, prefix } => {
                let var = self.var(*local);
                let old = self.b.use_var(var);
                let new = if ty.is_float() {
                    let one = if ty == Type::Float(32) { self.b.ins().f32const(1.0) } else { self.b.ins().f64const(1.0) };
                    if *inc { self.b.ins().fadd(old, one) } else { self.b.ins().fsub(old, one) }
                } else {
                    let one = self.int_const(cl.unwrap(), 1);
                    if *inc { self.b.ins().iadd(old, one) } else { self.b.ins().isub(old, one) }
                };
                self.b.def_var(var, new);
                if *prefix { new } else { old }
            }
            ExprKind::Out { newline, arg } => {
                self.out(*newline, arg);
                return None;
            }
        })
    }

    fn binary(&mut self, op: BinOp, operand: Type, a: Value, b: Value) -> Value {
        if operand.is_float() {
            let (name_rem, name_pow, f) = match operand {
                Type::Float(32) => ("fmodf", "powf", types::F32),
                _ => ("fmod", "pow", types::F64),
            };
            let cc = |op| match op {
                BinOp::Eq => FloatCC::Equal,
                BinOp::Ne => FloatCC::NotEqual,
                BinOp::Lt => FloatCC::LessThan,
                BinOp::Le => FloatCC::LessThanOrEqual,
                BinOp::Gt => FloatCC::GreaterThan,
                _ => FloatCC::GreaterThanOrEqual,
            };
            return match op {
                BinOp::Add => self.b.ins().fadd(a, b),
                BinOp::Sub => self.b.ins().fsub(a, b),
                BinOp::Mul => self.b.ins().fmul(a, b),
                BinOp::Div => self.b.ins().fdiv(a, b),
                BinOp::Rem => self.rt(name_rem, &[f, f], &[f], &[a, b])[0],
                BinOp::Pow => self.rt(name_pow, &[f, f], &[f], &[a, b])[0],
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => unreachable!("checked by typeck"),
                _ => self.b.ins().fcmp(cc(op), a, b),
            };
        }
        let signed = is_signed(operand);
        let cc = match (op, signed) {
            (BinOp::Eq, _) => IntCC::Equal,
            (BinOp::Ne, _) => IntCC::NotEqual,
            (BinOp::Lt, true) => IntCC::SignedLessThan,
            (BinOp::Le, true) => IntCC::SignedLessThanOrEqual,
            (BinOp::Gt, true) => IntCC::SignedGreaterThan,
            (BinOp::Ge, true) => IntCC::SignedGreaterThanOrEqual,
            (BinOp::Lt, false) => IntCC::UnsignedLessThan,
            (BinOp::Le, false) => IntCC::UnsignedLessThanOrEqual,
            (BinOp::Gt, false) => IntCC::UnsignedGreaterThan,
            _ => IntCC::UnsignedGreaterThanOrEqual,
        };
        match op {
            BinOp::Add => self.b.ins().iadd(a, b),
            BinOp::Sub => self.b.ins().isub(a, b),
            BinOp::Mul => self.b.ins().imul(a, b),
            BinOp::Div | BinOp::Rem => self.divide(op == BinOp::Div, operand, a, b),
            BinOp::Pow => unreachable!("integer power is IntPow"),
            BinOp::BitAnd => self.b.ins().band(a, b),
            BinOp::BitOr => self.b.ins().bor(a, b),
            BinOp::BitXor => self.b.ins().bxor(a, b),
            _ => self.b.ins().icmp(cc, a, b),
        }
    }

    /// Whole-number division or remainder: panics on zero, and the smallest
    /// signed value divided by -1 wraps instead of trapping.
    fn divide(&mut self, div: bool, ty: Type, a: Value, b: Value) -> Value {
        let cl = cl_type(ty).unwrap();
        let signed = is_signed(ty);
        if cl == types::I128 {
            // The runtime does the checks for 128 bits.
            let name = match (div, signed) {
                (true, false) => "drachm_div_u128",
                (false, false) => "drachm_rem_u128",
                (true, true) => "drachm_div_s128",
                (false, true) => "drachm_rem_s128",
            };
            let [alo, ahi] = self.halves(a, ty);
            let [blo, bhi] = self.halves(b, ty);
            let r = self.rt(name, &[types::I64; 4], &[types::I64; 2], &[alo, ahi, blo, bhi]);
            return self.join_halves(&r, cl);
        }

        let bad = self.b.create_block();
        let ok = self.b.create_block();
        self.b.ins().brif(b, ok, &[], bad, &[]);
        self.b.seal_block(bad);
        self.b.seal_block(ok);
        self.b.switch_to_block(bad);
        self.rt("drachm_div_zero", &[], &[], &[]);
        self.b.ins().trap(TrapCode::unwrap_user(2));
        self.b.switch_to_block(ok);

        if !signed {
            return if div { self.b.ins().udiv(a, b) } else { self.b.ins().urem(a, b) };
        }
        let minus_one = self.int_const(cl, u128::MAX);
        let is_minus_one = self.b.ins().icmp(IntCC::Equal, b, minus_one);
        let wrap_b = self.b.create_block();
        let normal_b = self.b.create_block();
        let merge = self.b.create_block();
        self.b.append_block_param(merge, cl);
        self.b.ins().brif(is_minus_one, wrap_b, &[], normal_b, &[]);
        self.b.seal_block(wrap_b);
        self.b.seal_block(normal_b);

        self.b.switch_to_block(wrap_b);
        let wrapped = if div { self.b.ins().ineg(a) } else { self.int_const(cl, 0) };
        self.b.ins().jump(merge, &[wrapped.into()]);

        self.b.switch_to_block(normal_b);
        let r = if div { self.b.ins().sdiv(a, b) } else { self.b.ins().srem(a, b) };
        self.b.ins().jump(merge, &[r.into()]);

        self.b.seal_block(merge);
        self.b.switch_to_block(merge);
        self.b.block_params(merge)[0]
    }

    fn convert(&mut self, v: Value, from: Type, to: Type) -> Value {
        let to_cl = cl_type(to).unwrap();
        let from_cl = self.b.func.dfg.value_type(v);
        match (from, to) {
            (Type::Int { .. }, Type::Int { .. }) => {
                if to_cl.bits() < from_cl.bits() {
                    self.b.ins().ireduce(to_cl, v)
                } else if to_cl.bits() == from_cl.bits() {
                    v
                } else if is_signed(from) {
                    self.b.ins().sextend(to_cl, v)
                } else {
                    self.b.ins().uextend(to_cl, v)
                }
            }
            (Type::Int { bits: 128, signed }, Type::Float(fb)) => {
                let name = match (signed, fb) {
                    (false, 32) => "drachm_u128_to_f32",
                    (true, 32) => "drachm_s128_to_f32",
                    (false, _) => "drachm_u128_to_f64",
                    (true, _) => "drachm_s128_to_f64",
                };
                let [lo, hi] = self.halves(v, from);
                self.rt(name, &[types::I64; 2], &[to_cl], &[lo, hi])[0]
            }
            (Type::Int { signed, .. }, Type::Float(_)) => {
                let wide = self.widen64(v, from);
                if signed { self.b.ins().fcvt_from_sint(to_cl, wide) } else { self.b.ins().fcvt_from_uint(to_cl, wide) }
            }
            (Type::Float(a), Type::Float(b)) => {
                if a < b { self.b.ins().fpromote(to_cl, v) } else { self.b.ins().fdemote(to_cl, v) }
            }
            (Type::Float(fb), Type::Int { bits, .. }) => {
                let d = if fb == 32 { self.b.ins().fpromote(types::F64, v) } else { v };
                if bits == 128 {
                    let r = self.rt("drachm_f64_to_u128", &[types::F64], &[types::I64; 2], &[d]);
                    self.join_halves(&r, to_cl)
                } else {
                    let r = self.rt("drachm_f64_to_u64", &[types::F64], &[types::I64], &[d]);
                    self.join_halves(&[r[0], r[0]], to_cl)
                }
            }
            _ => unreachable!("checked by typeck"),
        }
    }

    fn out(&mut self, newline: bool, arg: &Expr) {
        let p = self.place(arg);
        let (ptr, len) = self.load_str(p);
        let nl = self.b.ins().iconst(types::I32, newline as i64);
        self.rt("drachm_out_str", &[self.ptr, types::I64, types::I32], &[], &[ptr, len, nl]);
    }

    fn var(&self, id: LocalId) -> Variable {
        match self.vars[id] {
            Slot::Var(v) => v,
            Slot::Str(_) => unreachable!("a string local is not a variable"),
        }
    }

    /// The address of a string local.
    fn addr(&mut self, id: LocalId) -> Value {
        match self.vars[id] {
            Slot::Str(slot) => self.b.ins().stack_addr(self.ptr, slot, 0),
            Slot::Var(_) => unreachable!("not a string local"),
        }
    }

    fn new_slot(&mut self, size: u32) -> StackSlot {
        self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size, 3))
    }

    fn store_str(&mut self, dest: Value, ptr: Value, len: Value, cap: Value) {
        let f = MemFlagsData::trusted();
        self.b.ins().store(f, ptr, dest, 0);
        self.b.ins().store(f, len, dest, STR_LEN);
        self.b.ins().store(f, cap, dest, STR_CAP);
    }

    /// The pointer and length of the string at `addr`.
    fn load_str(&mut self, addr: Value) -> (Value, Value) {
        let f = MemFlagsData::trusted();
        let ptr = self.b.ins().load(self.ptr, f, addr, 0);
        let len = self.b.ins().load(types::I64, f, addr, STR_LEN);
        (ptr, len)
    }

    /// Copy the 24 bytes of a string value (not its text).
    fn copy_str(&mut self, dest: Value, src: Value) {
        let f = MemFlagsData::trusted();
        for off in [0, STR_LEN, STR_CAP] {
            let v = self.b.ins().load(types::I64, f, src, off);
            self.b.ins().store(f, v, dest, off);
        }
    }

    fn drops(&mut self, locals: &[LocalId]) {
        for &id in locals {
            let a = self.addr(id);
            self.rt("drachm_str_drop", &[self.ptr], &[], &[a]);
        }
    }

    /// A not-owned view of a literal's bytes: (pointer, length).
    fn literal(&mut self, s: &str) -> (Value, Value) {
        let len = self.b.ins().iconst(types::I64, s.len() as i64);
        if s.is_empty() {
            let null = self.b.ins().iconst(self.ptr, 0);
            return (null, len);
        }
        let id = match self.strings.get(s) {
            Some(id) => *id,
            None => {
                let name = format!("drachm_lit_{}", self.strings.len());
                let id = self.module.declare_data(&name, Linkage::Local, false, false).expect("string data");
                let mut data = DataDescription::new();
                data.define(s.as_bytes().to_vec().into_boxed_slice());
                self.module.define_data(id, &data).expect("string data");
                self.strings.insert(s.to_string(), id);
                id
            }
        };
        let gv = self.module.declare_data_in_func(id, self.b.func);
        (self.b.ins().symbol_value(self.ptr, gv), len)
    }

    /// Call a Drachm function. String arguments are owned values, each made
    /// in a slot of its own whose address is passed; the callee takes them
    /// over. `out` is where a string result goes.
    fn call(&mut self, id: crate::hir::FuncId, args: &[Expr], out: Option<Value>) -> Option<Value> {
        let mut vals: Vec<Value> = out.into_iter().collect();
        for a in args {
            if a.ty == Type::Str {
                let slot = self.new_slot(STR_SIZE);
                let addr = self.b.ins().stack_addr(self.ptr, slot, 0);
                self.into(a, addr);
                vals.push(addr);
            } else {
                vals.push(self.value(a));
            }
        }
        let callee = self.module.declare_func_in_func(self.ids[id], self.b.func);
        let call = self.b.ins().call(callee, &vals);
        self.b.inst_results(call).first().copied()
    }

    /// Arguments for a runtime function: strings by pointer, numbers by value.
    fn runtime_args(&mut self, params: &[Type], args: &[Expr]) -> (Vec<ClType>, Vec<Value>) {
        let mut types = Vec::new();
        let mut vals = Vec::new();
        for (a, &p) in args.iter().zip(params) {
            match cl_type(p) {
                Some(t) => {
                    types.push(t);
                    vals.push(self.value(a));
                }
                None => {
                    types.push(self.ptr);
                    vals.push(self.place(a));
                }
            }
        }
        (types, vals)
    }

    /// Emit what makes the string place `e`; returns its address.
    fn place(&mut self, e: &Expr) -> Value {
        match &e.kind {
            ExprKind::Local(id) => self.addr(*id),
            ExprKind::Str(_) => {
                let slot = self.new_slot(STR_SIZE);
                let addr = self.b.ins().stack_addr(self.ptr, slot, 0);
                self.into(e, addr);
                addr
            }
            ExprKind::Temp(id, inner) => {
                let addr = self.addr(*id);
                self.into(inner, addr);
                addr
            }
            _ => unreachable!("not a string place"),
        }
    }

    /// Emit what writes the owned string `e` to `dest`.
    fn into(&mut self, e: &Expr, dest: Value) {
        let (i32t, i64t, ptr) = (types::I32, types::I64, self.ptr);
        match &e.kind {
            ExprKind::Str(s) => {
                let (p, len) = self.literal(s);
                let zero = self.b.ins().iconst(i64t, 0);
                self.store_str(dest, p, len, zero);
            }
            ExprKind::Copy(inner) => {
                let src = self.place(inner);
                self.rt("drachm_str_copy", &[ptr, ptr], &[], &[dest, src]);
            }
            ExprKind::Move(id) => {
                let src = self.addr(*id);
                self.copy_str(dest, src);
                let zero = self.b.ins().iconst(i64t, 0);
                self.store_str(src, zero, zero, zero);
            }
            ExprKind::Text(value) => {
                let v = self.value(value);
                match value.ty {
                    Type::Bool => {
                        self.rt("drachm_text_bool", &[ptr, types::I8], &[], &[dest, v]);
                    }
                    Type::Float(32) => {
                        self.rt("drachm_text_f32", &[ptr, types::F32], &[], &[dest, v]);
                    }
                    Type::Float(_) => {
                        self.rt("drachm_text_f64", &[ptr, types::F64], &[], &[dest, v]);
                    }
                    Type::Int { bits: 128, signed } => {
                        let [lo, hi] = self.halves(v, value.ty);
                        let name = if signed { "drachm_text_s128" } else { "drachm_text_u128" };
                        self.rt(name, &[ptr, i64t, i64t], &[], &[dest, lo, hi]);
                    }
                    Type::Int { signed, .. } => {
                        let wide = self.widen64(v, value.ty);
                        let name = if signed { "drachm_text_s64" } else { "drachm_text_u64" };
                        self.rt(name, &[ptr, i64t], &[], &[dest, wide]);
                    }
                    Type::None | Type::Str => unreachable!("checked by typeck"),
                }
            }
            ExprKind::Radix { value, base, upper } => {
                let v = self.value(value);
                let [lo, hi] = self.halves(v, value.ty);
                let signed = self.b.ins().iconst(types::I8, is_signed(value.ty) as i64);
                let base = self.b.ins().iconst(i32t, *base as i64);
                let upper = self.b.ins().iconst(types::I8, *upper as i64);
                self.rt("drachm_str_radix", &[ptr, i64t, i64t, types::I8, i32t, types::I8], &[], &[dest, lo, hi, signed, base, upper]);
            }
            ExprKind::StrCall(f, args) => {
                let (mut params, mut vals) = self.runtime_args(f.params(), args);
                params.insert(0, ptr);
                vals.insert(0, dest);
                self.rt(f.name(), &params, &[], &vals);
            }
            ExprKind::Join(parts) => {
                // The parts' values side by side, as the runtime's array.
                let array = self.new_slot(STR_SIZE * parts.len() as u32);
                for (i, part) in parts.iter().enumerate() {
                    let src = self.place(part);
                    let at = self.b.ins().stack_addr(ptr, array, (STR_SIZE as usize * i) as i32);
                    self.copy_str(at, src);
                }
                let base = self.b.ins().stack_addr(ptr, array, 0);
                let n = self.b.ins().iconst(i64t, parts.len() as i64);
                self.rt("drachm_str_join", &[ptr, ptr, i64t], &[], &[dest, base, n]);
            }
            ExprKind::Call(id, args) => {
                self.call(*id, args, Some(dest));
            }
            ExprKind::Scoped(inner, drops) => {
                self.into(inner, dest);
                self.drops(drops);
            }
            _ => unreachable!("not an owned string"),
        }
    }
}
