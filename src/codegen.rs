use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, Signature, TrapCode, UserFuncName, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::ast::*;
use crate::typeck::Sig;

/// Drachm functions are mangled so they can't collide with C symbols.
fn mangle(name: &str) -> String {
    format!("drachm_{name}")
}

fn cl_type(t: Type) -> Option<types::Type> {
    match t {
        Type::I64 => Some(types::I64),
        Type::Bool => Some(types::I8),
        Type::Unit => None,
    }
}

pub fn compile(prog: &Program, sigs: &HashMap<String, Sig>, opt: bool) -> Result<Vec<u8>, String> {
    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    flags.set("opt_level", if opt { "speed" } else { "none" }).map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let builder = ObjectBuilder::new(isa, "drachm", cranelift_module::default_libcall_names())
        .map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);

    // Declare every function up front so calls can be emitted in any order.
    let mut ids = HashMap::new();
    // In source order, so the same input always gives the same object file.
    for f in &prog.funcs {
        let (name, sig) = (&f.name, &sigs[&f.name]);
        let mut s = module.make_signature();
        s.params.extend(sig.params.iter().filter_map(|&t| cl_type(t)).map(AbiParam::new));
        s.returns.extend(cl_type(sig.ret).map(AbiParam::new));
        let id = module.declare_function(&mangle(name), Linkage::Local, &s).map_err(|e| e.to_string())?;
        ids.insert(name.clone(), id);
    }
    let mut runtime = HashMap::new();
    for (name, param) in [("drachm_print_i64", types::I64), ("drachm_print_bool", types::I8)] {
        let mut s = module.make_signature();
        s.params.push(AbiParam::new(param));
        runtime.insert(name, module.declare_function(name, Linkage::Import, &s).map_err(|e| e.to_string())?);
    }
    let div_zero = module
        .declare_function("drachm_div_zero", Linkage::Import, &module.make_signature())
        .map_err(|e| e.to_string())?;
    runtime.insert("drachm_div_zero", div_zero);

    let mut ctx = module.make_context();
    let mut fctx = FunctionBuilderContext::new();

    for f in &prog.funcs {
        let id = ids[&f.name];
        ctx.func.signature = module.declarations().get_function_decl(id).signature.clone();
        ctx.func.name = UserFuncName::user(0, id.as_u32());
        let cfg = module.isa().frontend_config();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            b.seal_block(entry);

            let mut g = Gen {
                b,
                module: &mut module,
                ids: &ids,
                sigs,
                runtime: &runtime,
                scopes: vec![HashMap::new()],
                terminated: false,
            };
            for (i, (name, ty)) in f.params.iter().enumerate() {
                let v = g.b.block_params(entry)[i];
                let var = g.b.declare_var(cl_type(*ty).unwrap());
                g.b.def_var(var, v);
                g.scopes[0].insert(name.clone(), (var, *ty));
            }
            g.block(&f.body);
            if !g.terminated {
                if f.ret == Type::Unit {
                    g.b.ins().return_(&[]);
                } else {
                    // The type checker proved every path returns, so this is dead.
                    g.b.ins().trap(TrapCode::unwrap_user(1));
                }
            }
            g.b.finalize(cfg);
        }
        module.define_function(id, &mut ctx).map_err(|e| format!("{e:?}"))?;
        module.clear_context(&mut ctx);
    }

    emit_c_main(&mut module, &mut ctx, &mut fctx, ids["main"], sigs["main"].ret)?;
    module.finish().emit().map_err(|e| e.to_string())
}

