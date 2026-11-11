//! The fast backend: `hir` → Cranelift IR → object file. Used for plain
//! (non `-O`) builds. Must behave exactly like the C backend in `cgen`.
//!
//! Numbers and bools live in SSA variables. Strings and structs live in
//! memory (stack slots): a string is 24 bytes laid out as `drachm_str`
//! (pointer, length, capacity), a struct is laid out C-style (`hir::layout`).
//! Both go to Drachm functions by address:
//! - A string parameter points at a value the callee takes over (it copies
//!   the 24 bytes into its own slot and drops it later).
//! - A struct parameter points at memory the callee takes over and uses as
//!   its own; a method's `self` points at the caller's value.
//! - A string or struct result is written through a hidden first parameter
//!   that points at where the caller wants it.
//!
//! Strings go to the runtime by pointer.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlagsData, Signature, StackSlotData, StackSlotKind, TrapCode, UserFuncName, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::hir::*;
use crate::types::Type;

type ClType = types::Type;
type ClBlock = cranelift_codegen::ir::Block;

/// Loads and stores of strings and structs: always aligned, never trapping.
const MEM: MemFlagsData = MemFlagsData::trusted();

/// Offsets of a `drachm_str`'s length and capacity.
const STR_LEN: i32 = 8;
const STR_CAP: i32 = 16;

/// The Cranelift type of a number or bool; `None` for everything else.
fn cl_type(t: Type) -> Option<ClType> {
    match t {
        Type::Int { bits, .. } => ClType::int(bits),
        Type::Float(32) => Some(types::F32),
        Type::Float(_) => Some(types::F64),
        Type::Bool => Some(types::I8),
        Type::Ref { .. } | Type::Ptr { .. } => Some(types::I64),
        Type::None | Type::Str | Type::Struct { .. } => None,
    }
}

/// Strings and structs live in memory and are handled by address.
fn in_memory(t: Type) -> bool {
    t == Type::Str || t.struct_id().is_some()
}

fn signature(module: &ObjectModule, f: &Func) -> Signature {
    let ptr = module.target_config().pointer_type();
    let mut s = module.make_signature();
    if in_memory(f.ret) {
        s.params.push(AbiParam::new(ptr));
    }
    s.params.extend(f.locals[..f.params].iter().map(|l| AbiParam::new(cl_type(l.ty).unwrap_or(ptr))));
    s.returns.extend(cl_type(f.ret).map(AbiParam::new));
    s
}

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
        let s = signature(&module, f);
        ids.push(module.declare_function(&f.symbol, Linkage::Local, &s).map_err(err)?);
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
            let hidden = in_memory(f.ret) as usize;
            let mut g = Gen {
                b,
                module: &mut module,
                prog,
                func: f,
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
                    // A string parameter is taken over from the caller.
                    None if l.ty == Type::Str => {
                        let addr = g.mem(l.ty);
                        match incoming {
                            Some(from) => g.copy_bits(l.ty, addr, from),
                            None => g.clear(l.ty, addr),
                        }
                        Slot::Mem(addr)
                    }
                    // A struct parameter's memory is the callee's own (or, for
                    // `self`, the caller's value).
                    None => match incoming {
                        Some(p) => Slot::Mem(p),
                        None => {
                            let addr = g.mem(l.ty);
                            g.clear(l.ty, addr);
                            Slot::Mem(addr)
                        }
                    },
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
        module.define_function(id, &mut ctx).map_err(|e| format!("in `{}`: {e:?}", f.symbol))?;
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

/// Where a local lives: numbers and bools in SSA variables, strings and
/// structs in memory at a fixed address.
#[derive(Clone, Copy)]
enum Slot {
    Var(Variable),
    Mem(Value),
}

/// One piece of a value in memory, at an offset.
#[derive(Clone, Copy)]
enum Leaf {
    Scalar(ClType),
    Str,
}

struct Gen<'a, 'b> {
    b: FunctionBuilder<'b>,
    module: &'a mut ObjectModule,
    prog: &'a Program,
    func: &'a Func,
    ids: &'a [FuncId],
    /// Runtime functions imported so far.
    runtime: &'a mut HashMap<&'static str, FuncId>,
    /// The read-only data of each string literal emitted so far.
    strings: &'a mut HashMap<String, DataId>,
    ptr: ClType,
    vars: Vec<Slot>,
    /// Where a function that returns a string or struct writes it.
    ret_ptr: Option<Value>,
    /// Jump targets of each `Loop`: (step, exit).
    loops: HashMap<Label, (ClBlock, ClBlock)>,
    /// Jump targets of each `If`: (the test of each branch, exit).
    ifs: HashMap<Label, (Vec<ClBlock>, ClBlock)>,
    /// The current block already ended in a terminator (e.g. after `return`).
    terminated: bool,
}

impl Gen<'_, '_> {
    /// Fresh memory for a string or struct; returns its address.
    fn mem(&mut self, ty: Type) -> Value {
        let (size, align) = self.prog.size_align(ty);
        let slot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size.max(1), align.trailing_zeros() as u8));
        self.b.ins().stack_addr(self.ptr, slot, 0)
    }

    /// `base + off`.
    fn offset(&mut self, base: Value, off: u32) -> Value {
        if off == 0 { base } else { self.b.ins().iadd_imm_u(base, off as i64) }
    }

    /// Every number, bool and string inside a value of type `ty`, as (offset, leaf).
    fn leaves(&self, ty: Type, at: u32, out: &mut Vec<(u32, Leaf)>) {
        if ty == Type::Str {
            out.push((at, Leaf::Str));
            return;
        }
        if let Some(cl) = cl_type(ty) {
            out.push((at, Leaf::Scalar(cl)));
            return;
        }
        for f in &self.prog.struct_def(ty).fields {
            self.leaves(f.ty, at + f.offset, out);
        }
    }

    fn leaves_of(&self, ty: Type) -> Vec<(u32, Leaf)> {
        let mut out = Vec::new();
        self.leaves(ty, 0, &mut out);
        out
    }

    /// Copy the bits of a value of type `ty` (padding is never read). For an
    /// owning type this moves what it owns: only one of the two may keep it.
    fn copy_bits(&mut self, ty: Type, dst: Value, src: Value) {
        for (off, leaf) in self.leaves_of(ty) {
            let off = off as i32;
            match leaf {
                Leaf::Scalar(cl) => {
                    let v = self.b.ins().load(cl, MEM, src, off);
                    self.b.ins().store(MEM, v, dst, off);
                }
                Leaf::Str => {
                    for word in [0, STR_LEN, STR_CAP] {
                        let v = self.b.ins().load(types::I64, MEM, src, off + word);
                        self.b.ins().store(MEM, v, dst, off + word);
                    }
                }
            }
        }
    }

    /// A deep copy: like `copy_bits`, but each string gets its own text.
    fn deep_copy(&mut self, ty: Type, dst: Value, src: Value) {
        for (off, leaf) in self.leaves_of(ty) {
            match leaf {
                Leaf::Scalar(cl) => {
                    let v = self.b.ins().load(cl, MEM, src, off as i32);
                    self.b.ins().store(MEM, v, dst, off as i32);
                }
                Leaf::Str => {
                    let (d, s) = (self.offset(dst, off), self.offset(src, off));
                    self.rt("drachm_str_copy", &[self.ptr, self.ptr], &[], &[d, s]);
                }
            }
        }
    }

    /// Drop every string inside a value of type `ty`, leaving them empty.
    fn drop_value(&mut self, ty: Type, addr: Value) {
        for (off, leaf) in self.leaves_of(ty) {
            if let Leaf::Str = leaf {
                let a = self.offset(addr, off);
                self.rt("drachm_str_drop", &[self.ptr], &[], &[a]);
            }
        }
    }

    /// Make every string inside a value of type `ty` empty, without freeing.
    fn clear(&mut self, ty: Type, addr: Value) {
        let leaves = self.leaves_of(ty);
        if leaves.iter().any(|(_, l)| matches!(l, Leaf::Str)) {
            let zero = self.b.ins().iconst(types::I64, 0);
            for (off, leaf) in leaves {
                if let Leaf::Str = leaf {
                    for word in [0, STR_LEN, STR_CAP] {
                        self.b.ins().store(MEM, zero, addr, off as i32 + word);
                    }
                }
            }
        }
    }

    /// The address of a string or struct local.
    fn addr(&self, id: LocalId) -> Value {
        match self.vars[id] {
            Slot::Mem(a) => a,
            Slot::Var(_) => unreachable!("not a local in memory"),
        }
    }

    /// The memory of a place, as (base address, offset), and its type.
    fn place_addr(&self, place: &Place) -> (Value, u32, Type) {
        let mut ty = self.func.locals[place.local].ty;
        let mut off = 0;
        for i in place.fields() {
            let f = &self.prog.struct_def(ty).fields[i];
            off += f.offset;
            ty = f.ty;
        }
        (self.addr(place.local), off, ty)
    }

    /// The memory of a place-shaped expression (`Expr::is_place`), as (base
    /// address, offset), without copying it. A `Temp` is made here.
    fn expr_addr(&mut self, e: &Expr) -> (Value, u32) {
        match &e.kind {
            ExprKind::Local(id) => (self.addr(*id), 0),
            ExprKind::Temp(id, inner) => {
                let a = self.addr(*id);
                self.into(inner, a);
                (a, 0)
            }
            ExprKind::Field(inner, i) => {
                let off = self.prog.struct_def(inner.ty).fields[*i].offset;
                let (base, at) = if inner.is_place() { self.expr_addr(inner) } else { (self.value(inner), 0) };
                (base, at + off)
            }
            _ => unreachable!("not a place"),
        }
    }

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
            Stmt::Assign(place, value) => match self.vars[place.local] {
                Slot::Var(var) => {
                    let v = self.value(value);
                    self.b.def_var(var, v);
                }
                Slot::Mem(_) => {
                    let (base, off, ty) = self.place_addr(place);
                    if self.prog.owns(ty) {
                        // The place is empty and nothing reads it while the value is made.
                        let dest = self.offset(base, off);
                        self.into(value, dest);
                    } else if in_memory(ty) {
                        // A plain struct may be read while its new value is made:
                        // make it apart, then copy it in.
                        let v = self.value(value);
                        let dest = self.offset(base, off);
                        self.copy_bits(ty, dest, v);
                    } else {
                        let v = self.value(value);
                        self.b.ins().store(MEM, v, base, off as i32);
                    }
                }
            },
            Stmt::Decl(_) => {}
            Stmt::Unsafe(b) => self.block(b),
            Stmt::Destroy(_) => unreachable!("destroy cannot be built yet"),
            Stmt::Drop(place) => {
                let (base, off, ty) = self.place_addr(place);
                let a = self.offset(base, off);
                self.drop_value(ty, a);
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
            Stmt::Loop { label, cond, body, step, .. } => {
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
            Stmt::Return(Some(value)) if in_memory(value.ty) => {
                let dest = self.ret_ptr.expect("a function returning a string or struct has an out pointer");
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

    /// Returns `None` for expressions of type `none`. A struct value is the
    /// address of a fresh copy (only for structs that own no strings: owning
    /// values are made with `into` or used with `place`).
    fn expr(&mut self, e: &Expr) -> Option<Value> {
        let ty = e.ty;
        if ty.struct_id().is_some() {
            let tmp = self.mem(ty);
            self.into(e, tmp);
            return Some(tmp);
        }
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
            | ExprKind::Struct(_)
            | ExprKind::Join(_) => unreachable!("strings and structs are made with `into` or used with `place`"),
            ExprKind::Local(id) => match self.vars[*id] {
                Slot::Var(var) => self.b.use_var(var),
                Slot::Mem(_) => unreachable!("a string is used with `place`"),
            },
            ExprKind::Field(..) => {
                let (base, off) = self.expr_addr(e);
                self.b.ins().load(cl.unwrap(), MEM, base, off as i32)
            }
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
            ExprKind::IncDec { place, inc, prefix } => {
                let var = match self.vars[place.local] {
                    Slot::Var(var) => Some(var),
                    Slot::Mem(_) => None,
                };
                let addr = var.is_none().then(|| self.place_addr(place));
                let old = match (var, addr) {
                    (Some(var), _) => self.b.use_var(var),
                    (None, Some((base, off, _))) => self.b.ins().load(cl.unwrap(), MEM, base, off as i32),
                    _ => unreachable!(),
                };
                let new = if ty.is_float() {
                    let one = if ty == Type::Float(32) { self.b.ins().f32const(1.0) } else { self.b.ins().f64const(1.0) };
                    if *inc { self.b.ins().fadd(old, one) } else { self.b.ins().fsub(old, one) }
                } else {
                    let one = self.int_const(cl.unwrap(), 1);
                    if *inc { self.b.ins().iadd(old, one) } else { self.b.ins().isub(old, one) }
                };
                match (var, addr) {
                    (Some(var), _) => self.b.def_var(var, new),
                    (None, Some((base, off, _))) => {
                        self.b.ins().store(MEM, new, base, off as i32);
                    }
                    _ => unreachable!(),
                }
                if *prefix { new } else { old }
            }
            ExprKind::Borrow { .. }
            | ExprKind::Deref(_)
            | ExprKind::DeepCopy(_)
            | ExprKind::Take { .. }
            | ExprKind::Lend(_)
            | ExprKind::AddressOf(_)
            | ExprKind::NullPtr => unreachable!("borrows and pointers cannot be built yet"),
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

    fn store_str(&mut self, dest: Value, ptr: Value, len: Value, cap: Value) {
        self.b.ins().store(MEM, ptr, dest, 0);
        self.b.ins().store(MEM, len, dest, STR_LEN);
        self.b.ins().store(MEM, cap, dest, STR_CAP);
    }

    /// The pointer and length of the string at `addr`.
    fn load_str(&mut self, addr: Value) -> (Value, Value) {
        let ptr = self.b.ins().load(self.ptr, MEM, addr, 0);
        let len = self.b.ins().load(types::I64, MEM, addr, STR_LEN);
        (ptr, len)
    }

    fn drops(&mut self, locals: &[LocalId]) {
        for &id in locals {
            let a = self.addr(id);
            self.drop_value(self.func.locals[id].ty, a);
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

    /// Call a Drachm function. String and struct arguments are made in
    /// memory of their own whose address is passed; the callee takes them
    /// over. A method's receiver is passed by address instead: a place
    /// directly, any other value through a fresh copy. `out` is where a
    /// string or struct result goes.
    fn call(&mut self, id: crate::hir::FuncId, args: &[Expr], out: Option<Value>) -> Option<Value> {
        let has_self = self.prog.funcs[id].has_self;
        let mut vals: Vec<Value> = out.into_iter().collect();
        for (i, a) in args.iter().enumerate() {
            let v = if i == 0 && has_self && a.is_place() {
                let (base, off) = self.expr_addr(a);
                self.offset(base, off)
            } else if in_memory(a.ty) {
                let tmp = self.mem(a.ty);
                self.into(a, tmp);
                tmp
            } else {
                self.value(a)
            };
            vals.push(v);
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

    /// Emit what makes the place `e` (a string literal or `is_place`);
    /// returns its address.
    fn place(&mut self, e: &Expr) -> Value {
        match &e.kind {
            ExprKind::Str(_) => {
                let addr = self.mem(Type::Str);
                self.into(e, addr);
                addr
            }
            _ => {
                let (base, off) = self.expr_addr(e);
                self.offset(base, off)
            }
        }
    }

    /// Emit what writes the value `e`, a string or a struct, to `dest`. For
    /// an owning type `e` is an owned value; a plain struct may be any value
    /// (a place is copied).
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
                self.deep_copy(e.ty, dest, src);
            }
            ExprKind::Move(id) => {
                let src = self.addr(*id);
                self.copy_bits(e.ty, dest, src);
                self.clear(e.ty, src);
            }
            ExprKind::Local(_) | ExprKind::Field(..) | ExprKind::Temp(..) => {
                let src = self.place(e);
                self.copy_bits(e.ty, dest, src);
            }
            ExprKind::Struct(fields) => {
                for (i, value) in fields {
                    let off = self.prog.struct_def(e.ty).fields[*i].offset;
                    if in_memory(value.ty) {
                        let at = self.offset(dest, off);
                        self.into(value, at);
                    } else {
                        let v = self.value(value);
                        self.b.ins().store(MEM, v, dest, off as i32);
                    }
                }
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
                    Type::None | Type::Str | Type::Struct { .. } | Type::Ref { .. } | Type::Ptr { .. } => unreachable!("checked by typeck"),
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
                let size = STR_SIZE * parts.len() as u32;
                let array = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, size, 3));
                for (i, part) in parts.iter().enumerate() {
                    let src = self.place(part);
                    let at = self.b.ins().stack_addr(ptr, array, (STR_SIZE as usize * i) as i32);
                    self.copy_bits(Type::Str, at, src);
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
            _ => unreachable!("not a string or struct value"),
        }
    }
}