/// `int main(void)` that calls the Drachm `main` and turns its result into an exit code.
fn emit_c_main(
    module: &mut ObjectModule,
    ctx: &mut Context,
    fctx: &mut FunctionBuilderContext,
    drachm_main: FuncId,
    ret: Type,
) -> Result<(), String> {
    let mut sig = Signature::new(module.isa().default_call_conv());
    sig.returns.push(AbiParam::new(types::I32));
    let id = module.declare_function("main", Linkage::Export, &sig).map_err(|e| e.to_string())?;
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
        let code = if ret == Type::I64 {
            let r = b.inst_results(call)[0];
            b.ins().ireduce(types::I32, r)
        } else {
            b.ins().iconst(types::I32, 0)
        };
        b.ins().return_(&[code]);
        b.finalize(cfg);
    }
    module.define_function(id, ctx).map_err(|e| format!("{e:?}"))?;
    module.clear_context(ctx);
    Ok(())
}

struct Gen<'a, 'b> {
    b: FunctionBuilder<'b>,
    module: &'a mut ObjectModule,
    ids: &'a HashMap<String, FuncId>,
    sigs: &'a HashMap<String, Sig>,
    runtime: &'a HashMap<&'static str, FuncId>,
    scopes: Vec<HashMap<String, (Variable, Type)>>,
    /// The current block already ended in a terminator (e.g. after `return`).
    terminated: bool,
}

impl Gen<'_, '_> {
    fn lookup(&self, name: &str) -> (Variable, Type) {
        *self.scopes.iter().rev().find_map(|s| s.get(name)).expect("checked by typeck")
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

    fn jump_to(&mut self, target: cranelift_codegen::ir::Block) {
        if !self.terminated {
            self.b.ins().jump(target, &[]);
        }
    }

    fn block(&mut self, stmts: &Block) {
        self.scopes.push(HashMap::new());
        for s in stmts {
            self.ensure_open();
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, ty, value, .. } => {
                let v = self.expr(value).unwrap();
                let var = self.b.declare_var(cl_type(*ty).unwrap());
                self.b.def_var(var, v);
                self.scopes.last_mut().unwrap().insert(name.clone(), (var, *ty));
            }
            Stmt::Assign { name, value, .. } => {
                let v = self.expr(value).unwrap();
                let (var, _) = self.lookup(name);
                self.b.def_var(var, v);
            }
            Stmt::If { cond, then, els } => {
                let c = self.expr(cond).unwrap();
                let then_b = self.b.create_block();
                let else_b = self.b.create_block();
                let merge = self.b.create_block();
                self.b.ins().brif(c, then_b, &[], else_b, &[]);
                self.b.seal_block(then_b);
                self.b.seal_block(else_b);

                self.b.switch_to_block(then_b);
                self.block(then);
                self.jump_to(merge);
                self.terminated = false;

                self.b.switch_to_block(else_b);
                if let Some(els) = els {
                    self.block(els);
                }
                self.jump_to(merge);
                self.terminated = false;

                self.b.seal_block(merge);
                self.b.switch_to_block(merge);
            }
            Stmt::While { cond, body } => {
                let header = self.b.create_block();
                let body_b = self.b.create_block();
                let exit = self.b.create_block();
                self.b.ins().jump(header, &[]);

                self.b.switch_to_block(header);
                let c = self.expr(cond).unwrap();
                self.b.ins().brif(c, body_b, &[], exit, &[]);
                self.b.seal_block(body_b);

                self.b.switch_to_block(body_b);
                self.block(body);
                self.jump_to(header);
                self.terminated = false;
                self.b.seal_block(header);

                self.b.seal_block(exit);
                self.b.switch_to_block(exit);
            }
            Stmt::Return { value, .. } => {
                let vals: Vec<Value> = value.iter().filter_map(|v| self.expr(v)).collect();
                self.b.ins().return_(&vals);
                self.terminated = true;
            }
            Stmt::Expr(e) => {
                self.expr(e);
            }
        }
    }

    fn call(&mut self, id: FuncId, args: &[Value]) -> Option<Value> {
        let callee = self.module.declare_func_in_func(id, self.b.func);
        let call = self.b.ins().call(callee, args);
        self.b.inst_results(call).first().copied()
    }

    /// Returns `None` for expressions of type `()`.
    fn expr(&mut self, e: &Expr) -> Option<Value> {
        Some(match &e.kind {
            ExprKind::Int(n) => self.b.ins().iconst(types::I64, *n),
            ExprKind::Bool(v) => self.b.ins().iconst(types::I8, *v as i64),
            ExprKind::Var(name) => {
                let (var, _) = self.lookup(name);
                self.b.use_var(var)
            }
            ExprKind::Unary(UnOp::Neg, inner) => {
                let v = self.expr(inner)?;
                self.b.ins().ineg(v)
            }
            ExprKind::Unary(UnOp::Not, inner) => {
                let v = self.expr(inner)?;
                self.b.ins().bxor_imm_u(v, 1)
            }
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), l, r) => {
                // Short-circuit: only evaluate `r` when it decides the result.
                let lv = self.expr(l)?;
                let rhs_b = self.b.create_block();
                let merge = self.b.create_block();
                self.b.append_block_param(merge, types::I8);
                if *op == BinOp::And {
                    self.b.ins().brif(lv, rhs_b, &[], merge, &[lv.into()]);
                } else {
                    self.b.ins().brif(lv, merge, &[lv.into()], rhs_b, &[]);
                }
                self.b.seal_block(rhs_b);
                self.b.switch_to_block(rhs_b);
                let rv = self.expr(r)?;
                self.b.ins().jump(merge, &[rv.into()]);
                self.b.seal_block(merge);
                self.b.switch_to_block(merge);
                self.b.block_params(merge)[0]
            }
            ExprKind::Binary(op, l, r) => {
                let lv = self.expr(l)?;
                let rv = self.expr(r)?;
                let ins = self.b.ins();
                match op {
                    BinOp::Add => ins.iadd(lv, rv),
                    BinOp::Sub => ins.isub(lv, rv),
                    BinOp::Mul => ins.imul(lv, rv),
                    BinOp::Div | BinOp::Rem => {
                        self.check_nonzero(rv);
                        let ins = self.b.ins();
                        if *op == BinOp::Div { ins.sdiv(lv, rv) } else { ins.srem(lv, rv) }
                    }
                    BinOp::Eq => ins.icmp(IntCC::Equal, lv, rv),
                    BinOp::Ne => ins.icmp(IntCC::NotEqual, lv, rv),
                    BinOp::Lt => ins.icmp(IntCC::SignedLessThan, lv, rv),
                    BinOp::Le => ins.icmp(IntCC::SignedLessThanOrEqual, lv, rv),
                    BinOp::Gt => ins.icmp(IntCC::SignedGreaterThan, lv, rv),
                    BinOp::Ge => ins.icmp(IntCC::SignedGreaterThanOrEqual, lv, rv),
                    BinOp::And | BinOp::Or => unreachable!(),
                }
            }
            ExprKind::Call(name, args) if name == "print" => {
                let v = self.expr(&args[0])?;
                let f = if self.b.func.dfg.value_type(v) == types::I8 { "drachm_print_bool" } else { "drachm_print_i64" };
                self.call(self.runtime[f], &[v]);
                return None;
            }
            ExprKind::Call(name, args) => {
                let vals: Vec<Value> = args.iter().filter_map(|a| self.expr(a)).collect();
                let r = self.call(self.ids[name], &vals);
                if self.sigs[name].ret == Type::Unit {
                    return None;
                }
                r.unwrap()
            }
        })
    }

    /// Branch to the runtime's division-by-zero panic if `v == 0`.
    fn check_nonzero(&mut self, v: Value) {
        let bad = self.b.create_block();
        let ok = self.b.create_block();
        self.b.ins().brif(v, ok, &[], bad, &[]);
        self.b.seal_block(bad);
        self.b.seal_block(ok);
        self.b.switch_to_block(bad);
        self.call(self.runtime["drachm_div_zero"], &[]);
        self.b.ins().trap(TrapCode::unwrap_user(2));
        self.b.switch_to_block(ok);
    }
}
